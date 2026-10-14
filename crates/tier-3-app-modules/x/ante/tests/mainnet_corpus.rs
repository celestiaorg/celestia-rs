//! Real mainnet transactions through the v10 ante handler.
//!
//! `conformance/fixtures/ante/mainnet-tx-corpus.json.zst` holds 1,062 txs
//! sampled from Celestia mainnet with the signers' account state (account
//! number, sequence, public key, balances) and Go celestia-app **v6**'s
//! ante outcome (`conformance/antereplay`). It was replayed by the M3
//! prototype `ante` crate (removed: superseded by `x-ante`). v10 differs
//! from v6 in fees and gas (tx-size cost, utia-only fee rule, network
//! minimum gas price, param filters), so outcomes are not compared
//! wholesale; what must hold across versions is everything about the
//! *signatures*: every tx v6 accepted must pass v10 decoding and signature
//! verification (direct and legacy amino-JSON sign modes, multisig) and
//! advance the signers' sequences exactly as Go did, and every tx v6
//! rejected for a signature / sequence / decoding reason must be rejected
//! by v10 with the same code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::PathBuf;

use celestia_core_proto::tendermint::types::BlockParams;
use celestia_proto::Any;
use celestia_proto::cosmos::auth::v1beta1::{BaseAccount, Params as AuthParams};
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody, TxRaw};
use prost::Message;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};
use serde::Deserialize;
use x_ante::{StoreDeps, ante_handle};

const STORES: &[&str] = &["acc", "bank", "circuit", "minfee", "blob", "feegrant"];

/// sdkerrors codes tied to signatures, sequences and tx decoding.
const SIG_CODES: &[u32] = &[
    2,  // ErrTxDecode
    4,  // ErrUnauthorized (signature verification failed)
    8,  // ErrInvalidPubKey
    15, // ErrNoSignatures
    32, // ErrWrongSequence
];

#[derive(Deserialize)]
struct Corpus {
    chain_id: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    label: String,
    height: i64,
    mode: String,
    tx_raw_hex: String,
    #[serde(default)]
    accounts: Vec<FixtureAccount>,
    #[serde(default)]
    balances: Vec<FixtureBalance>,
    expected: Expected,
}

#[derive(Deserialize)]
struct FixtureAccount {
    address_hex: String,
    account_number: u64,
    sequence: u64,
    #[serde(default)]
    public_key_any_hex: Option<String>,
}

#[derive(Deserialize)]
struct FixtureBalance {
    address_hex: String,
    denom: String,
    amount: String,
}

#[derive(Deserialize)]
struct Expected {
    ok: bool,
    #[serde(default)]
    code: u32,
    #[serde(default)]
    codespace: String,
    #[serde(default)]
    signer_sequences: Vec<SignerSequence>,
}

#[derive(Deserialize)]
struct SignerSequence {
    address_hex: String,
    sequence: u64,
}

fn fixture() -> Corpus {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../conformance/fixtures/ante/mainnet-tx-corpus.json.zst");
    let f = std::fs::File::open(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut s = String::new();
    zstd::Decoder::new(f)
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    serde_json::from_str(&s).unwrap()
}

fn decode_tx(bz: &[u8]) -> Option<sdk::tx::Tx> {
    let raw = TxRaw::decode(bz).ok()?;
    let body = TxBody::decode(raw.body_bytes.as_slice()).ok()?;
    let auth_info = AuthInfo::decode(raw.auth_info_bytes.as_slice()).ok()?;
    Some(sdk::tx::Tx {
        tx_bytes: bz.to_vec(),
        body_bytes: raw.body_bytes,
        auth_info_bytes: raw.auth_info_bytes,
        signatures: raw.signatures,
        body,
        auth_info,
        ..Default::default()
    })
}

/// Outcome: Ok(signer sequences after the ante) or Err((code, log)).
type Outcome = Result<BTreeMap<Vec<u8>, u64>, (u32, String)>;

fn run_case(chain_id: &str, c: &Case) -> Outcome {
    let mut ms = MultiStore::new();
    for s in STORES {
        ms.mount(s, Kind::Iavl).unwrap();
    }
    let header = Header {
        chain_id: chain_id.to_owned(),
        height: c.height,
        ..Default::default()
    };
    let mode = match c.mode.as_str() {
        "check" | "CheckTx" | "checktx" => ExecMode::Check,
        _ => ExecMode::Finalize,
    };
    // Default module params (as at genesis) and the signers' state.
    {
        let mut ctx = Context::new(&mut ms, header.clone(), ExecMode::Finalize);
        x_auth::set_params(
            &mut ctx,
            &AuthParams {
                max_memo_characters: 256,
                tx_sig_limit: 7,
                tx_size_cost_per_byte: 10,
                sig_verify_cost_ed25519: 590,
                sig_verify_cost_secp256k1: 1000,
            },
        )
        .unwrap();
        x_auth::get_module_account(&mut ctx, x_auth::FEE_COLLECTOR).unwrap();
        // v10 genesis values: network minimum gas price 0.000001 utia
        // (LegacyDec, 18 decimals), blob params 8 gas/byte, 256x256.
        ctx.set(
            "minfee",
            b"params".to_vec(),
            celestia_proto::celestia::minfee::v1::Params {
                network_min_gas_price: "1000000000000".into(),
            }
            .encode_to_vec(),
        )
        .unwrap();
        ctx.set(
            "blob",
            b"params".to_vec(),
            celestia_proto::celestia::blob::v1::Params {
                gas_per_blob_byte: 8,
                gov_max_square_size: 256,
            }
            .encode_to_vec(),
        )
        .unwrap();
        for a in &c.accounts {
            let addr = hex::decode(&a.address_hex).unwrap();
            let pub_key = a
                .public_key_any_hex
                .as_deref()
                .filter(|h| !h.is_empty())
                .map(|h| Any::decode(hex::decode(h).unwrap().as_slice()).unwrap());
            x_auth::set_account(
                &mut ctx,
                &x_auth::Account::Base(BaseAccount {
                    address: x_auth::bech32(&addr).unwrap(),
                    pub_key,
                    account_number: a.account_number,
                    sequence: a.sequence,
                }),
            )
            .unwrap();
        }
        for b in &c.balances {
            let addr = hex::decode(&b.address_hex).unwrap();
            x_bank::set_balance(
                &mut ctx,
                &addr,
                &sdk::coins::Coin::with_amount(b.denom.clone(), b.amount.parse().unwrap()),
            )
            .unwrap();
        }
    }
    ms.flush().unwrap();
    ms.commit().unwrap();
    let bz = hex::decode(&c.tx_raw_hex).unwrap();
    let Some(tx) = decode_tx(&bz) else {
        return Err((2, "tx decode".into()));
    };
    let deps = StoreDeps::new(None);
    let mut ctx = Context::new(&mut ms, header, mode);
    ctx.tx_bytes.clone_from(&bz);
    ctx.consensus_params.block = Some(BlockParams {
        max_bytes: 8 << 20,
        max_gas: -1,
    });
    if let Err(e) = ante_handle(&mut ctx, &tx, false, &deps) {
        return Err((e.error.code, e.error.log));
    }
    let mut seqs = BTreeMap::new();
    for a in &c.accounts {
        let addr = hex::decode(&a.address_hex).unwrap();
        if let Some(acc) = x_auth::get_account(&mut ctx, &addr).unwrap() {
            seqs.insert(addr, acc.sequence());
        }
    }
    Ok(seqs)
}

#[test]
fn mainnet_txs_verify_under_v10() {
    let corpus = fixture();
    assert!(corpus.cases.len() >= 1_000, "{} cases", corpus.cases.len());
    let mut failures = Vec::new();
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut sequences_checked = 0;
    for c in &corpus.cases {
        let got = run_case(&corpus.chain_id, c);
        let go = if c.expected.ok {
            "ok".to_owned()
        } else {
            format!("{}/{}", c.expected.codespace, c.expected.code)
        };
        let rs = match &got {
            Ok(_) => "ok".to_owned(),
            Err((code, log)) => format!("{code} {}", log.chars().take(90).collect::<String>()),
        };
        *tally.entry(format!("go {go} -> v10 {rs}")).or_default() += 1;
        if c.expected.ok {
            match &got {
                Ok(seqs) => {
                    for s in &c.expected.signer_sequences {
                        let a = hex::decode(&s.address_hex).unwrap();
                        sequences_checked += 1;
                        if seqs.get(&a) != Some(&s.sequence) {
                            failures.push(format!(
                                "{}: signer {} sequence {:?} != go {}",
                                c.label,
                                s.address_hex,
                                seqs.get(&a),
                                s.sequence
                            ));
                        }
                    }
                }
                Err((code, log)) if SIG_CODES.contains(code) => failures.push(format!(
                    "{}: accepted by go v6, v10 signature/decoding failure {code}: {log}",
                    c.label
                )),
                Err(_) => {} // fee / gas rules changed since v6
            }
        } else if c.expected.codespace == "sdk" && SIG_CODES.contains(&c.expected.code) {
            match &got {
                Err((code, _)) if *code == c.expected.code => {}
                other => failures.push(format!(
                    "{}: go v6 rejected with sdk/{}, v10 gave {:?}",
                    c.label,
                    c.expected.code,
                    other.as_ref().map(|_| "ok").map_err(|e| e.0)
                )),
            }
        }
    }
    for (k, v) in &tally {
        eprintln!("{v:5} {k}");
    }
    for f in failures.iter().take(30) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    // The corpus must keep exercising signature verification.
    let accepted = corpus.cases.iter().filter(|c| c.expected.ok).count();
    assert!(accepted >= 900, "only {accepted} accepted cases");
    assert!(
        sequences_checked >= accepted,
        "only {sequences_checked} signer sequences compared"
    );
}
