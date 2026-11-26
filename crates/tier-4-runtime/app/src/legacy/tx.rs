//! cosmos-sdk v0.46 `BaseApp.runTx` in `runTxModeDeliver` for celestia-app
//! v1 to v3 (cosmos-sdk `v1.31.2-sdk-v0.46.16` `baseapp/baseapp.go`):
//! decode, `validateBasicTxMsgs`, the ante handler on a store branch, then
//! the messages on a second branch, with v0.46 gas accounting, block gas
//! and result shaping (`message` events, `ABCIMessageLogs`).
//!
//! Go reports `GasUsed` from the meter of the context `runTx` holds when it
//! returns: the transaction's meter once the ante handler has installed
//! it, and otherwise the block context's meter (`deliverState.ctx`, an
//! infinite meter that also carries the BeginBlocker's and every earlier
//! `getContextForTx` consensus-params read). That second meter is
//! `ctx.gas` of the driver's block context, kept across transactions.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::Event;
use kvstore::GasMeter;
use sdk::error::sdk as errs;
use sdk::tx::Tx;
use sdk::{Context, SdkError};

use super::{TxOutcome, TxSuccess, consensus_params, msgs};

/// `app.MsgGateKeeper`'s accepted message type URLs of celestia-app v1
/// (`configurator.GetAcceptedMessages()[1]`, read from v3.13.0).
const ACCEPTED_V1: &[&str] = &[
    "/celestia.blob.v1.MsgPayForBlobs",
    "/celestia.qgb.v1.MsgRegisterEVMAddress",
    "/cosmos.authz.v1beta1.MsgExec",
    "/cosmos.authz.v1beta1.MsgGrant",
    "/cosmos.authz.v1beta1.MsgRevoke",
    "/cosmos.bank.v1beta1.MsgMultiSend",
    "/cosmos.bank.v1beta1.MsgSend",
    "/cosmos.crisis.v1beta1.MsgVerifyInvariant",
    "/cosmos.distribution.v1beta1.MsgFundCommunityPool",
    "/cosmos.distribution.v1beta1.MsgSetWithdrawAddress",
    "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward",
    "/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission",
    "/cosmos.evidence.v1beta1.MsgSubmitEvidence",
    "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
    "/cosmos.feegrant.v1beta1.MsgRevokeAllowance",
    "/cosmos.gov.v1.MsgDeposit",
    "/cosmos.gov.v1.MsgExecLegacyContent",
    "/cosmos.gov.v1.MsgSubmitProposal",
    "/cosmos.gov.v1.MsgVote",
    "/cosmos.gov.v1.MsgVoteWeighted",
    "/cosmos.gov.v1beta1.MsgDeposit",
    "/cosmos.gov.v1beta1.MsgSubmitProposal",
    "/cosmos.gov.v1beta1.MsgVote",
    "/cosmos.gov.v1beta1.MsgVoteWeighted",
    "/cosmos.slashing.v1beta1.MsgUnjail",
    "/cosmos.staking.v1beta1.MsgBeginRedelegate",
    "/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation",
    "/cosmos.staking.v1beta1.MsgCreateValidator",
    "/cosmos.staking.v1beta1.MsgDelegate",
    "/cosmos.staking.v1beta1.MsgEditValidator",
    "/cosmos.staking.v1beta1.MsgUndelegate",
    "/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount",
    "/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount",
    "/cosmos.vesting.v1beta1.MsgCreateVestingAccount",
    "/ibc.applications.transfer.v1.MsgTransfer",
    "/ibc.core.channel.v1.MsgAcknowledgement",
    "/ibc.core.channel.v1.MsgChannelCloseConfirm",
    "/ibc.core.channel.v1.MsgChannelCloseInit",
    "/ibc.core.channel.v1.MsgChannelOpenAck",
    "/ibc.core.channel.v1.MsgChannelOpenConfirm",
    "/ibc.core.channel.v1.MsgChannelOpenInit",
    "/ibc.core.channel.v1.MsgChannelOpenTry",
    "/ibc.core.channel.v1.MsgRecvPacket",
    "/ibc.core.channel.v1.MsgTimeout",
    "/ibc.core.channel.v1.MsgTimeoutOnClose",
    "/ibc.core.client.v1.MsgCreateClient",
    "/ibc.core.client.v1.MsgSubmitMisbehaviour",
    "/ibc.core.client.v1.MsgUpdateClient",
    "/ibc.core.client.v1.MsgUpgradeClient",
    "/ibc.core.connection.v1.MsgConnectionOpenAck",
    "/ibc.core.connection.v1.MsgConnectionOpenConfirm",
    "/ibc.core.connection.v1.MsgConnectionOpenInit",
    "/ibc.core.connection.v1.MsgConnectionOpenTry",
];

/// The messages app versions 2 and 3 add to [`ACCEPTED_V1`] (x/signal);
/// they drop `MsgRegisterEVMAddress` (blobstream).
const ADDED_V2: &[&str] = &[
    "/celestia.signal.v1.MsgSignalVersion",
    "/celestia.signal.v1.MsgTryUpgrade",
];

/// Whether `MsgVersioningGateKeeper` accepts `type_url` at app version
/// `v`; `None` for an app version it does not know.
#[must_use]
pub fn gatekeeper_accepts(v: u64, type_url: &str) -> Option<bool> {
    match v {
        1 => Some(ACCEPTED_V1.contains(&type_url)),
        2 | 3 => Some(
            (ACCEPTED_V1.contains(&type_url)
                && type_url != "/celestia.qgb.v1.MsgRegisterEVMAddress")
                || ADDED_V2.contains(&type_url),
        ),
        _ => None,
    }
}

fn header_app_version(ctx: &Context<'_>) -> u64 {
    ctx.consensus_params.version.as_ref().map_or(0, |v| v.app)
}

/// The type URLs of `msgs` in the order `hasInvalidMsg` checks them: a
/// `MsgExec`'s nested messages before the `MsgExec` itself.
fn gatekeeper_order(msgs: &[Any], depth: usize, out: &mut Vec<String>) -> Result<(), SdkError> {
    use prost::Message as _;
    for msg in msgs {
        if msg.type_url == "/cosmos.authz.v1beta1.MsgExec" {
            // Go's `hasInvalidMsg` recurses without a limit and would go
            // on to the ante handler; this cap bounds the recursion here,
            // so exceeding it is not ported (the block aborts).
            if depth > 64 {
                return Err(super::not_ported(errs::invalid_request(
                    "max depth exceeded",
                )));
            }
            // Read as gogoproto decodes it (`decode_v1` accepted it).
            let view = msgs::gogo_view(msg)?;
            let exec =
                celestia_proto::cosmos::authz::v1beta1::MsgExec::decode(view.value.as_slice())
                    .map_err(|e| errs::tx_decode(e.to_string()))?;
            gatekeeper_order(&exec.msgs, depth + 1, out)?;
        }
        out.push(msg.type_url.clone());
    }
    Ok(())
}

/// `MsgVersioningGateKeeper.AnteHandle` (it runs before
/// `SetUpContextDecorator`, so a failure leaves the block context's gas
/// meter in place).
fn gatekeeper(ctx: &Context<'_>, tx: &Tx) -> Result<(), SdkError> {
    let v = header_app_version(ctx);
    if gatekeeper_accepts(v, "").is_none() {
        return Err(errs::not_supported(format!(
            "app version {v} is not supported"
        )));
    }
    let mut urls = Vec::new();
    gatekeeper_order(tx.msgs(), 0, &mut urls)?;
    for url in urls {
        if gatekeeper_accepts(v, &url) != Some(true) {
            return Err(errs::not_supported(format!(
                "message type {url} is not supported in version {v}"
            )));
        }
    }
    Ok(())
}

/// The ante keepers of the v1 ante handler: a keeper path that is not
/// ported records it ([`super::record_not_ported`]).
pub(crate) fn ante_deps() -> x_ante::StoreDeps {
    x_ante::StoreDeps {
        on_not_ported: Some(super::record_not_ported),
        ..crate::txpipe::ante_deps(None)
    }
}

/// The block gas meter (`ctx.BlockGasMeter()`): finite for a positive
/// consensus `max_gas`, infinite otherwise.
#[must_use]
pub fn block_gas_meter(ctx: &Context<'_>) -> GasMeter {
    match ctx.consensus_params.block.as_ref().map(|b| b.max_gas) {
        Some(max) if max > 0 => GasMeter::new(max.unsigned_abs()),
        _ => GasMeter::infinite(),
    }
}

/// Which meter `runTx`'s context holds when it returns (`GasUsed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reported {
    /// The block context's meter (before or without the ante handler's).
    Block,
    /// The transaction's meter, installed by `SetUpContextDecorator`.
    Tx,
}

struct Ran {
    gas_wanted: u64,
    reported: Reported,
    events: Vec<Event>,
    result: Result<TxSuccess, SdkError>,
    /// `consumeBlockGas` already ran (it runs at most once).
    block_gas_consumed: bool,
}

impl Ran {
    fn early(gas_wanted: u64, e: SdkError) -> Self {
        Self {
            gas_wanted,
            reported: Reported::Block,
            events: Vec::new(),
            result: Err(e),
            block_gas_consumed: false,
        }
    }
}

/// Executes one transaction of a v0.46 block (`DeliverTx`).
///
/// `ctx` is the block context: its gas meter is the block context's
/// (kept, with the consensus-params read every transaction adds), its
/// store the block's working state. `block_gas` is the block gas meter.
pub fn deliver_tx(ctx: &mut Context<'_>, block_gas: &mut GasMeter, tx_bytes: &[u8]) -> TxOutcome {
    // `getContextForTx` reads the consensus params on the block context's
    // meter, before the block gas check.
    let _ = consensus_params::load(ctx, header_app_version(ctx));
    if block_gas.is_out_of_gas() {
        // Returned before `runTx` registers its defers: no gas info.
        return fail(
            0,
            0,
            Vec::new(),
            errs::out_of_gas("no block gas left to run tx"),
        );
    }
    let block_meter = ctx.gas.clone();
    ctx.tx_bytes = tx_bytes.to_vec();
    let mut ran = run(ctx, block_gas, tx_bytes);
    let reported = match ran.reported {
        Reported::Block => block_meter.clone(),
        Reported::Tx => ctx.gas.clone(),
    };
    ctx.gas = block_meter;
    ctx.tx_bytes.clear();
    // The deferred `consumeBlockGas`.
    if !ran.block_gas_consumed
        && let Err(e) = consume_block_gas(block_gas, ran.gas_wanted, &reported)
    {
        ran.result = Err(e);
    }
    TxOutcome {
        gas_wanted: ran.gas_wanted,
        gas_used: reported.consumed(),
        events: ran.events,
        result: ran.result,
    }
}

fn fail(gas_wanted: u64, gas_used: u64, events: Vec<Event>, e: SdkError) -> TxOutcome {
    TxOutcome {
        gas_wanted,
        gas_used,
        events,
        result: Err(e),
    }
}

/// `consumeBlockGas`: the reported meter's `GasConsumedToLimit` on the
/// block meter. Past the block limit Go panics and `runTx`'s
/// out-of-gas recovery reports it.
fn consume_block_gas(
    block: &mut GasMeter,
    gas_wanted: u64,
    meter: &GasMeter,
) -> Result<(), SdkError> {
    match block.consume(meter.consumed_to_limit(), "block gas meter") {
        Ok(()) => Ok(()),
        Err(kvstore::Error::OutOfGas { descriptor, .. }) => Err(errs::out_of_gas(format!(
            "out of gas in location: {descriptor}; gasWanted: {gas_wanted}, gasUsed: {}",
            meter.consumed()
        ))),
        Err(_) => Err(SdkError::new(
            x_ante::UNDEFINED_CODESPACE,
            x_ante::PANIC_CODE,
            "recovered: gas overflow: block gas meter",
        )),
    }
}

fn run(ctx: &mut Context<'_>, block_gas: &mut GasMeter, tx_bytes: &[u8]) -> Ran {
    // celestia-app v3.13.0's decoder: an undecodable transaction fails
    // with no gas wanted, before the ante handler (no fee, no sequence).
    let tx = match txdecode::decode_v1(tx_bytes) {
        Ok(tx) => tx,
        // A nesting limit Go v0.46's decoder does not have: Go would
        // decode on, so no result here is Go's (the block aborts).
        Err(e) if txdecode::is_v1_unsupported_limit(&e) => {
            return Ran::early(0, super::not_ported(e));
        }
        Err(e) => return Ran::early(0, e),
    };
    // `validateBasicTxMsgs` (a Go panic in a `ValidateBasic` is recovered
    // by `runTx`).
    if let Err(e) = sdk::error::recover(|| msgs::validate_basic_tx_msgs(tx.msgs())) {
        return Ran::early(0, e);
    }
    // `MsgVersioningGateKeeper` runs before `SetUpContextDecorator`:
    // `runTx` takes the context it returns, with the block meter, whose
    // limit (`math.MaxUint64`) becomes the gas wanted.
    if let Err(e) = gatekeeper(ctx, &tx) {
        return Ran::early(u64::MAX, e);
    }
    let events_before = std::mem::take(&mut ctx.events);
    let deps = ante_deps();
    let ante = x_ante::ante_handle_v046(ctx, &tx, false, &deps);
    let ante_events = std::mem::replace(&mut ctx.events, events_before).take();
    let gas_wanted = match ante {
        Ok(o) => o.gas_wanted,
        Err(e) => {
            // After a panic, or a decorator returning `sdk.Context{}`,
            // `runTx` keeps its own context (the block meter).
            let reported = if e.panicked || e.zero_ctx {
                Reported::Block
            } else {
                Reported::Tx
            };
            return Ran {
                reported,
                ..Ran::early(e.gas_wanted, e.error)
            };
        }
    };
    let mut ran = Ran {
        gas_wanted,
        reported: Reported::Tx,
        events: ante_events,
        result: Err(errs::logic("")),
        block_gas_consumed: false,
    };
    match run_msgs(ctx, &tx, gas_wanted, block_gas) {
        Ok((msg_responses, log, events)) => {
            ran.block_gas_consumed = true;
            ran.events.extend(events);
            ran.result = Ok(TxSuccess { msg_responses, log });
        }
        Err((e, block_gas_consumed)) => {
            ran.block_gas_consumed = block_gas_consumed;
            ran.result = Err(e);
        }
    }
    ran
}

type MsgsOk = (Vec<Any>, String, Vec<Event>);

/// `runMsgs` on a store branch. On success `consumeBlockGas` runs and the
/// branch is written unless the block gas ran out (the error then says
/// whether block gas was consumed).
fn run_msgs(
    ctx: &mut Context<'_>,
    tx: &Tx,
    gas_wanted: u64,
    block_gas: &mut GasMeter,
) -> Result<MsgsOk, (SdkError, bool)> {
    let outer_events = std::mem::take(&mut ctx.events);
    let base = ctx.multistore().depth();
    let cp = ctx.branch();
    let mut responses = Vec::with_capacity(tx.msgs().len());
    let mut events: Vec<Event> = Vec::new();
    let mut logs: Vec<(usize, Vec<Event>)> = Vec::with_capacity(tx.msgs().len());
    let mut result = sdk::error::recover(|| {
        for (i, msg) in tx.msgs().iter().enumerate() {
            // Each handler runs with a new event manager.
            let resp = msgs::route(ctx, msg)
                .map_err(|e| e.wrap(format!("failed to execute message; message index: {i}")))?;
            responses.push(resp);
            let mut msg_events = vec![sdk::event::event(
                "message",
                [("action", msg.type_url.clone())],
            )];
            msg_events.extend(ctx.events.take());
            events.extend(msg_events.iter().cloned());
            logs.push((i, msg_events));
        }
        Ok(())
    });
    // `ConsumeGas` panics at the first consumption past the limit and
    // `runTx` recovers it there, whatever Rust did with the error after.
    if let Some((descriptor, used)) = ctx.gas.first_out_of_gas() {
        ctx.gas.rewind_to_first_out_of_gas();
        result = Err(errs::out_of_gas(format!(
            "out of gas in location: {descriptor}; gasWanted: {gas_wanted}, gasUsed: {used}"
        )));
    }
    ctx.events = outer_events;
    let mut block_gas_consumed = false;
    if result.is_ok() {
        block_gas_consumed = true;
        let meter = ctx.gas.clone();
        result = consume_block_gas(block_gas, gas_wanted, &meter);
    }
    match result {
        Ok(()) => {
            ctx.write(cp);
            Ok((responses, message_logs(&logs), events))
        }
        Err(e) => {
            // Everything the messages branched, also what a failing
            // handler left open.
            while ctx.multistore().depth() > base {
                ctx.multistore().discard_branch();
            }
            Err((e, block_gas_consumed))
        }
    }
}

/// `strings.TrimSpace(ABCIMessageLogs.String())`: go-amino JSON of each
/// message's events flattened by type (`StringifyEvents`: attributes of
/// equal types concatenated, types sorted).
#[must_use]
pub fn message_logs(logs: &[(usize, Vec<Event>)]) -> String {
    use std::fmt::Write as _;
    use x_params::v046::json_string;
    let mut s = String::from("[");
    for (n, (i, events)) in logs.iter().enumerate() {
        if n > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"msg_index\":{i},\"events\":");
        let mut flat: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
            std::collections::BTreeMap::new();
        for e in events {
            flat.entry(e.r#type.as_str()).or_default().extend(
                e.attributes
                    .iter()
                    .map(|a| (a.key.as_str(), a.value.as_str())),
            );
        }
        if flat.is_empty() {
            s.push_str("null");
        } else {
            s.push('[');
            for (m, (ty, attrs)) in flat.iter().enumerate() {
                if m > 0 {
                    s.push(',');
                }
                s.push('{');
                if !ty.is_empty() {
                    let _ = write!(s, "\"type\":{},", json_string(ty));
                }
                s.push_str("\"attributes\":");
                if attrs.is_empty() {
                    s.push_str("null");
                } else {
                    s.push('[');
                    for (j, (k, v)) in attrs.iter().enumerate() {
                        if j > 0 {
                            s.push(',');
                        }
                        let mut fields = Vec::new();
                        if !k.is_empty() {
                            fields.push(format!("\"key\":{}", json_string(k)));
                        }
                        if !v.is_empty() {
                            fields.push(format!("\"value\":{}", json_string(v)));
                        }
                        s.push('{');
                        s.push_str(&fields.join(","));
                        s.push('}');
                    }
                    s.push(']');
                }
                s.push('}');
            }
            s.push(']');
        }
        s.push('}');
    }
    s.push(']');
    s
}
