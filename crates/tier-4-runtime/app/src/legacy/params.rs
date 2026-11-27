//! The cosmos-sdk v0.46 parameter subspaces of celestia-app v1
//! (`initParamsKeeper`, `app/app.go` at v3.13.0) with their key tables,
//! and the module `SetParams` calls that InitGenesis makes, from the
//! module's genesis JSON.
//!
//! Gov's subspace lives with its genesis shim ([`x_gov::V046_SUBSPACE`]);
//! the consensus params' subspace `baseapp` is
//! [`super::consensus_params`]. `mint` registers an empty subspace (celestia's
//! mint has no parameters); `icahost`, `minfee` and packet forward
//! middleware join at app version 2.
//!
//! Genesis JSON is decoded as gogoproto `jsonpb` does for these
//! messages: fields by proto name or JSON name, unknown fields rejected,
//! `null` as the zero value, integers as numbers or quoted, durations
//! through `time.ParseDuration`, `sdk.Dec` through `NewDecFromStr`.
//! Every module except crisis then calls `SetParamSet`, which validates
//! each pair (Go panics, here an error).

use math::{Int, LegacyDec};
use sdk::{Context, Result, SdkError};
use serde_json::{Map, Value as Json};
use x_params::v046::{Kind, SendEnabled, Subspace, Value, param, validate};

fn bad(what: impl std::fmt::Display) -> SdkError {
    sdk::error::sdk::json_unmarshal(what.to_string())
}

/// The members of one JSON object, consumed field by field; any member
/// left over is an unknown field.
pub(super) struct Fields {
    message: &'static str,
    members: Map<String, Json>,
}

/// `lower_snake` to protoc's JSON name (`lowerCamel`).
fn json_name(orig: &str) -> String {
    let mut out = String::with_capacity(orig.len());
    let mut upper = false;
    for c in orig.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

impl Fields {
    /// The members of `v`; `null` is the zero message.
    pub(super) fn new(message: &'static str, v: &Json) -> Result<Self> {
        let members = match v {
            Json::Null => Map::new(),
            Json::Object(m) => m.clone(),
            _ => return Err(bad(format!("{message}: not an object"))),
        };
        Ok(Self { message, members })
    }

    /// The member for proto field `orig`, by JSON name or proto name (the
    /// JSON name wins when both are present). `None` when absent or `null`.
    pub(super) fn take(&mut self, orig: &'static str) -> Option<Json> {
        let by_orig = self.members.remove(orig);
        let camel = json_name(orig);
        let by_json = if camel == orig {
            None
        } else {
            self.members.remove(&camel)
        };
        by_json.or(by_orig).filter(|v| !v.is_null())
    }

    fn err(&self, field: &str) -> SdkError {
        bad(format!("{}: invalid {field}", self.message))
    }

    /// An integer field: a JSON number or a quoted one (`None` is 0).
    pub(super) fn int<T: std::str::FromStr + Default>(&mut self, orig: &'static str) -> Result<T> {
        let quoted = match self.take(orig) {
            None => return Ok(T::default()),
            Some(Json::String(s)) => s,
            Some(Json::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
            Some(_) => return Err(self.err(orig)),
        };
        // jsonpb removes the quotes and gives the contents to
        // `encoding/json`: JSON whitespace around the value is allowed and
        // `null` leaves the field at zero.
        let text = quoted.trim_matches([' ', '\t', '\r', '\n']);
        if text == "null" {
            return Ok(T::default());
        }
        // A JSON integer: an optional `-`, then `0` or digits with no
        // leading zero. No `+`, fraction or exponent.
        let digits = text.strip_prefix('-').unwrap_or(text);
        let json_integer = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'));
        if !json_integer {
            return Err(self.err(orig));
        }
        text.parse().map_err(|_| self.err(orig))
    }

    fn bool(&mut self, orig: &'static str) -> Result<bool> {
        match self.take(orig) {
            None => Ok(false),
            Some(Json::Bool(b)) => Ok(b),
            Some(_) => Err(self.err(orig)),
        }
    }

    pub(super) fn string(&mut self, orig: &'static str) -> Result<String> {
        match self.take(orig) {
            None => Ok(String::new()),
            Some(Json::String(s)) => Ok(s),
            Some(_) => Err(self.err(orig)),
        }
    }

    /// A non-nullable `stdduration` (nanoseconds; absent is 0).
    fn duration(&mut self, orig: &'static str) -> Result<i64> {
        match self.take(orig) {
            None => Ok(0),
            Some(Json::String(s)) => sdk::time::parse_duration(&s)
                .map_err(|e| bad(format!("{}: bad Duration: {e}", self.message))),
            Some(_) => Err(self.err(orig)),
        }
    }

    /// An `sdk.Dec` customtype. An absent field is a nil `Dec`, which
    /// every v1 validator rejects (or dereferences: a Go panic).
    fn dec(&mut self, orig: &'static str) -> Result<LegacyDec> {
        match self.take(orig) {
            None => Err(bad(format!("{}: {orig} must be not nil", self.message))),
            Some(Json::String(s)) => LegacyDec::from_str_go(&s)
                .map_err(|e| bad(format!("{}: {orig}: {e}", self.message))),
            Some(_) => Err(self.err(orig)),
        }
    }

    /// A repeated field: `None` when absent or `null` (a nil slice).
    pub(super) fn list(&mut self, orig: &'static str) -> Result<Option<Vec<Json>>> {
        match self.take(orig) {
            None => Ok(None),
            Some(Json::Array(a)) => Ok(Some(a)),
            Some(_) => Err(self.err(orig)),
        }
    }

    /// Rejects any member not consumed (`unknown field`).
    pub(super) fn finish(self) -> Result<()> {
        match self.members.keys().next() {
            None => Ok(()),
            Some(k) => Err(bad(format!("unknown field \"{k}\" in {}", self.message))),
        }
    }
}

macro_rules! table {
    ($name:ident, $space:literal, [$($p:expr),* $(,)?]) => {
        #[doc = concat!("Subspace `", $space, "` and its key table.")]
        pub const $name: Subspace = Subspace {
            name: $space,
            table: &[$($p),*],
        };
    };
}

table!(
    AUTH,
    "auth",
    [
        param(
            "MaxMemoCharacters",
            Kind::Uint64,
            |v| validate::nonzero_u64(v, "invalid max memo characters")
        ),
        param("TxSigLimit", Kind::Uint64, |v| validate::nonzero_u64(
            v,
            "invalid tx signature limit"
        )),
        param(
            "TxSizeCostPerByte",
            Kind::Uint64,
            |v| validate::nonzero_u64(v, "invalid tx size cost per byte")
        ),
        param("SigVerifyCostED25519", Kind::Uint64, |v| {
            validate::nonzero_u64(v, "invalid ED25519 signature verification cost")
        }),
        param("SigVerifyCostSecp256k1", Kind::Uint64, |v| {
            validate::nonzero_u64(v, "invalid SECK256k1 signature verification cost")
        }),
    ]
);

table!(
    BANK,
    "bank",
    [
        param("SendEnabled", Kind::SendEnabled, validate::send_enabled),
        param("DefaultSendEnabled", Kind::Bool, validate::any),
    ]
);

table!(
    STAKING,
    "staking",
    [
        param("UnbondingTime", Kind::Duration, |v| {
            validate::positive_duration(v, "unbonding time must be positive")
        }),
        param("MaxValidators", Kind::Uint32, |v| validate::nonzero_u32(
            v,
            "max validators must be positive"
        )),
        param("MaxEntries", Kind::Uint32, |v| validate::nonzero_u32(
            v,
            "max entries must be positive"
        )),
        param("HistoricalEntries", Kind::Uint32, validate::any),
        param("BondDenom", Kind::Str, validate::bond_denom),
        param("MinCommissionRate", Kind::Dec, |v| validate::unit_dec(
            v,
            "minimum commission rate cannot be negative",
            "minimum commission rate cannot be greater than 100%",
        )),
    ]
);

table!(
    DISTRIBUTION,
    "distribution",
    [
        param("communitytax", Kind::Dec, |v| validate::unit_dec(
            v,
            "community tax must be positive",
            "community tax too large"
        )),
        param("baseproposerreward", Kind::Dec, |v| validate::unit_dec(
            v,
            "base proposer reward must be positive",
            "base proposer reward too large"
        )),
        param("bonusproposerreward", Kind::Dec, |v| validate::unit_dec(
            v,
            "bonus proposer reward must be positive",
            "bonus proposer reward too large"
        )),
        param("withdrawaddrenabled", Kind::Bool, validate::any),
    ]
);

table!(
    SLASHING,
    "slashing",
    [
        param("SignedBlocksWindow", Kind::Int64, |v| {
            validate::positive_i64(v, "signed blocks window must be positive")
        }),
        param("MinSignedPerWindow", Kind::Dec, |v| validate::unit_dec(
            v,
            "min signed per window cannot be negative",
            "min signed per window too large"
        )),
        // `%s` of the `time.Duration` (staking's unbonding time uses `%d`).
        param("DowntimeJailDuration", Kind::Duration, |v| match v {
            Value::Duration(n) if *n <= 0 => Err(format!(
                "downtime jail duration must be positive: {}",
                sdk::time::go_duration_string(i128::from(*n))
            )),
            v => validate::positive_duration(v, "downtime jail duration must be positive"),
        }),
        param(
            "SlashFractionDoubleSign",
            Kind::Dec,
            |v| validate::unit_dec(
                v,
                "double sign slash fraction cannot be negative",
                "double sign slash fraction too large"
            )
        ),
        param("SlashFractionDowntime", Kind::Dec, |v| validate::unit_dec(
            v,
            "downtime slash fraction cannot be negative",
            "downtime slash fraction too large"
        )),
    ]
);

/// crisis `validateConstantFee`: `Coin.IsValid`.
fn constant_fee(v: &Value) -> std::result::Result<(), String> {
    match v {
        Value::Coin { denom, amount }
            if sdk::coins::is_valid_denom(denom) && !amount.is_negative() =>
        {
            Ok(())
        }
        Value::Coin { denom, amount } => Err(format!("invalid constant fee: {amount}{denom}")),
        _ => validate::of_kind(v, Kind::Coin),
    }
}

table!(
    CRISIS,
    "crisis",
    [param("ConstantFee", Kind::Coin, constant_fee)]
);

// ibc-go v6 core keeper: the client key table with the connection
// params registered on it (`modules/core/keeper/keeper.go`).
table!(
    IBC,
    "ibc",
    [
        param("AllowedClients", Kind::Strings, validate::clients),
        param("MaxExpectedTimePerBlock", Kind::Uint64, |v| {
            validate::of_kind(v, Kind::Uint64)
        }),
    ]
);

table!(
    TRANSFER,
    "transfer",
    [
        param("SendEnabled", Kind::Bool, validate::any),
        param("ReceiveEnabled", Kind::Bool, validate::any),
    ]
);

/// blob `validateGovMaxSquareSize`: a non-zero power of two.
fn gov_max_square_size(v: &Value) -> std::result::Result<(), String> {
    match v {
        Value::Uint64(0) => Err("gov max square size cannot be zero".to_owned()),
        Value::Uint64(n) if !n.is_power_of_two() => {
            Err(format!("gov max square size must be a power of two: {n}"))
        }
        _ => validate::of_kind(v, Kind::Uint64),
    }
}

table!(
    BLOB,
    "blob",
    [
        param("GasPerBlobByte", Kind::Uint32, |v| match v {
            Value::Uint32(0) => Err("gas per blob byte cannot be 0".to_owned()),
            _ => validate::of_kind(v, Kind::Uint32),
        }),
        param("GovMaxSquareSize", Kind::Uint64, gov_max_square_size),
    ]
);

/// `types.MinimumDataCommitmentWindow` (x/blobstream at v3.13.0).
const MIN_DATA_COMMITMENT_WINDOW: u64 = 100;
/// `appconsts.DataCommitmentBlocksLimit` (celestia-core `consts`).
const DATA_COMMITMENT_BLOCKS_LIMIT: u64 = 1000;

/// blobstream `validateDataCommitmentWindow`.
fn data_commitment_window(v: &Value) -> std::result::Result<(), String> {
    match v {
        Value::Uint64(n) if *n < MIN_DATA_COMMITMENT_WINDOW => Err(format!(
            "data commitment window {n} must be >= minimum data commitment window {MIN_DATA_COMMITMENT_WINDOW}: invalid data commitment window"
        )),
        Value::Uint64(n) if *n > DATA_COMMITMENT_BLOCKS_LIMIT => Err(format!(
            "data commitment window {n} must be <= data commitment blocks limit {DATA_COMMITMENT_BLOCKS_LIMIT}: invalid data commitment window"
        )),
        _ => validate::of_kind(v, Kind::Uint64),
    }
}

table!(
    QGB,
    "qgb",
    [param(
        "DataCommitmentWindow",
        Kind::Uint64,
        data_commitment_window
    )]
);

/// The registered subspaces of a v1 app (with gov's and `baseapp`), each
/// with its key table. `mint` has an empty table.
pub const V1_SUBSPACES: &[Subspace] = &[
    AUTH,
    BANK,
    STAKING,
    Subspace {
        name: "mint",
        table: &[],
    },
    DISTRIBUTION,
    SLASHING,
    x_gov::V046_SUBSPACE,
    CRISIS,
    TRANSFER,
    IBC,
    BLOB,
    QGB,
    super::consensus_params::SUBSPACE,
];

/// auth `SetParams` from genesis `auth.params`.
pub fn set_auth(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("auth Params", params)?;
    let pairs = [
        (
            "MaxMemoCharacters",
            Value::Uint64(f.int("max_memo_characters")?),
        ),
        ("TxSigLimit", Value::Uint64(f.int("tx_sig_limit")?)),
        (
            "TxSizeCostPerByte",
            Value::Uint64(f.int("tx_size_cost_per_byte")?),
        ),
        (
            "SigVerifyCostED25519",
            Value::Uint64(f.int("sig_verify_cost_ed25519")?),
        ),
        (
            "SigVerifyCostSecp256k1",
            Value::Uint64(f.int("sig_verify_cost_secp256k1")?),
        ),
    ];
    f.finish()?;
    AUTH.set_param_set(ctx, &pairs)
}

/// bank `SetParams` from genesis `bank.params`.
pub fn set_bank(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("bank Params", params)?;
    let send_enabled = f
        .list("send_enabled")?
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let mut e = Fields::new("bank SendEnabled", item)?;
                    let se = SendEnabled {
                        denom: e.string("denom")?,
                        enabled: e.bool("enabled")?,
                    };
                    e.finish()?;
                    Ok(se)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let pairs = [
        ("SendEnabled", Value::SendEnabled(send_enabled)),
        (
            "DefaultSendEnabled",
            Value::Bool(f.bool("default_send_enabled")?),
        ),
    ];
    f.finish()?;
    BANK.set_param_set(ctx, &pairs)
}

/// staking `SetParams` from genesis `staking.params`.
pub fn set_staking(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("staking Params", params)?;
    let pairs = [
        (
            "UnbondingTime",
            Value::Duration(f.duration("unbonding_time")?),
        ),
        ("MaxValidators", Value::Uint32(f.int("max_validators")?)),
        ("MaxEntries", Value::Uint32(f.int("max_entries")?)),
        (
            "HistoricalEntries",
            Value::Uint32(f.int("historical_entries")?),
        ),
        ("BondDenom", Value::Str(f.string("bond_denom")?)),
        (
            "MinCommissionRate",
            Value::Dec(f.dec("min_commission_rate")?),
        ),
    ];
    f.finish()?;
    STAKING.set_param_set(ctx, &pairs)
}

/// distribution `SetParams` from genesis `distribution.params`.
pub fn set_distribution(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("distribution Params", params)?;
    let pairs = [
        ("communitytax", Value::Dec(f.dec("community_tax")?)),
        (
            "baseproposerreward",
            Value::Dec(f.dec("base_proposer_reward")?),
        ),
        (
            "bonusproposerreward",
            Value::Dec(f.dec("bonus_proposer_reward")?),
        ),
        (
            "withdrawaddrenabled",
            Value::Bool(f.bool("withdraw_addr_enabled")?),
        ),
    ];
    f.finish()?;
    DISTRIBUTION.set_param_set(ctx, &pairs)
}

/// slashing `SetParams` from genesis `slashing.params`.
pub fn set_slashing(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("slashing Params", params)?;
    let pairs = [
        (
            "SignedBlocksWindow",
            Value::Int64(f.int("signed_blocks_window")?),
        ),
        (
            "MinSignedPerWindow",
            Value::Dec(f.dec("min_signed_per_window")?),
        ),
        (
            "DowntimeJailDuration",
            Value::Duration(f.duration("downtime_jail_duration")?),
        ),
        (
            "SlashFractionDoubleSign",
            Value::Dec(f.dec("slash_fraction_double_sign")?),
        ),
        (
            "SlashFractionDowntime",
            Value::Dec(f.dec("slash_fraction_downtime")?),
        ),
    ];
    f.finish()?;
    SLASHING.set_param_set(ctx, &pairs)
}

/// crisis `SetConstantFee` from genesis `crisis.constant_fee`: a plain
/// `Set` (no validation). An absent amount is a nil `sdk.Int`, which
/// amino cannot marshal (Go panics).
pub fn set_crisis(ctx: &mut Context<'_>, constant_fee: &Json) -> Result<()> {
    let mut f = Fields::new("Coin", constant_fee)?;
    let denom = f.string("denom")?;
    let amount = match f.take("amount") {
        Some(Json::String(s)) => Int::parse(&s).map_err(|e| bad(format!("Coin: amount: {e}")))?,
        _ => return Err(bad("Coin: amount is nil")),
    };
    f.finish()?;
    CRISIS.set(ctx, "ConstantFee", &Value::Coin { denom, amount })
}

/// ibc 02-client `SetParams` from genesis `ibc.client_genesis.params`.
pub fn set_ibc_client(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("client Params", params)?;
    let clients = f
        .list("allowed_clients")?
        .map(|items| {
            items
                .into_iter()
                .map(|c| match c {
                    Json::String(s) => Ok(s),
                    Json::Null => Ok(String::new()),
                    _ => Err(bad("client Params: invalid allowed_clients")),
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    f.finish()?;
    IBC.set_param_set(ctx, &[("AllowedClients", Value::Strings(clients))])
}

/// ibc 03-connection `SetParams` from genesis
/// `ibc.connection_genesis.params`.
pub fn set_ibc_connection(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("connection Params", params)?;
    let pairs = [(
        "MaxExpectedTimePerBlock",
        Value::Uint64(f.int("max_expected_time_per_block")?),
    )];
    f.finish()?;
    IBC.set_param_set(ctx, &pairs)
}

/// transfer `SetParams` from genesis `transfer.params`.
pub fn set_transfer(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("transfer Params", params)?;
    let pairs = [
        ("SendEnabled", Value::Bool(f.bool("send_enabled")?)),
        ("ReceiveEnabled", Value::Bool(f.bool("receive_enabled")?)),
    ];
    f.finish()?;
    TRANSFER.set_param_set(ctx, &pairs)
}

/// blob `SetParams` from genesis `blob.params`.
pub fn set_blob(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    let mut f = Fields::new("blob Params", params)?;
    let pairs = [
        ("GasPerBlobByte", Value::Uint32(f.int("gas_per_blob_byte")?)),
        (
            "GovMaxSquareSize",
            Value::Uint64(f.int("gov_max_square_size")?),
        ),
    ];
    f.finish()?;
    BLOB.set_param_set(ctx, &pairs)
}

/// blobstream (`qgb`) `SetParams` from genesis `qgb.params`, a pointer:
/// `null` dereferences nil (Go panics).
pub fn set_qgb(ctx: &mut Context<'_>, params: &Json) -> Result<()> {
    if params.is_null() {
        return Err(sdk::error::go_panic(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
    }
    let mut f = Fields::new("qgb Params", params)?;
    let pairs = [(
        "DataCommitmentWindow",
        Value::Uint64(f.int("data_commitment_window")?),
    )];
    f.finish()?;
    QGB.set_param_set(ctx, &pairs)
}
