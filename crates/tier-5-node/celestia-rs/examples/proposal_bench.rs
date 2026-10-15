//! In-process proposal benchmark: a single-validator chain (RocksDB app
//! state, as `celestia-rs start --network local` creates it) driven
//! directly through the `App` with B1-shaped blocks (`bench_load`'s load:
//! bank sends, small PFBs and, every `--big-every` heights, 4 big PFBs
//! alternating 128x128 and 256x256 squares).
//!
//! Every height: CheckTx of every tx, PrepareProposal (`--reps` times),
//! ProcessProposal of the result (`--reps` times), FinalizeBlock, Commit.
//! Prints per-height timings (median of the repetitions) for comparing
//! two builds of the app on the same machine; nothing here talks to Go.
//!
//! usage: `proposal_bench [--heights 6] [--big-every 2] [--bank 600]
//!        [--pfbs 180] [--reps 3] [--home DIR] [--keep]`
//!        `proposal_bench --open-only --home DIR` (time to reopen a kept state)
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
use std::time::{Duration, Instant};

use anyhow::anyhow;
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::tx::v1beta1::Fee;
use celestia_rs::{genesis, local, txs};
use rand::{Rng, RngCore, SeedableRng};
use serde_json::{Value, json};

const PFB_ACCOUNTS: usize = 400;
const BIG_ACCOUNTS: usize = 8;

struct Acct {
    key: keyring::KeyRecord,
    addr: String,
    number: u64,
    seq: u64,
}

fn key(seed: &str) -> anyhow::Result<keyring::KeyRecord> {
    let secret = hash::sha256(seed.as_bytes());
    let pub_key =
        crypto::secp256k1::public_key_from_secret_key(&secret).map_err(|e| anyhow!("{e}"))?;
    Ok(keyring::KeyRecord {
        name: seed.to_owned(),
        pub_key,
        secret: Some(secret),
        kind: keyring::KeyKind::Local,
    })
}

fn sign(
    a: &Acct,
    chain_id: &str,
    msgs: Vec<Any>,
    gas: u64,
    price_milli: u64,
    blobs: Option<&[shares::Blob]>,
) -> anyhow::Result<Vec<u8>> {
    let utx = txs::UnsignedTx {
        msgs,
        memo: String::new(),
        timeout_height: 0,
        fee: Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: (gas * price_milli / 1000 + 1).to_string(),
            }],
            gas_limit: gas,
            payer: String::new(),
            granter: String::new(),
        },
        chain_id: chain_id.to_owned(),
        account_number: a.number,
        sequence: a.seq,
    };
    let signed = txs::sign(&utx, &a.key)?;
    Ok(match blobs {
        Some(b) => txs::blob_tx(&signed.raw, b),
        None => signed.raw,
    })
}

fn pfb(
    a: &Acct,
    chain_id: &str,
    rng: &mut impl RngCore,
    size: usize,
    price_milli: u64,
) -> anyhow::Result<Vec<u8>> {
    let mut data = vec![0u8; size];
    rng.fill_bytes(&mut data);
    let ns = format!("{:08x}", rng.next_u32() | 1);
    let blob = txs::blob_v0(&ns, data)?;
    let msg = txs::msg_pay_for_blobs(&a.addr, std::slice::from_ref(&blob))?;
    // Blob gas (8 per share byte) on top of the tx size cost (10 per
    // byte), which `pfb_gas` leaves out for big blobs.
    let shares = (size / 478 + 2) as u64;
    let gas = (txs::pfb_gas(&msg) * 12 / 10).max(shares * 512 * 8 + size as u64 * 12 + 200_000);
    sign(
        a,
        chain_id,
        vec![Any::pack(&msg)],
        gas,
        price_milli,
        Some(&[blob]),
    )
}

fn median(mut v: Vec<Duration>) -> f64 {
    v.sort();
    v[v.len() / 2].as_secs_f64() * 1e3
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let num = |name: &str, d: usize| flag(name).and_then(|v| v.parse().ok()).unwrap_or(d);
    let heights = num("--heights", 6) as i64;
    let big_every = num("--big-every", 2) as i64;
    let bank = num("--bank", 600);
    let pfbs = num("--pfbs", 180);
    let repeats = num("--reps", 3).max(1);
    let home = flag("--home").map_or_else(
        || std::env::temp_dir().join(format!("proposal-bench-{}", std::process::id())),
        std::path::PathBuf::from,
    );
    if args.iter().any(|a| a == "--open-only") {
        // Restart cost: open the app state a previous `--keep` run left.
        let t = Instant::now();
        let app = app::App::open(&home)?;
        println!(
            "opened app state at height {} in {:.0} ms",
            app.info().last_block_height,
            t.elapsed().as_secs_f64() * 1e3
        );
        return Ok(());
    }
    let keep = args.iter().any(|a| a == "--keep");
    let _ = std::fs::remove_dir_all(&home);

    // Genesis: the local single-validator devnet plus the load accounts.
    local::setup(&home)?;
    let gfile = home.join("config").join("genesis.json");
    let mut g = genesis::read(&gfile)?;
    let seeds: Vec<String> = (0..bank)
        .map(|k| format!("bench-bank-{k}"))
        .chain((0..PFB_ACCOUNTS).map(|k| format!("bench-pfb-{k}")))
        .chain((0..BIG_ACCOUNTS).map(|k| format!("bench-big-{k}")))
        .collect();
    let amount: u64 = 100_000_000_000_000;
    let first_number = g["app_state"]["auth"]["accounts"]
        .as_array()
        .map_or(0, Vec::len) as u64;
    let mut accts = Vec::new();
    for (i, s) in seeds.iter().enumerate() {
        let key = key(s)?;
        let addr = key.bech32("celestia");
        let number = first_number + i as u64;
        g["app_state"]["auth"]["accounts"]
            .as_array_mut()
            .ok_or_else(|| anyhow!("auth accounts"))?
            .push(
                json!({"@type": "/cosmos.auth.v1beta1.BaseAccount", "address": addr,
                         "pub_key": null, "account_number": number.to_string(), "sequence": "0"}),
            );
        g["app_state"]["bank"]["balances"]
            .as_array_mut()
            .ok_or_else(|| anyhow!("bank balances"))?
            .push(json!({"address": addr, "coins": [{"denom": "utia", "amount": amount.to_string()}]}));
        accts.push(Acct {
            key,
            addr,
            number,
            seq: 0,
        });
    }
    if let Some(supply) = g["app_state"]["bank"]["supply"].as_array_mut()
        && !supply.is_empty()
    {
        for c in supply.iter_mut() {
            if c["denom"] == "utia" {
                let cur: u128 = c["amount"].as_str().unwrap_or("0").parse()?;
                c["amount"] =
                    Value::String((cur + u128::from(amount) * seeds.len() as u128).to_string());
            }
        }
    }
    // No block gas limit: the big PFB rounds come on top of full blocks.
    for path in ["/consensus/params/block", "/consensus_params/block"] {
        if let Some(block) = g.pointer_mut(path) {
            block["max_gas"] = Value::String("-1".into());
        }
    }
    genesis::write(&gfile, &g)?;

    let bz = std::fs::read(&gfile)?;
    let doc = nodestore::GenesisDoc::from_json(&bz, consensus::ticker::system_now())
        .map_err(|e| anyhow!("{e}"))?;
    let chain_id = doc.chain_id.clone();
    let mut app = app::App::open(&home)?;
    let init = app
        .init_chain(abci::RequestInitChain {
            time: Some(doc.genesis_time.to_proto()),
            chain_id: chain_id.clone(),
            consensus_params: Some(doc.consensus_params.to_proto()),
            validators: Vec::new(),
            app_state_bytes: doc.app_state.clone(),
            initial_height: doc.initial_height,
        })
        .map_err(|e| anyhow!("InitChain: {}", e.log))?;
    let proposer = init
        .validators
        .first()
        .and_then(|v| v.pub_key.as_ref())
        .and_then(|k| match &k.sum {
            Some(
                celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum::Ed25519(
                    b,
                ),
            ) => Some(hash::sha256(b)[..20].to_vec()),
            _ => None,
        })
        .ok_or_else(|| anyhow!("no genesis validator"))?;
    let max_tx_bytes = doc.consensus_params.block.max_bytes;
    let genesis_secs = doc.genesis_time.to_proto().seconds;

    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let (bank_ix, pfb_start, big_start) = (0..bank, bank, bank + PFB_ACCOUNTS);
    let (mut pfb_next, mut big_round) = (0usize, 0usize);
    println!(
        "height  txs  bytes(MB)  square  checktx(ms)  prepare(ms)  process(ms)  finalize(ms)  commit(ms)"
    );
    for h in doc.initial_height..doc.initial_height + heights {
        let mut round: Vec<(usize, Vec<u8>)> = Vec::new();
        if big_every > 0 && h % big_every == 0 {
            let size = if big_round % 2 == 0 {
                1_900_000
            } else {
                7_300_000
            };
            big_round += 1;
            for (ai, a) in accts.iter().enumerate().skip(big_start).take(4) {
                round.push((ai, pfb(a, &chain_id, &mut rng, size, 100)?));
            }
        }
        for ai in bank_ix.clone() {
            let mut to = [0u8; 20];
            rng.fill_bytes(&mut to);
            let a = &accts[ai];
            let msg = celestia_proto::cosmos::bank::v1beta1::MsgSend {
                from_address: a.addr.clone(),
                to_address: encoding::bech32::encode("celestia", &to).unwrap_or_default(),
                amount: vec![Coin {
                    denom: "utia".into(),
                    amount: rng.gen_range(1..1000u32).to_string(),
                }],
            };
            round.push((
                ai,
                sign(a, &chain_id, vec![Any::pack(&msg)], 120_000, 10, None)?,
            ));
        }
        for _ in 0..pfbs {
            let ai = pfb_start + pfb_next % PFB_ACCOUNTS;
            pfb_next += 1;
            let size = rng.gen_range(100..1000);
            round.push((ai, pfb(&accts[ai], &chain_id, &mut rng, size, 10)?));
        }
        let txs: Vec<Vec<u8>> = round.iter().map(|(_, t)| t.clone()).collect();
        let bytes: usize = txs.iter().map(Vec::len).sum();

        let t = Instant::now();
        // CheckTx runs on committed state: none before the first commit.
        for tx in txs.iter().filter(|_| h > doc.initial_height) {
            let res = app
                .check_tx(&abci::RequestCheckTx {
                    tx: tx.clone(),
                    r#type: 0,
                })
                .map_err(|e| anyhow!("CheckTx: {e}"))?;
            if res.code != 0 {
                eprintln!("CheckTx rejected: {}", res.log);
            }
        }
        let check = t.elapsed();

        let mut time = doc.genesis_time.to_proto();
        time.seconds = genesis_secs + h * 6;
        let time = Some(time);
        let prep_req = abci::RequestPrepareProposal {
            max_tx_bytes,
            txs: txs.clone(),
            height: h,
            time,
            proposer_address: proposer.clone(),
            ..Default::default()
        };
        let mut prep_t = Vec::new();
        let mut prepared = None;
        for _ in 0..repeats {
            let t = Instant::now();
            let res = app
                .prepare_proposal(&prep_req)
                .map_err(|e| anyhow!("PrepareProposal: {}", e.log))?;
            prep_t.push(t.elapsed());
            prepared = Some(res);
        }
        let prepared = prepared.ok_or_else(|| anyhow!("no proposal"))?;
        let proc_req = abci::RequestProcessProposal {
            txs: prepared.txs.clone(),
            height: h,
            time,
            proposer_address: proposer.clone(),
            square_size: prepared.square_size,
            data_root_hash: prepared.data_root_hash.clone(),
            ..Default::default()
        };
        let mut proc_t = Vec::new();
        for _ in 0..repeats {
            let t = Instant::now();
            let res = app
                .process_proposal(&proc_req)
                .map_err(|e| anyhow!("ProcessProposal: {}", e.log))?;
            proc_t.push(t.elapsed());
            if res.status != abci::response_process_proposal::ProposalStatus::Accept as i32 {
                return Err(anyhow!("height {h}: own proposal rejected"));
            }
        }
        let t = Instant::now();
        let fin = app
            .finalize_block(abci::RequestFinalizeBlock {
                txs: prepared.txs.clone(),
                height: h,
                time,
                proposer_address: proposer.clone(),
                hash: hash::sha256(&h.to_be_bytes()).to_vec(),
                ..Default::default()
            })
            .map_err(|e| anyhow!("FinalizeBlock: {}", e.log))?;
        let finalize = t.elapsed();
        let t = Instant::now();
        app.commit().map_err(|e| anyhow!("Commit: {}", e.log))?;
        let commit = t.elapsed();

        // Sequences advance for every included tx that passed the ante
        // handler (the load's txs only fail there, if at all).
        let ok: std::collections::HashSet<&[u8]> = prepared
            .txs
            .iter()
            .zip(&fin.tx_results)
            .filter(|(_, r)| r.code == 0)
            .map(|(t, _)| t.as_slice())
            .collect();
        for (ai, tx) in &round {
            if ok.contains(tx.as_slice()) {
                accts[*ai].seq += 1;
            }
        }
        let failed: Vec<_> = fin.tx_results.iter().filter(|r| r.code != 0).collect();
        if let Some(first) = failed.first() {
            eprintln!(
                "height {h}: {} txs failed, first: {}",
                failed.len(),
                first.log
            );
        }
        println!(
            "{h:>6} {:>4} {:>10.1} {:>7} {:>12.1} {:>12.1} {:>12.1} {:>13.1} {:>11.1}",
            prepared.txs.len(),
            bytes as f64 / 1e6,
            prepared.square_size,
            check.as_secs_f64() * 1e3,
            median(prep_t),
            median(proc_t),
            finalize.as_secs_f64() * 1e3,
            commit.as_secs_f64() * 1e3,
        );
    }
    drop(app);
    if !keep {
        let _ = std::fs::remove_dir_all(&home);
    }
    Ok(())
}
