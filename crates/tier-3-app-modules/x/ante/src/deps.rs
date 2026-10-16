//! Keepers of modules that are not ported in this repository (or are ported
//! concurrently). Every method stands in for one Go call made by a
//! decorator; KV reads must be performed on the given context so that gas is
//! charged exactly like the Go keeper.

use std::sync::{Mutex, OnceLock};

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
            let want = i128::from(UNBONDING_TIME_SECS) * 1_000_000_000;
            let got = duration_nanos(p.unbonding_time.as_ref());
            if got != want {
                return Err(errs::unauthorized(format!(
                    "invalid unbonding time: expected {}, got {}",
                    sdk::time::go_duration_string(want),
                    sdk::time::go_duration_string(got)
                )));
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

/// The IBC checks of `RedundantRelayDecorator`, supplied by the app (they
/// run IBC message handlers, which need the app's ICS-26 router).
#[derive(Debug, Clone, Copy)]
pub struct IbcHooks {
    /// See [`AnteDeps::ibc_packet_is_noop`].
    pub packet_is_noop: fn(&mut Context<'_>, IbcPacketMsg, &Any, bool) -> sdk::Result<bool>,
    /// See [`AnteDeps::ibc_update_client_check`].
    pub update_client_check: fn(&mut Context<'_>, &Any, bool) -> sdk::Result<()>,
}

/// [`AnteDeps`] backed by the Go modules' store layouts.
///
/// Implements the circuit, minfee and blob reads exactly like their Go
/// keepers, feegrant allowances (`x_feegrant`), the fibre promise and
/// validator-signature checks (`x_fibre`), and uses the process-wide PFF
/// signature cache ([`pff_sig_cache`]). IBC packet checks are not ported
/// yet and fail with `ErrNotSupported` when reached (only for IBC packet
/// txs in CheckTx).
#[derive(Debug, Default)]
pub struct StoreDeps {
    /// Node-local minimum gas price (utia), see
    /// [`AnteDeps::node_min_gas_price`].
    pub node_min_gas_price: Option<LegacyDec>,
    /// The app's IBC checks (CheckTx); without them IBC packet txs fail
    /// with `ErrNotSupported` in CheckTx.
    pub ibc: Option<IbcHooks>,
}

impl StoreDeps {
    /// Creates deps with the given node minimum gas price.
    #[must_use]
    pub fn new(node_min_gas_price: Option<LegacyDec>) -> Self {
        Self {
            node_min_gas_price,
            ibc: None,
        }
    }

    /// Sets the app's IBC checks.
    #[must_use]
    pub fn with_ibc(mut self, hooks: IbcHooks) -> Self {
        self.ibc = Some(hooks);
        self
    }
}

/// `defaultPffSigVerificationCacheCapacity`.
pub const PFF_SIG_CACHE_CAPACITY: usize = 10_000;

/// `PffSigVerificationCache` (celestia `app/pff_sig_cache.go`): a bounded
/// LRU set of certificates whose PFF signatures were verified. It is
/// node-local and only skips repeated verification, so it never changes a
/// result.
#[derive(Debug, Default)]
pub struct PffSigCache {
    // Most recently used last.
    entries: std::collections::VecDeque<[u8; 32]>,
    index: std::collections::HashSet<[u8; 32]>,
}

impl PffSigCache {
    /// `IsCached` (refreshes the entry's recency, like `lru.Cache.Get`).
    pub fn is_cached(&mut self, key: &[u8; 32]) -> bool {
        if !self.index.contains(key) {
            return false;
        }
        if let Some(pos) = self.entries.iter().position(|k| k == key)
            && let Some(k) = self.entries.remove(pos)
        {
            self.entries.push_back(k);
        }
        true
    }

    /// `Cache`.
    pub fn cache(&mut self, key: [u8; 32]) {
        if self.is_cached(&key) {
            return;
        }
        if self.entries.len() >= PFF_SIG_CACHE_CAPACITY
            && let Some(old) = self.entries.pop_front()
        {
            self.index.remove(&old);
        }
        self.entries.push_back(key);
        self.index.insert(key);
    }
}

/// The process-wide PFF signature cache (Go keeps one per `App`).
pub fn pff_sig_cache() -> &'static Mutex<PffSigCache> {
    static CACHE: OnceLock<Mutex<PffSigCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
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
        fee: &Coins,
        msgs: &[Any],
    ) -> sdk::Result<()> {
        x_feegrant::use_granted_fees(ctx, granter, grantee, fee, msgs)
    }

    fn fibre_validate_payment_promise(
        &self,
        ctx: &mut Context<'_>,
        promise: &PaymentPromise,
    ) -> sdk::Result<()> {
        x_fibre::msgs::validate_payment_promise_stateful(ctx, promise).map(|_| ())
    }

    fn pff_sig_cached(&self, key: &[u8; 32]) -> bool {
        pff_sig_cache().lock().is_ok_and(|mut c| c.is_cached(key))
    }

    fn pff_sig_cache(&self, key: [u8; 32]) {
        if let Ok(mut c) = pff_sig_cache().lock() {
            c.cache(key);
        }
    }

    fn fibre_validate_signatures(
        &self,
        ctx: &mut Context<'_>,
        msg: &MsgPayForFibre,
    ) -> sdk::Result<()> {
        x_fibre::msgs::validate_pay_for_fibre_signatures(ctx, msg)
    }

    fn ibc_packet_is_noop(
        &self,
        ctx: &mut Context<'_>,
        kind: IbcPacketMsg,
        msg: &Any,
        recheck: bool,
    ) -> sdk::Result<bool> {
        if let Some(h) = self.ibc {
            return (h.packet_is_noop)(ctx, kind, msg, recheck);
        }
        Err(not_supported("IBC redundant relay check"))
    }

    fn ibc_update_client_check(
        &self,
        ctx: &mut Context<'_>,
        msg: &Any,
        recheck: bool,
    ) -> sdk::Result<()> {
        if let Some(h) = self.ibc {
            return (h.update_client_check)(ctx, msg, recheck);
        }
        // clienttypes.UnpackClientMessage: a nil client message fails before
        // any state is read.
        let m: celestia_proto::ibc::core::client::v1::MsgUpdateClient = decode(msg)?;
        if m.client_message.is_none() {
            return Err(SdkError::new(
                "ibc",
                14,
                "protobuf Any message cannot be nil: failed unpacking protobuf message from Any",
            ));
        }
        Err(not_supported("IBC update client check"))
    }
}
