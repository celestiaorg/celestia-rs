//! Differential fuzzer driver.
//!
//! ```text
//! fuzz-diff [--app-version 10] [--ops tx,blobtx,iw,amino_dec,amino_enc]
//!           [--iters N] [--max-secs S] [--seed S] [--findings DIR]
//!           [--corpus DIR]...
//! ```
//!
//! Seeds every op with the real-tx corpus (devnet fixtures, txdecode and
//! amino vectors, plus every file of each `--corpus` directory, e.g. a
//! cargo-fuzz corpus), then sends batches of mutated inputs to the Go oracle
//! and to the Rust code. Each distinct mismatch is printed and written to
//! `DIR/<n>.json` (`{app_version, op, input, go, rust, diff}`); the exit
//! status is 1 if any was found.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::cast_possible_truncation
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use fuzz_diff::mutate::Mutator;
use fuzz_diff::oracle::Oracle;
use fuzz_diff::{compare, corpus, rust_out};

struct Args {
    app_version: u64,
    ops: Vec<String>,
    iters: u64,
    max_secs: u64,
    seed: u64,
    findings: Option<PathBuf>,
    corpus_dirs: Vec<PathBuf>,
    batch: usize,
}

fn args() -> anyhow::Result<Args> {
    let mut a = Args {
        app_version: 10,
        ops: vec!["tx".into(), "blobtx".into(), "iw".into()],
        iters: 100_000,
        max_secs: 0,
        seed: 1,
        findings: None,
        corpus_dirs: Vec::new(),
        batch: 256,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().context("missing value");
        match k.as_str() {
            "--app-version" => a.app_version = v()?.parse()?,
            "--ops" => a.ops = v()?.split(',').map(str::to_owned).collect(),
            "--iters" => a.iters = v()?.parse()?,
            "--max-secs" => a.max_secs = v()?.parse()?,
            "--seed" => a.seed = v()?.parse()?,
            "--findings" => a.findings = Some(v()?.into()),
            "--corpus" => a.corpus_dirs.push(v()?.into()),
            "--batch" => a.batch = v()?.parse()?,
            _ => anyhow::bail!("unknown argument {k}"),
        }
    }
    Ok(a)
}

/// A mismatch signature: the first line with hex runs and numbers removed,
/// so one bug is reported once.
fn signature(op: &str, diff: &str) -> String {
    let first = diff.lines().next().unwrap_or_default();
    let mut s = String::new();
    let mut prev_digit = false;
    for c in first.chars() {
        let d = c.is_ascii_hexdigit() && !c.is_ascii_alphabetic() || c.is_ascii_digit();
        if d {
            if !prev_digit {
                s.push('#');
            }
        } else {
            s.push(c);
        }
        prev_digit = d;
    }
    format!("{op}: {}", s.chars().take(160).collect::<String>())
}

/// `fuzz-diff record <app version> <note> < lines`: each stdin line is
/// `<op> <hex input>`; prints one regression entry per line (JSON with the
/// Go outcome) for `tests/regressions.jsonl`, and reports on stderr whether
/// Rust currently agrees.
fn record(app_version: u64, note: &str) -> anyhow::Result<()> {
    let mut oracle = Oracle::start(app_version)?;
    for line in std::io::stdin().lines() {
        let line = line?;
        let Some((op, h)) = line.trim().split_once(' ') else {
            continue;
        };
        let input = hex::decode(h)?;
        let go = oracle.query(op, &input)?;
        let rust = rust_out(app_version, op, &input);
        match compare(op, &go, &rust) {
            Some(d) => eprintln!("MISMATCH {op}: {d}"),
            None => eprintln!("agree {op}"),
        }
        let j = serde_json::json!({
            "app_version": app_version,
            "op": op,
            "note": note,
            "input": h,
            "go": go,
        });
        println!("{}", serde_json::to_string(&j)?);
    }
    Ok(())
}

/// `fuzz-diff craft <home> list` prints `<case> <signer> <what>` per
/// workload tx; `fuzz-diff craft <home> <chain id> <case> <account number>
/// <sequence>` prints the signed tx (hex) of `case` (see
/// `fuzz_diff::craft`, `conformance/devnet/workloads/utf8.sh`).
fn craft(args: &[String]) -> anyhow::Result<()> {
    use fuzz_diff::craft::{Accounts, cases, sign};
    let home = PathBuf::from(args.first().context("home")?);
    let mut kr = keyring::Keyring::new(&home, keyring::Backend::Test);
    let val0 = kr.key("val0").map_err(|e| anyhow::anyhow!("{e}"))?;
    let user0 = kr.key("user0").map_err(|e| anyhow::anyhow!("{e}"))?;
    let a = Accounts {
        val0: val0.bech32("celestia"),
        val0_oper: val0.bech32("celestiavaloper"),
        user0: user0.bech32("celestia"),
    };
    let cs = cases(&a);
    if args.get(1).map(String::as_str) == Some("list") {
        for (i, c) in cs.iter().enumerate() {
            println!("{i} {} {}", c.signer, c.what);
        }
        return Ok(());
    }
    let chain_id = args.get(1).context("chain id")?;
    let i: usize = args.get(2).context("case")?.parse()?;
    let acc: u64 = args.get(3).context("account number")?.parse()?;
    let seq: u64 = args.get(4).context("sequence")?.parse()?;
    let c = cs.get(i).context("no such case")?;
    let key = if c.signer == "val0" { &val0 } else { &user0 };
    println!("{}", hex::encode(sign(c, key, chain_id, acc, seq)?));
    Ok(())
}

/// `fuzz-diff seed-corpus <dir>`: small seed corpora for the cargo-fuzz
/// targets (`fuzz/corpus/<target>/`) from the real-tx corpus: distinct txs
/// (one per message-type mix and wrapper kind), the amino vectors and a
/// few devnet blocks as `square` inputs.
fn seed_corpus(dir: &std::path::Path) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    let write = |target: &str, name: &str, data: &[u8]| -> anyhow::Result<()> {
        let d = dir.join(target);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join(name), data)?;
        Ok(())
    };
    // One tx per (wrapper, message type URLs) shape, smallest first.
    let mut shapes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut txs = corpus::devnet_txs();
    txs.extend(corpus::txdecode_vectors());
    txs.sort_by_key(Vec::len);
    for tx in txs {
        if tx.len() > 20_000 {
            continue;
        }
        let key = match txdecode::decode(&tx).or_else(|_| txdecode::decode_v9(&tx)) {
            Ok(t) => format!(
                "{}{}{}",
                !t.blobs.is_empty(),
                !t.share_indexes.is_empty(),
                t.msgs()
                    .iter()
                    .map(|m| m.type_url.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Err(e) => format!("err {} {}", e.codespace, e.code),
        };
        shapes.entry(key).or_insert(tx);
    }
    for (i, tx) in shapes.values().enumerate() {
        write("txdecode", &format!("tx{i:03}"), tx)?;
    }
    for (i, (op, input)) in corpus::amino_vectors().iter().enumerate() {
        write("amino", &format!("{op}{i:03}"), input)?;
    }
    for (i, (_, txs)) in corpus::devnet_blocks()
        .iter()
        .filter(|(_, t)| t.iter().map(Vec::len).sum::<usize>() < 50_000)
        .take(16)
        .enumerate()
    {
        write(
            "square",
            &format!("block{i:02}"),
            &fuzz_diff::eds_input(txs, 0),
        )?;
    }
    eprintln!("{} tx shapes", shapes.len());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("seed-corpus") {
        return seed_corpus(&PathBuf::from(argv.get(2).context("output dir")?));
    }
    if argv.get(1).map(String::as_str) == Some("pp") {
        let scenario = argv.get(2).context("scenario")?;
        let addr = argv.get(3).context("go abci address")?;
        let per_height = argv.get(4).map_or(Ok(20), |s| s.parse())?;
        let seed = argv.get(5).map_or(Ok(1), |s| s.parse())?;
        let st = fuzz_diff::pp::run(scenario, addr, per_height, seed)?;
        for m in &st.mismatches {
            println!("MISMATCH {m}");
        }
        eprintln!(
            "{scenario}: {} mutated proposals, Go accepted {}, {} mismatches",
            st.compared,
            st.go_accepted,
            st.mismatches.len()
        );
        if !st.mismatches.is_empty() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if argv.get(1).map(String::as_str) == Some("craft") {
        return craft(&argv[2..]);
    }
    if argv.get(1).map(String::as_str) == Some("exec") {
        let scenario = argv.get(2).context("scenario")?;
        let addr = argv.get(3).context("go abci address")?;
        let grpc = argv.get(4).context("go grpc address")?;
        let per_height = argv.get(5).map_or(Ok(20), |s| s.parse())?;
        let seed = argv.get(6).map_or(Ok(1), |s| s.parse())?;
        let st = fuzz_diff::exec::run(scenario, addr, grpc, per_height, seed)?;
        for m in &st.mismatches {
            println!("MISMATCH {m}");
        }
        for m in &st.log_mismatches {
            println!("LOG {m}");
        }
        eprintln!(
            "{scenario}: {} simulated txs, Go executed {}; {} with invalid UTF-8 ({} executed); {} mismatches, {} error-text-only differences",
            st.compared,
            st.go_ok,
            st.invalid_utf8,
            st.invalid_utf8_ok,
            st.mismatches.len(),
            st.log_mismatches.len()
        );
        if !st.mismatches.is_empty() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if argv.get(1).map(String::as_str) == Some("record") {
        let av = argv.get(2).context("app version")?.parse()?;
        return record(av, argv.get(3).map_or("", String::as_str));
    }
    let a = args()?;
    let mut oracle = Oracle::start(a.app_version)?;
    let mut m = Mutator::new(a.seed);

    // Seeds per op.
    let mut tx_seeds = corpus::devnet_txs();
    tx_seeds.extend(corpus::txdecode_vectors());
    for d in &a.corpus_dirs {
        for e in std::fs::read_dir(d).with_context(|| d.display().to_string())? {
            tx_seeds.push(std::fs::read(e?.path())?);
        }
    }
    tx_seeds.sort();
    tx_seeds.dedup();
    let amino = corpus::amino_vectors();
    for s in &tx_seeds {
        m.harvest(s);
    }
    let seeds_for = |op: &str| -> Vec<Vec<u8>> {
        match op {
            "amino_dec" => amino
                .iter()
                .filter(|(o, _)| *o == "amino_dec")
                .map(|x| x.1.clone())
                .collect(),
            "amino_enc" => amino
                .iter()
                .filter(|(o, _)| *o == "amino_enc")
                .map(|x| x.1.clone())
                .collect(),
            // The first round sends each scenario's blocks as one square.
            "eds" => corpus::devnet_blocks()
                .into_iter()
                .map(|(av, txs)| (av, fuzz_diff::eds_input(&txs, 0)))
                .filter(|(av, _)| (*av < 10) == (a.app_version < 10))
                .map(|x| x.1)
                .collect(),
            _ => tx_seeds.clone(),
        }
    };
    let seeds: BTreeMap<String, Vec<Vec<u8>>> =
        a.ops.iter().map(|o| (o.clone(), seeds_for(o))).collect();
    eprintln!(
        "app version {}: {} tx seeds, {} amino seeds, ops {:?}",
        a.app_version,
        tx_seeds.len(),
        amino.len(),
        a.ops
    );

    let start = Instant::now();
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    let mut done = 0u64;
    let mut stats: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut first = true;
    while done < a.iters {
        if a.max_secs > 0 && start.elapsed() > Duration::from_secs(a.max_secs) {
            break;
        }
        // The first round replays the seeds unmutated.
        let mut batch: Vec<(String, Vec<u8>)> = Vec::new();
        if first {
            for (op, ss) in &seeds {
                for s in ss {
                    batch.push((op.clone(), s.clone()));
                }
            }
            first = false;
        } else {
            for _ in 0..a.batch {
                let op = &a.ops[m.rng.below(a.ops.len())];
                let ss = &seeds[op];
                if ss.is_empty() {
                    continue;
                }
                let s = ss[m.rng.below(ss.len())].clone();
                let k = 1 + m.rng.below(4);
                let input = if op == "eds" {
                    let x = gen_eds(&mut m, a.app_version, &tx_seeds);
                    if m.rng.one_in(4) { m.mutate(&x) } else { x }
                } else if op == "amino_enc" && !m.rng.one_in(4) {
                    json_mutate(&mut m, &s)
                } else {
                    m.mutate_n(&s, k)
                };
                batch.push((op.clone(), input));
            }
        }
        let reqs: Vec<(&str, &[u8])> = batch
            .iter()
            .map(|(o, i)| (o.as_str(), i.as_slice()))
            .collect();
        if done / fuzz_diff::oracle::RESTART_EVERY
            != (done + reqs.len() as u64) / fuzz_diff::oracle::RESTART_EVERY
        {
            oracle = Oracle::start(a.app_version)?;
        }
        let go = oracle.query_batch(&reqs)?;
        for ((op, input), g) in batch.iter().zip(&go) {
            done += 1;
            let r = rust_out(a.app_version, op, input);
            let st = stats.entry(op.clone()).or_default();
            st.0 += 1;
            if g.ok {
                st.1 += 1;
            }
            if let Some(diff) = compare(op, g, &r) {
                let sig = signature(op, &diff);
                let n = found.len();
                let e = found.entry(sig.clone()).or_insert(0);
                *e += 1;
                if *e > 1 {
                    continue;
                }
                println!(
                    "MISMATCH #{n} [{sig}]\n  input {}\n  {diff}",
                    hex::encode(input)
                );
                if let Some(dir) = &a.findings {
                    std::fs::create_dir_all(dir)?;
                    let j = serde_json::json!({
                        "app_version": a.app_version,
                        "op": op,
                        "input": hex::encode(input),
                        "go": g,
                        "rust": r,
                        "diff": diff,
                    });
                    std::fs::write(
                        dir.join(format!("v{}-{op}-{n}.json", a.app_version)),
                        serde_json::to_vec_pretty(&j)?,
                    )?;
                }
            }
        }
    }
    eprintln!(
        "done: {done} inputs in {:.0}s; per op (inputs, Go accepts): {stats:?}",
        start.elapsed().as_secs_f64()
    );
    for (sig, n) in &found {
        eprintln!("  {n:6}  {sig}");
    }
    if !found.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// Blob sizes around share boundaries (first sparse share holds 478 bytes,
/// 458 with a signer; continuation shares 482).
const EDGE_SIZES: &[usize] = &[
    1, 2, 457, 458, 459, 477, 478, 479, 481, 482, 483, 939, 940, 941, 959, 960, 961, 1023, 1024,
    4096, 30_000, 63_999, 64_000, 64_001,
];

/// A random canonical `BlobTx` around `inner`.
fn synthetic_blob_tx(m: &mut Mutator, inner: &[u8]) -> Vec<u8> {
    let n = 1 + m.rng.below(4);
    let mut blobs = Vec::new();
    for _ in 0..n {
        let size = if m.rng.one_in(2) {
            EDGE_SIZES[m.rng.below(EDGE_SIZES.len())]
        } else {
            let cap = if m.rng.one_in(8) { 400_000 } else { 3000 };
            1 + m.rng.below(cap)
        };
        let mut id = vec![0u8; 28];
        for b in &mut id[18..] {
            *b = m.rng.next_u64() as u8;
        }
        if m.rng.one_in(30) {
            id[0] = 1; // invalid version-0 namespace
        }
        let share_version = u32::from(m.rng.one_in(3));
        blobs.push(sdk::tx::Blob {
            namespace_id: id,
            namespace_version: 0,
            share_version,
            data: (0..size).map(|i| (i as u8) ^ 0x5a).collect(),
            signer: if share_version == 1 {
                vec![7u8; 20]
            } else {
                Vec::new()
            },
        });
    }
    txdecode::marshal_blob_tx(inner, &blobs)
}

/// The class rc4 / v4.0.1 square construction puts a tx in, for ordering.
fn tx_class(app_version: u64, tx: &[u8]) -> u8 {
    let blob = if app_version < 10 {
        txdecode::unmarshal_blob_tx_v9(tx).is_some()
    } else {
        txdecode::unmarshal_blob_tx(tx).is_some()
    };
    if blob {
        return 1;
    }
    let fibre = if app_version < 10 {
        app::fibre_tx::try_parse_v9(tx)
    } else {
        app::fibre_tx::try_parse(tx)
    };
    if matches!(fibre, Ok(Some(_)) | Err(_)) {
        2
    } else {
        0
    }
}

/// A random `eds` input: real txs (some mutated), synthetic blob txs and
/// random normal txs, mostly in the required order, with a random max
/// square size.
fn gen_eds(m: &mut Mutator, app_version: u64, seeds: &[Vec<u8>]) -> Vec<u8> {
    let mut txs = Vec::new();
    for _ in 0..m.rng.below(10) {
        let s = &seeds[m.rng.below(seeds.len())];
        txs.push(if m.rng.one_in(3) {
            m.mutate(s)
        } else {
            s.clone()
        });
    }
    for _ in 0..m.rng.below(4) {
        let inner = seeds[m.rng.below(seeds.len())].clone();
        txs.push(synthetic_blob_tx(m, &inner));
    }
    for _ in 0..m.rng.below(3) {
        let size = if m.rng.one_in(2) {
            EDGE_SIZES[m.rng.below(EDGE_SIZES.len())]
        } else {
            1 + m.rng.below(2000)
        };
        txs.push((0..size).map(|_| m.rng.next_u64() as u8).collect());
    }
    if !m.rng.one_in(5) {
        txs.sort_by_key(|t| tx_class(app_version, t));
    }
    let max = [0u64, 0, 0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 3, 1024][m.rng.below(15)];
    fuzz_diff::eds_input(&txs, max)
}

/// Mutates an amino JSON document at the value level: replaces a random
/// scalar with another JSON value, drops / duplicates keys, renames type
/// tags.
fn json_mutate(m: &mut Mutator, input: &[u8]) -> Vec<u8> {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(input) else {
        return m.bytes(input);
    };
    let mut count = 0usize;
    count_nodes(&v, &mut count);
    let target = m.rng.below(count.max(1));
    let mut i = 0usize;
    edit_node(m, &mut v, target, &mut i);
    serde_json::to_vec(&v).unwrap_or_else(|_| input.to_vec())
}

fn count_nodes(v: &serde_json::Value, n: &mut usize) {
    *n += 1;
    match v {
        serde_json::Value::Array(a) => a.iter().for_each(|x| count_nodes(x, n)),
        serde_json::Value::Object(o) => o.values().for_each(|x| count_nodes(x, n)),
        _ => {}
    }
}

fn edit_node(m: &mut Mutator, v: &mut serde_json::Value, target: usize, i: &mut usize) {
    use serde_json::Value as J;
    if *i == target {
        *i += 1;
        let choices: [J; 16] = [
            J::Null,
            J::Bool(true),
            J::from(0),
            J::from(-1),
            J::from(1u64 << 63),
            J::from(1.5),
            J::from(""),
            J::from("0"),
            J::from("-1"),
            J::from("18446744073709551616"),
            J::from("1.000000000000000000"),
            J::from("utia"),
            J::from("AA=="),
            J::from("celestia1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5wpwh0d"),
            J::Array(vec![]),
            J::Object(serde_json::Map::new()),
        ];
        match v {
            J::Object(o) if !o.is_empty() && m.rng.one_in(2) => {
                let keys: Vec<String> = o.keys().cloned().collect();
                let k = &keys[m.rng.below(keys.len())];
                if m.rng.one_in(2) {
                    o.remove(k);
                } else if k == "type" {
                    o.insert(k.clone(), J::from("cosmos-sdk/MsgSend"));
                } else {
                    o.insert(format!("{k}x"), J::Null);
                }
            }
            J::Array(a) if !a.is_empty() && m.rng.one_in(2) => {
                let x = a[m.rng.below(a.len())].clone();
                a.push(x);
            }
            _ => *v = choices[m.rng.below(choices.len())].clone(),
        }
        return;
    }
    *i += 1;
    match v {
        J::Array(a) => a.iter_mut().for_each(|x| edit_node(m, x, target, i)),
        J::Object(o) => o.values_mut().for_each(|x| edit_node(m, x, target, i)),
        _ => {}
    }
}
