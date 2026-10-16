//! The `json_*` ops: every JSON decode reachable from consensus execution,
//! called on the raw input as the state machine calls it (the Go side is
//! `conformance/fuzzoracle/*/jsondecode.go`, whose doc lists the ops and
//! the output format), plus seeds and a JSON-text mutator aimed at what
//! differs between JSON decoders: nesting depth, numbers out of `float64`
//! range, key folding, duplicate keys, invalid UTF-8.

use crate::Out;
use crate::mutate::Rng;

/// The JSON ops.
pub const OPS: &[&str] = &[
    "json_count",
    "json_gov",
    "json_pfm",
    "json_pfm_next",
    "json_ica_tx",
    "json_ica_vb",
    "json_ica_pd",
    "json_ica_md",
    "json_ics20",
    "json_ack",
    "json_ack_check",
];

fn h(s: &str) -> String {
    hex::encode(gostr::to_go_bytes(s))
}

fn ok(out: String) -> Out {
    Out {
        ok: true,
        out,
        ..Out::default()
    }
}

fn fail(err: impl Into<String>) -> Out {
    Out {
        err: err.into(),
        ..Out::default()
    }
}

/// The Rust outcome of a `json_*` op.
#[must_use]
pub fn rust_json(app_version: u64, op: &str, input: &[u8]) -> Out {
    let text = gostr::from_go_bytes(input);
    match op {
        "json_count" => match app::msg_count::count_json_msgs(input) {
            Some(n) => ok(n.to_string()),
            None => fail("count failed"),
        },
        "json_gov" => match x_gov::msgs::metadata_title_summary(input) {
            Some((t, s)) => ok(format!("{} {}", h(&t), h(&s))),
            None => fail("unmarshal failed"),
        },
        "json_pfm" => {
            use x_packetforward::memo::{self, Memo};
            match memo::classify(&text) {
                Memo::NotForward => ok("not_forward".into()),
                Memo::NilForward => ok("nil_forward".into()),
                Memo::ParseError(e) => fail(e),
                Memo::Forward(f) => {
                    let retries = f.retries.map_or_else(|| "-".into(), |r| r.to_string());
                    let next = match &f.next {
                        None => "-".into(),
                        Some(Ok(s)) => h(s),
                        Some(Err(_)) => "err".into(),
                    };
                    ok(format!(
                        "forward {} {} {} {} {retries} {next}",
                        h(&f.receiver),
                        h(&f.port),
                        h(&f.channel),
                        f.timeout
                    ))
                }
            }
        }
        "json_pfm_next" => match x_packetforward::memo::unmarshal_next(&text) {
            Ok(Ok(s)) => ok(h(&s)),
            Ok(Err(e)) | Err(e) => fail(e),
        },
        "json_ica_tx" | "json_ica_vb" => match x_icahost::host::deserialize_cosmos_tx(
            input,
            x_icahost::host::ENCODING_PROTO3_JSON,
            app::ibc::ica_registry(),
            app_version != 0 && app_version < 10,
        ) {
            Ok(msgs) => Out {
                ok: true,
                msgs: msgs
                    .iter()
                    .map(|m| {
                        let Some(m) = m else { return "nil".to_owned() };
                        // Go prints `Marshal` of the unpacked message.
                        let v9 = app_version != 0 && app_version < 10;
                        let bz = txdecode::scoped_v9(v9, || txdecode::gogo_repack(m)).value;
                        let mut s = format!("{}:{}", h(&m.type_url), hex::encode(bz));
                        if op == "json_ica_vb" {
                            match app::router::validate_basic(m) {
                                Ok(()) => s.push_str("|vb:ok"),
                                Err(e) if e.is_panic() => s.push_str("|vb:panic"),
                                Err(e) => {
                                    use std::fmt::Write as _;
                                    let _ = write!(s, "|vb:{}/{}", e.codespace, e.code);
                                }
                            }
                        }
                        s
                    })
                    .collect(),
                ..Out::default()
            },
            Err(e) if e.is_panic() => Out {
                panic: e.log,
                ..Out::default()
            },
            Err(e) => fail(e.log),
        },
        "json_ica_pd" => match x_icahost::host::packet_data(input) {
            Some((ty, data)) => ok(format!("{ty} {}", hex::encode(data))),
            None => fail("unmarshal failed"),
        },
        "json_ica_md" => match x_icahost::host::metadata_from_version(&text) {
            Ok(m) => ok([
                &m.version,
                &m.controller_connection_id,
                &m.host_connection_id,
                &m.address,
                &m.encoding,
                &m.tx_type,
            ]
            .map(|s| h(s))
            .join(" ")),
            Err(e) => fail(e.log),
        },
        "json_ics20" => match x_transfer::relay::PacketData::decode(input) {
            Ok(d) => ok([&d.denom, &d.amount, &d.sender, &d.receiver, &d.memo]
                .map(|s| h(s))
                .join(" ")),
            Err(e) => fail(e),
        },
        "json_ack_check" => ok(if x_ibc::channel::ack_marshalling_mismatch(input) {
            "mismatch".into()
        } else {
            "pass".into()
        }),
        "json_ack" => match x_ibc::channel::decode_acknowledgement(input) {
            Ok(x_ibc::channel::AckResponse::Empty) => ok("none".into()),
            Ok(x_ibc::channel::AckResponse::Result(r)) => ok(format!("result {}", hex::encode(r))),
            Ok(x_ibc::channel::AckResponse::Error(e)) => ok(format!("error {}", h(&e))),
            Err(e) => fail(e),
        },
        _ => fail(format!("unknown op {op}")),
    }
}

/// `None` when Go and Rust agree on a `json_*` op (accept / reject and the
/// decoded summary; error texts are not compared).
#[must_use]
pub fn compare(op: &str, go: &Out, rust: &Out) -> Option<String> {
    (go.ok != rust.ok || go.out != rust.out || go.msgs != rust.msgs || go.panic != rust.panic)
        .then(|| format!("{op}: go {go:?}\n  rust {rust:?}"))
}

fn nest(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "[".repeat(n), "]".repeat(n))
}

fn onest(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "{\"a\":".repeat(n), "}".repeat(n))
}

/// Total nesting depths around the limits of serde_json (128) and Go (10 000).
pub const DEPTHS: &[usize] = &[127, 128, 129, 1000, 9_999, 10_000, 10_001];

const FWD: &str = r#""forward":{"receiver":"r","port":"transfer","channel":"channel-0""#;

/// A well-formed input of `op` whose total nesting depth is `d`
/// (`variant` picks where the nesting sits).
#[must_use]
pub fn deep_input(op: &str, d: usize, variant: usize) -> String {
    let x = |n: usize| {
        if variant.is_multiple_of(2) {
            nest(n, "1")
        } else {
            onest(n, "1")
        }
    };
    match op {
        "json_count" if variant < 2 => format!(r#"{{"messages":[{{}}],"x":{}}}"#, x(d - 1)),
        "json_count" => format!(r#"{{"messages":[{{"a":{}}}]}}"#, x(d - 3)),
        "json_gov" => format!(r#"{{"title":"T","summary":"S","x":{}}}"#, x(d - 1)),
        "json_pfm" if variant < 2 => format!("{{{FWD}}},\"x\":{}}}", x(d - 1)),
        "json_pfm" => format!("{{{FWD},\"next\":{}}}}}", x(d - 2)),
        "json_pfm_next" => x(d),
        "json_ica_tx" | "json_ica_vb" => format!(
            r#"{{"messages":[{{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","x":{}}}]}}"#,
            x(d - 3)
        ),
        "json_ica_pd" => format!(r#"{{"type":"TYPE_EXECUTE_TX","data":"","x":{}}}"#, x(d - 1)),
        "json_ica_md" => format!(r#"{{"version":"ics27-1","x":{}}}"#, x(d - 1)),
        "json_ics20" => format!(
            r#"{{"denom":"d","amount":"1","sender":"s","receiver":"r","memo":"m","x":{}}}"#,
            x(d - 1)
        ),
        _ => format!(r#"{{"result":"AQ==","x":{}}}"#, x(d - 1)),
    }
}

/// Whole proto3json `CosmosTx` documents: `null` messages, the top-level
/// shapes and `@type` resolution.
const ICA_TX_DOCS: &[&str] = &[
    r#"{"messages":[{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"g","msgs":[{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","amount":[{"denom":"utia","amount":"1"}]}]}]}"#,
    r#"{"messages":[null]}"#,
    r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend"},null,{"@type":"/cosmos.gov.v1.MsgVote"}]}"#,
    r#"{"messages":null}"#,
    r#"{"messages":[]}"#,
    "null",
    "{}",
    r#"{"Messages":[]}"#,
    "[]",
    r#""x""#,
    r#"{"messages":{}}"#,
    r#"{"messages":[{"@type":""}]}"#,
    r#"{"messages":[{"@type":null}]}"#,
    r#"{"messages":[{"@type":5}]}"#,
    r#"{"messages":[{}]}"#,
    r#"{"messages":[[]]}"#,
    r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.SendAuthorization"}]}"#,
    r#"{"messages":[{"@type":"cosmos.bank.v1beta1.MsgSend"}]}"#,
    r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","x":1}]}"#,
    r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","@type":"/cosmos.gov.v1.MsgVote","voter":"v"}]}"#,
    r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend"}],"messages":[null]} trailing"#,
    r#"{"messages":[{"@type":"/celestia.fibre.v1.MsgDepositToEscrow","signer":"s"}]}"#,
    r#"{"messages":[{"@type":"/celestia.forwarding.v1.MsgForward","signer":"s","custom_hook_id":"h"}]}"#,
];

/// Messages (each sent as `{"messages":[m]}`): `null` elements of repeated
/// messages and `Any`s, both field-name forms, enums, integers, bytes,
/// times, durations and customtypes in their JSON forms.
const ICA_TX_MSGS: &[&str] = &[
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","amount":[{"denom":"utia","amount":"1"}]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","amount":[null]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","amount":[{"denom":"utia","amount":"1"},null]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","amount":[{"denom":"utia","amount":null}]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","amount":[{"denom":"utia","amount":"0x10"},{"denom":"utia","amount":""}]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","fromAddress":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","from_address":"x","toAddress":null,"amount":null}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","amount":[{"denom":"utia","amount":"1","denom":"u"}]}"#,
    r#"{"@type":"/cosmos.bank.v1beta1.MsgMultiSend","inputs":[{"address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","coins":[{"denom":"utia","amount":"2"}]}],"outputs":[null,{"address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","coins":[null]}]}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","msgs":[null]}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","msgs":[{"@type":"/cosmos.bank.v1beta1.MsgSend","amount":[null]},null],"x":1}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","msgs":[{"@type":"/cosmos.gov.v1.MsgVoteWeighted","options":[null]}]}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":"1","voter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","option":"VOTE_OPTION_YES","metadata":""}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposalId":1,"proposal_id":2,"voter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","option":1}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":" 7 ","option":"1"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":"null","option":99}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":"1.0","option":"VOTE_OPTION_NO"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":1e3}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":-1}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":"18446744073709551615","option":-1}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","proposal_id":true}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","option":"vote_option_yes"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","option":"VOTE_OPTION_YES"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","voter":5}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVote","metadata":null,"voter":"\ud800é"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVoteWeighted","proposal_id":"1","voter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","options":[{"option":"VOTE_OPTION_YES","weight":"1.0"}]}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVoteWeighted","proposal_id":"1","voter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","options":[null]}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgVoteWeighted","proposal_id":"1","options":[{"option":"VOTE_OPTION_YES","weight":".5"},{"option":2,"weight":null}]}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgSubmitProposal","messages":[null],"initial_deposit":[],"proposer":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","title":"t","summary":"s"}"#,
    r#"{"@type":"/cosmos.gov.v1.MsgSubmitProposal","messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","amount":[null]}],"initial_deposit":[null],"proposer":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","expedited":true}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","granter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","grantee":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","grant":{"authorization":{"@type":"/cosmos.bank.v1beta1.SendAuthorization","spend_limit":[{"denom":"utia","amount":"5"}],"allow_list":[]},"expiration":"2030-01-01T00:00:00Z"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","granter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","grantee":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","grant":{"authorization":{"@type":"/cosmos.authz.v1beta1.GenericAuthorization","msg":"/cosmos.bank.v1beta1.MsgSend"},"expiration":"2030-01-01T1:02:03,123456789123+24:60"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"authorization":{"@type":"/cosmos.bank.v1beta1.SendAuthorization","spend_limit":[null]},"expiration":null}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"authorization":null,"expiration":"0000-12-31T23:59:59Z"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"expiration":"9999-12-31T23:59:59.999999999-00:01"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"expiration":"2024-02-30T00:00:00Z"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"expiration":"2024-02-29T24:00:00Z"}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"expiration":""}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":null}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","granter":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","grantee":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","grant":{"authorization":{"@type":"/cosmos.staking.v1beta1.StakeAuthorization","max_tokens":null,"allow_list":{"address":["celestiavaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqlskg8z"]},"authorization_type":"AUTHORIZATION_TYPE_DELEGATE"}}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"authorization":{"@type":"/cosmos.staking.v1beta1.StakeAuthorization","denyList":null,"authorizationType":2,"validators":null}}}"#,
    r#"{"@type":"/cosmos.authz.v1beta1.MsgGrant","grant":{"authorization":{"@type":"/cosmos.staking.v1beta1.StakeAuthorization","validators":{}}}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgCreateValidator","description":{"moniker":"m"},"commission":{"rate":"0.1","max_rate":"0.2","max_change_rate":"0.01"},"min_self_delegation":"1","delegator_address":"","validator_address":"celestiavaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqlskg8z","pubkey":{"@type":"/cosmos.crypto.ed25519.PubKey","key":"MCowBQYDK2VwAyEA3zz5sGc2hwMs/pfv0Wf+5nLQ8D6qh5QT"},"value":{"denom":"utia","amount":"1"}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgCreateValidator","commission":{"rate":null,"maxRate":"1e3","max_change_rate":"-.5"},"min_self_delegation":null,"pubkey":{"@type":"/cosmos.crypto.secp256r1.PubKey","key":1},"value":null}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgCreateValidator","pubkey":{"@type":"/cosmos.crypto.secp256r1.PubKey","key":"AuWl3/9qCMyBEs8lzzzhHnSrNOHO2Ocr5uLqTOnMRZvg"},"commission":{"rate":"1.0000000000000000001"}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgCreateValidator","pubkey":{"@type":"/cosmos.crypto.secp256r1.PubKey","key":null},"min_self_delegation":"0b11"}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgUpdateParams","authority":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","params":{"unbonding_time":"1h30m","max_validators":"100","max_entries":7,"historical_entries":0,"bond_denom":"utia","min_commission_rate":"0.05"}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgUpdateParams","params":{"unbondingTime":"-1.5µs","min_commission_rate":null}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgUpdateParams","params":{"unbonding_time":null}}"#,
    r#"{"@type":"/cosmos.staking.v1beta1.MsgUpdateParams","params":{"unbonding_time":"9999999999h"}}"#,
    r#"{"@type":"/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade","authority":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","plan":{"name":"v11","time":"0001-01-01T00:00:00Z","height":"100","info":"","upgraded_client_state":null}}"#,
    r#"{"@type":"/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade","plan":{"time":null,"height":"0x10"}}"#,
    r#"{"@type":"/celestia.blob.v1.MsgPayForBlobs","signer":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","namespaces":["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="],"blob_sizes":[1,"2",null],"share_commitments":["AQ==\n","AQ",null,[1,2,255],""],"share_versions":[0]}"#,
    r#"{"@type":"/celestia.blob.v1.MsgPayForBlobs","namespaces":["-_8="]}"#,
    r#"{"@type":"/celestia.blob.v1.MsgPayForBlobs","namespaces":[[256]],"blobSizes":[4294967296]}"#,
    r#"{"@type":"/celestia.blob.v1.MsgPayForBlobs","namespaces":"AQ==","blob_sizes":null}"#,
    r#"{"@type":"/hyperlane.warp.v1.MsgRemoteTransfer","sender":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","token_id":"0x726f757465725f61707000000000000000000000000000010000000000000000","destination_domain":1,"recipient":"000000000000000000000000AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","amount":"0x10","custom_hook_id":null,"gas_limit":"1","max_fee":{"denom":"utia","amount":"1"}}"#,
    r#"{"@type":"/hyperlane.warp.v1.MsgRemoteTransfer","token_id":"0x0x00000000000000000000000000000000000000000000000000000000000000","amount":null,"gasLimit":"","customHookId":"0x00"}"#,
    r#"{"@type":"/hyperlane.warp.v1.MsgEnrollRemoteRouter","owner":"o","token_id":"0x726f757465725f61707000000000000000000000000000010000000000000000","remote_router":{"receiver_domain":1,"receiver_contract":"not hex","gas":"1"}}"#,
    r#"{"@type":"/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount","from_address":"celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7","to_address":"celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj","start_time":"-5","vesting_periods":[null,{"length":"10","amount":[null]}]}"#,
];

/// Seeds of `op`: valid documents of its shape and the depth cases.
#[must_use]
pub fn seeds(op: &str) -> Vec<Vec<u8>> {
    let base: &[&str] = match op {
        "json_count" => &[
            r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","amount":[{"denom":"utia","amount":"1"}]},{}]}"#,
            r#"{"Messages":null}"#,
        ],
        "json_gov" => &[
            r#"{"title":"T","authors":["a",null],"summary":"S","details":"d","proposal_forum_url":"u","vote_option_context":"v"}"#,
        ],
        "json_pfm" | "json_pfm_next" => &[
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"1m","retries":2,"next":{"b":1,"a":[1.50,"<x>",{"c":null}],"b":2}}}"#,
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","next":"{\"x\":1}"}}"#,
        ],
        "json_ica_tx" | "json_ica_vb" => ICA_TX_DOCS,
        "json_ica_pd" => &[r#"{"type":"TYPE_EXECUTE_TX","data":"AQI=","memo":"m"}"#],
        "json_ica_md" => &[
            r#"{"version":"ics27-1","controller_connection_id":"connection-0","host_connection_id":"connection-1","address":"","encoding":"proto3json","tx_type":"sdk_multi_msg"}"#,
        ],
        "json_ics20" => &[
            r#"{"denom":"utia","amount":"1","sender":"s","receiver":"r","memo":"{\"forward\":{}}"}"#,
        ],
        _ => &[
            r#"{"result":"AQ=="}"#,
            r#"{"error":"e"}"#,
            r#"{"result":null}"#,
            r#"{"result":[1,null]}"#,
            r#"{"error":null}"#,
            r#"{"response":null,"result":""}"#,
            r#"{"result":"AQ==","result":"AQ=="}"#,
        ],
    };
    let mut out: Vec<Vec<u8>> = base.iter().map(|s| s.as_bytes().to_vec()).collect();
    if op == "json_ica_tx" || op == "json_ica_vb" {
        for m in ICA_TX_MSGS {
            out.push(format!(r#"{{"messages":[{m}]}}"#).into_bytes());
        }
        // `MsgExec` nested around the interface registry's limit of 10
        // `Any` levels.
        for k in [1, 9, 10, 11] {
            let mut inner = r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","amount":[{"denom":"utia","amount":"1"}]}"#.to_owned();
            for _ in 0..k {
                inner = format!(
                    r#"{{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"g","msgs":[{inner}]}}"#
                );
            }
            out.push(format!(r#"{{"messages":[{inner}]}}"#).into_bytes());
        }
    }
    for &d in DEPTHS {
        for v in 0..4 {
            out.push(deep_input(op, d, v).into_bytes());
        }
    }
    out
}

/// Byte positions of the values of `s` (cheap: after `:`, `[` or `,`).
fn value_starts(s: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in s.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b':' | b'[' | b',' => v.push(i + 1),
            _ => {}
        }
    }
    v
}

/// A JSON-text mutation of `input`.
#[must_use]
pub fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let starts = value_starts(input);
    let mut s = input.to_vec();
    if starts.is_empty() {
        return s;
    }
    let at = starts[rng.below(starts.len())];
    let insert = |s: &mut Vec<u8>, at: usize, b: &[u8]| {
        s.splice(at..at, b.iter().copied());
    };
    match rng.below(12) {
        // A whole value replaced (`null` most often: zero elements, nil
        // pointers), on the parsed tree so the document stays valid.
        9 | 10 => {
            if let Ok(v) = gojson::parse_first(&s) {
                let mut spans = Vec::new();
                v.all(|x| {
                    spans.push(x.span.clone());
                    true
                });
                let sp = spans[rng.below(spans.len())].clone();
                let with: &[u8] = [
                    &b"null"[..],
                    b"null",
                    b"null",
                    b"[null]",
                    b"\"\"",
                    b"0",
                    b"\"1\"",
                    b"[]",
                    b"{}",
                    b"true",
                    b"\"0x10\"",
                    b"1",
                ][rng.below(12)];
                s.splice(sp, with.iter().copied());
            }
        }
        // A `snake_case` key in camelCase or the reverse (both are field
        // names in jsonpb).
        11 => {
            let keys: Vec<usize> = (1..s.len())
                .filter(|&i| s[i - 1] == b'"' && s[i].is_ascii_lowercase())
                .collect();
            if !keys.is_empty() {
                let i = keys[rng.below(keys.len())];
                let end = s[i..]
                    .iter()
                    .position(|&c| c == b'"')
                    .map_or(s.len(), |e| i + e);
                let key = &s[i..end];
                let renamed: Vec<u8> = if key.contains(&b'_') {
                    let mut out = Vec::new();
                    let mut up = false;
                    for &c in key {
                        if c == b'_' {
                            up = true;
                        } else {
                            out.push(if up { c.to_ascii_uppercase() } else { c });
                            up = false;
                        }
                    }
                    out
                } else {
                    key.iter()
                        .flat_map(|&c| {
                            if c.is_ascii_uppercase() {
                                vec![b'_', c.to_ascii_lowercase()]
                            } else {
                                vec![c]
                            }
                        })
                        .collect()
                };
                s.splice(i..end, renamed);
            }
        }
        // A deep value before an existing one (`[[[...]]],` keeps arrays
        // valid; in objects it is a syntax error, also worth comparing).
        0 => {
            let d = [100, 127, 128, 129, 5_000, 9_990, 9_999, 10_000, 10_001][rng.below(9)];
            insert(&mut s, at, format!("{},", nest(d, "0")).as_bytes());
        }
        // A deep member in front of the first key.
        1 => {
            if let Some(p) = s.iter().position(|&c| c == b'{') {
                let d = [126, 127, 128, 9_998, 9_999, 10_000][rng.below(6)];
                insert(
                    &mut s,
                    p + 1,
                    format!("\"zz\":{},", onest(d, "{}")).as_bytes(),
                );
            }
        }
        // Numbers float64 cannot hold, and exotic literals.
        2 => {
            let n = [
                "1e400",
                "-1e400",
                "1e-400",
                "0.0",
                "-0",
                "123456789012345678901234567890",
            ][rng.below(6)];
            insert(&mut s, at, format!("{n},").as_bytes());
        }
        // A key folded with long s / Kelvin sign, or upper case.
        3 => {
            let i = rng.below(s.len());
            match s[i] {
                b's' => {
                    s.splice(i..=i, "\u{17f}".bytes());
                }
                b'k' => {
                    s.splice(i..=i, "\u{212a}".bytes());
                }
                c if c.is_ascii_lowercase() => s[i] = c.to_ascii_uppercase(),
                _ => {}
            }
        }
        // Duplicate a member.
        4 => {
            if let Some(p) = s.iter().position(|&c| c == b'{') {
                let end = s[p..]
                    .iter()
                    .position(|&c| c == b',' || c == b'}')
                    .map_or(s.len(), |e| p + e);
                let member: Vec<u8> = s[p + 1..end].to_vec();
                if !member.is_empty() {
                    let mut m = member;
                    m.push(b',');
                    insert(&mut s, p + 1, &m);
                }
            }
        }
        // `null` or a wrong-typed scalar.
        5 => {
            let v = ["null,", "true,", "\"x\",", "{},", "[],"][rng.below(5)];
            insert(&mut s, at, v.as_bytes());
        }
        // Invalid UTF-8, escapes, control characters.
        6 => {
            let i = rng.below(s.len());
            let b: &[u8] = [
                &b"\xff"[..],
                b"\xc3",
                b"\\ud800",
                b"\\u0000",
                b"\n",
                b"\x01",
            ][rng.below(6)];
            insert(&mut s, i, b);
        }
        // Truncate or append garbage.
        7 => {
            if rng.one_in(2) {
                s.truncate(rng.below(s.len()));
            } else {
                s.extend_from_slice([&b" x"[..], b"{}", b" ", b"]"][rng.below(4)]);
            }
        }
        // Delete a byte.
        _ => {
            let i = rng.below(s.len());
            s.remove(i);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_inputs_have_the_depth() {
        for op in OPS {
            for v in 0..4 {
                for d in [127usize, 10_000] {
                    let s = deep_input(op, d, v);
                    assert_eq!(
                        gojson::check_valid(s.as_bytes()).map(|x| x.depth),
                        Ok(d),
                        "{op} {v} {d}"
                    );
                }
            }
        }
    }
}
