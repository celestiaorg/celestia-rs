//! Bech32 address codecs (`sdk.AccAddressFromBech32`,
//! `address.Bech32Codec`) and module addresses (`authtypes.NewModuleAddress`).
//!
//! Errors mirror Go: decoding and prefix failures are plain errors (ABCI
//! codespace `undefined`, code 1); empty or over-long decoded addresses are
//! `ErrUnknownAddress`.

use crate::error::{SdkError, sdk};

/// Account address prefix.
pub const ACC_PREFIX: &str = "celestia";
/// Validator operator address prefix.
pub const VALOPER_PREFIX: &str = "celestiavaloper";
/// Validator consensus address prefix.
pub const VALCONS_PREFIX: &str = "celestiavalcons";

/// An error from a Go `error` that has no registered ABCI code
/// (`errorsmod.ABCIInfo` maps it to codespace `undefined`, code 1).
pub fn undefined(log: impl Into<String>) -> SdkError {
    SdkError::new("undefined", 1, log)
}

/// Go `sdk.VerifyAddressFormat` with the default (no-op) verifier.
pub fn verify_address_format(bz: &[u8]) -> Result<(), SdkError> {
    if bz.is_empty() {
        return Err(sdk::unknown_address("addresses cannot be empty"));
    }
    if bz.len() > 255 {
        return Err(sdk::unknown_address("address max length is 255"));
    }
    Ok(())
}

/// Go `sdk.GetFromBech32` + `VerifyAddressFormat` for `prefix`.
pub fn from_bech32(s: &str, prefix: &str) -> Result<Vec<u8>, SdkError> {
    if s.trim().is_empty() {
        return Err(undefined("empty address string is not allowed"));
    }
    let (hrp, bz) = encoding::bech32::decode_and_convert(s).map_err(undefined)?;
    if hrp != prefix {
        return Err(undefined(format!(
            "invalid Bech32 prefix; expected {prefix}, got {hrp}"
        )));
    }
    verify_address_format(&bz)?;
    Ok(bz)
}

/// The account `address.Codec` (`x/auth/codec` `bech32Codec.StringToBytes`):
/// like `AccAddressFromBech32` but a wrong prefix is `ErrLogic`.
pub fn acc_codec_to_bytes(s: &str) -> Result<Vec<u8>, SdkError> {
    codec_to_bytes(s, ACC_PREFIX)
}

/// The validator operator `address.Codec` (`Bech32Codec.StringToBytes`).
pub fn val_codec_to_bytes(s: &str) -> Result<Vec<u8>, SdkError> {
    codec_to_bytes(s, VALOPER_PREFIX)
}

fn codec_to_bytes(s: &str, prefix: &str) -> Result<Vec<u8>, SdkError> {
    if s.trim().is_empty() {
        return Err(undefined("empty address string is not allowed"));
    }
    let (hrp, bz) = encoding::bech32::decode_and_convert(s).map_err(undefined)?;
    if hrp != prefix {
        return Err(sdk::logic(format!(
            "hrp does not match bech32 prefix: expected '{prefix}' got '{hrp}'"
        )));
    }
    verify_address_format(&bz)?;
    Ok(bz)
}

/// Go `sdk.AccAddressFromBech32` with its error's text (`err.Error()`):
/// `bech32.DecodeAndConvert`'s `decoding bech32 failed: ...`, the prefix
/// check, then `VerifyAddressFormat` (wrapping `unknown address`).
pub fn acc_from_bech32_go(s: &str) -> Result<Vec<u8>, String> {
    acc_from_bech32_frames(s).map_err(|(e, _)| e)
}

/// [`acc_from_bech32_go`] as Go's `%v` prints the error: an error built by
/// `errorsmod.Wrap` (`VerifyAddressFormat`'s) appends the frame where it
/// was created (`cosmossdk.io/errors` `writeSimpleFrame`), i.e. the
/// executing celestia-app's cosmos-sdk module path and line.
pub fn acc_from_bech32_go_v(s: &str) -> Result<Vec<u8>, String> {
    acc_from_bech32_frames(s).map_err(|(e, line)| match line {
        Some(line) => format!(
            "{e} [celestiaorg/cosmos-sdk@{}/types/address.go:{line}]",
            if crate::version::is_v9(crate::version::app_version()) {
                "v0.52.8"
            } else {
                "v0.52.12"
            }
        ),
        None => e,
    })
}

/// `AccAddressFromBech32`: the error text and, for the errors
/// `VerifyAddressFormat` builds with `errorsmod.Wrap(f)`, its line in
/// `types/address.go` (the same in celestia-app v10.4.0's and v9.0.8's
/// cosmos-sdk).
fn acc_from_bech32_frames(s: &str) -> Result<Vec<u8>, (String, Option<u32>)> {
    if s.trim().is_empty() {
        return Err(("empty address string is not allowed".into(), None));
    }
    let (hrp, bz) = encoding::bech32::decode_and_convert(s).map_err(|e| (e, None))?;
    if hrp != ACC_PREFIX {
        return Err((
            format!("invalid Bech32 prefix; expected {ACC_PREFIX}, got {hrp}"),
            None,
        ));
    }
    if bz.is_empty() {
        return Err((
            "addresses cannot be empty: unknown address".into(),
            Some(173),
        ));
    }
    if bz.len() > 255 {
        return Err((
            format!(
                "address max length is 255, got {}: unknown address",
                bz.len()
            ),
            Some(177),
        ));
    }
    Ok(bz)
}

/// The account `address.Codec` with its error's text (`err.Error()`, as
/// `%s` prints it): `empty address string is not allowed`, `decoding bech32
/// failed: ...`, `hrp does not match bech32 prefix: ...: internal logic
/// error`, `addresses cannot be empty: unknown address`.
pub fn acc_codec_go(s: &str) -> Result<Vec<u8>, String> {
    acc_codec_to_bytes(s).map_err(|e| e.log)
}

/// The validator operator `address.Codec` with its error's text.
pub fn val_codec_go(s: &str) -> Result<Vec<u8>, String> {
    val_codec_to_bytes(s).map_err(|e| e.log)
}

/// Go `sdk.AccAddressFromBech32` / the account address codec.
pub fn acc_from_bech32(s: &str) -> Result<Vec<u8>, SdkError> {
    from_bech32(s, ACC_PREFIX)
}

/// Go `sdk.ValAddressFromBech32`.
pub fn val_from_bech32(s: &str) -> Result<Vec<u8>, SdkError> {
    from_bech32(s, VALOPER_PREFIX)
}

/// Go `sdk.ConsAddressFromBech32`.
pub fn cons_from_bech32(s: &str) -> Result<Vec<u8>, SdkError> {
    from_bech32(s, VALCONS_PREFIX)
}

/// Bech32 encoding (`AccAddress.String()` etc.; empty bytes encode to "").
#[must_use]
pub fn to_bech32(prefix: &str, bz: &[u8]) -> String {
    if bz.is_empty() {
        return String::new();
    }
    encoding::bech32::encode(prefix, bz).unwrap_or_default()
}

/// `authtypes.NewModuleAddress(name)`: `sha256(name)[:20]`.
#[must_use]
pub fn module_address(name: &str) -> Vec<u8> {
    hash::sha256(name.as_bytes())[..20].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_address_gov() {
        // celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7 is the gov module.
        assert_eq!(
            to_bech32(ACC_PREFIX, &module_address("gov")),
            "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
        );
        let bz = acc_from_bech32("celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7");
        assert_eq!(bz, Ok(module_address("gov")));
        assert_eq!(
            acc_from_bech32("").map_err(|e| (e.codespace, e.code)),
            Err(("undefined", 1))
        );
        assert!(val_from_bech32("celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7").is_err());
    }
}
