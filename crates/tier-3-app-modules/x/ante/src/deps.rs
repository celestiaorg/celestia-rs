//! Keepers of modules that are not ported in this repository (or are ported
//! concurrently). Every method stands in for one Go call made by a
//! decorator; KV reads must be performed on the given context so that gas is
//! charged exactly like the Go keeper.

use std::cell::RefCell;
use std::collections::HashSet;

use celestia_proto::Any;
use celestia_proto::celestia::fibre::v1::{MsgPayForFibre, PaymentPromise};
use math::LegacyDec;
use prost::{Message, Name};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::{Context, SdkError};

/// The IBC packet messages `RedundantRelayDecorator` inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbcPacketMsg {
    /// `MsgRecvPacket` (`recvPacketCheckTx` / `recvPacketReCheckTx`).
    RecvPacket,
    /// `MsgAcknowledgement` (`keeper.Acknowledgement`).
    Acknowledgement,
    /// `MsgTimeout` (`keeper.Timeout`).
    Timeout,
    /// `MsgTimeoutOnClose` (`keeper.TimeoutOnClose`).
    TimeoutOnClose,
}

/// State the ante handler reads from modules outside `x/auth` and `x/bank`.
pub trait AnteDeps {
    /// circuit `Keeper.IsAllowed(ctx, typeURL)` (cosmossdk.io/x/circuit
    /// v0.1.1): `!DisableList.Has(ctx, typeURL)`.
    fn circuit_is_allowed(&self, ctx: &mut Context<'_>, type_url: &str) -> sdk::Result<bool>;

    /// `minfeeKeeper.GetParams(ctx).NetworkMinGasPrice` (celestia
    /// `x/minfee/keeper/params.go`), read by `ValidateTxFee`.
    fn network_min_gas_price(&self, ctx: &mut Context<'_>) -> sdk::Result<LegacyDec>;

    /// Node-local `ctx.MinGasPrices()` as `ValidateTxFee` uses it: `None`
    /// when no minimum gas prices are configured (Go then uses
    /// `appconsts.DefaultMinGasPrice`), otherwise `AmountOf("utia")`.
    fn node_min_gas_price(&self) -> Option<LegacyDec>;

    /// `blobKeeper.GetParams(ctx).GovMaxSquareSize` (celestia
    /// `x/blob/keeper/params.go`), read by `BlobShareDecorator`.
    fn blob_gov_max_square_size(&self, ctx: &mut Context<'_>) -> sdk::Result<u64>;

    /// feegrant `Keeper.UseGrantedFees(ctx, granter, grantee, fee, msgs)`
    /// (cosmossdk.io/x/feegrant v0.1.1).
    fn use_granted_fees(
        &self,
        ctx: &mut Context<'_>,
        granter: &[u8],
        grantee: &[u8],
        fee: &Coins,
        msgs: &[Any],
    ) -> sdk::Result<()>;

    /// fibre `Keeper.ValidatePaymentPromiseStateful(ctx, promise)` (celestia
    /// `x/fibre/keeper`). Called with an infinite gas meter.
    fn fibre_validate_payment_promise(
        &self,
        ctx: &mut Context<'_>,
        promise: &PaymentPromise,
    ) -> sdk::Result<()>;

    /// `PffSigCache.IsCached(key)` (celestia `app/pff_sig_cache.go`,
    /// node-local).
    fn pff_sig_cached(&self, key: &[u8; 32]) -> bool;

    /// `PffSigCache.Cache(key)`.
    fn pff_sig_cache(&self, key: [u8; 32]);

    /// fibre `Keeper.ValidatePayForFibreSignatures(ctx, msg)`. Called with an
    /// infinite gas meter.
    fn fibre_validate_signatures(
        &self,
        ctx: &mut Context<'_>,
        msg: &MsgPayForFibre,
    ) -> sdk::Result<()>;

    /// ibc-go `RedundantRelayDecorator` packet handling: `recvPacketCheckTx`
    /// / `recvPacketReCheckTx`, `keeper.Acknowledgement`, `keeper.Timeout`,
    /// `keeper.TimeoutOnClose`. Returns whether the response result is
    /// `NOOP`.
    fn ibc_packet_is_noop(
        &self,
        ctx: &mut Context<'_>,
        kind: IbcPacketMsg,
        msg: &Any,
        recheck: bool,
    ) -> sdk::Result<bool>;

    /// ibc-go `RedundantRelayDecorator.updateClientCheckTx`.
    fn ibc_update_client_check(
        &self,
        ctx: &mut Context<'_>,
        msg: &Any,
        recheck: bool,
    ) -> sdk::Result<()>;

    /// `paramFilters[MsgTypeURL(msg)]` (celestia `app/param_filters.go`
    /// `GovParamFilters`); `None` when no filter is registered.
    fn param_filter(&self, msg: &Any) -> Option<sdk::Result<()>> {
        default_param_filter(msg)
    }

    /// `ctx.IsSigverifyTx()` (`BaseApp.sigverifyTx`, true unless disabled).
    fn sigverify_tx(&self) -> bool {
        true
    }
}

const UNBONDING_TIME_SECS: i64 = 337 * 3600;

fn duration_nanos(d: Option<&pbjson_types::Duration>) -> i128 {
    d.map_or(0, |d| {
        i128::from(d.seconds) * 1_000_000_000 + i128::from(d.nanos)
    })
}

fn decode<M: Message + Default>(any: &Any) -> sdk::Result<M> {
    M::decode(any.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

#[must_use]
/// celestia-app v10 `GovParamFilters`: bank, staking and consensus
/// `MsgUpdateParams` filters.
pub fn default_param_filter(msg: &Any) -> Option<sdk::Result<()>> {
    use celestia_proto::cosmos::{bank, consensus, staking};
    let url = msg.type_url.as_str();
    if url == bank::v1beta1::MsgUpdateParams::type_url() {
        return Some((|| {
            let m: bank::v1beta1::MsgUpdateParams = decode(msg)?;
            let p = m.params.unwrap_or_default();
            #[allow(deprecated)]
            let modified = !p.send_enabled.is_empty() || !p.default_send_enabled;
            if modified {
                return Err(errs::unauthorized(
                    "modification of SendEnabled is not allowed",
                ));
            }
            Ok(())
        })());
    }
    if url == staking::v1beta1::MsgUpdateParams::type_url() {
        return Some((|| {
            let m: staking::v1beta1::MsgUpdateParams = decode(msg)?;
            let p = m.params.unwrap_or_default();
            if p.bond_denom != "utia" {
                return Err(errs::unauthorized(format!(
                    "invalid bond denom: expected utia, got {}",
                    p.bond_denom
                )));
            }
            if duration_nanos(p.unbonding_time.as_ref())
                != i128::from(UNBONDING_TIME_SECS) * 1_000_000_000
            {
                return Err(errs::unauthorized("invalid unbonding time"));
            }
            Ok(())
        })());
    }
    if url == consensus::v1::MsgUpdateParams::type_url() {
        return Some((|| {
            let m: consensus::v1::MsgUpdateParams = decode(msg)?;
            let (Some(_block), Some(evidence), Some(validator)) =
                (&m.block, &m.evidence, &m.validator)
            else {
                return Err(crate::internal("all parameters must be present"));
            };
            if validator.pub_key_types != ["ed25519"] {
                return Err(errs::unauthorized("invalid validator parameters"));
            }
            let ev_ok = evidence.max_age_num_blocks == 404_400
                && duration_nanos(evidence.max_age_duration.as_ref())
                    == i128::from(UNBONDING_TIME_SECS) * 1_000_000_000
                && evidence.max_bytes == 1_048_576;
            if !ev_ok {
                return Err(errs::unauthorized("invalid evidence parameters"));
            }
            Ok(())
        })());
    }
    None
}

/// Store key of `x/circuit`.
pub const CIRCUIT_STORE_KEY: &str = "circuit";
/// Store key of `x/minfee`.
pub const MINFEE_STORE_KEY: &str = "minfee";
/// Store key of `x/blob`.
pub const BLOB_STORE_KEY: &str = "blob";
/// Store key of `x/feegrant`.
pub const FEEGRANT_STORE_KEY: &str = "feegrant";

fn not_supported(what: &str) -> SdkError {
    errs::not_supported(format!("{what} is not ported yet"))
}

/// [`AnteDeps`] backed by the Go modules' store layouts.
///
/// Implements the circuit, minfee and blob reads exactly like their Go
/// keepers, the feegrant "no grant" path, and holds the node-local PFF
/// signature cache. Fibre and IBC checks are not ported yet and fail with
/// `ErrNotSupported` when reached (only for `MsgPayForFibre` txs and IBC
/// packet txs in CheckTx).
#[derive(Debug, Default)]
pub struct StoreDeps {
    /// Node-local minimum gas price (utia), see
    /// [`AnteDeps::node_min_gas_price`].
    pub node_min_gas_price: Option<LegacyDec>,
    pff_cache: RefCell<HashSet<[u8; 32]>>,
}

impl StoreDeps {
    /// Creates deps with the given node minimum gas price.
    #[must_use]
    pub fn new(node_min_gas_price: Option<LegacyDec>) -> Self {
        Self {
            node_min_gas_price,
            pff_cache: RefCell::default(),
        }
    }
}

impl AnteDeps for StoreDeps {
    fn circuit_is_allowed(&self, ctx: &mut Context<'_>, type_url: &str) -> sdk::Result<bool> {
        // collections.KeySet[string] with prefix 2: terminal string key.
        let mut key = vec![2u8];
        key.extend_from_slice(type_url.as_bytes());
        Ok(!ctx.has(CIRCUIT_STORE_KEY, &key)?)
    }

    fn network_min_gas_price(&self, ctx: &mut Context<'_>) -> sdk::Result<LegacyDec> {
        let Some(bz) = ctx.get(MINFEE_STORE_KEY, b"params")? else {
            return Err(not_supported("minfee legacy param subspace fallback"));
        };
        let p = celestia_proto::celestia::minfee::v1::Params::decode(bz.as_slice())
            .map_err(|e| errs::json_unmarshal(e.to_string()))?;
        LegacyDec::unmarshal(p.network_min_gas_price.as_bytes())
            .map_err(|e| errs::invalid_type(e.to_string()))
    }

    fn node_min_gas_price(&self) -> Option<LegacyDec> {
        self.node_min_gas_price.clone()
    }

    fn blob_gov_max_square_size(&self, ctx: &mut Context<'_>) -> sdk::Result<u64> {
        let Some(bz) = ctx.get(BLOB_STORE_KEY, b"params")? else {
            return Err(not_supported("blob legacy param subspace fallback"));
        };
        let p = celestia_proto::celestia::blob::v1::Params::decode(bz.as_slice())
            .map_err(|e| errs::json_unmarshal(e.to_string()))?;
        Ok(p.gov_max_square_size)
    }

    fn use_granted_fees(
        &self,
        ctx: &mut Context<'_>,
        granter: &[u8],
        grantee: &[u8],
        _fee: &Coins,
        _msgs: &[Any],
    ) -> sdk::Result<()> {
        // FeeAllowanceKey: 0x00 | len(grantee) grantee | len(granter) granter
        let key = sdk::codec::key(&[
            &[0x00],
            &sdk::codec::length_prefixed(grantee),
            &sdk::codec::length_prefixed(granter),
        ]);
        match ctx.get(FEEGRANT_STORE_KEY, &key)? {
            None => Err(errs::not_found("fee-grant not found")),
            Some(bz) if bz.is_empty() => Err(errs::not_found("fee-grant not found")),
            Some(_) => Err(not_supported("feegrant allowances")),
        }
    }

    fn fibre_validate_payment_promise(
        &self,
        _ctx: &mut Context<'_>,
        _promise: &PaymentPromise,
    ) -> sdk::Result<()> {
        Err(not_supported("fibre ValidatePaymentPromiseStateful"))
    }

    fn pff_sig_cached(&self, key: &[u8; 32]) -> bool {
        self.pff_cache.borrow().contains(key)
    }

    fn pff_sig_cache(&self, key: [u8; 32]) {
        self.pff_cache.borrow_mut().insert(key);
    }

    fn fibre_validate_signatures(
        &self,
        _ctx: &mut Context<'_>,
        _msg: &MsgPayForFibre,
    ) -> sdk::Result<()> {
        Err(not_supported("fibre ValidatePayForFibreSignatures"))
    }

    fn ibc_packet_is_noop(
        &self,
        _ctx: &mut Context<'_>,
        _kind: IbcPacketMsg,
        _msg: &Any,
        _recheck: bool,
    ) -> sdk::Result<bool> {
        Err(not_supported("IBC redundant relay check"))
    }

    fn ibc_update_client_check(
        &self,
        _ctx: &mut Context<'_>,
        _msg: &Any,
        _recheck: bool,
    ) -> sdk::Result<()> {
        Err(not_supported("IBC update client check"))
    }
}
