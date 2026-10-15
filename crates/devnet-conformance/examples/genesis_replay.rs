//! Replays a public network from genesis with the Rust app, using blocks
//! fetched by `conformance/devnet/fetch-chain.sh`, and checks every app
//! hash (header `h+1`) and every tx result's code / gas against the
//! network's `/block_results`.
//!
//! A real-data check of the state machine at the network's genesis app
//! version (e.g. Mocha, `mocha-5`, starts at app version 9). Blocks are
//! not light-client verified: the fetched data is trusted.
//!
//! `genesis_replay <dir> [last-height]`
#![allow(
    clippy::print_stdout,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use celestia_proto::celestia_core_proto::abci::{
    CommitInfo, RequestFinalizeBlock, RequestInitChain, Validator, VoteInfo,
};
use celestia_proto::celestia_core_proto::tendermint::types::{
    AbciParams, BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams,
};
use serde_json::Value;

fn read(path: &Path) -> anyhow::Result<Value> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| path.display().to_string())?)
        .with_context(|| path.display().to_string())
}

fn num(v: &Value) -> i64 {
    match v {
        Value::String(s) => s.parse().unwrap_or_default(),
        Value::Number(n) => n.as_i64().unwrap_or_default(),
        _ => 0,
    }
}

fn hexb(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap_or_default()).unwrap_or_default()
}

/// RFC 3339 UTC time (`2026-08-18T12:00:00.123456789Z`) as a timestamp.
fn timestamp(s: &str) -> anyhow::Result<pbjson_types::Timestamp> {
    let s = s.trim_end_matches('Z');
    let (date, time) = s.split_once('T').ok_or_else(|| anyhow!("bad time {s}"))?;
    let d: Vec<i64> = date.split('-').map(str::parse).collect::<Result<_, _>>()?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let t: Vec<i64> = hms.split(':').map(str::parse).collect::<Result<_, _>>()?;
    let (y, m, day) = (d[0], d[1], d[2]);
    // days_from_civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let nanos = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<9}")[..9].parse::<i32>()?
    };
    Ok(pbjson_types::Timestamp {
        seconds: days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2],
        nanos,
    })
}

fn consensus_params(v: &Value) -> ConsensusParams {
    let dur = |s: &Value| {
        let ns = num(s);
        pbjson_types::Duration {
            seconds: ns / 1_000_000_000,
            nanos: i32::try_from(ns % 1_000_000_000).unwrap_or_default(),
        }
    };
    ConsensusParams {
        block: Some(BlockParams {
            max_bytes: num(&v["block"]["max_bytes"]),
            max_gas: num(&v["block"]["max_gas"]),
        }),
        evidence: Some(EvidenceParams {
            max_age_num_blocks: num(&v["evidence"]["max_age_num_blocks"]),
            max_age_duration: Some(dur(&v["evidence"]["max_age_duration"])),
            max_bytes: num(&v["evidence"]["max_bytes"]),
        }),
        validator: Some(ValidatorParams {
            pub_key_types: v["validator"]["pub_key_types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        version: Some(VersionParams {
            app: u64::try_from(num(&v["version"]["app"])).unwrap_or_default(),
        }),
        abci: Some(AbciParams {
            vote_extensions_enable_height: num(&v["abci"]["vote_extensions_enable_height"]),
        }),
    }
}

/// The validator set of `height` (all pages), in set order: the latest
/// fetched set at or below `height` (fetch-chain.py stores a set only where
/// it changes). `cache` holds the last set read and its height.
fn validators(
    dir: &Path,
    height: i64,
    cache: &mut (i64, Vec<(Vec<u8>, i64)>),
) -> anyhow::Result<Vec<(Vec<u8>, i64)>> {
    let mut h = height;
    while h > cache.0 && !dir.join("validators").join(format!("{h}.json")).exists() {
        h -= 1;
    }
    if h == cache.0 && !cache.1.is_empty() {
        return Ok(cache.1.clone());
    }
    if h < 1 {
        bail!("no validator set at or below {height}");
    }
    let set = read_validators(dir, h)?;
    *cache = (h, set.clone());
    Ok(set)
}

fn read_validators(dir: &Path, height: i64) -> anyhow::Result<Vec<(Vec<u8>, i64)>> {
    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let name = if page == 1 {
            format!("{height}.json")
        } else {
            format!("{height}.p{page}.json")
        };
        let path = dir.join("validators").join(name);
        if !path.exists() {
            break;
        }
        for v in read(&path)?["result"]["validators"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            out.push((hexb(&v["address"]), num(&v["voting_power"])));
        }
        page += 1;
    }
    Ok(out)
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow!("usage: genesis_replay <dir> [last-height]"))?,
    );
    let last: i64 = args.next().map_or(Ok(i64::MAX), |s| s.parse())?;
    let genesis = read(&dir.join("genesis.json"))?;
    let b64 = base64::engine::general_purpose::STANDARD;

    let mut app = app::App::new();
    let init = RequestInitChain {
        time: Some(timestamp(
            genesis["genesis_time"].as_str().unwrap_or_default(),
        )?),
        chain_id: genesis["chain_id"].as_str().unwrap_or_default().to_owned(),
        consensus_params: Some(consensus_params(&genesis["consensus_params"])),
        validators: Vec::new(),
        app_state_bytes: serde_json::to_vec(&genesis["app_state"])?,
        initial_height: num(&genesis["initial_height"]).max(1),
    };
    if genesis["validators"]
        .as_array()
        .is_some_and(|v| !v.is_empty())
    {
        bail!("genesis validators are not supported (gentx genesis only)");
    }
    app.init_chain(init)
        .map_err(|e| anyhow!("InitChain: {}", e.log))?;

    let (mut heights, mut txs) = (0, 0);
    let mut val_cache: (i64, Vec<(Vec<u8>, i64)>) = (0, Vec::new());
    for h in 1.. {
        if h > last {
            break;
        }
        let (Ok(block), Ok(next)) = (
            read(&dir.join("block").join(format!("{h}.json"))),
            read(&dir.join("block").join(format!("{}.json", h + 1))),
        ) else {
            break;
        };
        // Fetched only for heights with txs (fetch-chain.py).
        let res_path = dir.join("results").join(format!("{h}.json"));
        let res = if res_path.exists() {
            read(&res_path)?
        } else {
            Value::Null
        };
        let b = &block["result"]["block"];
        let hdr = &b["header"];
        if b["evidence"]["evidence"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
        {
            bail!("height {h}: blocks with evidence are not supported");
        }
        let decided_last_commit = if h > 1 {
            let vals = validators(&dir, h - 1, &mut val_cache)?;
            let sigs = b["last_commit"]["signatures"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if sigs.len() != vals.len() {
                bail!(
                    "height {h}: {} signatures for {} validators",
                    sigs.len(),
                    vals.len()
                );
            }
            Some(CommitInfo {
                round: i32::try_from(num(&b["last_commit"]["round"]))?,
                votes: vals
                    .into_iter()
                    .zip(sigs)
                    .map(|((address, power), s)| VoteInfo {
                        validator: Some(Validator { address, power }),
                        block_id_flag: i32::try_from(num(&s["block_id_flag"])).unwrap_or_default(),
                    })
                    .collect(),
            })
        } else {
            Some(CommitInfo::default())
        };
        let block_txs: Vec<Vec<u8>> = b["data"]["txs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| b64.decode(t))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let req = RequestFinalizeBlock {
            txs: block_txs.clone(),
            decided_last_commit,
            misbehavior: Vec::new(),
            hash: hexb(&block["result"]["block_id"]["hash"]),
            height: h,
            time: Some(timestamp(hdr["time"].as_str().unwrap_or_default())?),
            next_validators_hash: hexb(&hdr["next_validators_hash"]),
            proposer_address: hexb(&hdr["proposer_address"]),
            header: None,
        };
        let out = app
            .finalize_block(req)
            .map_err(|e| anyhow!("height {h}: FinalizeBlock: {}", e.log))?;
        let want = hexb(&next["result"]["block"]["header"]["app_hash"]);
        let go_results = res["result"]["txs_results"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (i, (ours, go)) in out.tx_results.iter().zip(&go_results).enumerate() {
            let go_code = u32::try_from(num(&go["code"])).unwrap_or_default();
            if ours.code != go_code
                || ours.gas_used != num(&go["gas_used"])
                || ours.gas_wanted != num(&go["gas_wanted"])
            {
                bail!(
                    "height {h} tx {i}: rust code={} gas={}/{} log={:?}, network code={go_code} gas={}/{} log={:?}",
                    ours.code,
                    ours.gas_used,
                    ours.gas_wanted,
                    ours.log,
                    num(&go["gas_used"]),
                    num(&go["gas_wanted"]),
                    go["log"]
                );
            }
        }
        if out.tx_results.len() != go_results.len() {
            bail!(
                "height {h}: {} tx results, network {}",
                out.tx_results.len(),
                go_results.len()
            );
        }
        if out.app_hash != want {
            bail!(
                "height {h}: app hash {} != network {}",
                hex::encode(&out.app_hash),
                hex::encode(&want)
            );
        }
        app.commit()
            .map_err(|e| anyhow!("height {h}: Commit: {}", e.log))?;
        heights += 1;
        txs += block_txs.len();
    }
    println!("{heights} heights, {txs} txs: every app hash and tx result matches");
    Ok(())
}
