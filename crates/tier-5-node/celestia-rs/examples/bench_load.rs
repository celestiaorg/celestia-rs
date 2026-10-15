//! Heavy-chain load generator for the B1 benchmarks
//! (`conformance/bench/gen-chain.sh`).
//!
//! On every new height it submits one round of txs signed locally
//! (SIGN_MODE_DIRECT, deterministic keys from fixed seeds):
//!
//! - `--bank` bank sends from distinct accounts to fresh random addresses
//!   (each creates an account, so state grows like a busy chain);
//! - `--pfbs` small PFBs (1 blob of 100 B .. 1 KB);
//! - every `--big-every` heights a near-max round of big PFBs, alternating
//!   4 x 1.9 MB (128x128 square) and 4 x 7.3 MB (256x256 square).
//!
//! celestia-app caps a block at 600 SDK messages + 200 PFBs
//! (`appconsts.MaxSDKMessages`, `MaxPFBMessages`), so the defaults (600 +
//! 180) fill blocks to ~780 txs.
//!
//! usage: `bench_load --print-addresses [--accounts N]`
//!        `bench_load --node URL --chain-id ID --heights H [--bank N] [--pfbs N] [--big-every N] [--accounts N]`
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::tx::v1beta1::Fee;
use celestia_rs::txs;
use rand::{Rng, RngCore, SeedableRng};
use serde_json::{Value, json};

const PFB_ACCOUNTS: usize = 400;
const BIG_ACCOUNTS: usize = 8;

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

fn seeds(accounts: usize) -> Vec<String> {
    let mut v: Vec<String> = (0..accounts).map(|k| format!("bench-bank-{k}")).collect();
    v.extend((0..PFB_ACCOUNTS).map(|k| format!("bench-pfb-{k}")));
    v.extend((0..BIG_ACCOUNTS).map(|k| format!("bench-big-{k}")));
    v
}

/// One JSON-RPC POST (a single call or a batch array).
fn post(hostport: &str, body: &Value) -> anyhow::Result<Value> {
    let body = body.to_string();
    let mut s = TcpStream::connect(hostport)?;
    s.set_read_timeout(Some(Duration::from_secs(120)))?;
    write!(
        s,
        "POST / HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body.as_bytes())?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp)?;
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("bad HTTP response"))?;
    let head = String::from_utf8_lossy(&resp[..split]).to_lowercase();
    let mut payload = resp[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut i = 0;
        while i < payload.len() {
            let end = payload[i..]
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or_else(|| anyhow!("bad chunk"))?;
            let n =
                usize::from_str_radix(String::from_utf8_lossy(&payload[i..i + end]).trim(), 16)?;
            i += end + 2;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&payload[i..i + n]);
            i += n + 2;
        }
        payload = out;
    }
    serde_json::from_slice(&payload).context("decode JSON-RPC response")
}

struct Acct {
    key: keyring::KeyRecord,
    addr: String,
    number: u64,
    seq: u64,
}

/// Signs with a fee of `price_milli` / 1000 utia per gas.
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
    let gas = txs::pfb_gas(&msg) * 12 / 10;
    sign(
        a,
        chain_id,
        vec![Any::pack(&msg)],
        gas,
        price_milli,
        Some(&[blob]),
    )
}

/// Broadcasts `txs` (`broadcast_tx_sync`, JSON-RPC batches of `chunk`) and
/// returns the accepted count; fixes sequences from "expected N" errors.
fn broadcast(
    hostport: &str,
    txs: &[(usize, Vec<u8>)],
    accts: &mut [Acct],
    chunk: usize,
) -> anyhow::Result<(usize, usize)> {
    let (mut ok, mut bad) = (0, 0);
    for part in txs.chunks(chunk) {
        let batch: Vec<Value> = part
            .iter()
            .enumerate()
            .map(|(i, (_, tx))| {
                json!({"jsonrpc": "2.0", "id": i, "method": "broadcast_tx_sync",
                       "params": {"tx": B64.encode(tx)}})
            })
            .collect();
        let resp = post(hostport, &Value::Array(batch))?;
        let arr = match resp {
            Value::Array(a) => a,
            other => vec![other],
        };
        for r in arr {
            let id = r["id"].as_u64().unwrap_or(0) as usize;
            let Some((ai, _)) = part.get(id) else {
                continue;
            };
            let code = r["result"]["code"].as_u64().unwrap_or(u64::MAX);
            if code == 0 {
                accts[*ai].seq += 1;
                ok += 1;
            } else {
                bad += 1;
                let log = r["result"]["log"].as_str().unwrap_or_default().to_owned()
                    + r["error"]["data"].as_str().unwrap_or_default();
                if let Some(n) = log
                    .split("expected ")
                    .nth(1)
                    .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|s| s.parse().ok())
                {
                    accts[*ai].seq = n;
                } else if bad < 5 {
                    eprintln!("rejected (code {code}): {}", &log[..log.len().min(200)]);
                }
            }
        }
    }
    Ok((ok, bad))
}

/// [`broadcast`] split over `threads` connections (distinct accounts per
/// round, so sequences do not interleave).
fn broadcast_parallel(
    hostport: &str,
    txs: &[(usize, Vec<u8>)],
    accts: &mut [Acct],
    threads: usize,
) -> anyhow::Result<(usize, usize)> {
    let part = txs.len().div_ceil(threads.max(1)).max(1);
    let seqs: Vec<u64> = accts.iter().map(|a| a.seq).collect();
    let results: Vec<anyhow::Result<Vec<(usize, u64)>>> = std::thread::scope(|s| {
        let hs: Vec<_> = txs
            .chunks(part)
            .map(|chunk| {
                let seqs = &seqs;
                s.spawn(move || {
                    // Per-thread scratch accounts carrying only the sequence.
                    let mut local: Vec<Acct> = Vec::new();
                    let mut map = Vec::new();
                    let mut sub = Vec::new();
                    for (ai, tx) in chunk {
                        map.push(*ai);
                        local.push(Acct {
                            key: keyring::KeyRecord {
                                name: String::new(),
                                pub_key: [0; 33],
                                secret: None,
                                kind: keyring::KeyKind::Local,
                            },
                            addr: String::new(),
                            number: 0,
                            seq: seqs[*ai],
                        });
                        sub.push((local.len() - 1, tx.clone()));
                    }
                    broadcast(hostport, &sub, &mut local, 100)?;
                    Ok(map
                        .into_iter()
                        .zip(local.into_iter().map(|a| a.seq))
                        .collect())
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow!("broadcast thread panicked")))
            })
            .collect()
    });
    let (mut ok, mut bad) = (0, 0);
    for r in results {
        for (ai, seq) in r? {
            if seq > accts[ai].seq {
                ok += 1;
            } else {
                bad += 1;
            }
            accts[ai].seq = seq;
        }
    }
    Ok((ok, bad))
}

fn status_height(hostport: &str) -> Option<i64> {
    let r = post(
        hostport,
        &json!({"jsonrpc": "2.0", "id": 0, "method": "status", "params": {}}),
    )
    .ok()?;
    r["result"]["sync_info"]["latest_block_height"]
        .as_str()?
        .parse()
        .ok()
}

#[allow(clippy::too_many_lines)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let num = |name: &str, d: usize| flag(name).and_then(|v| v.parse().ok()).unwrap_or(d);
    let accounts = num("--accounts", 600);
    if args.iter().any(|a| a == "--print-addresses") {
        for s in seeds(accounts) {
            println!("{}", key(&s)?.bech32("celestia"));
        }
        return Ok(());
    }
    let node = flag("--node").ok_or_else(|| anyhow!("--node"))?;
    let hostport = node
        .split_once("://")
        .map_or(node.as_str(), |(_, r)| r)
        .trim_end_matches('/')
        .to_owned();
    let chain_id = flag("--chain-id")
        .ok_or_else(|| anyhow!("--chain-id"))?
        .clone();
    let heights = num("--heights", 500) as i64;
    let bank = num("--bank", 600);
    let pfbs = num("--pfbs", 180);
    let big_every = num("--big-every", 20) as i64;
    let rpc = txs::Node::new(node);
    let mut accts = Vec::new();
    for s in seeds(accounts) {
        let key = key(&s)?;
        let addr = key.bech32("celestia");
        let (number, seq) = rpc.account(&addr)?;
        accts.push(Acct {
            key,
            addr,
            number,
            seq,
        });
    }
    let (bank_ix, rest) = (0..accounts, accounts);
    let pfb_ix = rest..rest + PFB_ACCOUNTS;
    let big_ix = rest + PFB_ACCOUNTS..rest + PFB_ACCOUNTS + BIG_ACCOUNTS;
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let mut last = status_height(&hostport).unwrap_or(0);
    let start = last;
    let (mut pfb_next, mut big_round) = (0usize, 0usize);
    let (mut sent, mut rejected) = (0usize, 0usize);
    let t0 = Instant::now();
    while last < start + heights {
        let h = status_height(&hostport).unwrap_or(last);
        if h == last {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        last = h;
        let mut round: Vec<(usize, Vec<u8>)> = Vec::new();
        for ai in bank_ix.clone().take(bank) {
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
            let ai = pfb_ix.start + pfb_next % PFB_ACCOUNTS;
            pfb_next += 1;
            let size = rng.gen_range(100..1000);
            round.push((ai, pfb(&accts[ai], &chain_id, &mut rng, size, 10)?));
        }
        let (ok, bad) = broadcast_parallel(&hostport, &round, &mut accts, 4)?;
        sent += ok;
        rejected += bad;
        if big_every > 0 && (h - start) % big_every == big_every / 2 {
            let size = if big_round % 2 == 0 {
                1_900_000
            } else {
                7_300_000
            };
            big_round += 1;
            let mut big = Vec::new();
            for ai in big_ix.clone().take(4) {
                // 10x the gas price: ahead of the small txs in the mempool.
                big.push((ai, pfb(&accts[ai], &chain_id, &mut rng, size, 100)?));
            }
            let (ok, bad) = broadcast(&hostport, &big, &mut accts, 1)?;
            sent += ok;
            rejected += bad;
            println!("height {h}: big round 4 x {size} B ({ok} accepted)");
        }
        if (h - start) % 25 == 0 {
            println!(
                "height {h}: {sent} txs accepted, {rejected} rejected, {:.0}s",
                t0.elapsed().as_secs_f64()
            );
        }
    }
    println!("done: {sent} txs accepted, {rejected} rejected");
    Ok(())
}
