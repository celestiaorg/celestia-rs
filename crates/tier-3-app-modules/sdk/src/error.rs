//! ABCI-visible errors (`codespace` + `code`), mirroring `cosmossdk.io/errors`.

/// An error with an ABCI codespace and code. `code` and `codespace` are part
/// of `ExecTxResult` and therefore consensus-relevant; `log` is not.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{log}")]
pub struct SdkError {
    /// Module codespace (`sdk`, `bank`, …).
    pub codespace: &'static str,
    /// Code within the codespace (never 0 for an error).
    pub code: u32,
    /// Human-readable message (non-deterministic across implementations).
    pub log: String,
}

impl SdkError {
    /// Creates an error.
    pub fn new(codespace: &'static str, code: u32, log: impl Into<String>) -> Self {
        Self {
            codespace,
            code,
            log: log.into(),
        }
    }

    /// Returns a copy with `msg: ` prepended to the log (`errors.Wrap`).
    #[must_use]
    pub fn wrap(mut self, msg: impl AsRef<str>) -> Self {
        self.log = format!("{}: {}", msg.as_ref(), self.log);
        self
    }
}

/// Result alias.
pub type Result<T> = core::result::Result<T, SdkError>;

/// A recovered Go panic inside a transaction (`sdkerrors.ErrPanic`:
/// codespace `undefined`, code 111222).
pub fn go_panic(log: impl Into<String>) -> SdkError {
    SdkError::new("undefined", 111_222, log)
}

/// An error that is not registered with a codespace (plain `fmt.Errorf` /
/// `errors.New` in Go); `ABCIInfo` reports it as `undefined`/1.
pub fn internal(log: impl Into<String>) -> SdkError {
    SdkError::new("undefined", 1, log)
}

macro_rules! sdk_errors {
    ($($name:ident = $code:expr, $desc:expr;)*) => {
        /// Root `sdk` codespace errors (`types/errors`).
        pub mod sdk {
            use super::SdkError;
            $(
                #[doc = $desc]
                pub fn $name(log: impl AsRef<str>) -> SdkError {
                    let log = log.as_ref();
                    let full = if log.is_empty() { $desc.to_string() } else { format!("{log}: {}", $desc) };
                    SdkError::new("sdk", $code, full)
                }
            )*
        }
    };
}

sdk_errors! {
    tx_decode = 2, "tx parse error";
    invalid_sequence = 3, "invalid sequence";
    unauthorized = 4, "unauthorized";
    insufficient_funds = 5, "insufficient funds";
    unknown_request = 6, "unknown request";
    invalid_address = 7, "invalid address";
    invalid_pub_key = 8, "invalid pubkey";
    unknown_address = 9, "unknown address";
    invalid_coins = 10, "invalid coins";
    out_of_gas = 11, "out of gas";
    memo_too_large = 12, "memo too large";
    insufficient_fee = 13, "insufficient fee";
    too_many_signatures = 14, "maximum number of signatures exceeded";
    no_signatures = 15, "no signatures supplied";
    json_marshal = 16, "failed to marshal JSON bytes";
    json_unmarshal = 17, "failed to unmarshal JSON bytes";
    invalid_request = 18, "invalid request";
    tx_in_mempool_cache = 19, "tx already in mempool";
    mempool_is_full = 20, "mempool is full";
    tx_too_large = 21, "tx too large";
    key_not_found = 22, "key not found";
    invalid_signer = 24, "tx intended signer does not match the given signer";
    invalid_height = 26, "invalid height";
    invalid_version = 27, "invalid version";
    invalid_chain_id = 28, "invalid chain-id";
    invalid_type = 29, "invalid type";
    tx_timeout_height = 30, "tx timeout height";
    unknown_extension_options = 31, "unknown extension options";
    wrong_sequence = 32, "incorrect account sequence";
    pack_any = 33, "failed packing protobuf message to Any";
    unpack_any = 34, "failed unpacking protobuf message from Any";
    logic = 35, "internal logic error";
    conflict = 36, "conflict";
    not_supported = 37, "feature not supported";
    not_found = 38, "not found";
    io = 39, "Internal IO error";
    app_config = 40, "error in app.toml";
    invalid_gas_limit = 41, "invalid gas limit";
    tx_timeout = 42, "tx timeout";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_wrap() {
        let e = sdk::insufficient_funds("1utia is smaller than 2utia");
        assert_eq!((e.codespace, e.code), ("sdk", 5));
        assert_eq!(e.log, "1utia is smaller than 2utia: insufficient funds");
        assert_eq!(sdk::out_of_gas("").log, "out of gas");
        assert!(e.wrap("send").log.starts_with("send: "));
    }
}
