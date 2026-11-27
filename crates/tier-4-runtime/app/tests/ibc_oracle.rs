//! The IBC chain against the Go oracle (`conformance/v1-oracle`
//! `TestIBCFixture`, fixture `conformance/fixtures/synthetic/ibc-v2`).
//!
//! celestia-app v3.13.0 is chain A of an ibc-go v6 ibctesting
//! coordinator with two ibc-go simapp counterparties. It crosses the v1
//! to v2 upgrade at block 2 (`--v2-upgrade-height 3`), then opens
//! transfer channels to B and C, sends utia to B, and receives it back
//! three times with a packet-forward-middleware `forward` memo to C:
//!
//! - forwarded (block 37) and acknowledged by C (block 42, which writes
//!   the acknowledgement of B's packet);
//! - forwarded (block 45) and rejected by C (block 50: the utia moves
//!   back to the A-B escrow and B's packet gets an error
//!   acknowledgement);
//! - forwarded with one retry (block 53), timed out and sent again
//!   (block 56), timed out again (block 58: A gives up, as above).
//!
//! Then it opens an interchain account channel for a controller on B
//! (blocks 69 and 72, which create the account), executes the account's
//! delegate, send and withdraw-rewards messages (block 75), then a
//! withdraw address, an undelegation, a redelegation and an IBC transfer
//! to B (block 78; B's acknowledgement in block 83). Two transactions
//! fail with error acknowledgements: one signed by another account
//! (block 85) and one whose send exceeds the balance (block 88, which
//! rolls back its delegation too).
//!
//! Then a sender creates a validator with 100 times the genesis
//! validators' power (block 90), signals version 3 (block 91) and sends
//! `MsgTryUpgrade` (block 92); the EndBlocker of block 95 sets app
//! version 3. At app version 3: a transfer to B (block 96), a PFM
//! forward from B to C (block 102, acknowledged in block 107) and an
//! interchain account delegation to the new validator (block 110).
//! Then an interchain account packet times out on B, so the ordered
//! channel closes (block 115: `MsgChannelCloseConfirm`), B registers
//! again and A reopens the same account on a new channel (blocks 117 and
//! 120) and executes a send over it (block 122), then a multi-send
//! (block 125). Last, a proposal (block 127, the genesis has 40 second
//! gov periods) narrows the host's `AllowMessages` to `MsgSend`; it
//! passes in block 130, the account's delegation gets an error
//! acknowledgement (block 132) and its send runs (block 135). Then utia
//! goes to B through authz `MsgExec` with a GenericAuthorization (block
//! 138) and an ibc-go TransferAuthorization (block 143), and a plain
//! transfer (block 148) times out on B and is refunded (block 150). The
//! fixture keeps its `ibc-v2` name.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("ibc-v2")
}

/// The last block that celestia-rs executes as Go does: every block.
const MATCHED_THROUGH: i64 = 174;

/// The fixture files are the ones the oracle wrote.
#[test]
fn ibc_fixture_files_match_their_record() {
    let f = fix();
    let r = f.roots();
    let sha = |b: &[u8]| hex::encode(hash::sha256(b));
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 3);
    assert_eq!(r["genesis_sha256"], sha(&f.read("genesis.json")));
    assert_eq!(r["blocks_sha256"], sha(&f.gunzip("blocks.jsonl.gz")));
    assert_eq!(
        r["tx_results_sha256"],
        sha(&f.gunzip("tx-results.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-height-0.jsonl"],
        sha(&f.gunzip("kv-height-0.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-diffs.jsonl"],
        sha(&f.gunzip("kv-diffs.jsonl.gz"))
    );
    assert_eq!(f.last_height(), 174);
}

#[test]
fn ibc_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
