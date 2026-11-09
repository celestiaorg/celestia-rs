//! Delivery differential (`fuzz-diff deliver`): like [`crate::exec`], a
//! devnet scenario is re-driven into a Go `celestia-appd` (standalone ABCI)
//! and the Rust [`app::App`] in lockstep, but the genesis gets funded
//! accounts whose keys this harness holds, so the generated txs are
//! *signed* and go through the whole block path on both apps:
//! `CheckTx`, `PrepareProposal` (Go's block is the one used),
//! `ProcessProposal`, `FinalizeBlock` and `Commit`. Compared: every
//! `CheckTx` response, the prepared block, the proposal decision, each
//! tx result (code, codespace, data, gas wanted / used: `LastResultsHash`;
//! then log, info and events), block events, validator and consensus
//! parameter updates, and the app hash after every block.
//!
//! The recorded blocks are replayed first (with the patched genesis, Go
//! and Rust compared at every height too), then `blocks` generated blocks
//! follow on the scenario's final state: messages of every registered
//! type built from scratch or taken from the scenario and mutated at the
//! field level (boundary numbers, decimals, denoms, addresses, times,
//! repeated fields), state-building flows (sends, delegations, validators,
//! grants, fee allowances, proposals executed by gov with the fuzz
//! accounts' votes), wrappers (`MsgExec`, gov proposals), several txs per
//! account and block, and ante variants (fees, gas, memo, timeout height,
//! extension options, fee granter / payer, sign modes, sequences).
//! `crates/fuzz-diff/deliver-diff.sh` starts the Go app.

// Generator code: random numbers are cut to the widths the fields take.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, BTreeSet};

use abci_server::record::{Direction, parse};
use anyhow::Context as _;
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{self, Request, Response, request, response};
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::tx::signing::v1beta1::SignMode;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignDocDirectAux, SignerInfo, TxBody, TxRaw, mode_info,
};
use math::{Int, LegacyDec};
use prost::{Message as _, Name as _};
use prost_reflect::{Cardinality, FieldDescriptor, Kind, MessageDescriptor};

use crate::mutate::{Field, Mutator, encode, parse as parse_fields, put_varint};
use crate::pp::GoApp;

mod gov_coverage;
mod gov_errors;
mod gov_params;
mod hyperlane;
mod trace;
mod upgrade;
mod upgrade_plan;

/// Funded single-key accounts added to the genesis (the multisig ones of
/// [`multisig_trees`] follow).
const ACCOUNTS: usize = 12;
const HL_MAILBOX: u8 = 0;
const HL_ISM: u8 = 1;
const HL_ROUTING_ISM: u8 = 2;
const HL_HOOK: u8 = 3;
const HL_IGP: u8 = 4;
const HL_TOKEN: u8 = 5;

/// The hyperlane object kind a create message's response id names.
fn hl_kind(response_type_url: &str) -> Option<u8> {
    let t = response_type_url.rsplit('.').next()?;
    Some(match t {
        "MsgCreateMailboxResponse" => HL_MAILBOX,
        "MsgCreateRoutingIsmResponse" => HL_ROUTING_ISM,
        "MsgCreateNoopIsmResponse"
        | "MsgCreateMerkleRootMultisigIsmResponse"
        | "MsgCreateMessageIdMultisigIsmResponse" => HL_ISM,
        "MsgCreateMerkleTreeHookResponse" | "MsgCreateNoopHookResponse" => HL_HOOK,
        "MsgCreateIgpResponse" => HL_IGP,
        "MsgCreateCollateralTokenResponse"
        | "MsgCreateSyntheticTokenResponse"
        | "MsgCreateNativeSyntheticTokenResponse" => HL_TOKEN,
        _ => return None,
    })
}

/// Hyperlane objects the fuzz accounts created, as far as messages that
/// target them successfully need to know.
#[derive(Default)]
struct HlState {
    /// Mailbox id -> local domain.
    mailbox_domain: BTreeMap<String, u32>,
    /// Noop ISMs (they verify every message).
    noop_isms: BTreeSet<String>,
    /// Token id -> origin mailbox.
    token_mailbox: BTreeMap<String, String>,
    /// Enrolled remote routers: (token id, domain, receiver contract).
    routers: Vec<(String, u32, String)>,
    /// IGP id -> denom.
    igp_denom: BTreeMap<String, String>,
    /// Destination gas configs set: (IGP id, domain).
    igp_domains: Vec<(String, u32)>,
    /// Message ids dispatched (`MsgRemoteTransfer` responses).
    message_ids: Vec<String>,
}

fn push_capped<T>(v: &mut Vec<T>, x: T) {
    v.push(x);
    if v.len() > 64 {
        v.remove(0);
    }
}

/// The multisig public key type URL.
const MULTISIG_URL: &str = "/cosmos.crypto.multisig.LegacyAminoPubKey";
/// Their utia balance each.
const BALANCE: u128 = 1_000_000_000_000_000;
/// The gov module account.
const GOV: &str = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7";
const CONTROLLED_REMOVED_VALIDATOR: [u8; 20] = [0xa5; 20];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvidenceRequirement {
    Stale,
    RemovedValidator,
    StakeLifecycle,
    ZeroDoubleSignSlash,
}

impl EvidenceRequirement {
    fn from_env() -> anyhow::Result<Option<Self>> {
        std::env::var_os("DELIVER_EVIDENCE_REQUIRE")
            .map(|value| {
                let value = value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("DELIVER_EVIDENCE_REQUIRE must be UTF-8"))?;
                match value.as_str() {
                    "stale" => Ok(Self::Stale),
                    "removed-validator" => Ok(Self::RemovedValidator),
                    "stake-lifecycle" => Ok(Self::StakeLifecycle),
                    "zero-double-sign-slash" => Ok(Self::ZeroDoubleSignSlash),
                    _ => {
                        anyhow::bail!("DELIVER_EVIDENCE_REQUIRE must be stale, removed-validator, stake-lifecycle, or zero-double-sign-slash")
                    }
                }
            })
            .transpose()
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::RemovedValidator => "removed-validator",
            Self::StakeLifecycle => "stake-lifecycle",
            Self::ZeroDoubleSignSlash => "zero-double-sign-slash",
        }
    }
}

/// Outcome counts of a run.
#[derive(Debug, Default)]
pub struct Stats {
    /// Recorded heights replayed.
    pub replayed: u64,
    /// Generated blocks finalized.
    pub blocks: u64,
    /// Generated txs sent to `CheckTx`.
    pub checked: u64,
    /// Of which Go accepted.
    pub check_ok: u64,
    /// Generated txs with a multisig signer info sent to `CheckTx`, and of
    /// those Go accepted.
    pub multisig: (u64, u64),
    /// Txs finalized in generated blocks.
    pub delivered: u64,
    /// Of which Go executed successfully (code 0).
    pub delivered_ok: u64,
    /// Per message type URL (nested ones prefixed by `>`): (finalized,
    /// Go code 0, rejected by Go's `CheckTx`).
    pub per_type: BTreeMap<String, (u64, u64, u64)>,
    /// Exact top-level message types required to execute successfully.
    pub type_required: Vec<String>,
    /// Go's results of finalized txs by first message type and
    /// `codespace/code`.
    pub codes: BTreeMap<String, u64>,
    /// Top-level handler failures identified by Go's outer SDK message index.
    pub handler_errors: BTreeMap<String, u64>,
    /// Finalized failures without a usable SDK handler index (not necessarily ante).
    pub unattributed_errors: BTreeMap<String, u64>,
    /// Exact begin-block evidence boundary selected for this run.
    pub evidence_required: Option<String>,
    /// Controlled misbehavior requests injected, by boundary.
    pub evidence_injected: BTreeMap<String, u64>,
    /// Consensus disagreements (app hash, tx result code / data / gas,
    /// validator / param updates, `CheckTx` code / gas, proposals).
    pub mismatches: Vec<String>,
    /// Disagreements in logs / events / info only.
    pub log_mismatches: Vec<String>,
    /// The run stopped at an app-hash split.
    pub split: bool,
    /// Gas-sweep copies sent to `CheckTx` (`DELIVER_SWEEP`).
    pub swept: u64,
    /// Results of the scripted txs (`DELIVER_SCRIPT`).
    pub script: Vec<String>,
    /// Required Go outcomes of finalized scripted transactions.
    pub script_required: Vec<ScriptExpectation>,
    /// Observed type and Go result of every finalized scripted transaction.
    pub script_outcomes: Vec<(String, abci::ExecTxResult)>,
    /// A required script must finalize every signed transaction in order.
    pub script_incomplete: Option<&'static str>,
    /// Whether the run requires the controlled direct-aux signer fixture.
    pub direct_aux_required: Option<()>,
    /// Controlled direct-aux fixtures that passed their exact CheckTx,
    /// proposal, execution, balance, and sequence postconditions.
    pub direct_aux_completed: u64,
    /// Whether the run requires the controlled malformed multisig fixtures.
    pub malformed_multisig_required: Option<()>,
    /// Malformed multisig fixtures that passed CheckTx, proposal, execution,
    /// and committed-state rollback postconditions.
    pub malformed_multisig_completed: u64,
    /// Whether the run requires zero-threshold multisig persistence coverage.
    pub zero_threshold_multisig_required: Option<()>,
    /// Zero-threshold multisig sends that passed exact execution and
    /// committed-key postconditions.
    pub zero_threshold_multisig_completed: u64,
    /// Whether the run requires P-256 account execution coverage.
    pub p256_required: Option<()>,
    /// P-256 fixtures that passed their exact execution and persistence
    /// postconditions.
    pub p256_completed: u64,
    /// Whether the run requires malformed-key panic recovery and Ed25519
    /// rollback coverage.
    pub pubkey_rejection_required: Option<()>,
    /// Controlled public-key fixtures that passed their exact CheckTx,
    /// proposal, execution, recovery, and committed-state postconditions.
    pub pubkey_rejection_completed: u64,
    /// Whether the run requires the controlled never-bonded unjail lifecycle.
    pub never_bonded_unjail_required: Option<()>,
    /// Never-bonded unjail lifecycles that passed all four transactions and
    /// committed validator, delegation, signing-info, pool, and balance gates.
    pub never_bonded_unjail_completed: u64,
    /// Whether the run requires the all-shares unbonded redelegation rollback.
    pub unbonded_redelegation_rollback_required: Option<()>,
    /// Controlled rollback fixtures that reached the exact failed delivery,
    /// unchanged staking/distribution state, and recovery commit.
    pub unbonded_redelegation_rollback_completed: u64,
    /// Whether the run requires every deterministic governance Params
    /// validation rejection to persist through a passed-tally proposal.
    pub gov_params_rejection_required: Option<()>,
    /// Complete 29-case governance Params rejection lifecycles.
    pub gov_params_rejection_completed: u64,
    /// Exact terminal credits by governance Params mutation name.
    pub gov_params_rejection_cases: BTreeMap<String, u64>,
    /// Whether the run requires the remaining reachable governance message
    /// server rejection boundaries and their successful metadata control.
    pub gov_handler_rejection_required: Option<()>,
    /// Complete governance message-server rejection lifecycles.
    pub gov_handler_rejection_completed: u64,
    /// Exact terminal credits by governance message-server case name.
    pub gov_handler_rejection_cases: BTreeMap<String, u64>,
    /// Whether the run requires validator-removal reward cleanup with
    /// nonzero commission and decimal remainders.
    pub validator_removal_rewards_required: Option<()>,
    /// Validator-removal reward fixtures that passed exact payout,
    /// conservation, retained-user-reward, and key-deletion gates.
    pub validator_removal_rewards_completed: u64,
    /// Whether the run requires the controlled governance upgrade-plan lifecycle
    /// and handler-rejection boundaries.
    pub upgrade_plan_required: Option<()>,
    /// Complete schedule, overwrite, cancel, idempotent-cancel, five rejected
    /// plan boundaries, and recovery lifecycles that passed every proposal and
    /// committed-state postcondition.
    pub upgrade_plan_completed: u64,
    /// Distinct upgrade-plan validation/past-height proposal executions that
    /// failed with their exact expected governance result and query state.
    pub upgrade_plan_rejections: u64,
    /// Whether the run requires an exact due-plan halt for a binary without
    /// the scheduled upgrade handler.
    pub upgrade_halt_required: Option<()>,
    /// Missing-handler halts that proved the exact error, upgrade-info file,
    /// retained plan, and absence of a commit at the due height.
    pub upgrade_halt_completed: u64,
    /// Whether the run requires a configured unsafe skip of a due plan.
    pub upgrade_skip_required: Option<()>,
    /// Due-plan skips that cleared the plan and committed the controlled send
    /// without marking the upgrade applied.
    pub upgrade_skip_completed: u64,
    /// Whether the run requires a persistent governance v9-to-v10 restart.
    pub upgrade_restart_required: Option<()>,
    /// Governance upgrades that halted v9, restarted both apps with v10
    /// semantics, applied and survived a second restart plus recovery block.
    pub upgrade_restart_completed: u64,
    /// Results of the proposals that ended (`active_proposal` events).
    pub gov_results: BTreeMap<String, u64>,
    /// Messages actually executed by Go in passed generated proposals,
    /// counted by type URL (submission alone is not execution).
    pub gov_executed: BTreeMap<String, u64>,
    /// Exact payload types explicitly required to execute through governance.
    pub gov_required: Vec<String>,
    /// Substring required in a failed proposal's Go `proposal_log`.
    pub gov_failure_required: Option<String>,
    /// Payload type required on that exact tracked failed proposal.
    pub gov_failure_type_required: Option<String>,
    /// Failed Go proposal logs that contained the required substring.
    pub gov_failure_matches: u64,
    /// Require a rejected proposal whose final tally has only abstain power.
    pub gov_abstain_required: Option<()>,
    /// Rejected proposals observed with nonzero abstain and zero other power.
    pub gov_abstain_rejections: u64,
    /// IBC relays of the mock counterparty (`DELIVER_IBC`), by kind.
    pub ibc: BTreeMap<&'static str, u64>,
    /// Require a code-0 channel-upgrade Ack FAILURE response with the matching
    /// callback error after the host aborts the otherwise valid proof path.
    pub ibc_ack_failure_required: bool,
    /// Named fail-closed IBC path requirements that do not have a successful
    /// top-level transaction to count.
    pub ibc_required: BTreeSet<&'static str>,
    /// Selected `DELIVER_HL` ISM.
    pub hl_verification: Option<String>,
    /// Successful collateral / synthetic transfers verified by that ISM.
    pub hl_verified: BTreeMap<String, u64>,
    /// Successful controlled registry handlers, attributed to their reserved
    /// Hyperlane sequence transaction.
    pub hl_registry: BTreeMap<String, u64>,
    /// Both controlled registry handlers must successfully finalize.
    pub hl_registry_required: Option<()>,
    /// Successful scripted transfers by app version and path
    /// (`v9/collateral`, for example). Signed upgrade runs require these.
    pub hl_verified_versions: BTreeMap<String, u64>,
    /// A signed upgrade run must cover both incoming paths before switching.
    pub hl_preupgrade_required: bool,
    /// Generated blocks reserved for mandatory signed incoming paths at v9.
    pub hl_preupgrade_blocks: u64,
    /// Required rejection / successful same-message retry cases.
    pub hl_rejection_cases: Vec<String>,
    /// Complete expected-rejection then successful retry pairs, by case.
    pub hl_retried: BTreeMap<String, u64>,
    /// Incomplete or invalid rejection/retry pairs; not consensus findings.
    pub hl_rejection_failures: Vec<String>,
    /// Generated blocks in the controlled rejection setup phase.
    pub hl_rejection_blocks: u64,
    /// Height where every rejection/retry case was completed.
    pub hl_rejection_complete_height: Option<i64>,
    /// Require the v9/v10 Hyperlane recovery-id boundary fixture.
    pub hl_recovery_id_required: Option<()>,
    /// Exact recovery-id boundary blocks completed.
    pub hl_recovery_id_completed: u64,
    /// Incomplete or invalid recovery-id boundary observations.
    pub hl_recovery_id_failures: Vec<String>,
    /// `DELIVER_WIRE` mutations by class: (in finalized txs, of those Go
    /// code 0, in txs Go's `CheckTx` rejected).
    pub wire: BTreeMap<String, (u64, u64, u64)>,
    /// Queries that echo stored messages (`query_echo`), by path: (sent,
    /// Go code 0 with a non-empty value).
    pub queries: BTreeMap<String, (u64, u64)>,
    /// `DELIVER_UPGRADE`: the signal / try-upgrade results and the
    /// switch.
    pub upgrade: Vec<String>,
    /// `DELIVER_UPGRADE`: per phase of the upgrade (see
    /// `upgrade::Upgrade::phase`): (blocks, txs finalized, of those Go code
    /// 0, txs Go's `CheckTx` rejected).
    pub phases: BTreeMap<String, (u64, u64, u64, u64)>,
    /// `DELIVER_UPGRADE`: the driver's txs by first message type.
    pub upgrade_built: BTreeMap<String, u64>,
    /// `DELIVER_UPGRADE`: Go's `CheckTx` rejections by phase, code and log
    /// (digits dropped).
    pub upgrade_check: BTreeMap<String, u64>,
    /// Comet's mempool recheck after a `Commit` (`ReCheckTx` of the txs
    /// Go's `CheckTx` accepted and no block included), by phase ("switch"
    /// for the first recheck on the next app version's fresh check
    /// state): (rechecked, removed: code != 0 or `PostCheckMaxGas`).
    pub recheck: BTreeMap<String, (u64, u64)>,
    /// `DELIVER_SIM`: every generated tx simulated on both apps before its
    /// `CheckTx` (`Service/Simulate`), as signed and as `--gas auto`
    /// builds it (empty signatures), by first message type: (simulated,
    /// Go succeeded, gas or code differed).
    pub sim: BTreeMap<String, (u64, u64, u64)>,
}

/// A funded account.
struct Acct {
    /// The secp256k1 key (of a multisig account: its first single key,
    /// unused).
    key: keyring::KeyRecord,
    acc: String,
    val: String,
    number: u64,
    seq: u64,
    /// Address bytes.
    addr: Vec<u8>,
    /// The public key packed in an `Any`.
    pk: Any,
    /// The key tree of a multisig (`LegacyAminoPubKey`) account.
    ms: Option<MsKey>,
}

/// A multisig key tree over the single fuzz accounts (by index).
#[derive(Clone, Debug)]
enum MsKey {
    Single(usize),
    Multi { threshold: u32, members: Vec<MsKey> },
}

/// The multisig accounts added after the single ones: 2-of-3, 1-of-2,
/// nested, 4-of-8 (more sub-keys than `tx_sig_limit`), a repeated sub-key
/// and 1-of-1.
fn multisig_trees() -> Vec<MsKey> {
    use MsKey::{Multi, Single};
    let m = |threshold, members| Multi { threshold, members };
    vec![
        m(2, vec![Single(0), Single(1), Single(2)]),
        m(1, vec![Single(3), Single(4)]),
        m(2, vec![Single(5), m(1, vec![Single(3), Single(4)])]),
        m(4, (0..8).map(Single).collect()),
        m(2, vec![Single(6), Single(6), Single(7)]),
        m(1, vec![Single(8)]),
    ]
}

fn ms_pubkey(singles: &[Acct], k: &MsKey) -> x_ante::pubkey::PubKey {
    use x_ante::pubkey::PubKey;
    match k {
        MsKey::Single(i) => PubKey::Secp256k1(singles[*i].key.pub_key.to_vec()),
        MsKey::Multi { threshold, members } => {
            let keys: Vec<PubKey> = members.iter().map(|m| ms_pubkey(singles, m)).collect();
            PubKey::new_multisig(*threshold, keys)
        }
    }
}

/// How one signer's signature is made: a single key in a sign mode
/// (sometimes corrupted), or a multisig's chosen sub-signatures.
#[derive(Clone, Debug)]
enum Plan {
    Single {
        acct: usize,
        mode: SignMode,
        corrupt: bool,
    },
    Multi {
        subs: Vec<Plan>,
        /// 0: none, 1: an extra signature, 2: the last one dropped, 3: an
        /// unknown field in `MultiSignature`.
        tweak: u8,
    },
}

impl Plan {
    fn uses_amino(&self) -> bool {
        match self {
            Self::Single { mode, .. } => *mode == SignMode::LegacyAminoJson,
            Self::Multi { subs, .. } => subs.iter().any(Self::uses_amino),
        }
    }
}

fn single_mode_info(mode: SignMode) -> ModeInfo {
    ModeInfo {
        sum: Some(mode_info::Sum::Single(mode_info::Single {
            mode: mode as i32,
        })),
    }
}

type BitArray = celestia_proto::cosmos::crypto::multisig::v1beta1::CompactBitArray;

fn bit_array(n: usize, set: &[usize]) -> BitArray {
    let mut elems = vec![0u8; n.div_ceil(8)];
    for &i in set {
        if let Some(b) = elems.get_mut(i / 8) {
            *b |= 1 << (7 - i % 8);
        }
    }
    BitArray {
        extra_bits_stored: (n % 8) as u32,
        elems,
    }
}

/// The mode info and signing plan of key tree `k`. With `rng`, the
/// sub-signers, their modes and the encoding are varied (threshold +/- 1,
/// bit-array sizes, mode-info counts, nested and mixed modes); without,
/// the first `threshold` members sign in `mode`.
fn ms_plan(
    mut rng: Option<&mut crate::mutate::Rng>,
    k: &MsKey,
    mode: SignMode,
) -> (ModeInfo, Plan) {
    let other = |m| {
        if m == SignMode::Direct {
            SignMode::LegacyAminoJson
        } else {
            SignMode::Direct
        }
    };
    match k {
        MsKey::Single(i) => {
            let (mode, corrupt) = match rng.as_deref_mut() {
                Some(r) => (if r.one_in(6) { other(mode) } else { mode }, r.one_in(40)),
                None => (mode, false),
            };
            (
                single_mode_info(mode),
                Plan::Single {
                    acct: *i,
                    mode,
                    corrupt,
                },
            )
        }
        MsKey::Multi { threshold, members } => {
            let n = members.len();
            let t = *threshold as usize;
            let mut set: Vec<usize> = (0..t.min(n)).collect();
            let mut tweak = 0u8;
            let mut ba_tweak = 0usize;
            let mut mi_tweak = 0usize;
            if let Some(r) = rng.as_deref_mut() {
                let c = match r.below(10) {
                    0..=5 => t,
                    6 => t.saturating_sub(1),
                    7 => n,
                    8 => t + 1,
                    _ => r.below(n + 1),
                }
                .min(n);
                // c distinct indices, in order.
                let mut all: Vec<usize> = (0..n).collect();
                for j in (1..n).rev() {
                    all.swap(j, r.below(j + 1));
                }
                set = all[..c].to_vec();
                set.sort_unstable();
                tweak = match r.below(40) {
                    0 => 1,
                    1 => 2,
                    2 => 3,
                    _ => 0,
                };
                ba_tweak = if r.below(30) < 5 { 1 + r.below(6) } else { 0 };
                mi_tweak = match r.below(30) {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
            }
            let mut mode_infos = Vec::new();
            let mut subs = Vec::new();
            for &j in &set {
                let (mi, p) = ms_plan(rng.as_deref_mut(), &members[j], mode);
                mode_infos.push(mi);
                subs.push(p);
            }
            let mut ba = Some(bit_array(n, &set));
            if let Some(b) = ba.as_mut() {
                match ba_tweak {
                    1 => b.extra_bits_stored = ((n + 1) % 8) as u32,
                    2 => b.elems.push(0xff),
                    3 => {
                        b.elems.pop();
                    }
                    4 => {
                        // A bit past the keys (in the last byte's padding).
                        let pad = n % 8;
                        if pad != 0
                            && let Some(last) = b.elems.last_mut()
                        {
                            *last |= 1 << (7 - pad);
                        }
                    }
                    5 => b.extra_bits_stored = 9,
                    _ => {}
                }
            }
            if ba_tweak == 6 {
                ba = None;
            }
            match mi_tweak {
                1 => {
                    mode_infos.pop();
                }
                2 => mode_infos.push(single_mode_info(mode)),
                _ => {}
            }
            (
                ModeInfo {
                    sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                        bitarray: ba,
                        mode_infos,
                    })),
                },
                Plan::Multi { subs, tweak },
            )
        }
    }
}

/// The signature bytes of `p` over `direct` (the `SignDoc`) or `amino`
/// (the amino-JSON sign bytes, when they could be built).
fn sign_plan(accts: &[Acct], p: &Plan, direct: &[u8], amino: Option<&[u8]>) -> Vec<u8> {
    match p {
        Plan::Single {
            acct,
            mode,
            corrupt,
        } => {
            let bytes = if *mode == SignMode::LegacyAminoJson {
                amino
            } else {
                Some(direct)
            };
            let mut sig = bytes
                .and_then(|b| accts[*acct].key.sign(b).ok())
                .map_or_else(|| vec![1u8; 64], |s| s.to_vec());
            if *corrupt {
                sig[7] ^= 1;
            }
            sig
        }
        Plan::Multi { subs, tweak } => {
            let mut signatures: Vec<Vec<u8>> = subs
                .iter()
                .map(|s| sign_plan(accts, s, direct, amino))
                .collect();
            match tweak {
                1 => signatures.push(vec![1u8; 64]),
                2 => {
                    signatures.pop();
                }
                _ => {}
            }
            let mut out =
                celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature { signatures }
                    .encode_to_vec();
            if *tweak == 3 {
                out.extend_from_slice(&[0x18, 0x01]);
            }
            out
        }
    }
}

fn accounts(first_number: u64) -> Vec<Acct> {
    let singles = (0..ACCOUNTS)
        .map(|i| {
            let mut secret = hash::sha256(format!("fuzz-diff deliver {i}").as_bytes());
            secret[0] &= 0x7f;
            let pub_key =
                crypto::secp256k1::public_key_from_secret_key(&secret).unwrap_or([2u8; 33]);
            let key = keyring::KeyRecord {
                name: format!("fuzz{i}"),
                pub_key,
                secret: Some(secret),
                kind: keyring::KeyKind::Local,
            };
            let acc = key.bech32("celestia");
            let val = key.bech32("celestiavaloper");
            Acct {
                addr: key.address().to_vec(),
                pk: key.pubkey_any(),
                key,
                acc,
                val,
                number: first_number + i as u64,
                seq: 0,
                ms: None,
            }
        })
        .collect::<Vec<_>>();
    let mut out = singles;
    for (j, t) in multisig_trees().into_iter().enumerate() {
        let pk = ms_pubkey(&out[..ACCOUNTS], &t);
        let addr = pk.address().unwrap_or_default();
        out.push(Acct {
            key: out[0].key.clone(),
            acc: encoding::bech32::encode("celestia", &addr).unwrap_or_default(),
            val: encoding::bech32::encode("celestiavaloper", &addr).unwrap_or_default(),
            number: first_number + (ACCOUNTS + j) as u64,
            seq: 0,
            addr,
            pk: pk.to_any(),
            ms: Some(t),
        });
    }
    out
}

fn zero_threshold_account(singles: &[Acct], number: u64) -> Acct {
    let tree = MsKey::Multi {
        threshold: 0,
        members: vec![MsKey::Single(0)],
    };
    let pk = ms_pubkey(&singles[..ACCOUNTS], &tree);
    let addr = pk.address().unwrap_or_default();
    Acct {
        key: singles[0].key.clone(),
        acc: encoding::bech32::encode("celestia", &addr).unwrap_or_default(),
        val: encoding::bech32::encode("celestiavaloper", &addr).unwrap_or_default(),
        number,
        seq: 0,
        addr,
        pk: pk.to_any(),
        ms: Some(tree),
    }
}

fn p256_account(singles: &[Acct], number: u64) -> anyhow::Result<Acct> {
    let secret = p256::ecdsa::SigningKey::from_slice(&[42; 32])?;
    let key = secret
        .verifying_key()
        .to_sec1_point(true)
        .as_bytes()
        .to_vec();
    let pk = x_ante::pubkey::PubKey::Secp256r1(key);
    let addr = pk
        .address()
        .map_err(|error| anyhow::anyhow!("P-256 address: {error:?}"))?;
    Ok(Acct {
        key: singles[0].key.clone(),
        acc: encoding::bech32::encode("celestia", &addr)?,
        val: encoding::bech32::encode("celestiavaloper", &addr)?,
        number,
        seq: 0,
        addr,
        pk: pk.to_any(),
        ms: None,
    })
}

fn controlled_secp_account(number: u64, case: &str) -> Acct {
    let mut secret = hash::sha256(format!("fuzz-diff pubkey rejection {case}").as_bytes());
    secret[0] &= 0x7f;
    let pub_key = crypto::secp256k1::public_key_from_secret_key(&secret).unwrap_or([2u8; 33]);
    let key = keyring::KeyRecord {
        name: format!("pubkey-{case}"),
        pub_key,
        secret: Some(secret),
        kind: keyring::KeyKind::Local,
    };
    Acct {
        addr: key.address().to_vec(),
        pk: key.pubkey_any(),
        acc: key.bech32("celestia"),
        val: key.bech32("celestiavaloper"),
        key,
        number,
        seq: 0,
        ms: None,
    }
}

fn controlled_ed25519_account(singles: &[Acct], number: u64) -> anyhow::Result<Acct> {
    let pk = x_ante::pubkey::PubKey::Ed25519(vec![45; 32]);
    let addr = pk
        .address()
        .map_err(|error| anyhow::anyhow!("Ed25519 address: {error:?}"))?;
    Ok(Acct {
        key: singles[0].key.clone(),
        acc: encoding::bech32::encode("celestia", &addr)?,
        val: encoding::bech32::encode("celestiavaloper", &addr)?,
        number,
        seq: 0,
        addr,
        pk: pk.to_any(),
        ms: None,
    })
}

fn max_account_number(v: &serde_json::Value, max: &mut u64) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                if k == "account_number" {
                    if let Some(n) = x.as_str().and_then(|s| s.parse::<u64>().ok()) {
                        *max = (*max).max(n);
                    }
                } else {
                    max_account_number(x, max);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| max_account_number(x, max)),
        _ => {}
    }
}

/// Adds the fuzz accounts (auth accounts, bank balances, supply) to the
/// InitChain app state; returns the patched state and the accounts.
fn patch_app_state(app_state: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<Acct>)> {
    let mut g: serde_json::Value = serde_json::from_slice(app_state)?;
    if std::env::var_os("DELIVER_NEVER_BONDED_UNJAIL_REQUIRE").is_some()
        || std::env::var_os("DELIVER_UNBONDED_REDELEGATION_ROLLBACK_REQUIRE").is_some()
        || std::env::var_os("DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE").is_some()
    {
        g["staking"]["params"]["max_validators"] = serde_json::Value::from(1);
    }
    if std::env::var_os("DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE").is_some() {
        g["distribution"]["params"]["community_tax"] =
            serde_json::Value::from("0.000000000000000000");
    }
    if std::env::var("DELIVER_EVIDENCE_REQUIRE").as_deref()
        == Ok(EvidenceRequirement::ZeroDoubleSignSlash.name())
    {
        g["slashing"]["params"]["slash_fraction_double_sign"] =
            serde_json::Value::from("0.000000000000000000");
    }
    let mut max = 0;
    max_account_number(&g["auth"]["accounts"], &mut max);
    let mut accts = accounts(max + 1);
    if std::env::var_os("DELIVER_ZERO_THRESHOLD_MULTISIG_REQUIRE").is_some() {
        let number = max + 1 + accts.len() as u64;
        let controlled = zero_threshold_account(&accts, number);
        accts.push(controlled);
    }
    if std::env::var_os("DELIVER_P256_REQUIRE").is_some() {
        let number = max + 1 + accts.len() as u64;
        let controlled = p256_account(&accts, number)?;
        accts.push(controlled);
    }
    if std::env::var_os("DELIVER_PUBKEY_REJECTION_REQUIRE").is_some() {
        let number = max + 1 + accts.len() as u64;
        accts.push(controlled_secp_account(number, "secp"));
        accts.push(controlled_secp_account(number + 1, "ed-malformed"));
        accts.push(controlled_ed25519_account(&accts, number + 2)?);
    }
    let accs = g["auth"]["accounts"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("auth.accounts"))?;
    for a in &accts {
        accs.push(serde_json::json!({
            "@type": "/cosmos.auth.v1beta1.BaseAccount",
            "address": a.acc,
            "pub_key": null,
            "account_number": a.number.to_string(),
            "sequence": "0",
        }));
    }
    let bals = g["bank"]["balances"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("bank.balances"))?;
    for a in &accts {
        bals.push(serde_json::json!({
            "address": a.acc,
            "coins": [{"denom": "utia", "amount": BALANCE.to_string()}],
        }));
    }
    if let Some(sup) = g["bank"]["supply"].as_array_mut() {
        for c in sup.iter_mut() {
            if c["denom"] == "utia" {
                let n: u128 = c["amount"].as_str().unwrap_or("0").parse()?;
                c["amount"] =
                    serde_json::Value::String((n + BALANCE * accts.len() as u128).to_string());
            }
        }
    }
    if std::env::var("DELIVER_KEEP_PARAMS").is_err() {
        let set = |v: &mut serde_json::Value, k: &str, x: &str| {
            if v.get(k).is_some() {
                v[k] = serde_json::Value::String(x.to_owned());
            }
        };
        set(&mut g["slashing"]["params"], "signed_blocks_window", "20");
        set(
            &mut g["slashing"]["params"],
            "min_signed_per_window",
            "0.500000000000000000",
        );
        set(&mut g["staking"]["params"], "unbonding_time", "600s");
        set(&mut g["gov"]["params"], "voting_period", "120s");
        set(&mut g["gov"]["params"], "expedited_voting_period", "60s");
    }
    Ok((serde_json::to_vec(&g)?, accts))
}

// ---------------------------------------------------------------------
// Values.

/// Values seen in the scenario's messages and in delivered blocks' events,
/// by field / attribute name.
#[derive(Default)]
struct Dict {
    strings: BTreeMap<String, Vec<Vec<u8>>>,
    ints: BTreeMap<String, Vec<u64>>,
    msgs: BTreeMap<String, Vec<Vec<u8>>>,
}

fn push_bounded<T: PartialEq>(v: &mut Vec<T>, x: T, cap: usize) {
    if v.contains(&x) {
        return;
    }
    if v.len() >= cap {
        v.remove(0);
    }
    v.push(x);
}

impl Dict {
    fn string(&mut self, k: &str, v: &[u8]) {
        if v.len() <= 512 {
            push_bounded(
                self.strings.entry(k.to_owned()).or_default(),
                v.to_vec(),
                48,
            );
        }
    }

    fn harvest(&mut self, desc: &MessageDescriptor, b: &[u8], depth: usize) {
        if depth > 8 {
            return;
        }
        if b.len() <= 4096 {
            push_bounded(
                self.msgs.entry(desc.full_name().to_owned()).or_default(),
                b.to_vec(),
                24,
            );
        }
        let Some(fs) = parse_fields(b) else {
            return;
        };
        for f in fs {
            let Some(fd) = u32::try_from(f.num).ok().and_then(|n| desc.get_field(n)) else {
                continue;
            };
            match fd.kind() {
                Kind::String if f.wt == 2 => self.string(fd.name(), &f.val),
                Kind::Message(md) if f.wt == 2 => {
                    if md.full_name() == "google.protobuf.Any" {
                        if let Some(any) = parse_fields(&f.val) {
                            let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                            if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                                for a in &any {
                                    if a.num == 2 && a.wt == 2 {
                                        self.harvest(&inner, &a.val, depth + 1);
                                    }
                                }
                            }
                        }
                    } else {
                        self.harvest(&md, &f.val, depth + 1);
                    }
                }
                _ if f.wt == 0 => {
                    if let Some(v) = varint(&f.val) {
                        push_bounded(self.ints.entry(fd.name().to_owned()).or_default(), v, 32);
                    }
                }
                _ => {}
            }
        }
    }

    fn harvest_events(&mut self, events: &[abci::Event]) {
        for e in events {
            for a in &e.attributes {
                let v = gostr::to_go_bytes(&a.value);
                self.string(&a.key, &v);
                if let Ok(n) = std::str::from_utf8(&v).unwrap_or("x").parse::<u64>() {
                    push_bounded(self.ints.entry(a.key.clone()).or_default(), n, 32);
                }
            }
        }
    }
}

fn varint(b: &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate().take(10) {
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

fn pool() -> &'static prost_reflect::DescriptorPool {
    x_ante::signing::pool()
}

fn desc_for_type_url(type_url: &[u8]) -> Option<MessageDescriptor> {
    let s = std::str::from_utf8(type_url).ok()?;
    pool().get_message_by_name(s.rsplit('/').next()?)
}

/// The `cosmos.msg.v1.signer` field names of `md` (empty when it is not an
/// `sdk.Msg`).
fn signer_fields(md: &MessageDescriptor) -> Vec<String> {
    let Some(ext) = pool().get_extension_by_name("cosmos.msg.v1.signer") else {
        return Vec::new();
    };
    let o = md.options();
    if !o.has_extension(&ext) {
        return Vec::new();
    }
    match o.get_extension(&ext).into_owned() {
        prost_reflect::Value::List(l) => l
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Every `sdk.Msg` type of the registry.
fn msg_types() -> &'static [MessageDescriptor] {
    static T: std::sync::OnceLock<Vec<MessageDescriptor>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let filter = std::env::var("DELIVER_TYPES").ok();
        pool()
            .all_messages()
            .filter(|md| !signer_fields(md).is_empty())
            .filter(|md| {
                filter
                    .as_deref()
                    .is_none_or(|f| f.split(',').any(|p| md.full_name().contains(p)))
            })
            .collect()
    })
}

fn scalar_of(f: &FieldDescriptor) -> Option<String> {
    let e = pool().get_extension_by_name("cosmos_proto.scalar")?;
    let o = f.options();
    if !o.has_extension(&e) {
        return None;
    }
    match o.get_extension(&e).into_owned() {
        prost_reflect::Value::String(s) => Some(s),
        _ => None,
    }
}

const MAX_U256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";
const POW_256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639936";

/// A `math.Int` string: boundaries and plausible amounts.
fn int_value(m: &mut Mutator) -> Vec<u8> {
    let s: String = match m.rng.below(24) {
        0 => "0".into(),
        1 => "1".into(),
        2 => "-1".into(),
        3 => MAX_U256.into(),
        4 => POW_256.into(),
        5 => format!("-{MAX_U256}"),
        6 => "9223372036854775807".into(),
        7 => "18446744073709551615".into(),
        8 => "18446744073709551616".into(),
        9 => String::new(),
        10 => "01".into(),
        11 => "+1".into(),
        12 => " 1".into(),
        13 => "1.0".into(),
        14 => "1e3".into(),
        15 => "0x10".into(),
        16 => "-0".into(),
        17 => "1".repeat(78),
        18 => format!("{}", 1u64 << (m.rng.below(62) + 1)),
        _ => format!("{}", 1 + m.rng.below(1_000_000_000)),
    };
    s.into_bytes()
}

/// A `LegacyDec` wire string (the value times 10^18, as gogoproto writes
/// it) or a malformed one.
fn dec_value(m: &mut Mutator) -> Vec<u8> {
    let s: String = match m.rng.below(20) {
        0 => "0".into(),
        1 => "1".into(),
        2 => "1000000000000000000".into(),
        3 => "999999999999999999".into(),
        4 => "1000000000000000001".into(),
        5 => "-1".into(),
        6 => "-1000000000000000000".into(),
        7 => MAX_U256.into(),
        8 => format!("{MAX_U256}0"),
        9 => String::new(),
        10 => "0.5".into(),
        11 => "500000000000000000".into(),
        12 => "100000000000000000".into(),
        13 => "10000000000000000".into(),
        14 => "1".repeat(95),
        15 => "1".repeat(96),
        _ => format!("{}", m.rng.below(1_000_000_000_000_000_000)),
    };
    s.into_bytes()
}

fn denom_value(m: &mut Mutator, dict: &Dict) -> Vec<u8> {
    match m.rng.below(20) {
        0..=7 => b"utia".to_vec(),
        8 => Vec::new(),
        9 => b"u".to_vec(),
        10 => b"UTIA".to_vec(),
        11 => {
            let mut s = b"a".to_vec();
            s.extend(std::iter::repeat_n(b'b', 127 + m.rng.below(2)));
            s
        }
        12 => format!("ibc/{}", "A1".repeat(32)).into_bytes(),
        13 => format!("ibc/{}", "a1".repeat(32)).into_bytes(),
        14 => b"1abc".to_vec(),
        15 => b"a b".to_vec(),
        16 => b"transfer/channel-0/utia".to_vec(),
        17 => b"hyperlane/0x0000".to_vec(),
        _ => dict
            .strings
            .get("denom")
            .filter(|v| !v.is_empty())
            .map_or_else(|| b"uatom".to_vec(), |v| v[m.rng.below(v.len())].clone()),
    }
}

fn module_address(name: &str) -> String {
    let h = hash::sha256(name.as_bytes());
    encoding::bech32::encode("celestia", &h[..20]).unwrap_or_default()
}

const MODULES: &[&str] = &[
    "fee_collector",
    "distribution",
    "mint",
    "bonded_tokens_pool",
    "not_bonded_tokens_pool",
    "gov",
    "transfer",
    "hyperlane",
    "warp",
    "fibre",
    "interchainaccounts",
    "forwarding",
];

/// The parties a generated message refers to.
struct Who {
    /// Signer account (bech32) and validator operator forms.
    acc: String,
    val: String,
}

// ---------------------------------------------------------------------
// Generation.

struct Gen {
    m: Mutator,
    dict: Dict,
    accts: Vec<Acct>,
    /// Scenario messages (top-level and nested) to rewrite and mutate.
    scenario_msgs: Vec<Any>,
    /// Validator operators known to exist.
    validators: Vec<String>,
    /// Proposal ids seen.
    proposals: Vec<u64>,
    /// (delegator index, validator, creation height) of unbondings.
    unbondings: Vec<(usize, String, i64)>,
    /// (proposal id, proposer index) of proposals the fuzz accounts made.
    proposers: Vec<(u64, usize)>,
    /// Gas Go used for txs that succeeded, by first message type: gas
    /// limits just below it run out of gas inside the message handlers.
    gas_seen: BTreeMap<String, u64>,
    /// Hyperlane objects the fuzz accounts created: (kind, id, owner
    /// index); kinds [`HL_MAILBOX`], [`HL_ISM`], [`HL_ROUTING_ISM`],
    /// [`HL_HOOK`], [`HL_IGP`], [`HL_TOKEN`].
    hl_objects: Vec<(u8, String, usize)>,
    /// What the fuzz accounts' hyperlane objects are wired to.
    hl: HlState,
    hex_ids: Vec<String>,
    chain_id: String,
    app_version: u64,
    height: i64,
    /// The `sdk.Msg` types the decoder knows at `app_version`.
    types: Vec<MessageDescriptor>,
    /// Sign every tx in `SIGN_MODE_LEGACY_AMINO_JSON` ([`amino_sign_seeds`]).
    force_amino: bool,
    /// Systematic wire-level mutations (`DELIVER_WIRE`).
    wire: WireGen,
    /// Gov-authority messages built from the app's current parameters
    /// (`DELIVER_GOVPASS`, see [`authority_msgs`]).
    gov_msgs: Vec<Any>,
    /// An account that unrelated generated txs must not sign with.
    reserved_account: Option<usize>,
}

/// State of `DELIVER_WIRE`: messages get a [`crate::wire`] mutation of
/// the class used least so far.
#[derive(Default)]
struct WireGen {
    on: bool,
    /// Mutations applied, by class.
    counts: BTreeMap<&'static str, u64>,
    /// Classes applied to the messages of the tx being built.
    pending: Vec<&'static str>,
    /// Classes of the generated txs not yet finalized.
    txs: BTreeMap<Vec<u8>, Vec<&'static str>>,
}

/// `DELIVER_HL`: the hyperlane success paths in order, one tx per
/// generated block from a dedicated fuzz account, each step using the ids
/// the earlier ones returned: an ISM (noop by default), a mailbox on it, an IGP with a
/// gas config, a merkle tree hook, the mailbox's hooks, a collateral
/// token with an enrolled router, a transfer out (the IGP charges its
/// fee), `MsgPayForGas`, `MsgClaim` (fees collected), `MsgProcessMessage`
/// of a transfer in (the selected ISM verifies it), a synthetic token, a
/// transfer in minting it and one out burning it; then the transfer /
/// payment / claim / process steps repeat. Steps 16 and 17 (first pass,
/// app version 10) propose a `MsgCreateNativeSyntheticToken` (gov-only)
/// and have every fuzz account vote yes.
#[derive(Default)]
struct HlSeq {
    verifier: hyperlane::Verifier,
    on: bool,
    step: usize,
    tries: usize,
    /// The tx of the current step, until it is finalized.
    pending: Option<Vec<u8>>,
    ism: String,
    mailbox: String,
    igp: String,
    hook: String,
    token: String,
    synth: String,
    message_id: String,
    nonce: u32,
    /// The fuzz accounts delegated for step 16 vote.
    delegated: bool,
    /// App version 10 (step 16 needs `MsgCreateNativeSyntheticToken`).
    v10: bool,
    /// The proposal of step 16 (`MsgCreateNativeSyntheticToken`).
    proposal: u64,
    /// Go's result of each step: (step, type URL, code, log).
    results: Vec<String>,
}

struct HlRecoveryIdCase {
    wire_id: u8,
    tx: Vec<u8>,
    validator: [u8; 20],
    location: String,
    check: Option<abci::ResponseCheckTx>,
}

struct HlRecoveryIdBlock {
    cases: Vec<HlRecoveryIdCase>,
    good: Vec<u8>,
}

/// The fuzz account `DELIVER_HL` signs with (the IBC relayer is the last).
const HL_ACCT: usize = ACCOUNTS - 2;
/// `DELIVER_HL`'s mailbox domain and its remote domain.
const HL_LOCAL: u32 = 77;
const HL_REMOTE: u32 = 88;
/// `DELIVER_HL`'s enrolled remote router (the sender of incoming messages).
const HL_ROUTER: &str = "0x00000000000000000000000000000000000000000000000000000000000ab0c0";

impl HlSeq {
    const LAST: usize = 17;
    const REPEAT_FROM: usize = 8;

    fn msgs(&mut self, gn: &Gen) -> Vec<Any> {
        let message = self.msg(gn);
        if self.step == 2 {
            // The mailbox id returned by step 1 makes this a real successful
            // announcement rather than a state-independent generated shape.
            match hyperlane::announcement(
                gn.accts[HL_ACCT].acc.clone(),
                self.mailbox.clone(),
                HL_LOCAL,
            ) {
                Ok(announcement) => vec![message, announcement],
                Err(error) => {
                    // A prerequisite can fail three times and be skipped. Keep
                    // the sequence's retry/skip policy; an explicit registry
                    // requirement will fail at the final evidence gate.
                    self.results
                        .push(format!("step 2 announcement unavailable: {error}"));
                    vec![message]
                }
            }
        } else if self.verifier.name() != "noop" && matches!(self.step, 11 | 14) {
            // Other fuzz txs can change mailbox/token overrides. Selecting
            // the immutable multisig immediately before verification in the
            // same atomic tx prevents a noop override from counting as
            // successful signed coverage.
            vec![
                Any::pack(&celestia_proto::hyperlane::warp::v1::MsgSetToken {
                    owner: gn.accts[HL_ACCT].acc.clone(),
                    token_id: if self.step == 11 {
                        self.token.clone()
                    } else {
                        self.synth.clone()
                    },
                    ism_id: self.ism.clone(),
                    new_owner: String::new(),
                    renounce_ownership: false,
                }),
                message,
            ]
        } else {
            vec![message]
        }
    }

    fn msg(&mut self, gn: &Gen) -> Any {
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = gn.accts[HL_ACCT].acc.clone();
        let utia = |n: u64| Coin {
            denom: "utia".into(),
            amount: n.to_string(),
        };
        let router = |token: &str| warp::MsgEnrollRemoteRouter {
            owner: me.clone(),
            token_id: token.to_owned(),
            remote_router: Some(warp::RemoteRouter {
                receiver_domain: HL_REMOTE,
                receiver_contract: HL_ROUTER.into(),
                gas: "50000".into(),
            }),
        };
        let transfer = |token: &str, amount: u64| warp::MsgRemoteTransfer {
            sender: me.clone(),
            token_id: token.to_owned(),
            destination_domain: HL_REMOTE,
            recipient: format!("0x{}", "11".repeat(32)),
            amount: amount.to_string(),
            custom_hook_id: String::new(),
            gas_limit: "0".into(),
            max_fee: Some(utia(10_000_000)),
            custom_hook_metadata: String::new(),
        };
        match self.step {
            0 => self.verifier.create(me),
            1 => Any::pack(&core::MsgCreateMailbox {
                owner: me,
                local_domain: HL_LOCAL,
                default_ism: self.ism.clone(),
                default_hook: String::new(),
                required_hook: String::new(),
            }),
            2 => Any::pack(&pd::MsgCreateIgp {
                owner: me,
                denom: "utia".into(),
            }),
            3 => Any::pack(&pd::MsgSetDestinationGasConfig {
                owner: me,
                igp_id: self.igp.clone(),
                destination_gas_config: Some(pd::DestinationGasConfig {
                    remote_domain: HL_REMOTE,
                    gas_oracle: Some(pd::GasOracle {
                        token_exchange_rate: "10000000000".into(),
                        gas_price: "1".into(),
                    }),
                    gas_overhead: "1000".into(),
                }),
            }),
            4 => Any::pack(&pd::MsgCreateMerkleTreeHook {
                owner: me,
                mailbox_id: self.mailbox.clone(),
            }),
            5 => Any::pack(&core::MsgSetMailbox {
                owner: me,
                mailbox_id: self.mailbox.clone(),
                default_ism: String::new(),
                default_hook: self.igp.clone(),
                required_hook: self.hook.clone(),
                new_owner: String::new(),
                renounce_ownership: false,
            }),
            6 => Any::pack(&warp::MsgCreateCollateralToken {
                owner: me,
                origin_mailbox: self.mailbox.clone(),
                origin_denom: "utia".into(),
            }),
            7 => Any::pack(&router(&self.token)),
            8 => Any::pack(&transfer(&self.token, 5000)),
            9 => Any::pack(&pd::MsgPayForGas {
                sender: me,
                igp_id: self.igp.clone(),
                message_id: self.message_id.clone(),
                destination_domain: HL_REMOTE,
                gas_limit: "10000".into(),
                amount: Some(utia(100_000)),
            }),
            10 => Any::pack(&pd::MsgClaim {
                sender: me,
                igp_id: self.igp.clone(),
            }),
            11 | 14 => {
                let token = if self.step == 11 {
                    self.token.clone()
                } else {
                    self.synth.clone()
                };
                self.nonce += 1;
                let hex32 = |s: &str| {
                    let b = hex::decode(s.trim_start_matches("0x")).unwrap_or_default();
                    let mut out = [0u8; 32];
                    let n = b.len().min(32);
                    out[32 - n..].copy_from_slice(&b[b.len() - n..]);
                    out
                };
                let mut m = vec![3u8];
                m.extend_from_slice(&self.nonce.to_be_bytes());
                m.extend_from_slice(&HL_REMOTE.to_be_bytes());
                m.extend_from_slice(&hex32(HL_ROUTER));
                m.extend_from_slice(&HL_LOCAL.to_be_bytes());
                m.extend_from_slice(&hex32(&token));
                let mut rcpt = [0u8; 32];
                rcpt[12..].copy_from_slice(&gn.accts[HL_ACCT].addr[..20]);
                m.extend_from_slice(&rcpt);
                let mut amount = [0u8; 32];
                amount[24..].copy_from_slice(&1000u64.to_be_bytes());
                m.extend_from_slice(&amount);
                let metadata = self.verifier.metadata(HL_REMOTE, &m, self.nonce);
                Any::pack(&core::MsgProcessMessage {
                    mailbox_id: self.mailbox.clone(),
                    relayer: me,
                    metadata: if metadata.is_empty() {
                        String::new()
                    } else {
                        format!("0x{}", hex::encode(metadata))
                    },
                    message: format!("0x{}", hex::encode(m)),
                })
            }
            12 => Any::pack(&warp::MsgCreateSyntheticToken {
                owner: me,
                origin_mailbox: self.mailbox.clone(),
            }),
            13 => Any::pack(&router(&self.synth)),
            15 => Any::pack(&transfer(&self.synth, 500)),
            16 => Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                messages: vec![Any::pack(&warp::MsgCreateNativeSyntheticToken {
                    owner: GOV.into(),
                    origin_mailbox: self.mailbox.clone(),
                    origin_denom: format!("native{}", self.nonce),
                })],
                initial_deposit: vec![utia(100_000_000_000)],
                proposer: me,
                metadata: String::new(),
                title: "native".into(),
                summary: "native".into(),
                expedited: false,
            }),
            _ => Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                proposal_id: self.proposal,
                voter: me,
                option: 1,
                metadata: String::new(),
            }),
        }
    }

    /// Records Go's result of the step's tx and moves on (a failed step is
    /// retried twice, then skipped).
    fn result(&mut self, type_url: &str, res: &abci::ExecTxResult) {
        self.results.push(format!(
            "step {} {type_url} code {}/{} gas {}: {}",
            self.step,
            res.codespace,
            res.code,
            res.gas_used,
            gostr::to_go_bytes(&res.log)
                .escape_ascii()
                .to_string()
                .chars()
                .take(160)
                .collect::<String>()
        ));
        self.pending = None;
        // The delegations went into the same block as the step's tx.
        self.delegated |= self.step == 16;
        if res.code != 0 {
            self.tries += 1;
            if self.tries < 3 {
                return;
            }
        }
        if res.code == 0 {
            let id =
                celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(res.data.as_slice())
                    .ok()
                    .and_then(|d| d.msg_responses.into_iter().next())
                    .and_then(|r| {
                        parse_fields(&r.value)
                            .unwrap_or_default()
                            .into_iter()
                            .find(|f| f.num == 1 && f.wt == 2)
                            .map(|f| String::from_utf8_lossy(&f.val).into_owned())
                    })
                    .unwrap_or_default();
            match self.step {
                0 => self.ism = id,
                1 => self.mailbox = id,
                2 => self.igp = id,
                4 => self.hook = id,
                6 => self.token = id,
                8 | 15 => self.message_id = id,
                12 => self.synth = id,
                16 => {
                    // `MsgSubmitProposalResponse.proposal_id` (varint 1).
                    self.proposal = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(
                        res.data.as_slice(),
                    )
                    .ok()
                    .and_then(|d| d.msg_responses.into_iter().next())
                    .and_then(|r| {
                        celestia_proto::cosmos::gov::v1::MsgSubmitProposalResponse::decode(
                            r.value.as_slice(),
                        )
                        .ok()
                    })
                    .map_or(0, |r| r.proposal_id);
                }
                _ => {}
            }
        }
        self.tries = 0;
        self.step =
            if self.step >= Self::LAST || (self.step == 15 && (self.proposal != 0 || !self.v10)) {
                Self::REPEAT_FROM
            } else {
                self.step + 1
            };
    }
}

/// Expected Go result of one scripted transaction, in script order.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptExpectation {
    /// Exact top-level message type URL.
    pub type_url: String,
    /// Expected Go codespace; empty for a successful transaction.
    pub codespace: String,
    /// Expected Go result code.
    pub code: u32,
    /// Required substring of the Go result log, if specified.
    #[serde(default)]
    pub log_contains: String,
}

fn script_expectations(text: &str) -> anyhow::Result<Vec<ScriptExpectation>> {
    let expected: Vec<ScriptExpectation> = serde_json::from_str(text)?;
    anyhow::ensure!(
        !expected.is_empty(),
        "DELIVER_SCRIPT_REQUIRE must be nonempty"
    );
    for result in &expected {
        anyhow::ensure!(
            result.type_url.starts_with('/') && result.type_url.len() > 1,
            "DELIVER_SCRIPT_REQUIRE needs an exact message type URL"
        );
        anyhow::ensure!(
            (result.code == 0) == result.codespace.is_empty(),
            "DELIVER_SCRIPT_REQUIRE needs an empty codespace only for success"
        );
    }
    Ok(expected)
}

#[derive(Debug)]
struct TransferAuthzPlan {
    granter: String,
    grantee: String,
    final_grant: celestia_proto::cosmos::authz::v1beta1::Grant,
    successful_transfers: u64,
}

fn transfer_authz_plan(
    script: &std::collections::VecDeque<Vec<(usize, Any, u64)>>,
    expected: &[ScriptExpectation],
) -> anyhow::Result<TransferAuthzPlan> {
    use celestia_proto::cosmos::authz::v1beta1 as authz;
    use celestia_proto::ibc::applications::transfer::v1 as transfer;

    let messages: Vec<_> = script.iter().flatten().collect();
    anyhow::ensure!(
        messages.len() == expected.len() && script.iter().all(|block| block.len() == 1),
        "DELIVER_TRANSFER_AUTHZ_REQUIRE needs one exact scripted transaction per block"
    );
    let mut final_grant = None;
    let mut final_grant_index = None;
    let mut successful_transfers = 0;
    let mut parties = None;
    for (index, ((signer, message, _), outcome)) in messages.into_iter().zip(expected).enumerate() {
        anyhow::ensure!(
            message.type_url == outcome.type_url,
            "transfer authorization fixture type mismatch at {index}"
        );
        match message.type_url.as_str() {
            "/cosmos.authz.v1beta1.MsgGrant" => {
                anyhow::ensure!(
                    *signer == 1,
                    "transfer authorization grants must be signed by account 1"
                );
                let msg = authz::MsgGrant::decode(message.value.as_slice())?;
                let grant = msg.grant.context("transfer authorization fixture grant")?;
                let authorization = grant
                    .authorization
                    .as_ref()
                    .context("transfer authorization fixture authorization")?;
                anyhow::ensure!(
                    authorization.type_url == transfer::TransferAuthorization::type_url(),
                    "transfer authorization fixture contains another grant type"
                );
                if let Some((granter, grantee)) = &parties {
                    anyhow::ensure!(
                        granter == &msg.granter && grantee == &msg.grantee,
                        "transfer authorization fixture parties changed"
                    );
                } else {
                    parties = Some((msg.granter.clone(), msg.grantee.clone()));
                }
                if outcome.code == 0 {
                    final_grant = Some(grant);
                    final_grant_index = Some(index);
                }
            }
            "/cosmos.authz.v1beta1.MsgExec" => {
                anyhow::ensure!(
                    *signer == 0,
                    "transfer authorization executions must be signed by account 0"
                );
                let msg = authz::MsgExec::decode(message.value.as_slice())?;
                let (granter, grantee) =
                    parties.as_ref().context("grant must precede execution")?;
                anyhow::ensure!(
                    &msg.grantee == grantee && msg.msgs.len() == 1,
                    "transfer authorization execution parties/messages changed"
                );
                let inner = &msg.msgs[0];
                anyhow::ensure!(
                    inner.type_url == transfer::MsgTransfer::type_url(),
                    "transfer authorization execution contains another message type"
                );
                let transfer = transfer::MsgTransfer::decode(inner.value.as_slice())?;
                let token = transfer
                    .token
                    .as_ref()
                    .context("transfer authorization token")?;
                anyhow::ensure!(
                    &transfer.sender == granter
                        && transfer.source_port == "transfer"
                        && matches!(transfer.source_channel.as_str(), "channel-0" | "channel-9")
                        && token.denom == "utia"
                        && matches!(
                            token.amount.as_str(),
                            "1" | "100" | "123" | "200" | "250" | "456"
                        ),
                    "transfer authorization execution fields changed at {index}"
                );
                if outcome.code == 0 {
                    successful_transfers += 1;
                }
            }
            _ => anyhow::bail!("unexpected transfer authorization fixture type at {index}"),
        }
    }
    let final_grant_index =
        final_grant_index.context("no successful transfer authorization grant")?;
    anyhow::ensure!(
        expected[final_grant_index + 1..]
            .iter()
            .all(|outcome| outcome.code != 0),
        "final transfer authorization grant must be followed only by handler failures"
    );
    let (granter, grantee) = parties.context("transfer authorization fixture parties")?;
    anyhow::ensure!(
        successful_transfers == 7,
        "transfer authorization fixture must commit seven packets"
    );
    Ok(TransferAuthzPlan {
        granter,
        grantee,
        final_grant: final_grant.context("final transfer authorization grant")?,
        successful_transfers,
    })
}

impl Stats {
    /// Require the exact direct-aux rejection and successful separate-payer
    /// transaction to complete. Matching behavior without the state
    /// postconditions is insufficient evidence.
    pub fn require_direct_aux_coverage(&self) -> anyhow::Result<()> {
        if self.direct_aux_required.is_some() {
            anyhow::ensure!(
                self.direct_aux_completed == 1,
                "incomplete DELIVER_DIRECT_AUX_REQUIRE coverage: expected one completed fixture, observed {}",
                self.direct_aux_completed
            );
        }
        Ok(())
    }

    /// Require all three controlled malformed multisig shapes to reach their
    /// exact terminal signing errors without changing committed state.
    pub fn require_malformed_multisig_coverage(&self) -> anyhow::Result<()> {
        if self.malformed_multisig_required.is_some() {
            anyhow::ensure!(
                self.malformed_multisig_completed == 3,
                "incomplete DELIVER_MALFORMED_MULTISIG_REQUIRE coverage: expected three completed fixtures, observed {}",
                self.malformed_multisig_completed
            );
        }
        Ok(())
    }

    /// Require both zero-threshold multisig sends to execute and persist the
    /// exact wire-decoded key across commits.
    pub fn require_zero_threshold_multisig_coverage(&self) -> anyhow::Result<()> {
        if self.zero_threshold_multisig_required.is_some() {
            anyhow::ensure!(
                self.zero_threshold_multisig_completed == 2,
                "incomplete DELIVER_ZERO_THRESHOLD_MULTISIG_REQUIRE coverage: expected two completed fixtures, observed {}",
                self.zero_threshold_multisig_completed
            );
        }
        Ok(())
    }

    /// Require two successful P-256 sends plus the controlled high-S
    /// rejection to reach their exact runtime postconditions.
    pub fn require_p256_coverage(&self) -> anyhow::Result<()> {
        if self.p256_required.is_some() {
            anyhow::ensure!(
                self.p256_completed == 6,
                "incomplete DELIVER_P256_REQUIRE coverage: expected six completed fixtures, observed {}",
                self.p256_completed
            );
        }
        Ok(())
    }

    /// Require both malformed public-key panics, their successful recovery
    /// sends, and two Ed25519 gas-consumer rejections to reach exact runtime
    /// and committed-state postconditions.
    pub fn require_pubkey_rejection_coverage(&self) -> anyhow::Result<()> {
        if self.pubkey_rejection_required.is_some() {
            anyhow::ensure!(
                self.pubkey_rejection_completed == 6,
                "incomplete DELIVER_PUBKEY_REJECTION_REQUIRE coverage: expected six completed fixtures, observed {}",
                self.pubkey_rejection_completed
            );
            anyhow::ensure!(
                self.log_mismatches.len() == 2
                    && self
                        .log_mismatches
                        .iter()
                        .any(|line| line.contains("length of pubkey is incorrect"))
                    && self
                        .log_mismatches
                        .iter()
                        .any(|line| line.contains("pubkey is incorrect size")),
                "public-key rejection coverage expected exactly the two stack-only malformed-key log differences"
            );
        }
        Ok(())
    }

    /// Require the never-bonded validator to pass the complete jail, stake
    /// restoration, and unjail lifecycle without ever gaining signing info.
    pub fn require_never_bonded_unjail_coverage(&self) -> anyhow::Result<()> {
        if self.never_bonded_unjail_required.is_some() {
            anyhow::ensure!(
                self.never_bonded_unjail_completed == 1,
                "incomplete DELIVER_NEVER_BONDED_UNJAIL_REQUIRE coverage: expected one completed fixture, observed {}",
                self.never_bonded_unjail_completed
            );
        }
        Ok(())
    }

    /// Require the all-shares redelegation from an unbonded validator to fail
    /// after the source-removal path and roll every message-state write back.
    pub fn require_unbonded_redelegation_rollback_coverage(&self) -> anyhow::Result<()> {
        if self.unbonded_redelegation_rollback_required.is_some() {
            anyhow::ensure!(
                self.unbonded_redelegation_rollback_completed == 1,
                "incomplete DELIVER_UNBONDED_REDELEGATION_ROLLBACK_REQUIRE coverage: expected one completed fixture, observed {}",
                self.unbonded_redelegation_rollback_completed
            );
        }
        Ok(())
    }

    /// Require all deterministic Params validation errors to reach handler
    /// execution after a passing tally and preserve their exact terminal state.
    pub fn require_gov_params_rejection_coverage(&self) -> anyhow::Result<()> {
        if self.gov_params_rejection_required.is_some() {
            let expected_cases = gov_params::CASE_NAMES
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<_>>();
            anyhow::ensure!(
                self.gov_params_rejection_completed == 1
                    && self.gov_params_rejection_cases.len() == 29
                    && self
                        .gov_params_rejection_cases
                        .keys()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        == expected_cases
                    && self
                        .gov_params_rejection_cases
                        .values()
                        .all(|count| *count == 1)
                    && self
                        .gov_executed
                        .get("/cosmos.gov.v1.MsgUpdateParams")
                        .copied()
                        .unwrap_or(0)
                        == 0,
                "incomplete DELIVER_GOV_PARAMS_REJECTION_REQUIRE coverage: completed={} cases={:?} executed={:?}",
                self.gov_params_rejection_completed,
                self.gov_params_rejection_cases,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require every named governance message-server boundary exactly once.
    pub fn require_gov_handler_rejection_coverage(&self) -> anyhow::Result<()> {
        if self.gov_handler_rejection_required.is_some() {
            let expected = gov_errors::CASE_NAMES
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<_>>();
            anyhow::ensure!(
                self.gov_handler_rejection_completed == 1
                    && self.gov_handler_rejection_cases.len() == gov_errors::CASE_NAMES.len()
                    && self
                        .gov_handler_rejection_cases
                        .keys()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        == expected
                    && self
                        .gov_handler_rejection_cases
                        .values()
                        .all(|count| *count == 1)
                    && self
                        .gov_executed
                        .get("/cosmos.gov.v1.MsgUpdateParams")
                        .copied()
                        .unwrap_or(0)
                        == 2,
                "incomplete DELIVER_GOV_HANDLER_REJECTION_REQUIRE coverage: completed={} cases={:?} executed={:?}",
                self.gov_handler_rejection_completed,
                self.gov_handler_rejection_cases,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require removal of an unbonded validator to settle integral
    /// commission, retain integral delegator rewards, move decimal dust to
    /// the community pool, and delete every validator-scoped reward record.
    pub fn require_validator_removal_rewards_coverage(&self) -> anyhow::Result<()> {
        if self.validator_removal_rewards_required.is_some() {
            anyhow::ensure!(
                self.validator_removal_rewards_completed == 1,
                "incomplete DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE coverage: expected one completed fixture, observed {}",
                self.validator_removal_rewards_completed
            );
        }
        Ok(())
    }

    /// Require a passed circuit-reset proposal, then separate passed proposals
    /// to schedule, overwrite, cancel and cancel again; five exact failed
    /// plan validations; and a normal signed recovery transaction.
    pub fn require_upgrade_plan_coverage(&self) -> anyhow::Result<()> {
        if self.upgrade_plan_required.is_some() {
            anyhow::ensure!(
                self.upgrade_plan_completed == 1
                    && self.upgrade_plan_rejections == 5
                    && self
                        .gov_executed
                        .get("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")
                        .copied()
                        == Some(2)
                    && self
                        .gov_executed
                        .get("/cosmos.upgrade.v1beta1.MsgCancelUpgrade")
                        .copied()
                        == Some(2)
                    && self
                        .gov_executed
                        .get("/cosmos.circuit.v1.MsgResetCircuitBreaker")
                        .copied()
                        == Some(1),
                "incomplete DELIVER_UPGRADE_PLAN_REQUIRE coverage: completed={} rejections={} executed={:?}",
                self.upgrade_plan_completed,
                self.upgrade_plan_rejections,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require the controlled unknown-handler plan to stop both apps at the
    /// exact due height without advancing committed state.
    pub fn require_upgrade_halt_coverage(&self) -> anyhow::Result<()> {
        if self.upgrade_halt_required.is_some() {
            anyhow::ensure!(
                self.upgrade_halt_completed == 1
                    && self
                        .gov_executed
                        .get("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")
                        .copied()
                        == Some(1)
                    && self
                        .gov_executed
                        .get("/cosmos.circuit.v1.MsgResetCircuitBreaker")
                        .copied()
                        == Some(1),
                "incomplete DELIVER_UPGRADE_HALT_REQUIRE coverage: completed={} executed={:?}",
                self.upgrade_halt_completed,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require the configured due-plan skip and its exact governance setup.
    pub fn require_upgrade_skip_coverage(&self) -> anyhow::Result<()> {
        if self.upgrade_skip_required.is_some() {
            anyhow::ensure!(
                self.upgrade_skip_completed == 1
                    && self
                        .gov_executed
                        .get("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")
                        .copied()
                        == Some(1)
                    && self
                        .gov_executed
                        .get("/cosmos.circuit.v1.MsgResetCircuitBreaker")
                        .copied()
                        == Some(1),
                "incomplete DELIVER_UPGRADE_SKIP_REQUIRE coverage: completed={} executed={:?}",
                self.upgrade_skip_completed,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require the persistent v9 halt, v10 handler retry, second restart and
    /// recovery block after the exact governance setup.
    pub fn require_upgrade_restart_coverage(&self) -> anyhow::Result<()> {
        if self.upgrade_restart_required.is_some() {
            anyhow::ensure!(
                self.upgrade_restart_completed == 1
                    && self
                        .gov_executed
                        .get("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")
                        .copied()
                        == Some(1)
                    && self
                        .gov_executed
                        .get("/cosmos.circuit.v1.MsgResetCircuitBreaker")
                        .copied()
                        == Some(1),
                "incomplete DELIVER_UPGRADE_RESTART_REQUIRE coverage: completed={} executed={:?}",
                self.upgrade_restart_completed,
                self.gov_executed
            );
        }
        Ok(())
    }

    /// Require every scripted transaction, including deliberate handler failures,
    /// to reach FinalizeBlock with the expected Go result. Parity is checked
    /// separately; matching rejection on both apps is insufficient evidence.
    pub fn require_script_coverage(&self) -> anyhow::Result<()> {
        if self.script_required.is_empty() {
            return Ok(());
        }
        anyhow::ensure!(
            self.script_incomplete.is_none()
                && self.script_required.len() == self.script_outcomes.len(),
            "incomplete DELIVER_SCRIPT_REQUIRE: expected {} finalized transactions, observed {} (incomplete: {:?})",
            self.script_required.len(),
            self.script_outcomes.len(),
            self.script_incomplete
        );
        for (index, (expected, (url, result))) in self
            .script_required
            .iter()
            .zip(&self.script_outcomes)
            .enumerate()
        {
            anyhow::ensure!(
                expected.type_url == *url
                    && expected.codespace == result.codespace
                    && expected.code == result.code
                    && result.log.contains(&expected.log_contains),
                "DELIVER_SCRIPT_REQUIRE transaction {index}: expected {expected:?}, observed {url} {result:?}"
            );
        }
        Ok(())
    }

    fn hl_rejection_setup(&self) -> bool {
        self.hl_rejection_cases
            .iter()
            .any(|case| self.hl_retried.get(case).copied().unwrap_or(0) == 0)
    }

    fn observe_hl_pair(
        &mut self,
        case: hyperlane::Rejection,
        bad: &[u8],
        good: &[u8],
        txs: &[Vec<u8>],
        results: &[abci::ExecTxResult],
    ) {
        let bad_result = txs
            .iter()
            .zip(results)
            .enumerate()
            .find(|(_, (tx, _))| tx.as_slice() == bad);
        let good_result = txs
            .iter()
            .zip(results)
            .enumerate()
            .find(|(_, (tx, _))| tx.as_slice() == good);
        let complete = match (bad_result, good_result) {
            (Some((b, (_, rejected))), Some((g, (_, retried)))) => {
                b + 1 == g
                    && rejected.code != 0
                    && rejected.log.contains(case.error_marker())
                    && retried.code == 0
            }
            _ => false,
        };
        if complete {
            *self.hl_retried.entry(case.name().into()).or_default() += 1;
        } else {
            let outcome = |r: Option<(usize, (&Vec<u8>, &abci::ExecTxResult))>| {
                r.map_or_else(
                    || "not finalized".into(),
                    |(index, (_, r))| {
                        format!(
                            "index={index} code={}/{} log={}",
                            r.codespace,
                            r.code,
                            r.log.chars().take(160).collect::<String>()
                        )
                    },
                )
            };
            self.hl_rejection_failures.push(format!(
                "{}: rejected [{}], retry [{}]",
                case.name(),
                outcome(bad_result),
                outcome(good_result)
            ));
        }
    }

    /// Reject missing authority-handler execution even when proposal submission
    /// and unrelated proposal payloads succeeded.
    pub fn require_gov_coverage(&self) -> anyhow::Result<()> {
        for required in &self.gov_required {
            anyhow::ensure!(
                self.gov_executed.get(required).copied().unwrap_or(0) > 0,
                "incomplete DELIVER_GOV_REQUIRE coverage: no executed {required} proposal payload"
            );
        }
        if let Some(required) = &self.gov_failure_required {
            anyhow::ensure!(
                self.gov_failure_matches > 0,
                "incomplete DELIVER_GOV_FAILURE_REQUIRE coverage: no tracked failed proposal with payload {:?} had a log containing {required:?}",
                self.gov_failure_type_required
            );
        }
        if self.gov_abstain_required.is_some() {
            anyhow::ensure!(
                self.gov_abstain_rejections > 0,
                "incomplete DELIVER_GOV_ABSTAIN_REQUIRE coverage: no rejected proposal had an all-abstain final tally"
            );
        }
        Ok(())
    }

    /// Reject a run that did not successfully finalize every explicitly
    /// required top-level message type.
    pub fn require_type_coverage(&self) -> anyhow::Result<()> {
        for required in &self.type_required {
            let successful = self.per_type.get(required).map_or(0, |counts| counts.1);
            anyhow::ensure!(
                successful > 0,
                "incomplete DELIVER_TYPE_REQUIRE coverage: no successful {required} transaction"
            );
        }
        Ok(())
    }

    /// Require the selected controlled misbehavior to reach `FinalizeBlock`
    /// exactly once. Go/Rust result and app-hash comparison establish parity.
    pub fn require_evidence_coverage(&self) -> anyhow::Result<()> {
        let Some(required) = &self.evidence_required else {
            return Ok(());
        };
        anyhow::ensure!(
            self.evidence_injected.get(required).copied() == Some(1)
                && self.evidence_injected.values().sum::<u64>() == 1,
            "incomplete DELIVER_EVIDENCE_REQUIRE coverage: expected one controlled {required} injection, observed {:?}",
            self.evidence_injected
        );
        Ok(())
    }

    /// A successful transaction alone is insufficient for channel-upgrade
    /// Ack coverage: ibc-go uses code 0 for both SUCCESS and FAILURE. Require
    /// the decoded failure response and matching callback error observed by
    /// the mock driver.
    pub fn require_ibc_coverage(&self) -> anyhow::Result<()> {
        if self.ibc_ack_failure_required {
            anyhow::ensure!(
                self.ibc
                    .get("chan_upgrade_ack_failure")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_ACK_FAILURE_REQUIRE coverage: no attributed code-0 MsgChannelUpgradeAck FAILURE response"
            );
        }
        if self.ibc_required.contains("legacy_upgrade_fixture") {
            anyhow::ensure!(
                self.ibc
                    .get("legacy_upgrade_fixture_trusted_height")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_LEGACY_UPGRADE_FIXTURE coverage: trusted consensus height 43 did not finalize"
            );
        }
        if self.ibc_required.contains("close_init_failure") {
            anyhow::ensure!(
                self.ibc.get("close_init_failure").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_CLOSE_INIT_FAILURE_REQUIRE coverage: no attributed MsgChannelCloseInit callback rejection"
            );
            anyhow::ensure!(
                self.ibc
                    .get("close_init_reused_closed")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_CLOSE_INIT_FAILURE_REQUIRE coverage: the rejected channel did not remain usable through counterparty close"
            );
        }
        if self.ibc_required.contains("recover_client") {
            anyhow::ensure!(
                self.ibc.get("misbehaviour_confirmed").copied().unwrap_or(0) > 0,
                "incomplete MsgRecoverClient coverage: no matching confirmed misbehaviour event"
            );
            anyhow::ensure!(
                self.ibc.get("recover_client").copied().unwrap_or(0) > 0,
                "incomplete MsgRecoverClient coverage: no matching block-scope recovery event"
            );
            anyhow::ensure!(
                self.ibc
                    .get("recovered_client_update")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete MsgRecoverClient coverage: recovered subject did not accept a subsequent update"
            );
        }
        if self.ibc_required.contains("frozen_client_update") {
            anyhow::ensure!(
                self.ibc.get("misbehaviour_confirmed").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_FROZEN_CLIENT_REQUIRE coverage: no matching confirmed misbehaviour event"
            );
            anyhow::ensure!(
                self.ibc
                    .get("frozen_client_update_rejected")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_FROZEN_CLIENT_REQUIRE coverage: no exact frozen-client update rejection"
            );
        }
        if self.ibc_required.contains("pfm_lifecycle") {
            anyhow::ensure!(
                self.ibc.get("pfm_forwarded").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_REQUIRE coverage: no exact forwarded packet"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_downstream_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_REQUIRE coverage: no exact successful downstream acknowledgement"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_original_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_REQUIRE coverage: no delayed success acknowledgement for the exact original packet"
            );
        }
        if self.ibc_required.contains("pfm_error_lifecycle") {
            anyhow::ensure!(
                self.ibc.get("pfm_forwarded").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_ERROR_REQUIRE coverage: no exact forwarded packet"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_downstream_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_ERROR_REQUIRE coverage: no exact downstream error acknowledgement"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_original_error_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_ERROR_REQUIRE coverage: no delayed error acknowledgement for the exact original packet"
            );
        }
        if self.ibc_required.contains("pfm_timeout_lifecycle") {
            anyhow::ensure!(
                self.ibc.get("pfm_forwarded").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_TIMEOUT_REQUIRE coverage: no exact forwarded packet"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_timeout").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_TIMEOUT_REQUIRE coverage: no exact forwarded timeout"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_retry").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_TIMEOUT_REQUIRE coverage: no exact PFM retry"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_downstream_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_TIMEOUT_REQUIRE coverage: retried packet was not acknowledged"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_original_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_TIMEOUT_REQUIRE coverage: no delayed success acknowledgement for the exact original packet"
            );
        }
        if self.ibc_required.contains("pfm_exhaust_lifecycle") {
            anyhow::ensure!(
                self.ibc.get("pfm_forwarded").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_EXHAUST_REQUIRE coverage: no exact forwarded packet"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_timeout").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_EXHAUST_REQUIRE coverage: no exact first timeout"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_retry").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_EXHAUST_REQUIRE coverage: no exact configured retry"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_retry_exhausted").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_EXHAUST_REQUIRE coverage: retry did not time out without another retry"
            );
            anyhow::ensure!(
                self.ibc.get("pfm_original_error_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_PFM_EXHAUST_REQUIRE coverage: no exact delayed refund acknowledgement for the original packet"
            );
        }
        if self.ibc_required.contains("pfm_metadata_lifecycle") {
            for event in [
                "pfm_metadata_parse_ack",
                "pfm_metadata_validate_ack",
                "pfm_metadata_null_control",
                "pfm_metadata_committed_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_PFM_METADATA_REQUIRE coverage: expected one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
        }
        if self.ibc_required.contains("ica_encoding_lifecycle") {
            anyhow::ensure!(
                self.ibc.get("ica_proto3_success_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_ICA_ENCODINGS_REQUIRE coverage: no exact successful proto3 packet acknowledgement"
            );
            anyhow::ensure!(
                self.ibc
                    .get("ica_proto3json_success_ack")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_ICA_ENCODINGS_REQUIRE coverage: no exact successful proto3json packet acknowledgement after channel upgrade"
            );
        }
        if self.ibc_required.contains("ica_order_lifecycle") {
            for event in [
                "ica_unordered_baseline_success_ack",
                "ica_upgrade_to_ordered",
                "ica_ordered_success_ack",
                "ica_upgrade_to_unordered",
                "ica_unordered_success_ack",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_ICA_ORDER_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
        }
        if self.ibc_required.contains("upgrade_timeout_recovery") {
            for event in [
                "upgrade_timeout_try",
                "upgrade_timeout_rejected",
                "upgrade_timeout_cleaned",
                "upgrade_timeout_recovery_ack",
                "upgrade_timeout_recovery_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
            anyhow::ensure!(
                self.ibc.get("upgrade_timeout_check_ok").copied() == Some(4),
                "incomplete DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE coverage: expected exactly four accepted controlled CheckTx transactions, observed {:?}",
                self.ibc.get("upgrade_timeout_check_ok")
            );
            for event in [
                "upgrade_timeout_check_unauthorized",
                "upgrade_timeout_check_out_of_gas",
                "upgrade_timeout_check_insufficient_funds",
                "upgrade_timeout_check_mempool_duplicate",
                "upgrade_timeout_check_account_sequence",
                "upgrade_timeout_check_rejected",
                "upgrade_timeout_missing_checktx",
                "upgrade_timeout_unexpected_result",
                "upgrade_timeout_boundary_invalid_proof",
            ] {
                anyhow::ensure!(
                    !self.ibc.contains_key(event),
                    "DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE observed forbidden {event}: {:?}",
                    self.ibc
                );
            }
        }
        if self.ibc_required.contains("upgrade_timeout_boundary") {
            for event in [
                "upgrade_timeout_try",
                "upgrade_timeout_boundary_rejected",
                "upgrade_timeout_cleaned",
                "upgrade_timeout_recovery_ack",
                "upgrade_timeout_recovery_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
            anyhow::ensure!(
                self.ibc.get("upgrade_timeout_check_ok").copied() == Some(4),
                "incomplete DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE coverage: expected exactly four accepted controlled CheckTx transactions, observed {:?}",
                self.ibc.get("upgrade_timeout_check_ok")
            );
            for event in [
                "upgrade_timeout_check_unauthorized",
                "upgrade_timeout_check_out_of_gas",
                "upgrade_timeout_check_insufficient_funds",
                "upgrade_timeout_check_mempool_duplicate",
                "upgrade_timeout_check_account_sequence",
                "upgrade_timeout_check_rejected",
                "upgrade_timeout_missing_checktx",
                "upgrade_timeout_unexpected_result",
                "upgrade_timeout_boundary_invalid_proof",
            ] {
                anyhow::ensure!(
                    !self.ibc.contains_key(event),
                    "DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE observed forbidden {event}: {:?}",
                    self.ibc
                );
            }
        }
        if self.ibc_required.contains("upgrade_confirm_timeout") {
            for event in [
                "upgrade_timeout_try",
                "upgrade_confirm_timeout_aborted",
                "upgrade_timeout_recovery_ack",
                "upgrade_timeout_recovery_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
            anyhow::ensure!(
                self.ibc.get("upgrade_timeout_check_ok").copied() == Some(3),
                "incomplete DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE coverage: expected exactly three accepted controlled CheckTx transactions, observed {:?}",
                self.ibc.get("upgrade_timeout_check_ok")
            );
            for event in [
                "upgrade_timeout_check_unauthorized",
                "upgrade_timeout_check_out_of_gas",
                "upgrade_timeout_check_insufficient_funds",
                "upgrade_timeout_check_mempool_duplicate",
                "upgrade_timeout_check_account_sequence",
                "upgrade_timeout_check_rejected",
                "upgrade_timeout_missing_checktx",
                "upgrade_timeout_unexpected_result",
                "upgrade_confirm_timeout_invalid_proof",
            ] {
                anyhow::ensure!(
                    !self.ibc.contains_key(event),
                    "DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE observed forbidden {event}: {:?}",
                    self.ibc
                );
            }
        }
        if self.ibc_required.contains("upgrade_try_fast_forward") {
            for event in [
                "upgrade_timeout_try",
                "upgrade_try_fast_forward_confirm",
                "upgrade_try_fast_forward_stale_open_rejected",
                "upgrade_try_fast_forward_open",
                "upgrade_timeout_recovery_ack",
                "upgrade_timeout_recovery_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event).copied() == Some(1),
                    "incomplete DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc.get(event)
                );
            }
            anyhow::ensure!(
                self.ibc.get("upgrade_timeout_check_ok").copied() == Some(5),
                "incomplete DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE coverage: expected exactly five accepted controlled CheckTx transactions, observed {:?}",
                self.ibc.get("upgrade_timeout_check_ok")
            );
            for event in [
                "upgrade_timeout_check_unauthorized",
                "upgrade_timeout_check_out_of_gas",
                "upgrade_timeout_check_insufficient_funds",
                "upgrade_timeout_check_mempool_duplicate",
                "upgrade_timeout_check_account_sequence",
                "upgrade_timeout_check_rejected",
                "upgrade_timeout_missing_checktx",
                "upgrade_timeout_unexpected_result",
            ] {
                anyhow::ensure!(
                    !self.ibc.contains_key(event),
                    "DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE observed forbidden {event}: {:?}",
                    self.ibc
                );
            }
        }
        if self.ibc_required.contains("upgrade_open_flushcomplete") {
            for event in [
                "upgrade_timeout_try",
                "upgrade_open_flushcomplete_confirm",
                "upgrade_open_flushcomplete_open",
                "upgrade_timeout_recovery_ack",
                "upgrade_timeout_recovery_state",
            ] {
                anyhow::ensure!(
                    self.ibc.get(event) == Some(&1),
                    "incomplete DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE coverage: expected exactly one {event}, observed {:?}",
                    self.ibc
                );
            }
            anyhow::ensure!(
                self.ibc.get("upgrade_timeout_check_ok") == Some(&4),
                "incomplete DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE coverage: expected exactly four accepted controlled CheckTx transactions, observed {:?}",
                self.ibc
            );
            for event in [
                "upgrade_open_flushcomplete_invalid_proof",
                "upgrade_open_flushcomplete_invalid_counterparty_state",
                "upgrade_timeout_check_unauthorized",
                "upgrade_timeout_check_out_of_gas",
                "upgrade_timeout_check_insufficient_funds",
                "upgrade_timeout_check_mempool_duplicate",
                "upgrade_timeout_check_account_sequence",
                "upgrade_timeout_check_rejected",
                "upgrade_timeout_missing_checktx",
                "upgrade_timeout_unexpected_result",
            ] {
                anyhow::ensure!(
                    !self.ibc.contains_key(event),
                    "DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE observed forbidden {event}: {:?}",
                    self.ibc
                );
            }
        }
        if self.ibc_required.contains("ica_error_ack") {
            anyhow::ensure!(
                self.ibc.get("ica_error_ack").copied().unwrap_or(0) > 0,
                "incomplete DELIVER_IBC_ICA_ERROR_REQUIRE coverage: no exact ICA application-error acknowledgement"
            );
        }
        if self.ibc_required.contains("expired_client_update") {
            anyhow::ensure!(
                self.ibc
                    .get("expired_client_update_rejected")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "incomplete DELIVER_IBC_EXPIRED_CLIENT_REQUIRE coverage: no exact expired-client update rejection"
            );
        }
        Ok(())
    }

    fn gov_setup(&self) -> bool {
        self.gov_required
            .iter()
            .any(|required| self.gov_executed.get(required).copied().unwrap_or(0) == 0)
    }

    fn hl_preupgrade_setup(&self, app_version: u64) -> bool {
        self.hl_preupgrade_required
            && app_version < 10
            && ["collateral", "synthetic"].iter().any(|kind| {
                self.hl_verified_versions
                    .get(&format!("v9/{kind}"))
                    .copied()
                    .unwrap_or(0)
                    == 0
            })
    }

    fn hl_registry_setup(&self) -> bool {
        self.hl_registry_required.is_some()
            && ["merkle-root-ism", "validator-announcement"]
                .iter()
                .any(|handler| self.hl_registry.get(*handler).copied().unwrap_or(0) == 0)
    }

    fn hl_verification_setup(&self) -> bool {
        self.hl_verification
            .as_deref()
            .is_some_and(|mode| mode != "noop")
            && ["collateral", "synthetic"]
                .iter()
                .any(|kind| self.hl_verified.get(*kind).copied().unwrap_or(0) == 0)
    }

    fn ibc_pfm_setup(&self) -> bool {
        (self.ibc_required.contains("pfm_lifecycle")
            || self.ibc_required.contains("pfm_timeout_lifecycle"))
            && self.ibc.get("pfm_original_ack").copied().unwrap_or(0) == 0
            || (self.ibc_required.contains("pfm_error_lifecycle")
                || self.ibc_required.contains("pfm_exhaust_lifecycle"))
                && self.ibc.get("pfm_original_error_ack").copied().unwrap_or(0) == 0
    }

    fn ibc_recover_setup(&self) -> bool {
        self.ibc_required.contains("recover_client")
            && (self.ibc.get("recover_client").copied().unwrap_or(0) == 0
                || self
                    .ibc
                    .get("recovered_client_update")
                    .copied()
                    .unwrap_or(0)
                    == 0)
    }

    /// Require both signed incoming-transfer paths when a multisig ISM was
    /// explicitly selected. Matching rejections alone are incomplete evidence.
    pub fn require_hl_coverage(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.hl_rejection_failures.is_empty(),
            "incomplete Hyperlane rejection/retry evidence: {:?}",
            self.hl_rejection_failures
        );
        anyhow::ensure!(
            !self.hl_rejection_setup(),
            "incomplete Hyperlane rejection/retry coverage: required {:?}, completed {:?}",
            self.hl_rejection_cases,
            self.hl_retried
        );
        if self.hl_recovery_id_required.is_some() {
            anyhow::ensure!(
                self.hl_recovery_id_completed == 1 && self.hl_recovery_id_failures.is_empty(),
                "incomplete DELIVER_HL_RECOVERY_ID_REQUIRE coverage: completed={}, failures={:?}",
                self.hl_recovery_id_completed,
                self.hl_recovery_id_failures
            );
        }
        if self.hl_registry_required.is_some() {
            for handler in ["merkle-root-ism", "validator-announcement"] {
                anyhow::ensure!(
                    self.hl_registry.get(handler).copied().unwrap_or(0) > 0,
                    "incomplete DELIVER_HL_REGISTRY_REQUIRE coverage: no successful controlled {handler} transaction"
                );
            }
        }
        if let Some(mode) = &self.hl_verification
            && mode != "noop"
        {
            for kind in ["collateral", "synthetic"] {
                anyhow::ensure!(
                    self.hl_verified.get(kind).copied().unwrap_or(0) > 0,
                    "incomplete DELIVER_HL_ISM={mode} coverage: no successful {kind} MsgProcessMessage"
                );
            }
            if self.hl_preupgrade_required {
                for kind in ["collateral", "synthetic"] {
                    let key = format!("v9/{kind}");
                    anyhow::ensure!(
                        self.hl_verified_versions.get(&key).copied().unwrap_or(0) > 0,
                        "incomplete DELIVER_UPGRADE signed coverage: no successful v9 {kind} MsgProcessMessage before the binary switch"
                    );
                }
            }
        }
        Ok(())
    }
}

fn legacy_upgrade_fixture_block_index(
    generated: usize,
    paused: &mut usize,
    enabled: bool,
    ready: bool,
    script_at: usize,
) -> (usize, bool) {
    let logical = generated.saturating_sub(*paused);
    let setup = enabled && !ready && logical >= script_at;
    *paused += usize::from(setup);
    (logical, setup)
}

fn script_vote_proposal_id(
    legacy_upgrade_fixture: bool,
    pinned: Option<u64>,
    proposals: &[u64],
) -> anyhow::Result<u64> {
    if legacy_upgrade_fixture {
        pinned.context("legacy upgrade fixture vote has no finalized submission proposal ID")
    } else {
        Ok(proposals.iter().copied().max().unwrap_or(1))
    }
}

fn finalized_script_proposal_id(
    expected_tx: &[u8],
    tx: &[u8],
    result: &abci::ExecTxResult,
) -> anyhow::Result<Option<u64>> {
    if tx != expected_tx || result.code != 0 {
        return Ok(None);
    }
    let responses =
        celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(result.data.as_slice())?;
    let response = responses
        .msg_responses
        .into_iter()
        .find(|response| response.type_url == "/cosmos.gov.v1.MsgSubmitProposalResponse")
        .context("legacy upgrade fixture submission has no typed proposal response")?;
    Ok(Some(
        celestia_proto::cosmos::gov::v1::MsgSubmitProposalResponse::decode(
            response.value.as_slice(),
        )?
        .proposal_id,
    ))
}

fn gov_setup_ready(stats: &Stats, messages: &[Any]) -> bool {
    stats.gov_setup()
        && stats.gov_required.iter().any(|required| {
            stats.gov_executed.get(required).copied().unwrap_or(0) == 0
                && (required != "/ibc.core.client.v1.MsgRecoverClient"
                    || messages.iter().any(|message| &message.type_url == required))
        })
}

fn gov_requirements(value: Option<&str>, govpass: bool) -> anyhow::Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    anyhow::ensure!(govpass, "DELIVER_GOV_REQUIRE requires DELIVER_GOVPASS");
    let mut required = BTreeSet::new();
    for url in value.split(',').map(str::trim) {
        anyhow::ensure!(
            url.starts_with('/')
                && url.len() > 1
                && url[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
            "DELIVER_GOV_REQUIRE expects nonempty exact message type URLs"
        );
        required.insert(url.to_owned());
    }
    Ok(required.into_iter().collect())
}

fn type_requirements(value: Option<&str>) -> anyhow::Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut required = BTreeSet::new();
    for url in value.split(',').map(str::trim) {
        anyhow::ensure!(
            url.starts_with('/')
                && url.len() > 1
                && url[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
            "DELIVER_TYPE_REQUIRE expects nonempty exact message type URLs"
        );
        required.insert(url.to_owned());
    }
    Ok(required.into_iter().collect())
}

fn allow_postupgrade_hl(
    value: Option<&str>,
    scripted: bool,
    gov_failure_required: bool,
) -> anyhow::Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    anyhow::ensure!(
        value == "governance-fixture",
        "DELIVER_HL_ALLOW_POSTUPGRADE must be governance-fixture"
    );
    anyhow::ensure!(
        scripted && gov_failure_required,
        "DELIVER_HL_ALLOW_POSTUPGRADE=governance-fixture requires DELIVER_SCRIPT and DELIVER_GOV_FAILURE_REQUIRE"
    );
    Ok(true)
}

/// A generated, signed tx.
struct Built {
    raw: Vec<u8>,
}

impl Gen {
    fn pick<T: Clone>(&mut self, v: &[T]) -> Option<T> {
        (!v.is_empty()).then(|| v[self.m.rng.below(v.len())].clone())
    }

    /// A fuzz account: mostly a single-key one, sometimes a multisig.
    fn acct_idx(&mut self) -> usize {
        let i = if self.accts.len() > ACCOUNTS && !self.m.rng.one_in(6) {
            self.m.rng.below(ACCOUNTS)
        } else {
            self.m.rng.below(self.accts.len())
        };
        if self.reserved_account == Some(i) {
            // Preserve the RNG stream while moving this signer away from the
            // dedicated account. HL_ACCT is not the first account.
            i - 1
        } else {
            i
        }
    }

    fn uses_reserved_signer(&self, idx: &[Option<usize>]) -> bool {
        self.reserved_account
            .is_some_and(|reserved| idx.iter().flatten().any(|i| *i == reserved))
    }

    fn address_value(&mut self) -> Vec<u8> {
        let r = self.m.rng.below(24);
        match r {
            0..=9 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            10..=12 => {
                let v = self.dict.strings.get("sender").cloned().unwrap_or_default();
                self.pick(&v).unwrap_or_default()
            }
            13 => {
                let n = MODULES[self.m.rng.below(MODULES.len())];
                module_address(n).into_bytes()
            }
            14 => GOV.as_bytes().to_vec(),
            15 => {
                let i = self.acct_idx();
                self.accts[i].val.clone().into_bytes()
            }
            16 => encoding::bech32::encode("cosmos", &[7u8; 20])
                .unwrap_or_default()
                .into_bytes(),
            17 => encoding::bech32::encode("celestia", &[7u8; 32])
                .unwrap_or_default()
                .into_bytes(),
            18 => encoding::bech32::encode("celestia", &[7u8; 19])
                .unwrap_or_default()
                .into_bytes(),
            19 => encoding::bech32::encode("celestia", &[])
                .unwrap_or_default()
                .into_bytes(),
            20 => {
                let i = self.acct_idx();
                self.accts[i].acc.to_uppercase().into_bytes()
            }
            21 => Vec::new(),
            22 => {
                let i = self.acct_idx();
                let mut s = self.accts[i].acc.clone().into_bytes();
                if let Some(l) = s.last_mut() {
                    *l = if *l == b'q' { b'p' } else { b'q' };
                }
                s
            }
            _ => encoding::bech32::encode("celestia", &[0u8; 20])
                .unwrap_or_default()
                .into_bytes(),
        }
    }

    fn valoper_value(&mut self) -> Vec<u8> {
        match self.m.rng.below(11) {
            0..=5 => {
                let v = self.validators.clone();
                self.pick(&v).unwrap_or_default().into_bytes()
            }
            6 | 7 => {
                let i = self.acct_idx();
                self.accts[i].val.clone().into_bytes()
            }
            8 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            // Valid BIP-173 all-uppercase Bech32: decodes to the same bytes but
            // is stored verbatim where Go keeps the original string.
            9 => {
                let i = self.acct_idx();
                self.accts[i].val.to_uppercase().into_bytes()
            }
            _ => self.address_value(),
        }
    }

    fn string_value(&mut self, fd: &FieldDescriptor) -> Option<Vec<u8>> {
        let name = fd.name().to_owned();
        match scalar_of(fd).as_deref() {
            Some("cosmos.ValidatorAddressString") => return Some(self.valoper_value()),
            Some("cosmos.AddressString") => return Some(self.address_value()),
            _ => {}
        }
        if name.contains("denom") {
            return Some(denom_value(&mut self.m, &self.dict));
        }
        if name.contains("address")
            || name == "authority"
            || name == "sender"
            || name == "receiver"
            || name == "owner"
            || name.ends_with("er")
                && [
                    "granter",
                    "grantee",
                    "voter",
                    "depositor",
                    "proposer",
                    "signer",
                ]
                .contains(&name.as_str())
        {
            if self.m.rng.one_in(6) {
                return Some(self.valoper_value());
            }
            return Some(self.address_value());
        }
        if let Some(v) = self.dict.strings.get(&name).cloned()
            && !v.is_empty()
            && !self.m.rng.one_in(3)
        {
            return self.pick(&v);
        }
        Some(match self.m.rng.below(12) {
            0 => return None,
            1 => Vec::new(),
            2 => b"utia".to_vec(),
            3 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            4 => vec![
                b'a';
                [
                    1usize, 63, 64, 65, 70, 71, 128, 129, 140, 141, 255, 256, 257, 280, 281
                ][self.m.rng.below(15)]
            ],
            5 => b"\xff\xfe".to_vec(),
            6 => b"[do-not-modify]".to_vec(),
            7 => format!("0x{}", "00".repeat(32)).into_bytes(),
            8 => b"channel-0".to_vec(),
            9 => b"07-tendermint-0".to_vec(),
            10 => {
                let v = self.hex_ids.clone();
                self.pick(&v).unwrap_or_default().into_bytes()
            }
            _ => b"a".to_vec(),
        })
    }

    fn int_field(&mut self, fd: &FieldDescriptor) -> u64 {
        if let Some(v) = self.dict.ints.get(fd.name()).cloned()
            && !v.is_empty()
            && self.m.rng.one_in(2)
        {
            return self.pick(&v).unwrap_or_default();
        }
        if fd.name().contains("proposal_id") && !self.proposals.is_empty() && self.m.rng.one_in(2) {
            let p = self.proposals.clone();
            return self.pick(&p).unwrap_or_default();
        }
        if fd.name().contains("height") && self.m.rng.one_in(2) {
            let h = u64::try_from(self.height).unwrap_or_default();
            return h.saturating_sub(self.m.rng.below(3) as u64) + self.m.rng.below(3) as u64;
        }
        let neg = matches!(
            fd.kind(),
            Kind::Int32
                | Kind::Int64
                | Kind::Sint32
                | Kind::Sint64
                | Kind::Sfixed32
                | Kind::Sfixed64
        );
        match self.m.rng.below(14) {
            0 => 0,
            1 | 2 => 1,
            3 => 2,
            4 => u64::MAX,
            5 => u64::from(u32::MAX),
            6 => 1 << 63,
            7 => (1 << 63) - 1,
            8 if neg => u64::MAX, // -1
            9 => 100,
            10 => 1_000_000,
            _ => 1 + self.m.rng.below(1000) as u64,
        }
    }

    fn time_value(&mut self, duration: bool) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let (secs, nanos): (i64, i64) = match self.m.rng.below(12) {
            0 => return None,
            1 => (0, 0),
            2 => (-1, 0),
            3 => (253_402_300_799, 999_999_999),
            4 => (253_402_300_800, 0),
            5 => (-62_135_596_800, 0),
            6 => (-62_135_596_801, 0),
            7 => (1, 1_000_000_000),
            8 => (0, -1),
            9 if duration => (315_576_000_001, 0),
            _ => {
                let base = if duration {
                    0
                } else {
                    1_700_000_000 + self.height * 6
                };
                (base + self.m.rng.below(10_000_000) as i64, 0)
            }
        };
        if secs != 0 {
            out.push(8);
            #[allow(clippy::cast_sign_loss)]
            put_varint(&mut out, secs as u64);
        }
        if nanos != 0 {
            out.push(16);
            #[allow(clippy::cast_sign_loss)]
            put_varint(&mut out, nanos as u64);
        }
        Some(out)
    }

    fn bytes_value(&mut self) -> Option<Vec<u8>> {
        let n = [0usize, 1, 20, 32, 33, 64, 65, 100][self.m.rng.below(8)];
        if self.m.rng.one_in(6) {
            return None;
        }
        Some((0..n).map(|_| self.m.rng.next_u64() as u8).collect())
    }

    /// An `Any` for the field `fd` (by name: messages, authorizations,
    /// allowances, keys, content, else something harvested).
    fn any_value(&mut self, fd: &FieldDescriptor, who: &Who, depth: usize) -> Option<Any> {
        let name = fd.name();
        let url = |s: &str| format!("/{s}");
        let pick_type = |g: &mut Self, names: &[&str]| -> Option<MessageDescriptor> {
            let n = names[g.m.rng.below(names.len())];
            pool().get_message_by_name(n)
        };
        let md = if name == "msgs" || name == "messages" {
            let t = self.types.clone();
            return (!t.is_empty() && depth < 3).then(|| {
                let md = t[self.m.rng.below(t.len())].clone();
                Any {
                    type_url: url(md.full_name()),
                    value: self.synth(&md, who, depth + 1),
                }
            });
        } else if name.contains("authorization") {
            pick_type(
                self,
                &[
                    "cosmos.authz.v1beta1.GenericAuthorization",
                    "cosmos.bank.v1beta1.SendAuthorization",
                    "cosmos.staking.v1beta1.StakeAuthorization",
                ],
            )
        } else if name.contains("allowance") {
            pick_type(
                self,
                &[
                    "cosmos.feegrant.v1beta1.BasicAllowance",
                    "cosmos.feegrant.v1beta1.PeriodicAllowance",
                    "cosmos.feegrant.v1beta1.AllowedMsgAllowance",
                ],
            )
        } else if name.contains("pubkey") || name.contains("pub_key") {
            pick_type(
                self,
                &[
                    "cosmos.crypto.ed25519.PubKey",
                    "cosmos.crypto.secp256k1.PubKey",
                ],
            )
        } else if name == "content" {
            pick_type(self, &["cosmos.gov.v1beta1.TextProposal"])
        } else {
            None
        };
        if let Some(md) = md {
            return Some(Any {
                type_url: url(md.full_name()),
                value: self.synth(&md, who, depth + 1),
            });
        }
        // Harvested Anys of that field (IBC client states, headers, ...).
        let k = format!("any:{name}");
        let v = self.dict.msgs.get(&k).cloned().unwrap_or_default();
        let b = self.pick(&v)?;
        Any::decode(b.as_slice()).ok()
    }

    /// The wire occurrences of field `fd` in a synthesized message.
    fn gen_field(&mut self, fd: &FieldDescriptor, who: &Who, depth: usize) -> Vec<Field> {
        let num = u64::from(fd.number());
        let o = x_ante::gogotext::go_opts(fd);
        let repeated = fd.cardinality() == Cardinality::Repeated;
        let n = if repeated {
            match self.m.rng.below(16) {
                0 | 1 => 0,
                2..=10 => 1,
                11..=13 => 2,
                14 => 3 + self.m.rng.below(4),
                _ => 40 + self.m.rng.below(160),
            }
        } else {
            usize::from(!self.m.rng.one_in(10))
        };
        let mut out = Vec::new();
        for _ in 0..n {
            let ct = o.customtype.as_deref().unwrap_or("");
            let val: Option<(u8, Vec<u8>)> = if ct.ends_with("LegacyDec") {
                Some((2, dec_value(&mut self.m)))
            } else if ct.ends_with("math.Int") || ct.ends_with("math.Uint") {
                Some((2, int_value(&mut self.m)))
            } else if ct.ends_with("HexAddress") {
                let v = if self.hex_ids.is_empty() || self.m.rng.one_in(6) {
                    format!("0x{}", "00".repeat(32))
                } else {
                    let h = self.hex_ids.clone();
                    self.pick(&h).unwrap_or_default()
                };
                Some((2, v.into_bytes()))
            } else {
                match fd.kind() {
                    Kind::String => self.string_value(fd).map(|v| (2, v)),
                    Kind::Bytes => self.bytes_value().map(|v| (2, v)),
                    Kind::Bool => Some((0, vec![u8::from(self.m.rng.one_in(2))])),
                    Kind::Enum(e) => {
                        let max = e.values().map(|v| v.number()).max().unwrap_or(0);
                        let v = match self.m.rng.below(8) {
                            0 => u64::try_from(max).unwrap_or(0) + 1,
                            1 => u64::MAX,
                            _ => self.m.rng.below(usize::try_from(max).unwrap_or(0) + 1) as u64,
                        };
                        let mut b = Vec::new();
                        put_varint(&mut b, v);
                        Some((0, b))
                    }
                    Kind::Int32
                    | Kind::Int64
                    | Kind::Uint32
                    | Kind::Uint64
                    | Kind::Sint32
                    | Kind::Sint64 => {
                        let v = self.int_field(fd);
                        let mut b = Vec::new();
                        put_varint(&mut b, v);
                        Some((0, b))
                    }
                    Kind::Fixed64 | Kind::Sfixed64 | Kind::Double => {
                        Some((1, self.m.rng.next_u64().to_le_bytes().to_vec()))
                    }
                    Kind::Fixed32 | Kind::Sfixed32 | Kind::Float => {
                        Some((5, (self.m.rng.next_u64() as u32).to_le_bytes().to_vec()))
                    }
                    Kind::Message(md) => match md.full_name() {
                        "google.protobuf.Timestamp" => self.time_value(false).map(|v| (2, v)),
                        "google.protobuf.Duration" => self.time_value(true).map(|v| (2, v)),
                        "google.protobuf.Any" => self
                            .any_value(fd, who, depth)
                            .map(|a| (2, a.encode_to_vec())),
                        "cosmos.base.v1beta1.Coin" | "cosmos.base.v1beta1.DecCoin" => {
                            let mut c = vec![Field {
                                num: 1,
                                wt: 2,
                                val: denom_value(&mut self.m, &self.dict),
                            }];
                            let amount = if md.full_name().ends_with("DecCoin") {
                                dec_value(&mut self.m)
                            } else if self.m.rng.one_in(3) {
                                int_value(&mut self.m)
                            } else {
                                format!("{}", 1 + self.m.rng.below(10_000_000)).into_bytes()
                            };
                            if !self.m.rng.one_in(10) {
                                c.push(Field {
                                    num: 2,
                                    wt: 2,
                                    val: amount,
                                });
                            }
                            Some((2, encode(&c)))
                        }
                        full => {
                            let h = self.dict.msgs.get(full).cloned().unwrap_or_default();
                            if !h.is_empty() && self.m.rng.one_in(2) {
                                let b = self.pick(&h).unwrap_or_default();
                                Some((2, b))
                            } else if depth < 5
                                && (!o.nullable || repeated || !self.m.rng.one_in(5))
                            {
                                Some((2, self.synth(&md, who, depth + 1)))
                            } else {
                                None
                            }
                        }
                    },
                }
            };
            if let Some((wt, val)) = val {
                out.push(Field { num, wt, val });
            }
        }
        out
    }

    /// A message of type `md` built from scratch; its signer fields are
    /// `who`'s (validator form for validator address scalars).
    fn synth(&mut self, md: &MessageDescriptor, who: &Who, depth: usize) -> Vec<u8> {
        let signers = signer_fields(md);
        let mut out = Vec::new();
        for f in md.fields() {
            if signers.iter().any(|s| s == f.name()) && f.kind() == Kind::String {
                let v = if scalar_of(&f).as_deref() == Some("cosmos.ValidatorAddressString") {
                    who.val.clone()
                } else {
                    who.acc.clone()
                };
                if !self.m.rng.one_in(40) {
                    out.push(Field {
                        num: u64::from(f.number()),
                        wt: 2,
                        val: v.into_bytes(),
                    });
                }
                continue;
            }
            out.extend(self.gen_field(&f, who, depth));
        }
        encode(&out)
    }

    /// Field-level mutation of the message `b`: one to three fields
    /// regenerated, dropped, duplicated or added; recursion into nested
    /// messages and `Any`s.
    fn mutate_fields(
        &mut self,
        md: &MessageDescriptor,
        b: &[u8],
        who: &Who,
        depth: usize,
    ) -> Vec<u8> {
        let Some(mut fs) = parse_fields(b) else {
            return b.to_vec();
        };
        let fields: Vec<FieldDescriptor> = md.fields().collect();
        if fields.is_empty() || depth > 6 {
            return b.to_vec();
        }
        for _ in 0..=self.m.rng.below(3) {
            let fd = fields[self.m.rng.below(fields.len())].clone();
            let num = u64::from(fd.number());
            let at: Vec<usize> = (0..fs.len()).filter(|&i| fs[i].num == num).collect();
            match self.m.rng.below(6) {
                // Regenerate.
                0 | 1 => {
                    fs.retain(|f| f.num != num);
                    let pos = at.first().copied().unwrap_or(fs.len()).min(fs.len());
                    let new = self.gen_field(&fd, who, depth);
                    for (k, f) in new.into_iter().enumerate() {
                        fs.insert(pos + k, f);
                    }
                }
                // Drop.
                2 => fs.retain(|f| f.num != num),
                // Duplicate an occurrence.
                3 if !at.is_empty() => {
                    let f = fs[at[0]].clone();
                    fs.insert(at[0], f);
                }
                // Recurse.
                _ => {
                    if let (Some(&i), Kind::Message(sub)) = (at.first(), fd.kind()) {
                        if sub.full_name() == "google.protobuf.Any" {
                            if let Some(mut any) = parse_fields(&fs[i].val) {
                                let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                                if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                                    for a in &mut any {
                                        if a.num == 2 && a.wt == 2 {
                                            a.val =
                                                self.mutate_fields(&inner, &a.val, who, depth + 1);
                                        }
                                    }
                                }
                                fs[i].val = encode(&any);
                            }
                        } else if fs[i].wt == 2 {
                            let v = std::mem::take(&mut fs[i].val);
                            fs[i].val = self.mutate_fields(&sub, &v, who, depth + 1);
                        }
                    } else {
                        let new = self.gen_field(&fd, who, depth);
                        fs.retain(|f| f.num != num);
                        fs.extend(new);
                    }
                }
            }
        }
        encode(&fs)
    }

    /// Rewrites the signer fields of a scenario message to `who`.
    fn resign(md: &MessageDescriptor, b: &[u8], who: &Who) -> Vec<u8> {
        let signers = signer_fields(md);
        let Some(mut fs) = parse_fields(b) else {
            return b.to_vec();
        };
        for f in &mut fs {
            let Some(fd) = u32::try_from(f.num).ok().and_then(|n| md.get_field(n)) else {
                continue;
            };
            if f.wt == 2 && fd.kind() == Kind::String && signers.iter().any(|s| s == fd.name()) {
                let v = if scalar_of(&fd).as_deref() == Some("cosmos.ValidatorAddressString") {
                    &who.val
                } else {
                    &who.acc
                };
                f.val = v.clone().into_bytes();
            }
        }
        encode(&fs)
    }

    fn who(&self, i: usize) -> Who {
        Who {
            acc: self.accts[i].acc.clone(),
            val: self.accts[i].val.clone(),
        }
    }

    fn gov_who() -> Who {
        Who {
            acc: GOV.into(),
            val: GOV.into(),
        }
    }

    fn coin(&mut self, max: usize) -> Coin {
        Coin {
            denom: "utia".into(),
            amount: (1 + self.m.rng.below(max)).to_string(),
        }
    }

    /// A state-building message signed by `i`.
    #[allow(clippy::too_many_lines)]
    fn flow(&mut self, i: usize) -> Any {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        use celestia_proto::cosmos::bank::v1beta1 as bank;
        use celestia_proto::cosmos::distribution::v1beta1 as distr;
        use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::staking::v1beta1 as staking;
        let me = self.accts[i].acc.clone();
        // Grants and allowances mostly go to the next account, which
        // `tx` uses as the grantee of `MsgExec` and fee grants.
        let j = if self.m.rng.one_in(4) {
            self.acct_idx()
        } else {
            (i + 1) % self.accts.len()
        };
        let other = self.accts[j].acc.clone();
        let vals = self.validators.clone();
        let val = self.pick(&vals).unwrap_or_default();
        let val2 = self.pick(&vals).unwrap_or_default();
        match self.m.rng.below(30) {
            22..=26 => self.flow2(i),
            27..=29 => self.flow_hyperlane(i),
            0 => Any::pack(&bank::MsgSend {
                from_address: me,
                to_address: other,
                amount: vec![self.coin(1_000_000)],
            }),
            1 => Any::pack(&bank::MsgMultiSend {
                inputs: vec![bank::Input {
                    address: me,
                    coins: vec![Coin {
                        denom: "utia".into(),
                        amount: "2000".into(),
                    }],
                }],
                outputs: vec![
                    bank::Output {
                        address: other,
                        coins: vec![Coin {
                            denom: "utia".into(),
                            amount: "1000".into(),
                        }],
                    },
                    bank::Output {
                        address: module_address(MODULES[self.m.rng.below(MODULES.len())]),
                        coins: vec![Coin {
                            denom: "utia".into(),
                            amount: "1000".into(),
                        }],
                    },
                ],
            }),
            2 | 3 => Any::pack(&staking::MsgDelegate {
                delegator_address: me,
                validator_address: val,
                amount: Some(self.coin(10_000_000_000)),
            }),
            4 => Any::pack(&staking::MsgUndelegate {
                delegator_address: me,
                validator_address: val,
                amount: Some(self.coin(10_000_000)),
            }),
            5 => Any::pack(&staking::MsgBeginRedelegate {
                delegator_address: me,
                validator_src_address: val,
                validator_dst_address: val2,
                amount: Some(self.coin(10_000_000)),
            }),
            6 => {
                let ubs: Vec<_> = self
                    .unbondings
                    .iter()
                    .filter(|u| u.0 == i)
                    .cloned()
                    .collect();
                let (v, h) = self
                    .pick(&ubs)
                    .map_or((val, self.height - 1), |(_, v, h)| (v, h));
                Any::pack(&staking::MsgCancelUnbondingDelegation {
                    delegator_address: me,
                    validator_address: v,
                    amount: Some(self.coin(5_000_000)),
                    creation_height: h + i64::from(self.m.rng.one_in(8)),
                })
            }
            7 => {
                let mut key = [0u8; 32];
                for b in &mut key {
                    *b = self.m.rng.next_u64() as u8;
                }
                Any::pack(&staking::MsgCreateValidator {
                    description: Some(staking::Description {
                        moniker: format!("fuzz{i}"),
                        ..Default::default()
                    }),
                    commission: Some(staking::CommissionRates {
                        rate: "100000000000000000".into(),
                        max_rate: "200000000000000000".into(),
                        max_change_rate: "10000000000000000".into(),
                    }),
                    min_self_delegation: "1".into(),
                    #[allow(deprecated)]
                    delegator_address: String::new(),
                    // Sometimes register the operator with its uppercase Bech32
                    // form, which Go persists verbatim as the operator address.
                    validator_address: if self.m.rng.one_in(4) {
                        self.accts[i].val.to_uppercase()
                    } else {
                        self.accts[i].val.clone()
                    },
                    pubkey: Some(Any::pack(
                        &celestia_proto::cosmos::crypto::ed25519::PubKey { key: key.to_vec() },
                    )),
                    value: Some(Coin {
                        denom: "utia".into(),
                        amount: (1_000_000 + self.m.rng.below(1_000_000_000_000)).to_string(),
                    }),
                })
            }
            8 => Any::pack(&distr::MsgWithdrawDelegatorReward {
                delegator_address: me,
                validator_address: val,
            }),
            9 => Any::pack(&distr::MsgWithdrawValidatorCommission {
                validator_address: self.accts[i].val.clone(),
            }),
            10 => Any::pack(&distr::MsgSetWithdrawAddress {
                delegator_address: me,
                withdraw_address: if self.m.rng.one_in(3) {
                    module_address(MODULES[self.m.rng.below(MODULES.len())])
                } else {
                    other
                },
            }),
            11 => Any::pack(&distr::MsgFundCommunityPool {
                amount: vec![self.coin(100_000)],
                depositor: me,
            }),
            12 => {
                let t = self.types.clone();
                let common = [
                    "/cosmos.bank.v1beta1.MsgSend",
                    "/cosmos.staking.v1beta1.MsgDelegate",
                    "/cosmos.staking.v1beta1.MsgUndelegate",
                    "/cosmos.gov.v1.MsgVote",
                    "/cosmos.gov.v1.MsgSubmitProposal",
                    "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward",
                    "/cosmos.authz.v1beta1.MsgExec",
                    "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
                ];
                let msg = if self.m.rng.one_in(2) {
                    common[self.m.rng.below(common.len())].to_owned()
                } else {
                    t.get(self.m.rng.below(t.len().max(1)))
                        .map(|d| format!("/{}", d.full_name()))
                        .unwrap_or_default()
                };
                let auth = match self.m.rng.below(4) {
                    0 => Any::pack(&celestia_proto::cosmos::bank::v1beta1::SendAuthorization {
                        spend_limit: vec![self.coin(100_000_000)],
                        allow_list: Vec::new(),
                    }),
                    1 => Any::pack(&staking::StakeAuthorization {
                        max_tokens: Some(self.coin(100_000_000)),
                        authorization_type: 1 + self.m.rng.below(4) as i32,
                        policy: Some(staking::stake_authorization::Policy::AllowList(
                            staking::stake_authorization::Validators {
                                address: vals.clone(),
                            },
                        )),
                    }),
                    _ => Any::pack(&authz::GenericAuthorization { msg }),
                };
                Any::pack(&authz::MsgGrant {
                    granter: me,
                    grantee: other,
                    grant: Some(authz::Grant {
                        authorization: Some(auth),
                        expiration: if self.m.rng.one_in(3) {
                            Some(pbjson_types::Timestamp {
                                seconds: 1_700_000_000
                                    + self.height * 6
                                    + self.m.rng.below(100_000) as i64,
                                nanos: 0,
                            })
                        } else {
                            None
                        },
                    }),
                })
            }
            13 => Any::pack(&authz::MsgRevoke {
                granter: me,
                grantee: other,
                msg_type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
            }),
            14 => {
                let basic = feegrant::BasicAllowance {
                    spend_limit: if self.m.rng.one_in(2) {
                        vec![self.coin(10_000_000)]
                    } else {
                        Vec::new()
                    },
                    expiration: None,
                };
                let allowance = match self.m.rng.below(3) {
                    0 => Any::pack(&basic),
                    1 => Any::pack(&feegrant::PeriodicAllowance {
                        basic: Some(basic),
                        period: Some(pbjson_types::Duration {
                            seconds: 1 + self.m.rng.below(100) as i64,
                            nanos: 0,
                        }),
                        period_spend_limit: vec![self.coin(1_000_000)],
                        period_can_spend: Vec::new(),
                        period_reset: None,
                    }),
                    _ => Any::pack(&feegrant::AllowedMsgAllowance {
                        allowance: Some(Any::pack(&basic)),
                        allowed_messages: vec![
                            "/cosmos.bank.v1beta1.MsgSend".into(),
                            "/cosmos.staking.v1beta1.MsgDelegate".into(),
                        ],
                    }),
                };
                Any::pack(&feegrant::MsgGrantAllowance {
                    granter: me,
                    grantee: other,
                    allowance: Some(allowance),
                })
            }
            15 => Any::pack(&feegrant::MsgRevokeAllowance {
                granter: me,
                grantee: other,
            }),
            16 | 17 => {
                let p = self.proposals.clone();
                let id = self.pick(&p).unwrap_or(1);
                if self.m.rng.one_in(4) {
                    Any::pack(&gov::MsgVoteWeighted {
                        proposal_id: id,
                        voter: me,
                        options: vec![
                            gov::WeightedVoteOption {
                                option: 1,
                                weight: "0.5".into(),
                            },
                            gov::WeightedVoteOption {
                                option: 1 + self.m.rng.below(4) as i32,
                                weight: "0.5".into(),
                            },
                        ],
                        metadata: String::new(),
                    })
                } else {
                    Any::pack(&gov::MsgVote {
                        proposal_id: id,
                        voter: me,
                        option: if self.m.rng.one_in(5) {
                            1 + self.m.rng.below(4) as i32
                        } else {
                            1
                        },
                        metadata: String::new(),
                    })
                }
            }
            18 => {
                let p = self.proposals.clone();
                Any::pack(&gov::MsgDeposit {
                    proposal_id: self.pick(&p).unwrap_or(1),
                    depositor: me,
                    amount: vec![self.coin(10_000_000_000)],
                })
            }
            19 | 20 => {
                // A proposal of gov-signed messages of any type.
                let t = self.types.clone();
                let n = 1 + self.m.rng.below(2);
                let gw = Self::gov_who();
                let mut msgs = Vec::new();
                for _ in 0..n {
                    if self.m.rng.one_in(3) {
                        msgs.push(self.valid_gov_msg());
                        continue;
                    }
                    if t.is_empty() {
                        break;
                    }
                    let md = t[self.m.rng.below(t.len())].clone();
                    msgs.push(go_deterministic(Any {
                        type_url: format!("/{}", md.full_name()),
                        value: self.synth(&md, &gw, 1),
                    }));
                }
                Any::pack(&gov::MsgSubmitProposal {
                    messages: msgs,
                    initial_deposit: vec![Coin {
                        denom: "utia".into(),
                        amount: "100000000000".into(),
                    }],
                    proposer: me,
                    metadata: String::new(),
                    title: "fuzz".into(),
                    summary: "fuzz".into(),
                    expedited: self.m.rng.one_in(3),
                })
            }
            _ => Any::pack(&celestia_proto::cosmos::slashing::v1beta1::MsgUnjail {
                validator_addr: self.accts[i].val.clone(),
            }),
        }
    }

    /// A gov-authority message Go executes when the proposal passes:
    /// `MsgExecLegacyContent` of a text proposal, staking `MsgUpdateParams`
    /// within celestia's parameter filter (sometimes outside it).
    fn valid_gov_msg(&mut self) -> Any {
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::gov::v1beta1 as gov1b;
        use celestia_proto::cosmos::staking::v1beta1 as staking;
        let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
        if self.app_version >= 10 && !mailboxes.is_empty() && self.m.rng.one_in(3) {
            // Gov-only (`owner` is the authority): a native synthetic token
            // on a mailbox the fuzz accounts created, for a new denom (or
            // one with a supply).
            let origin_denom = match self.m.rng.below(4) {
                0 => "utia".into(),
                1 => String::new(),
                _ => format!("native{}", self.m.rng.below(1_000_000)),
            };
            return Any::pack(
                &celestia_proto::hyperlane::warp::v1::MsgCreateNativeSyntheticToken {
                    owner: GOV.into(),
                    origin_mailbox: self.pick(&mailboxes).unwrap_or_default(),
                    origin_denom,
                },
            );
        }
        if !self.gov_msgs.is_empty() && self.m.rng.one_in(2) {
            return self.gov_msgs[self.m.rng.below(self.gov_msgs.len())].clone();
        }
        if self.m.rng.one_in(2) {
            let content = Any::pack(&gov1b::TextProposal {
                title: "t".into(),
                description: "d".into(),
            });
            return Any::pack(&gov::MsgExecLegacyContent {
                content: Some(content),
                authority: if self.m.rng.one_in(8) {
                    self.accts[0].acc.clone()
                } else {
                    GOV.into()
                },
            });
        }
        let unbonding = if self.m.rng.one_in(6) {
            3600
        } else {
            337 * 3600
        };
        Any::pack(&staking::MsgUpdateParams {
            authority: GOV.into(),
            params: Some(staking::Params {
                unbonding_time: Some(pbjson_types::Duration {
                    seconds: unbonding,
                    nanos: 0,
                }),
                max_validators: [100, 100, 3, 0][self.m.rng.below(4)],
                max_entries: 1 + self.m.rng.below(10) as u32,
                historical_entries: self.m.rng.below(10_001) as u32,
                bond_denom: if self.m.rng.one_in(8) {
                    "uatom".into()
                } else {
                    "utia".into()
                },
                min_commission_rate: ["50000000000000000", "0", "1000000000000000000", ""]
                    [self.m.rng.below(4)]
                .into(),
            }),
        })
    }

    /// A hyperlane object of `kind` owned by account `i` (mostly), else any
    /// object of that kind, else a random id.
    fn hl_object(&mut self, kind: u8, i: usize) -> String {
        let anyone = self.m.rng.one_in(8);
        let own: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == kind && (o.2 == i || anyone))
            .map(|o| o.1.clone())
            .collect();
        if let Some(id) = self.pick(&own) {
            return id;
        }
        let any: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == kind)
            .map(|o| o.1.clone())
            .collect();
        self.pick(&any).unwrap_or_else(|| self.hex_id())
    }

    /// A create message for a hyperlane object of `kind` owned by account
    /// `i` (or for what it needs first: an ISM for a mailbox, a mailbox for
    /// a token).
    fn hl_create(&mut self, kind: u8, i: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = self.accts[i].acc.clone();
        let isms: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == HL_ISM)
            .map(|o| o.1.clone())
            .collect();
        let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
        match kind {
            HL_ROUTING_ISM => Any::pack(&ism::MsgCreateRoutingIsm {
                creator: me,
                routes: Vec::new(),
            }),
            HL_IGP => Any::pack(&pd::MsgCreateIgp {
                owner: me,
                denom: "utia".into(),
            }),
            HL_HOOK => Any::pack(&pd::MsgCreateNoopHook { owner: me }),
            HL_MAILBOX if !isms.is_empty() => Any::pack(&core::MsgCreateMailbox {
                owner: me,
                local_domain: 1 + self.m.rng.below(5) as u32,
                default_ism: self.pick(&isms).unwrap_or_default(),
                default_hook: String::new(),
                required_hook: String::new(),
            }),
            HL_TOKEN if !mailboxes.is_empty() => {
                let origin_mailbox = self.pick(&mailboxes).unwrap_or_default();
                if self.m.rng.one_in(2) {
                    Any::pack(&warp::MsgCreateCollateralToken {
                        owner: me,
                        origin_mailbox,
                        origin_denom: "utia".into(),
                    })
                } else {
                    Any::pack(&warp::MsgCreateSyntheticToken {
                        owner: me,
                        origin_mailbox,
                    })
                }
            }
            HL_TOKEN => self.hl_create(HL_MAILBOX, i),
            _ => Any::pack(&ism::MsgCreateNoopIsm { creator: me }),
        }
    }

    /// A random 32-byte hyperlane id (`0x` + 64 hex digits).
    fn hex_id(&mut self) -> String {
        let mut b = [0u8; 32];
        for x in &mut b {
            *x = self.m.rng.next_u64() as u8;
        }
        format!("0x{}", hex::encode(b))
    }

    /// Hyperlane owner operations on objects the fuzz accounts created (the
    /// owner signs, mostly): routing ISM domains and owner, IGP gas
    /// configs, owner and claims, mailbox settings, warp token settings and
    /// remote routers, multisig ISMs with sorted validator sets.
    fn flow_hyperlane(&mut self, i: usize) -> Any {
        let op = self.m.rng.below(19);
        self.flow_hyperlane_op(i, op)
    }

    /// An owner operation on a hyperlane object a fuzz account owns, signed
    /// by that account (sent unmutated: the success paths).
    fn hl_owner_msg(&mut self) -> Option<Any> {
        let cands: Vec<(u8, usize)> = self
            .hl_objects
            .iter()
            .filter(|o| {
                matches!(o.0, HL_ROUTING_ISM | HL_IGP | HL_MAILBOX | HL_TOKEN)
                    && self.reserved_account != Some(o.2)
            })
            .map(|o| (o.0, o.2))
            .collect();
        let (kind, i) = self.pick(&cands)?;
        let ops: &[usize] = match kind {
            HL_ROUTING_ISM => &[0, 1, 2],
            HL_IGP => &[3, 4, 5],
            HL_MAILBOX => &[6],
            _ => &[7, 8, 9, 10],
        };
        let op = ops[self.m.rng.below(ops.len())];
        Some(self.flow_hyperlane_op(i, op))
    }

    /// Hyperlane flow `op` (see [`Self::flow_hyperlane`]) signed by `i`.
    #[allow(clippy::too_many_lines)]
    fn flow_hyperlane_op(&mut self, i: usize, op: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = self.accts[i].acc.clone();
        let n = self.accts.len();
        let other = self.accts[(i + 1) % n].acc.clone();
        let domain = 1 + self.m.rng.below(5) as u32;
        let empty_or =
            |s: String, rng: &mut crate::mutate::Rng| if rng.one_in(4) { String::new() } else { s };
        // The object an owner operation targets: one the signer owns, else
        // (mostly) a create message for one first.
        let target = match op {
            0..=2 => Some(HL_ROUTING_ISM),
            3..=5 => Some(HL_IGP),
            6 => Some(HL_MAILBOX),
            7..=10 => Some(HL_TOKEN),
            _ => None,
        };
        if let Some(kind) = target
            && !self.hl_objects.iter().any(|o| o.0 == kind && o.2 == i)
            && !self.m.rng.one_in(8)
        {
            return self.hl_create(kind, i);
        }
        match op {
            13..=15 => {
                if let Some(m) = self.hl_process(i) {
                    return m;
                }
                Any::pack(&pd::MsgClaim {
                    sender: me,
                    igp_id: self.hl_object(HL_IGP, i),
                })
            }
            16 => {
                if let Some(m) = self.hl_pay_for_gas(i) {
                    return m;
                }
                Any::pack(&pd::MsgClaim {
                    sender: me,
                    igp_id: self.hl_object(HL_IGP, i),
                })
            }
            17 => self.hl_announce(i),
            18 => {
                // A mailbox whose default ISM verifies everything, and a
                // synthetic token on it: what `MsgProcessMessage` needs to
                // succeed (with a remote router enrolled).
                let noop: Vec<String> = self.hl.noop_isms.iter().cloned().collect();
                let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
                match self.m.rng.below(3) {
                    0 if !noop.is_empty() => Any::pack(&core::MsgCreateMailbox {
                        owner: me,
                        local_domain: 1 + self.m.rng.below(5) as u32,
                        default_ism: self.pick(&noop).unwrap_or_default(),
                        default_hook: String::new(),
                        required_hook: String::new(),
                    }),
                    1 if !mailboxes.is_empty() => Any::pack(&warp::MsgCreateSyntheticToken {
                        owner: me,
                        origin_mailbox: self.pick(&mailboxes).unwrap_or_default(),
                    }),
                    _ => Any::pack(&ism::MsgCreateNoopIsm { creator: me }),
                }
            }
            0 => {
                let ism_id = self.hl_object(HL_ROUTING_ISM, i);
                let target = self.hl_object(HL_ISM, i);
                Any::pack(&ism::MsgSetRoutingIsmDomain {
                    ism_id,
                    route: Some(ism::Route {
                        ism: target,
                        domain,
                    }),
                    owner: me,
                })
            }
            1 => Any::pack(&ism::MsgRemoveRoutingIsmDomain {
                ism_id: self.hl_object(HL_ROUTING_ISM, i),
                domain,
                owner: me,
            }),
            2 => Any::pack(&ism::MsgUpdateRoutingIsmOwner {
                ism_id: self.hl_object(HL_ROUTING_ISM, i),
                owner: me,
                new_owner: if self.m.rng.one_in(3) {
                    String::new()
                } else {
                    other
                },
                renounce_ownership: self.m.rng.one_in(6),
            }),
            3 => Any::pack(&pd::MsgSetDestinationGasConfig {
                owner: me,
                igp_id: self.hl_object(HL_IGP, i),
                destination_gas_config: Some(pd::DestinationGasConfig {
                    remote_domain: domain,
                    gas_oracle: Some(pd::GasOracle {
                        token_exchange_rate: (1 + self.m.rng.below(10_000_000_000)).to_string(),
                        gas_price: (1 + self.m.rng.below(1000)).to_string(),
                    }),
                    gas_overhead: (self.m.rng.below(300_000)).to_string(),
                }),
            }),
            4 => Any::pack(&pd::MsgSetIgpOwner {
                owner: me,
                igp_id: self.hl_object(HL_IGP, i),
                new_owner: if self.m.rng.one_in(3) {
                    String::new()
                } else {
                    other
                },
                renounce_ownership: self.m.rng.one_in(6),
            }),
            5 => Any::pack(&pd::MsgClaim {
                sender: me,
                igp_id: self.hl_object(HL_IGP, i),
            }),
            6 => {
                let mailbox_id = self.hl_object(HL_MAILBOX, i);
                let default_ism = self.hl_object(HL_ISM, i);
                let default_hook = self.hl_object(HL_HOOK, i);
                let kind = if self.m.rng.one_in(2) {
                    HL_IGP
                } else {
                    HL_HOOK
                };
                let required_hook = self.hl_object(kind, i);
                let rng = &mut self.m.rng;
                Any::pack(&core::MsgSetMailbox {
                    owner: me,
                    mailbox_id,
                    default_ism: empty_or(default_ism, rng),
                    default_hook: empty_or(default_hook, rng),
                    required_hook: empty_or(required_hook, rng),
                    new_owner: if rng.one_in(3) { other } else { String::new() },
                    renounce_ownership: rng.one_in(8),
                })
            }
            7 => {
                let token_id = self.hl_object(HL_TOKEN, i);
                let ism_id = self.hl_object(HL_ISM, i);
                let rng = &mut self.m.rng;
                Any::pack(&warp::MsgSetToken {
                    owner: me,
                    token_id,
                    new_owner: if rng.one_in(3) { other } else { String::new() },
                    ism_id: empty_or(ism_id, rng),
                    renounce_ownership: rng.one_in(8),
                })
            }
            8 | 9 => {
                let token_id = self.hl_object(HL_TOKEN, i);
                let receiver_contract = self.hex_id();
                Any::pack(&warp::MsgEnrollRemoteRouter {
                    owner: me,
                    token_id,
                    remote_router: Some(warp::RemoteRouter {
                        receiver_domain: domain,
                        receiver_contract,
                        gas: (self.m.rng.below(500_000)).to_string(),
                    }),
                })
            }
            10 => {
                // A router enrolled on a token the signer owns when there is
                // one (the success path), picked by `domain` so the
                // generator's draws stay as before.
                let token_id = self.hl_object(HL_TOKEN, i);
                let enrolled: Vec<(String, u32)> = self
                    .hl
                    .routers
                    .iter()
                    .filter(|r| {
                        self.hl_objects
                            .iter()
                            .any(|o| o.0 == HL_TOKEN && o.1 == r.0 && o.2 == i)
                    })
                    .map(|r| (r.0.clone(), r.1))
                    .collect();
                let (token_id, receiver_domain) = if enrolled.is_empty() || domain == 5 {
                    (token_id, domain)
                } else {
                    enrolled[domain as usize % enrolled.len()].clone()
                };
                Any::pack(&warp::MsgUnrollRemoteRouter {
                    owner: me,
                    token_id,
                    receiver_domain,
                })
            }
            _ => {
                let k = 1 + self.m.rng.below(5);
                let mut vals: Vec<String> = (0..k)
                    .map(|_| {
                        let mut b = [0u8; 20];
                        for x in &mut b {
                            *x = self.m.rng.next_u64() as u8;
                        }
                        format!("0x{}", hex::encode(b))
                    })
                    .collect();
                if !self.m.rng.one_in(5) {
                    vals.sort();
                }
                let threshold = self.m.rng.below(k + 2) as u32;
                if self.m.rng.one_in(2) {
                    Any::pack(&ism::MsgCreateMerkleRootMultisigIsm {
                        creator: me,
                        validators: vals,
                        threshold,
                    })
                } else {
                    Any::pack(&ism::MsgCreateMessageIdMultisigIsm {
                        creator: me,
                        validators: vals,
                        threshold,
                    })
                }
            }
        }
    }

    /// `MsgProcessMessage` of a warp transfer to a token the fuzz accounts
    /// created, from one of its enrolled routers (sometimes with a field
    /// off: version, destination, sender, body).
    fn hl_process(&mut self, i: usize) -> Option<Any> {
        use celestia_proto::hyperlane::core::v1 as core;
        let routers = self.hl.routers.clone();
        let (token, origin, sender) = self.pick(&routers)?;
        let mailbox = self.hl.token_mailbox.get(&token)?.clone();
        let domain = *self.hl.mailbox_domain.get(&mailbox)?;
        let hex32 = |s: &str| {
            let b = hex::decode(s.trim_start_matches("0x")).unwrap_or_default();
            let mut out = [0u8; 32];
            let n = b.len().min(32);
            out[32 - n..].copy_from_slice(&b[b.len() - n..]);
            out
        };
        let mut msg = Vec::new();
        let version = if self.m.rng.one_in(12) { 1 } else { 3 };
        msg.push(version);
        msg.extend_from_slice(&(self.m.rng.next_u64() as u32).to_be_bytes());
        msg.extend_from_slice(&origin.to_be_bytes());
        let mut snd = hex32(&sender);
        if self.m.rng.one_in(12) {
            snd[31] ^= 1;
        }
        msg.extend_from_slice(&snd);
        let dest = if self.m.rng.one_in(12) {
            domain.wrapping_add(1)
        } else {
            domain
        };
        msg.extend_from_slice(&dest.to_be_bytes());
        msg.extend_from_slice(&hex32(&token));
        // Warp body: recipient (32 bytes) and amount (uint256).
        let mut rcpt = [0u8; 32];
        let k = self.acct_idx();
        let addr = self.accts[k].addr.clone();
        if self.m.rng.one_in(6) {
            for b in &mut rcpt {
                *b = self.m.rng.next_u64() as u8;
            }
        } else {
            let n = addr.len().min(20);
            rcpt[12..12 + n].copy_from_slice(&addr[..n]);
        }
        msg.extend_from_slice(&rcpt);
        let mut amount = [0u8; 32];
        let a: u128 = match self.m.rng.below(6) {
            0 => 0,
            1 => u128::MAX,
            _ => 1 + self.m.rng.below(10_000_000) as u128,
        };
        amount[16..].copy_from_slice(&a.to_be_bytes());
        if self.m.rng.one_in(10) {
            amount[0] = 0xff;
        }
        msg.extend_from_slice(&amount);
        if self.m.rng.one_in(10) {
            let cut = 1 + self.m.rng.below(40);
            msg.truncate(msg.len() - cut);
        }
        let mailbox_id = if self.m.rng.one_in(12) {
            self.hl_object(HL_MAILBOX, i)
        } else {
            mailbox
        };
        Some(Any::pack(&core::MsgProcessMessage {
            mailbox_id,
            relayer: self.accts[i].acc.clone(),
            metadata: if self.m.rng.one_in(10) {
                "0xzz".into()
            } else {
                String::new()
            },
            message: format!("0x{}", hex::encode(msg)),
        }))
    }

    /// `MsgPayForGas` to an IGP with a gas config for the domain, in its
    /// denom (sometimes another one or zero).
    fn hl_pay_for_gas(&mut self, i: usize) -> Option<Any> {
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        let configs = self.hl.igp_domains.clone();
        let (igp, domain) = self.pick(&configs)?;
        let denom = self.hl.igp_denom.get(&igp).cloned().unwrap_or_default();
        let ids = self.hl.message_ids.clone();
        let message_id = self.pick(&ids).unwrap_or_else(|| self.hex_id());
        let destination_domain = if self.m.rng.one_in(8) {
            domain.wrapping_add(7)
        } else {
            domain
        };
        let denom = if self.m.rng.one_in(8) {
            "uatom".into()
        } else {
            denom
        };
        Some(Any::pack(&pd::MsgPayForGas {
            sender: self.accts[i].acc.clone(),
            igp_id: igp,
            message_id,
            destination_domain,
            gas_limit: (self.m.rng.below(2_000_000)).to_string(),
            amount: Some(Coin {
                denom,
                amount: (self.m.rng.below(100_000)).to_string(),
            }),
        }))
    }

    /// `MsgAnnounceValidator` signed with an Ethereum key from a small set
    /// for a mailbox the fuzz accounts created (sometimes a location
    /// announced before, a wrong validator or a corrupted signature).
    fn hl_announce(&mut self, i: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        let mailboxes: Vec<(String, u32)> = self
            .hl
            .mailbox_domain
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let (mailbox, domain) = match self.pick(&mailboxes) {
            Some(m) => m,
            None => (self.hex_id(), 1),
        };
        let mut secret = [0u8; 32];
        secret[31] = 1 + self.m.rng.below(4) as u8;
        let Ok(sk) = k256::ecdsa::SigningKey::from_slice(&secret) else {
            return Any::default();
        };
        let point = sk.verifying_key().to_sec1_point(false);
        let h = x_hyperlane::util::keccak256(&[&point.as_bytes()[1..]]);
        let mut validator = h[12..].to_vec();
        let location = format!("s3://bucket/{}", self.m.rng.below(4));
        let mbox = hex::decode(mailbox.trim_start_matches("0x")).unwrap_or_default();
        let digest = x_hyperlane::ism::announcement_digest(&location, domain, &mbox);
        let hash = x_hyperlane::util::eth_signing_hash(&digest);
        let (signed, rid) = sk.sign_prehash_recoverable(&hash);
        let mut sig = signed.to_bytes().to_vec();
        sig.push(27 + rid.to_byte());
        if self.m.rng.one_in(10) {
            sig[3] ^= 1;
        }
        if self.m.rng.one_in(10) {
            validator[0] ^= 1;
        }
        Any::pack(&ism::MsgAnnounceValidator {
            validator: format!("0x{}", hex::encode(validator)),
            storage_location: location,
            signature: format!("0x{}", hex::encode(sig)),
            mailbox_id: mailbox,
            creator: self.accts[i].acc.clone(),
        })
    }

    /// A random account address (no account yet).
    fn fresh_addr(&mut self) -> String {
        let mut a = [0u8; 20];
        for b in &mut a {
            *b = self.m.rng.next_u64() as u8;
        }
        encoding::bech32::encode("celestia", &a).unwrap_or_default()
    }

    /// More state-building flows: legacy (`v1beta1`) gov, proposal
    /// cancellation by its proposer, vesting accounts for fresh addresses,
    /// ICS-20 transfers.
    fn flow2(&mut self, i: usize) -> Any {
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::gov::v1beta1 as gov1b;
        use celestia_proto::cosmos::vesting::v1beta1 as vesting;
        let me = self.accts[i].acc.clone();
        let p = self.proposals.clone();
        let id = self.pick(&p).unwrap_or(1);
        // A rough block time for absolute times (vesting starts and ends).
        let now = 1_700_000_000 + self.height * 6;
        let to = self.fresh_addr();
        let utia = |n: u64| Coin {
            denom: "utia".into(),
            amount: n.to_string(),
        };
        match self.m.rng.below(9) {
            0 => Any::pack(&gov1b::MsgVote {
                proposal_id: id,
                voter: me,
                option: 1 + self.m.rng.below(4) as i32,
            }),
            1 => Any::pack(&gov1b::MsgVoteWeighted {
                proposal_id: id,
                voter: me,
                options: vec![
                    gov1b::WeightedVoteOption {
                        option: 1,
                        weight: "700000000000000000".into(),
                    },
                    gov1b::WeightedVoteOption {
                        option: 2 + self.m.rng.below(3) as i32,
                        weight: "300000000000000000".into(),
                    },
                ],
            }),
            2 => {
                // Mostly a small deposit on one of the newest proposals (in
                // its deposit or voting period: the success path). Derived
                // from `id`, so the generator's draws stay as before.
                let recent: Vec<u64> = p.iter().rev().take(3).copied().collect();
                let (proposal_id, amount) = if recent.is_empty() || id.is_multiple_of(4) {
                    (id, 1_000_000_000)
                } else {
                    (recent[id as usize % recent.len()], 1_000_000)
                };
                Any::pack(&gov1b::MsgDeposit {
                    proposal_id,
                    depositor: me,
                    amount: vec![utia(amount)],
                })
            }
            3 => Any::pack(&gov1b::MsgSubmitProposal {
                content: Some(Any::pack(&gov1b::TextProposal {
                    title: "fuzz".into(),
                    description: "fuzz text proposal".into(),
                })),
                initial_deposit: vec![utia(100_000_000_000)],
                proposer: me,
            }),
            4 => {
                let own: Vec<u64> = self
                    .proposers
                    .iter()
                    .filter(|(_, k)| *k == i)
                    .map(|(id, _)| *id)
                    .collect();
                Any::pack(&gov::MsgCancelProposal {
                    proposal_id: self.pick(&own).unwrap_or(id),
                    proposer: me,
                })
            }
            5 => {
                // Mostly 0 < start < end (start in the past or future).
                let start = if self.m.rng.one_in(10) {
                    0
                } else {
                    now + self.m.rng.below(2_000) as i64 - 1_500
                };
                let end = if self.m.rng.one_in(10) {
                    start
                } else {
                    start + 1 + self.m.rng.below(10_000) as i64
                };
                Any::pack(&vesting::MsgCreateVestingAccount {
                    from_address: me,
                    to_address: to,
                    amount: vec![utia(1 + self.m.rng.below(1_000_000) as u64)],
                    end_time: end,
                    delayed: self.m.rng.one_in(2),
                    start_time: start,
                })
            }
            6 => {
                let n = 1 + self.m.rng.below(4);
                let periods = (0..n)
                    .map(|_| vesting::Period {
                        length: 1 + self.m.rng.below(3_000) as i64,
                        amount: vec![utia(1 + self.m.rng.below(100_000) as u64)],
                    })
                    .collect();
                Any::pack(&vesting::MsgCreatePeriodicVestingAccount {
                    from_address: me,
                    to_address: to,
                    start_time: now + self.m.rng.below(1_000) as i64 - 500,
                    vesting_periods: periods,
                })
            }
            7 => Any::pack(&vesting::MsgCreatePermanentLockedAccount {
                from_address: me,
                to_address: to,
                amount: vec![utia(1 + self.m.rng.below(1_000_000) as u64)],
            }),
            _ => Any::pack(
                &celestia_proto::ibc::applications::transfer::v1::MsgTransfer {
                    source_port: "transfer".into(),
                    source_channel: "channel-0".into(),
                    token: Some(utia(1 + self.m.rng.below(1_000_000) as u64)),
                    sender: me,
                    receiver: self.fresh_addr(),
                    timeout_height: None,
                    timeout_timestamp: 4_000_000_000_000_000_000,
                    memo: String::new(),
                },
            ),
        }
    }

    /// One message signed by account `i`.
    fn msg(&mut self, i: usize) -> Any {
        let who = self.who(i);
        let t = self.types.clone();
        let base = match self.m.rng.below(10) {
            0..=3 => self.flow(i),
            4..=6 if !self.scenario_msgs.is_empty() => {
                let s = self.scenario_msgs.clone();
                let a = self.pick(&s).unwrap_or_default();
                match desc_for_type_url(a.type_url.as_bytes()) {
                    Some(md) => Any {
                        type_url: a.type_url.clone(),
                        value: Self::resign(&md, &a.value, &who),
                    },
                    None => a,
                }
            }
            _ if !t.is_empty() => {
                let md = t[self.m.rng.below(t.len())].clone();
                Any {
                    type_url: format!("/{}", md.full_name()),
                    value: self.synth(&md, &who, 0),
                }
            }
            _ => self.flow(i),
        };
        if self.wire.on && !self.m.rng.one_in(4) {
            return self.wire_mutate(base);
        }
        let Some(md) = desc_for_type_url(base.type_url.as_bytes()) else {
            return base;
        };
        match self.m.rng.below(10) {
            0..=3 => base,
            4..=8 => Any {
                type_url: base.type_url,
                value: self.mutate_fields(&md, &base.value, &who, 0),
            },
            _ => Any {
                type_url: base.type_url,
                value: self.m.mutate(&base.value),
            },
        }
    }

    /// `base` with one [`crate::wire`] mutation, of the class applied
    /// least so far among those `base` has.
    fn wire_mutate(&mut self, base: Any) -> Any {
        let name = base.type_url.rsplit('/').next().unwrap_or_default();
        let muts = crate::wire::enumerate(name, &base.value);
        let Some(min) = muts
            .iter()
            .map(|m| self.wire.counts.get(m.class).copied().unwrap_or(0))
            .min()
        else {
            return base;
        };
        let pick: Vec<&crate::wire::Mutation> = muts
            .iter()
            .filter(|m| self.wire.counts.get(m.class).copied().unwrap_or(0) == min)
            .collect();
        let classes: BTreeSet<&'static str> = pick.iter().map(|m| m.class).collect();
        let classes: Vec<&'static str> = classes.into_iter().collect();
        let class = classes[self.m.rng.below(classes.len())];
        let pick: Vec<&&crate::wire::Mutation> = pick.iter().filter(|m| m.class == class).collect();
        let mu = pick[self.m.rng.below(pick.len())];
        *self.wire.counts.entry(class).or_default() += 1;
        self.wire.pending.push(class);
        Any {
            type_url: base.type_url,
            value: mu.bytes.clone(),
        }
    }

    /// A generated tx (signed), or `None` when it could not be built.
    #[allow(clippy::too_many_lines)]
    /// A blob tx (`MsgPayForBlobs` with its blobs) signed by account `i`,
    /// with blob sizes at share boundaries and, sometimes, a mismatched
    /// commitment, size, namespace or share version.
    fn blob_tx(&mut self, i: usize) -> Option<Vec<u8>> {
        let a = &self.accts[i];
        let signer: [u8; 20] = a.addr.as_slice().try_into().unwrap_or([0u8; 20]);
        let acc = a.acc.clone();
        let n = 1 + self.m.rng.below(3);
        let mut blobs = Vec::new();
        let mut msg = celestia_proto::celestia::blob::v1::MsgPayForBlobs {
            signer: acc,
            ..Default::default()
        };
        for _ in 0..n {
            let size = [1usize, 478, 479, 482, 483, 512, 1000, 4000, 20_000, 120_000]
                [self.m.rng.below(10)];
            let data: Vec<u8> = (0..size).map(|_| self.m.rng.next_u64() as u8).collect();
            let id: Vec<u8> = (0..10).map(|_| self.m.rng.next_u64() as u8).collect();
            let ns = shares::Namespace::new_v0(&id).ok()?;
            let v1 = self.m.rng.one_in(3);
            let blob = if v1 {
                shares::Blob::new_v1(ns, data.clone(), signer).ok()?
            } else {
                shares::Blob::new_v0(ns, data.clone()).ok()?
            };
            let commitment =
                square::create_commitment(&blob, merkle::root_from_byte_slices, 64).ok()?;
            msg.namespaces.push(ns.as_bytes().to_vec());
            msg.blob_sizes.push(size as u32);
            msg.share_commitments.push(commitment.to_vec());
            msg.share_versions.push(u32::from(v1));
            blobs.push(sdk::tx::Blob {
                namespace_id: ns.id().to_vec(),
                namespace_version: 0,
                share_version: u32::from(v1),
                data,
                signer: if v1 { signer.to_vec() } else { Vec::new() },
            });
        }
        match self.m.rng.below(12) {
            0 => msg.share_commitments[0][0] ^= 1,
            1 => msg.blob_sizes[0] += 1,
            2 => msg.share_versions[0] = 1 - msg.share_versions[0].min(1),
            3 => {
                let b = blobs[0].clone();
                blobs.push(b);
            }
            4 => blobs[0].namespace_id[5] ^= 1,
            5 => msg.namespaces[0] = shares::namespace::TX_NAMESPACE.as_bytes().to_vec(),
            _ => {}
        }
        let total: u64 = msg.blob_sizes.iter().map(|s| u64::from(*s)).sum();
        let gas = 200_000 + total * 20 + if self.m.rng.one_in(8) { 0 } else { 100_000 };
        let gas = if self.m.rng.one_in(10) { gas / 4 } else { gas };
        let mut pfb = Any::pack(&msg);
        if self.wire.on && !self.m.rng.one_in(3) {
            pfb = self.wire_mutate(pfb);
        }
        let inner = sign_simple(self, i, vec![pfb], gas)?;
        Some(txdecode::marshal_blob_tx(&inner, &blobs))
    }

    fn tx(&mut self) -> Option<Built> {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        let i = self.acct_idx();
        if self.m.rng.one_in(10) {
            return self.blob_tx(i).map(|raw| Built { raw });
        }
        let n = match self.m.rng.below(10) {
            0..=6 => 1,
            7 | 8 => 2,
            _ => 3 + self.m.rng.below(3),
        };
        let mut msgs: Vec<Any> = (0..n)
            .map(|_| {
                // Mostly the same signer; sometimes another fuzz account
                // (a multi-signer tx).
                let s = if self.m.rng.one_in(8) {
                    self.acct_idx()
                } else {
                    i
                };
                self.msg(s)
            })
            .collect();
        if self.m.rng.one_in(10)
            && let Some(m) = self.hl_owner_msg()
        {
            msgs = vec![m];
        } else if self.m.rng.one_in(8) {
            // MsgExec by `i` of messages signed by a granter.
            let n = self.accts.len();
            let g = if self.m.rng.one_in(4) {
                self.acct_idx()
            } else {
                (i + n - 1) % n
            };
            let inner: Vec<Any> = (0..=self.m.rng.below(2)).map(|_| self.msg(g)).collect();
            msgs = vec![Any::pack(&authz::MsgExec {
                grantee: self.accts[i].acc.clone(),
                msgs: inner,
            })];
        }
        let mut body = TxBody {
            messages: msgs,
            memo: String::new(),
            timeout_height: 0,
            extension_options: Vec::new(),
            non_critical_extension_options: Vec::new(),
        };
        match self.m.rng.below(30) {
            0 => body.memo = "m".repeat(256),
            1 => body.memo = "m".repeat(257),
            2 => body.memo = gostr::from_go_bytes(b"memo \xff").into_owned(),
            3 => {
                body.timeout_height =
                    u64::try_from(self.height + 1 - self.m.rng.below(3) as i64).unwrap_or(0);
            }
            4 => body.extension_options.push(Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Vec::new(),
            }),
            5 => body.non_critical_extension_options.push(Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Vec::new(),
            }),
            _ => {}
        }
        // Gas and fee.
        let seen = body
            .messages
            .first()
            .and_then(|m| self.gas_seen.get(&m.type_url).copied());
        let gas: u64 = match self.m.rng.below(20) {
            0 => 0,
            1 => 1000,
            // Just below what a successful tx of this type used: out of gas
            // somewhere in the message handler.
            7..=9 if seen.is_some() => {
                let g = seen.unwrap_or_default();
                g.saturating_sub(self.m.rng.below((g / 3).max(1) as usize) as u64)
            }
            2 => 50_000 + self.m.rng.below(100_000) as u64,
            3 => 100_000 + self.m.rng.below(200_000) as u64,
            4 => 200_000 + self.m.rng.below(300_000) as u64,
            5 => 30_000_000,
            6 => u64::MAX,
            _ => 4_000_000,
        };
        let mut fee_amount: Vec<Coin> = match self.m.rng.below(20) {
            0 => Vec::new(),
            1 => vec![Coin {
                denom: "utia".into(),
                amount: "1".into(),
            }],
            2 => vec![Coin {
                denom: "uatom".into(),
                amount: (gas / 10).to_string(),
            }],
            3 => vec![
                Coin {
                    denom: "uatom".into(),
                    amount: "1".into(),
                },
                Coin {
                    denom: "utia".into(),
                    amount: (gas / 50).max(1).to_string(),
                },
            ],
            4 => vec![Coin {
                denom: "utia".into(),
                amount: (gas / 1_000_000).max(1).to_string(),
            }],
            _ => vec![Coin {
                denom: "utia".into(),
                amount: (u128::from(gas) / 50 + 1).to_string(),
            }],
        };
        // Fee validation precedes balance deduction, so unaffordable fees
        // still reach Go's Int64 telemetry conversion and priority overflow.
        if self.m.rng.one_in(100) {
            let boundaries = [
                "9223372036854775807",
                "9223372036854775808",
                "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            ];
            fee_amount = vec![Coin {
                denom: "utia".into(),
                amount: boundaries[self.m.rng.below(boundaries.len())].into(),
            }];
        }
        let mut granter = String::new();
        let mut payer = String::new();
        match self.m.rng.below(20) {
            0..=2 => {
                let n = self.accts.len();
                granter.clone_from(&self.accts[(i + n - 1) % n].acc);
            }
            3 => {
                let k = self.acct_idx();
                payer.clone_from(&self.accts[k].acc);
            }
            _ => {}
        }
        let amino = self.force_amino || self.m.rng.one_in(10);
        let unsigned_tx = |body: &TxBody, signer_infos: Vec<SignerInfo>, fee: Fee, sigs: usize| {
            #[allow(deprecated)]
            let auth = AuthInfo {
                signer_infos,
                fee: Some(fee),
                tip: None,
            };
            (body.encode_to_vec(), auth.encode_to_vec(), sigs)
        };
        // Signers in Go's order (message signers, then the fee payer).
        let probe = TxRaw {
            body_bytes: body.encode_to_vec(),
            auth_info_bytes: Vec::new(),
            signatures: Vec::new(),
        }
        .encode_to_vec();
        let mut signers: Vec<Vec<u8>> = txdecode::decode_for(self.app_version, &probe)
            .ok()
            .and_then(|t| x_ante::txview::signers(&t).ok())
            .unwrap_or_default();
        if signers.is_empty() {
            signers.push(self.accts[i].addr.clone());
        }
        if !payer.is_empty()
            && let Ok((_, p)) = encoding::bech32::decode(&payer)
            && !signers.contains(&p)
        {
            signers.push(p);
        }
        let idx: Vec<Option<usize>> = signers
            .iter()
            .map(|s| self.accts.iter().position(|a| a.addr == *s))
            .collect();
        let mut infos = Vec::new();
        let mut plans: Vec<Option<Plan>> = Vec::new();
        let mode = if amino {
            SignMode::LegacyAminoJson
        } else {
            SignMode::Direct
        };
        for (k, ix) in idx.iter().enumerate() {
            let (pk, seq) = match ix {
                Some(a) => {
                    let mut seq = self.accts[*a].seq;
                    if self.m.rng.one_in(40) {
                        seq = seq.wrapping_add(1);
                    }
                    (Some(self.accts[*a].pk.clone()), seq)
                }
                None => (Some(self.accts[k % self.accts.len()].pk.clone()), 0),
            };
            let (mi, plan) = match ix.and_then(|a| self.accts[a].ms.clone().map(|t| (a, t))) {
                Some((_, t)) => {
                    let (mi, p) = ms_plan(Some(&mut self.m.rng), &t, mode);
                    (mi, Some(p))
                }
                None => (
                    single_mode_info(mode),
                    ix.map(|a| Plan::Single {
                        acct: a,
                        mode,
                        corrupt: false,
                    }),
                ),
            };
            plans.push(plan);
            infos.push(SignerInfo {
                public_key: if self.m.rng.one_in(30) { None } else { pk },
                mode_info: Some(mi),
                sequence: seq,
            });
        }
        let fee = Fee {
            amount: fee_amount,
            gas_limit: gas,
            payer,
            granter,
        };
        let (body_bytes, auth_info_bytes, nsig) =
            unsigned_tx(&body, infos.clone(), fee, signers.len());
        let mut sigs = Vec::with_capacity(nsig);
        let decoded = if plans.iter().flatten().any(Plan::uses_amino) {
            let unsigned = TxRaw {
                body_bytes: body_bytes.clone(),
                auth_info_bytes: auth_info_bytes.clone(),
                signatures: vec![Vec::new(); nsig],
            }
            .encode_to_vec();
            txdecode::decode_for(self.app_version, &unsigned).ok()
        } else {
            None
        };
        for (k, ix) in idx.iter().enumerate() {
            let Some(a) = ix else {
                sigs.push(vec![1u8; 64]);
                continue;
            };
            let acct = &self.accts[*a];
            let account_number = if self.m.rng.one_in(60) {
                acct.number + 1
            } else {
                acct.number
            };
            let Some(plan) = &plans[k] else {
                sigs.push(vec![1u8; 64]);
                continue;
            };
            let amino_bytes = if plan.uses_amino() {
                decoded.as_ref().and_then(|tx| {
                    let sd = x_ante::signing::SignerData {
                        address: acct.acc.clone(),
                        chain_id: self.chain_id.clone(),
                        account_number,
                        sequence: infos[k].sequence,
                        pub_key: acct.pk.clone(),
                    };
                    x_ante::aminojson::sign_bytes(&sd, tx).ok()
                })
            } else {
                None
            };
            let direct = SignDoc {
                body_bytes: body_bytes.clone(),
                auth_info_bytes: auth_info_bytes.clone(),
                chain_id: self.chain_id.clone(),
                account_number,
            }
            .encode_to_vec();
            sigs.push(sign_plan(
                &self.accts,
                plan,
                &direct,
                amino_bytes.as_deref(),
            ));
        }
        if self.m.rng.one_in(80)
            && let Some(s) = sigs.first_mut()
            && s.len() > 5
        {
            s[5] ^= 1;
        }
        let raw = TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: sigs,
        }
        .encode_to_vec();
        // Message fields can draw a signer from the harvested dictionary, so
        // acct_idx alone cannot reserve an account. Finish generation first to
        // preserve the RNG stream, then drop any tx that actually needs it.
        if self.uses_reserved_signer(&idx) {
            return None;
        }
        Some(Built { raw })
    }
}

// ---------------------------------------------------------------------
// Comparison.

type CheckFields = (u32, String, i64, i64, i64, Vec<u8>, Vec<u8>, u64);

fn check_fields(r: &abci::ResponseCheckTx) -> CheckFields {
    (
        r.code,
        r.codespace.clone(),
        r.gas_wanted,
        r.gas_used,
        r.priority,
        r.data.clone(),
        r.address.clone(),
        r.sequence,
    )
}

fn event_text(e: &abci::Event) -> String {
    let attrs: Vec<String> = e
        .attributes
        .iter()
        .map(|a| {
            format!(
                "{}=\"{}\"{}",
                gostr::to_go_bytes(&a.key).escape_ascii(),
                gostr::to_go_bytes(&a.value).escape_ascii(),
                if a.index { "" } else { " (noindex)" }
            )
        })
        .collect();
    format!("{}{{{}}}", e.r#type, attrs.join(", "))
}

fn events_diff(what: &str, o: &[abci::Event], g: &[abci::Event]) -> Option<String> {
    if o == g {
        return None;
    }
    let i = (0..o.len().max(g.len()))
        .find(|&i| o.get(i) != g.get(i))
        .unwrap_or_default();
    Some(format!(
        "{what} event {i} (go {} / rust {}):\n    go:   {}\n    rust: {}",
        g.len(),
        o.len(),
        g.get(i).map_or_else(|| "-".into(), event_text),
        o.get(i).map_or_else(|| "-".into(), event_text)
    ))
}

/// Differences between Go's and Rust's `FinalizeBlock` responses:
/// (consensus, log-only) texts.
fn finalize_diff(
    go: &abci::ResponseFinalizeBlock,
    ours: &abci::ResponseFinalizeBlock,
) -> (Vec<String>, Vec<String>) {
    let mut cons = Vec::new();
    let mut logs = Vec::new();
    if go.tx_results.len() != ours.tx_results.len() {
        cons.push(format!(
            "tx result count go={} rust={}",
            go.tx_results.len(),
            ours.tx_results.len()
        ));
    }
    for (i, (g, o)) in go.tx_results.iter().zip(&ours.tx_results).enumerate() {
        let det = |t: &abci::ExecTxResult| {
            (
                t.code,
                t.codespace.clone(),
                t.data.clone(),
                t.gas_wanted,
                t.gas_used,
            )
        };
        if det(g) != det(o) {
            cons.push(format!(
                "tx {i}: go=(code {} {} gas {}/{} data {}) log={:?}\n      rust=(code {} {} gas {}/{} data {}) log={:?}",
                g.code,
                g.codespace,
                g.gas_used,
                g.gas_wanted,
                hex::encode(&g.data),
                gostr::to_go_bytes(&g.log).escape_ascii().to_string(),
                o.code,
                o.codespace,
                o.gas_used,
                o.gas_wanted,
                hex::encode(&o.data),
                gostr::to_go_bytes(&o.log).escape_ascii().to_string(),
            ));
            continue;
        }
        if g.log != o.log {
            logs.push(format!(
                "tx {i} log:\n    go:   {}\n    rust: {}",
                gostr::to_go_bytes(&g.log).escape_ascii(),
                gostr::to_go_bytes(&o.log).escape_ascii()
            ));
        } else if g.info != o.info {
            logs.push(format!("tx {i} info: go={:?} rust={:?}", g.info, o.info));
        } else if g.signers != o.signers {
            logs.push(format!(
                "tx {i} signers: go={:?} rust={:?}",
                g.signers, o.signers
            ));
        } else if let Some(d) = events_diff(&format!("tx {i}"), &o.events, &g.events) {
            logs.push(d);
        }
    }
    if go.validator_updates != ours.validator_updates {
        cons.push(format!(
            "validator updates go={:?} rust={:?}",
            go.validator_updates, ours.validator_updates
        ));
    }
    if go.consensus_param_updates != ours.consensus_param_updates {
        cons.push(format!(
            "consensus param updates go={:?} rust={:?}",
            go.consensus_param_updates, ours.consensus_param_updates
        ));
    }
    if go.app_hash != ours.app_hash {
        cons.push(format!(
            "app hash go={} rust={}",
            hex::encode(&go.app_hash),
            hex::encode(&ours.app_hash)
        ));
    }
    if let Some(d) = events_diff("block", &ours.events, &go.events) {
        logs.push(d);
    }
    (cons, logs)
}

/// The type URLs of the messages of `tx` (nested ones of `MsgExec` /
/// `MsgSubmitProposal` prefixed by `>`).
fn type_urls(app_version: u64, tx: &[u8]) -> Vec<String> {
    let Ok(t) = txdecode::decode_for(app_version, tx) else {
        return vec!["<undecodable>".into()];
    };
    message_type_urls(&t.body.messages, true)
}

fn executed_type_urls(app_version: u64, tx: &[u8]) -> Vec<String> {
    txdecode::decode_for(app_version, tx)
        .map(|t| message_type_urls(&t.body.messages, false))
        .unwrap_or_default()
}

/// Go's runMsgs adds this prefix only when a message-service handler returns
/// an error. Do not infer a handler from the transaction's first message or
/// search user-controlled inner error text for another index.
fn failing_handler_type(app_version: u64, tx: &[u8], log: &str) -> Option<String> {
    let rest = log.strip_prefix("failed to execute message; message index: ")?;
    let (index, _) = rest.split_once(": ")?;
    if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let index = index.parse::<usize>().ok()?;
    let tx = txdecode::decode_for(app_version, tx).ok()?;
    tx.body
        .messages
        .get(index)
        .map(|message| message.type_url.clone())
}

fn observe_handler_error(
    stats: &mut Stats,
    app_version: u64,
    tx: &[u8],
    result: &abci::ExecTxResult,
) {
    if result.code == 0 {
        return;
    }
    let code = format!("{}/{}", result.codespace, result.code);
    if let Some(kind) = failing_handler_type(app_version, tx, &result.log) {
        *stats
            .handler_errors
            .entry(format!("{kind} {code}"))
            .or_default() += 1;
    } else {
        *stats.unattributed_errors.entry(code).or_default() += 1;
    }
}

fn message_type_urls(messages: &[Any], proposal_contents: bool) -> Vec<String> {
    let mut out = Vec::new();
    for a in messages {
        out.push(a.type_url.clone());
        let inner: Vec<Any> = match a.type_url.as_str() {
            "/cosmos.authz.v1beta1.MsgExec" => {
                celestia_proto::cosmos::authz::v1beta1::MsgExec::decode(a.value.as_slice())
                    .map(|m| m.msgs)
                    .unwrap_or_default()
            }
            "/cosmos.gov.v1.MsgSubmitProposal" if proposal_contents => {
                celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice())
                    .map(|m| m.messages)
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        out.extend(inner.into_iter().map(|x| format!(">{}", x.type_url)));
    }
    out
}

fn go_value(r: Response) -> anyhow::Result<response::Value> {
    r.value.ok_or_else(|| anyhow::anyhow!("empty Go response"))
}

/// Re-drives `scenario` (patched genesis) into the Go app at `addr` and
/// the Rust app, then finalizes `blocks` generated blocks of up to
/// `per_block` txs on both.
#[allow(clippy::too_many_lines)]
pub fn run(
    scenario: &str,
    addr: &str,
    blocks: usize,
    per_block: usize,
    seed: u64,
) -> anyhow::Result<Stats> {
    let evidence_requirement = EvidenceRequirement::from_env()?;
    let verifier = hyperlane::Verifier::from_env()?;
    let hl_on = std::env::var("DELIVER_HL").is_ok();
    let hl_reject = std::env::var("DELIVER_HL_REJECT").is_ok();
    let hl_recovery_id_required = std::env::var_os("DELIVER_HL_RECOVERY_ID_REQUIRE").is_some();
    let hl_registry_required = std::env::var_os("DELIVER_HL_REGISTRY_REQUIRE").is_some();
    anyhow::ensure!(
        hl_on || verifier.name() == "noop",
        "DELIVER_HL_ISM requires DELIVER_HL"
    );
    anyhow::ensure!(
        !hl_reject || (hl_on && verifier.name() != "noop"),
        "DELIVER_HL_REJECT requires DELIVER_HL and a signed DELIVER_HL_ISM"
    );
    anyhow::ensure!(
        !hl_reject || std::env::var("DELIVER_UPGRADE").is_err(),
        "DELIVER_HL_REJECT cannot be combined with DELIVER_UPGRADE: the controlled phase would skip early upgrade fixtures"
    );
    anyhow::ensure!(
        !hl_recovery_id_required || (hl_on && !hl_reject && per_block == 0),
        "DELIVER_HL_RECOVERY_ID_REQUIRE requires DELIVER_HL, DELIVER_HL_REJECT unset and per_block=0"
    );
    anyhow::ensure!(
        !hl_recovery_id_required || std::env::var("DELIVER_UPGRADE").is_err(),
        "DELIVER_HL_RECOVERY_ID_REQUIRE cannot be combined with DELIVER_UPGRADE"
    );
    anyhow::ensure!(
        !hl_registry_required || (hl_on && verifier.name() == "merkle-root"),
        "DELIVER_HL_REGISTRY_REQUIRE requires DELIVER_HL and DELIVER_HL_ISM=merkle-root"
    );
    let govpass_env = std::env::var("DELIVER_GOVPASS").is_ok();
    let upgrade_plan_required = std::env::var_os("DELIVER_UPGRADE_PLAN_REQUIRE").is_some();
    let upgrade_halt_required = std::env::var_os("DELIVER_UPGRADE_HALT_REQUIRE").is_some();
    let upgrade_skip_required = std::env::var_os("DELIVER_UPGRADE_SKIP_REQUIRE").is_some();
    let upgrade_restart_required = std::env::var_os("DELIVER_UPGRADE_RESTART_REQUIRE").is_some();
    let gov_params_rejection_required =
        std::env::var_os("DELIVER_GOV_PARAMS_REJECTION_REQUIRE").is_some();
    let gov_handler_rejection_required =
        std::env::var_os("DELIVER_GOV_HANDLER_REJECTION_REQUIRE").is_some();
    let governed_upgrade_required = upgrade_plan_required
        || upgrade_halt_required
        || upgrade_skip_required
        || upgrade_restart_required;
    let governed_fixture_required = governed_upgrade_required
        || gov_params_rejection_required
        || gov_handler_rejection_required;
    let govpass = govpass_env || governed_fixture_required;
    let ibc_on = std::env::var("DELIVER_IBC").is_ok();
    let transfer_authz_required = std::env::var_os("DELIVER_TRANSFER_AUTHZ_REQUIRE").is_some();
    let direct_aux_required = std::env::var_os("DELIVER_DIRECT_AUX_REQUIRE").is_some();
    let malformed_multisig_required =
        std::env::var_os("DELIVER_MALFORMED_MULTISIG_REQUIRE").is_some();
    let zero_threshold_multisig_required =
        std::env::var_os("DELIVER_ZERO_THRESHOLD_MULTISIG_REQUIRE").is_some();
    let p256_required = std::env::var_os("DELIVER_P256_REQUIRE").is_some();
    let pubkey_rejection_required = std::env::var_os("DELIVER_PUBKEY_REJECTION_REQUIRE").is_some();
    let never_bonded_unjail_required =
        std::env::var_os("DELIVER_NEVER_BONDED_UNJAIL_REQUIRE").is_some();
    let unbonded_redelegation_rollback_required =
        std::env::var_os("DELIVER_UNBONDED_REDELEGATION_ROLLBACK_REQUIRE").is_some();
    let validator_removal_rewards_required =
        std::env::var_os("DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE").is_some();
    anyhow::ensure!(
        !gov_params_rejection_required
            || (!govpass_env
                && !governed_upgrade_required
                && !gov_handler_rejection_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !never_bonded_unjail_required
                && !unbonded_redelegation_rollback_required
                && !validator_removal_rewards_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_GOV_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_ABSTAIN_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_FAILURE_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_FAILURE_TYPE_REQUIRE").is_none()
                && std::env::var_os("DELIVER_SIM").is_none()
                && std::env::var_os("DELIVER_RECHECK").is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && std::env::var_os("DELIVER_TIME_JUMP_AT").as_deref()
                    == Some(std::ffi::OsStr::new("4"))
                && std::env::var_os("DELIVER_TIME_JUMP_SECONDS").as_deref()
                    == Some(std::ffi::OsStr::new("3600"))
                && blocks == 6
                && per_block == 0),
        "DELIVER_GOV_PARAMS_REJECTION_REQUIRE needs exactly six blocks, zero random transactions and the controlled block-4 3600-second jump, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !gov_handler_rejection_required
            || (!govpass_env
                && !governed_upgrade_required
                && !gov_params_rejection_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !never_bonded_unjail_required
                && !unbonded_redelegation_rollback_required
                && !validator_removal_rewards_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_GOV_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_ABSTAIN_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_FAILURE_REQUIRE").is_none()
                && std::env::var_os("DELIVER_GOV_FAILURE_TYPE_REQUIRE").is_none()
                && std::env::var_os("DELIVER_SIM").is_none()
                && std::env::var_os("DELIVER_RECHECK").is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && std::env::var_os("DELIVER_TIME_JUMP_AT").as_deref()
                    == Some(std::ffi::OsStr::new("4"))
                && std::env::var_os("DELIVER_TIME_JUMP_SECONDS").as_deref()
                    == Some(std::ffi::OsStr::new("3600"))
                && blocks == 6
                && per_block == 0),
        "DELIVER_GOV_HANDLER_REJECTION_REQUIRE needs exactly six blocks, zero random transactions and the controlled block-4 3600-second jump, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !never_bonded_unjail_required
            || (!unbonded_redelegation_rollback_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !governed_upgrade_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 5
                && per_block == 0),
        "DELIVER_NEVER_BONDED_UNJAIL_REQUIRE needs exactly five blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !unbonded_redelegation_rollback_required
            || (!never_bonded_unjail_required
                && !validator_removal_rewards_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !governed_upgrade_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 4
                && per_block == 0),
        "DELIVER_UNBONDED_REDELEGATION_ROLLBACK_REQUIRE needs exactly four blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !validator_removal_rewards_required
            || (!never_bonded_unjail_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !governed_upgrade_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 5
                && per_block == 0),
        "DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE needs exactly five blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !upgrade_plan_required
            || (!govpass_env
                && !gov_params_rejection_required
                && !gov_handler_rejection_required
                && !upgrade_halt_required
                && !upgrade_skip_required
                && !upgrade_restart_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 240
                && per_block == 0),
        "DELIVER_UPGRADE_PLAN_REQUIRE needs exactly 240 blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !upgrade_halt_required
            || (!govpass_env
                && !gov_params_rejection_required
                && !gov_handler_rejection_required
                && !upgrade_plan_required
                && !upgrade_skip_required
                && !upgrade_restart_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 80
                && per_block == 0),
        "DELIVER_UPGRADE_HALT_REQUIRE needs exactly 80 blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !upgrade_skip_required
            || (!govpass_env
                && !gov_params_rejection_required
                && !gov_handler_rejection_required
                && !upgrade_plan_required
                && !upgrade_halt_required
                && !upgrade_restart_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 80
                && per_block == 0),
        "DELIVER_UPGRADE_SKIP_REQUIRE needs exactly 80 blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !upgrade_restart_required
            || (!govpass_env
                && !gov_params_rejection_required
                && !gov_handler_rejection_required
                && !upgrade_plan_required
                && !upgrade_halt_required
                && !upgrade_skip_required
                && !direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 82
                && per_block == 0),
        "DELIVER_UPGRADE_RESTART_REQUIRE needs exactly 82 blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !direct_aux_required
            || (!hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 2
                && per_block == 0),
        "DELIVER_DIRECT_AUX_REQUIRE needs exactly two blocks and zero random transactions, and cannot be combined with Hyperlane, governance, IBC, evidence, scripts, or upgrades"
    );
    anyhow::ensure!(
        !malformed_multisig_required
            || (!direct_aux_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 4
                && per_block == 0),
        "DELIVER_MALFORMED_MULTISIG_REQUIRE needs exactly four blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !zero_threshold_multisig_required
            || (!direct_aux_required
                && !malformed_multisig_required
                && !p256_required
                && !pubkey_rejection_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 3
                && per_block == 0),
        "DELIVER_ZERO_THRESHOLD_MULTISIG_REQUIRE needs exactly three blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !p256_required
            || (!direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !pubkey_rejection_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 7
                && per_block == 0),
        "DELIVER_P256_REQUIRE needs exactly seven blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !pubkey_rejection_required
            || (!direct_aux_required
                && !malformed_multisig_required
                && !zero_threshold_multisig_required
                && !p256_required
                && !hl_on
                && !govpass
                && !ibc_on
                && evidence_requirement.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && std::env::var_os("DELIVER_SCRIPT").is_none()
                && blocks == 7
                && per_block == 0),
        "DELIVER_PUBKEY_REJECTION_REQUIRE needs exactly seven blocks and zero random transactions, and cannot be combined with other controlled modes"
    );
    anyhow::ensure!(
        !transfer_authz_required
            || (ibc_on
                && std::env::var_os("DELIVER_SCRIPT").is_some()
                && std::env::var_os("DELIVER_SCRIPT_REQUIRE").is_some()
                && std::env::var_os("DELIVER_UPGRADE").is_none()),
        "DELIVER_TRANSFER_AUTHZ_REQUIRE needs DELIVER_IBC, DELIVER_SCRIPT, and DELIVER_SCRIPT_REQUIRE without an upgrade"
    );
    anyhow::ensure!(
        evidence_requirement.is_none()
            || (!hl_on
                && !govpass
                && !ibc_on
                && std::env::var_os("DELIVER_UPGRADE").is_none()
                && (std::env::var_os("DELIVER_SCRIPT").is_none()
                    || evidence_requirement == Some(EvidenceRequirement::StakeLifecycle))),
        "DELIVER_EVIDENCE_REQUIRE cannot be combined with Hyperlane, governance, IBC, or upgrade; scripted traffic is reserved for stake-lifecycle"
    );
    anyhow::ensure!(
        evidence_requirement != Some(EvidenceRequirement::StakeLifecycle)
            || (std::env::var_os("DELIVER_SCRIPT").is_some()
                && std::env::var_os("DELIVER_SCRIPT_REQUIRE").is_some()),
        "DELIVER_EVIDENCE_REQUIRE=stake-lifecycle requires DELIVER_SCRIPT and DELIVER_SCRIPT_REQUIRE"
    );
    anyhow::ensure!(
        evidence_requirement != Some(EvidenceRequirement::ZeroDoubleSignSlash)
            || (blocks == 3 && per_block == 0 && std::env::var_os("DELIVER_SCRIPT").is_none()),
        "DELIVER_EVIDENCE_REQUIRE=zero-double-sign-slash needs exactly three blocks and zero random transactions"
    );
    let ibc_connection_confirm = std::env::var_os("DELIVER_IBC_CONNECTION_CONFIRM").is_some();
    let ibc_close_init_failure_required =
        std::env::var_os("DELIVER_IBC_CLOSE_INIT_FAILURE_REQUIRE").is_some();
    let ibc_legacy_upgrade_fixture =
        std::env::var_os("DELIVER_IBC_LEGACY_UPGRADE_FIXTURE").is_some();
    let ibc_ack_failure_required = std::env::var_os("DELIVER_IBC_ACK_FAILURE_REQUIRE").is_some();
    let ibc_pfm_required = std::env::var_os("DELIVER_IBC_PFM_REQUIRE").is_some();
    let ibc_pfm_error_required = std::env::var_os("DELIVER_IBC_PFM_ERROR_REQUIRE").is_some();
    let ibc_pfm_timeout_required = std::env::var_os("DELIVER_IBC_PFM_TIMEOUT_REQUIRE").is_some();
    let ibc_pfm_exhaust_required = std::env::var_os("DELIVER_IBC_PFM_EXHAUST_REQUIRE").is_some();
    let ibc_pfm_metadata_required = std::env::var_os("DELIVER_IBC_PFM_METADATA_REQUIRE").is_some();
    let ibc_ica_encodings_required =
        std::env::var_os("DELIVER_IBC_ICA_ENCODINGS_REQUIRE").is_some();
    let ibc_ica_order_required = std::env::var_os("DELIVER_IBC_ICA_ORDER_REQUIRE").is_some();
    let ibc_upgrade_timeout_required =
        std::env::var_os("DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE").is_some();
    let ibc_upgrade_timeout_boundary_required =
        std::env::var_os("DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE").is_some();
    let ibc_upgrade_confirm_timeout_required =
        std::env::var_os("DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE").is_some();
    let ibc_upgrade_try_fast_forward_required =
        std::env::var_os("DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE").is_some();
    let ibc_upgrade_open_flushcomplete_required =
        std::env::var_os("DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE").is_some();
    let ibc_ica_error_required = std::env::var_os("DELIVER_IBC_ICA_ERROR_REQUIRE").is_some();
    let ibc_expired_client_required =
        std::env::var_os("DELIVER_IBC_EXPIRED_CLIENT_REQUIRE").is_some();
    let ibc_frozen_client_required =
        std::env::var_os("DELIVER_IBC_FROZEN_CLIENT_REQUIRE").is_some();
    anyhow::ensure!(
        !ibc_connection_confirm || ibc_on,
        "DELIVER_IBC_CONNECTION_CONFIRM requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_close_init_failure_required || ibc_on,
        "DELIVER_IBC_CLOSE_INIT_FAILURE_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_legacy_upgrade_fixture || ibc_on,
        "DELIVER_IBC_LEGACY_UPGRADE_FIXTURE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_ack_failure_required || ibc_on,
        "DELIVER_IBC_ACK_FAILURE_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_pfm_required || ibc_on,
        "DELIVER_IBC_PFM_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_pfm_error_required || ibc_on,
        "DELIVER_IBC_PFM_ERROR_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_pfm_timeout_required || ibc_on,
        "DELIVER_IBC_PFM_TIMEOUT_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_pfm_exhaust_required || ibc_on,
        "DELIVER_IBC_PFM_EXHAUST_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_pfm_metadata_required || ibc_on,
        "DELIVER_IBC_PFM_METADATA_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_ica_encodings_required || ibc_on,
        "DELIVER_IBC_ICA_ENCODINGS_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_ica_order_required || ibc_on,
        "DELIVER_IBC_ICA_ORDER_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_upgrade_timeout_required || (ibc_on && per_block == 0),
        "DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE requires DELIVER_IBC and per_block=0"
    );
    anyhow::ensure!(
        !ibc_upgrade_timeout_boundary_required || (ibc_on && per_block == 0),
        "DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE requires DELIVER_IBC and per_block=0"
    );
    anyhow::ensure!(
        !ibc_upgrade_confirm_timeout_required || (ibc_on && per_block == 0),
        "DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE requires DELIVER_IBC and per_block=0"
    );
    anyhow::ensure!(
        !ibc_upgrade_try_fast_forward_required || (ibc_on && per_block == 0),
        "DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE requires DELIVER_IBC and per_block=0"
    );
    anyhow::ensure!(
        !ibc_upgrade_open_flushcomplete_required || (ibc_on && per_block == 0),
        "DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE requires DELIVER_IBC and per_block=0"
    );
    anyhow::ensure!(
        !ibc_ica_error_required || ibc_on,
        "DELIVER_IBC_ICA_ERROR_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        usize::from(ibc_ica_encodings_required)
            + usize::from(ibc_ica_order_required)
            + usize::from(ibc_ica_error_required)
            + usize::from(ibc_upgrade_timeout_required)
            + usize::from(ibc_upgrade_timeout_boundary_required)
            + usize::from(ibc_upgrade_confirm_timeout_required)
            + usize::from(ibc_upgrade_try_fast_forward_required)
            + usize::from(ibc_upgrade_open_flushcomplete_required)
            <= 1,
        "DELIVER_IBC ICA encoding-success, order-transition, upgrade-timeout, and application-error requirements are mutually exclusive"
    );
    anyhow::ensure!(
        !ibc_expired_client_required || ibc_on,
        "DELIVER_IBC_EXPIRED_CLIENT_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_frozen_client_required || ibc_on,
        "DELIVER_IBC_FROZEN_CLIENT_REQUIRE requires DELIVER_IBC"
    );
    anyhow::ensure!(
        usize::from(ibc_pfm_required)
            + usize::from(ibc_pfm_error_required)
            + usize::from(ibc_pfm_timeout_required)
            + usize::from(ibc_pfm_exhaust_required)
            + usize::from(ibc_pfm_metadata_required)
            <= 1,
        "DELIVER_IBC PFM success, error, timeout, exhaustion, and metadata requirements are mutually exclusive"
    );
    anyhow::ensure!(
        !(ibc_ica_encodings_required
            || ibc_ica_order_required
            || ibc_upgrade_timeout_required
            || ibc_upgrade_timeout_boundary_required
            || ibc_upgrade_confirm_timeout_required
            || ibc_upgrade_try_fast_forward_required
            || ibc_upgrade_open_flushcomplete_required)
            || !(ibc_pfm_required
                || ibc_pfm_error_required
                || ibc_pfm_timeout_required
                || ibc_pfm_exhaust_required
                || ibc_pfm_metadata_required),
        "DELIVER_IBC ICA success requirements cannot be combined with a PFM lifecycle requirement"
    );
    let required = std::env::var_os("DELIVER_GOV_REQUIRE")
        .map(|v| {
            v.into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_GOV_REQUIRE must be UTF-8"))
        })
        .transpose()?;
    let gov_required = gov_requirements(required.as_deref(), govpass)?;
    let gov_abstain_required = std::env::var_os("DELIVER_GOV_ABSTAIN_REQUIRE").is_some();
    anyhow::ensure!(
        !gov_abstain_required || govpass,
        "DELIVER_GOV_ABSTAIN_REQUIRE requires DELIVER_GOVPASS"
    );
    anyhow::ensure!(
        !gov_abstain_required || gov_required.is_empty(),
        "DELIVER_GOV_ABSTAIN_REQUIRE cannot be combined with DELIVER_GOV_REQUIRE"
    );
    let ibc_recover_client = gov_required
        .iter()
        .any(|url| url == "/ibc.core.client.v1.MsgRecoverClient");
    anyhow::ensure!(
        !ibc_recover_client || ibc_on,
        "DELIVER_GOV_REQUIRE=/ibc.core.client.v1.MsgRecoverClient requires DELIVER_IBC"
    );
    anyhow::ensure!(
        !ibc_frozen_client_required || !ibc_recover_client,
        "DELIVER_IBC_FROZEN_CLIENT_REQUIRE cannot be combined with MsgRecoverClient coverage"
    );
    let type_required = std::env::var_os("DELIVER_TYPE_REQUIRE")
        .map(|v| {
            v.into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_TYPE_REQUIRE must be UTF-8"))
        })
        .transpose()?;
    let type_required = type_requirements(type_required.as_deref())?;
    let gov_failure_required = std::env::var_os("DELIVER_GOV_FAILURE_REQUIRE")
        .map(|value| {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_GOV_FAILURE_REQUIRE must be UTF-8"))?;
            anyhow::ensure!(
                !value.is_empty(),
                "DELIVER_GOV_FAILURE_REQUIRE must be nonempty"
            );
            Ok::<_, anyhow::Error>(value)
        })
        .transpose()?;
    let gov_failure_type_required = std::env::var_os("DELIVER_GOV_FAILURE_TYPE_REQUIRE")
        .map(|value| {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_GOV_FAILURE_TYPE_REQUIRE must be UTF-8"))?;
            anyhow::ensure!(
                value.starts_with('/') && value.len() > 1,
                "DELIVER_GOV_FAILURE_TYPE_REQUIRE must be an absolute type URL"
            );
            Ok::<_, anyhow::Error>(value)
        })
        .transpose()?;
    anyhow::ensure!(
        gov_failure_type_required.is_none() || gov_failure_required.is_some(),
        "DELIVER_GOV_FAILURE_TYPE_REQUIRE requires DELIVER_GOV_FAILURE_REQUIRE"
    );
    anyhow::ensure!(
        !gov_abstain_required || gov_failure_required.is_none(),
        "DELIVER_GOV_ABSTAIN_REQUIRE cannot be combined with DELIVER_GOV_FAILURE_REQUIRE"
    );
    let script_path = std::env::var_os("DELIVER_SCRIPT")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_SCRIPT must be UTF-8"))
        })
        .transpose()?;
    let script_required = std::env::var_os("DELIVER_SCRIPT_REQUIRE")
        .map(|path| -> anyhow::Result<_> {
            anyhow::ensure!(
                script_path.is_some(),
                "DELIVER_SCRIPT_REQUIRE needs DELIVER_SCRIPT"
            );
            script_expectations(&std::fs::read_to_string(path)?)
        })
        .transpose()?
        .unwrap_or_default();
    let allow_postupgrade = std::env::var_os("DELIVER_HL_ALLOW_POSTUPGRADE")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_HL_ALLOW_POSTUPGRADE must be UTF-8"))
        })
        .transpose()?;
    let allow_postupgrade = allow_postupgrade_hl(
        allow_postupgrade.as_deref(),
        script_path.is_some(),
        gov_failure_required.is_some(),
    )?;
    if ibc_legacy_upgrade_fixture {
        anyhow::ensure!(
            script_path.as_deref()
                == Some("crates/fuzz-diff/fixtures/gov-duplicate-error-receipt.script")
                && gov_failure_required.as_deref() == Some("upgrade.go:1048")
                && std::env::var_os("DELIVER_UPGRADE").is_some()
                && allow_postupgrade,
            "DELIVER_IBC_LEGACY_UPGRADE_FIXTURE is restricted to the upgrade.go:1048 governance fixture"
        );
        anyhow::ensure!(
            !ibc_connection_confirm
                && !ibc_close_init_failure_required
                && !ibc_recover_client
                && !ibc_ack_failure_required
                && !ibc_pfm_required
                && !ibc_pfm_error_required
                && !ibc_pfm_timeout_required
                && !ibc_pfm_exhaust_required
                && !ibc_pfm_metadata_required
                && !ibc_ica_encodings_required
                && !ibc_ica_order_required
                && !ibc_upgrade_timeout_required
                && !ibc_upgrade_timeout_boundary_required
                && !ibc_upgrade_confirm_timeout_required
                && !ibc_upgrade_try_fast_forward_required
                && !ibc_upgrade_open_flushcomplete_required
                && !ibc_ica_error_required
                && !ibc_expired_client_required
                && !ibc_frozen_client_required,
            "DELIVER_IBC_LEGACY_UPGRADE_FIXTURE cannot be combined with connection-confirm, close-init, recover-client, Ack-failure, PFM, ICA, expired-client, or frozen-client requirements"
        );
    }
    if ibc_upgrade_timeout_required
        || ibc_upgrade_timeout_boundary_required
        || ibc_upgrade_confirm_timeout_required
        || ibc_upgrade_try_fast_forward_required
        || ibc_upgrade_open_flushcomplete_required
    {
        anyhow::ensure!(
            !ibc_connection_confirm
                && !ibc_close_init_failure_required
                && !ibc_legacy_upgrade_fixture
                && !ibc_ack_failure_required
                && !ibc_recover_client
                && !ibc_pfm_required
                && !ibc_pfm_error_required
                && !ibc_pfm_timeout_required
                && !ibc_pfm_exhaust_required
                && !ibc_pfm_metadata_required
                && !ibc_ica_encodings_required
                && !ibc_ica_order_required
                && !ibc_ica_error_required
                && !ibc_expired_client_required
                && !ibc_frozen_client_required
                && script_path.is_none()
                && std::env::var_os("DELIVER_UPGRADE").is_none(),
            "controlled IBC upgrade-timeout requirements must run without other controlled IBC, script, or upgrade fixtures"
        );
    }
    anyhow::ensure!(
        gov_required.is_empty() || std::env::var("DELIVER_UPGRADE").is_err(),
        "DELIVER_GOV_REQUIRE cannot be combined with DELIVER_UPGRADE: the controlled phase would skip early upgrade fixtures"
    );
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let records = parse(&data)?;
    // `DELIVER_UPGRADE=<n>`: the replay stops before the scenario's first
    // signal tx (held back for driver block `n`, excluding isolated setup).
    let mut upg = upgrade::Upgrade::default();
    let mut stop_at = i64::MAX;
    if let Ok(n) = std::env::var("DELIVER_UPGRADE") {
        let (h, held) = upgrade::held_signals(&records);
        stop_at = h;
        upg = upgrade::Upgrade {
            on: true,
            signal_at: n.parse().unwrap_or(20),
            held,
            ..Default::default()
        };
    } else if std::env::var("DELIVER_REPLAY_STOP").as_deref() == Ok("signal") {
        // Only stop the replay there (e.g. `upgrade`, which signals v11 and
        // halts: the generated blocks then run on its gov-upgrade state).
        stop_at = upgrade::held_signals(&records).0;
    }
    anyhow::ensure!(
        !allow_postupgrade || (upg.on && hl_on && verifier.name() != "noop"),
        "DELIVER_HL_ALLOW_POSTUPGRADE=governance-fixture requires a signed DELIVER_UPGRADE run"
    );
    // Go raised the app version in the last `FinalizeBlock`: the binary
    // switches after its `Commit`.
    let mut switch_to: Option<(u64, u64)> = None;
    let mut go = GoApp {
        addr: addr.to_owned(),
        conns: BTreeMap::new(),
    };
    let mut rust_app = app::App::new();
    if upgrade_halt_required || upgrade_skip_required || upgrade_restart_required {
        let rust_home = std::env::var_os("DELIVER_UPGRADE_RUST_HOME")
            .context("upgrade halt/skip/restart fixture needs DELIVER_UPGRADE_RUST_HOME")?;
        let unsafe_skip_upgrades = if upgrade_skip_required {
            vec![
                std::env::var("DELIVER_UPGRADE_SKIP_HEIGHT")
                    .context("DELIVER_UPGRADE_SKIP_REQUIRE needs DELIVER_UPGRADE_SKIP_HEIGHT")?
                    .parse()
                    .context("DELIVER_UPGRADE_SKIP_HEIGHT must be an integer")?,
            ]
        } else {
            Vec::new()
        };
        let node = app::NodeOptions {
            home: Some(rust_home.clone().into()),
            unsafe_skip_upgrades,
            ..Default::default()
        };
        if upgrade_restart_required {
            rust_app = app::App::open_with(
                std::path::PathBuf::from(rust_home),
                app::AppOptions {
                    node,
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow::anyhow!(e.log))?;
        } else {
            rust_app.set_node_options(node);
        }
    }
    let mut rust = trace::Traced::new(rust_app)?;
    let mut stats = Stats {
        gov_required,
        gov_failure_required,
        gov_failure_type_required,
        gov_abstain_required: gov_abstain_required.then_some(()),
        type_required,
        evidence_required: evidence_requirement.map(|required| required.name().into()),
        script_required,
        direct_aux_required: direct_aux_required.then_some(()),
        malformed_multisig_required: malformed_multisig_required.then_some(()),
        zero_threshold_multisig_required: zero_threshold_multisig_required.then_some(()),
        p256_required: p256_required.then_some(()),
        pubkey_rejection_required: pubkey_rejection_required.then_some(()),
        never_bonded_unjail_required: never_bonded_unjail_required.then_some(()),
        unbonded_redelegation_rollback_required: unbonded_redelegation_rollback_required
            .then_some(()),
        gov_params_rejection_required: gov_params_rejection_required.then_some(()),
        gov_handler_rejection_required: gov_handler_rejection_required.then_some(()),
        validator_removal_rewards_required: validator_removal_rewards_required.then_some(()),
        upgrade_plan_required: upgrade_plan_required.then_some(()),
        upgrade_halt_required: upgrade_halt_required.then_some(()),
        upgrade_skip_required: upgrade_skip_required.then_some(()),
        upgrade_restart_required: upgrade_restart_required.then_some(()),
        ibc_ack_failure_required,
        hl_verification: hl_on.then(|| verifier.name().to_owned()),
        hl_registry_required: hl_registry_required.then_some(()),
        hl_preupgrade_required: upg.on && hl_on && verifier.name() != "noop" && !allow_postupgrade,
        hl_rejection_cases: if hl_reject {
            verifier
                .rejections()
                .iter()
                .map(|case| case.name().into())
                .collect()
        } else {
            Vec::new()
        },
        hl_recovery_id_required: hl_recovery_id_required.then_some(()),
        ..Stats::default()
    };
    if ibc_close_init_failure_required {
        stats.ibc_required.insert("close_init_failure");
    }
    if ibc_legacy_upgrade_fixture {
        stats.ibc_required.insert("legacy_upgrade_fixture");
    }
    if ibc_recover_client {
        stats.ibc_required.insert("recover_client");
    }
    if ibc_pfm_required {
        stats.ibc_required.insert("pfm_lifecycle");
    }
    if ibc_pfm_error_required {
        stats.ibc_required.insert("pfm_error_lifecycle");
    }
    if ibc_pfm_timeout_required {
        stats.ibc_required.insert("pfm_timeout_lifecycle");
    }
    if ibc_pfm_exhaust_required {
        stats.ibc_required.insert("pfm_exhaust_lifecycle");
    }
    if ibc_pfm_metadata_required {
        stats.ibc_required.insert("pfm_metadata_lifecycle");
    }
    if ibc_ica_encodings_required {
        stats.ibc_required.insert("ica_encoding_lifecycle");
    }
    if ibc_ica_order_required {
        stats.ibc_required.insert("ica_order_lifecycle");
    }
    if ibc_upgrade_timeout_required {
        stats.ibc_required.insert("upgrade_timeout_recovery");
    }
    if ibc_upgrade_timeout_boundary_required {
        stats.ibc_required.insert("upgrade_timeout_boundary");
    }
    if ibc_upgrade_confirm_timeout_required {
        stats.ibc_required.insert("upgrade_confirm_timeout");
    }
    if ibc_upgrade_try_fast_forward_required {
        stats.ibc_required.insert("upgrade_try_fast_forward");
    }
    if ibc_upgrade_open_flushcomplete_required {
        stats.ibc_required.insert("upgrade_open_flushcomplete");
    }
    if ibc_ica_error_required {
        stats.ibc_required.insert("ica_error_ack");
    }
    if ibc_expired_client_required {
        stats.ibc_required.insert("expired_client_update");
    }
    if ibc_frozen_client_required {
        stats.ibc_required.insert("frozen_client_update");
    }
    let mut gov_coverage = gov_coverage::Proposals::default();
    let mut m = Mutator::new(seed);
    let pool_txs: Vec<Vec<u8>> = crate::corpus::scenario_txs(scenario)?;
    for t in &pool_txs {
        m.harvest(t);
    }
    let mut app_version = 0u64;
    let mut accts = Vec::new();
    let mut chain_id = String::new();
    let mut last_fb: Option<abci::RequestFinalizeBlock> = None;
    // The validator updates of the last two replayed blocks.
    let mut recent_updates: [Vec<abci::ValidatorUpdate>; 2] = Default::default();
    let mut last_pp: Option<abci::RequestPrepareProposal> = None;
    let (mut cons_conn, mut mem_conn) = (0u32, 0u32);
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let mut req = Request::decode(r.bytes.as_slice())?;
        if matches!(req.value, Some(request::Value::Flush(_))) {
            continue;
        }
        if upgrade::request_height(&req).is_some_and(|h| h >= stop_at) {
            break;
        }
        if let Some(request::Value::CheckTx(ct)) = &req.value
            && upg.held.contains(&ct.tx)
        {
            continue;
        }
        if let Some(request::Value::InitChain(ic)) = &mut req.value {
            let (patched, a) = patch_app_state(&ic.app_state_bytes)?;
            ic.app_state_bytes = patched;
            if evidence_requirement == Some(EvidenceRequirement::Stale) {
                let evidence = ic
                    .consensus_params
                    .as_mut()
                    .and_then(|params| params.evidence.as_mut())
                    .context("DELIVER_EVIDENCE_REQUIRE=stale needs evidence consensus params")?;
                evidence.max_age_num_blocks = 1;
                evidence.max_age_duration = Some(pbjson_types::Duration {
                    seconds: 1,
                    nanos: 0,
                });
            }
            accts = a;
            chain_id.clone_from(&ic.chain_id);
            app_version = ic
                .consensus_params
                .as_ref()
                .and_then(|c| c.version.as_ref())
                .map_or(0, |v| v.app);
        }
        let g = go_value(go.call_or_exception(r.conn, &req)?)?;
        match (&req.value, g) {
            (Some(request::Value::InitChain(ic)), response::Value::InitChain(gr)) => {
                let ours = rust
                    .init_chain(ic.clone())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if ours.app_hash != gr.app_hash || ours.validators != gr.validators {
                    stats
                        .mismatches
                        .push(format!("{scenario} InitChain differs"));
                }
            }
            (Some(request::Value::FinalizeBlock(fb)), g) => {
                cons_conn = r.conn;
                stats.replayed += 1;
                let ours = rust.finalize_block(fb.clone());
                match (g, ours) {
                    (response::Value::FinalizeBlock(g), Ok(o)) => {
                        let (c, l) = finalize_diff(&g, &o);
                        if !c.is_empty() {
                            stats.mismatches.push(format!(
                                "{scenario} replay h={}: {}",
                                fb.height,
                                c.join("\n  ")
                            ));
                            stats.split = true;
                            return Ok(stats);
                        }
                        stats.log_mismatches.extend(
                            l.into_iter()
                                .map(|l| format!("{scenario} replay h={}: {l}", fb.height)),
                        );
                        if let Some(v) = g
                            .consensus_param_updates
                            .as_ref()
                            .and_then(|c| c.version.as_ref())
                            && v.app != app_version
                        {
                            switch_to = Some((app_version, v.app));
                        }
                        if let Some(v) = o
                            .consensus_param_updates
                            .as_ref()
                            .and_then(|c| c.version.as_ref())
                        {
                            app_version = v.app;
                        }
                        recent_updates.rotate_left(1);
                        recent_updates[1].clone_from(&o.validator_updates);
                    }
                    (response::Value::Exception(_), Err(_)) => {
                        // Both halted (e.g. an upgrade without handler).
                        return Ok(stats);
                    }
                    (g, o) => {
                        stats.mismatches.push(format!(
                            "{scenario} replay h={}: go={g:?} rust={:?}",
                            fb.height,
                            o.map(|_| "ok").map_err(|e| e.to_string())
                        ));
                        stats.split = true;
                        return Ok(stats);
                    }
                }
                last_fb = Some(fb.clone());
            }
            (Some(request::Value::Commit(_)), _) => {
                rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
                if let Some((from, to)) = switch_to.take() {
                    upgrade::switch_go(&mut go, from, to)?;
                    stats.upgrade.push(format!(
                        "replay h={}: Go switched binaries, app version {from} -> {to}",
                        last_fb.as_ref().map_or(0, |f| f.height)
                    ));
                }
            }
            (Some(request::Value::CheckTx(ct)), response::Value::CheckTx(g)) => {
                mem_conn = r.conn;
                let o = rust.check_tx(ct);
                if o.as_ref().map(check_fields).ok() != Some(check_fields(&g)) {
                    stats.mismatches.push(format!(
                        "{scenario} replay CheckTx {}: go={g:?} rust={o:?}",
                        hex::encode(&ct.tx)
                    ));
                }
            }
            (Some(request::Value::PrepareProposal(pp)), g) => {
                last_pp = Some(pp.clone());
                let o = rust.prepare_proposal(pp);
                let same = match (&o, &g) {
                    (Ok(o), response::Value::PrepareProposal(g)) => o == g,
                    _ => false,
                };
                if !same {
                    stats.mismatches.push(format!(
                        "{scenario} replay PrepareProposal h={} differs",
                        pp.height
                    ));
                }
            }
            (Some(request::Value::ProcessProposal(pp)), g) => {
                let o = rust.process_proposal(pp).map(|o| o.status).ok();
                let gs = match g {
                    response::Value::ProcessProposal(p) => Some(p.status),
                    _ => None,
                };
                if o != gs {
                    stats.mismatches.push(format!(
                        "{scenario} replay ProcessProposal h={}: go={gs:?} rust={o:?}",
                        pp.height
                    ));
                }
            }
            _ => {}
        }
    }
    let Some(template) = last_fb else {
        return Ok(stats);
    };
    anyhow::ensure!(
        evidence_requirement != Some(EvidenceRequirement::RemovedValidator) || app_version >= 10,
        "DELIVER_EVIDENCE_REQUIRE=removed-validator requires app version 10 or newer"
    );
    if mem_conn == 0 {
        mem_conn = cons_conn + 100;
    }
    // Generation state from the scenario.
    let mut gn = Gen {
        m,
        dict: Dict::default(),
        accts,
        scenario_msgs: Vec::new(),
        validators: Vec::new(),
        proposals: Vec::new(),
        unbondings: Vec::new(),
        proposers: Vec::new(),
        gas_seen: BTreeMap::new(),
        hl_objects: Vec::new(),
        hl: HlState::default(),
        hex_ids: Vec::new(),
        chain_id,
        app_version,
        height: template.height,
        types: types_for(app_version, upg.on),
        force_amino: false,
        wire: WireGen::default(),
        gov_msgs: Vec::new(),
        reserved_account: None,
    };
    gn.wire.on = std::env::var("DELIVER_WIRE").is_ok();
    let mut hl_seq = HlSeq {
        verifier,
        on: hl_on,
        v10: app_version >= 10,
        ..HlSeq::default()
    };
    let mut ids = BTreeSet::new();
    for t in &pool_txs {
        for w in t.windows(66) {
            if w.starts_with(b"0x") && w[2..].iter().all(u8::is_ascii_hexdigit) {
                ids.insert(String::from_utf8_lossy(w).into_owned());
            }
        }
        let Ok(tx) = txdecode::decode_for(app_version, t) else {
            continue;
        };
        for a in &tx.body.messages {
            if let Some(md) = desc_for_type_url(a.type_url.as_bytes()) {
                gn.dict.harvest(&md, &a.value, 0);
                gn.scenario_msgs.push(a.clone());
            }
        }
    }
    gn.hex_ids = ids.into_iter().collect();
    for a in &gn.accts {
        gn.dict.string("sender", a.acc.as_bytes());
    }
    for (k, v) in &gn.dict.strings {
        if k.contains("validator") {
            for x in v {
                if x.starts_with(b"celestiavaloper1") {
                    let s = String::from_utf8_lossy(x).into_owned();
                    if !gn.validators.contains(&s) {
                        gn.validators.push(s);
                    }
                }
            }
        }
    }
    if gn.validators.is_empty() {
        // No operator address in the scenario's messages and events: the
        // bonded validators from the Rust app (same state as Go).
        gn.validators.extend(bonded_validators(&mut rust));
    }
    if let Some(ids) = gn.dict.ints.get("proposal_id") {
        gn.proposals.clone_from(ids);
    }
    // Delegate a large stake from every fuzz account so their votes pass
    // proposals.
    let mut height = template.height;
    let mut time = template.time.unwrap_or_default();
    let max_tx_bytes = last_pp.as_ref().map_or(7_897_088, |p| p.max_tx_bytes);
    let mut bootstrap = true;
    let initial: ValSet = template
        .decided_last_commit
        .as_ref()
        .map(|c| {
            c.votes
                .iter()
                .filter_map(|v| {
                    v.validator
                        .as_ref()
                        .map(|x| (x.address.clone(), x.power, false))
                })
                .collect()
        })
        .unwrap_or_default();
    let no_votes = initial.is_empty();
    // valsets[h]: the set that signs block h.
    let mut valsets: BTreeMap<i64, ValSet> = BTreeMap::new();
    // The updates of block h change the set of block h + 2 (the first
    // generated block looks up the set of the block after it).
    valsets.insert(template.height, initial.clone());
    let next = apply_updates(&initial, &recent_updates[0]);
    valsets.insert(
        template.height + 2,
        apply_updates(&next, &recent_updates[1]),
    );
    valsets.insert(template.height + 1, next);
    let mut times: BTreeMap<i64, pbjson_types::Timestamp> = BTreeMap::new();
    // Gas sweep: successful single-signer txs re-sent with gas limits
    // spread below what Go used, so they run out of gas at many points of
    // the message handlers (store access order and iterator charges).
    let sweep_on = std::env::var("DELIVER_SWEEP").is_ok();
    // A mock IBC counterparty driving the IBC success paths.
    let mut ibc = ibc_on.then(|| {
        crate::ibcmock::Driver::default()
            .with_connection_confirm(ibc_connection_confirm)
            .with_close_init_failure_requirement(ibc_close_init_failure_required)
            .with_recover_client(ibc_recover_client)
            .with_pfm_requirement(ibc_pfm_required)
            .with_pfm_error_requirement(ibc_pfm_error_required)
            .with_pfm_timeout_requirement(ibc_pfm_timeout_required)
            .with_pfm_exhaust_requirement(ibc_pfm_exhaust_required)
            .with_pfm_metadata_requirement(ibc_pfm_metadata_required)
            .with_ica_encoding_requirement(ibc_ica_encodings_required)
            .with_ica_order_requirement(ibc_ica_order_required)
            .with_upgrade_timeout_requirement(ibc_upgrade_timeout_required)
            .with_upgrade_timeout_boundary_requirement(ibc_upgrade_timeout_boundary_required)
            .with_upgrade_confirm_timeout_requirement(ibc_upgrade_confirm_timeout_required)
            .with_upgrade_try_fast_forward_requirement(ibc_upgrade_try_fast_forward_required)
            .with_upgrade_open_flushcomplete_requirement(ibc_upgrade_open_flushcomplete_required)
            .with_ica_error_requirement(ibc_ica_error_required)
            .with_expired_client_requirement(ibc_expired_client_required)
            .with_frozen_client_requirement(ibc_frozen_client_required)
            .with_legacy_upgrade_fixture(ibc_legacy_upgrade_fixture)
    });
    let mut ibc_upgrade_timeout_checkpoint: Option<UpgradeTimeoutCheckpoint> = None;
    let mut ibc_upgrade_initial_checkpoint: Option<UpgradeTimeoutCheckpoint> = None;
    let mut ibc_upgrade_confirm_checkpoint: Option<UpgradeTimeoutCheckpoint> = None;
    // The relayer: the last single-key fuzz account.
    let relayer = ACCOUNTS - 1;
    let mut script = match script_path {
        Some(path) => read_script(&std::fs::read_to_string(path)?, gn.accts.len())?,
        None => std::collections::VecDeque::new(),
    };
    let mut legacy_upgrade_fixture_submit_tx = None;
    let mut legacy_upgrade_fixture_proposal_id = None;
    let transfer_authz_plan = transfer_authz_required
        .then(|| transfer_authz_plan(&script, &stats.script_required))
        .transpose()?;
    let mut transfer_authz_sequence_start = None;
    let stake_lifecycle_plan = (evidence_requirement == Some(EvidenceRequirement::StakeLifecycle))
        .then(|| stake_lifecycle_plan(&script))
        .transpose()?;
    let mut stake_lifecycle_evidence = None;
    let mut direct_aux_fixture: Option<DirectAuxFixture> = None;
    let mut malformed_multisig_fixture: Option<MalformedMultisigFixture> = None;
    let mut zero_threshold_multisig_fixture: Option<ZeroThresholdMultisigFixture> = None;
    let mut p256_fixture: Option<P256Fixture> = None;
    let mut pubkey_rejection_fixture: Option<PubkeyRejectionFixture> = None;
    let mut never_bonded_unjail_fixture: Option<NeverBondedUnjailFixture> = None;
    let mut unbonded_redelegation_rollback_fixture: Option<UnbondedRedelegationRollbackFixture> =
        None;
    let mut gov_params_rejection_fixture: Option<gov_params::Fixture> = None;
    let mut gov_handler_rejection_fixture: Option<gov_errors::Fixture> = None;
    let mut validator_removal_rewards_fixture: Option<ValidatorRemovalRewardsFixture> = None;
    let mut zero_slash_recovery: Option<(Vec<u8>, Vec<u8>, u128)> = None;
    let mut upgrade_plan_fixture = if upgrade_halt_required {
        Some(upgrade_plan::Fixture::new_missing_handler(
            height,
            app_version,
        ))
    } else if upgrade_skip_required {
        Some(upgrade_plan::Fixture::new_skip(height, app_version))
    } else if upgrade_restart_required {
        Some(upgrade_plan::Fixture::new_restart(height, app_version))
    } else {
        upgrade_plan_required.then(|| upgrade_plan::Fixture::new(height, app_version))
    };
    if upgrade_skip_required {
        let configured: i64 = std::env::var("DELIVER_UPGRADE_SKIP_HEIGHT")
            .context("DELIVER_UPGRADE_SKIP_REQUIRE needs DELIVER_UPGRADE_SKIP_HEIGHT")?
            .parse()
            .context("DELIVER_UPGRADE_SKIP_HEIGHT must be an integer")?;
        anyhow::ensure!(
            configured == height + i64::try_from(blocks).unwrap_or(i64::MAX),
            "upgrade skip height {configured} does not match final generated height {}",
            height + i64::try_from(blocks).unwrap_or(i64::MAX)
        );
    }
    anyhow::ensure!(
        !upgrade_restart_required || app_version == 9,
        "DELIVER_UPGRADE_RESTART_REQUIRE needs a v9 scenario, got app version {app_version}"
    );
    if upgrade_plan_fixture.is_some() {
        upgrade_plan::Fixture::verify_initial(&mut go, cons_conn, &mut rust)?;
    }
    if !stats.script_required.is_empty() {
        anyhow::ensure!(
            script.iter().map(Vec::len).sum::<usize>() == stats.script_required.len(),
            "DELIVER_SCRIPT_REQUIRE must list every script transaction exactly once"
        );
    }
    anyhow::ensure!(
        !allow_postupgrade || !script.is_empty(),
        "DELIVER_HL_ALLOW_POSTUPGRADE=governance-fixture requires a nonempty DELIVER_SCRIPT"
    );
    let script_at = match std::env::var_os("DELIVER_SCRIPT_AT") {
        None => 0,
        Some(value) => value
            .into_string()
            .map_err(|_| anyhow::anyhow!("DELIVER_SCRIPT_AT must be UTF-8"))?
            .parse::<usize>()
            .context("DELIVER_SCRIPT_AT must be a generated block index")?,
    };
    let time_jump_at = std::env::var_os("DELIVER_TIME_JUMP_AT")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| anyhow::anyhow!("DELIVER_TIME_JUMP_AT must be UTF-8"))?
                .parse::<usize>()
                .context("DELIVER_TIME_JUMP_AT must be a generated block index")
        })
        .transpose()?;
    let time_jump_seconds = match std::env::var_os("DELIVER_TIME_JUMP_SECONDS") {
        None => 3600,
        Some(value) => value
            .into_string()
            .map_err(|_| anyhow::anyhow!("DELIVER_TIME_JUMP_SECONDS must be UTF-8"))?
            .parse::<i64>()
            .context("DELIVER_TIME_JUMP_SECONDS must be a positive number of seconds")?,
    };
    anyhow::ensure!(
        time_jump_seconds > 0,
        "DELIVER_TIME_JUMP_SECONDS must be a positive number of seconds"
    );
    let mut sweeps: Vec<(usize, Vec<Any>, u64)> = Vec::new();
    let sim_on = std::env::var("DELIVER_SIM").is_ok();
    // `DELIVER_GOVPASS`: the fuzz accounts decide every proposal (deposit and
    // votes, re-sent every 4 blocks). Normally they vote yes so each proposal's
    // messages run. `DELIVER_GOV_ABSTAIN_REQUIRE` makes them all abstain so the
    // rejected-proposal tally path is exercised instead.
    let mut govpass_delegated = false;
    let recheck_all = std::env::var("DELIVER_RECHECK").is_ok();
    // The txs a comet mempool would still hold: accepted by Go's `CheckTx`,
    // not in a block, not removed by a recheck (oldest first, capped).
    let mut pool: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
    let mut legacy_upgrade_fixture_paused_blocks = 0usize;
    for b in 0..blocks {
        height += 1;
        gn.height = height;
        let script_blocks_at_start = script.len();
        let legacy_upgrade_fixture_anchor_ready = ibc
            .as_ref()
            .map(|driver| &driver.stats)
            .unwrap_or(&stats.ibc)
            .get("legacy_upgrade_fixture_trusted_height")
            .copied()
            .unwrap_or(0)
            > 0;
        let legacy_upgrade_fixture_intermediate_ready = ibc
            .as_ref()
            .map(|driver| &driver.stats)
            .unwrap_or(&stats.ibc)
            .get("legacy_upgrade_fixture_intermediate_height")
            .copied()
            .unwrap_or(0)
            > 0;
        let legacy_upgrade_fixture_ready = ibc
            .as_ref()
            .is_some_and(crate::ibcmock::Driver::legacy_upgrade_fixture_ready);
        let legacy_upgrade_fixture_state_ready = ibc
            .as_ref()
            .is_some_and(crate::ibcmock::Driver::legacy_upgrade_fixture_state_ready);
        let (fixture_block, legacy_upgrade_fixture_setup) = legacy_upgrade_fixture_block_index(
            b,
            &mut legacy_upgrade_fixture_paused_blocks,
            ibc_legacy_upgrade_fixture,
            legacy_upgrade_fixture_ready,
            script_at,
        );
        if ibc_legacy_upgrade_fixture && fixture_block >= script_at {
            let state = ibc
                .as_ref()
                .map(crate::ibcmock::Driver::legacy_upgrade_fixture_state);
            stats.script.push(format!(
                "h={height} legacy fixture gate intermediate={legacy_upgrade_fixture_intermediate_ready} anchor={legacy_upgrade_fixture_anchor_ready} state={state:?} ready={legacy_upgrade_fixture_ready} setup={legacy_upgrade_fixture_setup}"
            ));
        }
        let last_committed_nanos = u64::try_from(time.seconds)
            .unwrap_or_default()
            .saturating_mul(1_000_000_000)
            .saturating_add(u64::try_from(time.nanos).unwrap_or_default());
        if gov_params_rejection_required || gov_handler_rejection_required {
            time.seconds += 5;
        } else if gn.m.rng.one_in(40) {
            time.seconds += [3600, 86_400, 90, 700_000, 1_300_000][gn.m.rng.below(5)];
        } else {
            time.seconds += 5 + gn.m.rng.below(3) as i64;
        }
        if legacy_upgrade_fixture_state_ready {
            // Keep this opt-in historical fixture after the retained header
            // time. The height-43 trust anchor can finalize through either the
            // ordinary IBC step or the held setup block, so this follows the
            // logical fixture clock rather than only the setup branch.
            time.seconds = time
                .seconds
                .max(crate::ibcmock::LEGACY_UPGRADE_FIXTURE_HEADER_TIME);
        }
        if time_jump_at == Some(fixture_block) && !legacy_upgrade_fixture_setup {
            time.seconds = time
                .seconds
                .checked_add(time_jump_seconds)
                .context("DELIVER_TIME_JUMP_SECONDS overflowed the block time")?;
        }
        if let Some(plan) = stake_lifecycle_plan.as_ref()
            && stake_lifecycle_evidence.is_none()
            && script_blocks_at_start == 4
        {
            stake_lifecycle_evidence = Some(stake_lifecycle_misbehavior(
                &rust, plan, height, time, &valsets,
            )?);
        }
        // Sequences from the committed state.
        for a in &mut gn.accts {
            a.seq = rust.query_sequence(&a.addr).sequence;
        }
        let mut txs: Vec<Vec<u8>> = Vec::new();
        // Complete mandatory signed transfers before unrelated governance can
        // change proposal capacity and starve the larger Merkle-root proof tx.
        // The requested block budget still bounds setup; missing coverage fails.
        let hl_preupgrade = upg.on && stats.hl_preupgrade_setup(app_version);
        let hl_rejection_setup = stats.hl_rejection_setup();
        let hl_recovery_id_setup =
            hl_recovery_id_required && stats.hl_recovery_id_completed == 0 && hl_seq.step == 2;
        let hl_verification_setup = stats.hl_verification_setup();
        // The governance fixture permits signed paths after the binary switch.
        // Reserve their signer once v10 is active and suppress only background
        // traffic through the exact failure receipt. Keeping this exception out
        // of `hl_setup` preserves the upgrade/script clock.
        let hl_postupgrade_setup = allow_postupgrade && app_version >= 10 && hl_verification_setup;
        let fixture_background_setup =
            allow_postupgrade && app_version >= 10 && stats.gov_failure_matches == 0;
        let hl_account_setup = hl_preupgrade || hl_postupgrade_setup;
        // Explicit registry and signed-verification requirements cover their
        // complete controlled sequences. Keep unrelated generated traffic from
        // changing mailbox, token or account state before the exact
        // transactions finalize.
        let hl_setup = hl_rejection_setup
            || hl_preupgrade
            || hl_recovery_id_setup
            || stats.hl_registry_setup()
            || (!allow_postupgrade && hl_verification_setup);
        let hl_background_setup = hl_setup || hl_postupgrade_setup || fixture_background_setup;
        stats.hl_rejection_blocks += u64::from(hl_rejection_setup);
        stats.hl_preupgrade_blocks += u64::from(hl_preupgrade);
        let mut hl_pair = None;
        let mut hl_recovery_id_block = None;
        let mut scripted = legacy_upgrade_fixture_setup;
        let phase = upg.phase(height, app_version);
        let block_av = app_version;
        if ibc_recover_client
            && govpass_delegated
            && !gn
                .gov_msgs
                .iter()
                .any(|msg| msg.type_url == "/ibc.core.client.v1.MsgRecoverClient")
            && let Some((subject_client_id, substitute_client_id)) = ibc
                .as_ref()
                .and_then(crate::ibcmock::Driver::recovery_clients)
        {
            gn.gov_msgs.push(Any::pack(
                &celestia_proto::ibc::core::client::v1::MsgRecoverClient {
                    subject_client_id,
                    substitute_client_id,
                    signer: GOV.into(),
                },
            ));
        }
        let gov_setup = !hl_background_setup && gov_setup_ready(&stats, &gn.gov_msgs);
        scripted |= gov_setup;
        // A signed Hyperlane sequence needs its dedicated account's committed
        // sequence. Before both incoming paths succeed at v9, keep unrelated
        // mempool traffic off that account and defer the held upgrade signal.
        gn.reserved_account = hl_account_setup.then_some(HL_ACCT);
        // The blocks around the upgrade height also take every generated
        // tx unfiltered (the first v10 block's `CheckTx` runs on the fresh
        // app's empty check state and rejects most of them).
        let force_raw = legacy_upgrade_fixture_setup
            || ((malformed_multisig_required || p256_required || pubkey_rejection_required)
                && b > 0)
            || (upg.on
                && upg
                    .upgrade_height
                    .is_some_and(|h| (h..=h + 2).contains(&height)));
        let upgrade_block = upg.block_index(b, hl_setup, bootstrap);
        if upg.on && !bootstrap && !hl_setup && !legacy_upgrade_fixture_setup {
            txs = upg.txs(&mut gn, upgrade_block, height, &hl_seq.token, hl_preupgrade);
        }
        if bootstrap {
            bootstrap = false;
            if let Some(v) = gn.validators.first().cloned() {
                for i in 0..gn.accts.len() {
                    if hl_account_setup && i == HL_ACCT {
                        continue;
                    }
                    if (zero_threshold_multisig_required || p256_required)
                        && i + 1 == gn.accts.len()
                    {
                        continue;
                    }
                    if pubkey_rejection_required && i + 3 >= gn.accts.len() {
                        continue;
                    }
                    let msg = Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                        delegator_address: gn.accts[i].acc.clone(),
                        validator_address: v.clone(),
                        amount: Some(Coin {
                            denom: "utia".into(),
                            amount: "100000000000000".into(),
                        }),
                    });
                    if let Some(t) = sign_simple(&gn, i, vec![msg], 1_000_000) {
                        txs.push(t);
                    }
                }
            }
        } else if direct_aux_required && b == 1 {
            let fixture = DirectAuxFixture::new(&gn, &mut rust)?;
            txs.push(fixture.bad.clone());
            txs.push(fixture.good.clone());
            direct_aux_fixture = Some(fixture);
        } else if malformed_multisig_required {
            if malformed_multisig_fixture.is_none() {
                malformed_multisig_fixture = Some(MalformedMultisigFixture::new(&gn, &mut rust)?);
            }
            let fixture = malformed_multisig_fixture
                .as_ref()
                .context("malformed multisig fixture was not initialized")?;
            txs.push(fixture.cases[b - 1].tx.clone());
        } else if zero_threshold_multisig_required {
            if zero_threshold_multisig_fixture.is_none() {
                zero_threshold_multisig_fixture =
                    Some(ZeroThresholdMultisigFixture::new(&gn, &mut rust)?);
            }
            let fixture = zero_threshold_multisig_fixture
                .as_ref()
                .context("zero-threshold multisig fixture was not initialized")?;
            txs.push(fixture.txs[b - 1].clone());
        } else if p256_required {
            if p256_fixture.is_none() {
                p256_fixture = Some(P256Fixture::new(&gn, &mut rust)?);
            }
            let fixture = p256_fixture
                .as_ref()
                .context("P-256 fixture was not initialized")?;
            txs.push(fixture.txs[b - 1].clone());
        } else if pubkey_rejection_required {
            if pubkey_rejection_fixture.is_none() {
                pubkey_rejection_fixture = Some(PubkeyRejectionFixture::new(&gn, &mut rust)?);
            }
            let fixture = pubkey_rejection_fixture
                .as_ref()
                .context("public-key rejection fixture was not initialized")?;
            txs.push(fixture.txs[b - 1].clone());
        } else if never_bonded_unjail_required {
            if never_bonded_unjail_fixture.is_none() {
                never_bonded_unjail_fixture = Some(NeverBondedUnjailFixture::new(&gn, &rust)?);
            }
            let fixture = never_bonded_unjail_fixture
                .as_mut()
                .context("never-bonded unjail fixture was not initialized")?;
            txs.push(fixture.tx(&gn, b - 1)?);
        } else if unbonded_redelegation_rollback_required {
            if unbonded_redelegation_rollback_fixture.is_none() {
                unbonded_redelegation_rollback_fixture =
                    Some(UnbondedRedelegationRollbackFixture::new(&gn, &rust)?);
            }
            let fixture = unbonded_redelegation_rollback_fixture
                .as_mut()
                .context("unbonded redelegation rollback fixture was not initialized")?;
            txs.push(fixture.tx(&gn, b - 1)?);
        } else if validator_removal_rewards_required {
            if validator_removal_rewards_fixture.is_none() {
                validator_removal_rewards_fixture =
                    Some(ValidatorRemovalRewardsFixture::new(&gn, &rust)?);
            }
            let fixture = validator_removal_rewards_fixture
                .as_mut()
                .context("validator-removal rewards fixture was not initialized")?;
            txs.push(fixture.tx(&gn, b - 1)?);
        } else if evidence_requirement == Some(EvidenceRequirement::ZeroDoubleSignSlash) && b == 2 {
            let recipient = gn.accts[1].addr.clone();
            let initial = account_balance_readonly(&rust, &gn.accts[1].acc)?;
            let tx = sign_simple(
                &gn,
                0,
                vec![Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                    from_address: gn.accts[0].acc.clone(),
                    to_address: gn.accts[1].acc.clone(),
                    amount: vec![Coin {
                        denom: "utia".into(),
                        amount: "1".into(),
                    }],
                })],
                1_000_000,
            )
            .context("signing zero-slash recovery transaction")?;
            zero_slash_recovery = Some((tx.clone(), recipient, initial));
            txs.push(tx);
        } else if !legacy_upgrade_fixture_setup
            && !hl_setup
            && !gov_setup
            && fixture_block >= script_at
            && let Some(block) = script.pop_front()
        {
            scripted = true;
            if transfer_authz_plan.is_some() && transfer_authz_sequence_start.is_none() {
                let channel = ibc
                    .as_ref()
                    .and_then(|driver| driver.a_transfer.as_deref())
                    .context(
                        "transfer authorization fixture needs an established IBC transfer channel",
                    )?;
                anyhow::ensure!(
                    channel == "channel-0",
                    "transfer authorization fixture expected channel-0, observed {channel}"
                );
                transfer_authz_sequence_start = Some(
                    next_sequence_send(&mut rust, "transfer", channel)
                        .context("transfer authorization starting send sequence")?,
                );
            }
            let required_count = block.len();
            for (i, mut msg, gas) in block {
                // `vote <account>`: a yes vote on the fixture's proposal, or
                // the latest proposal for ordinary scripts.
                if msg.type_url == "vote" {
                    let proposal_id = script_vote_proposal_id(
                        ibc_legacy_upgrade_fixture,
                        legacy_upgrade_fixture_proposal_id,
                        &gn.proposals,
                    )
                    .with_context(|| {
                        let outcome = stats.script_outcomes.last().map(|(_, result)| {
                            format!(
                                "code={} codespace={} log={}",
                                result.code,
                                result.codespace,
                                gostr::to_go_bytes(&result.log).escape_ascii()
                            )
                        });
                        let trace = stats
                            .script
                            .iter()
                            .rev()
                            .take(8)
                            .rev()
                            .cloned()
                            .collect::<Vec<_>>();
                        format!("last scripted outcome: {outcome:?}; trace: {trace:?}")
                    })?;
                    if ibc_legacy_upgrade_fixture {
                        stats.script.push(format!(
                            "h={height} legacy fixture vote targets proposal {proposal_id}"
                        ));
                    }
                    msg = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                        proposal_id,
                        voter: gn.accts[i].acc.clone(),
                        option: 1,
                        metadata: String::new(),
                    });
                }
                let is_legacy_fixture_submission = ibc_legacy_upgrade_fixture
                    && msg.type_url == "/cosmos.gov.v1.MsgSubmitProposal";
                if is_legacy_fixture_submission {
                    let mut proposal = celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(
                        msg.value.as_slice(),
                    )?;
                    let execution_signer = proposal
                        .messages
                        .first()
                        .filter(|message| message.type_url == "/ibc.core.client.v1.MsgUpdateClient")
                        .and_then(|message| {
                            celestia_proto::ibc::core::client::v1::MsgUpdateClient::decode(
                                message.value.as_slice(),
                            )
                            .ok()
                        })
                        .map(|message| message.signer)
                        .filter(|signer| !signer.is_empty())
                        .context("legacy upgrade fixture governance execution signer")?;
                    proposal.messages = ibc
                        .as_mut()
                        .and_then(|driver| {
                            driver.legacy_upgrade_fixture_messages(time.seconds, &execution_signer)
                        })
                        .context("legacy upgrade fixture dynamic proposal prerequisites")?;
                    msg = Any::pack(&proposal);
                }
                if let Some(t) = sign_simple(&gn, i, vec![msg], gas) {
                    gn.accts[i].seq += 1;
                    if is_legacy_fixture_submission {
                        legacy_upgrade_fixture_submit_tx = Some(t.clone());
                    }
                    txs.push(t);
                }
            }
            if txs.len() != required_count {
                stats.script_incomplete = Some("script signing failed");
            }
        } else {
            // IBC uses the dedicated relayer, not the Hyperlane account. Keep
            // it active so packets can be in flight across the switch.
            if ((!hl_setup && !gov_setup) || legacy_upgrade_fixture_setup)
                && let Some(d) = ibc.as_mut()
            {
                let ibc_relayer = d
                    .upgrade_timeout_signer_offset()
                    .map_or(relayer, |offset| relayer.saturating_sub(offset));
                let accounts: Vec<(String, Vec<u8>)> = gn
                    .accts
                    .iter()
                    .take(ACCOUNTS)
                    .map(|a| (a.acc.clone(), a.addr.clone()))
                    .collect();
                let mut c = crate::ibcmock::StepCtx {
                    app_version,
                    now: time.seconds,
                    now_nanos: u64::try_from(time.seconds)
                        .unwrap_or_default()
                        .saturating_mul(1_000_000_000)
                        .saturating_add(u64::try_from(time.nanos).unwrap_or_default()),
                    last_committed_nanos,
                    rng: &mut gn.m.rng,
                    signer: gn.accts[ibc_relayer].acc.clone(),
                    accounts,
                };
                if legacy_upgrade_fixture_setup
                    && legacy_upgrade_fixture_state_ready
                    && !legacy_upgrade_fixture_intermediate_ready
                {
                    if let Some(update) = d.legacy_upgrade_fixture_intermediate_update(
                        app_version,
                        &gn.accts[ibc_relayer].acc,
                    ) && let Some(t) = sign_simple(&gn, ibc_relayer, vec![update], 8_000_000)
                    {
                        gn.accts[ibc_relayer].seq += 1;
                        txs.push(t);
                    }
                } else if legacy_upgrade_fixture_setup
                    && legacy_upgrade_fixture_state_ready
                    && !legacy_upgrade_fixture_anchor_ready
                {
                    // Reserve the held logical block for the exact historical
                    // trust anchor. The ordinary IBC state machine may not
                    // reach its steady phase until the bounded run is nearly
                    // over, leaving no blocks for the dependent proposal.
                    if let Some(update) = d.legacy_upgrade_fixture_update(
                        app_version,
                        time.seconds,
                        &gn.accts[ibc_relayer].acc,
                    ) && let Some(t) = sign_simple(&gn, ibc_relayer, vec![update], 8_000_000)
                    {
                        gn.accts[ibc_relayer].seq += 1;
                        txs.push(t);
                    }
                } else if let Some(retry) = d.upgrade_timeout_retry_tx().map(<[u8]>::to_vec) {
                    txs.push(retry);
                } else {
                    for msgs in d.step(&mut c) {
                        if let Some(t) = sign_simple(&gn, ibc_relayer, msgs.clone(), 8_000_000) {
                            gn.accts[ibc_relayer].seq += 1;
                            d.bind_upgrade_timeout_signed_tx(&msgs, t.clone());
                            txs.push(t);
                        }
                    }
                }
                for id in [d.a_transfer.clone(), d.a_client.clone(), d.a_conn.clone()]
                    .into_iter()
                    .flatten()
                {
                    gn.dict.string("source_channel", id.as_bytes());
                }
            }
            if !hl_background_setup
                && !gov_setup
                && sweep_on
                && !sweeps.is_empty()
                && !gn.m.rng.one_in(3)
            {
                let (i, msgs, used) = sweeps[gn.m.rng.below(sweeps.len())].clone();
                if !hl_account_setup || i != HL_ACCT {
                    for t in sweep_txs(&mut gn, i, &msgs, used) {
                        stats.swept += 1;
                        txs.push(t);
                    }
                }
            }
            if !legacy_upgrade_fixture_setup && !gov_setup && hl_seq.on && hl_seq.pending.is_none()
            {
                let msgs = hl_seq.msgs(&gn);
                if hl_recovery_id_setup {
                    let mut cases = Vec::new();
                    for wire_id in [29, 30] {
                        let (message, validator, location) = hyperlane::boundary_announcement(
                            gn.accts[HL_ACCT].acc.clone(),
                            hl_seq.mailbox.clone(),
                            HL_LOCAL,
                            wire_id,
                        )?;
                        let tx = sign_simple(&gn, HL_ACCT, vec![message], 3_000_000)
                            .context("signing Hyperlane recovery-id boundary tx")?;
                        gn.accts[HL_ACCT].seq += 1;
                        txs.push(tx.clone());
                        cases.push(HlRecoveryIdCase {
                            wire_id,
                            tx,
                            validator,
                            location,
                            check: None,
                        });
                    }
                    let good = sign_simple(&gn, HL_ACCT, msgs, 3_000_000)
                        .context("signing Hyperlane recovery-id recovery tx")?;
                    gn.accts[HL_ACCT].seq += 1;
                    hl_seq.pending = Some(good.clone());
                    txs.push(good.clone());
                    hl_recovery_id_block = Some(HlRecoveryIdBlock { cases, good });
                    scripted = true;
                } else {
                    let rejection = if hl_rejection_setup && matches!(hl_seq.step, 11 | 14) {
                        hl_seq.verifier.rejections().iter().copied().find(|case| {
                            stats.hl_retried.get(case.name()).copied().unwrap_or(0) == 0
                        })
                    } else {
                        None
                    };
                    let bad = if let Some(case) = rejection {
                        let invalid = case.messages(&msgs)?;
                        let tx = sign_simple(&gn, HL_ACCT, invalid, 3_000_000)
                            .context("signing rejected Hyperlane tx")?;
                        gn.accts[HL_ACCT].seq += 1;
                        txs.push(tx.clone());
                        Some((case, tx))
                    } else {
                        None
                    };
                    if let Some(t) = sign_simple(&gn, HL_ACCT, msgs, 3_000_000) {
                        gn.accts[HL_ACCT].seq += 1;
                        hl_seq.pending = Some(t.clone());
                        if let Some((case, bad)) = bad {
                            hl_pair = Some((case, bad, t.clone()));
                        }
                        txs.push(t);
                    } else if bad.is_some() {
                        anyhow::bail!("signing valid Hyperlane retry");
                    }
                }
                // The generated votes (any option) stay off the proposal.
                gn.proposals.retain(|p| *p != hl_seq.proposal);
                // Steps 16 / 17: every result and CheckTx failure printed.
                scripted |= hl_seq.step >= 16;
                if hl_seq.step == 16 && !hl_seq.delegated {
                    // Enough stake behind the fuzz accounts' votes to pass
                    // the proposal of step 16 (sent with each try of the
                    // step until a block holds them).
                    // Spread over the bonded validators (only bonded stake
                    // is tallied; one may be jailed for downtime before
                    // the tally).
                    let vals = bonded_validators(&mut rust);
                    if !vals.is_empty() {
                        for i in (0..ACCOUNTS - 1).filter(|&i| i != HL_ACCT) {
                            let v = vals[i % vals.len()].clone();
                            let msg =
                                Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                                    delegator_address: gn.accts[i].acc.clone(),
                                    validator_address: v,
                                    amount: Some(Coin {
                                        denom: "utia".into(),
                                        amount: "800000000000000".into(),
                                    }),
                                });
                            if let Some(t) = sign_simple(&gn, i, vec![msg], 1_000_000) {
                                gn.accts[i].seq += 1;
                                txs.push(t);
                            }
                        }
                    }
                }
                if hl_seq.step == 17 {
                    // Every other single-key fuzz account votes yes too.
                    for i in (0..ACCOUNTS - 1).filter(|&i| i != HL_ACCT) {
                        let vote = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                            proposal_id: hl_seq.proposal,
                            voter: gn.accts[i].acc.clone(),
                            option: 1,
                            metadata: String::new(),
                        });
                        if let Some(t) = sign_simple(&gn, i, vec![vote], 400_000) {
                            gn.accts[i].seq += 1;
                            txs.push(t);
                        }
                    }
                }
            }
            if !hl_background_setup && govpass && !govpass_delegated {
                // Stake on every bonded validator (only bonded stake is
                // tallied; one may be jailed for downtime).
                let vals = bonded_validators(&mut rust);
                if !vals.is_empty() {
                    govpass_delegated = true;
                    if !governed_fixture_required {
                        gn.gov_msgs = authority_msgs(&mut rust, &gn.accts[0].acc);
                    }
                    for i in (0..ACCOUNTS - 1).filter(|&i| !hl_account_setup || i != HL_ACCT) {
                        let msg =
                            Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                                delegator_address: gn.accts[i].acc.clone(),
                                validator_address: vals[i % vals.len()].clone(),
                                amount: Some(Coin {
                                    denom: "utia".into(),
                                    amount: "800000000000000".into(),
                                }),
                            });
                        if let Some(t) = sign_simple(&gn, i, vec![msg], 1_000_000) {
                            gn.accts[i].seq += 1;
                            txs.push(t);
                        }
                    }
                }
            } else if !hl_background_setup && gov_params_rejection_required && govpass_delegated {
                if gov_params_rejection_fixture.is_none() {
                    gov_params_rejection_fixture = Some(gov_params::Fixture::new(&gn, &mut rust)?);
                }
                txs.extend(
                    gov_params_rejection_fixture
                        .as_mut()
                        .context("governance Params rejection fixture disappeared")?
                        .txs(&mut gn)?,
                );
            } else if !hl_background_setup && gov_handler_rejection_required && govpass_delegated {
                if gov_handler_rejection_fixture.is_none() {
                    gov_handler_rejection_fixture = Some(gov_errors::Fixture::new(&gn, &mut rust)?);
                }
                txs.extend(
                    gov_handler_rejection_fixture
                        .as_mut()
                        .context("governance handler rejection fixture disappeared")?
                        .txs(&mut gn)?,
                );
            } else if !hl_background_setup && governed_upgrade_required && govpass_delegated {
                let fixture = upgrade_plan_fixture
                    .as_mut()
                    .context("upgrade-plan fixture was not initialized")?;
                if let Some(submit) = fixture.proposal(&gn.accts[1].acc) {
                    let message = Any::pack(&submit);
                    if let Some(tx) = sign_simple(&gn, 1, vec![message], 2_000_000) {
                        gn.accts[1].seq += 1;
                        fixture.mark_submit(tx.clone());
                        txs.push(tx);
                    }
                } else if fixture.needs_recovery(height) {
                    let message = Any::pack(&fixture.recovery_message(&gn, &mut rust)?);
                    if let Some(tx) = sign_simple(&gn, 0, vec![message], 400_000) {
                        gn.accts[0].seq += 1;
                        fixture.mark_recovery(tx.clone());
                        txs.push(tx);
                    }
                }
                if let Some(id) = fixture.active_proposal() {
                    if fixture.needs_deposit() {
                        let deposit = Any::pack(&celestia_proto::cosmos::gov::v1::MsgDeposit {
                            proposal_id: id,
                            depositor: gn.accts[0].acc.clone(),
                            amount: vec![Coin {
                                denom: "utia".into(),
                                amount: "10000000000".into(),
                            }],
                        });
                        if let Some(tx) = sign_simple(&gn, 0, vec![deposit], 600_000) {
                            gn.accts[0].seq += 1;
                            txs.push(tx);
                        }
                    }
                    if fixture.needs_votes() {
                        for i in 0..ACCOUNTS - 1 {
                            let vote = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                                proposal_id: id,
                                voter: gn.accts[i].acc.clone(),
                                option: 1,
                                metadata: String::new(),
                            });
                            if let Some(tx) = sign_simple(&gn, i, vec![vote], 400_000) {
                                gn.accts[i].seq += 1;
                                txs.push(tx);
                            }
                        }
                    }
                }
            } else if !hl_background_setup && govpass && (gov_setup || b % 4 == 0) {
                // Each authority message in turn, in a proposal of its own
                // (the deposits and votes below pass it).
                let msg = if gov_setup {
                    gn.gov_msgs
                        .iter()
                        .find(|msg| {
                            stats.gov_required.contains(&msg.type_url)
                                && stats.gov_executed.get(&msg.type_url).copied().unwrap_or(0) == 0
                        })
                        .cloned()
                } else if gn.gov_msgs.is_empty() {
                    None
                } else {
                    Some(gn.gov_msgs[(b / 4) % gn.gov_msgs.len()].clone())
                };
                if let Some(msg) = msg {
                    let submit = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                        messages: vec![msg],
                        initial_deposit: vec![Coin {
                            denom: "utia".into(),
                            amount: "100000000000".into(),
                        }],
                        proposer: gn.accts[1].acc.clone(),
                        metadata: String::new(),
                        title: "params".into(),
                        summary: "params".into(),
                        expedited: false,
                    });
                    if let Some(t) = sign_simple(&gn, 1, vec![submit], 2_000_000) {
                        gn.accts[1].seq += 1;
                        txs.push(t);
                    }
                }
                // One message per tx: a vote on an ended proposal fails
                // alone.
                let ids: Vec<u64> = gn.proposals.iter().rev().take(6).copied().collect();
                for &id in &ids {
                    let deposit = Any::pack(&celestia_proto::cosmos::gov::v1::MsgDeposit {
                        proposal_id: id,
                        depositor: gn.accts[0].acc.clone(),
                        amount: vec![Coin {
                            denom: "utia".into(),
                            amount: "60000000".into(),
                        }],
                    });
                    if let Some(t) = sign_simple(&gn, 0, vec![deposit], 600_000) {
                        gn.accts[0].seq += 1;
                        txs.push(t);
                    }
                    if stats
                        .type_required
                        .iter()
                        .any(|url| url == "/cosmos.gov.v1beta1.MsgDeposit")
                        && let Some(amount) = legacy_deposit_amount(&mut rust, id)
                    {
                        // Exercise the legacy deposit message against the
                        // same active v1 proposal. Legacy proposal submission
                        // is disabled by Celestia's empty legacy router, but
                        // its deposit handler remains reachable on v1
                        // proposals. Keep this opt-in so established GOVPASS
                        // finding seeds retain their exact traffic schedule.
                        let legacy_deposit =
                            Any::pack(&celestia_proto::cosmos::gov::v1beta1::MsgDeposit {
                                proposal_id: id,
                                depositor: gn.accts[0].acc.clone(),
                                amount,
                            });
                        if let Some(t) = sign_simple(&gn, 0, vec![legacy_deposit], 600_000) {
                            gn.accts[0].seq += 1;
                            txs.push(t);
                        }
                    }
                    for i in (0..ACCOUNTS - 1).filter(|&i| !hl_preupgrade || i != HL_ACCT) {
                        let vote = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                            proposal_id: id,
                            voter: gn.accts[i].acc.clone(),
                            option: if stats.gov_abstain_required.is_some() {
                                2
                            } else {
                                1
                            },
                            metadata: String::new(),
                        });
                        if let Some(t) = sign_simple(&gn, i, vec![vote], 400_000) {
                            gn.accts[i].seq += 1;
                            txs.push(t);
                        }
                    }
                }
            }
            let k = if evidence_requirement.is_some()
                || hl_background_setup
                || gov_setup
                || governed_fixture_required
                || stats.ibc_pfm_setup()
                || stats.ibc_recover_setup()
            {
                0
            } else {
                match gn.m.rng.below(20) {
                    0 => 0,
                    1 => per_block * 4,
                    _ => 1 + gn.m.rng.below(per_block.max(1)),
                }
            };
            for _ in 0..k {
                gn.wire.pending.clear();
                if let Some(t) = gn.tx() {
                    if !gn.wire.pending.is_empty() {
                        let cls = std::mem::take(&mut gn.wire.pending);
                        gn.wire.txs.insert(t.raw.clone(), cls);
                    }
                    txs.push(t.raw);
                }
            }
        }
        // CheckTx on both (sequences advance in check state).
        let mut check_rejected: Vec<bool> = Vec::new();
        for t in &txs {
            if sim_on {
                simulate_both(&mut go, cons_conn, &mut rust, app_version, t, &mut stats);
            }
            let req = abci::RequestCheckTx {
                tx: t.clone(),
                r#type: 0,
            };
            let g = go_value(go.call_or_exception(
                mem_conn,
                &Request {
                    value: Some(request::Value::CheckTx(req.clone())),
                },
            )?)?;
            let o = rust.check_tx(&req);
            stats.checked += 1;
            let multisig = t
                .windows(MULTISIG_URL.len())
                .any(|w| w == MULTISIG_URL.as_bytes());
            if multisig {
                stats.multisig.0 += 1;
            }
            if scripted && !matches!(&g, response::Value::CheckTx(c) if c.code == 0) {
                stats.script.push(format!("h={height} CheckTx: go {g:?}"));
            }
            let response::Value::CheckTx(g) = g else {
                // Go's CheckTx returned an error (undecodable tx, ...).
                check_rejected.push(true);
                if let Ok(o) = &o {
                    stats.mismatches.push(format!(
                        "{scenario} h={height} CheckTx tx={}: Go errored ({g:?}), Rust code {} log={}",
                        hex::encode(t),
                        o.code,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ));
                }
                continue;
            };
            if let Some(fixture) = upgrade_plan_fixture.as_mut() {
                fixture.observe_check(app_version, t, &g)?;
            }
            if let Some(fixture) = gov_params_rejection_fixture.as_mut() {
                fixture.observe_check(t, &g)?;
            }
            if let Some(fixture) = gov_handler_rejection_fixture.as_mut() {
                fixture.observe_check(t, &g)?;
            }
            if let Some(block) = hl_recovery_id_block.as_mut()
                && let Some(case) = block.cases.iter_mut().find(|case| case.tx == *t)
            {
                case.check = Some(g.clone());
            }
            if let Some(driver) = ibc.as_mut() {
                driver.observe_upgrade_timeout_check(t, &g);
            }
            if let Some(fixture) = direct_aux_fixture.as_mut() {
                if t == &fixture.bad {
                    fixture.bad_check = Some((g.codespace.clone(), g.code, g.log.clone()));
                } else if t == &fixture.good {
                    fixture.good_check = Some((g.codespace.clone(), g.code, g.log.clone()));
                }
            }
            if let Some(case) = malformed_multisig_fixture
                .as_mut()
                .and_then(|fixture| fixture.case_for_tx_mut(t))
            {
                case.check = Some(g.clone());
            }
            if let Some(fixture) = zero_threshold_multisig_fixture.as_mut()
                && let Some(index) = fixture.tx_index(t)
            {
                fixture.check[index] = Some(g.clone());
            }
            if let Some(fixture) = p256_fixture.as_mut()
                && let Some(index) = fixture.tx_index(t)
            {
                fixture.check[index] = Some(g.clone());
            }
            if let Some(fixture) = pubkey_rejection_fixture.as_mut()
                && let Some(index) = fixture.tx_index(t)
            {
                fixture.check[index] = Some(g.clone());
                fixture.rust_check[index] = o.as_ref().ok().cloned();
            }
            if g.code == 0 {
                stats.check_ok += 1;
                pool.push_back(t.clone());
                if pool.len() > 300 {
                    pool.pop_front();
                }
                if multisig {
                    stats.multisig.1 += 1;
                }
                // The generator signs the next tx of these accounts with
                // the next sequence.
                if let Ok(tx) = txdecode::decode_for(app_version, t)
                    && let Ok(sigs) = x_ante::txview::signers(&tx)
                {
                    for s in sigs {
                        if let Some(a) = gn.accts.iter_mut().find(|a| a.addr == s) {
                            a.seq += 1;
                        }
                    }
                }
            }
            check_rejected.push(g.code != 0);
            if upg.on && g.code != 0 {
                let reason: String = gostr::to_go_bytes(&g.log)
                    .escape_ascii()
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .take(80)
                    .collect();
                *stats
                    .upgrade_check
                    .entry(format!("{phase} {}/{} {reason}", g.codespace, g.code))
                    .or_default() += 1;
            }
            if scripted && g.code != 0 {
                stats.script.push(format!(
                    "h={height} CheckTx code {}/{}: {}",
                    g.codespace,
                    g.code,
                    gostr::to_go_bytes(&g.log).escape_ascii()
                ));
            }
            match &o {
                Ok(o) if check_fields(o) == check_fields(&g) => {
                    if o.log != g.log {
                        stats.log_mismatches.push(format!(
                            "{scenario} h={height} CheckTx log tx={}\n    go:   {}\n    rust: {}",
                            hex::encode(t),
                            gostr::to_go_bytes(&g.log).escape_ascii(),
                            gostr::to_go_bytes(&o.log).escape_ascii()
                        ));
                    }
                }
                _ => stats.mismatches.push(format!(
                    "{scenario} h={height} CheckTx tx={}\n  go=(code {} {} gas {}/{} prio {}) log={}\n  rust={:?}",
                    hex::encode(t),
                    g.code,
                    g.codespace,
                    g.gas_used,
                    g.gas_wanted,
                    g.priority,
                    gostr::to_go_bytes(&g.log).escape_ascii(),
                    o.as_ref().map(|o| format!(
                        "(code {} {} gas {}/{} prio {}) log={}",
                        o.code,
                        o.codespace,
                        o.gas_used,
                        o.gas_wanted,
                        o.priority,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ))
                )),
            }
        }
        for (t, rej) in txs.iter().zip(&check_rejected) {
            if *rej {
                for u in type_urls(app_version, t) {
                    stats.per_type.entry(u).or_default().2 += 1;
                }
                for c in gn.wire.txs.get(t).into_iter().flatten() {
                    stats.wire.entry((*c).to_owned()).or_default().2 += 1;
                }
            }
        }
        // Votes of the last block's validators.
        // The commit of the previous block, by its signers (fuzz
        // validators sometimes absent); sometimes evidence against one.
        // A scenario recorded without commit votes (the ibctesting
        // fixtures: validators without slashing signing info) gets none.
        let signers = if no_votes {
            Vec::new()
        } else {
            valsets.get(&(height - 1)).cloned().unwrap_or_default()
        };
        let commit = abci::CommitInfo {
            round: 0,
            votes: signers
                .iter()
                .map(|(a, p, fuzz)| abci::VoteInfo {
                    validator: Some(abci::Validator {
                        address: a.clone(),
                        power: *p,
                    }),
                    block_id_flag: if *fuzz && gn.m.rng.one_in(3) {
                        [1, 3][gn.m.rng.below(2)]
                    } else {
                        2
                    },
                })
                .collect(),
        };
        let mut misbehavior = Vec::new();
        if evidence_requirement == Some(EvidenceRequirement::RemovedValidator) && b == 0 {
            anyhow::ensure!(
                signers
                    .iter()
                    .all(|(address, _, _)| address.as_slice() != CONTROLLED_REMOVED_VALIDATOR),
                "controlled removed-validator address is present in the current validator set"
            );
            misbehavior.push(removed_validator_misbehavior(
                height,
                time,
                signers.iter().map(|v| v.1).sum(),
            ));
        } else if evidence_requirement == Some(EvidenceRequirement::Stale) && b == 2 {
            misbehavior.push(stale_misbehavior(height, &valsets, &times)?);
        } else if evidence_requirement == Some(EvidenceRequirement::StakeLifecycle)
            && script_blocks_at_start == 0
            && stats.evidence_injected.is_empty()
        {
            anyhow::ensure!(
                stats.script_outcomes.len() == 5
                    && stats
                        .script_outcomes
                        .iter()
                        .all(|(_, result)| result.code == 0 && result.codespace.is_empty()),
                "stake-lifecycle setup did not finalize five successful scripted transactions"
            );
            misbehavior.push(
                stake_lifecycle_evidence
                    .clone()
                    .context("stake-lifecycle never captured an infraction validator set")?,
            );
        } else if evidence_requirement == Some(EvidenceRequirement::ZeroDoubleSignSlash) && b == 1 {
            misbehavior.push(active_double_sign_misbehavior(height, &valsets, &times)?);
        } else if evidence_requirement.is_none() && !no_votes && gn.m.rng.one_in(20) {
            let back = 1 + gn.m.rng.below(4) as i64;
            let h = (height - back).max(template.height + 1);
            if let Some(set) = valsets.get(&h) {
                let fuzzv: Vec<_> = set
                    .iter()
                    // `DELIVER_HL` keeps the scenario's validators bonded
                    // for its proposal's tally.
                    // `DELIVER_UPGRADE` too (its validator's signal
                    // decides the upgrade).
                    .filter(|v| (v.2 && !upg.late) || (!hl_seq.on && !upg.on && gn.m.rng.one_in(50)))
                    .cloned()
                    .collect();
                if let Some((a, p, _)) = gn.pick(&fuzzv) {
                    misbehavior.push(abci::Misbehavior {
                        r#type: 1 + i32::from(gn.m.rng.one_in(5)),
                        validator: Some(abci::Validator {
                            address: a,
                            power: p,
                        }),
                        height: h,
                        time: times.get(&h).copied().or(Some(time)),
                        total_voting_power: set.iter().map(|v| v.1).sum(),
                    });
                }
            }
        }
        let evidence_state_before = evidence_requirement
            .filter(|r| {
                !matches!(
                    r,
                    EvidenceRequirement::StakeLifecycle | EvidenceRequirement::ZeroDoubleSignSlash
                )
            })
            .zip(misbehavior.first())
            .map(|(requirement, evidence)| {
                require_evidence_branch_precondition(requirement, &rust, evidence, height, &time)
            })
            .transpose()?;
        let zero_slash_state_before = evidence_requirement
            .filter(|r| *r == EvidenceRequirement::ZeroDoubleSignSlash)
            .zip(misbehavior.first())
            .map(|(_, evidence)| {
                let state = zero_slash_state(&rust, evidence, &time)?;
                require_zero_slash_precondition(&state)?;
                Ok::<_, anyhow::Error>(state)
            })
            .transpose()?;
        let stake_lifecycle_state_before = evidence_requirement
            .filter(|r| *r == EvidenceRequirement::StakeLifecycle)
            .zip(stake_lifecycle_plan.as_ref())
            .zip(misbehavior.first())
            .map(|((_, plan), evidence)| {
                let state = stake_lifecycle_state(&rust, plan, evidence, &time)?;
                require_stake_lifecycle_precondition(&state, evidence)?;
                Ok::<_, anyhow::Error>(state)
            })
            .transpose()?;
        let prep = abci::RequestPrepareProposal {
            max_tx_bytes,
            txs: txs.clone(),
            local_last_commit: Some(abci::ExtendedCommitInfo {
                round: commit.round,
                votes: commit
                    .votes
                    .iter()
                    .map(|v| abci::ExtendedVoteInfo {
                        validator: v.validator.clone(),
                        vote_extension: Vec::new(),
                        extension_signature: Vec::new(),
                        block_id_flag: v.block_id_flag,
                    })
                    .collect(),
            }),
            misbehavior: misbehavior.clone(),
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
        };
        let gp = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::PrepareProposal(prep.clone())),
            },
        )?)?;
        let response::Value::PrepareProposal(gp) = gp else {
            stats.mismatches.push(format!(
                "{scenario} h={height} PrepareProposal: Go exception"
            ));
            stats.split = true;
            return Ok(stats);
        };
        if let Some(fixture) = malformed_multisig_fixture.as_mut() {
            for tx in &txs {
                if let Some(case) = fixture.case_for_tx_mut(tx) {
                    case.prepare_excluded = !gp.txs.contains(tx);
                }
            }
        }
        if let Some(fixture) = zero_threshold_multisig_fixture.as_mut() {
            for tx in &txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.prepare_included[index] = gp.txs.contains(tx);
                }
            }
        }
        if let Some(fixture) = p256_fixture.as_mut() {
            for tx in &txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.prepare_included[index] = gp.txs.contains(tx);
                }
            }
        }
        if let Some(fixture) = pubkey_rejection_fixture.as_mut() {
            for tx in &txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.prepare_included[index] = gp.txs.contains(tx);
                }
            }
        }
        let op = rust.prepare_proposal(&prep);
        if op.as_ref().ok() != Some(&gp) {
            stats.mismatches.push(format!(
                "{scenario} h={height} PrepareProposal differs: go txs={} square={} rust={:?}\n  go only={}\n  rust only={}\n  txs={}",
                gp.txs.len(),
                gp.square_size,
                op.as_ref().map(|o| (o.txs.len(), o.square_size)).map_err(|e| e.log.clone()),
                gp.txs
                    .iter()
                    .filter(|t| op.as_ref().is_ok_and(|o| !o.txs.contains(t)))
                    .map(hex::encode)
                    .collect::<Vec<_>>()
                    .join(","),
                op.as_ref()
                    .map(|o| {
                        o.txs
                            .iter()
                            .filter(|t| !gp.txs.contains(t))
                            .map(hex::encode)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default(),
                txs.iter().map(hex::encode).collect::<Vec<_>>().join(",")
            ));
        }
        let hash = hash::sha256(&height.to_be_bytes()).to_vec();
        let proc = abci::RequestProcessProposal {
            txs: gp.txs.clone(),
            proposed_last_commit: Some(commit.clone()),
            misbehavior: misbehavior.clone(),
            hash: hash.clone(),
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
            square_size: gp.square_size,
            data_root_hash: gp.data_root_hash.clone(),
            ..Default::default()
        };
        let gpp = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::ProcessProposal(proc.clone())),
            },
        )?)?;
        let gpp = match gpp {
            response::Value::ProcessProposal(p) => Some(p.status),
            _ => None,
        };
        let opp = rust.process_proposal(&proc).map(|p| p.status).ok();
        if gpp != opp {
            stats.mismatches.push(format!(
                "{scenario} h={height} ProcessProposal go={gpp:?} rust={opp:?} txs={}",
                gp.txs.iter().map(hex::encode).collect::<Vec<_>>().join(",")
            ));
        }
        if let Some(fixture) = zero_threshold_multisig_fixture.as_mut()
            && gpp == Some(abci::response_process_proposal::ProposalStatus::Accept as i32)
        {
            for tx in &gp.txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.process_accepted[index] = true;
                }
            }
        }
        if let Some(fixture) = p256_fixture.as_mut()
            && gpp == Some(abci::response_process_proposal::ProposalStatus::Accept as i32)
        {
            for tx in &gp.txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.process_status[index] = gpp;
                }
            }
        }
        if let Some(fixture) = pubkey_rejection_fixture.as_mut()
            && gpp == Some(abci::response_process_proposal::ProposalStatus::Accept as i32)
        {
            for tx in &gp.txs {
                if let Some(index) = fixture.tx_index(tx) {
                    fixture.process_status[index] = Some(gpp.unwrap_or_default());
                }
            }
        }
        // An unfiltered proposal of every generated tx too (data root
        // recomputed), every few blocks; when both accept it, it is
        // sometimes the block finalized (a proposer that skips
        // PrepareProposal's filter).
        // A block Go's ProcessProposal rejects is not finalized: an empty
        // block takes its place.
        let accepted = Some(abci::response_process_proposal::ProposalStatus::Accept as i32);
        let mut final_txs = if gpp == accepted {
            gp.txs.clone()
        } else {
            Vec::new()
        };
        if (scripted || force_raw || gn.m.rng.one_in(3)) && !txs.is_empty() {
            let mut raw = proc.clone();
            raw.txs.clone_from(&txs);
            if let Ok((size, root)) = app::proposal::block_data_root(app_version, &raw.txs) {
                raw.square_size = size;
                raw.data_root_hash = root;
            }
            let g = go_value(go.call_or_exception(
                cons_conn,
                &Request {
                    value: Some(request::Value::ProcessProposal(raw.clone())),
                },
            )?)?;
            let g = match g {
                response::Value::ProcessProposal(p) => Some(p.status),
                _ => None,
            };
            let o = rust.process_proposal(&raw).map(|p| p.status).ok();
            if g != o {
                stats.mismatches.push(format!(
                    "{scenario} h={height} ProcessProposal (unfiltered) go={g:?} rust={o:?} txs={}",
                    raw.txs
                        .iter()
                        .map(hex::encode)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            } else if malformed_multisig_fixture.as_ref().is_some_and(|fixture| {
                raw.txs.len() == 1 && fixture.cases.iter().any(|case| case.tx == raw.txs[0])
            }) {
                let rejected = Some(abci::response_process_proposal::ProposalStatus::Reject as i32);
                if let Some(fixture) = malformed_multisig_fixture.as_mut() {
                    for tx in &raw.txs {
                        if let Some(case) = fixture.case_for_tx_mut(tx) {
                            case.process_rejected = g == rejected;
                        }
                    }
                }
                // These invalid transactions cannot form a consensus-valid
                // proposal. Apply them directly only in this controlled mode
                // to compare FinalizeBlock's ante result and cache rollback.
                final_txs.clone_from(&raw.txs);
            } else if p256_fixture.as_ref().is_some_and(|fixture| {
                raw.txs.len() == 1 && fixture.tx_index(&raw.txs[0]).is_some()
            }) {
                let mut fixture_index = None;
                if let Some(fixture) = p256_fixture.as_mut()
                    && let Some(index) = fixture.tx_index(&raw.txs[0])
                {
                    fixture.process_status[index] = g;
                    fixture_index = Some(index);
                }
                if fixture_index.is_some_and(|index| index < 4) {
                    // Rejected signature-boundary transactions cannot form a
                    // consensus-valid proposal. Apply them directly only in
                    // this controlled mode to compare FinalizeBlock's ante
                    // result and cache rollback.
                    final_txs.clone_from(&raw.txs);
                } else if g == accepted {
                    final_txs.clone_from(&raw.txs);
                }
            } else if pubkey_rejection_fixture.as_ref().is_some_and(|fixture| {
                raw.txs.len() == 1 && fixture.tx_index(&raw.txs[0]).is_some()
            }) {
                let mut fixture_index = None;
                if let Some(fixture) = pubkey_rejection_fixture.as_mut()
                    && let Some(index) = fixture.tx_index(&raw.txs[0])
                {
                    fixture.process_status[index] = g;
                    fixture_index = Some(index);
                }
                if fixture_index.is_some_and(|index| index >= 4) {
                    // Returned Ed25519 ante errors can be applied directly to
                    // compare FinalizeBlock rollback. Malformed-key panics are
                    // kept behind ProcessProposal's rejection: bypassing it
                    // terminates the Go ABCI process rather than returning a
                    // tx result, so it is not a consensus-valid finalization.
                    final_txs.clone_from(&raw.txs);
                } else if g == accepted {
                    final_txs.clone_from(&raw.txs);
                }
            } else if g == accepted && (scripted || force_raw || gn.m.rng.one_in(2)) {
                final_txs.clone_from(&raw.txs);
            }
            if scripted {
                stats.script.push(format!(
                    "h={height} {} txs, {} after PrepareProposal, unfiltered proposal: {g:?}",
                    txs.len(),
                    gp.txs.len()
                ));
            }
        }
        // Give the reserved Hyperlane tx a proposer that includes it without
        // changing the generator's RNG stream or suppressing other traffic.
        // ProcessProposal still has to accept the resulting block on both apps.
        if hl_preupgrade
            && let Some(t) = hl_seq.pending.as_ref()
            && !final_txs.contains(t)
        {
            let mut forced = proc.clone();
            forced.txs.clone_from(&final_txs);
            forced.txs.push(t.clone());
            if let Ok((size, root)) = app::proposal::block_data_root(app_version, &forced.txs) {
                forced.square_size = size;
                forced.data_root_hash = root;
            }
            let g = go_value(go.call_or_exception(
                cons_conn,
                &Request {
                    value: Some(request::Value::ProcessProposal(forced.clone())),
                },
            )?)?;
            let g = match g {
                response::Value::ProcessProposal(p) => Some(p.status),
                _ => None,
            };
            let o = rust.process_proposal(&forced).map(|p| p.status).ok();
            if g != o {
                stats.mismatches.push(format!(
                    "{scenario} h={height} ProcessProposal (reserved Hyperlane tx) go={g:?} rust={o:?}"
                ));
            } else if g == accepted {
                final_txs = forced.txs;
            }
        }
        if scripted && final_txs != txs {
            stats.script_incomplete = Some("script transactions dropped, reordered or replaced");
        }
        let fb = abci::RequestFinalizeBlock {
            txs: final_txs,
            decided_last_commit: Some(commit),
            misbehavior,
            hash,
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
            header: None,
        };
        let g = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::FinalizeBlock(fb.clone())),
            },
        )?)?;
        let o = rust.finalize_block(fb.clone());
        let (g, o) = match (g, o) {
            (response::Value::FinalizeBlock(g), Ok(o)) => (g, o),
            (response::Value::Exception(e), Err(rust_error)) => {
                if upgrade_halt_required || upgrade_restart_required {
                    let fixture = upgrade_plan_fixture
                        .as_ref()
                        .context("upgrade halt/restart fixture was not initialized")?;
                    let expected_height = fixture
                        .halt_height()
                        .context("upgrade fixture was not ready at a paired halt")?;
                    anyhow::ensure!(
                        height == expected_height,
                        "upgrade paired error occurred at {height}, expected {expected_height}"
                    );
                    let go_home = std::path::PathBuf::from(
                        std::env::var_os("DELIVER_UPGRADE_GO_HOME").context(
                            "upgrade halt/restart fixture needs DELIVER_UPGRADE_GO_HOME",
                        )?,
                    );
                    let rust_home = std::path::PathBuf::from(
                        std::env::var_os("DELIVER_UPGRADE_RUST_HOME").context(
                            "upgrade halt/restart fixture needs DELIVER_UPGRADE_RUST_HOME",
                        )?,
                    );
                    fixture.require_missing_handler_halt(
                        &mut go,
                        cons_conn,
                        &mut rust,
                        upgrade_plan::HaltObservation {
                            height,
                            go_error: &e.error,
                            rust_error: &rust_error.to_string(),
                            go_home: &go_home,
                            rust_home: &rust_home,
                        },
                    )?;
                    if upgrade_halt_required {
                        stats.upgrade_halt_completed += 1;
                        return Ok(stats);
                    }

                    upgrade::switch_go(&mut go, 9, 10)?;
                    rust.reopen(
                        &rust_home,
                        app::AppOptions {
                            node: app::NodeOptions {
                                home: Some(rust_home.clone()),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    )?;
                    upgrade_plan_fixture
                        .as_mut()
                        .context("upgrade restart fixture disappeared")?
                        .mark_execution_restart()?;
                    app_version = 10;
                    gn.app_version = 10;
                    gn.types = types_for(10, false);
                    let retry_go = go_value(go.call_or_exception(
                        cons_conn,
                        &Request {
                            value: Some(request::Value::FinalizeBlock(fb.clone())),
                        },
                    )?)?;
                    let retry_rust = rust.finalize_block(fb.clone());
                    match (retry_go, retry_rust) {
                        (response::Value::FinalizeBlock(g), Ok(o)) => (g, o),
                        (go, rust) => anyhow::bail!(
                            "governance upgrade retry at {height} failed: Go {go:?}, Rust {:?}",
                            rust.map(|_| "ok").map_err(|err| err.to_string())
                        ),
                    }
                } else {
                    stats
                        .log_mismatches
                        .push(format!("{scenario} h={height}: both halted: {}", e.error));
                    return Ok(stats);
                }
            }
            (g, o) => {
                stats.mismatches.push(format!(
                    "{scenario} h={height} FinalizeBlock go={g:?} rust={:?}",
                    o.map(|_| "ok").map_err(|e| e.to_string())
                ));
                stats.split = true;
                return Ok(stats);
            }
        };
        stats.blocks += 1;
        times.insert(height, time);
        let next = valsets.get(&(height + 1)).cloned().unwrap_or_default();
        valsets.insert(height + 2, apply_updates(&next, &g.validator_updates));
        valsets.retain(|h, _| *h + 8 > height);
        times.retain(|h, _| *h + 8 > height);
        if let Some((case, bad, good)) = hl_pair {
            stats.observe_hl_pair(case, &bad, &good, &fb.txs, &g.tx_results);
            if !stats.hl_rejection_setup() && stats.hl_rejection_complete_height.is_none() {
                stats.hl_rejection_complete_height = Some(height);
            }
        }
        if hl_seq.pending.is_some() {
            if let Some((t, res)) = fb
                .txs
                .iter()
                .zip(&g.tx_results)
                .find(|(t, _)| Some(*t) == hl_seq.pending.as_ref())
            {
                let urls = type_urls(app_version, t);
                let u = urls.first().cloned().unwrap_or_default();
                if res.code == 0
                    && hl_seq.step == 0
                    && u == "/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm"
                {
                    *stats
                        .hl_registry
                        .entry("merkle-root-ism".into())
                        .or_default() += 1;
                }
                if res.code == 0
                    && hl_seq.step == 2
                    && urls.iter().any(|url| {
                        url == "/hyperlane.core.interchain_security.v1.MsgAnnounceValidator"
                    })
                {
                    *stats
                        .hl_registry
                        .entry("validator-announcement".into())
                        .or_default() += 1;
                }
                if res.code == 0 && matches!(hl_seq.step, 11 | 14) {
                    let kind = if hl_seq.step == 11 {
                        "collateral"
                    } else {
                        "synthetic"
                    };
                    *stats.hl_verified.entry(kind.into()).or_default() += 1;
                    *stats
                        .hl_verified_versions
                        .entry(format!("v{block_av}/{kind}"))
                        .or_default() += 1;
                }
                hl_seq.result(&u, res);
            } else {
                // Not in the block (CheckTx or PrepareProposal dropped it).
                hl_seq
                    .results
                    .push(format!("step {} not finalized", hl_seq.step));
                hl_seq.pending = None;
            }
            for l in hl_seq.results.drain(..) {
                stats.script.push(format!("h={height} HL {l}"));
            }
        }
        for (t, res) in fb.txs.iter().zip(&g.tx_results) {
            if let Some(expected) = legacy_upgrade_fixture_submit_tx.as_deref()
                && let Some(proposal_id) = finalized_script_proposal_id(expected, t, res)?
            {
                if let Some(existing) = legacy_upgrade_fixture_proposal_id {
                    anyhow::ensure!(
                        existing == proposal_id,
                        "legacy upgrade fixture proposal ID changed from {existing} to {proposal_id}"
                    );
                }
                legacy_upgrade_fixture_proposal_id = Some(proposal_id);
                stats.script.push(format!(
                    "h={height} legacy fixture submission pinned proposal {proposal_id}"
                ));
            }
            if let Some(fixture) = upgrade_plan_fixture.as_mut() {
                fixture.observe_finalized(app_version, t, res)?;
            }
            if let Some(fixture) = gov_params_rejection_fixture.as_mut() {
                fixture.observe_finalized(t, res)?;
            }
            if let Some(fixture) = gov_handler_rejection_fixture.as_mut() {
                fixture.observe_finalized(t, res)?;
            }
            if scripted {
                if !stats.script_required.is_empty() {
                    let url = type_urls(app_version, t)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    stats.script_outcomes.push((url, res.clone()));
                }
                stats.script.push(format!(
                    "h={height} {:?} code {}/{} gas {}: {}",
                    type_urls(app_version, t),
                    res.codespace,
                    res.code,
                    res.gas_used,
                    gostr::to_go_bytes(&res.log).escape_ascii()
                ));
            }
            stats.delivered += 1;
            if res.code == 0 {
                stats.delivered_ok += 1;
            }
            for u in type_urls(app_version, t) {
                let e = stats.per_type.entry(u).or_default();
                e.0 += 1;
            }
            if res.code == 0 {
                // Proposal payloads execute only when governance passes
                // them later; a successful submission is not coverage.
                for u in executed_type_urls(app_version, t) {
                    stats.per_type.entry(u).or_default().1 += 1;
                }
                gov_coverage.submitted(app_version, t, &res.events);
            }
            for c in gn.wire.txs.remove(t).into_iter().flatten() {
                let e = stats.wire.entry(c.to_owned()).or_default();
                e.0 += 1;
                if res.code == 0 {
                    e.1 += 1;
                }
            }
            if res.code == 0
                && let Some(u) = type_urls(app_version, t).into_iter().next()
            {
                gn.gas_seen.insert(u, res.gas_used.unsigned_abs());
            }
            if res.code != 0 {
                observe_handler_error(&mut stats, app_version, t, res);
                let first = type_urls(app_version, t)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let reason: String = gostr::to_go_bytes(&res.log)
                    .escape_ascii()
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .take(90)
                    .collect();
                *stats
                    .codes
                    .entry(format!("{first} {}/{} {reason}", res.codespace, res.code))
                    .or_default() += 1;
            }
            if res.code == 0 {
                learn(&mut gn, app_version, t, res, height);
                if sweep_on && let Some(tpl) = sweep_template(&gn, app_version, t, res) {
                    sweeps.push(tpl);
                    if sweeps.len() > 64 {
                        sweeps.remove(gn.m.rng.below(64));
                    }
                }
            }
            gn.dict.harvest_events(&res.events);
        }
        gn.dict.harvest_events(&g.events);
        stats.gov_failure_matches += gov_coverage.completed(
            &g.events,
            &mut stats.gov_executed,
            stats.gov_failure_type_required.as_deref(),
            stats.gov_failure_required.as_deref(),
        );
        if let Some(fixture) = upgrade_plan_fixture.as_mut()
            && let Some(completed) = fixture.observe_block(&g.events)?
        {
            gn.proposals.retain(|proposal| *proposal != completed);
        }
        if let Some(fixture) = gov_params_rejection_fixture.as_mut() {
            fixture.observe_block(&g.events)?;
        }
        if let Some(fixture) = gov_handler_rejection_fixture.as_mut() {
            fixture.observe_block(&g.events)?;
        }
        let mut abstain_rejected_proposals = Vec::new();
        for e in g.events.iter().filter(|e| e.r#type == "active_proposal") {
            if stats.gov_abstain_required.is_some()
                && e.attributes
                    .iter()
                    .any(|a| a.key == "proposal_result" && a.value == "proposal_rejected")
                && let Some(id) = e
                    .attributes
                    .iter()
                    .find(|a| a.key == "proposal_id")
                    .and_then(|a| a.value.parse::<u64>().ok())
            {
                abstain_rejected_proposals.push(id);
            }
            for a in e.attributes.iter().filter(|a| a.key == "proposal_result") {
                *stats.gov_results.entry(a.value.clone()).or_default() += 1;
                if hl_seq.on {
                    let attrs: Vec<String> = e
                        .attributes
                        .iter()
                        .map(|a| format!("{}={}", a.key, a.value))
                        .collect();
                    stats
                        .script
                        .push(format!("h={height} HL gov {}", attrs.join(" ")));
                }
            }
        }
        if let Some(d) = ibc.as_mut() {
            d.observe_upgrade_timeout_finalized(&fb.txs, &g.tx_results);
            d.observe(&g.tx_results);
            d.observe_block_events(&g.events);
        }
        let (c, l) = finalize_diff(&g, &o);
        let block_txs = || fb.txs.iter().map(hex::encode).collect::<Vec<_>>().join(",");
        stats.log_mismatches.extend(l.into_iter().map(|l| {
            format!(
                "{scenario} h={height} (block {b}): {l}\n  txs={}",
                block_txs()
            )
        }));
        if !c.is_empty() {
            stats.mismatches.push(format!(
                "{scenario} h={height} (block {b}) FinalizeBlock:\n  {}\n  txs={}",
                c.join("\n  "),
                block_txs()
            ));
            if g.app_hash != o.app_hash {
                stats.split = true;
                if std::env::var("DELIVER_DUMP_SPLIT").is_ok() {
                    // Commit both and print the store entries that differ.
                    let commit_req = Request {
                        value: Some(request::Value::Commit(abci::RequestCommit {})),
                    };
                    go.call_or_exception(cons_conn, &commit_req)?;
                    rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
                    for line in dump_split(&mut go, cons_conn, &mut rust) {
                        stats
                            .mismatches
                            .push(format!("{scenario} h={height} store {line}"));
                    }
                }
                return Ok(stats);
            }
        }
        let commit_req = Request {
            value: Some(request::Value::Commit(abci::RequestCommit {})),
        };
        let go_commit = go_value(go.call_or_exception(cons_conn, &commit_req)?)?;
        if !matches!(go_commit, response::Value::Commit(_)) {
            stats.mismatches.push(format!(
                "{scenario} h={height} Commit: unexpected Go response {go_commit:?}"
            ));
            stats.split = true;
            return Ok(stats);
        }
        rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(block) = hl_recovery_id_block.take() {
            match require_hl_recovery_id_block(
                &block,
                &fb.txs,
                &g.tx_results,
                app_version,
                &hl_seq.mailbox,
                &mut go,
                cons_conn,
                &mut rust,
            ) {
                Ok(()) => stats.hl_recovery_id_completed += 1,
                Err(error) => stats.hl_recovery_id_failures.push(error.to_string()),
            }
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_try_fast_forward_needs_initial_checkpoint()
        {
            let checkpoint =
                capture_upgrade_timeout_checkpoint(&mut go, cons_conn, &mut rust, driver)?;
            let (connection, old_version, _, sequence) = driver
                .upgrade_timeout_expected_state()
                .context("upgrade fast-forward initial state is incomplete")?;
            let channel = celestia_proto::ibc::core::channel::v1::Channel::decode(
                checkpoint.channel.as_slice(),
            )
            .context("decoding initial committed ICA Channel")?;
            let initial_next_send = u64::from_be_bytes(
                checkpoint
                    .next_sequence_send
                    .as_slice()
                    .try_into()
                    .context("initial ICA next send sequence is not eight bytes")?,
            );
            anyhow::ensure!(
                channel.state == celestia_proto::ibc::core::channel::v1::State::Open as i32
                    && channel.ordering
                        == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                    && channel.version == old_version
                    && channel.connection_hops == [connection]
                    && channel.upgrade_sequence == sequence
                    && checkpoint.upgrade.is_none()
                    && checkpoint.counterparty_upgrade.is_none()
                    && checkpoint.error_receipt.is_none()
                    && initial_next_send == 1
                    && !checkpoint.client_prefix.is_empty(),
                "initial ICA channel is not clean OPEN state: {channel:?}"
            );
            ibc_upgrade_initial_checkpoint = Some(checkpoint);
            driver.confirm_upgrade_try_fast_forward_initial_checkpoint();
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_timeout_needs_try_checkpoint()
        {
            let checkpoint =
                capture_upgrade_timeout_checkpoint(&mut go, cons_conn, &mut rust, driver)?;
            let (connection, old_version, proposed_version, sequence) = driver
                .upgrade_timeout_expected_state()
                .context("upgrade-timeout expected state is incomplete")?;
            anyhow::ensure!(
                connection == crate::ibcmock::CP_CONNECTION,
                "upgrade-timeout predicate requires local proposed hop {connection} to equal counterparty connection {}",
                crate::ibcmock::CP_CONNECTION
            );
            let channel = celestia_proto::ibc::core::channel::v1::Channel::decode(
                checkpoint.channel.as_slice(),
            )
            .context("decoding committed local Channel after Try")?;
            anyhow::ensure!(
                channel.state == celestia_proto::ibc::core::channel::v1::State::Flushing as i32
                    && channel.ordering
                        == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                    && channel.version == old_version
                    && channel.connection_hops == [connection]
                    && channel.upgrade_sequence == sequence,
                "committed local Channel after Try has unexpected fields: {channel:?}"
            );
            if driver.upgrade_success_required() {
                let initial = ibc_upgrade_initial_checkpoint
                    .as_ref()
                    .context("upgrade fast-forward has no initial raw checkpoint")?;
                let initial_channel = celestia_proto::ibc::core::channel::v1::Channel::decode(
                    initial.channel.as_slice(),
                )
                .context("decoding initial ICA Channel for sequence comparison")?;
                anyhow::ensure!(
                    sequence
                        == initial_channel.upgrade_sequence.saturating_add(
                            if driver.upgrade_try_fast_forward_required() {
                                2
                            } else {
                                1
                            },
                        )
                        && initial.next_sequence_send == checkpoint.next_sequence_send
                        && initial.client_prefix != checkpoint.client_prefix,
                    "Try did not advance the exact expected sequence or commit its client update"
                );
            }
            let upgrade = celestia_proto::ibc::core::channel::v1::Upgrade::decode(
                checkpoint
                    .upgrade
                    .as_deref()
                    .context("committed local Upgrade after Try is absent")?,
            )
            .context("decoding committed local Upgrade after Try")?;
            let fields = upgrade
                .fields
                .as_ref()
                .context("committed local Upgrade has no fields")?;
            anyhow::ensure!(
                fields.ordering == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                    && fields.connection_hops == [connection]
                    && fields.version == proposed_version,
                "committed local Upgrade has unexpected fields: {fields:?}"
            );
            let timeout = upgrade
                .timeout
                .context("committed local Upgrade has no timeout")?;
            anyhow::ensure!(
                timeout.timestamp != 0
                    && (!driver.upgrade_success_required()
                        || ibc_upgrade_initial_checkpoint
                            .as_ref()
                            .is_some_and(|initial| {
                                initial.next_sequence_send.as_slice()
                                    == upgrade.next_sequence_send.to_be_bytes()
                            }))
                    && checkpoint.counterparty_upgrade.is_none()
                    && checkpoint.error_receipt.is_none()
                    && !checkpoint.client_prefix.is_empty(),
                "committed Try checkpoint is incomplete or contains forbidden upgrade state"
            );
            ibc_upgrade_timeout_checkpoint = Some(checkpoint);
            driver.confirm_upgrade_timeout_try_checkpoint(timeout.timestamp);
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_timeout_needs_rejection_checkpoint()
        {
            let observed =
                capture_upgrade_timeout_checkpoint(&mut go, cons_conn, &mut rust, driver)?;
            ensure_upgrade_timeout_checkpoint_unchanged(
                ibc_upgrade_timeout_checkpoint
                    .as_ref()
                    .context("rejected upgrade timeout has no pre-transaction checkpoint")?,
                &observed,
            )?;
            driver.confirm_upgrade_timeout_rejection_checkpoint();
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_try_fast_forward_needs_confirm_checkpoint()
        {
            let checkpoint =
                capture_upgrade_timeout_checkpoint(&mut go, cons_conn, &mut rust, driver)?;
            let (connection, old_version, proposed_version, sequence) = driver
                .upgrade_timeout_expected_state()
                .context("upgrade fast-forward Confirm state is incomplete")?;
            let channel = celestia_proto::ibc::core::channel::v1::Channel::decode(
                checkpoint.channel.as_slice(),
            )
            .context("decoding committed local Channel after Confirm")?;
            let local_upgrade = checkpoint
                .upgrade
                .as_deref()
                .map(celestia_proto::ibc::core::channel::v1::Upgrade::decode)
                .transpose()
                .context("decoding local Upgrade after Confirm")?
                .context("local Upgrade after Confirm is absent")?;
            let counterparty_upgrade = checkpoint
                .counterparty_upgrade
                .as_deref()
                .map(celestia_proto::ibc::core::channel::v1::Upgrade::decode)
                .transpose()
                .context("decoding counterparty Upgrade after Confirm")?
                .context("counterparty Upgrade after Confirm is absent")?;
            let expected_fields = celestia_proto::ibc::core::channel::v1::UpgradeFields {
                ordering: celestia_proto::ibc::core::channel::v1::Order::Unordered as i32,
                connection_hops: vec![connection.to_owned()],
                version: proposed_version.to_owned(),
            };
            let counterparty_timeout = counterparty_upgrade.timeout.as_ref();
            let expected_next_send =
                ibc_upgrade_initial_checkpoint
                    .as_ref()
                    .and_then(|checkpoint| {
                        checkpoint
                            .next_sequence_send
                            .as_slice()
                            .try_into()
                            .ok()
                            .map(u64::from_be_bytes)
                    });
            anyhow::ensure!(
                channel.state
                    == celestia_proto::ibc::core::channel::v1::State::Flushcomplete as i32
                    && channel.ordering
                        == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                    && channel.version == old_version
                    && channel.connection_hops == [connection]
                    && channel.upgrade_sequence == sequence
                    && local_upgrade.fields.as_ref() == Some(&expected_fields)
                    && counterparty_upgrade.fields.as_ref() == Some(&expected_fields)
                    && counterparty_timeout.is_some_and(|timeout| timeout.timestamp != 0)
                    && expected_next_send == Some(counterparty_upgrade.next_sequence_send)
                    && ibc_upgrade_initial_checkpoint
                        .as_ref()
                        .is_some_and(|initial| {
                            initial.next_sequence_send == checkpoint.next_sequence_send
                        })
                    && checkpoint.error_receipt.is_none(),
                "committed fast-forward Confirm state is unexpected: channel={channel:?}, local={local_upgrade:?}, counterparty={counterparty_upgrade:?}"
            );
            ibc_upgrade_confirm_checkpoint = Some(checkpoint);
            driver.confirm_upgrade_try_fast_forward_confirm_checkpoint();
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_try_fast_forward_needs_stale_open_checkpoint()
        {
            let observed =
                capture_upgrade_timeout_checkpoint(&mut go, cons_conn, &mut rust, driver)?;
            ensure_upgrade_timeout_checkpoint_unchanged(
                ibc_upgrade_confirm_checkpoint
                    .as_ref()
                    .context("stale-sequence Open has no committed Confirm checkpoint")?,
                &observed,
            )?;
            driver.confirm_upgrade_try_fast_forward_stale_open_checkpoint();
        }
        if let Some(driver) = ibc.as_mut()
            && driver.upgrade_timeout_needs_cleanup_checkpoint()
        {
            let channel_id = driver
                .upgrade_timeout_channel()
                .context("upgrade-timeout cleanup has no channel")?;
            let client_id = driver
                .upgrade_timeout_client()
                .context("upgrade-timeout cleanup has no client")?;
            let (connection, old_version, proposed_version, sequence) = driver
                .upgrade_timeout_expected_state()
                .context("upgrade-timeout cleanup expected state is incomplete")?;
            let channel_key = format!("channelEnds/ports/icahost/channels/{channel_id}");
            let channel_bytes =
                query_raw_store_both(&mut go, cons_conn, &mut rust, "ibc", &channel_key)?
                    .context("successful upgrade timeout removed its Channel")?;
            let channel =
                celestia_proto::ibc::core::channel::v1::Channel::decode(channel_bytes.as_slice())
                    .context("decoding Channel after successful upgrade timeout")?;
            let path =
                |kind: &str| format!("channelUpgrades/{kind}/ports/icahost/channels/{channel_id}");
            let upgrade =
                query_raw_store_both(&mut go, cons_conn, &mut rust, "ibc", &path("upgrades"))?;
            let counterparty_upgrade = query_raw_store_both(
                &mut go,
                cons_conn,
                &mut rust,
                "ibc",
                &path("counterpartyUpgrade"),
            )?;
            let error_receipt =
                query_raw_store_both(&mut go, cons_conn, &mut rust, "ibc", &path("upgradeError"))?;
            if driver.upgrade_success_required() {
                anyhow::ensure!(
                    channel.state == celestia_proto::ibc::core::channel::v1::State::Open as i32
                        && channel.ordering
                            == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                        && channel.version == proposed_version
                        && channel.connection_hops == [connection]
                        && channel.upgrade_sequence == sequence
                        && upgrade.is_none()
                        && counterparty_upgrade.is_none()
                        && error_receipt.is_none(),
                    "successful fast-forward Open retained old fields or upgrade state: {channel:?}"
                );
            } else {
                anyhow::ensure!(
                    channel.state == celestia_proto::ibc::core::channel::v1::State::Open as i32
                        && channel.ordering
                            == celestia_proto::ibc::core::channel::v1::Order::Unordered as i32
                        && channel.version == old_version
                        && channel.connection_hops == [connection]
                        && channel.upgrade_sequence == sequence,
                    "successful upgrade timeout restored unexpected Channel fields: {channel:?}"
                );
                anyhow::ensure!(
                    upgrade.is_none() && counterparty_upgrade.is_none(),
                    "successful upgrade timeout retained upgrade state"
                );
                let receipt_bytes =
                    error_receipt.context("successful upgrade timeout wrote no error receipt")?;
                let receipt = celestia_proto::ibc::core::channel::v1::ErrorReceipt::decode(
                    receipt_bytes.as_slice(),
                )
                .context("decoding successful upgrade-timeout error receipt")?;
                anyhow::ensure!(
                    receipt.sequence == sequence
                        && receipt.message == driver.upgrade_timeout_expected_receipt(),
                    "successful upgrade-timeout error receipt is unexpected: {receipt:?}"
                );
            }
            let client_prefix = query_raw_prefix_both(
                &mut go,
                cons_conn,
                &mut rust,
                "ibc",
                format!("clients/{client_id}/").as_bytes(),
            )?;
            anyhow::ensure!(
                ibc_upgrade_timeout_checkpoint
                    .as_ref()
                    .is_some_and(|checkpoint| checkpoint.client_prefix != client_prefix),
                "controlled channel upgrade did not commit its distinct client update"
            );
            driver.confirm_upgrade_timeout_cleanup_checkpoint();
        }
        if let Some(fixture) = direct_aux_fixture.as_mut()
            && !fixture.completed
            && txs.iter().any(|tx| tx == &fixture.good)
        {
            fixture.require_complete(&fb.txs, &g.tx_results, &mut rust)?;
            fixture.completed = true;
            stats.direct_aux_completed += 1;
        }
        if let Some(fixture) = malformed_multisig_fixture.as_mut()
            && let Some(tx) = txs.first()
            && fixture
                .case_for_tx_mut(tx)
                .is_some_and(|case| !case.completed)
        {
            fixture.require_complete(tx, &fb.txs, &g.tx_results, &mut rust)?;
            stats.malformed_multisig_completed += 1;
        }
        if let Some(fixture) = zero_threshold_multisig_fixture.as_mut()
            && let Some(tx) = txs.first()
            && let Some(index) = fixture.tx_index(tx)
            && !fixture.completed[index]
        {
            fixture.require_complete(index, &fb.txs, &g.tx_results, &mut rust)?;
            stats.zero_threshold_multisig_completed += 1;
        }
        if let Some(fixture) = p256_fixture.as_mut()
            && let Some(tx) = txs.first()
            && let Some(index) = fixture.tx_index(tx)
            && !fixture.completed[index]
        {
            fixture.require_complete(index, &fb.txs, &g.tx_results, &mut rust)?;
            stats.p256_completed += 1;
        }
        if let Some(fixture) = pubkey_rejection_fixture.as_mut()
            && let Some(tx) = txs.first()
            && let Some(index) = fixture.tx_index(tx)
            && !fixture.completed[index]
        {
            fixture.require_complete(index, &fb.txs, &g.tx_results, &mut rust)?;
            stats.pubkey_rejection_completed += 1;
        }
        if let Some(fixture) = never_bonded_unjail_fixture.as_mut()
            && let Some(tx) = txs.first()
            && fixture.tx_matches_phase(tx)
        {
            fixture.require_phase(tx, &fb.txs, &g.tx_results, &rust)?;
            if fixture.phase == 4 {
                stats.never_bonded_unjail_completed += 1;
            }
        }
        if let Some(fixture) = unbonded_redelegation_rollback_fixture.as_mut()
            && let Some(tx) = txs.first()
            && fixture.pending_tx.as_deref() == Some(tx.as_slice())
            && fixture.require_phase(tx, &fb.txs, &g.tx_results, &mut rust)?
        {
            stats.unbonded_redelegation_rollback_completed += 1;
        }
        if let Some(fixture) = validator_removal_rewards_fixture.as_mut()
            && let Some(tx) = txs.first()
            && fixture.tx_matches_phase(tx)
        {
            fixture.require_phase(
                tx,
                &fb.txs,
                &g.tx_results,
                fb.time
                    .as_ref()
                    .context("validator-removal FinalizeBlock time is absent")?,
                &rust,
            )?;
            if fixture.phase == 4 {
                stats.validator_removal_rewards_completed += 1;
            }
        }
        if let Some(fixture) = upgrade_plan_fixture.as_mut()
            && fixture.after_commit(&mut go, cons_conn, &mut rust, app_version, &gn)?
        {
            if fixture.is_skip() {
                stats.upgrade_skip_completed += 1;
            } else if fixture.is_restart() {
                stats.upgrade_restart_completed += 1;
            } else {
                stats.upgrade_plan_completed += 1;
            }
        }
        if let Some(fixture) = gov_params_rejection_fixture.as_mut()
            && fixture.after_commit(&mut go, cons_conn, &mut rust, &gn)?
        {
            stats.gov_params_rejection_completed += 1;
            stats
                .gov_params_rejection_cases
                .clone_from(fixture.credits());
        }
        if let Some(fixture) = gov_handler_rejection_fixture.as_mut()
            && fixture.after_commit(&mut go, cons_conn, &mut rust, &gn)?
        {
            stats.gov_handler_rejection_completed += 1;
            stats
                .gov_handler_rejection_cases
                .clone_from(fixture.credits());
        }
        if let Some(fixture) = upgrade_plan_fixture.as_mut()
            && fixture.needs_post_commit_restart(height)
        {
            upgrade::switch_go(&mut go, 10, 10)?;
            let rust_home = std::path::PathBuf::from(
                std::env::var_os("DELIVER_UPGRADE_RUST_HOME")
                    .context("upgrade restart fixture needs DELIVER_UPGRADE_RUST_HOME")?,
            );
            rust.reopen(
                &rust_home,
                app::AppOptions {
                    node: app::NodeOptions {
                        home: Some(rust_home.clone()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )?;
            fixture.mark_post_commit_restart()?;
            fixture.require_post_commit_restart(&mut go, cons_conn, &mut rust)?;
        }
        if c.is_empty()
            && let (Some(requirement), Some(before), Some(evidence)) = (
                evidence_requirement.filter(|r| {
                    !matches!(
                        r,
                        EvidenceRequirement::StakeLifecycle
                            | EvidenceRequirement::ZeroDoubleSignSlash
                    )
                }),
                evidence_state_before.as_ref(),
                fb.misbehavior.first(),
            )
        {
            let after = evidence_branch_state(&rust, evidence)?;
            require_evidence_branch_unchanged(requirement, before, &after)?;
            *stats
                .evidence_injected
                .entry(requirement.name().into())
                .or_default() += 1;
        }
        if c.is_empty()
            && evidence_requirement == Some(EvidenceRequirement::ZeroDoubleSignSlash)
            && let (Some(before), Some(evidence)) =
                (zero_slash_state_before.as_ref(), fb.misbehavior.first())
        {
            let after = zero_slash_state(&rust, evidence, &time)?;
            require_zero_slash_postcondition(before, &after, evidence, &g.events)?;
            *stats
                .evidence_injected
                .entry(EvidenceRequirement::ZeroDoubleSignSlash.name().into())
                .or_default() += 1;
        }
        if evidence_requirement == Some(EvidenceRequirement::ZeroDoubleSignSlash) && b == 2 {
            let (tx, recipient, initial) = zero_slash_recovery
                .as_ref()
                .context("zero-slash recovery transaction is absent")?;
            let matches: Vec<_> = fb
                .txs
                .iter()
                .zip(&g.tx_results)
                .filter(|(candidate, _)| candidate.as_slice() == tx.as_slice())
                .collect();
            anyhow::ensure!(
                matches.len() == 1
                    && matches[0].1.code == 0
                    && matches[0].1.codespace.is_empty()
                    && bank_account_balance(
                        &mut rust.create_query_context(0, false)?.context(),
                        recipient,
                    )? == initial + 1,
                "zero-slash signed recovery transaction did not commit exactly once"
            );
        }
        if c.is_empty()
            && evidence_requirement == Some(EvidenceRequirement::StakeLifecycle)
            && let (Some(plan), Some(before), Some(evidence)) = (
                stake_lifecycle_plan.as_ref(),
                stake_lifecycle_state_before.as_ref(),
                fb.misbehavior.first(),
            )
        {
            let after = stake_lifecycle_state(&rust, plan, evidence, &time)?;
            require_stake_lifecycle_postcondition(before, &after, evidence)?;
            *stats
                .evidence_injected
                .entry(EvidenceRequirement::StakeLifecycle.name().into())
                .or_default() += 1;
        }
        for id in abstain_rejected_proposals {
            if proposal_is_all_abstain(&mut rust, id) == Some(true) {
                stats.gov_abstain_rejections += 1;
            }
        }
        // Go raised the app version: switch its binary after this
        // `Commit` (the multiplexer); the Rust app continues.
        if let Some(v) = g
            .consensus_param_updates
            .as_ref()
            .and_then(|c| c.version.as_ref())
            // A governance/cosmovisor switch changes the executable while
            // Version.App remains 9. Its migration response repeats that
            // consensus value; it must not switch the v10 process back.
            && !upgrade_restart_required
            && v.app != app_version
        {
            upgrade::switch_go(&mut go, app_version, v.app)?;
            stats.upgrade.push(format!(
                "h={height}: Go switched binaries, app version {app_version} -> {}",
                v.app
            ));
            app_version = v.app;
            gn.app_version = v.app;
            gn.types = types_for(app_version, upg.on);
            hl_seq.v10 = app_version >= 10;
            upg.v10_from = Some(height + 1);
        }
        // `TxPool.Update`: the block's txs leave the mempool, the rest are
        // rechecked against the new check state (`CheckTxType_Recheck`,
        // `handleRecheckResult` with `PostCheckMaxGas`), which right after
        // the switch is the next binary's fresh one.
        pool.retain(|t| !fb.txs.contains(t));
        let rphase = if upg.v10_from == Some(height + 1) {
            "switch".to_owned()
        } else if upg.on {
            phase.clone()
        } else {
            "all".to_owned()
        };
        // Every block with `DELIVER_RECHECK`, else only at the switch: the
        // recheck re-applies the pool's txs to the check state the
        // generator signs from, and the harness proposes new txs only.
        let do_recheck = recheck_all || rphase == "switch";
        let mut kept = std::collections::VecDeque::new();
        for t in std::mem::take(&mut pool) {
            if !do_recheck {
                kept.push_back(t);
                continue;
            }
            let req = abci::RequestCheckTx {
                tx: t.clone(),
                r#type: 1,
            };
            let g = go_value(go.call_or_exception(
                mem_conn,
                &Request {
                    value: Some(request::Value::CheckTx(req.clone())),
                },
            )?)?;
            let o = rust.check_tx(&req);
            let e = stats.recheck.entry(rphase.clone()).or_default();
            e.0 += 1;
            let response::Value::CheckTx(g) = g else {
                // Go's CheckTx returned an error (e.g. a tx the next
                // binary no longer decodes): Rust must error too, and
                // comet's `recheckTransactions` only logs it, so the tx
                // stays in the pool (Rust's `recheck_transactions` alike).
                if let Ok(o) = &o {
                    stats.mismatches.push(format!(
                        "{scenario} h={height} ReCheckTx tx={}: Go errored ({g:?}), Rust code {} log={}",
                        hex::encode(&t),
                        o.code,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ));
                }
                kept.push_back(t);
                continue;
            };
            match &o {
                Ok(o) if check_fields(o) == check_fields(&g) => {
                    if o.log != g.log {
                        stats.log_mismatches.push(format!(
                            "{scenario} h={height} ReCheckTx log tx={}\n    go:   {}\n    rust: {}",
                            hex::encode(&t),
                            gostr::to_go_bytes(&g.log).escape_ascii(),
                            gostr::to_go_bytes(&o.log).escape_ascii()
                        ));
                    }
                }
                _ => stats.mismatches.push(format!(
                    "{scenario} h={height} ReCheckTx ({rphase}) tx={}\n  go=(code {} {} gas {}/{} prio {}) log={}\n  rust={:?}",
                    hex::encode(&t),
                    g.code,
                    g.codespace,
                    g.gas_used,
                    g.gas_wanted,
                    g.priority,
                    gostr::to_go_bytes(&g.log).escape_ascii(),
                    o.as_ref().map(|o| format!(
                        "(code {} {} gas {}/{} prio {}) log={}",
                        o.code,
                        o.codespace,
                        o.gas_used,
                        o.gas_wanted,
                        o.priority,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ))
                )),
            }
            // `PostCheckMaxGas` (the scenarios' consensus params have no
            // gas limit, `-1`): a negative gas wanted fails it.
            if g.code == 0 && g.gas_wanted >= 0 {
                kept.push_back(t);
            } else {
                e.1 += 1;
            }
        }
        pool = kept;
        let near = upg.on && upg.upgrade_height.is_some_and(|h| (height - h).abs() <= 5);
        if upg.on {
            upg.observe(
                &mut gn,
                &mut rust,
                &fb.txs,
                &g.tx_results,
                &g.events,
                block_av,
                height,
            );
            upg.after_commit(&mut rust, height);
            let e = stats.phases.entry(phase).or_default();
            e.0 += 1;
            e.1 += fb.txs.len() as u64;
            e.2 += g.tx_results.iter().filter(|r| r.code == 0).count() as u64;
            e.3 += check_rejected.iter().filter(|r| **r).count() as u64;
            // The whole state at the last v9 height and the first v10
            // ones.
            if upg
                .upgrade_height
                .is_some_and(|h| (h..=h + 1).contains(&height))
            {
                for line in dump_split(&mut go, cons_conn, &mut rust) {
                    stats
                        .mismatches
                        .push(format!("{scenario} h={height} store {line}"));
                }
            }
        }
        if b % 25 == 24 || b + 1 == blocks || near || (upg.on && b % 5 == 4) {
            let extra = if upg.on {
                upgrade::queries(&gn, &hl_seq.token)
            } else {
                Vec::new()
            };
            let (c, l) = query_echo(
                &mut go,
                cons_conn,
                &mut rust,
                &gn,
                &mut stats.queries,
                extra,
                !near && b % 25 != 24 && b + 1 != blocks,
            );
            stats.mismatches.extend(
                c.into_iter()
                    .map(|c| format!("{scenario} h={height} query {c}")),
            );
            stats.log_mismatches.extend(
                l.into_iter()
                    .map(|l| format!("{scenario} h={height} query {l}")),
            );
        }
    }
    if let Some(plan) = transfer_authz_plan {
        stats.require_script_coverage()?;
        let sequence_start =
            transfer_authz_sequence_start.context("transfer authorization script did not start")?;
        let sequence_end = next_sequence_send(&mut rust, "transfer", "channel-0")
            .context("transfer authorization final send sequence")?;
        anyhow::ensure!(
            sequence_end == sequence_start + plan.successful_transfers,
            "transfer authorization committed packet count changed: start {sequence_start}, end {sequence_end}, expected {}",
            plan.successful_transfers
        );
        let response = query_app::<celestia_proto::cosmos::authz::v1beta1::QueryGrantsResponse>(
            &mut rust,
            "/cosmos.authz.v1beta1.Query/Grants",
            &celestia_proto::cosmos::authz::v1beta1::QueryGrantsRequest {
                granter: plan.granter,
                grantee: plan.grantee,
                msg_type_url: "/ibc.applications.transfer.v1.MsgTransfer".into(),
                pagination: None,
            },
        )
        .context("transfer authorization final grant query")?;
        anyhow::ensure!(
            response.grants == [plan.final_grant],
            "transfer authorization handler failure did not preserve the exact final grant"
        );
    }
    if let Some(mut d) = ibc {
        if ibc_pfm_metadata_required {
            let (query_mismatches, query_logs) = query_echo(
                &mut go,
                cons_conn,
                &mut rust,
                &gn,
                &mut stats.queries,
                pfm_metadata_queries(&d)?,
                true,
            );
            stats.mismatches.extend(
                query_mismatches
                    .into_iter()
                    .map(|mismatch| format!("PFM metadata committed-state query: {mismatch}")),
            );
            stats.log_mismatches.extend(
                query_logs
                    .into_iter()
                    .map(|mismatch| format!("PFM metadata committed-state query: {mismatch}")),
            );
            require_pfm_metadata_state(&mut rust, &d)?;
            *d.stats.entry("pfm_metadata_committed_state").or_default() += 1;
        }
        if ibc_upgrade_timeout_required
            || ibc_upgrade_timeout_boundary_required
            || ibc_upgrade_confirm_timeout_required
            || ibc_upgrade_try_fast_forward_required
            || ibc_upgrade_open_flushcomplete_required
        {
            require_upgrade_timeout_recovery_state(&mut go, cons_conn, &mut rust, &d)?;
            *d.stats.entry("upgrade_timeout_recovery_state").or_default() += 1;
        }
        stats.ibc = d.stats;
    }
    if let Some(fixture) = upgrade_plan_fixture.as_ref() {
        fixture.require_complete()?;
        stats.upgrade_plan_rejections = fixture.rejection_count();
    }
    if let Some(fixture) = never_bonded_unjail_fixture.as_ref() {
        fixture.require_complete()?;
    }
    if let Some(fixture) = unbonded_redelegation_rollback_fixture.as_ref() {
        fixture.require_complete()?;
    }
    if let Some(fixture) = gov_params_rejection_fixture.as_ref() {
        fixture.require_complete()?;
    }
    if let Some(fixture) = gov_handler_rejection_fixture.as_ref() {
        fixture.require_complete()?;
    }
    if let Some(fixture) = validator_removal_rewards_fixture.as_ref() {
        fixture.require_complete()?;
    }
    stats.upgrade.append(&mut upg.log);
    if let Some(h) = upg.upgrade_height {
        stats.upgrade.push(format!(
            "upgrade height {h}; app version 10 from {:?}; vesting accounts {}",
            upg.v10_from,
            upg.vesting.len()
        ));
    }
    for (k, n) in &upg.v9_proposals_ended {
        stats
            .upgrade
            .push(format!("proposals submitted at v9 that ended at {k}: {n}"));
    }
    stats.upgrade_built = upg.built;
    Ok(stats)
}

/// The operator addresses of the bonded validators (Rust's committed
/// state, the same as Go's).
fn bonded_validators(rust: &mut app::App) -> Vec<String> {
    use celestia_proto::cosmos::staking::v1beta1 as st;
    let r = rust.query(&abci::RequestQuery {
        data: st::QueryValidatorsRequest {
            status: "BOND_STATUS_BONDED".into(),
            pagination: None,
        }
        .encode_to_vec(),
        path: "/cosmos.staking.v1beta1.Query/Validators".into(),
        height: 0,
        prove: false,
    });
    st::QueryValidatorsResponse::decode(r.value.as_slice())
        .map(|v| {
            v.validators
                .into_iter()
                .map(|v| v.operator_address)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads one committed raw store key from both apps and requires byte-for-byte
/// equality. A missing key is returned as `None`.
fn query_raw_store_both(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    store: &str,
    key: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let query = abci::RequestQuery {
        data: key.as_bytes().to_vec(),
        path: format!("/store/{store}/key"),
        height: 0,
        prove: false,
    };
    let response = go.call_or_exception(
        conn,
        &Request {
            value: Some(request::Value::Query(query.clone())),
        },
    )?;
    let Some(response::Value::Query(go_response)) = response.value else {
        anyhow::bail!("raw Go query for {store}/{key} returned {response:?}");
    };
    let rust_response = rust.query(&query);
    anyhow::ensure!(
        (go_response.code, &go_response.codespace, &go_response.value)
            == (
                rust_response.code,
                &rust_response.codespace,
                &rust_response.value
            ),
        "raw store query differed for {store}/{key}: Go {}/{} {}, Rust {}/{} {}",
        go_response.codespace,
        go_response.code,
        hex::encode(&go_response.value),
        rust_response.codespace,
        rust_response.code,
        hex::encode(&rust_response.value)
    );
    anyhow::ensure!(
        go_response.code == 0,
        "raw store query failed for {store}/{key}: {}/{} {}",
        go_response.codespace,
        go_response.code,
        go_response.log
    );
    Ok((!go_response.value.is_empty()).then_some(go_response.value))
}

fn query_raw_store_bytes_both(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    store: &str,
    key: &[u8],
) -> anyhow::Result<Option<Vec<u8>>> {
    let query = abci::RequestQuery {
        data: key.to_vec(),
        path: format!("/store/{store}/key"),
        height: 0,
        prove: false,
    };
    let response = go.call_or_exception(
        conn,
        &Request {
            value: Some(request::Value::Query(query.clone())),
        },
    )?;
    let Some(response::Value::Query(go_response)) = response.value else {
        anyhow::bail!(
            "raw Go query for {store}/{} returned {response:?}",
            hex::encode(key)
        );
    };
    let rust_response = rust.query(&query);
    anyhow::ensure!(
        (go_response.code, &go_response.codespace, &go_response.value)
            == (
                rust_response.code,
                &rust_response.codespace,
                &rust_response.value
            ),
        "raw store query differed for {store}/{}: Go {}/{} {}, Rust {}/{} {}",
        hex::encode(key),
        go_response.codespace,
        go_response.code,
        hex::encode(&go_response.value),
        rust_response.codespace,
        rust_response.code,
        hex::encode(&rust_response.value)
    );
    anyhow::ensure!(
        go_response.code == 0,
        "raw store query failed for {store}/{}: {}/{} {}",
        hex::encode(key),
        go_response.codespace,
        go_response.code,
        go_response.log
    );
    Ok((!go_response.value.is_empty()).then_some(go_response.value))
}

#[allow(clippy::too_many_arguments)]
fn require_hl_recovery_id_block(
    block: &HlRecoveryIdBlock,
    txs: &[Vec<u8>],
    results: &[abci::ExecTxResult],
    app_version: u64,
    mailbox: &str,
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
) -> anyhow::Result<()> {
    anyhow::ensure!(block.cases.len() == 2, "recovery-id fixture case count");
    let expected_txs = [
        block.cases[0].tx.as_slice(),
        block.cases[1].tx.as_slice(),
        block.good.as_slice(),
    ];
    anyhow::ensure!(
        txs.iter().map(Vec::as_slice).eq(expected_txs),
        "recovery-id fixture transactions were missing, reordered or replaced"
    );
    anyhow::ensure!(
        results.len() == 3,
        "recovery-id fixture expected three finalized results, got {}",
        results.len()
    );
    for (index, case) in block.cases.iter().enumerate() {
        let check = case
            .check
            .as_ref()
            .context("recovery-id boundary transaction had no CheckTx response")?;
        let result = &results[index];
        let announcement_events = result
            .events
            .iter()
            .filter(|event| {
                event.r#type == "hyperlane.core.interchain_security.v1.EventAnnounceStorageLocation"
            })
            .count();
        if app_version < 10 {
            anyhow::ensure!(
                check.code == 0 && result.code == 0 && announcement_events == 1,
                "v9 recovery-id {} boundary outcome changed: CheckTx {}/{} {}, Finalize {}/{} {}, events={announcement_events}",
                case.wire_id,
                check.codespace,
                check.code,
                check.log,
                result.codespace,
                result.code,
                result.log
            );
        } else {
            anyhow::ensure!(
                check.code == 0
                    && result.codespace == "ism"
                    && result.code == 5
                    && result
                        .log
                        .contains("invalid signature recovery id: invalid signature")
                    && announcement_events == 0,
                "v10 recovery-id {} boundary outcome changed: CheckTx {}/{} {}, Finalize {}/{} {}, events={announcement_events}",
                case.wire_id,
                check.codespace,
                check.code,
                check.log,
                result.codespace,
                result.code,
                result.log
            );
        }
    }
    anyhow::ensure!(
        results[2].code == 0
            && results[2].events.iter().any(|event| {
                event.r#type == "hyperlane.core.interchain_security.v1.EventAnnounceStorageLocation"
            }),
        "canonical recovery transaction failed after boundary cases: {}/{} {}",
        results[2].codespace,
        results[2].code,
        results[2].log
    );

    let mailbox = x_hyperlane::util::HexAddress::decode(mailbox)
        .map_err(anyhow::Error::msg)?
        .internal_id();
    for case in &block.cases {
        let mut key = vec![1, 2];
        key.extend_from_slice(&mailbox.to_be_bytes());
        key.push(20);
        key.extend_from_slice(&case.validator);
        key.extend_from_slice(&0u64.to_be_bytes());
        let value = query_raw_store_bytes_both(go, conn, rust, "hyperlane", &key)?;
        if app_version < 10 {
            anyhow::ensure!(
                value.as_deref() == Some(case.location.as_bytes()),
                "v9 recovery-id {} announcement state missing or changed",
                case.wire_id
            );
        } else {
            anyhow::ensure!(
                value.is_none(),
                "v10 rejected recovery-id {} announcement mutated storage",
                case.wire_id
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UpgradeTimeoutCheckpoint {
    channel: Vec<u8>,
    next_sequence_send: Vec<u8>,
    upgrade: Option<Vec<u8>>,
    counterparty_upgrade: Option<Vec<u8>>,
    error_receipt: Option<Vec<u8>>,
    client_prefix: BTreeMap<Vec<u8>, Vec<u8>>,
}

fn ensure_upgrade_timeout_checkpoint_unchanged(
    expected: &UpgradeTimeoutCheckpoint,
    observed: &UpgradeTimeoutCheckpoint,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected == observed,
        "rejected upgrade timeout mutated the raw channel, send sequence, upgrade, counterparty upgrade, error receipt, or client checkpoint"
    );
    Ok(())
}

fn raw_pairs(value: &[u8]) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut pairs = BTreeMap::new();
    for pair in parse_fields(value).unwrap_or_default() {
        if pair.num != 1 || pair.wt != 2 {
            continue;
        }
        let (mut key, mut value) = (Vec::new(), Vec::new());
        for field in parse_fields(&pair.val).unwrap_or_default() {
            match field.num {
                1 => key = field.val,
                2 => value = field.val,
                _ => {}
            }
        }
        pairs.insert(key, value);
    }
    pairs
}

fn query_raw_prefix_both(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    store: &str,
    prefix: &[u8],
) -> anyhow::Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    anyhow::ensure!(!prefix.is_empty(), "raw subspace prefix is empty");
    let query = abci::RequestQuery {
        data: prefix.to_vec(),
        path: format!("/store/{store}/subspace"),
        height: 0,
        prove: false,
    };
    let response = go.call_or_exception(
        conn,
        &Request {
            value: Some(request::Value::Query(query.clone())),
        },
    )?;
    let Some(response::Value::Query(go_response)) = response.value else {
        anyhow::bail!("raw Go prefix query returned {response:?}");
    };
    let rust_response = rust.query(&query);
    let go_pairs = raw_pairs(&go_response.value);
    let rust_pairs = raw_pairs(&rust_response.value);
    anyhow::ensure!(
        go_response.code == 0 && rust_response.code == 0 && go_pairs == rust_pairs,
        "raw prefix query differed for {store}/{}: Go {}/{}, Rust {}/{}",
        hex::encode(prefix),
        go_response.codespace,
        go_response.code,
        rust_response.codespace,
        rust_response.code
    );
    Ok(go_pairs)
}

fn capture_upgrade_timeout_checkpoint(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    driver: &crate::ibcmock::Driver,
) -> anyhow::Result<UpgradeTimeoutCheckpoint> {
    let channel = driver
        .upgrade_timeout_channel()
        .context("upgrade-timeout checkpoint has no channel")?;
    let client = driver
        .upgrade_timeout_client()
        .context("upgrade-timeout checkpoint has no client")?;
    let path = |kind: &str| format!("channelUpgrades/{kind}/ports/icahost/channels/{channel}");
    Ok(UpgradeTimeoutCheckpoint {
        channel: query_raw_store_both(
            go,
            conn,
            rust,
            "ibc",
            &format!("channelEnds/ports/icahost/channels/{channel}"),
        )?
        .context("upgrade-timeout checkpoint channel is absent")?,
        next_sequence_send: query_raw_store_both(
            go,
            conn,
            rust,
            "ibc",
            &format!("nextSequenceSend/ports/icahost/channels/{channel}"),
        )?
        .context("upgrade-timeout checkpoint next send sequence is absent")?,
        upgrade: query_raw_store_both(go, conn, rust, "ibc", &path("upgrades"))?,
        counterparty_upgrade: query_raw_store_both(
            go,
            conn,
            rust,
            "ibc",
            &path("counterpartyUpgrade"),
        )?,
        error_receipt: query_raw_store_both(go, conn, rust, "ibc", &path("upgradeError"))?,
        client_prefix: query_raw_prefix_both(
            go,
            conn,
            rust,
            "ibc",
            format!("clients/{client}/").as_bytes(),
        )?,
    })
}

fn require_upgrade_timeout_recovery_state(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    driver: &crate::ibcmock::Driver,
) -> anyhow::Result<()> {
    let (packet, acknowledgement) =
        driver
            .upgrade_timeout_recovery_evidence()
            .with_context(|| {
                format!(
                "upgrade-timeout recovery has no exact credited packet; lifecycle stats: {:?}; diagnostic: {}",
                driver.stats,
                driver.upgrade_timeout_diagnostic().unwrap_or("none")
                )
            })?;
    let receipt_key = format!(
        "receipts/ports/{}/channels/{}/sequences/{}",
        packet.destination_port, packet.destination_channel, packet.sequence
    );
    let acknowledgement_key = format!(
        "acks/ports/{}/channels/{}/sequences/{}",
        packet.destination_port, packet.destination_channel, packet.sequence
    );
    anyhow::ensure!(
        query_raw_store_both(go, conn, rust, "ibc", &receipt_key)? == Some(vec![1]),
        "upgrade-timeout recovery did not commit its exact packet receipt"
    );
    let expected = x_ibc::channel::commit_acknowledgement(acknowledgement);
    anyhow::ensure!(
        query_raw_store_both(go, conn, rust, "ibc", &acknowledgement_key)? == Some(expected),
        "upgrade-timeout recovery did not commit its exact acknowledgement"
    );
    Ok(())
}

/// A gRPC query of the Rust app (`None` if it fails).
fn query_app<R: prost::Message + Default>(
    rust: &mut app::App,
    path: &str,
    req: &impl prost::Message,
) -> Option<R> {
    let r = rust.query(&abci::RequestQuery {
        data: req.encode_to_vec(),
        path: path.into(),
        height: 0,
        prove: false,
    });
    (r.code == 0)
        .then(|| R::decode(r.value.as_slice()).ok())
        .flatten()
}

fn require_pfm_metadata_state(
    rust: &mut app::App,
    driver: &crate::ibcmock::Driver,
) -> anyhow::Result<()> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::ibc::applications::transfer::v1::DenomTrace;
    use celestia_proto::ibc::core::channel::v1 as channel;

    let packets = driver.pfm_metadata_packets();
    anyhow::ensure!(
        packets.len() == 3,
        "PFM metadata requirement credited {} exact packets instead of three; {}",
        packets.len(),
        driver
            .pfm_metadata_diagnostic()
            .unwrap_or("no matching acknowledgement event observed")
    );
    for (index, (packet, acknowledgement, credit)) in packets.iter().enumerate() {
        let ack = query_app::<channel::QueryPacketAcknowledgementResponse>(
            rust,
            "/ibc.core.channel.v1.Query/PacketAcknowledgement",
            &channel::QueryPacketAcknowledgementRequest {
                port_id: packet.destination_port.clone(),
                channel_id: packet.destination_channel.clone(),
                sequence: packet.sequence,
            },
        )
        .with_context(|| format!("querying committed acknowledgement for {credit}"))?;
        let expected_commitment = x_ibc::channel::commit_acknowledgement(acknowledgement);
        anyhow::ensure!(
            ack.acknowledgement == expected_commitment,
            "wrong committed acknowledgement for {credit}: got {}, expected {}",
            hex::encode(ack.acknowledgement),
            hex::encode(expected_commitment)
        );
        let receipt = query_app::<channel::QueryPacketReceiptResponse>(
            rust,
            "/ibc.core.channel.v1.Query/PacketReceipt",
            &channel::QueryPacketReceiptRequest {
                port_id: packet.destination_port.clone(),
                channel_id: packet.destination_channel.clone(),
                sequence: packet.sequence,
            },
        )
        .with_context(|| format!("querying committed receipt for {credit}"))?;
        anyhow::ensure!(
            receipt.received,
            "missing committed packet receipt for {credit}"
        );

        let data: serde_json::Value = serde_json::from_slice(&packet.data)
            .with_context(|| format!("decoding exact packet data for {credit}"))?;
        let receiver = data["receiver"]
            .as_str()
            .with_context(|| format!("exact packet for {credit} has no receiver"))?;
        let base_denom = data["denom"]
            .as_str()
            .with_context(|| format!("exact packet for {credit} has no denom"))?;
        let denom = x_transfer::ibc_denom(&DenomTrace {
            path: format!("{}/{}", packet.destination_port, packet.destination_channel),
            base_denom: base_denom.into(),
        });
        let balance = query_app::<bank::QueryBalanceResponse>(
            rust,
            "/cosmos.bank.v1beta1.Query/Balance",
            &bank::QueryBalanceRequest {
                address: receiver.into(),
                denom,
            },
        )
        .with_context(|| format!("querying exact recipient balance for {credit}"))?
        .balance;
        let balance = match balance {
            Some(coin) => coin
                .amount
                .parse::<u128>()
                .with_context(|| format!("parsing exact recipient balance for {credit}"))?,
            None => 0,
        };
        let expected = if index == 2 {
            data["amount"]
                .as_str()
                .with_context(|| format!("exact packet for {credit} has no amount"))?
                .parse::<u128>()
                .with_context(|| format!("parsing exact packet amount for {credit}"))?
        } else {
            0
        };
        anyhow::ensure!(
            balance == expected,
            "wrong recipient balance for {credit}: got {balance}, expected {expected}"
        );
    }
    Ok(())
}

fn pfm_metadata_queries(
    driver: &crate::ibcmock::Driver,
) -> anyhow::Result<Vec<(&'static str, Vec<u8>)>> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::ibc::applications::transfer::v1::DenomTrace;
    use celestia_proto::ibc::core::channel::v1 as channel;

    let mut queries = Vec::new();
    for (packet, _, credit) in driver.pfm_metadata_packets() {
        queries.push((
            "/ibc.core.channel.v1.Query/PacketAcknowledgement",
            channel::QueryPacketAcknowledgementRequest {
                port_id: packet.destination_port.clone(),
                channel_id: packet.destination_channel.clone(),
                sequence: packet.sequence,
            }
            .encode_to_vec(),
        ));
        queries.push((
            "/ibc.core.channel.v1.Query/PacketReceipt",
            channel::QueryPacketReceiptRequest {
                port_id: packet.destination_port.clone(),
                channel_id: packet.destination_channel.clone(),
                sequence: packet.sequence,
            }
            .encode_to_vec(),
        ));
        let data: serde_json::Value = serde_json::from_slice(&packet.data)
            .with_context(|| format!("decoding exact packet data for {credit}"))?;
        let receiver = data["receiver"]
            .as_str()
            .with_context(|| format!("exact packet for {credit} has no receiver"))?;
        let base_denom = data["denom"]
            .as_str()
            .with_context(|| format!("exact packet for {credit} has no denom"))?;
        queries.push((
            "/cosmos.bank.v1beta1.Query/Balance",
            bank::QueryBalanceRequest {
                address: receiver.into(),
                denom: x_transfer::ibc_denom(&DenomTrace {
                    path: format!("{}/{}", packet.destination_port, packet.destination_channel),
                    base_denom: base_denom.into(),
                }),
            }
            .encode_to_vec(),
        ));
    }
    Ok(queries)
}

fn next_sequence_send(rust: &mut app::App, port_id: &str, channel_id: &str) -> Option<u64> {
    use celestia_proto::ibc::core::channel::v1 as channel;
    query_app::<channel::QueryNextSequenceSendResponse>(
        rust,
        "/ibc.core.channel.v1.Query/NextSequenceSend",
        &channel::QueryNextSequenceSendRequest {
            port_id: port_id.into(),
            channel_id: channel_id.into(),
        },
    )
    .map(|response| response.next_sequence_send)
}

/// Whether a committed proposal's final tally contains only abstain power.
fn proposal_is_all_abstain(rust: &mut app::App, id: u64) -> Option<bool> {
    use celestia_proto::cosmos::gov::v1 as gov;
    let tally = query_app::<gov::QueryProposalResponse>(
        rust,
        "/cosmos.gov.v1.Query/Proposal",
        &gov::QueryProposalRequest { proposal_id: id },
    )?
    .proposal?
    .final_tally_result?;
    Some(tally_is_all_abstain(&tally))
}

fn tally_is_all_abstain(tally: &celestia_proto::cosmos::gov::v1::TallyResult) -> bool {
    canonical_sdk_uint(&tally.abstain_count) == Some(true)
        && canonical_sdk_uint(&tally.yes_count) == Some(false)
        && canonical_sdk_uint(&tally.no_count) == Some(false)
        && canonical_sdk_uint(&tally.no_with_veto_count) == Some(false)
}

/// Whether a string is a canonical SDK nonnegative integer, and is nonzero.
fn canonical_sdk_uint(value: &str) -> Option<bool> {
    if value == "0" {
        return Some(false);
    }
    (!value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some(true)
}

/// Use the current proposal minimum instead of a fixed denom/amount. The final
/// Go-success requirement still catches parameter states that reject deposits.
fn legacy_deposit_amount(rust: &mut app::App, id: u64) -> Option<Vec<Coin>> {
    use celestia_proto::cosmos::gov::v1 as gov;
    let params = query_app::<gov::QueryParamsResponse>(
        rust,
        "/cosmos.gov.v1.Query/Params",
        &gov::QueryParamsRequest {
            params_type: "deposit".into(),
        },
    )?
    .params?;
    let proposal = query_app::<gov::QueryProposalResponse>(
        rust,
        "/cosmos.gov.v1.Query/Proposal",
        &gov::QueryProposalRequest { proposal_id: id },
    )?
    .proposal?;
    let amount = if proposal.expedited {
        params.expedited_min_deposit
    } else {
        params.min_deposit
    };
    (!amount.is_empty()).then_some(amount)
}

/// A proposal message whose failure text is deterministic in Go: gov
/// `Params.ValidateBasic` prints a non-positive `max_deposit_period` with
/// `%d` of its pointer (an address, different on every Go node;
/// `docs/security.md` Open), which the proposal stores as `failed_reason`.
/// Such a period becomes 1 s; any other message is kept as is.
fn go_deterministic(msg: Any) -> Any {
    use celestia_proto::cosmos::gov::v1 as gov;
    use prost::Name as _;
    if msg.type_url != gov::MsgUpdateParams::type_url() {
        return msg;
    }
    let Ok(mut m) = gov::MsgUpdateParams::decode(msg.value.as_slice()) else {
        return msg;
    };
    let Some(p) = m.params.as_mut() else {
        return msg;
    };
    match &p.max_deposit_period {
        Some(d) if sdk::time::duration_nanos(d) <= 0 => {
            p.max_deposit_period = Some(pbjson_types::Duration {
                seconds: 1,
                nanos: 0,
            });
            Any::pack(&m)
        }
        _ => msg,
    }
}

/// `DELIVER_GOVPASS`: gov-authority messages that set each module's
/// parameters to their current values (handlers and celestia's parameter
/// filter run, the state stays usable), and a community pool spend of 1
/// utia to `recipient`.
fn authority_msgs(rust: &mut app::App, recipient: &str) -> Vec<Any> {
    use celestia_proto::celestia::{blob::v1 as blob, minfee::v1 as minfee};
    use celestia_proto::cosmos::{
        auth::v1beta1 as auth, bank::v1beta1 as bank, circuit::v1 as circuit,
        consensus::v1 as cons, distribution::v1beta1 as distr, gov::v1 as gov,
        slashing::v1beta1 as slashing, staking::v1beta1 as staking,
    };
    use celestia_proto::ibc::applications::{
        interchain_accounts::host::v1 as icahost, transfer::v1 as transfer,
    };
    use celestia_proto::ibc::core::{
        channel::v1 as channel, client::v1 as client, connection::v1 as connection,
    };
    let mut out = Vec::new();
    let a = || GOV.to_owned();
    macro_rules! params {
        ($m:ident, $path:expr, $msg:ident { $who:ident }) => {
            if let Some(r) =
                query_app::<$m::QueryParamsResponse>(rust, $path, &$m::QueryParamsRequest {})
            {
                out.push(Any::pack(&$m::$msg {
                    $who: a(),
                    params: r.params,
                }));
            }
        };
    }
    params!(
        auth,
        "/cosmos.auth.v1beta1.Query/Params",
        MsgUpdateParams { authority }
    );
    params!(
        bank,
        "/cosmos.bank.v1beta1.Query/Params",
        MsgUpdateParams { authority }
    );
    // Exercise the separate gov-authority handler with a valid setting.
    // Enabling the native fee denom preserves the scenario's bank behavior.
    out.push(Any::pack(&bank::MsgSetSendEnabled {
        authority: a(),
        send_enabled: vec![bank::SendEnabled {
            denom: "utia".into(),
            enabled: true,
        }],
        use_default_for: Vec::new(),
    }));
    // Grant a fuzz account a permission for a message URL no scenario
    // sends. This reaches the circuit authorization handler without
    // allowing the grantee to interrupt subsequent generated traffic.
    out.push(Any::pack(&circuit::MsgAuthorizeCircuitBreaker {
        granter: a(),
        grantee: recipient.to_owned(),
        permissions: Some(circuit::Permissions {
            level: circuit::permissions::Level::SomeMsgs as i32,
            limit_type_urls: vec!["/coverage.unused.Msg".into()],
        }),
    }));
    if let Some(r) = query_app::<distr::QueryParamsResponse>(
        rust,
        "/cosmos.distribution.v1beta1.Query/Params",
        &distr::QueryParamsRequest {},
    ) && let Some(mut p) = r.params
    {
        out.push(Any::pack(&distr::MsgUpdateParams {
            authority: a(),
            params: Some(p.clone()),
        }));
        p.community_tax.clear();
        out.push(Any::pack(&distr::MsgUpdateParams {
            authority: a(),
            params: Some(p),
        }));
    }
    if let Some(r) = query_app::<slashing::QueryParamsResponse>(
        rust,
        "/cosmos.slashing.v1beta1.Query/Params",
        &slashing::QueryParamsRequest {},
    ) && let Some(mut p) = r.params
    {
        out.push(Any::pack(&slashing::MsgUpdateParams {
            authority: a(),
            params: Some(p.clone()),
        }));
        p.min_signed_per_window.clear();
        out.push(Any::pack(&slashing::MsgUpdateParams {
            authority: a(),
            params: Some(p),
        }));
    }
    if let Some(r) = query_app::<staking::QueryParamsResponse>(
        rust,
        "/cosmos.staking.v1beta1.Query/Params",
        &staking::QueryParamsRequest {},
    ) && let Some(mut p) = r.params
    {
        out.push(Any::pack(&staking::MsgUpdateParams {
            authority: a(),
            params: Some(p.clone()),
        }));
        p.min_commission_rate.clear();
        out.push(Any::pack(&staking::MsgUpdateParams {
            authority: a(),
            params: Some(p),
        }));
    }
    params!(
        minfee,
        "/celestia.minfee.v1.Query/Params",
        MsgUpdateMinfeeParams { authority }
    );
    params!(
        blob,
        "/celestia.blob.v1.Query/Params",
        MsgUpdateBlobParams { authority }
    );
    params!(
        transfer,
        "/ibc.applications.transfer.v1.Query/Params",
        MsgUpdateParams { signer }
    );
    params!(
        icahost,
        "/ibc.applications.interchain_accounts.host.v1.Query/Params",
        MsgUpdateParams { signer }
    );
    if let Some(r) = query_app::<gov::QueryParamsResponse>(
        rust,
        "/cosmos.gov.v1.Query/Params",
        &gov::QueryParamsRequest {
            params_type: "tallying".into(),
        },
    ) {
        out.push(Any::pack(&gov::MsgUpdateParams {
            authority: a(),
            params: r.params,
        }));
    }
    if let Some(r) = query_app::<cons::QueryParamsResponse>(
        rust,
        "/cosmos.consensus.v1.Query/Params",
        &cons::QueryParamsRequest {},
    ) {
        let p = r.params.unwrap_or_default();
        out.push(Any::pack(&cons::MsgUpdateParams {
            authority: a(),
            block: p.block,
            evidence: p.evidence,
            validator: p.validator,
            abci: p.abci,
        }));
    }
    if let Some(r) = query_app::<client::QueryClientParamsResponse>(
        rust,
        "/ibc.core.client.v1.Query/ClientParams",
        &client::QueryClientParamsRequest {},
    ) {
        out.push(Any::pack(&client::MsgUpdateParams {
            signer: a(),
            params: r.params,
        }));
    }
    if let Some(r) = query_app::<connection::QueryConnectionParamsResponse>(
        rust,
        "/ibc.core.connection.v1.Query/ConnectionParams",
        &connection::QueryConnectionParamsRequest {},
    ) {
        out.push(Any::pack(&connection::MsgUpdateParams {
            signer: a(),
            params: r.params,
        }));
    }
    if let Some(r) = query_app::<channel::QueryChannelParamsResponse>(
        rust,
        "/ibc.core.channel.v1.Query/ChannelParams",
        &channel::QueryChannelParamsRequest {},
    ) {
        out.push(Any::pack(&channel::MsgUpdateParams {
            authority: a(),
            params: r.params,
        }));
    }
    out.push(Any::pack(&distr::MsgCommunityPoolSpend {
        authority: a(),
        recipient: recipient.to_owned(),
        amount: vec![Coin {
            denom: "utia".into(),
            amount: "1".into(),
        }],
    }));
    out
}

/// `Service/Simulate` of `tx` on both apps (`DELIVER_SIM`), signed and with
/// its signatures emptied (`BuildSimTx`, what `--gas auto` simulates):
/// the gas estimate (`gas_info`) and the code must agree; the rest of the
/// response (events, data) and the error text are compared as logs.
fn simulate_both(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    app_version: u64,
    tx: &[u8],
    stats: &mut Stats,
) {
    use celestia_proto::cosmos::tx::v1beta1::{SimulateRequest, SimulateResponse, TxRaw};
    let first = type_urls(app_version, tx)
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut variants = vec![("signed", tx.to_vec())];
    if let Ok(mut raw) = TxRaw::decode(tx) {
        for s in &mut raw.signatures {
            s.clear();
        }
        variants.push(("unsigned", raw.encode_to_vec()));
    }
    for (kind, bytes) in variants {
        let req = abci::RequestQuery {
            data: SimulateRequest {
                tx_bytes: bytes.clone(),
                ..Default::default()
            }
            .encode_to_vec(),
            path: "/cosmos.tx.v1beta1.Service/Simulate".into(),
            height: 0,
            prove: false,
        };
        let g = match go.call_or_exception(
            conn,
            &Request {
                value: Some(request::Value::Query(req.clone())),
            },
        ) {
            Ok(Response {
                value: Some(response::Value::Query(g)),
            }) => g,
            other => {
                stats
                    .mismatches
                    .push(format!("Simulate {kind}: Go query failed: {other:?}"));
                continue;
            }
        };
        let r = rust.query(&req);
        let e = stats.sim.entry(first.clone()).or_default();
        e.0 += 1;
        if g.code == 0 {
            e.1 += 1;
        }
        let gas = |v: &[u8]| {
            SimulateResponse::decode(v)
                .ok()
                .and_then(|s| s.gas_info)
                .map(|gi| (gi.gas_wanted, gi.gas_used))
        };
        let what = || format!("Simulate {kind} {first} tx={}", hex::encode(&bytes));
        if (g.code, &g.codespace) != (r.code, &r.codespace) || gas(&g.value) != gas(&r.value) {
            e.2 += 1;
            stats.mismatches.push(format!(
                "{} go={}/{} gas={:?} log={:?}\n    rust={}/{} gas={:?} log={:?}",
                what(),
                g.codespace,
                g.code,
                gas(&g.value),
                g.log,
                r.codespace,
                r.code,
                gas(&r.value),
                r.log
            ));
        } else if g.value != r.value {
            stats.log_mismatches.push(format!(
                "{} response go={} rust={}",
                what(),
                hex::encode(&g.value),
                hex::encode(&r.value)
            ));
        } else if g.log != r.log {
            stats.log_mismatches.push(format!(
                "{} go log={:?} rust log={:?}",
                what(),
                g.log,
                r.log
            ));
        }
    }
}

/// The queries that return stored messages (`Any`s, customtypes,
/// `stdtime` / `stdduration` fields as the handlers stored them, or as the
/// query re-packs them: gov `v1beta1` proposals convert messages to legacy
/// content) sent to both apps on the committed state: (code / value
/// differences, log differences).
fn query_echo(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    gn: &Gen,
    counts: &mut BTreeMap<String, (u64, u64)>,
    extra: Vec<(&'static str, Vec<u8>)>,
    only_extra: bool,
) -> (Vec<String>, Vec<String>) {
    // `{1: s}` / `{1: n}` requests.
    let s1 = |s: &str| {
        encode(&[Field {
            num: 1,
            wt: 2,
            val: s.as_bytes().to_vec(),
        }])
    };
    let n1 = |n: u64| {
        let mut v = Vec::new();
        put_varint(&mut v, n);
        encode(&[Field {
            num: 1,
            wt: 0,
            val: v,
        }])
    };
    let mut reqs: Vec<(&str, Vec<u8>)> = [
        "/cosmos.gov.v1.Query/Proposals",
        "/cosmos.gov.v1beta1.Query/Proposals",
        "/cosmos.gov.v1.Query/Params",
        "/cosmos.staking.v1beta1.Query/Validators",
        "/cosmos.staking.v1beta1.Query/Params",
        "/cosmos.auth.v1beta1.Query/Accounts",
        "/cosmos.auth.v1beta1.Query/Params",
        "/cosmos.bank.v1beta1.Query/Params",
        "/cosmos.bank.v1beta1.Query/DenomsMetadata",
        "/cosmos.distribution.v1beta1.Query/Params",
        "/cosmos.slashing.v1beta1.Query/Params",
        "/cosmos.consensus.v1.Query/Params",
        "/cosmos.upgrade.v1beta1.Query/CurrentPlan",
        "/ibc.core.client.v1.Query/ClientStates",
        "/ibc.core.client.v1.Query/ClientParams",
        "/ibc.core.connection.v1.Query/Connections",
        "/ibc.core.channel.v1.Query/Channels",
        "/hyperlane.core.v1.Query/Mailboxes",
        "/hyperlane.core.interchain_security.v1.Query/Isms",
        "/hyperlane.core.post_dispatch.v1.Query/Igps",
        "/hyperlane.core.post_dispatch.v1.Query/MerkleTreeHooks",
        "/hyperlane.core.post_dispatch.v1.Query/NoopHooks",
        "/hyperlane.warp.v1.Query/Tokens",
    ]
    .into_iter()
    .map(|p| (p, Vec::new()))
    .collect();
    if only_extra {
        reqs.clear();
    }
    reqs.extend(extra);
    for id in gn.proposals.iter().rev().take(8) {
        for p in [
            "/cosmos.gov.v1.Query/Proposal",
            "/cosmos.gov.v1beta1.Query/Proposal",
            "/cosmos.gov.v1.Query/Votes",
            "/cosmos.gov.v1beta1.Query/Votes",
            "/cosmos.gov.v1.Query/Deposits",
        ] {
            reqs.push((p, n1(*id)));
        }
    }
    for a in &gn.accts {
        for p in [
            "/cosmos.authz.v1beta1.Query/GranterGrants",
            "/cosmos.authz.v1beta1.Query/GranteeGrants",
            "/cosmos.feegrant.v1beta1.Query/Allowances",
            "/cosmos.feegrant.v1beta1.Query/AllowancesByGranter",
            "/cosmos.auth.v1beta1.Query/Account",
        ] {
            reqs.push((p, s1(&a.acc)));
        }
    }
    let (mut c, mut l) = (Vec::new(), Vec::new());
    for (path, data) in reqs {
        let req = abci::RequestQuery {
            data,
            path: path.into(),
            height: 0,
            prove: false,
        };
        let g = match go.call_or_exception(
            conn,
            &Request {
                value: Some(request::Value::Query(req.clone())),
            },
        ) {
            Ok(Response {
                value: Some(response::Value::Query(g)),
            }) => g,
            other => {
                c.push(format!("{path}: Go query failed: {other:?}"));
                continue;
            }
        };
        let r = rust.query(&req);
        let e = counts.entry(path.to_owned()).or_default();
        e.0 += 1;
        if g.code == 0 && !g.value.is_empty() {
            e.1 += 1;
        }
        let what = || format!("{path} {}", hex::encode(&req.data));
        if (g.code, &g.codespace, &g.value) != (r.code, &r.codespace, &r.value) {
            c.push(format!(
                "{} go={}/{} {}\n    rust={}/{} {}\n    go log={:?} rust log={:?}",
                what(),
                g.codespace,
                g.code,
                hex::encode(&g.value),
                r.codespace,
                r.code,
                hex::encode(&r.value),
                g.log,
                r.log
            ));
        } else if g.log != r.log {
            l.push(format!(
                "{} go log={:?} rust log={:?}",
                what(),
                g.log,
                r.log
            ));
        }
    }
    (c, l)
}

/// After an app-hash split (both committed): the entries of every module
/// store that differ between Go and Rust (`/store/<name>/subspace`).
fn dump_split(go: &mut GoApp, conn: u32, rust: &mut app::App) -> Vec<String> {
    const STORES: &[&str] = &[
        "acc",
        "authz",
        "bank",
        "blob",
        "capability",
        "circuit",
        "consensus",
        "distribution",
        "evidence",
        "feegrant",
        "fibre",
        "gov",
        "hyperlane",
        "ibc",
        "icahost",
        "minfee",
        "mint",
        "packetfowardmiddleware",
        "params",
        "signal",
        "slashing",
        "staking",
        "transfer",
        "upgrade",
        "valaddr",
        "warp",
        "zkism",
    ];
    fn pairs(v: &[u8]) -> BTreeMap<Vec<u8>, Vec<u8>> {
        // kv.Pairs { repeated Pair pairs = 1 } / Pair { key = 1, value = 2 }.
        let mut out = BTreeMap::new();
        for p in parse_fields(v).unwrap_or_default() {
            if p.num != 1 || p.wt != 2 {
                continue;
            }
            let (mut k, mut val) = (Vec::new(), Vec::new());
            for f in parse_fields(&p.val).unwrap_or_default() {
                match f.num {
                    1 => k = f.val,
                    2 => val = f.val,
                    _ => {}
                }
            }
            out.insert(k, val);
        }
        out
    }
    let mut lines = Vec::new();
    for s in STORES {
        // A subspace query needs a non-empty prefix: one per first byte.
        let (mut gp, mut rp) = (BTreeMap::new(), BTreeMap::new());
        for first in 0..=255u8 {
            let req = abci::RequestQuery {
                data: vec![first],
                path: format!("/store/{s}/subspace"),
                height: 0,
                prove: false,
            };
            let g = go.call_or_exception(
                conn,
                &Request {
                    value: Some(request::Value::Query(req.clone())),
                },
            );
            let g = match g {
                Ok(Response {
                    value: Some(response::Value::Query(g)),
                }) => g,
                other => {
                    lines.push(format!("{s}: Go query failed: {other:?}"));
                    break;
                }
            };
            let r = rust.query(&req);
            gp.extend(pairs(&g.value));
            rp.extend(pairs(&r.value));
        }
        for k in gp.keys().chain(rp.keys()).collect::<BTreeSet<_>>() {
            let (a, b) = (gp.get(k), rp.get(k));
            if a != b {
                lines.push(format!(
                    "{s} key={}\n    go={}\n    rust={}",
                    hex::encode(k),
                    a.map_or_else(|| "<absent>".into(), hex::encode),
                    b.map_or_else(|| "<absent>".into(), hex::encode)
                ));
            }
        }
    }
    lines
}

/// The CometBFT address of a validator update's key.
fn update_address(u: &abci::ValidatorUpdate) -> Option<Vec<u8>> {
    use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;
    match u.pub_key.as_ref()?.sum.as_ref()? {
        Sum::Ed25519(k) => Some(hash::sha256(k)[..20].to_vec()),
        Sum::Secp256k1(k) => {
            Some(crypto::secp256k1::address(&k.as_slice().try_into().ok()?).to_vec())
        }
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// A validator set: (address, power, joined during the run).
type ValSet = Vec<(Vec<u8>, i64, bool)>;

#[derive(Debug, PartialEq)]
struct EvidenceBranchState {
    validator_status: Option<x_staking::types::BondStatus>,
    validator_jailed: Option<bool>,
    pubkey_present: bool,
    signing_info_present: bool,
    tombstoned: Option<bool>,
    jailed_until: Option<pbjson_types::Timestamp>,
    evidence_present: bool,
}

#[derive(Debug)]
struct ZeroSlashState {
    evidence: EvidenceBranchState,
    operator: Vec<u8>,
    tokens: Int,
    shares: LegacyDec,
    power_index_present: bool,
    slash_factor: LegacyDec,
    supply: Int,
    block_provision: Int,
    bonded_pool: Int,
    not_bonded_pool: Int,
    current_period: u64,
    retained_outstanding: sdk::dec_coins::DecCoins,
    historical_keys: usize,
    slash_event_keys: usize,
}

fn evidence_branch_state(
    rust: &app::App,
    misbehavior: &abci::Misbehavior,
) -> anyhow::Result<EvidenceBranchState> {
    let (equivocation, consensus_address) = x_evidence::from_abci(misbehavior);
    let mut state = rust.create_query_context(0, false)?;
    let mut ctx = state.context();
    let validator = x_staking::store::get_validator_by_cons_addr(&mut ctx, &consensus_address)?;
    let signing_info = x_slashing::get_signing_info(&mut ctx, &consensus_address)?;
    let mut evidence_key = vec![0x00];
    evidence_key.extend_from_slice(&x_evidence::hash(&equivocation));
    Ok(EvidenceBranchState {
        validator_status: validator.as_ref().map(|v| v.status),
        validator_jailed: validator.as_ref().map(|v| v.jailed),
        pubkey_present: ctx
            .get(
                x_slashing::STORE_KEY,
                &x_slashing::addr_pubkey_key(&consensus_address),
            )?
            .is_some(),
        signing_info_present: signing_info.is_some(),
        tombstoned: signing_info.as_ref().map(|info| info.tombstoned),
        jailed_until: signing_info.map(|info| info.jailed_until),
        evidence_present: ctx.get(x_evidence::STORE_KEY, &evidence_key)?.is_some(),
    })
}

fn zero_slash_state(
    rust: &app::App,
    misbehavior: &abci::Misbehavior,
    block_time: &pbjson_types::Timestamp,
) -> anyhow::Result<ZeroSlashState> {
    let (_, consensus_address) = x_evidence::from_abci(misbehavior);
    let mut state = rust.create_query_context(0, false)?;
    let mut ctx = state.context();
    let validator = x_staking::store::get_validator_by_cons_addr(&mut ctx, &consensus_address)?
        .context("zero-slash validator is absent")?;
    let operator = validator.operator()?;
    let power_key = x_staking::keys::power_index(validator.potential_consensus_power()?, &operator);
    let current = x_distribution::store::get_current(&mut ctx, &operator)?;
    let outstanding = x_distribution::store::get_outstanding(&mut ctx, &operator)?;
    let commission = x_distribution::store::get_commission(&mut ctx, &operator)?;
    let supply = x_bank::get_supply(&mut ctx, x_staking::DEFAULT_BOND_DENOM)?.amount;
    let mut minter = x_mint::get_minter(&mut ctx)?;
    let genesis_time = x_mint::get_genesis_time(&mut ctx)?;
    let inflation = x_mint::calculate_inflation_rate(&genesis_time, block_time)?;
    if inflation != minter.inflation_rate || minter.annual_provisions.is_zero() {
        minter.inflation_rate = inflation;
        minter.annual_provisions = minter
            .inflation_rate
            .checked_mul_int(&supply)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let block_provision = minter
        .previous_block_time
        .as_ref()
        .map(|previous| minter.calculate_block_provision(block_time, previous))
        .transpose()?
        .map_or_else(Int::zero, |coin| coin.amount);
    Ok(ZeroSlashState {
        evidence: evidence_branch_state(rust, misbehavior)?,
        operator: operator.clone(),
        tokens: validator.tokens,
        shares: validator.delegator_shares,
        power_index_present: ctx.get(x_staking::STORE_KEY, &power_key)?.is_some(),
        slash_factor: x_slashing::get_params(&mut ctx)?.slash_fraction_double_sign,
        supply,
        block_provision,
        bonded_pool: module_balance(&mut ctx, x_staking::BONDED_POOL)?,
        not_bonded_pool: module_balance(&mut ctx, x_staking::NOT_BONDED_POOL)?,
        current_period: current.period,
        retained_outstanding: outstanding.sub(&current.rewards)?.sub(&commission)?,
        historical_keys: prefix_len(
            &mut ctx,
            x_distribution::STORE_KEY,
            &x_distribution::store::historical_prefix(&operator),
        )?,
        slash_event_keys: prefix_len(
            &mut ctx,
            x_distribution::STORE_KEY,
            &x_distribution::store::slash_event_prefix(&operator),
        )?,
    })
}

fn require_zero_slash_precondition(state: &ZeroSlashState) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.evidence.validator_status == Some(x_staking::types::BondStatus::Bonded)
            && state.evidence.validator_jailed == Some(false)
            && state.evidence.pubkey_present
            && state.evidence.signing_info_present
            && state.evidence.tombstoned == Some(false)
            && !state.evidence.evidence_present
            && state.tokens.is_positive()
            && state.shares.is_positive()
            && state.power_index_present
            && state.slash_factor.is_zero(),
        "zero-slash precondition changed: {state:?}"
    );
    Ok(())
}

fn require_zero_slash_postcondition(
    before: &ZeroSlashState,
    after: &ZeroSlashState,
    misbehavior: &abci::Misbehavior,
    events: &[celestia_proto::celestia_core_proto::abci::Event],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        after.evidence.evidence_present
            && after.evidence.validator_jailed == Some(true)
            && after.evidence.validator_status == Some(x_staking::types::BondStatus::Unbonding)
            && after.evidence.tombstoned == Some(true)
            && after.evidence.jailed_until.is_some_and(|time| {
                time.seconds == x_evidence::DOUBLE_SIGN_JAIL_END_UNIX && time.nanos == 0
            })
            && after.operator == before.operator
            && after.tokens == before.tokens
            && after.shares == before.shares
            && !after.power_index_present
            && after.slash_factor.is_zero(),
        "zero-slash evidence, jail, tombstone, or validator state changed: before={before:?} after={after:?}"
    );
    anyhow::ensure!(
        after.bonded_pool == int_sub(&before.bonded_pool, &before.tokens)?
            && after.not_bonded_pool == int_add(&before.not_bonded_pool, &before.tokens)?
            && after.supply == int_add(&before.supply, &before.block_provision)?,
        "zero-slash economic accounting changed: before={before:?} after={after:?}"
    );
    anyhow::ensure!(
        after.current_period == before.current_period
            && after.retained_outstanding == before.retained_outstanding
            && after.historical_keys == before.historical_keys
            && after.slash_event_keys == before.slash_event_keys,
        "zero-slash unexpectedly changed distribution slash accounting: before={before:?} after={after:?}"
    );
    let validator = misbehavior
        .validator
        .as_ref()
        .context("zero-slash evidence has no validator")?;
    let expected_power = validator.power.to_string();
    let expected_address = encoding::bech32::encode(
        encoding::bech32::VALIDATOR_CONSENSUS_PREFIX,
        &validator.address,
    )?;
    let slash_events: Vec<_> = events
        .iter()
        .filter(|event| {
            event.r#type == "slash"
                && event
                    .attributes
                    .iter()
                    .any(|attribute| attribute.key == "reason" && attribute.value == "double_sign")
        })
        .collect();
    anyhow::ensure!(
        slash_events.len() == 1
            && slash_events[0]
                .attributes
                .iter()
                .any(|attribute| { attribute.key == "burned_coins" && attribute.value == "0" })
            && slash_events[0]
                .attributes
                .iter()
                .any(|attribute| { attribute.key == "power" && attribute.value == expected_power })
            && slash_events[0].attributes.iter().any(|attribute| {
                attribute.key == "address" && attribute.value == expected_address
            }),
        "zero-slash event changed: {slash_events:?}"
    );
    Ok(())
}

#[derive(Debug)]
struct StakeLifecycleState {
    evidence: EvidenceBranchState,
    source_tokens: Int,
    destination_tokens: Int,
    destination_total_shares: LegacyDec,
    destination_status: x_staking::types::BondStatus,
    direct_initial: Int,
    direct_balance: Int,
    direct_creation_height: i64,
    redelegation_initial: Int,
    redelegation_shares: LegacyDec,
    redelegation_creation_height: i64,
    downstream_initial: Int,
    downstream_balance: Int,
    downstream_creation_height: i64,
    destination_shares: LegacyDec,
    slash_factor: LegacyDec,
    supply: Int,
    block_provision: Int,
    bonded_pool: Int,
    not_bonded_pool: Int,
}

fn module_balance(ctx: &mut sdk::Context<'_>, module: &str) -> anyhow::Result<Int> {
    Ok(x_bank::get_balance(
        ctx,
        &x_auth::module_address(module),
        x_staking::DEFAULT_BOND_DENOM,
    )?
    .amount)
}

fn stake_lifecycle_state(
    rust: &app::App,
    plan: &StakeLifecyclePlan,
    misbehavior: &abci::Misbehavior,
    block_time: &pbjson_types::Timestamp,
) -> anyhow::Result<StakeLifecycleState> {
    let source = x_staking::addr::val_from_string(&plan.source)?;
    let destination = x_staking::addr::val_from_string(&plan.destination)?;
    let delegator = x_staking::addr::acc_from_string(&plan.delegator)?;
    let mut state = rust.create_query_context(0, false)?;
    let mut ctx = state.context();
    let source_validator = x_staking::store::validator(&mut ctx, &source)?;
    let destination_validator = x_staking::store::validator(&mut ctx, &destination)?;
    let direct = x_staking::store::get_ubd(&mut ctx, &delegator, &source)?
        .context("stake-lifecycle direct unbonding delegation is absent")?;
    let redelegation = x_staking::store::get_red(&mut ctx, &delegator, &source, &destination)?
        .context("stake-lifecycle redelegation is absent")?;
    let downstream = x_staking::store::get_ubd(&mut ctx, &delegator, &destination)?
        .context("stake-lifecycle downstream unbonding delegation is absent")?;
    let destination_delegation =
        x_staking::store::get_delegation(&mut ctx, &delegator, &destination)?
            .context("stake-lifecycle destination delegation is absent")?;
    anyhow::ensure!(
        direct.entries.len() == 1
            && redelegation.entries.len() == 1
            && downstream.entries.len() == 1,
        "stake-lifecycle expected exactly one direct, redelegation, and downstream entry"
    );
    let direct = &direct.entries[0];
    let redelegation = &redelegation.entries[0];
    let downstream = &downstream.entries[0];
    let supply = x_bank::get_supply(&mut ctx, x_staking::DEFAULT_BOND_DENOM)?.amount;
    let mut minter = x_mint::get_minter(&mut ctx)?;
    let genesis_time = x_mint::get_genesis_time(&mut ctx)?;
    let inflation = x_mint::calculate_inflation_rate(&genesis_time, block_time)?;
    if inflation != minter.inflation_rate || minter.annual_provisions.is_zero() {
        minter.inflation_rate = inflation;
        minter.annual_provisions = minter
            .inflation_rate
            .checked_mul_int(&supply)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let block_provision = minter
        .previous_block_time
        .as_ref()
        .map(|previous| minter.calculate_block_provision(block_time, previous))
        .transpose()?
        .map_or_else(Int::zero, |coin| coin.amount);
    let bonded_pool = module_balance(&mut ctx, x_staking::BONDED_POOL)?;
    let not_bonded_pool = module_balance(&mut ctx, x_staking::NOT_BONDED_POOL)?;
    let slash_factor = x_slashing::get_params(&mut ctx)?.slash_fraction_double_sign;
    Ok(StakeLifecycleState {
        evidence: evidence_branch_state(rust, misbehavior)?,
        source_tokens: source_validator.tokens,
        destination_tokens: destination_validator.tokens,
        destination_total_shares: destination_validator.delegator_shares,
        destination_status: destination_validator.status,
        direct_initial: direct.initial_balance.clone(),
        direct_balance: direct.balance.clone(),
        direct_creation_height: direct.creation_height,
        redelegation_initial: redelegation.initial_balance.clone(),
        redelegation_shares: redelegation.shares_dst.clone(),
        redelegation_creation_height: redelegation.creation_height,
        downstream_initial: downstream.initial_balance.clone(),
        downstream_balance: downstream.balance.clone(),
        downstream_creation_height: downstream.creation_height,
        destination_shares: destination_delegation.shares,
        slash_factor,
        supply,
        block_provision,
        bonded_pool,
        not_bonded_pool,
    })
}

fn dec_mul_int(dec: &LegacyDec, value: &Int) -> anyhow::Result<Int> {
    Ok(dec
        .checked_mul_int(value)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .truncate_int())
}

fn tokens_from_shares(
    shares: &LegacyDec,
    tokens: &Int,
    total_shares: &LegacyDec,
) -> anyhow::Result<Int> {
    Ok(shares
        .checked_mul_int(tokens)
        .and_then(|x| x.checked_quo(total_shares))
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .truncate_int())
}

fn int_sub(value: &Int, amount: &Int) -> anyhow::Result<Int> {
    value
        .checked_sub(amount)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn int_add(value: &Int, amount: &Int) -> anyhow::Result<Int> {
    value
        .checked_add(amount)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn min_int(a: &Int, b: &Int) -> Int {
    if a < b { a.clone() } else { b.clone() }
}

fn require_stake_lifecycle_precondition(
    state: &StakeLifecycleState,
    misbehavior: &abci::Misbehavior,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.evidence.validator_status == Some(x_staking::types::BondStatus::Bonded)
            && state.evidence.validator_jailed == Some(false)
            && state.evidence.pubkey_present
            && state.evidence.signing_info_present
            && state.evidence.tombstoned == Some(false)
            && !state.evidence.evidence_present,
        "stake-lifecycle source validator is not fresh, bonded, and slashable: {:?}",
        state.evidence
    );
    let distribution_height = misbehavior
        .height
        .checked_sub(x_evidence::VALIDATOR_UPDATE_DELAY)
        .context("stake-lifecycle distribution height underflow")?;
    anyhow::ensure!(
        state.direct_creation_height >= distribution_height
            && state.redelegation_creation_height >= distribution_height
            && state.downstream_creation_height >= distribution_height,
        "stake-lifecycle entries predate the slash distribution height"
    );
    anyhow::ensure!(
        state.direct_initial == Int::from_u64(200_000_000)
            && state.direct_balance == state.direct_initial
            && state.redelegation_initial == Int::from_u64(200_000_000)
            && state.downstream_initial == Int::from_u64(1_000_000)
            && state.downstream_balance == state.downstream_initial
            && state.redelegation_shares.is_positive()
            && state.destination_shares.is_positive()
            && state.destination_total_shares.is_positive()
            && state.destination_status == x_staking::types::BondStatus::Bonded
            && state.slash_factor.is_positive(),
        "stake-lifecycle historical stake state is incomplete: {state:?}"
    );
    let direct = dec_mul_int(&state.slash_factor, &state.direct_initial)?;
    let redelegated = dec_mul_int(&state.slash_factor, &state.redelegation_initial)?;
    anyhow::ensure!(
        direct.is_positive()
            && redelegated.is_positive()
            && min_int(&redelegated, &state.downstream_balance).is_positive(),
        "stake-lifecycle slash amounts must exercise every historical-stake branch"
    );
    Ok(())
}

fn require_stake_lifecycle_postcondition(
    before: &StakeLifecycleState,
    after: &StakeLifecycleState,
    misbehavior: &abci::Misbehavior,
) -> anyhow::Result<()> {
    let power = misbehavior
        .validator
        .as_ref()
        .context("stake-lifecycle evidence has no validator")?
        .power;
    let slash_base = x_staking::types::tokens_from_consensus_power(power)?;
    let total_slash = dec_mul_int(&before.slash_factor, &slash_base)?;
    let direct_total = dec_mul_int(&before.slash_factor, &before.direct_initial)?;
    let redelegation_total = dec_mul_int(&before.slash_factor, &before.redelegation_initial)?;
    let direct_burn = min_int(&direct_total, &before.direct_balance);
    let downstream_burn = min_int(&redelegation_total, &before.downstream_balance);
    let mut shares_burn = before
        .slash_factor
        .checked_mul(&before.redelegation_shares)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if shares_burn > before.destination_shares {
        shares_burn = before.destination_shares.clone();
    }
    let destination_burn = tokens_from_shares(
        &shares_burn,
        &before.destination_tokens,
        &before.destination_total_shares,
    )?;
    let source_remainder = int_sub(&int_sub(&total_slash, &direct_total)?, &redelegation_total)?;
    let source_burn = min_int(&source_remainder, &before.source_tokens);
    anyhow::ensure!(
        direct_burn.is_positive()
            && downstream_burn.is_positive()
            && destination_burn.is_positive()
            && source_burn.is_positive(),
        "stake-lifecycle expected nonzero burns in every target branch"
    );
    let expected_direct = int_sub(&before.direct_balance, &direct_burn)?;
    let expected_downstream = int_sub(&before.downstream_balance, &downstream_burn)?;
    let expected_destination_shares = before
        .destination_shares
        .checked_sub(&shares_burn)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let expected_destination_total_shares = before
        .destination_total_shares
        .checked_sub(&shares_burn)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let expected_destination_tokens = int_sub(&before.destination_tokens, &destination_burn)?;
    let expected_source_tokens = int_sub(&before.source_tokens, &source_burn)?;
    let expected_not_bonded = int_sub(
        &int_sub(
            &int_add(&before.not_bonded_pool, &expected_source_tokens)?,
            &direct_burn,
        )?,
        &downstream_burn,
    )?;
    let expected_bonded = int_sub(
        &int_sub(&before.bonded_pool, &before.source_tokens)?,
        &destination_burn,
    )?;
    let total_burn = int_add(
        &int_add(&direct_burn, &downstream_burn)?,
        &int_add(&destination_burn, &source_burn)?,
    )?;
    let expected_supply = int_sub(
        &int_add(&before.supply, &before.block_provision)?,
        &total_burn,
    )?;
    anyhow::ensure!(
        after.evidence.evidence_present
            && after.evidence.validator_jailed == Some(true)
            && after.evidence.validator_status == Some(x_staking::types::BondStatus::Unbonding)
            && after.evidence.tombstoned == Some(true)
            && after.evidence.jailed_until.is_some_and(|t| {
                t.seconds == x_evidence::DOUBLE_SIGN_JAIL_END_UNIX && t.nanos == 0
            }),
        "stake-lifecycle evidence, jail, or tombstone postcondition failed: {:?}",
        after.evidence
    );
    anyhow::ensure!(
        after.direct_balance == expected_direct
            && after.downstream_balance == expected_downstream
            && after.destination_shares == expected_destination_shares
            && after.destination_total_shares == expected_destination_total_shares
            && after.destination_tokens == expected_destination_tokens
            && after.source_tokens == expected_source_tokens
            && after.not_bonded_pool == expected_not_bonded
            && after.bonded_pool == expected_bonded
            && after.supply == expected_supply,
        "stake-lifecycle slash accounting differs: before={before:?} after={after:?}"
    );
    anyhow::ensure!(
        after.direct_initial == before.direct_initial
            && after.direct_creation_height == before.direct_creation_height
            && after.redelegation_initial == before.redelegation_initial
            && after.redelegation_shares == before.redelegation_shares
            && after.redelegation_creation_height == before.redelegation_creation_height
            && after.downstream_initial == before.downstream_initial
            && after.downstream_creation_height == before.downstream_creation_height,
        "stake-lifecycle slash changed immutable historical entry fields"
    );
    Ok(())
}

fn require_evidence_branch_precondition(
    requirement: EvidenceRequirement,
    rust: &app::App,
    misbehavior: &abci::Misbehavior,
    block_height: i64,
    block_time: &pbjson_types::Timestamp,
) -> anyhow::Result<EvidenceBranchState> {
    let state = evidence_branch_state(rust, misbehavior)?;
    anyhow::ensure!(
        !state.evidence_present,
        "controlled evidence already exists before FinalizeBlock"
    );
    match requirement {
        EvidenceRequirement::RemovedValidator => anyhow::ensure!(
            state.validator_status.is_none(),
            "controlled removed-validator address exists in the staking consensus-address index"
        ),
        EvidenceRequirement::Stale => {
            anyhow::ensure!(
                state
                    .validator_status
                    .is_some_and(|s| s != x_staking::types::BondStatus::Unbonded),
                "controlled stale-evidence validator is missing or unbonded"
            );
            anyhow::ensure!(
                state.pubkey_present,
                "controlled stale-evidence validator has no slashing consensus pubkey"
            );
            anyhow::ensure!(
                state.signing_info_present,
                "controlled stale-evidence validator has no signing info"
            );
            anyhow::ensure!(
                state.tombstoned == Some(false),
                "controlled stale-evidence validator is already tombstoned"
            );
            let evidence_time = misbehavior
                .time
                .as_ref()
                .context("controlled stale evidence has no time")?;
            anyhow::ensure!(
                block_height
                    .checked_sub(misbehavior.height)
                    .is_some_and(|age| age > 1)
                    && sdk::time::sub(block_time, evidence_time) > 1_000_000_000,
                "controlled stale evidence does not exceed the patched block and duration limits"
            );
        }
        EvidenceRequirement::StakeLifecycle => {
            anyhow::bail!("stake-lifecycle uses its dedicated state precondition")
        }
        EvidenceRequirement::ZeroDoubleSignSlash => {
            anyhow::bail!("zero-double-sign-slash uses its dedicated state precondition")
        }
    }
    Ok(state)
}

fn require_evidence_branch_unchanged(
    requirement: EvidenceRequirement,
    before: &EvidenceBranchState,
    after: &EvidenceBranchState,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !after.evidence_present,
        "controlled ignored evidence was stored"
    );
    match requirement {
        EvidenceRequirement::RemovedValidator => anyhow::ensure!(
            after.validator_status.is_none(),
            "controlled removed-validator address appeared in the staking index"
        ),
        EvidenceRequirement::Stale => {
            anyhow::ensure!(
                after.validator_status == before.validator_status
                    && after.validator_jailed == before.validator_jailed
                    && after.tombstoned == before.tombstoned,
                "controlled stale evidence changed validator jail, bond, or tombstone state: before={before:?} after={after:?}"
            );
        }
        EvidenceRequirement::StakeLifecycle => {
            anyhow::bail!("stake-lifecycle uses its dedicated state postcondition")
        }
        EvidenceRequirement::ZeroDoubleSignSlash => {
            anyhow::bail!("zero-double-sign-slash uses its dedicated state postcondition")
        }
    }
    Ok(())
}

fn removed_validator_misbehavior(
    height: i64,
    time: pbjson_types::Timestamp,
    total_voting_power: i64,
) -> abci::Misbehavior {
    abci::Misbehavior {
        r#type: 1,
        validator: Some(abci::Validator {
            // A deterministic 20-byte consensus address absent from the
            // controlled fixture's validator set.
            address: CONTROLLED_REMOVED_VALIDATOR.to_vec(),
            power: 1,
        }),
        height,
        time: Some(time),
        total_voting_power,
    }
}

fn stale_misbehavior(
    height: i64,
    valsets: &BTreeMap<i64, ValSet>,
    times: &BTreeMap<i64, pbjson_types::Timestamp>,
) -> anyhow::Result<abci::Misbehavior> {
    let evidence_height = height
        .checked_sub(2)
        .context("controlled stale evidence height underflow")?;
    let set = valsets
        .get(&evidence_height)
        .context("no validator set for controlled stale evidence")?;
    let (address, power, _) = set
        .first()
        .context("empty validator set for controlled stale evidence")?;
    let evidence_time = times
        .get(&evidence_height)
        .copied()
        .context("no block time for controlled stale evidence")?;
    Ok(abci::Misbehavior {
        r#type: 1,
        validator: Some(abci::Validator {
            address: address.clone(),
            power: *power,
        }),
        height: evidence_height,
        time: Some(evidence_time),
        total_voting_power: set.iter().map(|v| v.1).sum(),
    })
}

fn active_double_sign_misbehavior(
    height: i64,
    valsets: &BTreeMap<i64, ValSet>,
    times: &BTreeMap<i64, pbjson_types::Timestamp>,
) -> anyhow::Result<abci::Misbehavior> {
    let infraction_height = height
        .checked_sub(1)
        .context("zero-slash infraction height underflow")?;
    let set = valsets
        .get(&infraction_height)
        .context("zero-slash infraction validator set is absent")?;
    let (address, power, _) = set
        .iter()
        .find(|(_, power, _)| *power > 0)
        .context("zero-slash infraction validator set has no positive-power validator")?;
    let time = times
        .get(&infraction_height)
        .copied()
        .context("zero-slash infraction time is absent")?;
    anyhow::ensure!(
        x_staking::types::tokens_from_consensus_power(*power)?.is_positive(),
        "zero-slash evidence power converts to zero tokens"
    );
    Ok(abci::Misbehavior {
        r#type: 1,
        validator: Some(abci::Validator {
            address: address.clone(),
            power: *power,
        }),
        height: infraction_height,
        time: Some(time),
        total_voting_power: set.iter().map(|validator| validator.1).sum(),
    })
}

fn stake_lifecycle_misbehavior(
    rust: &app::App,
    plan: &StakeLifecyclePlan,
    height: i64,
    time: pbjson_types::Timestamp,
    valsets: &BTreeMap<i64, ValSet>,
) -> anyhow::Result<abci::Misbehavior> {
    let source = x_staking::addr::val_from_string(&plan.source)?;
    let mut state = rust.create_query_context(0, false)?;
    let mut ctx = state.context();
    let validator = x_staking::store::validator(&mut ctx, &source)?;
    anyhow::ensure!(
        validator.status == x_staking::types::BondStatus::Bonded && !validator.jailed,
        "stake-lifecycle source validator is not bonded and active"
    );
    let consensus_address = validator.cons_addr()?;
    let set = valsets
        .get(&height)
        .context("stake-lifecycle has no recorded validator set at the infraction height")?;
    let (_, power, _) = set
        .iter()
        .find(|(address, _, _)| *address == consensus_address)
        .context("stake-lifecycle source validator is absent from the infraction validator set")?;
    anyhow::ensure!(*power > 0, "stake-lifecycle source validator has no power");
    Ok(abci::Misbehavior {
        r#type: 1,
        validator: Some(abci::Validator {
            address: consensus_address,
            power: *power,
        }),
        height,
        time: Some(time),
        total_voting_power: set.iter().map(|v| v.1).sum(),
    })
}

fn apply_updates(set: &ValSet, updates: &[abci::ValidatorUpdate]) -> ValSet {
    let mut out = set.clone();
    for u in updates {
        let Some(a) = update_address(u) else {
            continue;
        };
        out.retain(|v| v.0 != a);
        if u.power > 0 {
            let fuzz = !set.iter().any(|v| v.0 == a && !v.2);
            out.push((a, u.power, fuzz));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// Learns created objects from a delivered tx (validators, proposals,
/// unbondings).
fn learn(gn: &mut Gen, app_version: u64, tx: &[u8], res: &abci::ExecTxResult, height: i64) {
    let events = &res.events;
    let mut submitted = Vec::new();
    for e in events {
        for a in &e.attributes {
            if a.key == "proposal_id"
                && let Ok(id) = a.value.parse::<u64>()
            {
                if e.r#type == "submit_proposal" && !submitted.contains(&id) {
                    submitted.push(id);
                }
                if !gn.proposals.contains(&id) {
                    gn.proposals.push(id);
                }
            }
        }
    }
    let Ok(t) = txdecode::decode_for(app_version, tx) else {
        return;
    };
    // The proposers of the proposals the tx submitted, in order.
    let proposers: Vec<String> = t
        .body
        .messages
        .iter()
        .filter_map(|a| match a.type_url.as_str() {
            "/cosmos.gov.v1.MsgSubmitProposal" => {
                celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice())
                    .ok()
                    .map(|m| m.proposer)
            }
            "/cosmos.gov.v1beta1.MsgSubmitProposal" => {
                celestia_proto::cosmos::gov::v1beta1::MsgSubmitProposal::decode(a.value.as_slice())
                    .ok()
                    .map(|m| m.proposer)
            }
            _ => None,
        })
        .collect();
    // Hyperlane objects: the create messages' response ids, owned by the
    // message signer.
    if let Ok(d) =
        celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(res.data.as_slice())
    {
        for (r, m) in d.msg_responses.iter().zip(&t.body.messages) {
            let Some(kind) = hl_kind(&r.type_url) else {
                continue;
            };
            let id = parse_fields(&r.value)
                .unwrap_or_default()
                .into_iter()
                .find(|f| f.num == 1 && f.wt == 2)
                .map(|f| String::from_utf8_lossy(&f.val).into_owned());
            let owner = parse_fields(&m.value)
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.wt == 2)
                .find_map(|f| {
                    let s = String::from_utf8_lossy(&f.val).into_owned();
                    gn.accts.iter().position(|a| a.acc == s)
                });
            if let (Some(id), Some(o)) = (id, owner) {
                gn.hl_objects.push((kind, id, o));
                if gn.hl_objects.len() > 128 {
                    gn.hl_objects.remove(0);
                }
            }
        }
    }
    learn_hyperlane(gn, &t.body.messages, &res.data);
    for (id, p) in submitted.iter().zip(&proposers) {
        if let Some(k) = gn.accts.iter().position(|x| x.acc == *p) {
            gn.proposers.push((*id, k));
            if gn.proposers.len() > 64 {
                gn.proposers.remove(0);
            }
        }
    }
    for a in &t.body.messages {
        match a.type_url.as_str() {
            "/cosmos.staking.v1beta1.MsgCreateValidator" => {
                if let Ok(m) = celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator::decode(
                    a.value.as_slice(),
                ) && !gn.validators.contains(&m.validator_address)
                {
                    gn.validators.push(m.validator_address);
                }
            }
            "/cosmos.staking.v1beta1.MsgUndelegate" => {
                if let Ok(m) = celestia_proto::cosmos::staking::v1beta1::MsgUndelegate::decode(
                    a.value.as_slice(),
                ) && let Some(i) = gn.accts.iter().position(|x| x.acc == m.delegator_address)
                {
                    gn.unbondings.push((i, m.validator_address, height));
                    if gn.unbondings.len() > 64 {
                        gn.unbondings.remove(0);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Blocks of scripted txs (`DELIVER_SCRIPT`), run before the generated
/// ones: one tx per line, `<account index> <type URL> <hex value> [gas]`
/// (one message, signed in direct mode), or `vote <account index>` (a yes
/// vote on the latest proposal); a line `---` ends a block; `#` starts a
/// comment.
type Script = std::collections::VecDeque<Vec<(usize, Any, u64)>>;

#[derive(Debug, Clone)]
struct StakeLifecyclePlan {
    delegator: String,
    source: String,
    destination: String,
}

fn required_coin(coin: Option<&Coin>, amount: &str) -> anyhow::Result<()> {
    let coin = coin.context("stake-lifecycle message has no amount")?;
    anyhow::ensure!(
        coin.denom == x_staking::DEFAULT_BOND_DENOM && coin.amount == amount,
        "stake-lifecycle amount must be {amount} utia, got {coin:?}"
    );
    Ok(())
}

fn stake_lifecycle_plan(script: &Script) -> anyhow::Result<StakeLifecyclePlan> {
    use celestia_proto::cosmos::staking::v1beta1::{
        MsgBeginRedelegate, MsgCreateValidator, MsgDelegate, MsgUndelegate,
    };
    use prost::Name as _;

    anyhow::ensure!(
        script.len() == 6 && script.iter().map(Vec::len).eq([1usize, 1, 0, 1, 1, 1]),
        "stake-lifecycle script must contain the exact six-block setup"
    );
    let one = |block: usize, account: usize| -> anyhow::Result<&Any> {
        let (actual, msg, _) = &script[block][0];
        anyhow::ensure!(
            *actual == account,
            "stake-lifecycle block {block} must use account {account}"
        );
        Ok(msg)
    };
    let create = MsgCreateValidator::decode(one(0, 0)?.value.as_slice())?;
    let delegate = MsgDelegate::decode(one(1, 1)?.value.as_slice())?;
    let direct = MsgUndelegate::decode(one(3, 1)?.value.as_slice())?;
    let redelegate = MsgBeginRedelegate::decode(one(4, 1)?.value.as_slice())?;
    let downstream = MsgUndelegate::decode(one(5, 1)?.value.as_slice())?;
    anyhow::ensure!(
        one(0, 0)?.type_url == MsgCreateValidator::type_url()
            && one(1, 1)?.type_url == MsgDelegate::type_url()
            && one(3, 1)?.type_url == MsgUndelegate::type_url()
            && one(4, 1)?.type_url == MsgBeginRedelegate::type_url()
            && one(5, 1)?.type_url == MsgUndelegate::type_url(),
        "stake-lifecycle script contains an unexpected message type"
    );
    required_coin(create.value.as_ref(), "100000000")?;
    required_coin(delegate.amount.as_ref(), "400000000")?;
    required_coin(direct.amount.as_ref(), "200000000")?;
    required_coin(redelegate.amount.as_ref(), "200000000")?;
    required_coin(downstream.amount.as_ref(), "1000000")?;
    anyhow::ensure!(
        delegate.delegator_address == direct.delegator_address
            && delegate.delegator_address == redelegate.delegator_address
            && delegate.delegator_address == downstream.delegator_address
            && delegate.validator_address == direct.validator_address
            && delegate.validator_address == redelegate.validator_src_address
            && create.validator_address == redelegate.validator_dst_address
            && create.validator_address == downstream.validator_address
            && delegate.validator_address != create.validator_address,
        "stake-lifecycle script does not describe the required source/destination topology"
    );
    Ok(StakeLifecyclePlan {
        delegator: delegate.delegator_address,
        source: delegate.validator_address,
        destination: create.validator_address,
    })
}

fn read_script(text: &str, accounts: usize) -> anyhow::Result<Script> {
    let mut out = Script::new();
    let mut cur = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == "---" {
            out.push_back(std::mem::take(&mut cur));
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if let ["vote", i] = f.as_slice() {
            let i: usize = i.parse()?;
            anyhow::ensure!(i < accounts, "account index {i} out of range");
            cur.push((
                i,
                Any {
                    type_url: "vote".into(),
                    value: Vec::new(),
                },
                1_000_000,
            ));
            continue;
        }
        let [i, url, val, rest @ ..] = f.as_slice() else {
            anyhow::bail!("bad script line {line:?}");
        };
        let i: usize = i.parse()?;
        anyhow::ensure!(i < accounts, "account index {i} out of range");
        let gas = rest.first().map_or(Ok(4_000_000), |g| g.parse())?;
        cur.push((
            i,
            Any {
                type_url: (*url).to_owned(),
                value: hex::decode(val)?,
            },
            gas,
        ));
    }
    if !cur.is_empty() {
        out.push_back(cur);
    }
    Ok(out)
}

/// A `genesis.json` for a scenario recorded without one (the ibctesting
/// fixtures): chain id, genesis time, initial height and app state of the
/// recorded `InitChain`, the consensus section of the `ibc` scenario's
/// genesis (the Go app takes the consensus parameters of `InitChain`).
pub fn genesis_json(scenario: &str) -> anyhow::Result<String> {
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let ic = parse(&data)?
        .into_iter()
        .filter(|r| r.dir == Direction::Request)
        .find_map(|r| match Request::decode(r.bytes.as_slice()).ok()?.value {
            Some(request::Value::InitChain(ic)) => Some(ic),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no InitChain in {scenario}"))?;
    let tpl = std::fs::read(devnet_conformance::scenario_dir("ibc").join("genesis.json"))?;
    let mut g: serde_json::Value = serde_json::from_slice(&tpl)?;
    let t = ic.time.unwrap_or_default();
    let secs = t.seconds;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    g["genesis_time"] = serde_json::Value::String(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:09}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        t.nanos
    ));
    g["chain_id"] = serde_json::Value::String(ic.chain_id.clone());
    g["initial_height"] = serde_json::Value::from(ic.initial_height.max(1));
    g["app_state"] = serde_json::from_slice(&ic.app_state_bytes)?;
    Ok(serde_json::to_string_pretty(&g)?)
}

/// The fuzz accounts' addresses (bech32), in index order.
#[must_use]
pub fn account_addresses() -> Vec<String> {
    accounts(1).into_iter().map(|a| a.acc).collect()
}

/// Hyperlane wiring from a successful tx's messages and responses.
fn learn_hyperlane(gn: &mut Gen, msgs: &[Any], data: &[u8]) {
    use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
    use celestia_proto::hyperlane::core::v1 as core;
    use celestia_proto::hyperlane::warp::v1 as warp;
    let Ok(d) = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(data) else {
        return;
    };
    let resp_id = |r: &Any| {
        parse_fields(&r.value)
            .unwrap_or_default()
            .into_iter()
            .find(|f| f.num == 1 && f.wt == 2)
            .map(|f| String::from_utf8_lossy(&f.val).into_owned())
    };
    for (r, m) in d.msg_responses.iter().zip(msgs) {
        let v = m.value.as_slice();
        let id = resp_id(r);
        match (m.type_url.as_str(), id) {
            ("/hyperlane.core.v1.MsgCreateMailbox", Some(id)) => {
                if let Ok(x) = core::MsgCreateMailbox::decode(v) {
                    gn.hl.mailbox_domain.insert(id, x.local_domain);
                }
            }
            ("/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm", Some(id)) => {
                gn.hl.noop_isms.insert(id);
            }
            ("/hyperlane.warp.v1.MsgCreateSyntheticToken", Some(id)) => {
                if let Ok(x) = warp::MsgCreateSyntheticToken::decode(v) {
                    gn.hl.token_mailbox.insert(id, x.origin_mailbox);
                }
            }
            ("/hyperlane.warp.v1.MsgCreateCollateralToken", Some(id)) => {
                if let Ok(x) = warp::MsgCreateCollateralToken::decode(v) {
                    gn.hl.token_mailbox.insert(id, x.origin_mailbox);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgCreateIgp", Some(id)) => {
                if let Ok(x) = pd::MsgCreateIgp::decode(v) {
                    gn.hl.igp_denom.insert(id, x.denom);
                }
            }
            ("/hyperlane.warp.v1.MsgRemoteTransfer", Some(id)) => {
                push_capped(&mut gn.hl.message_ids, id);
            }
            ("/hyperlane.warp.v1.MsgEnrollRemoteRouter", _) => {
                if let Ok(x) = warp::MsgEnrollRemoteRouter::decode(v)
                    && let Some(r) = x.remote_router
                {
                    push_capped(
                        &mut gn.hl.routers,
                        (x.token_id, r.receiver_domain, r.receiver_contract),
                    );
                }
            }
            ("/hyperlane.warp.v1.MsgUnrollRemoteRouter", _) => {
                if let Ok(x) = warp::MsgUnrollRemoteRouter::decode(v) {
                    gn.hl
                        .routers
                        .retain(|r| r.0 != x.token_id || r.1 != x.receiver_domain);
                }
            }
            ("/hyperlane.core.v1.MsgSetMailbox", _) => {
                if let Ok(x) = core::MsgSetMailbox::decode(v) {
                    hl_transfer(gn, &x.mailbox_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.warp.v1.MsgSetToken", _) => {
                if let Ok(x) = warp::MsgSetToken::decode(v) {
                    hl_transfer(gn, &x.token_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner", _) => {
                if let Ok(x) = pd::MsgSetIgpOwner::decode(v) {
                    hl_transfer(gn, &x.igp_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner", _) => {
                if let Ok(x) =
                    celestia_proto::hyperlane::core::interchain_security::v1::MsgUpdateRoutingIsmOwner::decode(v)
                {
                    hl_transfer(gn, &x.ism_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig", _) => {
                if let Ok(x) = pd::MsgSetDestinationGasConfig::decode(v)
                    && let Some(c) = x.destination_gas_config
                {
                    push_capped(&mut gn.hl.igp_domains, (x.igp_id, c.remote_domain));
                }
            }
            _ => {}
        }
    }
}

/// A hyperlane object changed owner (`new_owner` set) or was renounced:
/// its tracked owner follows, or it is dropped when no fuzz account owns
/// it any more.
fn hl_transfer(gn: &mut Gen, id: &str, new_owner: &str, renounce: bool) {
    if !renounce && new_owner.is_empty() {
        return;
    }
    let to = (!renounce)
        .then(|| gn.accts.iter().position(|a| a.acc == new_owner))
        .flatten();
    match to {
        Some(k) => {
            for o in gn.hl_objects.iter_mut().filter(|o| o.1 == id) {
                o.2 = k;
            }
        }
        None => gn.hl_objects.retain(|o| o.1 != id),
    }
}

/// A sweep template from a tx Go executed successfully: its only signer
/// (a fuzz account), its messages and the gas Go used.
fn sweep_template(
    gn: &Gen,
    app_version: u64,
    tx: &[u8],
    res: &abci::ExecTxResult,
) -> Option<(usize, Vec<Any>, u64)> {
    if txdecode::unmarshal_blob_tx(tx).is_some() {
        return None;
    }
    let t = txdecode::decode_for(app_version, tx).ok()?;
    let signers = x_ante::txview::signers(&t).ok()?;
    let [s] = signers.as_slice() else {
        return None;
    };
    let i = gn.accts.iter().position(|a| a.addr == *s)?;
    Some((i, t.body.messages.clone(), res.gas_used.unsigned_abs()))
}

/// Copies of a sweep template signed by account `i` with consecutive
/// sequences and decreasing gas limits below `used` (a copy that runs out
/// of gas in the ante handler fails `CheckTx`, and so do the rest).
fn sweep_txs(gn: &mut Gen, i: usize, msgs: &[Any], used: u64) -> Vec<Vec<u8>> {
    let n = 3 + gn.m.rng.below(8);
    let mut limits: Vec<u64> = (0..n)
        .map(|_| match gn.m.rng.below(4) {
            // Near the end of the handler.
            0 => used.saturating_sub(1 + gn.m.rng.below(4000) as u64),
            _ => used / 3 + gn.m.rng.below((used - used / 3).max(1) as usize) as u64,
        })
        .collect();
    limits.sort_unstable_by(|a, b| b.cmp(a));
    let mut out = Vec::new();
    for l in limits {
        if let Some(t) = sign_simple(gn, i, msgs.to_vec(), l) {
            out.push(t);
            gn.accts[i].seq += 1;
        }
    }
    out
}

const NEVER_BONDED_SELF_DELEGATION: u128 = 1_000_000;
const CONTROLLED_TX_FEE: u128 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnbondedRedelegationRollbackState {
    exact: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    prefixes: Vec<(Vec<u8>, Vec<u8>)>,
    destination_current_period: u64,
    bonded_pool: Int,
    not_bonded_pool: Int,
    signer_balance: u128,
    signer_sequence: u64,
    recovery_balance: u128,
}

struct UnbondedRedelegationRollbackFixture {
    account: String,
    account_bytes: Vec<u8>,
    source: String,
    source_bytes: Vec<u8>,
    destination: String,
    destination_bytes: Vec<u8>,
    recovery: String,
    baseline: Option<UnbondedRedelegationRollbackState>,
    pending_tx: Option<Vec<u8>>,
    phase: usize,
}

impl UnbondedRedelegationRollbackFixture {
    fn new(gn: &Gen, rust: &app::App) -> anyhow::Result<Self> {
        let account = gn.accts[0].acc.clone();
        let source = gn.accts[0].val.clone();
        let account_bytes = x_staking::addr::acc_from_string(&account)?;
        let source_bytes = x_staking::addr::val_from_string(&source)?;
        let mut state = rust.create_query_context(0, false)?;
        let mut ctx = state.context();
        anyhow::ensure!(
            x_staking::store::get_params(&mut ctx)?.max_validators == 1,
            "unbonded redelegation rollback fixture did not set max_validators=1"
        );
        anyhow::ensure!(
            x_staking::store::get_validator(&mut ctx, &source_bytes)?.is_none(),
            "unbonded redelegation rollback source already exists"
        );
        let bonded = bonded_validators_from_context(&mut ctx)?;
        anyhow::ensure!(
            bonded.len() == 1 && bonded[0] != source,
            "unbonded redelegation rollback needs one distinct bonded destination"
        );
        Ok(Self {
            destination_bytes: x_staking::addr::val_from_string(&bonded[0])?,
            destination: bonded[0].clone(),
            recovery: gn.accts[1].acc.clone(),
            account,
            account_bytes,
            source,
            source_bytes,
            baseline: None,
            pending_tx: None,
            phase: 0,
        })
    }

    fn tx(&mut self, gn: &Gen, phase: usize) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(
            phase == self.phase,
            "unbonded redelegation rollback phase changed"
        );
        let coin = |amount: u128| {
            Some(Coin {
                denom: "utia".into(),
                amount: amount.to_string(),
            })
        };
        let message = match phase {
            0 => Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator {
                    description: Some(celestia_proto::cosmos::staking::v1beta1::Description {
                        moniker: "unbonded-redelegation-rollback".into(),
                        ..Default::default()
                    }),
                    commission: Some(celestia_proto::cosmos::staking::v1beta1::CommissionRates {
                        rate: "100000000000000000".into(),
                        max_rate: "200000000000000000".into(),
                        max_change_rate: "10000000000000000".into(),
                    }),
                    min_self_delegation: NEVER_BONDED_SELF_DELEGATION.to_string(),
                    #[allow(deprecated)]
                    delegator_address: String::new(),
                    validator_address: self.source.clone(),
                    pubkey: Some(Any::pack(
                        &celestia_proto::cosmos::crypto::ed25519::PubKey { key: vec![43; 32] },
                    )),
                    value: coin(NEVER_BONDED_SELF_DELEGATION),
                },
            ),
            1 => Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgBeginRedelegate {
                    delegator_address: self.account.clone(),
                    validator_src_address: self.source.clone(),
                    validator_dst_address: self.destination.clone(),
                    amount: coin(NEVER_BONDED_SELF_DELEGATION),
                },
            ),
            2 => Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                from_address: self.account.clone(),
                to_address: self.recovery.clone(),
                amount: vec![coin(1).context("constructing recovery coin")?],
            }),
            _ => anyhow::bail!("unbonded redelegation rollback has no phase {phase}"),
        };
        let tx = sign_simple(gn, 0, vec![message], 1_000_000)
            .context("signing unbonded redelegation rollback transaction")?;
        self.pending_tx = Some(tx.clone());
        Ok(tx)
    }

    fn require_phase(
        &mut self,
        tx: &[u8],
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<bool> {
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "unbonded redelegation rollback phase {} did not finalize exactly once",
            self.phase
        );
        let result = matches[0].1;
        match self.phase {
            0 => {
                anyhow::ensure!(
                    result.code == 0 && result.codespace.is_empty(),
                    "unbonded redelegation source creation changed: {result:?}"
                );
                let state = self.state(rust)?;
                self.require_source_preconditions(rust, &state)?;
                self.baseline = Some(state);
            }
            1 => {
                anyhow::ensure!(
                    result.code == 3
                        && result.codespace == "staking"
                        && result.log
                            == "failed to execute message; message index: 0: validator does not exist",
                    "unbonded all-shares redelegation result changed: {result:?}"
                );
                let baseline = self
                    .baseline
                    .as_ref()
                    .context("unbonded redelegation rollback baseline is absent")?;
                let after = self.state(rust)?;
                anyhow::ensure!(
                    after.exact == baseline.exact
                        && after.prefixes == baseline.prefixes
                        && after.destination_current_period == baseline.destination_current_period
                        && after.bonded_pool == baseline.bonded_pool
                        && after.not_bonded_pool == baseline.not_bonded_pool
                        && after.signer_balance + CONTROLLED_TX_FEE == baseline.signer_balance
                        && after.signer_sequence == baseline.signer_sequence + 1
                        && after.recovery_balance == baseline.recovery_balance,
                    "unbonded all-shares redelegation did not roll back exact message state: before={baseline:?} after={after:?}"
                );
            }
            2 => {
                anyhow::ensure!(
                    result.code == 0 && result.codespace.is_empty(),
                    "unbonded redelegation recovery changed: {result:?}"
                );
                let baseline = self
                    .baseline
                    .as_ref()
                    .context("unbonded redelegation rollback baseline is absent")?;
                let after = self.state(rust)?;
                anyhow::ensure!(
                    after.exact == baseline.exact
                        && after.prefixes == baseline.prefixes
                        && after.destination_current_period == baseline.destination_current_period
                        && after.bonded_pool == baseline.bonded_pool
                        && after.not_bonded_pool == baseline.not_bonded_pool
                        && after.signer_balance + 2 * CONTROLLED_TX_FEE + 1
                            == baseline.signer_balance
                        && after.signer_sequence == baseline.signer_sequence + 2
                        && after.recovery_balance == baseline.recovery_balance + 1,
                    "unbonded redelegation recovery committed state changed: before={baseline:?} after={after:?}"
                );
            }
            _ => anyhow::bail!("unbonded redelegation rollback completed too many phases"),
        }
        self.phase += 1;
        self.pending_tx = None;
        Ok(self.phase == 3)
    }

    fn require_source_preconditions(
        &self,
        rust: &mut app::App,
        state: &UnbondedRedelegationRollbackState,
    ) -> anyhow::Result<()> {
        let mut query = rust.create_query_context(0, false)?;
        let mut ctx = query.context();
        let validator = x_staking::store::validator(&mut ctx, &self.source_bytes)?;
        let delegation =
            x_staking::store::get_delegation(&mut ctx, &self.account_bytes, &self.source_bytes)?
                .context("unbonded redelegation self-delegation is absent")?;
        let destination = x_staking::store::validator(&mut ctx, &self.destination_bytes)?;
        let source_consensus = validator.cons_addr()?;
        let source_power = validator.potential_consensus_power()?;
        for (store, key, label) in [
            (
                x_staking::STORE_KEY,
                x_staking::keys::validator(&self.source_bytes),
                "source validator",
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::validator_by_cons_addr(&source_consensus),
                "source consensus index",
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::power_index(source_power, &self.source_bytes),
                "source power index",
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::delegation(&self.account_bytes, &self.source_bytes),
                "source delegation",
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::delegation_by_val(&self.source_bytes, &self.account_bytes),
                "source delegation index",
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::outstanding_key(&self.source_bytes),
                "source outstanding rewards",
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::current_key(&self.source_bytes),
                "source current rewards",
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::commission_key(&self.source_bytes),
                "source commission",
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::starting_info_key(&self.source_bytes, &self.account_bytes),
                "source delegator starting info",
            ),
        ] {
            anyhow::ensure!(
                ctx.get(store, &key)?.is_some(),
                "unbonded redelegation rollback precondition lacks {label}"
            );
        }
        anyhow::ensure!(
            !raw_prefix_entries(
                &mut ctx,
                x_distribution::STORE_KEY,
                &x_distribution::store::historical_prefix(&self.source_bytes),
            )?
            .is_empty(),
            "unbonded redelegation rollback precondition lacks source historical rewards"
        );
        anyhow::ensure!(
            validator.status == x_staking::types::BondStatus::Unbonded
                && !validator.jailed
                && validator.tokens.to_string() == NEVER_BONDED_SELF_DELEGATION.to_string()
                && delegation.shares.to_string() == "1000000.000000000000000000"
                && destination.status == x_staking::types::BondStatus::Bonded
                && x_slashing::get_signing_info(&mut ctx, &validator.cons_addr()?)?.is_none()
                && x_staking::store::get_red(
                    &mut ctx,
                    &self.account_bytes,
                    &self.source_bytes,
                    &self.destination_bytes
                )?
                .is_none()
                && !x_staking::store::has_receiving_redelegation(
                    &mut ctx,
                    &self.account_bytes,
                    &self.source_bytes
                )?,
            "unbonded redelegation rollback precondition changed: {state:?}"
        );
        Ok(())
    }

    fn state(&self, rust: &mut app::App) -> anyhow::Result<UnbondedRedelegationRollbackState> {
        let signer_sequence = rust.query_sequence(&self.account_bytes).sequence;
        let mut query = rust.create_query_context(0, false)?;
        let mut ctx = query.context();
        let source = x_staking::store::validator(&mut ctx, &self.source_bytes)?;
        let source_consensus = source.cons_addr()?;
        let source_power = source.potential_consensus_power()?;
        let destination = x_staking::store::validator(&mut ctx, &self.destination_bytes)?;
        let destination_power = destination.potential_consensus_power()?;
        let exact_keys = vec![
            x_staking::keys::validator(&self.source_bytes),
            x_staking::keys::validator_by_cons_addr(&source_consensus),
            x_staking::keys::power_index(source_power, &self.source_bytes),
            x_staking::keys::delegation(&self.account_bytes, &self.source_bytes),
            x_staking::keys::delegation_by_val(&self.source_bytes, &self.account_bytes),
            x_staking::keys::validator(&self.destination_bytes),
            x_staking::keys::power_index(destination_power, &self.destination_bytes),
            x_staking::keys::delegation(&self.account_bytes, &self.destination_bytes),
            x_staking::keys::delegation_by_val(&self.destination_bytes, &self.account_bytes),
            x_staking::keys::red(
                &self.account_bytes,
                &self.source_bytes,
                &self.destination_bytes,
            ),
            x_staking::keys::red_by_val_src(
                &self.account_bytes,
                &self.source_bytes,
                &self.destination_bytes,
            ),
            x_staking::keys::red_by_val_dst(
                &self.account_bytes,
                &self.source_bytes,
                &self.destination_bytes,
            ),
            x_staking::keys::UNBONDING_ID.to_vec(),
        ];
        let mut exact = Vec::new();
        for key in exact_keys {
            let value = ctx.get(x_staking::STORE_KEY, &key)?;
            exact.push((key, value));
        }
        for key in [
            x_distribution::store::outstanding_key(&self.source_bytes),
            x_distribution::store::commission_key(&self.source_bytes),
            x_distribution::store::current_key(&self.source_bytes),
            x_distribution::store::starting_info_key(&self.source_bytes, &self.account_bytes),
            x_distribution::store::user_outstanding_key(&self.account_bytes, &self.source_bytes),
            x_distribution::store::starting_info_key(&self.destination_bytes, &self.account_bytes),
            x_distribution::store::user_outstanding_key(
                &self.account_bytes,
                &self.destination_bytes,
            ),
        ] {
            let value = ctx.get(x_distribution::STORE_KEY, &key)?;
            exact.push((key, value));
        }
        let mut prefixes = Vec::new();
        for (store, prefix) in [
            (
                x_distribution::STORE_KEY,
                x_distribution::store::historical_prefix(&self.source_bytes),
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::slash_event_prefix(&self.source_bytes),
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::historical_prefix(&self.destination_bytes),
            ),
            (
                x_distribution::STORE_KEY,
                x_distribution::store::slash_event_prefix(&self.destination_bytes),
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::reds(&self.account_bytes),
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::reds_from_val_src(&self.source_bytes),
            ),
            (
                x_staking::STORE_KEY,
                x_staking::keys::reds_by_del_to_val_dst(
                    &self.account_bytes,
                    &self.destination_bytes,
                ),
            ),
            (
                x_staking::STORE_KEY,
                vec![x_staking::keys::REDELEGATION_QUEUE],
            ),
            (x_staking::STORE_KEY, vec![x_staking::keys::UNBONDING_QUEUE]),
            (x_staking::STORE_KEY, vec![x_staking::keys::UNBONDING_INDEX]),
            (x_staking::STORE_KEY, vec![x_staking::keys::UNBONDING_TYPE]),
        ] {
            prefixes.extend(raw_prefix_entries(&mut ctx, store, &prefix)?);
        }
        Ok(UnbondedRedelegationRollbackState {
            exact,
            prefixes,
            destination_current_period: x_distribution::store::get_current(
                &mut ctx,
                &self.destination_bytes,
            )?
            .period,
            bonded_pool: module_balance(&mut ctx, x_staking::BONDED_POOL)?,
            not_bonded_pool: module_balance(&mut ctx, x_staking::NOT_BONDED_POOL)?,
            signer_balance: bank_account_balance(&mut ctx, &self.account_bytes)?,
            signer_sequence,
            recovery_balance: account_balance_readonly(rust, &self.recovery)?,
        })
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.phase == 3 && self.pending_tx.is_none(),
            "unbonded redelegation rollback completed {} of three phases (pending_tx={})",
            self.phase,
            self.pending_tx.is_some()
        );
        Ok(())
    }
}

#[derive(Debug)]
struct NeverBondedUnjailState {
    status: x_staking::types::BondStatus,
    jailed: bool,
    tokens: String,
    shares: String,
    signing_info_present: bool,
    bonded_pool: u128,
    not_bonded_pool: u128,
    account_balance: u128,
}

struct NeverBondedUnjailFixture {
    operator: String,
    account: String,
    account_bytes: Vec<u8>,
    initial_balance: u128,
    initial_bonded_pool: u128,
    initial_not_bonded_pool: u128,
    phase: usize,
    pending_tx: Option<Vec<u8>>,
}

impl NeverBondedUnjailFixture {
    fn new(gn: &Gen, rust: &app::App) -> anyhow::Result<Self> {
        let account = gn.accts[0].acc.clone();
        let operator = gn.accts[0].val.clone();
        let account_bytes = x_staking::addr::acc_from_string(&account)?;
        let operator_bytes = x_staking::addr::val_from_string(&operator)?;
        let mut state = rust.create_query_context(0, false)?;
        let mut ctx = state.context();
        anyhow::ensure!(
            x_staking::store::get_params(&mut ctx)?.max_validators == 1,
            "never-bonded unjail fixture did not set max_validators=1"
        );
        anyhow::ensure!(
            x_staking::store::get_validator(&mut ctx, &operator_bytes)?.is_none(),
            "never-bonded unjail operator already exists"
        );
        anyhow::ensure!(
            bonded_validators_from_context(&mut ctx)?.len() == 1,
            "never-bonded unjail fixture needs exactly one active validator"
        );
        Ok(Self {
            initial_balance: account_balance_readonly(rust, &account)?,
            initial_bonded_pool: module_balance(&mut ctx, x_staking::BONDED_POOL)?
                .to_string()
                .parse()?,
            initial_not_bonded_pool: module_balance(&mut ctx, x_staking::NOT_BONDED_POOL)?
                .to_string()
                .parse()?,
            operator,
            account,
            account_bytes,
            phase: 0,
            pending_tx: None,
        })
    }

    fn tx(&mut self, gn: &Gen, phase: usize) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(phase == self.phase, "never-bonded unjail phase changed");
        let staking = |amount: u128| {
            Some(Coin {
                denom: "utia".into(),
                amount: amount.to_string(),
            })
        };
        let message = match phase {
            0 => Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator {
                    description: Some(celestia_proto::cosmos::staking::v1beta1::Description {
                        moniker: "never-bonded-unjail".into(),
                        ..Default::default()
                    }),
                    commission: Some(celestia_proto::cosmos::staking::v1beta1::CommissionRates {
                        rate: "100000000000000000".into(),
                        max_rate: "200000000000000000".into(),
                        max_change_rate: "10000000000000000".into(),
                    }),
                    min_self_delegation: NEVER_BONDED_SELF_DELEGATION.to_string(),
                    #[allow(deprecated)]
                    delegator_address: String::new(),
                    validator_address: self.operator.clone(),
                    pubkey: Some(Any::pack(
                        &celestia_proto::cosmos::crypto::ed25519::PubKey { key: vec![42; 32] },
                    )),
                    value: staking(NEVER_BONDED_SELF_DELEGATION),
                },
            ),
            1 => Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgUndelegate {
                delegator_address: self.account.clone(),
                validator_address: self.operator.clone(),
                amount: staking(1),
            }),
            2 => Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                delegator_address: self.account.clone(),
                validator_address: self.operator.clone(),
                amount: staking(1),
            }),
            3 => Any::pack(&celestia_proto::cosmos::slashing::v1beta1::MsgUnjail {
                validator_addr: self.operator.clone(),
            }),
            _ => anyhow::bail!("never-bonded unjail has no phase {phase}"),
        };
        let tx = sign_simple(gn, 0, vec![message], 1_000_000)
            .context("signing never-bonded unjail transaction")?;
        self.pending_tx = Some(tx.clone());
        Ok(tx)
    }

    fn tx_matches_phase(&self, tx: &[u8]) -> bool {
        self.pending_tx.as_deref() == Some(tx)
    }

    fn require_phase(
        &mut self,
        tx: &[u8],
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &app::App,
    ) -> anyhow::Result<()> {
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1 && matches[0].1.code == 0 && matches[0].1.codespace.is_empty(),
            "never-bonded unjail phase {} was not finalized exactly once with code 0",
            self.phase
        );
        let state = self.state(rust)?;
        let (jailed, tokens, shares, balance_debit, pool_credit) = match self.phase {
            0 => (
                false,
                1_000_000,
                1_000_000,
                NEVER_BONDED_SELF_DELEGATION + CONTROLLED_TX_FEE,
                1_000_000,
            ),
            1 => (
                true,
                999_999,
                999_999,
                NEVER_BONDED_SELF_DELEGATION + 2 * CONTROLLED_TX_FEE,
                1_000_000,
            ),
            2 => (
                true,
                1_000_000,
                1_000_000,
                NEVER_BONDED_SELF_DELEGATION + 3 * CONTROLLED_TX_FEE + 1,
                1_000_001,
            ),
            3 => (
                false,
                1_000_000,
                1_000_000,
                NEVER_BONDED_SELF_DELEGATION + 4 * CONTROLLED_TX_FEE + 1,
                1_000_001,
            ),
            _ => anyhow::bail!("never-bonded unjail completed too many phases"),
        };
        anyhow::ensure!(
            state.status == x_staking::types::BondStatus::Unbonded
                && state.jailed == jailed
                && state.tokens == tokens.to_string()
                && state.shares == format!("{shares}.000000000000000000")
                && !state.signing_info_present
                && state.bonded_pool == self.initial_bonded_pool
                && state.not_bonded_pool == self.initial_not_bonded_pool + pool_credit
                && state.account_balance == self.initial_balance - balance_debit,
            "never-bonded unjail phase {} committed state changed: {state:?}",
            self.phase
        );
        self.phase += 1;
        self.pending_tx = None;
        Ok(())
    }

    fn state(&self, rust: &app::App) -> anyhow::Result<NeverBondedUnjailState> {
        let operator = x_staking::addr::val_from_string(&self.operator)?;
        let mut state = rust.create_query_context(0, false)?;
        let mut ctx = state.context();
        let validator = x_staking::store::validator(&mut ctx, &operator)?;
        let consensus_address = validator.cons_addr()?;
        let delegation =
            x_staking::store::get_delegation(&mut ctx, &self.account_bytes, &operator)?
                .context("never-bonded self-delegation is absent")?;
        Ok(NeverBondedUnjailState {
            status: validator.status,
            jailed: validator.jailed,
            tokens: validator.tokens.to_string(),
            shares: delegation.shares.to_string(),
            signing_info_present: x_slashing::get_signing_info(&mut ctx, &consensus_address)?
                .is_some(),
            bonded_pool: module_balance(&mut ctx, x_staking::BONDED_POOL)?
                .to_string()
                .parse()?,
            not_bonded_pool: module_balance(&mut ctx, x_staking::NOT_BONDED_POOL)?
                .to_string()
                .parse()?,
            account_balance: account_balance_readonly(rust, &self.account)?,
        })
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.phase == 4 && self.pending_tx.is_none(),
            "never-bonded unjail fixture completed {} of four phases",
            self.phase
        );
        Ok(())
    }
}

const VALIDATOR_REMOVAL_DEPOSIT: u128 = 11;

#[derive(Debug, Clone)]
struct ValidatorRemovalAccounting {
    distribution_balance: Int,
    active_outstanding: sdk::dec_coins::DecCoins,
    community_pool: sdk::dec_coins::DecCoins,
    withdraw_balance: u128,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct ValidatorRemovalRewardsState {
    validator: Option<x_staking::types::Validator>,
    consensus_index_present: bool,
    power_index_present: bool,
    delegation: Option<x_staking::types::Delegation>,
    unbonding: Option<x_staking::types::UnbondingDelegation>,
    signing_info_present: bool,
    withdraw_address: Vec<u8>,
    outstanding: sdk::dec_coins::DecCoins,
    commission: sdk::dec_coins::DecCoins,
    current: x_distribution::store::CurrentRewards,
    outstanding_key_present: bool,
    commission_key_present: bool,
    current_key_present: bool,
    historical_keys: usize,
    slash_event_keys: usize,
    starting_info_present: bool,
    user_outstanding: Option<sdk::coins::Coins>,
    accounting: ValidatorRemovalAccounting,
    recovery_balance: u128,
    unbonding_time: pbjson_types::Duration,
}

struct ValidatorRemovalRewardsFixture {
    operator: String,
    operator_bytes: Vec<u8>,
    account: String,
    account_bytes: Vec<u8>,
    withdraw: String,
    withdraw_bytes: Vec<u8>,
    recovery: String,
    recovery_bytes: Vec<u8>,
    recovery_initial_balance: u128,
    active_operator_bytes: Vec<u8>,
    consensus_address: Option<Vec<u8>>,
    power_key: Option<Vec<u8>>,
    before_deposit: Option<ValidatorRemovalAccounting>,
    after_deposit: Option<ValidatorRemovalAccounting>,
    removal_block_time: Option<pbjson_types::Timestamp>,
    phase: usize,
    pending_tx: Option<Vec<u8>>,
}

impl ValidatorRemovalRewardsFixture {
    fn new(gn: &Gen, rust: &app::App) -> anyhow::Result<Self> {
        let account = gn.accts[0].acc.clone();
        let operator = gn.accts[0].val.clone();
        let withdraw = gn.accts[1].acc.clone();
        let recovery = gn.accts[2].acc.clone();
        let account_bytes = x_staking::addr::acc_from_string(&account)?;
        let operator_bytes = x_staking::addr::val_from_string(&operator)?;
        let withdraw_bytes = x_staking::addr::acc_from_string(&withdraw)?;
        let recovery_bytes = x_staking::addr::acc_from_string(&recovery)?;
        let mut state = rust.create_query_context(0, false)?;
        let mut ctx = state.context();
        anyhow::ensure!(
            x_staking::store::get_params(&mut ctx)?.max_validators == 1,
            "validator-removal rewards fixture did not set max_validators=1"
        );
        anyhow::ensure!(
            x_staking::store::get_validator(&mut ctx, &operator_bytes)?.is_none(),
            "validator-removal rewards operator already exists"
        );
        let bonded = x_staking::store::get_bonded_validators_by_power(&mut ctx)?;
        anyhow::ensure!(
            bonded.len() == 1 && x_staking::store::get_all_validators(&mut ctx)?.len() == 1,
            "validator-removal rewards fixture needs exactly one existing validator"
        );
        let active_operator_bytes = bonded[0].operator()?;
        let recovery_initial_balance = bank_account_balance(&mut ctx, &recovery_bytes)?;
        Ok(Self {
            operator,
            operator_bytes,
            account,
            account_bytes,
            withdraw,
            withdraw_bytes,
            recovery,
            recovery_bytes,
            recovery_initial_balance,
            active_operator_bytes,
            consensus_address: None,
            power_key: None,
            before_deposit: None,
            after_deposit: None,
            removal_block_time: None,
            phase: 0,
            pending_tx: None,
        })
    }

    fn tx(&mut self, gn: &Gen, phase: usize) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(
            phase == self.phase,
            "validator-removal rewards phase changed"
        );
        let staking = |amount: u128| {
            Some(Coin {
                denom: "utia".into(),
                amount: amount.to_string(),
            })
        };
        let messages = match phase {
            0 => vec![
                Any::pack(
                    &celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator {
                        description: Some(celestia_proto::cosmos::staking::v1beta1::Description {
                            moniker: "validator-removal-rewards".into(),
                            ..Default::default()
                        }),
                        commission: Some(
                            celestia_proto::cosmos::staking::v1beta1::CommissionRates {
                                rate: "100000000000000000".into(),
                                max_rate: "200000000000000000".into(),
                                max_change_rate: "10000000000000000".into(),
                            },
                        ),
                        min_self_delegation: NEVER_BONDED_SELF_DELEGATION.to_string(),
                        #[allow(deprecated)]
                        delegator_address: String::new(),
                        validator_address: self.operator.clone(),
                        pubkey: Some(Any::pack(
                            &celestia_proto::cosmos::crypto::ed25519::PubKey { key: vec![43; 32] },
                        )),
                        value: staking(NEVER_BONDED_SELF_DELEGATION),
                    },
                ),
                Any::pack(
                    &celestia_proto::cosmos::distribution::v1beta1::MsgSetWithdrawAddress {
                        delegator_address: self.account.clone(),
                        withdraw_address: self.withdraw.clone(),
                    },
                ),
            ],
            1 => {
                vec![Any::pack(
                &celestia_proto::cosmos::distribution::v1beta1::MsgDepositValidatorRewardsPool {
                    depositor: self.account.clone(),
                    validator_address: self.operator.clone(),
                    amount: vec![staking(VALIDATOR_REMOVAL_DEPOSIT)
                        .context("validator-removal deposit coin")?],
                },
            )]
            }
            2 => vec![Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgUndelegate {
                    delegator_address: self.account.clone(),
                    validator_address: self.operator.clone(),
                    amount: staking(NEVER_BONDED_SELF_DELEGATION),
                },
            )],
            3 => vec![Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                from_address: self.account.clone(),
                to_address: self.recovery.clone(),
                amount: vec![staking(1).context("validator-removal recovery coin")?],
            })],
            _ => anyhow::bail!("validator-removal rewards has no phase {phase}"),
        };
        let tx = sign_simple(gn, 0, messages, 1_000_000)
            .context("signing validator-removal rewards transaction")?;
        self.pending_tx = Some(tx.clone());
        Ok(tx)
    }

    fn tx_matches_phase(&self, tx: &[u8]) -> bool {
        self.pending_tx.as_deref() == Some(tx)
    }

    fn require_phase(
        &mut self,
        tx: &[u8],
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        block_time: &pbjson_types::Timestamp,
        rust: &app::App,
    ) -> anyhow::Result<()> {
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1 && matches[0].1.code == 0 && matches[0].1.codespace.is_empty(),
            "validator-removal rewards phase {} was not finalized exactly once with code 0",
            self.phase
        );
        let state = self.state(rust)?;
        match self.phase {
            0 => {
                let validator = state
                    .validator
                    .as_ref()
                    .context("validator-removal validator was not created")?;
                let delegation = state
                    .delegation
                    .as_ref()
                    .context("validator-removal self-delegation is absent")?;
                anyhow::ensure!(
                    validator.status == x_staking::types::BondStatus::Unbonded
                        && !validator.jailed
                        && validator.tokens.to_string() == "1000000"
                        && delegation.shares.to_string() == "1000000.000000000000000000"
                        && !state.signing_info_present
                        && state.consensus_index_present
                        && state.power_index_present
                        && state.withdraw_address == self.withdraw_bytes
                        && state.outstanding.is_zero()
                        && state.commission.is_zero()
                        && state.current.rewards.is_zero()
                        && state.outstanding_key_present
                        && state.commission_key_present
                        && state.current_key_present
                        && state.historical_keys > 0
                        && state.slash_event_keys == 0
                        && state.starting_info_present
                        && state.user_outstanding.is_none(),
                    "validator-removal creation state changed: {state:?}"
                );
                self.consensus_address = Some(validator.cons_addr()?);
                self.power_key = Some(x_staking::keys::power_index(
                    validator.potential_consensus_power()?,
                    &self.operator_bytes,
                ));
                self.before_deposit = Some(state.accounting);
            }
            1 => {
                self.require_live_validator(&state)?;
                anyhow::ensure!(
                    dec_amount_is(&state.outstanding, 11_000_000_000_000_000_000)?
                        && dec_amount_is(&state.commission, 1_100_000_000_000_000_000)?
                        && dec_amount_is(&state.current.rewards, 9_900_000_000_000_000_000)?
                        && state.outstanding_key_present
                        && state.commission_key_present
                        && state.current_key_present
                        && state.historical_keys > 0
                        && state.starting_info_present
                        && state.user_outstanding.is_none(),
                    "validator-removal deposited reward state changed: {state:?}"
                );
                let before = self
                    .before_deposit
                    .as_ref()
                    .context("validator-removal pre-deposit accounting is absent")?;
                anyhow::ensure!(
                    state.accounting.withdraw_balance == before.withdraw_balance,
                    "validator-removal deposit paid commission before validator removal"
                );
                self.after_deposit = Some(state.accounting);
            }
            2 => {
                self.removal_block_time = Some(*block_time);
                self.require_removed_validator(&state)?;
            }
            3 => {
                self.require_removed_validator(&state)?;
                anyhow::ensure!(
                    state.recovery_balance == self.recovery_initial_balance + 1,
                    "validator-removal signed recovery transfer did not persist"
                );
            }
            _ => anyhow::bail!("validator-removal rewards completed too many phases"),
        }
        self.phase += 1;
        self.pending_tx = None;
        Ok(())
    }

    fn require_live_validator(&self, state: &ValidatorRemovalRewardsState) -> anyhow::Result<()> {
        let validator = state
            .validator
            .as_ref()
            .context("validator-removal validator disappeared before full undelegation")?;
        let delegation = state
            .delegation
            .as_ref()
            .context("validator-removal self-delegation disappeared before removal")?;
        anyhow::ensure!(
            validator.status == x_staking::types::BondStatus::Unbonded
                && validator.tokens.to_string() == "1000000"
                && delegation.shares.to_string() == "1000000.000000000000000000"
                && !state.signing_info_present
                && state.consensus_index_present
                && state.power_index_present
                && state.withdraw_address == self.withdraw_bytes,
            "validator-removal live validator state changed: {state:?}"
        );
        Ok(())
    }

    fn require_removed_validator(
        &self,
        state: &ValidatorRemovalRewardsState,
    ) -> anyhow::Result<()> {
        let unbonding = state
            .unbonding
            .as_ref()
            .context("validator-removal unbonding delegation is absent")?;
        anyhow::ensure!(
            unbonding.entries.len() == 1,
            "validator-removal expected exactly one unbonding entry"
        );
        let entry = &unbonding.entries[0];
        anyhow::ensure!(
            entry.initial_balance.to_string() == "1000000"
                && entry.balance.to_string() == "1000000"
                && entry.completion_time
                    == sdk::time::add_duration(
                        self.removal_block_time
                            .as_ref()
                            .context("validator-removal block time is absent")?,
                        &state.unbonding_time
                    ),
            "validator-removal unbonding entry changed: {entry:?}; header {:?}; unbonding {:?}; expected {:?}",
            self.removal_block_time,
            state.unbonding_time,
            sdk::time::add_duration(
                self.removal_block_time
                    .as_ref()
                    .context("validator-removal block time is absent")?,
                &state.unbonding_time
            )
        );
        let user = state
            .user_outstanding
            .as_ref()
            .context("validator-removal retained user rewards are absent")?;
        anyhow::ensure!(
            state.validator.is_none()
                && state.delegation.is_none()
                && !state.consensus_index_present
                && !state.power_index_present
                && !state.signing_info_present
                && state.withdraw_address == self.withdraw_bytes
                && state.outstanding.is_zero()
                && state.commission.is_zero()
                && state.current.rewards.is_zero()
                && !state.outstanding_key_present
                && !state.commission_key_present
                && !state.current_key_present
                && state.historical_keys == 0
                && state.slash_event_keys == 0
                && !state.starting_info_present
                && user.amount_of("utia").to_string() == "9",
            "validator-removal committed cleanup state changed: {state:?}"
        );
        let before = self
            .before_deposit
            .as_ref()
            .context("validator-removal pre-deposit accounting is absent")?;
        anyhow::ensure!(
            state.accounting.withdraw_balance == before.withdraw_balance + 1,
            "validator-removal integral commission payout changed"
        );
        if self.phase == 2 {
            let deposited = self
                .after_deposit
                .as_ref()
                .context("validator-removal post-deposit accounting is absent")?;
            let active_delta = state
                .accounting
                .active_outstanding
                .sub(&deposited.active_outstanding)?;
            let pool_delta = state
                .accounting
                .community_pool
                .sub(&deposited.community_pool)?;
            anyhow::ensure!(
                dec_amount_is(&pool_delta, 1_000_000_000_000_000_000)?,
                "validator-removal community-pool dust changed: {pool_delta}"
            );
            let lhs = dec_from_int(&state.accounting.distribution_balance)?
                .add(&dec_from_u128(VALIDATOR_REMOVAL_DEPOSIT)?)?;
            let rhs = dec_from_int(&deposited.distribution_balance)?
                .add(&active_delta)?
                .add(&pool_delta)?
                .add(&dec_from_u128(9)?)?;
            anyhow::ensure!(
                lhs == rhs,
                "validator-removal distribution module conservation changed: left {lhs}, right {rhs}"
            );
        }
        Ok(())
    }

    fn state(&self, rust: &app::App) -> anyhow::Result<ValidatorRemovalRewardsState> {
        let mut state = rust.create_query_context(0, false)?;
        let mut ctx = state.context();
        let validator = x_staking::store::get_validator(&mut ctx, &self.operator_bytes)?;
        let current_consensus = validator
            .as_ref()
            .map(x_staking::types::Validator::cons_addr)
            .transpose()?;
        let consensus = self
            .consensus_address
            .as_deref()
            .or(current_consensus.as_deref());
        let consensus_index_present = match consensus {
            Some(consensus) => {
                x_staking::store::get_validator_by_cons_addr(&mut ctx, consensus)?.is_some()
            }
            None => false,
        };
        let current_power_key = validator
            .as_ref()
            .map(|current| {
                Ok::<_, sdk::error::SdkError>(x_staking::keys::power_index(
                    current.potential_consensus_power()?,
                    &self.operator_bytes,
                ))
            })
            .transpose()?;
        let power_key = self.power_key.as_deref().or(current_power_key.as_deref());
        let power_index_present = match power_key {
            Some(key) => ctx.get(x_staking::STORE_KEY, key)?.is_some(),
            None => false,
        };
        let signing_info_present = if let Some(consensus) = consensus {
            x_slashing::get_signing_info(&mut ctx, consensus)?.is_some()
        } else {
            false
        };
        let outstanding_key = x_distribution::store::outstanding_key(&self.operator_bytes);
        let commission_key = x_distribution::store::commission_key(&self.operator_bytes);
        let current_key = x_distribution::store::current_key(&self.operator_bytes);
        let accounting = ValidatorRemovalAccounting {
            distribution_balance: module_balance(&mut ctx, x_distribution::MODULE_NAME)?,
            active_outstanding: x_distribution::store::get_outstanding(
                &mut ctx,
                &self.active_operator_bytes,
            )?,
            community_pool: x_distribution::store::get_fee_pool(&mut ctx)?,
            withdraw_balance: bank_account_balance(&mut ctx, &self.withdraw_bytes)?,
        };
        Ok(ValidatorRemovalRewardsState {
            validator,
            consensus_index_present,
            power_index_present,
            delegation: x_staking::store::get_delegation(
                &mut ctx,
                &self.account_bytes,
                &self.operator_bytes,
            )?,
            unbonding: x_staking::store::get_ubd(
                &mut ctx,
                &self.account_bytes,
                &self.operator_bytes,
            )?,
            signing_info_present,
            withdraw_address: x_distribution::store::get_withdraw_addr(
                &mut ctx,
                &self.account_bytes,
            )?,
            outstanding: x_distribution::store::get_outstanding(&mut ctx, &self.operator_bytes)?,
            commission: x_distribution::store::get_commission(&mut ctx, &self.operator_bytes)?,
            current: x_distribution::store::get_current(&mut ctx, &self.operator_bytes)?,
            outstanding_key_present: ctx
                .get(x_distribution::STORE_KEY, &outstanding_key)?
                .is_some(),
            commission_key_present: ctx
                .get(x_distribution::STORE_KEY, &commission_key)?
                .is_some(),
            current_key_present: ctx.get(x_distribution::STORE_KEY, &current_key)?.is_some(),
            historical_keys: prefix_len(
                &mut ctx,
                x_distribution::STORE_KEY,
                &x_distribution::store::historical_prefix(&self.operator_bytes),
            )?,
            slash_event_keys: prefix_len(
                &mut ctx,
                x_distribution::STORE_KEY,
                &x_distribution::store::slash_event_prefix(&self.operator_bytes),
            )?,
            starting_info_present: x_distribution::store::has_starting_info(
                &mut ctx,
                &self.operator_bytes,
                &self.account_bytes,
            )?,
            user_outstanding: x_distribution::store::get_user_outstanding(
                &mut ctx,
                &self.account_bytes,
                &self.operator_bytes,
            )?,
            accounting,
            recovery_balance: bank_account_balance(&mut ctx, &self.recovery_bytes)?,
            unbonding_time: x_staking::store::get_params(&mut ctx)?.unbonding_time,
        })
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.phase == 4 && self.pending_tx.is_none(),
            "validator-removal rewards fixture completed {} of four phases",
            self.phase
        );
        Ok(())
    }
}

fn bank_account_balance(ctx: &mut sdk::Context<'_>, address: &[u8]) -> anyhow::Result<u128> {
    Ok(
        x_bank::get_balance(ctx, address, x_staking::DEFAULT_BOND_DENOM)?
            .amount
            .to_string()
            .parse()?,
    )
}

fn prefix_len(ctx: &mut sdk::Context<'_>, store: &str, prefix: &[u8]) -> anyhow::Result<usize> {
    let mut iter = ctx.prefix_iter(store, prefix)?;
    let mut count = 0;
    while iter.next(ctx)?.is_some() {
        count += 1;
    }
    Ok(count)
}

fn raw_prefix_entries(
    ctx: &mut sdk::Context<'_>,
    store: &str,
    prefix: &[u8],
) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut iter = ctx.prefix_iter(store, prefix)?;
    let mut entries = Vec::new();
    while let Some(entry) = iter.next(ctx)? {
        entries.push(entry);
    }
    Ok(entries)
}

fn dec_amount_is(coins: &sdk::dec_coins::DecCoins, atomics: i128) -> anyhow::Result<bool> {
    Ok(coins.0.len() == 1
        && coins.0[0].denom == x_staking::DEFAULT_BOND_DENOM
        && coins.0[0].amount == LegacyDec::unmarshal(atomics.to_string().as_bytes())?)
}

fn dec_from_int(value: &Int) -> anyhow::Result<sdk::dec_coins::DecCoins> {
    sdk::dec_coins::DecCoins::from_coins(&sdk::coins::Coins(vec![sdk::coins::Coin::with_amount(
        x_staking::DEFAULT_BOND_DENOM,
        value.clone(),
    )]))
    .map_err(Into::into)
}

fn dec_from_u128(value: u128) -> anyhow::Result<sdk::dec_coins::DecCoins> {
    dec_from_int(&Int::from(u64::try_from(value)?))
}

fn bonded_validators_from_context(ctx: &mut sdk::Context<'_>) -> anyhow::Result<Vec<String>> {
    Ok(x_staking::store::get_bonded_validators_by_power(ctx)?
        .into_iter()
        .map(|validator| validator.operator_address)
        .collect())
}

fn account_balance_readonly(rust: &app::App, address: &str) -> anyhow::Result<u128> {
    let mut state = rust.create_query_context(0, false)?;
    let mut ctx = state.context();
    Ok(
        x_bank::get_balance(&mut ctx, &x_auth::parse_bech32(address)?, "utia")?
            .amount
            .to_string()
            .parse()?,
    )
}

const DIRECT_AUX_SEND: u128 = 123;
const DIRECT_AUX_FEE: u128 = 100_000;

const MALFORMED_MULTISIG_SEND: u128 = 1;
const MALFORMED_MULTISIG_FEE: u128 = 100_000;

struct MalformedMultisigCase {
    name: &'static str,
    expected_log: String,
    check_gas_used: i64,
    finalize_gas_used: i64,
    tx: Vec<u8>,
    signer: String,
    signer_addr: Vec<u8>,
    signer_balance: u128,
    signer_sequence: u64,
    check: Option<abci::ResponseCheckTx>,
    prepare_excluded: bool,
    process_rejected: bool,
    completed: bool,
}

struct MalformedMultisigFixture {
    cases: Vec<MalformedMultisigCase>,
    recipient: String,
    recipient_balance: u128,
}

impl MalformedMultisigFixture {
    fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let recipient = gn.accts[ACCOUNTS - 1].acc.clone();
        let specs = [
            (
                0,
                "single-key-multi-data",
                "expected <nil>, got *secp256k1.PubKey",
            ),
            (
                ACCOUNTS,
                "too-few-set-bits",
                "not enough signatures set, have 1, expected 2",
            ),
            (
                ACCOUNTS + 1,
                "nested-data-for-single-key",
                "unable to parse pubkey of index 0",
            ),
        ];
        let mut cases = Vec::new();
        let gas = [(32_640, 32_589), (38_229, 38_229), (35_829, 35_829)];
        for (shape, (signer, name, marker)) in specs.into_iter().enumerate() {
            let account = &gn.accts[signer];
            cases.push(MalformedMultisigCase {
                name,
                expected_log: format!(
                    "signature verification failed; please verify account number ({}) and chain-id ({}): ({marker}): unauthorized",
                    account.number, gn.chain_id
                ),
                check_gas_used: gas[shape].0,
                finalize_gas_used: gas[shape].1,
                tx: sign_malformed_multisig_send(gn, shape, signer)?,
                signer: account.acc.clone(),
                signer_addr: account.addr.clone(),
                signer_balance: account_balance(rust, &account.acc)?,
                signer_sequence: rust.query_sequence(&account.addr).sequence,
                check: None,
                prepare_excluded: false,
                process_rejected: false,
                completed: false,
            });
        }
        Ok(Self {
            recipient_balance: account_balance(rust, &recipient)?,
            recipient,
            cases,
        })
    }

    fn case_for_tx_mut(&mut self, tx: &[u8]) -> Option<&mut MalformedMultisigCase> {
        self.cases.iter_mut().find(|case| case.tx == tx)
    }

    fn require_complete(
        &mut self,
        tx: &[u8],
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        let recipient = self.recipient.clone();
        let recipient_balance = self.recipient_balance;
        let case = self
            .case_for_tx_mut(tx)
            .context("unknown malformed multisig fixture transaction")?;
        let check = case
            .check
            .as_ref()
            .context("malformed multisig transaction was not checked")?;
        anyhow::ensure!(
            check.codespace == "sdk"
                && check.code == 4
                && check.log == case.expected_log
                && check.gas_wanted == 300_000
                && check.gas_used == case.check_gas_used
                && check.data.is_empty()
                && check.info.is_empty()
                && check.events.is_empty()
                && check.address.is_empty()
                && check.priority == 0
                && check.sequence == 0,
            "malformed multisig {} CheckTx changed: {}/{} gas {}/{} {}",
            case.name,
            check.codespace,
            check.code,
            check.gas_wanted,
            check.gas_used,
            check.log
        );
        anyhow::ensure!(
            case.prepare_excluded,
            "malformed multisig {} was not excluded by PrepareProposal",
            case.name
        );
        anyhow::ensure!(
            case.process_rejected,
            "malformed multisig {} raw proposal was not rejected by ProcessProposal",
            case.name
        );
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "malformed multisig {} did not finalize exactly once: {} matches",
            case.name,
            matches.len()
        );
        let result = matches[0].1;
        anyhow::ensure!(
            result.codespace == check.codespace
                && result.code == check.code
                && result.log == case.expected_log
                && result.gas_wanted == 300_000
                && result.gas_used == case.finalize_gas_used
                && result.data.is_empty()
                && result.info.is_empty()
                && result.events.is_empty()
                && result.signers == [case.signer.clone()],
            "malformed multisig {} FinalizeBlock result differs from CheckTx: check={check:?}, final={result:?}",
            case.name
        );
        anyhow::ensure!(
            account_balance(rust, &case.signer)? == case.signer_balance
                && rust.query_sequence(&case.signer_addr).sequence == case.signer_sequence
                && account_balance(rust, &recipient)? == recipient_balance,
            "malformed multisig {} changed a signer/recipient balance or signer sequence",
            case.name
        );
        anyhow::ensure!(
            !case.completed,
            "malformed multisig {} completed twice",
            case.name
        );
        case.completed = true;
        Ok(())
    }
}

fn sign_malformed_multisig_send(gn: &Gen, shape: usize, signer: usize) -> anyhow::Result<Vec<u8>> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature;

    let account = &gn.accts[signer];
    let body_bytes = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: account.acc.clone(),
            to_address: gn.accts[ACCOUNTS - 1].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: MALFORMED_MULTISIG_SEND.to_string(),
            }],
        })],
        ..Default::default()
    }
    .encode_to_vec();
    let single = single_mode_info(SignMode::Direct);
    let mode_info = match shape {
        0 => ModeInfo {
            sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                bitarray: Some(bit_array(1, &[0])),
                mode_infos: vec![single.clone()],
            })),
        },
        1 => ModeInfo {
            sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                bitarray: Some(bit_array(3, &[0])),
                mode_infos: vec![single.clone(), single.clone()],
            })),
        },
        2 => ModeInfo {
            sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                bitarray: Some(bit_array(2, &[0])),
                mode_infos: vec![ModeInfo {
                    sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                        bitarray: Some(bit_array(1, &[0])),
                        mode_infos: vec![single],
                    })),
                }],
            })),
        },
        _ => anyhow::bail!("unknown malformed multisig shape {shape}"),
    };
    #[allow(deprecated)]
    let auth_info_bytes = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(account.pk.clone()),
            mode_info: Some(mode_info),
            sequence: account.seq,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: MALFORMED_MULTISIG_FEE.to_string(),
            }],
            gas_limit: 300_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec();
    let direct = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: gn.chain_id.clone(),
        account_number: account.number,
    }
    .encode_to_vec();
    let signatures = match shape {
        0 => vec![gn.accts[0].key.sign(&direct)?.to_vec()],
        1 => vec![
            gn.accts[0].key.sign(&direct)?.to_vec(),
            gn.accts[1].key.sign(&direct)?.to_vec(),
        ],
        2 => vec![
            MultiSignature {
                signatures: vec![gn.accts[3].key.sign(&direct)?.to_vec()],
            }
            .encode_to_vec(),
        ],
        _ => unreachable!(),
    };
    Ok(TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![MultiSignature { signatures }.encode_to_vec()],
    }
    .encode_to_vec())
}

const ZERO_THRESHOLD_MULTISIG_SEND: u128 = 17;
const ZERO_THRESHOLD_MULTISIG_FEE: u128 = 100_000;

struct ZeroThresholdMultisigFixture {
    txs: [Vec<u8>; 2],
    check: [Option<abci::ResponseCheckTx>; 2],
    prepare_included: [bool; 2],
    process_accepted: [bool; 2],
    completed: [bool; 2],
    signer: String,
    signer_addr: Vec<u8>,
    signer_balance: u128,
    signer_sequence: u64,
    recipient: String,
    recipient_balance: u128,
    public_key: Any,
}

impl ZeroThresholdMultisigFixture {
    fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let signer = gn
            .accts
            .last()
            .context("zero-threshold multisig account was not added to genesis")?;
        let recipient = gn.accts[2].acc.clone();
        anyhow::ensure!(
            signer.ms.as_ref().is_some_and(|tree| matches!(
                tree,
                MsKey::Multi {
                    threshold: 0,
                    members
                } if members.len() == 1
            )),
            "controlled zero-threshold account has the wrong key tree"
        );
        anyhow::ensure!(
            account_public_key(rust, &signer.acc)?.is_none(),
            "controlled zero-threshold account unexpectedly has a genesis public key"
        );
        Ok(Self {
            txs: [
                zero_threshold_multisig_send(gn, signer, signer.seq),
                zero_threshold_multisig_send(gn, signer, signer.seq + 1),
            ],
            check: [None, None],
            prepare_included: [false; 2],
            process_accepted: [false; 2],
            completed: [false; 2],
            signer: signer.acc.clone(),
            signer_addr: signer.addr.clone(),
            signer_balance: account_balance(rust, &signer.acc)?,
            signer_sequence: signer.seq,
            recipient_balance: account_balance(rust, &recipient)?,
            recipient,
            public_key: signer.pk.clone(),
        })
    }

    fn tx_index(&self, tx: &[u8]) -> Option<usize> {
        self.txs.iter().position(|candidate| candidate == tx)
    }

    fn require_complete(
        &mut self,
        index: usize,
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        let tx = &self.txs[index];
        let check = self.check[index]
            .as_ref()
            .context("zero-threshold multisig transaction was not checked")?;
        anyhow::ensure!(
            check.codespace.is_empty()
                && check.code == 0
                && check.log.is_empty()
                && check.gas_wanted == 300_000
                && check.gas_used == [64_938, 50_650][index]
                && check.data.is_empty()
                && check.info.is_empty()
                && check.address == self.signer_addr
                && check.priority == 333_333
                && check.sequence == index as u64,
            "zero-threshold multisig CheckTx {} changed: {check:?}",
            index + 1
        );
        anyhow::ensure!(
            self.prepare_included[index],
            "zero-threshold multisig transaction {} was excluded by PrepareProposal",
            index + 1
        );
        anyhow::ensure!(
            self.process_accepted[index],
            "zero-threshold multisig transaction {} was not accepted by ProcessProposal",
            index + 1
        );
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "zero-threshold multisig transaction {} finalized {} times",
            index + 1,
            matches.len()
        );
        let result = matches[0].1;
        anyhow::ensure!(
            result.codespace.is_empty()
                && result.code == 0
                && result.log.is_empty()
                && result.gas_wanted == 300_000
                && result.gas_used == [83_879, 69_624][index]
                && result.data
                    == hex::decode(
                        "12260a242f636f736d6f732e62616e6b2e763162657461312e4d736753656e64526573706f6e7365"
                    )?
                && result.info.is_empty()
                && result.signers == [self.signer.clone()],
            "zero-threshold multisig FinalizeBlock result {} changed: {result:?}",
            index + 1
        );
        let completed = (index + 1) as u128;
        anyhow::ensure!(
            account_balance(rust, &self.signer)?
                == self.signer_balance
                    - completed * (ZERO_THRESHOLD_MULTISIG_SEND + ZERO_THRESHOLD_MULTISIG_FEE)
                && account_balance(rust, &self.recipient)?
                    == self.recipient_balance + completed * ZERO_THRESHOLD_MULTISIG_SEND
                && rust.query_sequence(&self.signer_addr).sequence
                    == self.signer_sequence + index as u64 + 1,
            "zero-threshold multisig transaction {} has wrong committed balances or sequence",
            index + 1
        );
        anyhow::ensure!(
            account_public_key(rust, &self.signer)?.as_ref() == Some(&self.public_key),
            "zero-threshold multisig transaction {} did not preserve the exact account public key",
            index + 1
        );
        anyhow::ensure!(
            !self.completed[index],
            "zero-threshold multisig transaction {} completed twice",
            index + 1
        );
        self.completed[index] = true;
        Ok(())
    }
}

fn zero_threshold_multisig_send(gn: &Gen, signer: &Acct, sequence: u64) -> Vec<u8> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature;

    let body_bytes = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: signer.acc.clone(),
            to_address: gn.accts[2].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: ZERO_THRESHOLD_MULTISIG_SEND.to_string(),
            }],
        })],
        ..Default::default()
    }
    .encode_to_vec();
    #[allow(deprecated)]
    let auth_info_bytes = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(signer.pk.clone()),
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                    bitarray: Some(bit_array(1, &[])),
                    mode_infos: Vec::new(),
                })),
            }),
            sequence,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: ZERO_THRESHOLD_MULTISIG_FEE.to_string(),
            }],
            gas_limit: 300_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec();
    TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![MultiSignature::default().encode_to_vec()],
    }
    .encode_to_vec()
}

fn account_public_key(rust: &mut app::App, address: &str) -> anyhow::Result<Option<Any>> {
    use celestia_proto::cosmos::auth::v1beta1 as auth;
    let response = query_app::<auth::QueryAccountResponse>(
        rust,
        "/cosmos.auth.v1beta1.Query/Account",
        &auth::QueryAccountRequest {
            address: address.into(),
        },
    )
    .context("querying zero-threshold multisig account")?;
    let account = response
        .account
        .context("zero-threshold multisig account query returned no account")?;
    anyhow::ensure!(
        account.type_url == auth::BaseAccount::type_url(),
        "zero-threshold multisig account has unexpected type {}",
        account.type_url
    );
    Ok(auth::BaseAccount::decode(account.value.as_slice())?.pub_key)
}

const PUBKEY_REJECTION_SEND: u128 = 23;
const PUBKEY_REJECTION_FEE: u128 = 100_000;

struct PubkeyRejectionFixture {
    txs: [Vec<u8>; 6],
    check: [Option<abci::ResponseCheckTx>; 6],
    rust_check: [Option<abci::ResponseCheckTx>; 6],
    prepare_included: [bool; 6],
    process_status: [Option<i32>; 6],
    completed: [bool; 6],
    signers: [String; 3],
    signer_addrs: [Vec<u8>; 3],
    signer_balances: [u128; 3],
    signer_sequences: [u64; 3],
    public_keys: [Any; 3],
    recipient: String,
    recipient_balance: u128,
}

impl PubkeyRejectionFixture {
    fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let base = gn
            .accts
            .len()
            .checked_sub(3)
            .context("controlled public-key accounts were not added to genesis")?;
        let controlled = [&gn.accts[base], &gn.accts[base + 1], &gn.accts[base + 2]];
        anyhow::ensure!(
            controlled[0].pk.type_url == x_ante::pubkey::SECP256K1_TYPE_URL
                && controlled[1].pk.type_url == x_ante::pubkey::SECP256K1_TYPE_URL
                && controlled[2].pk.type_url == x_ante::pubkey::ED25519_TYPE_URL,
            "controlled public-key accounts have unexpected key types"
        );
        for account in controlled {
            anyhow::ensure!(
                account_public_key(rust, &account.acc)?.is_none(),
                "controlled public-key account unexpectedly has a genesis public key"
            );
        }
        let txs = pubkey_rejection_txs(gn, base)?;
        let recipient = gn.accts[2].acc.clone();
        let signer_balances = [
            account_balance(rust, &controlled[0].acc)?,
            account_balance(rust, &controlled[1].acc)?,
            account_balance(rust, &controlled[2].acc)?,
        ];
        Ok(Self {
            txs,
            check: [None, None, None, None, None, None],
            rust_check: [None, None, None, None, None, None],
            prepare_included: [false; 6],
            process_status: [None; 6],
            completed: [false; 6],
            signers: std::array::from_fn(|i| controlled[i].acc.clone()),
            signer_addrs: std::array::from_fn(|i| controlled[i].addr.clone()),
            signer_balances,
            signer_sequences: std::array::from_fn(|i| {
                rust.query_sequence(&controlled[i].addr).sequence
            }),
            public_keys: std::array::from_fn(|i| controlled[i].pk.clone()),
            recipient_balance: account_balance(rust, &recipient)?,
            recipient,
        })
    }

    fn tx_index(&self, tx: &[u8]) -> Option<usize> {
        self.txs.iter().position(|candidate| candidate == tx)
    }

    fn require_complete(
        &mut self,
        index: usize,
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        let check = self.check[index]
            .as_ref()
            .context("public-key fixture transaction was not checked")?;
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == self.txs[index])
            .collect();
        let account = index / 2;
        let accepted = abci::response_process_proposal::ProposalStatus::Accept as i32;
        let rejected = abci::response_process_proposal::ProposalStatus::Reject as i32;
        match index {
            0 | 2 => {
                let marker =
                    ["length of pubkey is incorrect", "pubkey is incorrect size"][index / 2];
                let rust_check = self.rust_check[index]
                    .as_ref()
                    .context("Rust public-key fixture transaction was not checked")?;
                let expected_gas = 1_180;
                anyhow::ensure!(
                    check.codespace == "undefined"
                        && check.code == 111_222
                        && check
                            .log
                            .starts_with(&format!("recovered: {marker}\ncaused by transaction:\n"))
                        && check.gas_wanted == 0
                        && check.gas_used == expected_gas
                        && check.data.is_empty()
                        && check.info.is_empty()
                        && check.events.is_empty()
                        && check.address.is_empty()
                        && check.priority == 0
                        && check.sequence == 0,
                    "malformed public-key CheckTx {} changed: {check:?}",
                    index + 1
                );
                anyhow::ensure!(
                    exact_rust_malformed_pubkey_check(rust_check, marker),
                    "Rust malformed public-key CheckTx {} changed: {rust_check:?}",
                    index + 1
                );
                anyhow::ensure!(
                    self.prepare_included[index]
                        && self.process_status[index] == Some(rejected)
                        && !finalized.contains(&self.txs[index])
                        && matches.is_empty(),
                    "malformed public-key transaction {} did not preserve exact proposal rejection behavior",
                    index + 1
                );
            }
            1 | 3 => {
                anyhow::ensure!(
                    matches.len() == 1,
                    "public-key recovery transaction {} finalized {} times",
                    index + 1,
                    matches.len()
                );
                let result = matches[0].1;
                anyhow::ensure!(
                    check.codespace.is_empty()
                        && check.code == 0
                        && check.log.is_empty()
                        && check.gas_wanted == 200_000
                        && check.gas_used == 63_773
                        && check.data.is_empty()
                        && check.info.is_empty()
                        && check.address == self.signer_addrs[account]
                        && check.priority == 500_000
                        && check.sequence == 0,
                    "public-key recovery CheckTx {} changed: {check:?}",
                    index + 1
                );
                anyhow::ensure!(
                    self.prepare_included[index] && self.process_status[index] == Some(accepted),
                    "public-key recovery transaction {} was not proposed and accepted",
                    index + 1
                );
                anyhow::ensure!(
                    result.codespace.is_empty()
                        && result.code == 0
                        && result.log.is_empty()
                        && result.gas_wanted == 200_000
                        && result.gas_used == 82_621
                        && result.data
                            == hex::decode(
                                "12260a242f636f736d6f732e62616e6b2e763162657461312e4d736753656e64526573706f6e7365"
                            )?
                        && result.info.is_empty()
                        && result.signers == [self.signers[account].clone()],
                    "public-key recovery FinalizeBlock {} changed: {result:?}",
                    index + 1
                );
            }
            4 | 5 => {
                anyhow::ensure!(
                    matches.len() == 1,
                    "Ed25519 transaction {} finalized {} times",
                    index + 1,
                    matches.len()
                );
                let result = matches[0].1;
                let expected = "ED25519 public keys are unsupported: invalid pubkey";
                let expected_check_gas = [44_909, 44_891][index - 4];
                anyhow::ensure!(
                    check.codespace == "sdk"
                        && check.code == 8
                        && check.log == expected
                        && check.gas_wanted == 200_000
                        && check.gas_used == expected_check_gas
                        && check.data.is_empty()
                        && check.info.is_empty()
                        && check.events.is_empty()
                        && check.address.is_empty()
                        && check.priority == 0
                        && check.sequence == 0,
                    "Ed25519 CheckTx {} changed: {check:?}",
                    index + 1
                );
                anyhow::ensure!(
                    !self.prepare_included[index] && self.process_status[index] == Some(rejected),
                    "Ed25519 transaction {} was not excluded and rejected",
                    index + 1
                );
                anyhow::ensure!(
                    result.codespace == check.codespace
                        && result.code == check.code
                        && result.log == expected
                        && result.gas_wanted == 200_000
                        && result.gas_used == 44_891
                        && result.data.is_empty()
                        && result.info.is_empty()
                        && result.events.is_empty()
                        && result.signers == [self.signers[account].clone()],
                    "Ed25519 FinalizeBlock {} changed: {result:?}",
                    index + 1
                );
            }
            _ => anyhow::bail!("unknown public-key fixture index {index}"),
        }
        let recovery_amounts = [PUBKEY_REJECTION_SEND + 1, PUBKEY_REJECTION_SEND + 3];
        for signer in 0..3 {
            let recovered = (signer == 0 && index >= 1) || (signer == 1 && index >= 3);
            let expected_balance = self.signer_balances[signer]
                - u128::from(recovered) * (recovery_amounts[signer.min(1)] + PUBKEY_REJECTION_FEE);
            anyhow::ensure!(
                account_balance(rust, &self.signers[signer])? == expected_balance
                    && rust.query_sequence(&self.signer_addrs[signer]).sequence
                        == self.signer_sequences[signer] + u64::from(recovered),
                "public-key fixture {} changed signer {} balance or sequence",
                index + 1,
                signer
            );
            let expected_key = recovered.then_some(&self.public_keys[signer]);
            anyhow::ensure!(
                account_public_key(rust, &self.signers[signer])?.as_ref() == expected_key,
                "public-key fixture {} changed signer {} public key",
                index + 1,
                signer
            );
        }
        let recovered_amount = recovery_amounts[0] * u128::from(index >= 1)
            + recovery_amounts[1] * u128::from(index >= 3);
        anyhow::ensure!(
            account_balance(rust, &self.recipient)? == self.recipient_balance + recovered_amount,
            "public-key fixture {} changed the recipient balance",
            index + 1
        );
        anyhow::ensure!(!self.completed[index], "public-key fixture completed twice");
        self.completed[index] = true;
        Ok(())
    }
}

fn exact_rust_malformed_pubkey_check(check: &abci::ResponseCheckTx, marker: &str) -> bool {
    check.codespace == "undefined"
        && check.code == 111_222
        && check.log == format!("recovered: {marker}: panic")
        && check.gas_wanted == 0
        && check.gas_used == 1_180
        && check.data.is_empty()
        && check.info.is_empty()
        && check.events.is_empty()
        && check.address.is_empty()
        && check.priority == 0
        && check.sequence == 0
}

fn pubkey_rejection_txs(gn: &Gen, base: usize) -> anyhow::Result<[Vec<u8>; 6]> {
    let malformed_secp = x_ante::pubkey::PubKey::Secp256k1(vec![2; 32]).to_any();
    let malformed_ed = x_ante::pubkey::PubKey::Ed25519(vec![3; 31]).to_any();
    let specs = [
        (base, malformed_secp, false),
        (base, gn.accts[base].pk.clone(), true),
        (base + 1, malformed_ed, false),
        (base + 1, gn.accts[base + 1].pk.clone(), true),
        (base + 2, gn.accts[base + 2].pk.clone(), false),
        (base + 2, gn.accts[base + 2].pk.clone(), false),
    ];
    let txs = specs
        .into_iter()
        .enumerate()
        .map(|(index, (signer, public_key, valid_signature))| {
            pubkey_rejection_send(
                gn,
                &gn.accts[signer],
                public_key,
                PUBKEY_REJECTION_SEND + index as u128,
                valid_signature,
                &format!("pubkey-rejection-{index}"),
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    txs.try_into()
        .map_err(|_| anyhow::anyhow!("wrong public-key fixture transaction count"))
}

fn pubkey_rejection_send(
    gn: &Gen,
    signer: &Acct,
    public_key: Any,
    amount: u128,
    valid_signature: bool,
    memo: &str,
) -> anyhow::Result<Vec<u8>> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;

    let body_bytes = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: signer.acc.clone(),
            to_address: gn.accts[2].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: amount.to_string(),
            }],
        })],
        memo: memo.into(),
        ..Default::default()
    }
    .encode_to_vec();
    #[allow(deprecated)]
    let auth_info_bytes = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(public_key),
            mode_info: Some(single_mode_info(SignMode::Direct)),
            sequence: 0,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: PUBKEY_REJECTION_FEE.to_string(),
            }],
            gas_limit: 200_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec();
    let direct = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: gn.chain_id.clone(),
        account_number: signer.number,
    }
    .encode_to_vec();
    let signature = if valid_signature {
        signer.key.sign(&direct)?.to_vec()
    } else {
        vec![7; 64]
    };
    Ok(TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![signature],
    }
    .encode_to_vec())
}

const P256_SEND: u128 = 19;
const P256_FEE: u128 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum P256SignatureCase {
    Short,
    ZeroScalar,
    WrongSignDoc,
    HighS,
    Valid,
}

struct P256Fixture {
    txs: [Vec<u8>; 6],
    check: [Option<abci::ResponseCheckTx>; 6],
    prepare_included: [bool; 6],
    process_status: [Option<i32>; 6],
    completed: [bool; 6],
    signer: String,
    signer_addr: Vec<u8>,
    signer_balance: u128,
    signer_sequence: u64,
    recipient: String,
    recipient_balance: u128,
    public_key: Any,
    expected_rejection: String,
}

impl P256Fixture {
    fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let signer = gn
            .accts
            .last()
            .context("controlled P-256 account was not added to genesis")?;
        anyhow::ensure!(
            signer.pk.type_url == x_ante::pubkey::SECP256R1_TYPE_URL,
            "controlled P-256 account has the wrong key type"
        );
        anyhow::ensure!(
            account_public_key(rust, &signer.acc)?.is_none(),
            "controlled P-256 account unexpectedly has a genesis public key"
        );
        let recipient = gn.accts[2].acc.clone();
        let cases = [
            P256SignatureCase::Short,
            P256SignatureCase::ZeroScalar,
            P256SignatureCase::WrongSignDoc,
            P256SignatureCase::HighS,
            P256SignatureCase::Valid,
            P256SignatureCase::Valid,
        ];
        let txs: [Vec<u8>; 6] = cases
            .into_iter()
            .enumerate()
            .map(|(index, case)| {
                p256_send(gn, signer, signer.seq + u64::from(index == 5), case, index)
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| anyhow::anyhow!("wrong P-256 fixture transaction count"))?;
        Ok(Self {
            txs,
            check: [None, None, None, None, None, None],
            prepare_included: [false; 6],
            process_status: [None, None, None, None, None, None],
            completed: [false; 6],
            signer: signer.acc.clone(),
            signer_addr: signer.addr.clone(),
            signer_balance: account_balance(rust, &signer.acc)?,
            signer_sequence: signer.seq,
            recipient_balance: account_balance(rust, &recipient)?,
            recipient,
            public_key: signer.pk.clone(),
            expected_rejection: format!(
                "signature verification failed; please verify account number ({}) and chain-id ({}): (unable to verify single signer signature): unauthorized",
                signer.number, gn.chain_id
            ),
        })
    }

    fn tx_index(&self, tx: &[u8]) -> Option<usize> {
        self.txs.iter().position(|candidate| candidate == tx)
    }

    fn require_complete(
        &mut self,
        index: usize,
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        let check = self.check[index]
            .as_ref()
            .context("P-256 transaction was not checked")?;
        let tx = &self.txs[index];
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(candidate, _)| candidate.as_slice() == tx)
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "P-256 transaction {} finalized {} times",
            index + 1,
            matches.len()
        );
        let result = matches[0].1;
        let accepted = abci::response_process_proposal::ProposalStatus::Accept as i32;
        let rejected = abci::response_process_proposal::ProposalStatus::Reject as i32;
        if index >= 4 {
            let success_index = index - 4;
            anyhow::ensure!(
                check.codespace.is_empty()
                    && check.code == 0
                    && check.log.is_empty()
                    && check.gas_wanted == 300_000
                    && check.gas_used == [67_699, 53_033][success_index]
                    && check.data.is_empty()
                    && check.info.is_empty()
                    && check.address == self.signer_addr
                    && check.priority == 333_333
                    && check.sequence == success_index as u64,
                "P-256 success CheckTx {} changed: {check:?}",
                index + 1
            );
            anyhow::ensure!(
                self.prepare_included[index] && self.process_status[index] == Some(accepted),
                "P-256 success transaction {} was not proposed and accepted",
                index + 1
            );
            anyhow::ensure!(
                result.codespace.is_empty()
                    && result.code == 0
                    && result.log.is_empty()
                    && result.gas_wanted == 300_000
                    && result.gas_used == [87_402, 72_718][success_index]
                    && result.data
                        == hex::decode(
                            "12260a242f636f736d6f732e62616e6b2e763162657461312e4d736753656e64526573706f6e7365"
                        )?
                    && result.info.is_empty()
                    && result.signers == [self.signer.clone()],
                "P-256 success FinalizeBlock {} changed: {result:?}",
                index + 1
            );
        } else {
            let expected_check_gas = [49_389, 49_348, 49_348, 49_348][index];
            let expected_finalize_gas = [49_338, 49_348, 49_348, 49_348][index];
            anyhow::ensure!(
                check.codespace == "sdk"
                    && check.code == 4
                    && check.log == self.expected_rejection
                    && check.gas_wanted == 300_000
                    && check.gas_used == expected_check_gas
                    && check.data.is_empty()
                    && check.info.is_empty()
                    && check.events.is_empty()
                    && check.address.is_empty()
                    && check.priority == 0
                    && check.sequence == 0,
                "P-256 rejection CheckTx {} changed: {check:?}",
                index + 1
            );
            anyhow::ensure!(
                !self.prepare_included[index] && self.process_status[index] == Some(rejected),
                "P-256 rejection transaction {} was not excluded and rejected",
                index + 1
            );
            anyhow::ensure!(
                result.codespace == check.codespace
                    && result.code == check.code
                    && result.log == self.expected_rejection
                    && result.gas_wanted == 300_000
                    && result.gas_used == expected_finalize_gas
                    && result.data.is_empty()
                    && result.info.is_empty()
                    && result.events.is_empty()
                    && result.signers == [self.signer.clone()],
                "P-256 rejection FinalizeBlock {} changed: {result:?}",
                index + 1
            );
        }
        let successful = index.saturating_sub(3) as u128;
        let sent = match successful {
            0 => 0,
            1 => P256_SEND + 4,
            2 => (P256_SEND + 4) + (P256_SEND + 5),
            _ => anyhow::bail!("invalid P-256 success count {successful}"),
        };
        anyhow::ensure!(
            account_balance(rust, &self.signer)?
                == self.signer_balance - successful * P256_FEE - sent
                && account_balance(rust, &self.recipient)? == self.recipient_balance + sent
                && rust.query_sequence(&self.signer_addr).sequence
                    == self.signer_sequence + successful as u64,
            "P-256 transaction {} has wrong committed balances or sequence",
            index + 1
        );
        let expected_key = (successful > 0).then_some(&self.public_key);
        anyhow::ensure!(
            account_public_key(rust, &self.signer)?.as_ref() == expected_key,
            "P-256 transaction {} did not preserve the exact account public key state",
            index + 1
        );
        anyhow::ensure!(!self.completed[index], "P-256 transaction completed twice");
        self.completed[index] = true;
        Ok(())
    }
}

fn p256_send(
    gn: &Gen,
    signer: &Acct,
    sequence: u64,
    case: P256SignatureCase,
    index: usize,
) -> anyhow::Result<Vec<u8>> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use p256::ecdsa::signature::Signer as _;

    let body_bytes = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: signer.acc.clone(),
            to_address: gn.accts[2].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: (P256_SEND + index as u128).to_string(),
            }],
        })],
        memo: format!("p256-signature-{index}"),
        ..Default::default()
    }
    .encode_to_vec();
    #[allow(deprecated)]
    let auth_info_bytes = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(signer.pk.clone()),
            mode_info: Some(single_mode_info(SignMode::Direct)),
            sequence,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: P256_FEE.to_string(),
            }],
            gas_limit: 300_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec();
    let signing_body_bytes = if case == P256SignatureCase::WrongSignDoc {
        let mut body = TxBody::decode(body_bytes.as_slice())?;
        body.memo.clear();
        body.encode_to_vec()
    } else {
        body_bytes.clone()
    };
    let direct = SignDoc {
        body_bytes: signing_body_bytes,
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: gn.chain_id.clone(),
        account_number: signer.number,
    }
    .encode_to_vec();
    let secret = p256::ecdsa::SigningKey::from_slice(&[42; 32])?;
    let signature: p256::ecdsa::Signature = secret.sign(&direct);
    let low = signature.normalize_s();
    let low_bytes = low.to_bytes().to_vec();
    let signature = match case {
        P256SignatureCase::Short => low_bytes[..63].to_vec(),
        P256SignatureCase::ZeroScalar => {
            let mut bytes = vec![0; 64];
            bytes[63] = 1;
            bytes
        }
        P256SignatureCase::WrongSignDoc | P256SignatureCase::Valid => low_bytes,
        P256SignatureCase::HighS => {
            let (r, s) = low.split_scalars();
            p256::ecdsa::Signature::from_scalars(r, -*s)?
                .to_bytes()
                .to_vec()
        }
    };
    Ok(TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![signature],
    }
    .encode_to_vec())
}

/// State and exact outcomes for a direct-aux sender whose fee is paid by a
/// second signer. The rejected transaction uses the same body and sender
/// sequence but leaves the fee payer implicit, making that direct-aux signer
/// the fee payer.
struct DirectAuxFixture {
    bad: Vec<u8>,
    good: Vec<u8>,
    sender: String,
    payer: String,
    recipient: String,
    sender_addr: Vec<u8>,
    payer_addr: Vec<u8>,
    sender_balance: u128,
    payer_balance: u128,
    recipient_balance: u128,
    sender_sequence: u64,
    payer_sequence: u64,
    bad_check: Option<(String, u32, String)>,
    good_check: Option<(String, u32, String)>,
    completed: bool,
}

impl DirectAuxFixture {
    fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let sender = gn.accts[0].acc.clone();
        let payer = gn.accts[1].acc.clone();
        let recipient = gn.accts[2].acc.clone();
        Ok(Self {
            bad: sign_direct_aux_send(gn, None)?,
            good: sign_direct_aux_send(gn, Some(1))?,
            sender_balance: account_balance(rust, &sender)?,
            payer_balance: account_balance(rust, &payer)?,
            recipient_balance: account_balance(rust, &recipient)?,
            sender_sequence: rust.query_sequence(&gn.accts[0].addr).sequence,
            payer_sequence: rust.query_sequence(&gn.accts[1].addr).sequence,
            sender,
            payer,
            recipient,
            sender_addr: gn.accts[0].addr.clone(),
            payer_addr: gn.accts[1].addr.clone(),
            bad_check: None,
            good_check: None,
            completed: false,
        })
    }

    fn require_complete(
        &self,
        finalized: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        let (codespace, code, log) = self
            .bad_check
            .as_ref()
            .context("direct-aux implicit-fee-payer transaction was not checked")?;
        anyhow::ensure!(
            codespace == "sdk"
                && *code == 4
                && log.contains("cannot sign with SIGN_MODE_DIRECT_AUX"),
            "direct-aux implicit-fee-payer CheckTx changed: {codespace}/{code} {log}"
        );
        let (codespace, code, log) = self
            .good_check
            .as_ref()
            .context("direct-aux separate-fee-payer transaction was not checked")?;
        anyhow::ensure!(
            codespace.is_empty() && *code == 0,
            "direct-aux separate-fee-payer CheckTx changed: {codespace}/{code} {log}"
        );
        anyhow::ensure!(
            !finalized.contains(&self.bad),
            "rejected direct-aux implicit-fee-payer transaction was finalized"
        );
        let matches: Vec<_> = finalized
            .iter()
            .zip(results)
            .filter(|(tx, _)| *tx == &self.good)
            .collect();
        anyhow::ensure!(
            matches.len() == 1 && matches[0].1.code == 0 && matches[0].1.codespace.is_empty(),
            "direct-aux separate-fee-payer transaction did not finalize exactly once with code 0: {matches:?}"
        );
        anyhow::ensure!(
            account_balance(rust, &self.sender)? == self.sender_balance - DIRECT_AUX_SEND
                && account_balance(rust, &self.recipient)?
                    == self.recipient_balance + DIRECT_AUX_SEND
                && account_balance(rust, &self.payer)? == self.payer_balance - DIRECT_AUX_FEE,
            "direct-aux committed balances did not debit the sender transfer, credit the recipient, and charge only the separate fee payer"
        );
        anyhow::ensure!(
            rust.query_sequence(&self.sender_addr).sequence == self.sender_sequence + 1,
            "direct-aux sender sequence did not advance exactly once"
        );
        anyhow::ensure!(
            rust.query_sequence(&self.payer_addr).sequence == self.payer_sequence + 1,
            "direct-aux fee-payer sequence did not advance exactly once"
        );
        Ok(())
    }
}

fn account_balance(rust: &mut app::App, address: &str) -> anyhow::Result<u128> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    let response = query_app::<bank::QueryBalanceResponse>(
        rust,
        "/cosmos.bank.v1beta1.Query/Balance",
        &bank::QueryBalanceRequest {
            address: address.into(),
            denom: "utia".into(),
        },
    )
    .context("querying direct-aux account balance")?;
    response
        .balance
        .context("direct-aux account had no utia balance")?
        .amount
        .parse()
        .context("parsing direct-aux utia balance")
}

fn sign_direct_aux_send(gn: &Gen, payer: Option<usize>) -> anyhow::Result<Vec<u8>> {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    let sender = &gn.accts[0];
    let body_bytes = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: sender.acc.clone(),
            to_address: gn.accts[2].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: DIRECT_AUX_SEND.to_string(),
            }],
        })],
        ..Default::default()
    }
    .encode_to_vec();
    let mut signer_infos = vec![SignerInfo {
        public_key: Some(sender.pk.clone()),
        mode_info: Some(single_mode_info(SignMode::DirectAux)),
        sequence: sender.seq,
    }];
    if let Some(i) = payer {
        signer_infos.push(SignerInfo {
            public_key: Some(gn.accts[i].pk.clone()),
            mode_info: Some(single_mode_info(SignMode::Direct)),
            sequence: gn.accts[i].seq,
        });
    }
    #[allow(deprecated)]
    let auth_info_bytes = AuthInfo {
        signer_infos,
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: DIRECT_AUX_FEE.to_string(),
            }],
            gas_limit: 300_000,
            payer: payer.map_or_else(String::new, |i| gn.accts[i].acc.clone()),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec();
    #[allow(deprecated)]
    let aux_bytes = SignDocDirectAux {
        body_bytes: body_bytes.clone(),
        public_key: Some(sender.pk.clone()),
        chain_id: gn.chain_id.clone(),
        account_number: sender.number,
        sequence: sender.seq,
        tip: None,
    }
    .encode_to_vec();
    let mut signatures = vec![sender.key.sign(&aux_bytes)?.to_vec()];
    if let Some(i) = payer {
        let payer = &gn.accts[i];
        let direct = SignDoc {
            body_bytes: body_bytes.clone(),
            auth_info_bytes: auth_info_bytes.clone(),
            chain_id: gn.chain_id.clone(),
            account_number: payer.number,
        }
        .encode_to_vec();
        signatures.push(payer.key.sign(&direct)?.to_vec());
    }
    Ok(TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures,
    }
    .encode_to_vec())
}

/// A plain direct-mode tx of `msgs` signed by account `i`.
fn sign_simple(gn: &Gen, i: usize, msgs: Vec<Any>, gas: u64) -> Option<Vec<u8>> {
    sign_with(gn, i, msgs, gas, String::new())
}

/// [`sign_simple`] with the fee paid by `granter`'s allowance (when not
/// empty).
fn sign_with(gn: &Gen, i: usize, msgs: Vec<Any>, gas: u64, granter: String) -> Option<Vec<u8>> {
    let a = &gn.accts[i];
    let body = TxBody {
        messages: msgs,
        ..Default::default()
    };
    let (mode_info, plan) = match &a.ms {
        Some(t) => ms_plan(None, t, SignMode::Direct),
        None => (
            single_mode_info(SignMode::Direct),
            Plan::Single {
                acct: i,
                mode: SignMode::Direct,
                corrupt: false,
            },
        ),
    };
    #[allow(deprecated)]
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(a.pk.clone()),
            mode_info: Some(mode_info),
            sequence: a.seq,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "100000".into(),
            }],
            gas_limit: gas,
            payer: String::new(),
            granter,
        }),
        tip: None,
    };
    let body_bytes = body.encode_to_vec();
    let auth_info_bytes = auth.encode_to_vec();
    let direct = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: gn.chain_id.clone(),
        account_number: a.number,
    }
    .encode_to_vec();
    let sig = sign_plan(&gn.accts, &plan, &direct, None);
    Some(
        TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: vec![sig],
        }
        .encode_to_vec(),
    )
}

/// Generated txs signed in `SIGN_MODE_LEGACY_AMINO_JSON` (the
/// `aminosign` oracle op's seeds): the delivery generator's messages
/// (nested ones too) with values harvested from the devnet txs.
#[must_use]
pub fn amino_sign_seeds(app_version: u64, seed: u64, n: usize) -> Vec<Vec<u8>> {
    let mut g = offline_gen(app_version, seed, true);
    (0..n).filter_map(|_| g.tx().map(|b| b.raw)).collect()
}

/// Valid messages of every registered `sdk.Msg` type at `app_version` for
/// the wire-level enumeration ([`crate::wire`]): per type, `per_type`
/// synthesized ones and up to `per_type` from the recorded scenarios
/// (signer rewritten to fuzz account 0), plus state-building flows.
#[must_use]
pub fn wire_bases(app_version: u64, seed: u64, per_type: usize) -> Vec<Any> {
    let mut g = offline_gen(app_version, seed, false);
    let who = g.who(0);
    let mut out = Vec::new();
    for md in g.types.clone() {
        let url = format!("/{}", md.full_name());
        for _ in 0..per_type {
            out.push(Any {
                type_url: url.clone(),
                value: g.synth(&md, &who, 0),
            });
        }
        let sc: Vec<Any> = g
            .scenario_msgs
            .iter()
            .filter(|a| a.type_url == url)
            .take(per_type)
            .cloned()
            .collect();
        for a in sc {
            out.push(Any {
                type_url: a.type_url.clone(),
                value: Gen::resign(&md, &a.value, &who),
            });
        }
    }
    for _ in 0..per_type * 40 {
        out.push(g.flow(0));
    }
    out.retain(|a| registered(app_version, &a.type_url));
    out
}

/// How [`wire_input`] carries a (mutated) message to the oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireWrap {
    /// The tx's only message.
    Direct,
    /// In a `MsgExec` (fuzz account 0 the grantee).
    Exec,
    /// In a gov `MsgSubmitProposal`.
    Gov,
    /// The message of a protobuf ICA `CosmosTx` (`json_ica_pb`: the host's
    /// decoding and `ValidateBasic`).
    Ica,
}

/// The oracle input of `msg` carried as `w`: `(op, input)` pairs (the
/// tx decoder, and the amino-JSON sign bytes where a tx carries it).
#[must_use]
pub fn wire_input(msg: &Any, w: WireWrap) -> Vec<(&'static str, Vec<u8>)> {
    use celestia_proto::cosmos::authz::v1beta1 as authz;
    use celestia_proto::cosmos::gov::v1 as gov;
    use celestia_proto::ibc::applications::interchain_accounts::v1::CosmosTx;
    let accts = wire_accounts();
    let me = accts[0].acc.clone();
    let inner = match w {
        WireWrap::Direct => msg.clone(),
        WireWrap::Exec => Any::pack(&authz::MsgExec {
            grantee: me,
            msgs: vec![msg.clone()],
        }),
        WireWrap::Gov => Any::pack(&gov::MsgSubmitProposal {
            messages: vec![msg.clone()],
            initial_deposit: Vec::new(),
            proposer: me,
            metadata: String::new(),
            title: "t".into(),
            summary: "s".into(),
            expedited: false,
        }),
        WireWrap::Ica => {
            return vec![(
                "json_ica_pb",
                CosmosTx {
                    messages: vec![msg.clone()],
                }
                .encode_to_vec(),
            )];
        }
    };
    let body = TxBody {
        messages: vec![inner],
        ..Default::default()
    }
    .encode_to_vec();
    let tx = wire_tx(&body, &wire_auth_info(&accts[0]));
    vec![("tx", tx.clone()), ("aminosign", tx)]
}

/// A tx of `body` and `auth_info` with one 64-byte signature.
#[must_use]
pub fn wire_tx(body: &[u8], auth_info: &[u8]) -> Vec<u8> {
    TxRaw {
        body_bytes: body.to_vec(),
        auth_info_bytes: auth_info.to_vec(),
        signatures: vec![vec![1u8; 64]],
    }
    .encode_to_vec()
}

fn wire_accounts() -> &'static [Acct] {
    static A: std::sync::OnceLock<Vec<Acct>> = std::sync::OnceLock::new();
    A.get_or_init(|| accounts(1))
}

fn wire_auth_info(a: &Acct) -> Vec<u8> {
    #[allow(deprecated)]
    AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(a.pk.clone()),
            mode_info: Some(single_mode_info(SignMode::LegacyAminoJson)),
            sequence: 3,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "5000".into(),
            }],
            gas_limit: 200_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec()
}

/// A valid tx body (one `MsgSend` of fuzz account 0) and auth info, for
/// the enumeration of `TxBody` / `AuthInfo` themselves.
#[must_use]
pub fn wire_envelope() -> (Vec<u8>, Vec<u8>) {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    let accts = wire_accounts();
    let body = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: accts[0].acc.clone(),
            to_address: accts[1].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "1".into(),
            }],
        })],
        memo: "m".into(),
        timeout_height: 9,
        extension_options: Vec::new(),
        // No extension option type is registered: any would fail the
        // decoder (the `default_explicit` class adds empty ones).
        non_critical_extension_options: Vec::new(),
    }
    .encode_to_vec();
    (body, wire_auth_info(&accts[0]))
}

/// A generator without a chain (the accounts at numbers from 1, the
/// recorded scenarios' messages and values).
fn offline_gen(app_version: u64, seed: u64, force_amino: bool) -> Gen {
    let mut g = Gen {
        m: Mutator::new(seed),
        dict: Dict::default(),
        accts: accounts(1),
        scenario_msgs: Vec::new(),
        validators: Vec::new(),
        proposals: vec![1, 2],
        unbondings: Vec::new(),
        proposers: Vec::new(),
        gas_seen: BTreeMap::new(),
        hl_objects: Vec::new(),
        hl: HlState::default(),
        hex_ids: Vec::new(),
        chain_id: "c".into(),
        app_version,
        height: 10,
        types: msg_types()
            .iter()
            .filter(|md| registered(app_version, &format!("/{}", md.full_name())))
            .cloned()
            .collect(),
        force_amino,
        wire: WireGen::default(),
        gov_msgs: Vec::new(),
        reserved_account: None,
    };
    for t in crate::corpus::devnet_txs() {
        g.m.harvest(&t);
        let Ok(tx) = txdecode::decode_for(app_version, &t) else {
            continue;
        };
        for a in &tx.body.messages {
            if let Some(md) = desc_for_type_url(a.type_url.as_bytes()) {
                g.dict.harvest(&md, &a.value, 0);
                g.scenario_msgs.push(a.clone());
            }
        }
    }
    for (k, v) in &g.dict.strings {
        if k.contains("validator") {
            for x in v {
                if x.starts_with(b"celestiavaloper1") {
                    g.validators.push(String::from_utf8_lossy(x).into_owned());
                }
            }
        }
    }
    g
}

/// Whether `type_url` is in the tx decoder's registry at `app_version`.
/// The generated message types: those the decoder knows at
/// `app_version`; with `both` (`DELIVER_UPGRADE`), those of app versions 9
/// and 10 (v10-only ones before the upgrade, v9 shapes after).
fn types_for(app_version: u64, both: bool) -> Vec<MessageDescriptor> {
    msg_types()
        .iter()
        .filter(|md| {
            let u = format!("/{}", md.full_name());
            registered(app_version, &u) || (both && (registered(9, &u) || registered(10, &u)))
        })
        .cloned()
        .collect()
}

fn registered(app_version: u64, type_url: &str) -> bool {
    let body = TxBody {
        messages: vec![Any {
            type_url: type_url.to_owned(),
            value: Vec::new(),
        }],
        ..Default::default()
    };
    let raw = TxRaw {
        body_bytes: body.encode_to_vec(),
        auth_info_bytes: Vec::new(),
        signatures: Vec::new(),
    }
    .encode_to_vec();
    txdecode::decode_for(app_version, &raw).is_ok()
}

/// The coverage table (`fuzz-diff coverage`): every `sdk.Msg` type of the
/// registry, whether app versions 10 / 9 decode it, and in how many
/// recorded scenario txs Go executed it with code 0 / another code (`>`:
/// nested in `MsgExec` / a proposal), plus whether `EXEC_MODE=synth`
/// builds it.
#[must_use]
pub fn coverage() -> String {
    use std::fmt::Write as _;
    let mut rec: BTreeMap<String, (u64, u64, BTreeSet<String>)> = BTreeMap::new();
    let mut rec_gov: BTreeMap<String, u64> = BTreeMap::new();
    for sc in crate::corpus::scenarios() {
        let Ok(exs) = devnet_conformance::load_exchanges(&sc) else {
            continue;
        };
        let av = if sc.starts_with("v9-") { 9 } else { 10 };
        let mut proposals = gov_coverage::Proposals::default();
        for ex in exs {
            if let (
                Some(request::Value::FinalizeBlock(r)),
                Some(response::Value::FinalizeBlock(g)),
            ) = (ex.request.value, ex.response.value)
            {
                for (t, res) in r.txs.iter().zip(&g.tx_results) {
                    let version = if txdecode::decode_for(av, t).is_ok() {
                        av
                    } else {
                        10
                    };
                    let urls = if res.code == 0 {
                        proposals.submitted(version, t, &res.events);
                        executed_type_urls(version, t)
                    } else {
                        type_urls(version, t)
                    };
                    for u in urls {
                        let e = rec.entry(u).or_default();
                        if res.code == 0 {
                            e.0 += 1;
                        } else {
                            e.1 += 1;
                        }
                        e.2.insert(sc.clone());
                    }
                }
                proposals.completed(&g.events, &mut rec_gov, None, None);
            }
        }
    }
    let mut out = String::from(
        "type\tv10\tv9\trust_handler\tsynth\trecorded_ok\trecorded_fail\tnested_ok\tnested_fail\tproposal_executed\tscenarios\n",
    );
    for md in msg_types() {
        let url = format!("/{}", md.full_name());
        let top = rec.get(&url).cloned().unwrap_or_default();
        let nested = rec.get(&format!(">{url}")).cloned().unwrap_or_default();
        let scs: Vec<String> = top.2.union(&nested.2).cloned().collect();
        let _ = writeln!(
            out,
            "{url}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            u8::from(registered(10, &url)),
            u8::from(registered(9, &url)),
            u8::from(app::router::has_handler(&url)),
            u8::from(crate::exec::reaches_math(md, 0)),
            top.0,
            top.1,
            nested.0,
            nested.1,
            rec_gov.get(&url).copied().unwrap_or(0),
            scs.join(",")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn direct_aux_fixture_has_exact_signer_and_fee_payer_shapes() {
        use celestia_proto::cosmos::bank::v1beta1::MsgSend;

        let g = offline_gen(10, 1, false);
        let bad = TxRaw::decode(sign_direct_aux_send(&g, None).unwrap().as_slice()).unwrap();
        let good = TxRaw::decode(sign_direct_aux_send(&g, Some(1)).unwrap().as_slice()).unwrap();
        assert_eq!(bad.body_bytes, good.body_bytes);
        let body = TxBody::decode(good.body_bytes.as_slice()).unwrap();
        let send = MsgSend::decode(body.messages[0].value.as_slice()).unwrap();
        assert_eq!(send.from_address, g.accts[0].acc);
        assert_eq!(send.to_address, g.accts[2].acc);
        assert_eq!(send.amount[0].amount, DIRECT_AUX_SEND.to_string());

        let bad_auth = AuthInfo::decode(bad.auth_info_bytes.as_slice()).unwrap();
        assert_eq!(bad_auth.signer_infos.len(), 1);
        assert_eq!(bad.signatures.len(), 1);
        assert_eq!(bad_auth.fee.unwrap().payer, "");
        let good_auth = AuthInfo::decode(good.auth_info_bytes.as_slice()).unwrap();
        assert_eq!(good_auth.signer_infos.len(), 2);
        assert_eq!(good.signatures.len(), 2);
        assert_eq!(good_auth.fee.unwrap().payer, g.accts[1].acc);
        let modes: Vec<_> = good_auth
            .signer_infos
            .iter()
            .map(|info| match &info.mode_info.as_ref().unwrap().sum {
                Some(mode_info::Sum::Single(single)) => single.mode,
                _ => panic!("fixture must use single-key modes"),
            })
            .collect();
        assert_eq!(
            modes,
            vec![SignMode::DirectAux as i32, SignMode::Direct as i32]
        );
    }

    #[test]
    fn direct_aux_requirement_fails_closed() {
        let mut stats = Stats {
            direct_aux_required: Some(()),
            ..Stats::default()
        };
        assert!(stats.require_direct_aux_coverage().is_err());
        stats.direct_aux_completed = 1;
        stats.require_direct_aux_coverage().unwrap();
        stats.direct_aux_completed = 2;
        assert!(stats.require_direct_aux_coverage().is_err());
        assert!(Stats::default().require_direct_aux_coverage().is_ok());
    }

    #[test]
    fn malformed_multisig_fixtures_have_the_three_terminal_shapes() {
        use celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature;

        let g = offline_gen(10, 1, false);
        let txs: Vec<_> = [(0, 0), (1, ACCOUNTS), (2, ACCOUNTS + 1)]
            .into_iter()
            .map(|(shape, signer)| {
                TxRaw::decode(
                    sign_malformed_multisig_send(&g, shape, signer)
                        .unwrap()
                        .as_slice(),
                )
                .unwrap()
            })
            .collect();
        let auths: Vec<_> = txs
            .iter()
            .map(|tx| AuthInfo::decode(tx.auth_info_bytes.as_slice()).unwrap())
            .collect();
        let mode = |index: usize| match auths[index].signer_infos[0]
            .mode_info
            .as_ref()
            .unwrap()
            .sum
            .as_ref()
            .unwrap()
        {
            mode_info::Sum::Multi(multi) => multi,
            mode_info::Sum::Single(_) => panic!("fixture must carry multisig mode data"),
        };
        assert_eq!(mode(0).bitarray.as_ref().unwrap(), &bit_array(1, &[0]));
        assert_eq!(mode(0).mode_infos.len(), 1);
        assert_eq!(mode(1).bitarray.as_ref().unwrap(), &bit_array(3, &[0]));
        assert_eq!(mode(1).mode_infos.len(), 2);
        assert_eq!(
            MultiSignature::decode(txs[1].signatures[0].as_slice())
                .unwrap()
                .signatures
                .len(),
            2
        );
        assert_eq!(mode(2).bitarray.as_ref().unwrap(), &bit_array(2, &[0]));
        assert!(matches!(
            mode(2).mode_infos[0].sum,
            Some(mode_info::Sum::Multi(_))
        ));
        let outer = MultiSignature::decode(txs[2].signatures[0].as_slice()).unwrap();
        assert!(MultiSignature::decode(outer.signatures[0].as_slice()).is_ok());
    }

    #[test]
    fn malformed_multisig_requirement_fails_closed() {
        let mut stats = Stats {
            malformed_multisig_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=4 {
            stats.malformed_multisig_completed = completed;
            assert_eq!(
                stats.require_malformed_multisig_coverage().is_ok(),
                completed == 3
            );
        }
        assert!(
            Stats::default()
                .require_malformed_multisig_coverage()
                .is_ok()
        );
    }

    #[test]
    fn zero_threshold_multisig_fixtures_have_no_child_signatures() {
        use celestia_proto::cosmos::crypto::multisig::LegacyAminoPubKey;
        use celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature;

        let mut g = offline_gen(10, 1, false);
        let account = zero_threshold_account(&g.accts, 999);
        g.accts.push(account);
        for sequence in [0, 1] {
            let raw = TxRaw::decode(
                zero_threshold_multisig_send(&g, g.accts.last().unwrap(), sequence).as_slice(),
            )
            .unwrap();
            let auth = AuthInfo::decode(raw.auth_info_bytes.as_slice()).unwrap();
            let signer = &auth.signer_infos[0];
            let key =
                LegacyAminoPubKey::decode(signer.public_key.as_ref().unwrap().value.as_slice())
                    .unwrap();
            assert_eq!(key.threshold, 0);
            assert_eq!(key.public_keys.len(), 1);
            let mode = match signer.mode_info.as_ref().unwrap().sum.as_ref().unwrap() {
                mode_info::Sum::Multi(multi) => multi,
                mode_info::Sum::Single(_) => panic!("fixture must carry multisig mode data"),
            };
            assert_eq!(mode.bitarray.as_ref().unwrap(), &bit_array(1, &[]));
            assert!(mode.mode_infos.is_empty());
            assert_eq!(signer.sequence, sequence);
            assert_eq!(raw.signatures.len(), 1);
            assert!(
                MultiSignature::decode(raw.signatures[0].as_slice())
                    .unwrap()
                    .signatures
                    .is_empty()
            );
        }
    }

    #[test]
    fn zero_threshold_multisig_requirement_fails_closed() {
        let mut stats = Stats {
            zero_threshold_multisig_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=3 {
            stats.zero_threshold_multisig_completed = completed;
            assert_eq!(
                stats.require_zero_threshold_multisig_coverage().is_ok(),
                completed == 2
            );
        }
        assert!(
            Stats::default()
                .require_zero_threshold_multisig_coverage()
                .is_ok()
        );
    }

    #[test]
    fn never_bonded_unjail_requirement_fails_closed() {
        let mut stats = Stats {
            never_bonded_unjail_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=2 {
            stats.never_bonded_unjail_completed = completed;
            assert_eq!(
                stats.require_never_bonded_unjail_coverage().is_ok(),
                completed == 1
            );
        }
        assert!(
            Stats::default()
                .require_never_bonded_unjail_coverage()
                .is_ok()
        );
    }

    #[test]
    fn unbonded_redelegation_rollback_requirement_fails_closed() {
        let mut stats = Stats {
            unbonded_redelegation_rollback_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=2 {
            stats.unbonded_redelegation_rollback_completed = completed;
            assert_eq!(
                stats
                    .require_unbonded_redelegation_rollback_coverage()
                    .is_ok(),
                completed == 1
            );
        }
        assert!(
            Stats::default()
                .require_unbonded_redelegation_rollback_coverage()
                .is_ok()
        );
    }

    #[test]
    fn gov_params_rejection_requirement_fails_closed() {
        let complete_cases = gov_params::CASE_NAMES
            .into_iter()
            .map(|name| (name.to_owned(), 1))
            .collect::<BTreeMap<_, _>>();
        let mut stats = Stats {
            gov_params_rejection_required: Some(()),
            gov_params_rejection_completed: 1,
            gov_params_rejection_cases: complete_cases.clone(),
            ..Stats::default()
        };
        assert!(stats.require_gov_params_rejection_coverage().is_ok());
        stats.gov_params_rejection_completed = 0;
        assert!(stats.require_gov_params_rejection_coverage().is_err());
        stats.gov_params_rejection_completed = 1;
        stats
            .gov_params_rejection_cases
            .remove(gov_params::CASE_NAMES[0]);
        assert!(stats.require_gov_params_rejection_coverage().is_err());
        stats
            .gov_params_rejection_cases
            .insert("wrong-case".into(), 1);
        assert!(stats.require_gov_params_rejection_coverage().is_err());
        stats.gov_params_rejection_cases = complete_cases;
        stats
            .gov_executed
            .insert("/cosmos.gov.v1.MsgUpdateParams".into(), 1);
        assert!(stats.require_gov_params_rejection_coverage().is_err());
        assert!(
            Stats::default()
                .require_gov_params_rejection_coverage()
                .is_ok()
        );
    }

    #[test]
    fn gov_handler_rejection_requirement_fails_closed() {
        let complete_cases = gov_errors::CASE_NAMES
            .into_iter()
            .map(|name| (name.to_owned(), 1))
            .collect::<BTreeMap<_, _>>();
        let mut stats = Stats {
            gov_handler_rejection_required: Some(()),
            gov_handler_rejection_completed: 1,
            gov_handler_rejection_cases: complete_cases.clone(),
            ..Stats::default()
        };
        stats
            .gov_executed
            .insert("/cosmos.gov.v1.MsgUpdateParams".into(), 2);
        assert!(stats.require_gov_handler_rejection_coverage().is_ok());
        stats.gov_handler_rejection_completed = 0;
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        stats.gov_handler_rejection_completed = 1;
        stats
            .gov_handler_rejection_cases
            .remove(gov_errors::CASE_NAMES[0]);
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        stats
            .gov_handler_rejection_cases
            .insert("wrong-case".into(), 1);
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        stats.gov_handler_rejection_cases = complete_cases.clone();
        *stats
            .gov_handler_rejection_cases
            .get_mut(gov_errors::CASE_NAMES[0])
            .unwrap() = 2;
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        stats.gov_handler_rejection_cases = complete_cases;
        stats
            .gov_executed
            .insert("/cosmos.gov.v1.MsgUpdateParams".into(), 1);
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        stats
            .gov_executed
            .insert("/cosmos.gov.v1.MsgUpdateParams".into(), 3);
        assert!(stats.require_gov_handler_rejection_coverage().is_err());
        assert!(
            Stats::default()
                .require_gov_handler_rejection_coverage()
                .is_ok()
        );
    }

    #[test]
    fn validator_removal_rewards_requirement_fails_closed() {
        let mut stats = Stats {
            validator_removal_rewards_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=2 {
            stats.validator_removal_rewards_completed = completed;
            assert_eq!(
                stats.require_validator_removal_rewards_coverage().is_ok(),
                completed == 1
            );
        }
        assert!(
            Stats::default()
                .require_validator_removal_rewards_coverage()
                .is_ok()
        );
    }

    #[test]
    fn pubkey_rejection_fixtures_bind_key_lengths_and_signers() {
        use celestia_proto::cosmos::bank::v1beta1::MsgSend;
        use celestia_proto::cosmos::crypto::{ed25519, secp256k1};

        let mut g = offline_gen(10, 1, false);
        let base = g.accts.len();
        g.accts.push(controlled_secp_account(999, "secp"));
        g.accts.push(controlled_secp_account(1_000, "ed-malformed"));
        let ed = controlled_ed25519_account(&g.accts, 1_001).unwrap();
        g.accts.push(ed);
        let txs = pubkey_rejection_txs(&g, base).unwrap();
        let expected = [
            (x_ante::pubkey::SECP256K1_TYPE_URL, 32, base),
            (x_ante::pubkey::SECP256K1_TYPE_URL, 33, base),
            (x_ante::pubkey::ED25519_TYPE_URL, 31, base + 1),
            (x_ante::pubkey::SECP256K1_TYPE_URL, 33, base + 1),
            (x_ante::pubkey::ED25519_TYPE_URL, 32, base + 2),
            (x_ante::pubkey::ED25519_TYPE_URL, 32, base + 2),
        ];
        let unique: std::collections::BTreeSet<_> = txs.iter().collect();
        assert_eq!(unique.len(), txs.len());
        for (index, tx) in txs.iter().enumerate() {
            let raw = TxRaw::decode(tx.as_slice()).unwrap();
            let auth = AuthInfo::decode(raw.auth_info_bytes.as_slice()).unwrap();
            let signer = &auth.signer_infos[0];
            let public_key = signer.public_key.as_ref().unwrap();
            assert_eq!(public_key.type_url, expected[index].0);
            let key_len = if public_key.type_url == x_ante::pubkey::SECP256K1_TYPE_URL {
                secp256k1::PubKey::decode(public_key.value.as_slice())
                    .unwrap()
                    .key
                    .len()
            } else {
                ed25519::PubKey::decode(public_key.value.as_slice())
                    .unwrap()
                    .key
                    .len()
            };
            assert_eq!(key_len, expected[index].1);
            assert_eq!(signer.sequence, 0);
            assert_eq!(raw.signatures.len(), 1);
            assert_eq!(raw.signatures[0].len(), 64);
            let body = TxBody::decode(raw.body_bytes.as_slice()).unwrap();
            assert_eq!(body.memo, format!("pubkey-rejection-{index}"));
            assert_eq!(body.messages.len(), 1);
            let send = MsgSend::decode(body.messages[0].value.as_slice()).unwrap();
            assert_eq!(send.from_address, g.accts[expected[index].2].acc);
            assert_eq!(send.to_address, g.accts[2].acc);
        }
    }

    #[test]
    fn pubkey_rejection_requirement_fails_closed() {
        let mut stats = Stats {
            pubkey_rejection_required: Some(()),
            log_mismatches: vec![
                "length of pubkey is incorrect".into(),
                "pubkey is incorrect size".into(),
            ],
            ..Stats::default()
        };
        for completed in 0..=7 {
            stats.pubkey_rejection_completed = completed;
            assert_eq!(
                stats.require_pubkey_rejection_coverage().is_ok(),
                completed == 6
            );
        }
        stats.pubkey_rejection_completed = 6;
        stats.log_mismatches.pop();
        assert!(stats.require_pubkey_rejection_coverage().is_err());
        assert!(Stats::default().require_pubkey_rejection_coverage().is_ok());
    }

    #[test]
    fn pubkey_rejection_rejects_wrong_rust_panic_value() {
        let mut check = abci::ResponseCheckTx {
            codespace: "undefined".into(),
            code: 111_222,
            log: "recovered: length of pubkey is incorrect: panic".into(),
            gas_used: 1_180,
            ..Default::default()
        };
        assert!(exact_rust_malformed_pubkey_check(
            &check,
            "length of pubkey is incorrect"
        ));
        check.log = "recovered: different panic value: panic".into();
        assert!(!exact_rust_malformed_pubkey_check(
            &check,
            "length of pubkey is incorrect"
        ));
    }

    #[test]
    fn p256_fixtures_bind_signature_boundaries_and_recovery() {
        use celestia_proto::cosmos::crypto::secp256r1;
        use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, SignDoc, TxBody, TxRaw};

        let mut g = offline_gen(10, 1, false);
        let account = p256_account(&g.accts, 999).unwrap();
        g.accts.push(account);
        let cases = [
            P256SignatureCase::Short,
            P256SignatureCase::ZeroScalar,
            P256SignatureCase::WrongSignDoc,
            P256SignatureCase::HighS,
            P256SignatureCase::Valid,
            P256SignatureCase::Valid,
        ];
        let txs: Vec<_> = cases
            .into_iter()
            .enumerate()
            .map(|(index, case)| {
                p256_send(
                    &g,
                    g.accts.last().unwrap(),
                    u64::from(index == 5),
                    case,
                    index,
                )
                .unwrap()
            })
            .collect();
        let unique: std::collections::BTreeSet<_> = txs.iter().collect();
        assert_eq!(unique.len(), txs.len());
        for (index, tx) in txs.iter().enumerate() {
            let raw = TxRaw::decode(tx.as_slice()).unwrap();
            let auth = AuthInfo::decode(raw.auth_info_bytes.as_slice()).unwrap();
            assert_eq!(auth.signer_infos[0].sequence, u64::from(index == 5));
            let any = auth.signer_infos[0].public_key.as_ref().unwrap();
            let key = secp256r1::PubKey::decode(any.value.as_slice()).unwrap().key;
            assert!(x_ante::signing::p256_point_valid(&key));
            let body = TxBody::decode(raw.body_bytes.as_slice()).unwrap();
            assert_eq!(body.memo, format!("p256-signature-{index}"));
            let submitted = SignDoc {
                body_bytes: raw.body_bytes.clone(),
                auth_info_bytes: raw.auth_info_bytes.clone(),
                chain_id: g.chain_id.clone(),
                account_number: 999,
            }
            .encode_to_vec();
            let sig = &raw.signatures[0];
            match index {
                0 => {
                    assert_eq!(sig.len(), 63);
                    assert!(p256::ecdsa::Signature::from_slice(sig).is_err());
                    assert!(!x_ante::signing::verify_secp256r1(&key, &submitted, sig));
                }
                1 => {
                    assert_eq!(sig.len(), 64);
                    assert!(sig[..63].iter().all(|byte| *byte == 0));
                    assert_eq!(sig[63], 1);
                    assert!(p256::ecdsa::Signature::from_slice(sig).is_err());
                    assert!(!x_ante::signing::verify_secp256r1(&key, &submitted, sig));
                }
                2 => {
                    let signature = p256::ecdsa::Signature::from_slice(sig).unwrap();
                    assert_eq!(signature.normalize_s(), signature);
                    assert!(!x_ante::signing::verify_secp256r1(&key, &submitted, sig));
                    let mut signing_body = body;
                    signing_body.memo.clear();
                    let signed = SignDoc {
                        body_bytes: signing_body.encode_to_vec(),
                        auth_info_bytes: raw.auth_info_bytes.clone(),
                        chain_id: g.chain_id.clone(),
                        account_number: 999,
                    }
                    .encode_to_vec();
                    assert!(x_ante::signing::verify_secp256r1(&key, &signed, sig));
                }
                3 => {
                    let signature = p256::ecdsa::Signature::from_slice(sig).unwrap();
                    assert_ne!(signature.normalize_s(), signature);
                }
                4 | 5 => {
                    let signature = p256::ecdsa::Signature::from_slice(sig).unwrap();
                    assert_eq!(signature.normalize_s(), signature);
                    assert!(x_ante::signing::verify_secp256r1(&key, &submitted, sig));
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn p256_requirement_fails_closed() {
        let mut stats = Stats {
            p256_required: Some(()),
            ..Stats::default()
        };
        for completed in 0..=7 {
            stats.p256_completed = completed;
            assert_eq!(stats.require_p256_coverage().is_ok(), completed == 6);
        }
        assert!(Stats::default().require_p256_coverage().is_ok());
    }

    #[test]
    fn script_requirements_reject_incomplete_or_wrong_finalized_results() {
        let expected = r#"[{"type_url":"/cosmos.authz.v1beta1.MsgExec","codespace":"staking","code":3,"log_contains":"validator does not exist"}]"#;
        let mut stats = Stats {
            script_required: script_expectations(expected).unwrap(),
            ..Stats::default()
        };
        assert!(stats.require_script_coverage().is_err());
        let result = abci::ExecTxResult {
            codespace: "staking".into(),
            code: 3,
            log: "message index: 0: validator does not exist".into(),
            ..abci::ExecTxResult::default()
        };
        stats
            .script_outcomes
            .push(("/cosmos.authz.v1beta1.MsgExec".into(), result.clone()));
        stats.require_script_coverage().unwrap();
        stats.script_incomplete = Some("script transaction missing");
        assert!(stats.require_script_coverage().is_err());
        stats.script_incomplete = None;
        stats.script_outcomes[0].1.code = 0;
        assert!(stats.require_script_coverage().is_err());
        stats.script_outcomes[0].1 = result.clone();
        stats.script_outcomes[0].1.codespace = "sdk".into();
        assert!(stats.require_script_coverage().is_err());
        stats.script_outcomes[0].1 = result.clone();
        stats.script_outcomes[0].1.log = "an unrelated staking error".into();
        assert!(stats.require_script_coverage().is_err());
        stats.script_outcomes[0] = ("/cosmos.authz.v1beta1.MsgGrant".into(), result.clone());
        assert!(stats.require_script_coverage().is_err());
        stats.script_outcomes[0].0 = "/cosmos.authz.v1beta1.MsgExec".into();
        stats
            .script_outcomes
            .push(("/cosmos.authz.v1beta1.MsgExec".into(), result));
        assert!(stats.require_script_coverage().is_err());
    }

    #[test]
    fn script_requirement_schema_rejects_empty_or_ambiguous_contracts() {
        for bad in [
            "[]",
            "{}",
            r#"[{"type_url":"/msg","codespace":"","code":5}]"#,
            r#"[{"type_url":"/msg","codespace":"sdk","code":0}]"#,
            r#"[{"type_url":"msg","codespace":"","code":0}]"#,
            r#"[{"type_url":"/msg","codespace":"","code":0,"typo":true}]"#,
        ] {
            assert!(script_expectations(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn authz_boundary_script_reaches_the_real_nested_staking_handler() {
        use celestia_proto::cosmos::{
            authz::v1beta1 as authz, bank::v1beta1::MsgSend, staking::v1beta1::MsgDelegate,
        };
        use prost::Name;
        let script = read_script(
            include_str!("../fixtures/authz-boundaries.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected =
            script_expectations(include_str!("../fixtures/authz-boundaries.expected.json"))
                .unwrap();
        assert_eq!(script.len(), 27);
        assert_eq!(expected.len(), 27);
        assert!(script.iter().all(|block| block.len() == 1));
        for (block, result) in script.iter().zip(&expected) {
            assert_eq!(block[0].1.type_url, result.type_url);
        }
        let (_, outer, _) = &script[18][0];
        let exec = authz::MsgExec::decode(outer.value.as_slice()).unwrap();
        assert_eq!(exec.grantee, accounts(1)[1].acc);
        assert_eq!(exec.msgs.len(), 2);
        assert_eq!(exec.msgs[0].type_url, MsgSend::type_url());
        let send = MsgSend::decode(exec.msgs[0].value.as_slice()).unwrap();
        assert_eq!(send.from_address, accounts(1)[0].acc);
        assert_eq!(send.amount[0].amount, "5");
        assert_eq!(exec.msgs[1].type_url, MsgDelegate::type_url());
        let delegate = MsgDelegate::decode(exec.msgs[1].value.as_slice()).unwrap();
        assert_eq!(delegate.delegator_address, send.from_address);
        assert_eq!(delegate.validator_address, accounts(1)[4].val);
        assert_eq!(expected[18].codespace, "staking");
        assert_eq!(expected[19].code, 0);
        assert_eq!(expected[20].codespace, "authz");
    }

    #[test]
    fn authz_grant_validation_script_pins_reachable_rejections() {
        use celestia_proto::cosmos::{
            authz::v1beta1::MsgGrant, bank::v1beta1::SendAuthorization,
            staking::v1beta1::StakeAuthorization,
        };

        let script = read_script(
            include_str!("../fixtures/authz-grant-validation.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected = script_expectations(include_str!(
            "../fixtures/authz-grant-validation.expected.json"
        ))
        .unwrap();
        assert_eq!(script.len(), 4);
        assert_eq!(expected.len(), 4);
        assert!(script.iter().all(|block| block.len() == 1));
        assert!(
            script.iter().zip(&expected).all(|(block, result)| {
                block[0].1.type_url == result.type_url && result.code != 0
            })
        );

        let authorization = |index: usize| {
            MsgGrant::decode(script[index][0].1.value.as_slice())
                .unwrap()
                .grant
                .unwrap()
                .authorization
                .unwrap()
        };
        let zero = SendAuthorization::decode(authorization(0).value.as_slice()).unwrap();
        assert_eq!(zero.spend_limit[0].amount, "0");
        let duplicate = SendAuthorization::decode(authorization(1).value.as_slice()).unwrap();
        assert_eq!(duplicate.allow_list.len(), 2);
        assert_eq!(duplicate.allow_list[0], duplicate.allow_list[1]);
        let negative = StakeAuthorization::decode(authorization(2).value.as_slice()).unwrap();
        assert_eq!(negative.max_tokens.unwrap().amount, "-1");
        let unknown = StakeAuthorization::decode(authorization(3).value.as_slice()).unwrap();
        assert_eq!(unknown.authorization_type, 0);

        assert_eq!(expected[0].codespace, "sdk");
        assert_eq!(expected[1].codespace, "bank");
        assert_eq!(expected[2].codespace, "authz");
        assert_eq!(expected[3].codespace, "authz");
    }

    #[test]
    fn transfer_authz_boundary_scripts_pin_policies_budgets_and_rollback() {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        use celestia_proto::ibc::applications::transfer::v1::{MsgTransfer, TransferAuthorization};

        let script = read_script(
            include_str!("../fixtures/transfer-authz-boundaries.script"),
            ACCOUNTS,
        )
        .unwrap();
        let v9_script = read_script(
            include_str!("../fixtures/transfer-authz-boundaries-v9.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected = script_expectations(include_str!(
            "../fixtures/transfer-authz-boundaries.expected.json"
        ))
        .unwrap();
        let plan = transfer_authz_plan(&script, &expected).unwrap();
        transfer_authz_plan(&v9_script, &expected).unwrap();
        assert_eq!(script, v9_script);
        assert_eq!(script.len(), 28);
        assert_eq!(plan.successful_transfers, 7);
        assert_eq!(plan.granter, accounts(1)[1].acc);
        assert_eq!(plan.grantee, accounts(1)[0].acc);

        let grant = |index: usize| {
            let message = authz::MsgGrant::decode(script[index][0].1.value.as_slice()).unwrap();
            TransferAuthorization::decode(
                message
                    .grant
                    .unwrap()
                    .authorization
                    .unwrap()
                    .value
                    .as_slice(),
            )
            .unwrap()
        };
        let execution = |index: usize| {
            let message = authz::MsgExec::decode(script[index][0].1.value.as_slice()).unwrap();
            MsgTransfer::decode(message.msgs[0].value.as_slice()).unwrap()
        };

        assert!(grant(0).allocations.is_empty());
        assert_eq!(grant(1).allocations.len(), 2);
        assert_eq!(grant(2).allocations[0].spend_limit, []);
        assert_eq!(grant(3).allocations[0].spend_limit[0].amount, "0");
        assert_eq!(grant(4).allocations[0].source_port, "p");
        assert_eq!(grant(5).allocations[0].source_channel, "x");
        assert_eq!(grant(6).allocations[0].allow_list, ["same", "same"]);

        let mut authorization = grant(7);
        let finite = authorization.allocations.remove(0);
        assert_eq!(finite.spend_limit[0].amount, "300");
        assert_eq!(finite.allow_list, ["wrong", "cosmos1receiver"]);
        assert_eq!(execution(8).token.unwrap().amount, "100");
        assert_eq!(execution(9).token.unwrap().amount, "250");
        assert_eq!(execution(10).token.unwrap().amount, "200");
        assert_eq!(
            (expected[11].codespace.as_str(), expected[11].code),
            ("authz", 2)
        );

        assert_eq!(execution(13).receiver, "other");
        assert_eq!(execution(14).memo, "forbidden");
        assert_eq!(execution(15).memo, " \t ");
        assert_eq!(grant(16).allocations[0].allowed_packet_data, ["*", "ok"]);
        assert_eq!(execution(17).memo, "different");
        assert_eq!(execution(18).memo, " ok ");
        assert_eq!(
            grant(19).allocations[0].spend_limit[0].amount,
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
        assert_eq!(grant(22).allocations.len(), 2);
        assert_eq!(grant(22).allocations[1].source_channel, "channel-9");
        let mut authorization = grant(25);
        let final_allocation = authorization.allocations.remove(0);
        assert_eq!(final_allocation.source_channel, "channel-9");
        assert_eq!(final_allocation.spend_limit[0].amount, "100");
        assert_eq!(
            (expected[26].codespace.as_str(), expected[26].code),
            ("channel", 3)
        );
        assert_eq!(
            (expected[27].codespace.as_str(), expected[27].code),
            ("channel", 3)
        );
    }

    #[test]
    fn transfer_authz_plan_rejects_weakened_fixture_contracts() {
        use celestia_proto::cosmos::{authz::v1beta1 as authz, bank::v1beta1::MsgSend};

        let load = || {
            (
                read_script(
                    include_str!("../fixtures/transfer-authz-boundaries.script"),
                    ACCOUNTS,
                )
                .unwrap(),
                script_expectations(include_str!(
                    "../fixtures/transfer-authz-boundaries.expected.json"
                ))
                .unwrap(),
            )
        };

        let (mut script, expected) = load();
        let second_message = script[1][0].clone();
        script[0].push(second_message);
        assert!(transfer_authz_plan(&script, &expected).is_err());

        let (mut script, expected) = load();
        script[7][0].0 = 0;
        assert!(transfer_authz_plan(&script, &expected).is_err());

        let (mut script, expected) = load();
        let mut exec = authz::MsgExec::decode(script[8][0].1.value.as_slice()).unwrap();
        exec.msgs[0].type_url = MsgSend::type_url();
        script[8][0].1.value = exec.encode_to_vec();
        assert!(transfer_authz_plan(&script, &expected).is_err());

        let (script, mut expected) = load();
        expected[26].codespace.clear();
        expected[26].code = 0;
        assert!(transfer_authz_plan(&script, &expected).is_err());
    }

    #[test]
    fn stake_authz_boundary_script_uses_real_validator_and_exact_policies() {
        use celestia_proto::cosmos::{
            authz::v1beta1 as authz,
            staking::v1beta1::{MsgDelegate, StakeAuthorization, stake_authorization::Policy},
        };
        let script = read_script(
            include_str!("../fixtures/stake-authz-boundaries.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected = script_expectations(include_str!(
            "../fixtures/stake-authz-boundaries.expected.json"
        ))
        .unwrap();
        assert_eq!(script.len(), 20);
        assert_eq!(expected.len(), 20);
        assert!(script.iter().all(|block| block.len() == 1));
        for (block, result) in script.iter().zip(&expected) {
            assert_eq!(block[0].1.type_url, result.type_url);
        }

        let decode_grant = |index: usize| {
            let grant = authz::MsgGrant::decode(script[index][0].1.value.as_slice()).unwrap();
            let authorization = grant.grant.unwrap().authorization.unwrap();
            StakeAuthorization::decode(authorization.value.as_slice()).unwrap()
        };
        let decode_delegate = |index: usize| {
            let exec = authz::MsgExec::decode(script[index][0].1.value.as_slice()).unwrap();
            assert_eq!(exec.msgs.len(), 1);
            MsgDelegate::decode(exec.msgs[0].value.as_slice()).unwrap()
        };

        let validator = "celestiavaloper19pxhsd5vj4uf8xa0268z7jmsgrm40n94udw976";
        let missing = "celestiavaloper1jn9nn488lzkyt44u0s9wrcu8stqj2h8p9zcmvy";
        let first = decode_grant(0);
        assert_eq!(first.max_tokens.unwrap().amount, "20");
        assert_eq!(first.authorization_type, 1);
        assert_eq!(
            first.policy,
            Some(Policy::AllowList(
                celestia_proto::cosmos::staking::v1beta1::stake_authorization::Validators {
                    address: vec![validator.into()],
                }
            ))
        );
        assert_eq!(decode_delegate(1).validator_address, validator);
        assert_eq!(decode_delegate(2).amount.unwrap().amount, "16");

        match decode_grant(5).policy.unwrap() {
            Policy::AllowList(v) => assert_eq!(v.address, [missing, validator]),
            Policy::DenyList(_) => panic!("expected allow list"),
        }
        match decode_grant(8).policy.unwrap() {
            Policy::AllowList(v) => assert_eq!(v.address, [validator, missing]),
            Policy::DenyList(_) => panic!("expected allow list"),
        }
        match decode_grant(12).policy.unwrap() {
            Policy::DenyList(v) => assert_eq!(v.address, [validator]),
            Policy::AllowList(_) => panic!("expected deny list"),
        }
        match decode_grant(14).policy.unwrap() {
            Policy::DenyList(v) => assert_eq!(v.address, [missing]),
            Policy::AllowList(_) => panic!("expected deny list"),
        }
        let unlimited = decode_grant(17);
        assert!(unlimited.max_tokens.is_none());
        assert!(unlimited.policy.is_none());

        let v9_script = read_script(
            include_str!("../fixtures/stake-authz-boundaries-v9.script"),
            ACCOUNTS,
        )
        .unwrap();
        assert_eq!(v9_script.len(), 20);
        let v9_validator = "celestiavaloper15xzecda33sep38697tvlq7xc8tcz2sm8dlxmkx";
        let v9_grant = authz::MsgGrant::decode(v9_script[0][0].1.value.as_slice()).unwrap();
        let v9_authorization = StakeAuthorization::decode(
            v9_grant
                .grant
                .unwrap()
                .authorization
                .unwrap()
                .value
                .as_slice(),
        )
        .unwrap();
        match v9_authorization.policy.unwrap() {
            Policy::AllowList(v) => assert_eq!(v.address, [v9_validator]),
            Policy::DenyList(_) => panic!("expected allow list"),
        }
        let v9_exec = authz::MsgExec::decode(v9_script[1][0].1.value.as_slice()).unwrap();
        let v9_delegate = MsgDelegate::decode(v9_exec.msgs[0].value.as_slice()).unwrap();
        assert_eq!(v9_delegate.validator_address, v9_validator);

        assert_eq!(
            (expected[2].codespace.as_str(), expected[2].code),
            ("undefined", 1)
        );
        assert_eq!(
            (expected[4].codespace.as_str(), expected[4].code),
            ("authz", 2)
        );
        assert_eq!(
            (expected[11].codespace.as_str(), expected[11].code),
            ("sdk", 4)
        );
        assert_eq!(
            (expected[13].codespace.as_str(), expected[13].code),
            ("sdk", 4)
        );
    }

    #[test]
    fn stake_authz_operation_scripts_pin_all_staking_message_types() {
        use celestia_proto::cosmos::{
            authz::v1beta1 as authz,
            staking::v1beta1::{
                MsgBeginRedelegate, MsgCancelUnbondingDelegation, MsgUndelegate,
                StakeAuthorization, stake_authorization::Policy,
            },
        };

        let destination = "celestiavaloper1f4znva43r73c9806e0qkgcgzpdl2p8lz4rurq4";
        let granter = "celestia1fnkqcndst4mcynj7psupn6l60flrawltny3q7q";
        let grantee = "celestia1f4znva43r73c9806e0qkgcgzpdl2p8lzsu76kn";
        for (script_text, source, creation_height) in [
            (
                include_str!("../fixtures/stake-authz-operations.script"),
                "celestiavaloper1295za0yx99pqrjqnms4ez7a3gre6u6zugw8nvy",
                66,
            ),
            (
                include_str!("../fixtures/stake-authz-operations-v9.script"),
                "celestiavaloper1p70gu5c3ppwq7al8mp68h8hsxeytvn75tjaptp",
                46,
            ),
        ] {
            let script = read_script(script_text, ACCOUNTS).unwrap();
            let expected = script_expectations(include_str!(
                "../fixtures/stake-authz-operations.expected.json"
            ))
            .unwrap();
            assert_eq!(
                script.iter().map(Vec::len).collect::<Vec<_>>(),
                [1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
            );
            let messages: Vec<_> = script.iter().flatten().collect();
            assert_eq!((messages.len(), expected.len()), (16, 16));
            for ((_, msg, _), outcome) in messages.iter().zip(&expected) {
                assert_eq!(msg.type_url, outcome.type_url);
            }

            let grant = |index: usize| {
                let msg = authz::MsgGrant::decode(messages[index].1.value.as_slice()).unwrap();
                assert_eq!(msg.granter, granter);
                assert_eq!(msg.grantee, grantee);
                StakeAuthorization::decode(
                    msg.grant.unwrap().authorization.unwrap().value.as_slice(),
                )
                .unwrap()
            };
            let inner = |index: usize| {
                let msg = authz::MsgExec::decode(messages[index].1.value.as_slice()).unwrap();
                assert_eq!(msg.grantee, grantee);
                assert_eq!(msg.msgs.len(), 1);
                msg.msgs.into_iter().next().unwrap()
            };
            let policy =
                |authorization: &StakeAuthorization| match authorization.policy.as_ref().unwrap() {
                    Policy::AllowList(v) => (true, v.address.clone()),
                    Policy::DenyList(v) => (false, v.address.clone()),
                };

            let undelegate_grant = grant(2);
            assert_eq!(undelegate_grant.authorization_type, 2);
            assert_eq!(
                undelegate_grant.max_tokens.as_ref().unwrap().amount,
                "20000000"
            );
            assert_eq!(policy(&undelegate_grant), (true, vec![source.to_owned()]));
            for (index, amount) in [
                (3, "5000000"),
                (4, "16000000"),
                (5, "15000000"),
                (6, "1000000"),
            ] {
                let msg = MsgUndelegate::decode(inner(index).value.as_slice()).unwrap();
                assert_eq!(msg.delegator_address, granter);
                assert_eq!(msg.validator_address, source);
                let coin = msg.amount.unwrap();
                assert_eq!(
                    (coin.denom.as_str(), coin.amount.as_str()),
                    ("utia", amount)
                );
            }

            let rejected_redelegate_grant = grant(7);
            assert_eq!(rejected_redelegate_grant.authorization_type, 3);
            assert_eq!(
                policy(&rejected_redelegate_grant),
                (true, vec![source.to_owned()])
            );
            let redelegate_grant = grant(9);
            assert_eq!(redelegate_grant.authorization_type, 3);
            assert_eq!(
                policy(&redelegate_grant),
                (true, vec![destination.to_owned()])
            );
            for (index, amount) in [
                (8, "5000000"),
                (10, "5000000"),
                (11, "15000000"),
                (12, "1000000"),
            ] {
                let msg = MsgBeginRedelegate::decode(inner(index).value.as_slice()).unwrap();
                assert_eq!(msg.delegator_address, granter);
                assert_eq!(msg.validator_src_address, source);
                assert_eq!(msg.validator_dst_address, destination);
                let coin = msg.amount.unwrap();
                assert_eq!(
                    (coin.denom.as_str(), coin.amount.as_str()),
                    ("utia", amount)
                );
            }

            let cancel_grant = grant(13);
            assert_eq!(cancel_grant.authorization_type, 4);
            assert_eq!(cancel_grant.max_tokens.as_ref().unwrap().amount, "5000000");
            assert_eq!(policy(&cancel_grant), (false, vec![destination.to_owned()]));
            for (index, amount) in [(14, "5000000"), (15, "1000000")] {
                let msg =
                    MsgCancelUnbondingDelegation::decode(inner(index).value.as_slice()).unwrap();
                assert_eq!(msg.delegator_address, granter);
                assert_eq!(msg.validator_address, source);
                assert_eq!(msg.creation_height, creation_height);
                let coin = msg.amount.unwrap();
                assert_eq!(
                    (coin.denom.as_str(), coin.amount.as_str()),
                    ("utia", amount)
                );
            }

            assert_eq!(
                (expected[4].codespace.as_str(), expected[4].code),
                ("undefined", 1)
            );
            assert_eq!(
                (expected[8].codespace.as_str(), expected[8].code),
                ("sdk", 4)
            );
            for index in [6, 12, 15] {
                assert_eq!(
                    (expected[index].codespace.as_str(), expected[index].code),
                    ("authz", 2)
                );
            }
        }
    }

    #[test]
    fn evidence_submit_script_pins_stored_and_nil_router_outcomes() {
        use celestia_proto::cosmos::evidence::v1beta1::{Equivocation, MsgSubmitEvidence};
        use prost::Name;

        let v10 = read_script(
            include_str!("../fixtures/evidence-submit-boundaries.script"),
            ACCOUNTS,
        )
        .unwrap();
        let v10_expected = script_expectations(include_str!(
            "../fixtures/evidence-submit-boundaries.expected.json"
        ))
        .unwrap();
        let v9 = read_script(
            include_str!("../fixtures/evidence-submit-boundaries-v9.script"),
            ACCOUNTS,
        )
        .unwrap();
        let v9_expected = script_expectations(include_str!(
            "../fixtures/evidence-submit-boundaries-v9.expected.json"
        ))
        .unwrap();
        assert_eq!((v10.len(), v10_expected.len()), (2, 2));
        assert_eq!((v9.len(), v9_expected.len()), (1, 1));
        assert!(v10.iter().chain(&v9).all(|block| block.len() == 1));

        let decode = |a: &Any| {
            assert_eq!(a.type_url, MsgSubmitEvidence::type_url());
            let msg = MsgSubmitEvidence::decode(a.value.as_slice()).unwrap();
            assert_eq!(msg.submitter, accounts(1)[0].acc);
            let evidence = msg.evidence.unwrap();
            assert_eq!(evidence.type_url, Equivocation::type_url());
            Equivocation::decode(evidence.value.as_slice()).unwrap()
        };
        let stored = decode(&v10[0][0].1);
        assert_eq!((stored.height, stored.power), (19, 52));
        assert_eq!(stored.time.unwrap().seconds, 1_791_962_284);
        assert_eq!(
            stored.consensus_address,
            "celestiavalcons16ywnaneqxr8f84rg3acggyelrym93697hexncq"
        );
        let new_v10 = decode(&v10[1][0].1);
        let new_v9 = decode(&v9[0][0].1);
        assert_eq!(new_v10, new_v9);
        assert_eq!((new_v10.height, new_v10.power), (1, 1));
        assert_eq!(new_v10.time.unwrap().seconds, 2_000_000_000);

        assert_eq!(
            (v10_expected[0].codespace.as_str(), v10_expected[0].code),
            ("evidence", 5)
        );
        for expected in [&v10_expected[1], &v9_expected[0]] {
            assert_eq!(
                (expected.codespace.as_str(), expected.code),
                ("undefined", 111_222)
            );
            assert!(expected.log_contains.contains("nil pointer dereference"));
        }
    }

    #[test]
    fn evidence_validation_script_pins_each_reachable_boundary() {
        use celestia_proto::cosmos::evidence::v1beta1::{Equivocation, MsgSubmitEvidence};
        use prost::Name;

        let script = read_script(
            include_str!("../fixtures/evidence-validation-boundaries.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected = script_expectations(include_str!(
            "../fixtures/evidence-validation-boundaries.expected.json"
        ))
        .unwrap();
        assert_eq!((script.len(), expected.len()), (5, 5));
        assert!(script.iter().all(|block| block.len() == 1));

        let decode = |index: usize| {
            let any = &script[index][0].1;
            assert_eq!(any.type_url, MsgSubmitEvidence::type_url());
            let msg = MsgSubmitEvidence::decode(any.value.as_slice()).unwrap();
            assert_eq!(msg.submitter, accounts(1)[0].acc);
            msg
        };
        assert!(decode(0).evidence.is_none());
        let equivocation = |index: usize| {
            let evidence = decode(index).evidence.unwrap();
            assert_eq!(evidence.type_url, Equivocation::type_url());
            Equivocation::decode(evidence.value.as_slice()).unwrap()
        };
        assert_eq!(equivocation(1).time.unwrap().seconds, 0);
        assert_eq!(equivocation(2).height, 0);
        assert_eq!(equivocation(3).power, 0);
        assert!(equivocation(4).consensus_address.is_empty());

        let markers = [
            "missing evidence",
            "invalid equivocation time",
            "invalid equivocation height",
            "invalid equivocation validator power",
            "invalid equivocation validator consensus address",
        ];
        for ((block, result), marker) in script.iter().zip(&expected).zip(markers) {
            assert_eq!(block[0].1.type_url, result.type_url);
            assert_eq!((result.codespace.as_str(), result.code), ("evidence", 3));
            assert!(result.log_contains.contains(marker));
        }
    }

    #[test]
    fn evidence_zero_time_governance_script_pins_persisted_error_path() {
        use celestia_proto::cosmos::{
            evidence::v1beta1::{Equivocation, MsgSubmitEvidence},
            gov::v1::{MsgSubmitProposal, MsgVote},
        };
        use prost::Name;

        let script = read_script(
            include_str!("../fixtures/evidence-zero-time-governance.script"),
            ACCOUNTS,
        )
        .unwrap();
        let expected = script_expectations(include_str!(
            "../fixtures/evidence-zero-time-governance.expected.json"
        ))
        .unwrap();
        assert_eq!((script.len(), expected.len()), (2, 12));
        assert_eq!((script[0].len(), script[1].len()), (1, 11));
        assert_eq!(script[0][0].1.type_url, MsgSubmitProposal::type_url());
        assert!(script[1].iter().all(|(_, msg, _)| msg.type_url == "vote"));
        assert!(
            expected[1..]
                .iter()
                .all(|result| result.type_url == MsgVote::type_url())
        );

        let proposal = MsgSubmitProposal::decode(script[0][0].1.value.as_slice()).unwrap();
        assert!(proposal.expedited);
        assert_eq!(proposal.messages.len(), 1);
        let submitted = &proposal.messages[0];
        assert_eq!(submitted.type_url, MsgSubmitEvidence::type_url());
        let submitted = MsgSubmitEvidence::decode(submitted.value.as_slice()).unwrap();
        assert_eq!(
            submitted.submitter,
            "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
        );
        let evidence = submitted.evidence.unwrap();
        assert_eq!(evidence.type_url, Equivocation::type_url());
        let evidence = Equivocation::decode(evidence.value.as_slice()).unwrap();
        assert_eq!(evidence.time.unwrap().seconds, 0);
        assert_eq!((evidence.height, evidence.power), (1, 1));
        assert!(!evidence.consensus_address.is_empty());
    }

    use super::*;

    #[test]
    fn rejected_upgrade_timeout_checkpoint_rejects_each_raw_mutation() {
        let mut client_prefix = BTreeMap::new();
        client_prefix.insert(b"clients/07-tendermint-0/clientState".to_vec(), vec![5]);
        let expected = UpgradeTimeoutCheckpoint {
            channel: vec![1],
            next_sequence_send: 1u64.to_be_bytes().to_vec(),
            upgrade: Some(vec![2]),
            counterparty_upgrade: None,
            error_receipt: None,
            client_prefix,
        };
        assert!(ensure_upgrade_timeout_checkpoint_unchanged(&expected, &expected).is_ok());

        let mut mutations = Vec::new();
        let mut changed = expected.clone();
        changed.channel.push(9);
        mutations.push(changed);
        let mut changed = expected.clone();
        changed.next_sequence_send = 2u64.to_be_bytes().to_vec();
        mutations.push(changed);
        let mut changed = expected.clone();
        changed.upgrade.as_mut().unwrap().push(9);
        mutations.push(changed);
        let mut changed = expected.clone();
        changed.counterparty_upgrade = Some(vec![9]);
        mutations.push(changed);
        let mut changed = expected.clone();
        changed.error_receipt = Some(vec![9]);
        mutations.push(changed);
        let mut changed = expected.clone();
        changed.client_prefix.insert(
            b"clients/07-tendermint-0/consensusStates/1-9".to_vec(),
            vec![9],
        );
        mutations.push(changed);

        for observed in mutations {
            assert!(ensure_upgrade_timeout_checkpoint_unchanged(&expected, &observed).is_err());
        }
    }

    #[test]
    fn required_upgrade_timeout_recovery_needs_four_clean_checktx_results() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("upgrade_timeout_recovery");
        for event in [
            "upgrade_timeout_try",
            "upgrade_timeout_rejected",
            "upgrade_timeout_cleaned",
            "upgrade_timeout_recovery_ack",
            "upgrade_timeout_recovery_state",
        ] {
            stats.ibc.insert(event, 1);
        }
        stats.ibc.insert("upgrade_timeout_check_ok", 3);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("upgrade_timeout_check_ok", 4);
        assert!(stats.require_ibc_coverage().is_ok());

        stats
            .ibc
            .insert("upgrade_timeout_check_account_sequence", 1);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_upgrade_timeout_boundary_needs_exact_credits() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("upgrade_timeout_boundary");
        for event in [
            "upgrade_timeout_try",
            "upgrade_timeout_boundary_rejected",
            "upgrade_timeout_cleaned",
            "upgrade_timeout_recovery_ack",
            "upgrade_timeout_recovery_state",
        ] {
            stats.ibc.insert(event, 1);
        }
        stats.ibc.insert("upgrade_timeout_check_ok", 3);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("upgrade_timeout_check_ok", 4);
        assert!(stats.require_ibc_coverage().is_ok());

        stats.ibc.insert("upgrade_timeout_unexpected_result", 1);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_upgrade_confirm_timeout_needs_three_clean_checktx_results() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("upgrade_confirm_timeout");
        for event in [
            "upgrade_timeout_try",
            "upgrade_confirm_timeout_aborted",
            "upgrade_timeout_recovery_ack",
            "upgrade_timeout_recovery_state",
        ] {
            stats.ibc.insert(event, 1);
        }
        stats.ibc.insert("upgrade_timeout_check_ok", 2);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("upgrade_timeout_check_ok", 3);
        assert!(stats.require_ibc_coverage().is_ok());

        stats.ibc.insert("upgrade_confirm_timeout_invalid_proof", 1);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_upgrade_try_fast_forward_needs_each_exact_credit() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("upgrade_try_fast_forward");
        for event in [
            "upgrade_timeout_try",
            "upgrade_try_fast_forward_confirm",
            "upgrade_try_fast_forward_stale_open_rejected",
            "upgrade_try_fast_forward_open",
            "upgrade_timeout_recovery_ack",
            "upgrade_timeout_recovery_state",
        ] {
            stats.ibc.insert(event, 1);
        }
        stats.ibc.insert("upgrade_timeout_check_ok", 4);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("upgrade_timeout_check_ok", 5);
        assert!(stats.require_ibc_coverage().is_ok());

        stats.ibc.insert("upgrade_timeout_unexpected_result", 1);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_upgrade_open_flushcomplete_needs_each_exact_credit() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("upgrade_open_flushcomplete");
        for event in [
            "upgrade_timeout_try",
            "upgrade_open_flushcomplete_confirm",
            "upgrade_open_flushcomplete_open",
            "upgrade_timeout_recovery_ack",
            "upgrade_timeout_recovery_state",
        ] {
            stats.ibc.insert(event, 1);
        }
        stats.ibc.insert("upgrade_timeout_check_ok", 3);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("upgrade_timeout_check_ok", 4);
        assert!(stats.require_ibc_coverage().is_ok());

        stats
            .ibc
            .insert("upgrade_open_flushcomplete_invalid_proof", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.remove("upgrade_open_flushcomplete_invalid_proof");
        stats
            .ibc
            .insert("upgrade_timeout_check_account_sequence", 1);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn evidence_requirement_is_exact_and_fail_closed() {
        assert!(Stats::default().require_evidence_coverage().is_ok());

        let mut stats = Stats {
            evidence_required: Some("stale".into()),
            ..Stats::default()
        };
        assert!(stats.require_evidence_coverage().is_err());
        stats
            .evidence_injected
            .insert("removed-validator".into(), 1);
        assert!(stats.require_evidence_coverage().is_err());
        stats.evidence_injected.clear();
        stats.evidence_injected.insert("stale".into(), 2);
        assert!(stats.require_evidence_coverage().is_err());
        stats.evidence_injected.insert("stale".into(), 1);
        assert!(stats.require_evidence_coverage().is_ok());
        stats
            .evidence_injected
            .insert("removed-validator".into(), 1);
        assert!(stats.require_evidence_coverage().is_err());
    }

    #[test]
    fn zero_slash_misbehavior_uses_recorded_positive_power_validator() {
        let address = vec![7; 20];
        let time = pbjson_types::Timestamp {
            seconds: 123,
            nanos: 456,
        };
        let valsets = BTreeMap::from([(9, vec![(address.clone(), 53, false)])]);
        let times = BTreeMap::from([(9, time)]);
        let evidence = active_double_sign_misbehavior(10, &valsets, &times).unwrap();
        assert_eq!(evidence.r#type, 1);
        assert_eq!(evidence.height, 9);
        assert_eq!(evidence.time, Some(time));
        assert_eq!(evidence.total_voting_power, 53);
        assert_eq!(
            evidence.validator.unwrap(),
            abci::Validator { address, power: 53 }
        );
        assert!(active_double_sign_misbehavior(10, &BTreeMap::new(), &times).is_err());
        assert!(active_double_sign_misbehavior(10, &valsets, &BTreeMap::new()).is_err());
        assert!(active_double_sign_misbehavior(i64::MIN, &valsets, &times).is_err());
        let zero = BTreeMap::from([(9, vec![(vec![8; 20], 0, false)])]);
        assert!(active_double_sign_misbehavior(10, &zero, &times).is_err());
    }

    #[test]
    fn stake_lifecycle_scripts_pin_the_required_topology() {
        for (script_text, expected_text, source) in [
            (
                include_str!("../fixtures/stake-slash-lifecycle.script"),
                include_str!("../fixtures/stake-slash-lifecycle.expected.json"),
                "celestiavaloper1295za0yx99pqrjqnms4ez7a3gre6u6zugw8nvy",
            ),
            (
                include_str!("../fixtures/stake-slash-lifecycle-v9.script"),
                include_str!("../fixtures/stake-slash-lifecycle-v9.expected.json"),
                "celestiavaloper1p70gu5c3ppwq7al8mp68h8hsxeytvn75tjaptp",
            ),
        ] {
            let script = read_script(script_text, ACCOUNTS).unwrap();
            let expected = script_expectations(expected_text).unwrap();
            let plan = stake_lifecycle_plan(&script).unwrap();
            assert_eq!(
                script.iter().map(Vec::len).collect::<Vec<_>>(),
                [1, 1, 0, 1, 1, 1]
            );
            assert_eq!(expected.len(), 5);
            assert!(expected.iter().all(|result| result.code == 0));
            assert_eq!(plan.source, source);
            assert_eq!(
                plan.destination,
                "celestiavaloper1f4znva43r73c9806e0qkgcgzpdl2p8lz4rurq4"
            );
            assert_eq!(
                plan.delegator,
                "celestia1fnkqcndst4mcynj7psupn6l60flrawltny3q7q"
            );

            let mut missing_wait = script.clone();
            missing_wait.remove(2);
            assert!(stake_lifecycle_plan(&missing_wait).is_err());
        }
    }

    #[test]
    fn stake_lifecycle_uses_cosmos_share_rounding_before_integer_truncation() {
        let shares = LegacyDec::unmarshal(b"2000000000000000000").unwrap();
        let total = LegacyDec::unmarshal(b"2000000000000000001").unwrap();
        let tokens = Int::one();
        assert_eq!(
            tokens_from_shares(&shares, &tokens, &total).unwrap(),
            tokens
        );
        let incorrectly_truncated = shares
            .checked_mul_int(&tokens)
            .unwrap()
            .checked_quo_truncate(&total)
            .unwrap()
            .truncate_int();
        assert_eq!(incorrectly_truncated, Int::zero());
    }

    #[test]
    fn controlled_stale_misbehavior_uses_recorded_validator_and_time() {
        let time = pbjson_types::Timestamp {
            seconds: 1_800_000_000,
            nanos: 123,
        };
        let valsets =
            BTreeMap::from([(8, vec![(vec![7; 20], 41, false), (vec![8; 20], 11, false)])]);
        let times = BTreeMap::from([(8, time)]);
        let evidence = stale_misbehavior(10, &valsets, &times).unwrap();
        assert_eq!(evidence.r#type, 1);
        assert_eq!(evidence.height, 8);
        assert_eq!(evidence.time, Some(time));
        assert_eq!(evidence.total_voting_power, 52);
        assert_eq!(evidence.validator.unwrap().address, vec![7; 20]);

        assert!(stale_misbehavior(10, &BTreeMap::new(), &times).is_err());
        assert!(stale_misbehavior(10, &valsets, &BTreeMap::new()).is_err());
        assert!(stale_misbehavior(i64::MIN, &valsets, &times).is_err());
    }

    #[test]
    fn controlled_removed_validator_misbehavior_is_well_formed() {
        let time = pbjson_types::Timestamp {
            seconds: 1_800_000_000,
            nanos: 0,
        };
        let evidence = removed_validator_misbehavior(12, time, 53);
        assert_eq!(evidence.r#type, 1);
        assert_eq!(evidence.height, 12);
        assert_eq!(evidence.time, Some(time));
        assert_eq!(evidence.total_voting_power, 53);
        let validator = evidence.validator.unwrap();
        assert_eq!(validator.address, vec![0xa5; 20]);
        assert_eq!(validator.power, 1);
    }

    #[test]
    fn handler_errors_use_outer_top_level_index_and_retain_unattributed_failures() {
        let inner = Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend::default());
        let exec = Any::pack(&celestia_proto::cosmos::authz::v1beta1::MsgExec {
            msgs: vec![inner],
            ..Default::default()
        });
        let delegate = Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate::default());
        let body = TxBody {
            messages: vec![exec, delegate.clone()],
            ..Default::default()
        };
        let tx = wire_tx(&body.encode_to_vec(), &[]);
        for version in [9, 10] {
            let mut stats = Stats::default();
            let failure = |log: &str| abci::ExecTxResult {
                code: 2,
                codespace: "staking".into(),
                log: log.into(),
                ..Default::default()
            };
            observe_handler_error(
                &mut stats,
                version,
                &tx,
                &failure(
                    "failed to execute message; message index: 1: inner: failed to execute message; message index: 0: detail",
                ),
            );
            assert_eq!(
                stats.handler_errors,
                BTreeMap::from([(format!("{} staking/2", delegate.type_url), 1)])
            );
            for log in [
                "signature verification failed",
                "user text: failed to execute message; message index: 0: detail",
                "failed to execute message; message index: 2: outside tx",
                "failed to execute message; message index: -1: negative",
                "failed to execute message; message index: +1: non-SDK syntax",
                "failed to execute message; message index: : empty",
                "failed to execute message; message index: 999999999999999999999999999: overflow",
                "recovered: panic",
            ] {
                observe_handler_error(&mut stats, version, &tx, &failure(log));
            }
            observe_handler_error(
                &mut stats,
                version,
                &[0xff],
                &failure("failed to execute message; message index: 0: undecodable tx"),
            );
            observe_handler_error(&mut stats, version, &tx, &abci::ExecTxResult::default());
            assert_eq!(stats.handler_errors.len(), 1);
            assert_eq!(stats.unattributed_errors["staking/2"], 9);
            assert_eq!(
                stats.handler_errors.values().sum::<u64>()
                    + stats.unattributed_errors.values().sum::<u64>(),
                10
            );
        }
    }

    #[test]
    fn rejection_coverage_requires_a_complete_ordered_same_message_pair() {
        let case = hyperlane::Rejection::MissingSignature;
        let txs = vec![vec![1], vec![2]];
        let rejected = abci::ExecTxResult {
            code: 1,
            log: case.error_marker().into(),
            ..Default::default()
        };
        let accepted = abci::ExecTxResult::default();
        let base = || Stats {
            hl_rejection_cases: vec![case.name().into()],
            ..Default::default()
        };
        assert!(base().hl_rejection_setup());
        let mut stats = base();
        stats.observe_hl_pair(
            case,
            &[1],
            &[2],
            &txs,
            &[rejected.clone(), accepted.clone()],
        );
        assert_eq!(stats.hl_retried[case.name()], 1);
        assert!(!stats.hl_rejection_setup());
        assert!(stats.require_hl_coverage().is_ok());
        let wrong_stage = abci::ExecTxResult {
            code: 1,
            log: "out of gas".into(),
            ..Default::default()
        };
        for (txs, results) in [
            (vec![vec![1]], vec![rejected.clone()]),
            (
                vec![vec![2], vec![1]],
                vec![accepted.clone(), rejected.clone()],
            ),
            (
                vec![vec![1], vec![3], vec![2]],
                vec![rejected.clone(), accepted.clone(), accepted.clone()],
            ),
            (txs.clone(), vec![accepted.clone(), accepted.clone()]),
            (txs.clone(), vec![wrong_stage, accepted]),
            (txs, vec![rejected.clone(), rejected]),
        ] {
            let mut stats = base();
            stats.observe_hl_pair(case, &[1], &[2], &txs, &results);
            assert!(stats.hl_retried.is_empty());
            assert_eq!(stats.hl_rejection_failures.len(), 1);
            // A later successful pair must not hide an earlier failure.
            stats.hl_retried.insert(case.name().into(), 1);
            assert!(stats.require_hl_coverage().is_err());
        }
    }

    #[test]
    fn required_governance_payloads_need_actual_execution() {
        let bank = "/cosmos.bank.v1beta1.MsgSetSendEnabled";
        let circuit = "/cosmos.circuit.v1.MsgAuthorizeCircuitBreaker";
        let mut stats = Stats {
            gov_required: vec![bank.into(), circuit.into()],
            ..Stats::default()
        };
        assert!(stats.gov_setup());
        stats
            .gov_executed
            .insert("/hyperlane.warp.v1.MsgCreateNativeSyntheticToken".into(), 1);
        assert!(stats.require_gov_coverage().is_err());
        stats.gov_executed.insert(bank.into(), 0);
        assert!(stats.require_gov_coverage().is_err());
        stats.gov_executed.insert(bank.into(), 1);
        assert!(stats.gov_setup());
        assert!(stats.require_gov_coverage().is_err());
        stats.gov_executed.insert(circuit.into(), 1);
        assert!(!stats.gov_setup());
        assert!(stats.require_gov_coverage().is_ok());
        assert!(Stats::default().require_gov_coverage().is_ok());
    }

    #[test]
    fn required_upgrade_plan_lifecycle_needs_every_exact_execution() {
        let complete = || {
            let mut stats = Stats {
                upgrade_plan_required: Some(()),
                upgrade_plan_completed: 1,
                upgrade_plan_rejections: 5,
                ..Stats::default()
            };
            for (url, count) in [
                ("/cosmos.circuit.v1.MsgResetCircuitBreaker", 1),
                ("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade", 2),
                ("/cosmos.upgrade.v1beta1.MsgCancelUpgrade", 2),
            ] {
                stats.gov_executed.insert(url.into(), count);
            }
            stats
        };
        let stats = complete();
        assert!(stats.require_upgrade_plan_coverage().is_ok());
        for url in [
            "/cosmos.circuit.v1.MsgResetCircuitBreaker",
            "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
            "/cosmos.upgrade.v1beta1.MsgCancelUpgrade",
        ] {
            let mut incomplete = complete();
            incomplete.gov_executed.remove(url);
            assert!(incomplete.require_upgrade_plan_coverage().is_err());
        }
        let mut stats = complete();
        stats.upgrade_plan_completed = 0;
        assert!(stats.require_upgrade_plan_coverage().is_err());
        let mut stats = complete();
        stats.upgrade_plan_rejections = 4;
        assert!(stats.require_upgrade_plan_coverage().is_err());
        assert!(Stats::default().require_upgrade_plan_coverage().is_ok());
    }

    #[test]
    fn required_upgrade_halt_needs_exact_governance_and_terminal_credit() {
        let complete = || {
            let mut stats = Stats {
                upgrade_halt_required: Some(()),
                upgrade_halt_completed: 1,
                ..Stats::default()
            };
            stats
                .gov_executed
                .insert("/cosmos.circuit.v1.MsgResetCircuitBreaker".into(), 1);
            stats
                .gov_executed
                .insert("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade".into(), 1);
            stats
        };
        assert!(complete().require_upgrade_halt_coverage().is_ok());
        for url in [
            "/cosmos.circuit.v1.MsgResetCircuitBreaker",
            "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
        ] {
            let mut incomplete = complete();
            incomplete.gov_executed.remove(url);
            assert!(incomplete.require_upgrade_halt_coverage().is_err());
        }
        let mut incomplete = complete();
        incomplete.upgrade_halt_completed = 0;
        assert!(incomplete.require_upgrade_halt_coverage().is_err());
        assert!(Stats::default().require_upgrade_halt_coverage().is_ok());
    }

    #[test]
    fn required_upgrade_skip_needs_exact_governance_and_commit_credit() {
        let complete = || {
            let mut stats = Stats {
                upgrade_skip_required: Some(()),
                upgrade_skip_completed: 1,
                ..Stats::default()
            };
            stats
                .gov_executed
                .insert("/cosmos.circuit.v1.MsgResetCircuitBreaker".into(), 1);
            stats
                .gov_executed
                .insert("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade".into(), 1);
            stats
        };
        assert!(complete().require_upgrade_skip_coverage().is_ok());
        for url in [
            "/cosmos.circuit.v1.MsgResetCircuitBreaker",
            "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
        ] {
            let mut incomplete = complete();
            incomplete.gov_executed.remove(url);
            assert!(incomplete.require_upgrade_skip_coverage().is_err());
        }
        let mut incomplete = complete();
        incomplete.upgrade_skip_completed = 0;
        assert!(incomplete.require_upgrade_skip_coverage().is_err());
        assert!(Stats::default().require_upgrade_skip_coverage().is_ok());
    }

    #[test]
    fn required_upgrade_restart_needs_exact_governance_and_recovery_credit() {
        let complete = || {
            let mut stats = Stats {
                upgrade_restart_required: Some(()),
                upgrade_restart_completed: 1,
                ..Stats::default()
            };
            stats
                .gov_executed
                .insert("/cosmos.circuit.v1.MsgResetCircuitBreaker".into(), 1);
            stats
                .gov_executed
                .insert("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade".into(), 1);
            stats
        };
        assert!(complete().require_upgrade_restart_coverage().is_ok());
        for url in [
            "/cosmos.circuit.v1.MsgResetCircuitBreaker",
            "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
        ] {
            let mut incomplete = complete();
            incomplete.gov_executed.remove(url);
            assert!(incomplete.require_upgrade_restart_coverage().is_err());
        }
        let mut incomplete = complete();
        incomplete.upgrade_restart_completed = 0;
        assert!(incomplete.require_upgrade_restart_coverage().is_err());
        assert!(Stats::default().require_upgrade_restart_coverage().is_ok());
    }

    #[test]
    fn required_governance_failure_needs_the_exact_go_log_marker() {
        let mut stats = Stats {
            gov_failure_required: Some("upgrade.go:1048".into()),
            ..Stats::default()
        };
        assert!(stats.require_gov_coverage().is_err());
        stats.gov_failure_matches = 1;
        assert!(stats.require_gov_coverage().is_ok());
    }

    #[test]
    fn required_all_abstain_rejection_needs_a_committed_tally() {
        use celestia_proto::cosmos::gov::v1::TallyResult;

        let mut stats = Stats {
            gov_abstain_required: Some(()),
            ..Stats::default()
        };
        assert!(stats.require_gov_coverage().is_err());
        stats.gov_abstain_rejections = 1;
        assert!(stats.require_gov_coverage().is_ok());

        let tally = |abstain: &str, yes: &str, no: &str, veto: &str| TallyResult {
            abstain_count: abstain.into(),
            yes_count: yes.into(),
            no_count: no.into(),
            no_with_veto_count: veto.into(),
        };
        assert!(tally_is_all_abstain(&tally("1", "0", "0", "0")));
        for value in ["", "00", "01", "-1", "+1", "one"] {
            assert!(!tally_is_all_abstain(&tally(value, "0", "0", "0")));
        }
        for (yes, no, veto) in [("1", "0", "0"), ("0", "1", "0"), ("0", "0", "1")] {
            assert!(!tally_is_all_abstain(&tally("1", yes, no, veto)));
        }
    }

    #[test]
    fn required_message_types_need_a_successful_finalized_transaction() {
        let deposit = "/cosmos.gov.v1beta1.MsgDeposit";
        let mut stats = Stats {
            type_required: vec![deposit.into()],
            ..Stats::default()
        };
        assert!(stats.require_type_coverage().is_err());
        stats.per_type.insert(format!(">{deposit}"), (1, 1, 0));
        assert!(stats.require_type_coverage().is_err());
        stats.per_type.insert(deposit.into(), (1, 0, 0));
        assert!(stats.require_type_coverage().is_err());
        stats.per_type.insert(deposit.into(), (1, 1, 0));
        assert!(stats.require_type_coverage().is_ok());
        assert!(Stats::default().require_type_coverage().is_ok());

        assert!(type_requirements(None).unwrap().is_empty());
        for invalid in ["", ",", "MsgDeposit", "/", "/cosmos gov.MsgDeposit"] {
            assert!(type_requirements(Some(invalid)).is_err(), "{invalid:?}");
        }
        assert_eq!(
            type_requirements(Some(&format!(" {deposit},{deposit} "))).unwrap(),
            vec![deposit]
        );
    }

    #[test]
    fn governance_requirement_is_explicit_and_never_silently_empty() {
        assert!(gov_requirements(None, false).unwrap().is_empty());
        let bank = "/cosmos.bank.v1beta1.MsgSetSendEnabled";
        assert!(gov_requirements(Some(bank), false).is_err());
        for invalid in [
            "",
            ",",
            " , ",
            "MsgSetSendEnabled",
            "/",
            "/cosmos bank.Msg",
            "/a,,/b",
        ] {
            assert!(
                gov_requirements(Some(invalid), true).is_err(),
                "{invalid:?}"
            );
        }
        assert_eq!(
            gov_requirements(Some(&format!(" {bank},{bank} ")), true).unwrap(),
            vec![bank]
        );
        assert!(!allow_postupgrade_hl(None, false, false).unwrap());
        assert!(allow_postupgrade_hl(Some("yes"), true, true).is_err());
        assert!(allow_postupgrade_hl(Some("governance-fixture"), false, true).is_err());
        assert!(allow_postupgrade_hl(Some("governance-fixture"), true, false).is_err());
        assert!(allow_postupgrade_hl(Some("governance-fixture"), true, true).unwrap());
    }

    #[test]
    fn required_governance_setup_waits_until_dynamic_payload_exists() {
        let recover = "/ibc.core.client.v1.MsgRecoverClient";
        let mut stats = Stats {
            gov_required: vec![recover.into()],
            ..Stats::default()
        };
        assert!(stats.gov_setup());
        assert!(!gov_setup_ready(&stats, &[]));
        let messages = vec![Any {
            type_url: recover.into(),
            value: Vec::new(),
        }];
        assert!(gov_setup_ready(&stats, &messages));
        stats.gov_executed.insert(recover.into(), 1);
        assert!(!gov_setup_ready(&stats, &messages));

        let static_required = "/cosmos.bank.v1beta1.MsgSetSendEnabled";
        let static_stats = Stats {
            gov_required: vec![static_required.into()],
            ..Stats::default()
        };
        assert!(gov_setup_ready(&static_stats, &[]));
    }

    #[test]
    fn registry_requirement_reserves_setup_until_every_exact_path_lands() {
        let mut stats = Stats {
            hl_registry_required: Some(()),
            ..Stats::default()
        };
        assert!(stats.hl_registry_setup());
        stats.hl_registry.insert("merkle-root-ism".into(), 1);
        stats.hl_registry.insert("validator-announcement".into(), 1);
        assert!(!stats.hl_registry_setup());
    }

    #[test]
    fn signed_verification_reserves_setup_until_both_exact_paths_land() {
        let mut stats = Stats {
            hl_verification: Some("message-id".into()),
            ..Stats::default()
        };
        assert!(stats.hl_verification_setup());
        stats.hl_verified.insert("collateral".into(), 1);
        assert!(stats.hl_verification_setup());
        stats.hl_verified.insert("synthetic".into(), 1);
        assert!(!stats.hl_verification_setup());
    }

    #[test]
    fn pfm_requirement_reserves_setup_until_the_exact_original_ack_lands() {
        let mut success = Stats::default();
        success.ibc_required.insert("pfm_timeout_lifecycle");
        assert!(success.ibc_pfm_setup());
        success.ibc.insert("pfm_original_ack", 1);
        assert!(!success.ibc_pfm_setup());

        let mut error = Stats::default();
        error.ibc_required.insert("pfm_exhaust_lifecycle");
        assert!(error.ibc_pfm_setup());
        error.ibc.insert("pfm_original_error_ack", 1);
        assert!(!error.ibc_pfm_setup());
    }

    #[test]
    fn recover_requirement_reserves_setup_through_the_post_recovery_update() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("recover_client");
        assert!(stats.ibc_recover_setup());
        stats.ibc.insert("recover_client", 1);
        assert!(stats.ibc_recover_setup());
        stats.ibc.insert("recovered_client_update", 1);
        assert!(!stats.ibc_recover_setup());
    }

    #[test]
    fn required_ibc_ack_failure_needs_the_decoded_failure_response() {
        let mut stats = Stats {
            ibc_ack_failure_required: true,
            ..Stats::default()
        };
        // A code-0 Ack transaction without response attribution is not
        // handshake or negative-path coverage.
        stats.per_type.insert(
            "/ibc.core.channel.v1.MsgChannelUpgradeAck".into(),
            (1, 1, 0),
        );
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("chan_upgrade_ack_failure", 1);
        assert!(stats.require_ibc_coverage().is_ok());
        assert!(Stats::default().require_ibc_coverage().is_ok());
    }

    #[test]
    fn legacy_upgrade_fixture_requires_the_exact_trusted_height_event() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("legacy_upgrade_fixture");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("legacy_upgrade_fixture_trusted_height", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn legacy_upgrade_fixture_pauses_script_and_time_jump_until_anchor() {
        let mut paused = 0;
        assert_eq!(
            legacy_upgrade_fixture_block_index(53, &mut paused, true, false, 54),
            (53, false)
        );
        assert_eq!(
            legacy_upgrade_fixture_block_index(54, &mut paused, true, false, 54),
            (54, true)
        );
        assert_eq!(
            legacy_upgrade_fixture_block_index(55, &mut paused, true, false, 54),
            (54, true)
        );
        assert_eq!(
            legacy_upgrade_fixture_block_index(56, &mut paused, true, true, 54),
            (54, false)
        );
        assert_eq!(
            legacy_upgrade_fixture_block_index(57, &mut paused, true, true, 54),
            (55, false)
        );

        let mut ordinary_paused = 0;
        assert_eq!(
            legacy_upgrade_fixture_block_index(54, &mut ordinary_paused, false, false, 54),
            (54, false)
        );
    }

    #[test]
    fn legacy_upgrade_fixture_votes_use_the_pinned_submission() {
        assert_eq!(script_vote_proposal_id(true, Some(9), &[9, 10]).unwrap(), 9);
        assert!(script_vote_proposal_id(true, None, &[9, 10]).is_err());
        assert_eq!(script_vote_proposal_id(false, None, &[9, 10]).unwrap(), 10);
        assert_eq!(script_vote_proposal_id(false, None, &[]).unwrap(), 1);
    }

    #[test]
    fn legacy_upgrade_fixture_pins_only_its_successful_typed_response() {
        let expected = b"fixture submission";
        let response = Any::pack(
            &celestia_proto::cosmos::gov::v1::MsgSubmitProposalResponse { proposal_id: 9 },
        );
        let mut result = abci::ExecTxResult {
            data: celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
                msg_responses: vec![response],
                ..Default::default()
            }
            .encode_to_vec()
            .into(),
            ..Default::default()
        };

        assert_eq!(
            finalized_script_proposal_id(expected, expected, &result).unwrap(),
            Some(9)
        );
        assert_eq!(
            finalized_script_proposal_id(expected, b"unrelated submission", &result).unwrap(),
            None
        );
        result.code = 1;
        assert_eq!(
            finalized_script_proposal_id(expected, expected, &result).unwrap(),
            None
        );
    }

    #[test]
    fn required_ibc_close_init_failure_needs_rejection_and_channel_reuse() {
        let mut stats = Stats { ..Stats::default() };
        stats.ibc_required.insert("close_init_failure");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("close_init_failure", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("close_init_reused_closed", 1);
        assert!(stats.require_ibc_coverage().is_ok());
        assert!(Stats::default().require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_recovery_needs_the_matching_block_event() {
        let mut stats = Stats { ..Stats::default() };
        stats.ibc_required.insert("recover_client");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("recover_client", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("misbehaviour_confirmed", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("recovered_client_update", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_pfm_needs_forward_and_original_ack() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("pfm_lifecycle");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_forwarded", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_downstream_ack", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_original_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
        assert!(Stats::default().require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_pfm_error_needs_forward_and_both_error_ack_events() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("pfm_error_lifecycle");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_forwarded", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_downstream_ack", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_original_error_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_pfm_timeout_needs_retry_and_final_ack_lifecycle() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("pfm_timeout_lifecycle");
        for event in [
            "pfm_forwarded",
            "pfm_timeout",
            "pfm_retry",
            "pfm_downstream_ack",
        ] {
            assert!(stats.require_ibc_coverage().is_err());
            stats.ibc.insert(event, 1);
        }
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_original_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_pfm_exhaustion_needs_both_timeouts_and_refund_ack() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("pfm_exhaust_lifecycle");
        for event in [
            "pfm_forwarded",
            "pfm_timeout",
            "pfm_retry",
            "pfm_retry_exhausted",
        ] {
            assert!(stats.require_ibc_coverage().is_err());
            stats.ibc.insert(event, 1);
        }
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_original_error_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ibc_pfm_metadata_needs_events_and_committed_state() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("pfm_metadata_lifecycle");
        for event in [
            "pfm_metadata_parse_ack",
            "pfm_metadata_validate_ack",
            "pfm_metadata_null_control",
        ] {
            assert!(stats.require_ibc_coverage().is_err());
            stats.ibc.insert(event, 1);
        }
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("pfm_metadata_committed_state", 1);
        assert!(stats.require_ibc_coverage().is_ok());
        stats.ibc.insert("pfm_metadata_parse_ack", 2);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_ica_encodings_need_exact_success_ack_in_both_encodings() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("ica_encoding_lifecycle");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("ica_proto3_success_ack", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("ica_proto3json_success_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_ica_order_needs_each_transition_and_exact_ack_once() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("ica_order_lifecycle");
        for event in [
            "ica_unordered_baseline_success_ack",
            "ica_upgrade_to_ordered",
            "ica_ordered_success_ack",
            "ica_upgrade_to_unordered",
            "ica_unordered_success_ack",
        ] {
            assert!(stats.require_ibc_coverage().is_err());
            stats.ibc.insert(event, 1);
        }
        assert!(stats.require_ibc_coverage().is_ok());
        stats.ibc.insert("ica_unordered_success_ack", 2);
        assert!(stats.require_ibc_coverage().is_err());
    }

    #[test]
    fn required_ica_error_needs_exact_application_error_ack() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("ica_error_ack");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("ica_error_ack", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_expired_client_update_needs_exact_rejection() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("expired_client_update");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("expired_client_update_rejected", 1);
        assert!(stats.require_ibc_coverage().is_ok());
        assert!(Stats::default().require_ibc_coverage().is_ok());
    }

    #[test]
    fn required_frozen_client_update_needs_freeze_and_exact_rejection() {
        let mut stats = Stats::default();
        stats.ibc_required.insert("frozen_client_update");
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("misbehaviour_confirmed", 1);
        assert!(stats.require_ibc_coverage().is_err());
        stats.ibc.insert("frozen_client_update_rejected", 1);
        assert!(stats.require_ibc_coverage().is_ok());
    }

    #[test]
    fn signed_hyperlane_coverage_requires_both_successful_transfers() {
        let mut stats = Stats {
            hl_verification: Some("message-id".into()),
            ..Stats::default()
        };
        assert!(stats.require_hl_coverage().is_err());
        stats.hl_verified.insert("collateral".into(), 1);
        assert!(stats.require_hl_coverage().is_err());
        stats.hl_verified.insert("synthetic".into(), 1);
        assert!(stats.require_hl_coverage().is_ok());
        assert!(!stats.hl_preupgrade_setup(9));

        stats.hl_preupgrade_required = true;
        assert!(stats.require_hl_coverage().is_err());
        assert!(stats.hl_preupgrade_setup(9));
        assert!(!stats.hl_preupgrade_setup(10));
        stats
            .hl_verified_versions
            .insert("v10/collateral".into(), 1);
        stats.hl_verified_versions.insert("v10/synthetic".into(), 1);
        assert!(stats.require_hl_coverage().is_err());
        assert!(stats.hl_preupgrade_setup(9));
        stats.hl_verified_versions.insert("v9/collateral".into(), 1);
        assert!(stats.require_hl_coverage().is_err());
        stats.hl_verified_versions.insert("v9/synthetic".into(), 1);
        assert!(stats.require_hl_coverage().is_ok());
        assert!(!stats.hl_preupgrade_setup(9));
    }

    #[test]
    fn required_hyperlane_registry_coverage_is_sequence_attributed() {
        let mut stats = Stats {
            hl_registry_required: Some(()),
            ..Stats::default()
        };
        // Generic successes cannot substitute for the reserved transactions.
        stats.per_type.insert(
            "/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm".into(),
            (1, 1, 0),
        );
        stats.per_type.insert(
            "/hyperlane.core.interchain_security.v1.MsgAnnounceValidator".into(),
            (1, 1, 0),
        );
        assert!(stats.require_hl_coverage().is_err());
        stats.hl_registry.insert("merkle-root-ism".into(), 1);
        assert!(stats.require_hl_coverage().is_err());
        stats.hl_registry.insert("validator-announcement".into(), 1);
        assert!(stats.require_hl_coverage().is_ok());
    }

    #[test]
    fn successful_submission_does_not_execute_its_payload() {
        let send = Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend::default());
        let proposal = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
            messages: vec![send.clone()],
            ..Default::default()
        });
        let authz = Any::pack(&celestia_proto::cosmos::authz::v1beta1::MsgExec {
            grantee: String::new(),
            msgs: vec![send],
        });
        let msgs = [proposal, authz];
        let declared = message_type_urls(&msgs, true);
        let executed = message_type_urls(&msgs, false);
        let nested = ">/cosmos.bank.v1beta1.MsgSend";
        assert_eq!(declared.iter().filter(|u| *u == nested).count(), 2);
        assert_eq!(executed.iter().filter(|u| *u == nested).count(), 1);
    }

    #[test]
    fn genesis_patch_adds_accounts() {
        let g = br#"{"auth":{"accounts":[{"@type":"/cosmos.auth.v1beta1.BaseAccount","account_number":"4"}]},"bank":{"balances":[],"supply":[{"denom":"utia","amount":"5"}]}}"#;
        let (out, accts) = patch_app_state(g).unwrap();
        assert_eq!(accts[0].number, 5);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v["auth"]["accounts"].as_array().unwrap().len(),
            1 + accts.len()
        );
        assert_eq!(
            v["bank"]["supply"][0]["amount"],
            (5 + BALANCE * accts.len() as u128).to_string()
        );
    }

    #[test]
    fn every_msg_type_synthesizes() {
        let mut g = Gen {
            m: Mutator::new(1),
            dict: Dict::default(),
            accts: accounts(10),
            scenario_msgs: Vec::new(),
            validators: vec!["celestiavaloper1x".into()],
            proposals: vec![1],
            unbondings: Vec::new(),
            proposers: Vec::new(),
            gas_seen: BTreeMap::new(),
            hl_objects: Vec::new(),
            hl: HlState::default(),
            hex_ids: Vec::new(),
            chain_id: "c".into(),
            app_version: 10,
            height: 5,
            types: msg_types().to_vec(),
            force_amino: false,
            wire: WireGen::default(),
            gov_msgs: Vec::new(),
            reserved_account: None,
        };
        assert!(!amino_sign_seeds(10, 1, 5).is_empty());
        assert!(msg_types().len() > 100);
        for md in msg_types() {
            let who = g.who(0);
            let b = g.synth(md, &who, 0);
            let _ = g.mutate_fields(md, &b, &who, 0);
        }
        for _ in 0..200 {
            assert!(g.tx().is_some());
        }
    }

    #[test]
    fn reserved_account_is_filtered_even_when_harvested_as_a_signer() {
        let mut g = offline_gen(10, 7, false);
        let address = g.accts[HL_ACCT].addr.clone();
        let account = g.accts[HL_ACCT].acc.clone().into_bytes();
        g.reserved_account = Some(HL_ACCT);
        g.dict
            .strings
            .insert("sender".into(), vec![account.clone()]);

        assert!((0..500).any(|_| g.address_value() == account));
        let mut decoded = 0;
        for _ in 0..2_000 {
            let Some(tx) = g.tx() else {
                continue;
            };
            let Ok(tx) = txdecode::decode_for(10, &tx.raw) else {
                continue;
            };
            decoded += 1;
            let signers = x_ante::txview::signers(&tx).unwrap_or_default();
            assert!(!signers.contains(&address));
        }
        assert!(decoded > 100);
    }
}
