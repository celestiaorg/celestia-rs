//! `runTx` (celestiaorg/cosmos-sdk v0.52.12 `baseapp`): ante and message
//! phases, cache branching, gas accounting and `ExecTxResult` construction.
//!
//! celestia's fork executes a block's transactions in two phases: every
//! transaction's ante handler first, then every surviving transaction's
//! messages (`executeTxsPhased`). Ante writes are kept even if the messages
//! later fail.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{Event, EventAttribute, ExecTxResult};
use celestia_proto::cosmos::base::abci::v1beta1::TxMsgData;
use celestia_proto::signers::Signer;
use kvstore::GasMeter;
use prost::Message;
use sdk::error::sdk as errs;
use sdk::tx::Tx;
use sdk::{Context, Result, SdkError};

use crate::router;

/// Decodes raw tx bytes into a [`Tx`].
pub type DecodeFn = fn(&[u8]) -> Result<Tx>;

/// The ante handler. It sets `ctx.gas` (SetUpContextDecorator), owns its
/// store branch (written only on success) and returns gas wanted as
/// `runTxAnte` reports it, on success and on failure.
pub type AnteFn = fn(&mut Context<'_>, &Tx, bool) -> core::result::Result<u64, (SdkError, u64)>;

/// A transaction that went through the ante phase.
#[derive(Debug)]
pub struct Prepared {
    /// Raw tx bytes.
    pub tx_bytes: Vec<u8>,
    /// The decoded tx (absent if decoding failed).
    pub tx: Option<Tx>,
    /// Gas meter state after the ante handler (cumulative across phases).
    pub gas: GasMeter,
    /// Gas wanted (`gasMeter.Limit()`).
    pub gas_wanted: u64,
    /// Events emitted by the ante handler (only on success).
    pub ante_events: Vec<Event>,
}

/// Gas limit reported by Go's infinite meter (`math.MaxUint64`).
pub const INFINITE_GAS_LIMIT: u64 = u64::MAX;

/// `validateBasicTxMsgs` + handler existence check.
pub(crate) fn check_msgs(tx: &Tx) -> Result<()> {
    if tx.msgs().is_empty() {
        return Err(errs::invalid_request("must contain at least one message"));
    }
    for msg in tx.msgs() {
        router::validate_basic(msg)?;
    }
    for msg in tx.msgs() {
        if !router::has_handler(&msg.type_url) {
            // Go's `%T` of the message.
            return Err(errs::unknown_request(format!(
                "no message handler found for {}",
                txdecode::go_type_name(msg.type_url.trim_start_matches('/'))
            )));
        }
    }
    Ok(())
}

/// `runTxAnte`. `ctx` is the mode's state context (block layer for
/// Finalize); its gas meter and events are replaced per tx.
pub fn run_ante(
    ctx: &mut Context<'_>,
    tx_bytes: &[u8],
    decode: DecodeFn,
    ante: AnteFn,
    simulate: bool,
) -> (Prepared, Result<()>) {
    ctx.gas = GasMeter::infinite();
    ctx.tx_bytes = tx_bytes.to_vec();
    let mut p = Prepared {
        tx_bytes: tx_bytes.to_vec(),
        tx: None,
        gas: GasMeter::infinite(),
        gas_wanted: 0,
        ante_events: Vec::new(),
    };
    // `getContextForTx`: `ctx.WithConsensusParams(app.GetConsensusParams(ctx))`
    // reads the params on the tx's fresh infinite meter; that gas is what
    // Go reports for failures before the ante handler installs its meter.
    let _ = x_consensus::get_params(ctx);
    p.gas = ctx.gas.clone();
    let tx = match decode(tx_bytes) {
        Ok(tx) => tx,
        Err(e) => return (p, Err(e)),
    };
    if let Err(e) = check_msgs(&tx) {
        p.tx = Some(tx);
        return (p, Err(e));
    }
    let events_before = std::mem::take(&mut ctx.events);
    let res = ante(ctx, &tx, simulate);
    p.gas = ctx.gas.clone();
    let ante_events = std::mem::replace(&mut ctx.events, events_before).take();
    p.tx = Some(tx);
    match res {
        Ok(gas_wanted) => {
            p.gas_wanted = gas_wanted;
            p.ante_events = ante_events;
            (p, Ok(()))
        }
        Err((e, gas_wanted)) => {
            p.gas_wanted = gas_wanted;
            (p, Err(e))
        }
    }
}

/// Result of the message phase.
#[derive(Debug, Default)]
pub struct MsgsResult {
    /// `TxMsgData` bytes.
    pub data: Vec<u8>,
    /// Ante events followed by message events.
    pub events: Vec<Event>,
}

/// First signer of `msg` as an account bech32 string (`createEvents`).
fn sender_of(msg: &Any) -> Option<String> {
    // `GetMsgV1Signers` reads the re-marshaled message.
    let msg = &txdecode::gogo_repack(msg);
    let signers = celestia_proto::signers::get_signers(&msg.type_url, &msg.value)?.ok()?;
    let bytes = match signers.into_iter().next()? {
        Signer::Str(s) => encoding::bech32::decode_and_convert(&s).ok()?.1,
        Signer::Raw(b) => b,
    };
    x_auth::bech32(&bytes).ok()
}

/// `sdk.GetModuleNameFromTypeURL`.
fn module_from_type_url(type_url: &str) -> Option<&str> {
    type_url.split('.').nth(1)
}

/// `runTxMsgs`: runs in a new branch; writes it (Finalize) on success.
/// `ctx.gas` must hold the meter from the ante phase.
pub fn run_msgs(ctx: &mut Context<'_>, p: &mut Prepared, write: bool) -> Result<MsgsResult> {
    run_msgs_gated(ctx, p, write, |_| Ok(()))
}

/// [`run_msgs`] where `gate` runs after the messages succeeded and before
/// their branch is written; an error from it discards the branch (v0.52.8
/// `runTx` consumes block gas there, which may panic).
pub fn run_msgs_gated(
    ctx: &mut Context<'_>,
    p: &mut Prepared,
    write: bool,
    gate: impl FnOnce(&GasMeter) -> Result<()>,
) -> Result<MsgsResult> {
    let Some(tx) = p.tx.as_ref() else {
        return Err(errs::tx_decode(""));
    };
    ctx.gas = p.gas.clone();
    let outer_events = std::mem::take(&mut ctx.events);
    let base = ctx.multistore().depth();
    let cp = ctx.branch();
    let mut events: Vec<Event> = Vec::new();
    let mut responses: Vec<Any> = Vec::new();
    // runTx's `defer recover()`: a Go panic in a handler fails the tx
    // there, even where Rust handled it as a value afterwards.
    let mut result = sdk::error::recover(|| {
        for (i, msg) in tx.msgs().iter().enumerate() {
            let before = ctx.events.len();
            match router::route(ctx, msg) {
                Ok(resp) => {
                    let handler_events = ctx.events.events()[before..].to_vec();
                    let mut msg_event = Event {
                        r#type: "message".into(),
                        attributes: vec![attr("action", &msg.type_url)],
                    };
                    if let Some(s) = sender_of(msg) {
                        msg_event.attributes.push(attr("sender", &s));
                    }
                    let has_module = handler_events
                        .iter()
                        .any(|e| e.attributes.iter().any(|a| a.key == "module"));
                    if !has_module && let Some(m) = module_from_type_url(&msg.type_url) {
                        msg_event.attributes.push(attr("module", m));
                    }
                    for mut e in std::iter::once(msg_event).chain(handler_events) {
                        e.attributes.push(attr("msg_index", &i.to_string()));
                        events.push(e);
                    }
                    responses.push(resp);
                }
                Err(e) => {
                    return Err(e.wrap(format!("failed to execute message; message index: {i}")));
                }
            }
        }
        Ok(())
    });
    // Go's `ConsumeGas` panics at the first consumption past the limit and
    // `runTx` recovers it: the tx fails right there, with this log and gas
    // used, even where Rust handled the error as a value afterwards (e.g.
    // an IBC app turning it into an error acknowledgement).
    if let Some((descriptor, used)) = ctx.gas.first_out_of_gas()
        && p.gas.first_out_of_gas().is_none()
    {
        ctx.gas.rewind_to_first_out_of_gas();
        let wanted = ctx.gas.limit().unwrap_or(0);
        result = Err(sdk::error::sdk::out_of_gas(format!(
            "out of gas in location: {descriptor}; gasWanted: {wanted}, gasUsed: {used}"
        )));
    }
    p.gas = ctx.gas.clone();
    ctx.events = sdk::event::EventManager::default();
    ctx.events.emit_all(outer_events.events().iter().cloned());
    if result.is_ok()
        && let Err(e) = gate(&p.gas)
    {
        result = Err(e);
    }
    match result {
        Ok(()) => {
            if write {
                ctx.write(cp);
            } else {
                ctx.multistore().discard_branch();
            }
            let data = TxMsgData {
                msg_responses: responses,
                ..Default::default()
            }
            .encode_to_vec();
            let mut all = p.ante_events.clone();
            all.extend(events);
            Ok(MsgsResult { data, events: all })
        }
        Err(e) => {
            // Everything the messages branched, also what a handler that
            // failed mid-way (a Go panic unwinding) left open.
            while ctx.multistore().depth() > base {
                ctx.multistore().discard_branch();
            }
            Err(e)
        }
    }
}

fn attr(k: &str, v: &str) -> EventAttribute {
    EventAttribute {
        key: k.into(),
        value: v.into(),
        index: true,
    }
}

#[allow(clippy::cast_possible_wrap)]
fn as_i64(v: u64) -> i64 {
    v as i64
}

/// `ResponseExecTxResultWithEvents` / successful result.
#[must_use]
pub fn exec_result(p: &Prepared, res: Result<MsgsResult>) -> ExecTxResult {
    let gas_used = as_i64(p.gas.consumed());
    let gas_wanted = as_i64(p.gas_wanted);
    match res {
        Ok(r) => ExecTxResult {
            code: 0,
            data: r.data,
            log: String::new(),
            info: String::new(),
            gas_wanted,
            gas_used,
            events: r.events,
            codespace: String::new(),
            ..Default::default()
        },
        Err(e) => error_result(&e, gas_wanted, gas_used, p.ante_events.clone()),
    }
}

/// Error `ExecTxResult`.
#[must_use]
pub fn error_result(
    e: &SdkError,
    gas_wanted: i64,
    gas_used: i64,
    events: Vec<Event>,
) -> ExecTxResult {
    ExecTxResult {
        code: e.code,
        log: e.log.clone(),
        gas_wanted,
        gas_used,
        events,
        codespace: e.codespace.to_owned(),
        ..Default::default()
    }
}
