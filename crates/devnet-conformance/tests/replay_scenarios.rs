//! Every devnet scenario must replay with full parity: app hash and the
//! consensus fields of every tx result (code, codespace, data, gas) at every
//! height, validator updates, PrepareProposal responses (txs, square size,
//! data root), ProcessProposal status and CheckTx responses (code,
//! codespace, gas, priority, signer address and sequence).

use devnet_conformance::replay::replay;

fn assert_parity(scenario: &str) -> anyhow::Result<()> {
    let r = replay(scenario)?;
    assert!(r.errors.is_empty(), "{scenario}: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "{scenario}: stores {:#?}",
        r.store_diffs
    );
    assert!(
        !r.matched_heights.is_empty(),
        "{scenario}: nothing replayed"
    );
    assert!(
        r.compared.prepare > 0 && r.compared.process > 0,
        "{scenario}: proposals not replayed: {:?}",
        r.compared
    );
    Ok(())
}

/// `workloads/cli.sh`: txs built, signed and broadcast by the Rust CLI
/// (checked identical to the Go CLI's) on a Go node.
#[test]
fn cli_parity() -> anyhow::Result<()> {
    assert_parity("cli")
}

#[test]
fn staking_parity() -> anyhow::Result<()> {
    assert_parity("staking")
}

#[test]
fn staking_lifecycle_parity() -> anyhow::Result<()> {
    assert_parity("staking-lifecycle")
}

#[test]
fn hyperlane_parity() -> anyhow::Result<()> {
    assert_parity("hyperlane")
}

#[test]
fn gov_parity() -> anyhow::Result<()> {
    assert_parity("gov")
}

#[test]
fn authz_feegrant_parity() -> anyhow::Result<()> {
    assert_parity("authz-feegrant")
}

#[test]
fn vesting_parity() -> anyhow::Result<()> {
    assert_parity("vesting")
}

#[test]
fn consensus_params_parity() -> anyhow::Result<()> {
    assert_parity("consensus")
}

/// `staking` re-driven through Go with duplicate-vote evidence injected
/// (conformance/devnet/inject-evidence.sh staking evidence 20 30): slash,
/// jail, tombstone; unknown validator, unknown type and an already
/// tombstoned validator are ignored.
#[test]
fn evidence_parity() -> anyhow::Result<()> {
    assert_parity("evidence")
}

/// Gov-scheduled v10 upgrade (handler runs), schedule + cancel, a repeated
/// completed upgrade, then a signalled v11 upgrade: the app version bumps
/// and both Go and Rust refuse the next block (no v11 handler).
#[test]
fn upgrade_parity_and_halt() -> anyhow::Result<()> {
    assert_parity("upgrade")?;
    let r = devnet_conformance::replay::replay("upgrade")?;
    assert!(r.halted_at.is_some(), "Go halted but Rust did not");
    Ok(())
}

/// x/fibre + x/valaddr (`workloads/fibre.sh`): provider registration,
/// escrow deposits/withdrawals (paid out by the BeginBlocker),
/// MsgPayForFibre settlement (PFF system blobs in the square, validator
/// signatures checked in CheckTx and ProcessProposal), unsettleable
/// promises dropped by PrepareProposal, timeouts and a gov
/// MsgUpdateFibreParams.
#[test]
fn fibre_parity() -> anyhow::Result<()> {
    assert_parity("fibre")?;
    let c = devnet_conformance::replay::replay("fibre")?.compared;
    assert!(c.prepare_with_txs >= 15 && c.check_tx >= 40, "{c:?}");
    Ok(())
}

/// x/zkism + x/forwarding (`workloads/zkism-forwarding.sh`): SP1 Groth16
/// state-transition and membership proofs (celestia-app's test vectors)
/// verified on chain, failing proofs, the zk ISM on the hyperlane router,
/// and MsgForward through warp with IGP fee collection/refund.
#[test]
fn zkism_forwarding_parity() -> anyhow::Result<()> {
    assert_parity("zkism-forwarding")
}

#[test]
fn blob_parity() -> anyhow::Result<()> {
    assert_parity("blob")?;
    let c = devnet_conformance::replay::replay("blob")?.compared;
    assert!(c.prepare_with_txs >= 7 && c.check_tx >= 50, "{c:?}");
    Ok(())
}

/// Two Go chains linked by a relayer: client creation and updates,
/// connection and channel handshakes, ICS-20 escrow / mint / burn, error
/// acks, packet-forward-middleware forwards (success and failure) and a
/// packet timeout, all from the main chain's side.
#[test]
fn ibc_parity() -> anyhow::Result<()> {
    assert_parity("ibc")
}

/// IBC edge cases from ibc-go's testing package driving a recorded
/// celestia-app against simapp chains (conformance/ibcvectors): handshake
/// Try/Confirm, redundant and invalid relays, timeouts by height and time,
/// timeout-on-close and close confirm, PFM forwards / refunds / retries and
/// invalid forward metadata, the ICA host (account creation, executed and
/// rejected txs), misbehaviour freezing a client, a wrong-valset header and
/// an expired client.
/// ibctesting drives FinalizeBlock and Commit directly (no proposals or
/// CheckTx), so only the block-level parity applies.
/// `conformance/ibcvectors/upgrade_test.go`: 06-solomachine clients, ICS-27
/// host channel upgrades, proto3json ICA txs, `MsgModuleQuerySafe` and the
/// gov-executed `MsgIBCSoftwareUpgrade`, with CheckTx of every tx.
#[test]
fn ibc_upgrades_parity() -> anyhow::Result<()> {
    let r = replay("ibc-upgrades")?;
    assert!(r.errors.is_empty(), "ibc-upgrades: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "ibc-upgrades: stores {:#?}",
        r.store_diffs
    );
    assert!(r.matched_heights.len() > 180, "{}", r.matched_heights.len());
    assert!(r.compared.check_tx > 140, "{:?}", r.compared);
    Ok(())
}

#[test]
fn ibc_edge_parity() -> anyhow::Result<()> {
    let r = replay("ibc-edge")?;
    assert!(r.errors.is_empty(), "ibc-edge: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "ibc-edge: stores {:#?}",
        r.store_diffs
    );
    assert!(
        r.matched_heights.len() > 150,
        "ibc-edge: {:?}",
        r.matched_heights.len()
    );
    Ok(())
}

/// App version 9 (celestia-app v9.0.8, ADR 0004): an empty chain started at
/// app version 9.
#[test]
fn v9_empty_parity() -> anyhow::Result<()> {
    assert_parity("v9-empty")
}

/// The v10 workloads recorded from Go v9.0.8 (`APP_V9=1`).
#[test]
fn v9_workloads_parity() -> anyhow::Result<()> {
    for s in [
        "v9-staking",
        "v9-gov",
        "v9-authz-feegrant",
        "v9-vesting",
        "v9-blob",
        "v9-hyperlane",
        // Two Go v9 chains linked by cosmos/relayer.
        "v9-ibc",
        // celestia-app v9's SP1 v5 Groth16 vectors (x/zkism testdata/v9).
        "v9-zkism-forwarding",
    ] {
        assert_parity(s)?;
    }
    Ok(())
}

/// `workloads/v9-exec.sh`: v9-only behavior: sequential `deliverTx` (two
/// txs of one account in one block, the second cannot pay its fee),
/// multi-denom fees and their priority, `MsgExec` wrapping
/// `MsgSubmitProposal`.
#[test]
fn v9_exec_parity() -> anyhow::Result<()> {
    assert_parity("v9-exec")
}

/// App version 9 → 10 like mainnet / Mocha (`workloads/v9-upgrade.sh`):
/// txs under v9, MsgSignalVersion + MsgTryUpgrade, the v9 EndBlocker bumps
/// the app version, the recorder switches to the v10 binary after that
/// Commit (as the multiplexer does), the v10 store loader adds fibre and
/// valaddr, the v10 handler runs, then txs under v10.
#[test]
fn v9_upgrade_parity() -> anyhow::Result<()> {
    let r = replay("v9-upgrade")?;
    assert!(r.errors.is_empty(), "v9-upgrade: {:?}", r.errors);
    assert_eq!(r.first_mismatch, None, "v9-upgrade: {:#?}", r.store_diffs);
    assert!(r.matched_heights.len() >= 40, "{:?}", r.matched_heights);
    assert!(r.compared.check_tx > 0, "{:?}", r.compared);
    Ok(())
}

/// The state right after the commit that switches binaries (security.md
/// 70), as Go's freshly opened v10 app has it until the next commit,
/// found by `fuzz-diff deliver` (`DELIVER_UPGRADE`): CheckTx runs on a check
/// state with an empty header (height 0: signatures verified with account
/// number 0, an infinite gas meter), and the stores the upgrade adds are
/// nil in a query of the last v9 height (a recovered nil-pointer panic).
#[test]
fn v9_upgrade_switch_state() -> anyhow::Result<()> {
    use celestia_proto::celestia_core_proto::abci;
    let exchanges = devnet_conformance::load_exchanges("v9-upgrade")?;
    // The height whose FinalizeBlock raised the app version.
    let switch = exchanges
        .iter()
        .find_map(|e| match (&e.request.value, &e.response.value) {
            (
                Some(abci::request::Value::FinalizeBlock(r)),
                Some(abci::response::Value::FinalizeBlock(g)),
            ) if g
                .consensus_param_updates
                .as_ref()
                .and_then(|c| c.version.as_ref())
                .is_some_and(|v| v.app == 10) =>
            {
                Some(r.height)
            }
            _ => None,
        })
        .expect("upgrade height");
    // The txs Go executed at app version 10 (signed with their real
    // account numbers; the validator's is 0).
    let txs: Vec<Vec<u8>> = exchanges
        .iter()
        .filter_map(|e| match &e.request.value {
            Some(abci::request::Value::FinalizeBlock(r)) if r.height > switch + 1 => {
                Some(r.txs.clone())
            }
            _ => None,
        })
        .flatten()
        .collect();
    let mut app = app::App::new();
    let r = devnet_conformance::replay::replay_into("v9-upgrade", &mut app, 0, Some(switch))?;
    assert!(r.errors.is_empty() && r.first_mismatch.is_none(), "{r:?}");
    let mut rejected = 0;
    for tx in txs {
        let c = app
            .check_tx(&abci::RequestCheckTx { tx, r#type: 0 })
            .unwrap();
        assert_eq!(c.gas_wanted, -1);
        if c.code != 0 {
            rejected += 1;
            assert_eq!((c.code, c.codespace.as_str()), (4, "sdk"));
            assert_eq!(
                c.log,
                "signature verification failed; please verify account number (0) and chain-id (test): (unable to verify single signer signature): unauthorized"
            );
        }
    }
    assert!(rejected > 0);
    let q = app.query(&abci::RequestQuery {
        data: Vec::new(),
        path: "/celestia.fibre.v1.Query/Params".into(),
        height: 0,
        prove: false,
    });
    assert_eq!((q.code, q.codespace.as_str()), (111_222, "undefined"));
    assert_eq!(
        q.log,
        "runtime error: invalid memory address or nil pointer dereference: panic"
    );
    Ok(())
}

/// Full parity plus byte-identical logs and events (non-consensus fields
/// of `FinalizeBlock`): the strings of these scenarios carry invalid UTF-8.
fn assert_full_parity(scenario: &str) -> anyhow::Result<()> {
    assert_parity(scenario)?;
    let r = replay(scenario)?;
    assert!(
        r.event_log_diffs.is_empty(),
        "{scenario}: {:#?}",
        r.event_log_diffs
    );
    Ok(())
}

/// `workloads/utf8.sh`: txs with invalid UTF-8 in validator descriptions,
/// memos, gov proposals / votes, authz grants and `MsgExec`, feegrant
/// allowed messages, amino-JSON signed txs (ADR 0005).
#[test]
fn utf8_parity() -> anyhow::Result<()> {
    assert_full_parity("utf8")
}

/// The `utf8` workload on celestia-app v9.0.8.
#[test]
fn v9_utf8_parity() -> anyhow::Result<()> {
    assert_full_parity("v9-utf8")
}

/// Scenarios whose (non-consensus) logs or events still differ from Go.
/// Ratchet: empty since every scenario matches (2026-10-16); a new
/// scenario with a known gap is listed here until it matches.
const KNOWN_EVENT_LOG_GAPS: &[&str] = &[];

/// The two error texts docs/security.md lists as not emulated, in a diff
/// with nothing else: a Go panic's log with the stack Go records when it
/// runs with tracing (`ibctesting`; Rust logs the panic value), and the
/// space protobuf-go randomizes after `proto:` (U+00A0 in some processes).
fn known_text_gap(d: &str) -> bool {
    let (mut go, mut rust) = (Vec::new(), Vec::new());
    for l in d.lines() {
        if let Some(g) = l.trim_start().strip_prefix("go:") {
            go.push(
                g.trim_start()
                    .replace("proto:\u{a0}", "proto: ")
                    .replace("proto:\\xc2\\xa0", "proto: "),
            );
        } else if let Some(r) = l.trim_start().strip_prefix("rust:") {
            rust.push(r.trim_start().to_owned());
        }
    }
    !go.is_empty()
        && go.len() == rust.len()
        && go.iter().zip(&rust).all(|(g, r)| {
            g == r
                || (g.starts_with(&format!("recovered: {r}\\nstack:\\n")) && g.ends_with(": panic"))
        })
}

/// Logs and events of every tx result and block (non-consensus) match Go
/// byte for byte in every scenario but the known gaps.
#[test]
fn events_and_logs_parity() -> anyhow::Result<()> {
    let dir = devnet_conformance::scenario_dir("empty");
    let root = dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("fixtures dir"))?;
    let mut bad = Vec::new();
    for e in std::fs::read_dir(root)? {
        let e = e?;
        if !e.path().join("abci.rec").exists() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let r = replay(&name)?;
        if KNOWN_EVENT_LOG_GAPS.contains(&name.as_str()) {
            continue;
        }
        if let Some(d) = r.event_log_diffs.iter().find(|d| !known_text_gap(d)) {
            bad.push(format!("{name} ({}): {d}", r.event_log_diffs.len()));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
    Ok(())
}
