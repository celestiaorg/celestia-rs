//! celestia-app v10.4.0 ante handler (`app/ante/ante.go` `NewAnteHandler`).
//!
//! A decorator-by-decorator port of the Go chain, in Go order:
//!
//! | # | Go decorator | Rust |
//! |---|---|---|
//! | 1 | `HandlePanicDecorator` (celestia) | [`ante_handle`] (Go panics become [`AnteError::panicked`]) |
//! | 2 | `SetUpContextDecorator` (sdk) | [`setup::set_up_context`] |
//! | 3 | `CircuitBreakerDecorator` (x/circuit) | [`basic::circuit_breaker`] |
//! | 4 | `RejectExtensionOptionsDecorator` (sdk) | [`basic::extension_options`] |
//! | 5 | `ValidateBasicDecorator` (sdk) | [`basic::validate_basic`] |
//! | 6 | `TxTimeoutHeightDecorator` (sdk) | [`basic::timeout_height`] |
//! | 7 | `ValidateMemoDecorator` (sdk) | [`basic::validate_memo`] |
//! | 8 | `ConsumeTxSizeGasDecorator` (celestia `app/ante/tx_size_gas.go`) | [`basic::consume_tx_size_gas`] |
//! | 9 | `DeductFeeDecorator` + `ValidateTxFeeWrapper` (sdk + celestia `fee.go`) | [`fee::deduct_fee`] |
//! | 10 | `SetPubKeyDecorator` (sdk) | [`sigverify::set_pub_key`] |
//! | 11 | `ValidateSigCountDecorator` (sdk) | [`sigverify::validate_sig_count`] |
//! | 12 | `SigGasConsumeDecorator` (sdk) | [`sigverify::sig_gas_consume`] |
//! | 13 | `SigVerificationDecorator` (sdk) | [`sigverify::sig_verification`] |
//! | 14 | `NestedMsgDecorator` (celestia) | [`celestia::nested_msg`] |
//! | 15 | `FibreSignatureGasDecorator` (x/fibre) | [`celestia::fibre_signature_gas`] |
//! | 16 | `MinGasPFBDecorator` (x/blob) | [`celestia::min_gas_pfb`] |
//! | 17 | `BlobShareDecorator` (x/blob) | [`celestia::blob_share`] |
//! | 18 | `FibreStatefulValidationDecorator` (x/fibre) | [`celestia::fibre_stateful_validation`] |
//! | 19 | `FibreSignatureVerificationDecorator` (x/fibre) | [`celestia::fibre_sig_verification`] |
//! | 20 | `ParamFilterDecorator` (celestia) | [`celestia::param_filter`] |
//! | 21 | `IncrementSequenceDecorator` (sdk) | [`sigverify::increment_sequence`] |
//! | 22 | `RedundantRelayDecorator` (ibc-go) | [`celestia::redundant_relay`] |
//!
//! State owned by modules that are not ported in this repository yet is
//! reached through [`AnteDeps`]; [`StoreDeps`] implements it by reading
//! those modules' stores exactly like their Go keepers do.
//!
//! # Go semantics preserved
//!
//! `runTxAnte` (cosmos-sdk fork `baseapp/runtx_phases.go`) reports, for a
//! failed ante, the gas meter of the context the failing decorator returned.
//! Most decorators return their input context (carrying the tx gas meter),
//! some return `sdk.Context{}` (then the pre-ante meter is reported) and
//! panics are recovered with `GasWanted = 0`. [`ante_handle`] reproduces all
//! three: after it returns, `ctx.gas` is the meter Go would report and
//! [`AnteOutcome::gas_wanted`] / [`AnteError::gas_wanted`] the gas wanted.

// JSON is assembled with `format!` pushes; Go lengths/indices are `int`
// (i64) and bit-array arithmetic mirrors Go's signed math.
#![allow(
    clippy::format_push_string,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::match_same_arms,
    clippy::items_after_statements
)]

pub mod aminojson;
pub mod basic;
pub mod celestia;
pub mod deps;
pub mod fee;
pub mod pubkey;
pub mod setup;
pub mod signing;
pub mod sigverify;
pub mod txview;

use sdk::{Context, ExecMode, SdkError};

pub use deps::{AnteDeps, StoreDeps};

/// `sdkerrors.ErrPanic` (`undefined` codespace, code 111222).
pub const PANIC_CODE: u32 = 111_222;
/// Codespace of errors not registered with `cosmossdk.io/errors`.
pub const UNDEFINED_CODESPACE: &str = "undefined";
/// ABCI code of errors not registered with `cosmossdk.io/errors`.
pub const INTERNAL_CODE: u32 = 1;

/// How a decorator failed, as `runTxAnte` observes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    /// The decorator returned `(ctx, err)`: the tx gas meter is reported.
    Err(SdkError),
    /// The decorator returned `(sdk.Context{}, err)`: the pre-ante gas meter
    /// is reported.
    ZeroCtx(SdkError),
    /// A Go panic other than out-of-gas (recovered by `runTxAnte` as
    /// `ErrPanic`).
    Panic(String),
}

impl From<SdkError> for Fail {
    fn from(e: SdkError) -> Self {
        Self::Err(e)
    }
}

/// Decorator result.
pub type R<T> = Result<T, Fail>;

/// An unregistered Go error (`errors.New` / `fmt.Errorf`): ABCI code 1 in
/// the `undefined` codespace.
#[must_use]
pub fn internal(log: impl Into<String>) -> SdkError {
    SdkError::new(UNDEFINED_CODESPACE, INTERNAL_CODE, log)
}

/// A Go runtime panic.
pub(crate) fn panic<T>(msg: impl Into<String>) -> R<T> {
    Err(Fail::Panic(msg.into()))
}

/// Indexes like Go: out-of-range is a runtime panic.
pub(crate) fn idx<T>(s: &[T], i: usize) -> R<&T> {
    s.get(i).map_or_else(
        || {
            panic(format!(
                "runtime error: index out of range [{i}] with length {}",
                s.len()
            ))
        },
        Ok,
    )
}

/// Consumes gas on the context meter with Go `ConsumeGas` semantics:
/// exceeding the limit is an out-of-gas error (recovered by
/// `SetUpContextDecorator`), overflow is a panic.
pub(crate) fn consume_gas(ctx: &mut Context<'_>, amount: u64, desc: &'static str) -> R<()> {
    match ctx.gas.consume(amount, desc) {
        Ok(()) => Ok(()),
        Err(kvstore::Error::OutOfGas { descriptor, .. }) => Err(Fail::Err(
            sdk::error::sdk::out_of_gas(format!("out of gas in location: {descriptor}")),
        )),
        Err(e) => panic(format!("gas overflow: {e:?}")),
    }
}

/// `ctx.IsCheckTx()`: true for CheckTx, ReCheckTx and simulation (whose
/// context is derived from `checkState`).
#[must_use]
pub fn is_check_tx(mode: ExecMode) -> bool {
    matches!(
        mode,
        ExecMode::Check | ExecMode::ReCheck | ExecMode::Simulate
    )
}

/// `ctx.IsReCheckTx()`.
#[must_use]
pub fn is_recheck_tx(mode: ExecMode) -> bool {
    mode == ExecMode::ReCheck
}

/// Result of a successful ante run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnteOutcome {
    /// Tx priority (`ctx.Priority()`), set by the fee checker.
    pub priority: i64,
    /// Gas wanted (`ctx.GasMeter().Limit()`; `u64::MAX` for an infinite
    /// meter).
    pub gas_wanted: u64,
}

/// A failed ante run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteError {
    /// The ABCI error.
    pub error: SdkError,
    /// Gas wanted as `runTxAnte` reports it (0 after a panic).
    pub gas_wanted: u64,
    /// The Go ante handler panicked (the error is `ErrPanic`).
    pub panicked: bool,
}

fn limit_of(meter: &kvstore::GasMeter) -> u64 {
    meter.limit().unwrap_or(u64::MAX)
}

/// Runs the celestia-app v10 ante handler (`NewAnteHandler`) on `tx`.
///
/// The caller provides a context whose `tx_bytes` are the bytes Go puts in
/// `ctx.TxBytes()` (the raw tx for `FinalizeBlock`, the inner SDK tx for a
/// `BlobTx` in `CheckTx`), `mode` set to the execution mode and a fresh
/// (infinite) gas meter. The ante runs on a store branch that is written on
/// success and discarded on failure; events it emits are kept only on
/// success. Afterwards `ctx.gas` holds the meter whose consumption Go
/// reports as `GasUsed`.
pub fn ante_handle(
    ctx: &mut Context<'_>,
    tx: &sdk::tx::Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> Result<AnteOutcome, AnteError> {
    let original_meter = ctx.gas.clone();
    let cp = ctx.branch();
    let res = run_chain(ctx, tx, simulate, deps);
    match res {
        Ok(priority) => {
            ctx.write(cp);
            Ok(AnteOutcome {
                priority,
                gas_wanted: limit_of(&ctx.gas),
            })
        }
        Err(fail) => {
            ctx.discard(cp);
            // A KV gas overflow is a panic in Go (`ErrorGasOverflow` is not
            // recovered by SetUpContextDecorator).
            let fail = match fail {
                Fail::Err(e) | Fail::ZeroCtx(e)
                    if e.codespace == "sdk" && e.code == 35 && ctx.gas.consumed() == u64::MAX =>
                {
                    Fail::Panic(e.log)
                }
                f => f,
            };
            match fail {
                Fail::Err(error) => Err(AnteError {
                    error,
                    gas_wanted: limit_of(&ctx.gas),
                    panicked: false,
                }),
                Fail::ZeroCtx(error) => {
                    ctx.gas = original_meter;
                    Err(AnteError {
                        error,
                        gas_wanted: limit_of(&ctx.gas),
                        panicked: false,
                    })
                }
                Fail::Panic(msg) => {
                    ctx.gas = original_meter;
                    Err(AnteError {
                        error: SdkError::new(
                            UNDEFINED_CODESPACE,
                            PANIC_CODE,
                            format!("recovered: {msg}: panic"),
                        ),
                        gas_wanted: 0,
                        panicked: true,
                    })
                }
            }
        }
    }
}

fn run_chain(
    ctx: &mut Context<'_>,
    tx: &sdk::tx::Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> R<i64> {
    setup::set_up_context(ctx, tx, simulate)?;
    basic::circuit_breaker(ctx, tx, deps)?;
    basic::extension_options(tx)?;
    basic::validate_basic(ctx, tx)?;
    basic::timeout_height(ctx, tx)?;
    basic::validate_memo(ctx, tx)?;
    basic::consume_tx_size_gas(ctx, tx, simulate)?;
    let priority = fee::deduct_fee(ctx, tx, simulate, deps)?;
    sigverify::set_pub_key(ctx, tx, simulate, deps)?;
    sigverify::validate_sig_count(ctx, tx)?;
    sigverify::sig_gas_consume(ctx, tx, simulate)?;
    sigverify::sig_verification(ctx, tx, simulate, deps)?;
    celestia::nested_msg(tx)?;
    celestia::fibre_signature_gas(ctx, tx)?;
    celestia::min_gas_pfb(ctx, tx)?;
    celestia::blob_share(ctx, tx, deps)?;
    celestia::fibre_stateful_validation(ctx, tx, simulate, deps)?;
    celestia::fibre_sig_verification(ctx, tx, simulate, deps)?;
    celestia::param_filter(tx, deps)?;
    sigverify::increment_sequence(ctx, tx)?;
    celestia::redundant_relay(ctx, tx, simulate, deps)?;
    Ok(priority)
}
