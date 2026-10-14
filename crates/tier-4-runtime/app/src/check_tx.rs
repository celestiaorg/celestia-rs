//! `CheckTx` (celestia-app v10 `app/check_tx.go` over cosmos-sdk
//! `BaseApp.CheckTx`): mempool admission on the check state.
//!
//! `Err(message)` is a Go error returned from `CheckTx`; the ABCI socket
//! server answers it with `ResponseException`.

use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_proto::celestia_core_proto::abci::{CheckTxType, RequestCheckTx, ResponseCheckTx};
use prost::Name;
use sdk::tx::Tx;
use sdk::{Context, ExecMode, SdkError};
use txdecode::{BlobTxError, UnwrappedBlobTx};

use crate::msg_count::{MAX_SDK_MESSAGES, count_executable_msgs};
use crate::proposal::{MAX_TX_SIZE, validate_blob_tx};
use crate::{App, fibre_tx, tx};

/// `responseCheckTxWithEvents(err, gw, gu, nil, false)`.
#[allow(clippy::cast_possible_wrap)]
fn error_response(e: &SdkError, gas_wanted: u64, gas_used: u64) -> ResponseCheckTx {
    ResponseCheckTx {
        code: e.code,
        codespace: e.codespace.to_owned(),
        log: e.log.clone(),
        gas_wanted: gas_wanted as i64,
        gas_used: gas_used as i64,
        ..ResponseCheckTx::default()
    }
}

/// `signerDataFromTx`: the first signature's public key address and
/// sequence.
fn signer_data(tx: &Tx) -> Result<(Vec<u8>, u64), String> {
    let failed = |e: x_ante::Fail| match e {
        x_ante::Fail::Err(e) | x_ante::Fail::ZeroCtx(e) => e.log,
        x_ante::Fail::Panic(p) => format!("failed to get signer data: {p}"),
    };
    let sigs = x_ante::txview::signatures_v2(tx).map_err(failed)?;
    let Some(first) = sigs.first() else {
        return Err("tx of type *tx.wrapper contains no signatures".into());
    };
    let Some(pk) = &first.pub_key else {
        return Err("tx signer 0 has no associated pubkey".into());
    };
    let addr = pk.address().map_err(failed)?;
    Ok((addr, first.sequence))
}

impl App {
    /// `CheckTx`.
    pub fn check_tx(&mut self, req: &RequestCheckTx) -> Result<ResponseCheckTx, String> {
        let raw = &req.tx;
        if raw.len() > MAX_TX_SIZE {
            let e = SdkError::new(
                "app",
                11_142,
                format!(
                    "tx size {} bytes is larger than the application's configured MaxTxSize of {MAX_TX_SIZE} bytes for version 10: transaction size exceeds maximum allowed limit",
                    raw.len()
                ),
            );
            return Ok(error_response(&e, 0, 0));
        }
        match txdecode::unmarshal_blob_tx(raw) {
            Some(Err(BlobTxError::NonCanonical | BlobTxError::Nested)) => {
                let e = SdkError::new("app", 11_147, "blob tx is not canonically encoded");
                return Ok(error_response(&e, 0, 0));
            }
            Some(Err(BlobTxError::Invalid(e))) => return Err(e),
            Some(Ok(u)) => return self.check_blob_tx(req, &u),
            None => {}
        }
        let sdk_tx = txdecode::decode(raw).map_err(|e| e.log)?;
        let pfb = MsgPayForBlobs::type_url();
        if sdk_tx.msgs().iter().any(|m| m.type_url == pfb) {
            return Ok(error_response(&x_blob::errors::no_blobs(), 0, 0));
        }
        let header = self.check_header.clone();
        let mut ctx = Context::new(&mut self.ms, header, ExecMode::Check);
        let count = count_executable_msgs(Some(&mut ctx), sdk_tx.msgs()).unwrap_or_default();
        drop(ctx);
        if count > MAX_SDK_MESSAGES {
            let e = SdkError::new(
                "app",
                11_143,
                format!(
                    "tx contains {count} messages, limit is {MAX_SDK_MESSAGES}: transaction exceeds maximum allowed SDK message count"
                ),
            );
            return Ok(error_response(&e, 0, 0));
        }
        if let Err(e) = fibre_tx::validate_shape(&sdk_tx) {
            return Ok(error_response(&e, 0, 0));
        }
        self.forward_check_tx(req.r#type, raw, &sdk_tx)
    }

    /// `handleBlobCheckTx`.
    fn check_blob_tx(
        &mut self,
        req: &RequestCheckTx,
        u: &UnwrappedBlobTx,
    ) -> Result<ResponseCheckTx, String> {
        match CheckTxType::try_from(req.r#type) {
            Ok(CheckTxType::New) => {
                let inner = txdecode::decode(&u.tx).map_err(|e| e.log)?;
                validate_blob_tx(&inner, u).map_err(|e| e.log)?;
            }
            Ok(CheckTxType::Recheck) => {}
            Err(_) => return Err(format!("unknown RequestCheckTx type: {}", req.r#type)),
        }
        let sdk_tx = txdecode::decode(&u.tx).map_err(|e| e.log)?;
        self.forward_check_tx(req.r#type, &u.tx, &sdk_tx)
    }

    /// `forwardCheckTx`: `BaseApp.CheckTx` plus the priority-mempool
    /// signer fields.
    fn forward_check_tx(
        &mut self,
        check_type: i32,
        tx_bytes: &[u8],
        sdk_tx: &Tx,
    ) -> Result<ResponseCheckTx, String> {
        let mode = match CheckTxType::try_from(check_type) {
            Ok(CheckTxType::New) => ExecMode::Check,
            Ok(CheckTxType::Recheck) => ExecMode::ReCheck,
            Err(_) => return Err(format!("unknown RequestCheckTx type: {check_type}")),
        };
        let res = self.run_check_tx(mode, tx_bytes);
        if res.code != 0 {
            return Ok(res);
        }
        let (address, sequence) = signer_data(sdk_tx)?;
        Ok(ResponseCheckTx {
            address,
            sequence,
            ..res
        })
    }

    /// `BaseApp.CheckTx` → `runTx(execModeCheck|ReCheck)`.
    #[allow(clippy::cast_possible_wrap)]
    fn run_check_tx(&mut self, mode: ExecMode, tx_bytes: &[u8]) -> ResponseCheckTx {
        let header = self.check_header.clone();
        let deps = x_ante::StoreDeps::new(self.node_min_gas_price.clone());
        let mut ctx = Context::new(&mut self.ms, header, mode);
        ctx.consensus_params = x_consensus::get_params(&mut ctx)
            .ok()
            .flatten()
            .unwrap_or_default();
        ctx.tx_bytes = tx_bytes.to_vec();
        let decoded = match txdecode::decode(tx_bytes) {
            Ok(t) => t,
            Err(e) => return error_response(&e, 0, 0),
        };
        if let Err(e) = tx::check_msgs(&decoded) {
            return error_response(&e, 0, 0);
        }
        match x_ante::ante_handle(&mut ctx, &decoded, false, &deps) {
            Ok(out) => ResponseCheckTx {
                gas_wanted: out.gas_wanted as i64,
                gas_used: ctx.gas.consumed() as i64,
                priority: out.priority,
                ..ResponseCheckTx::default()
            },
            Err(e) => error_response(&e.error, e.gas_wanted, ctx.gas.consumed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(tx: Vec<u8>) -> Result<ResponseCheckTx, String> {
        App::new().check_tx(&RequestCheckTx { tx, r#type: 0 })
    }

    #[test]
    fn stateless_rejections() {
        // Oversized: app/11142, no gas.
        let r = check(vec![0; MAX_TX_SIZE + 1]).unwrap_or_default();
        assert_eq!((r.codespace.as_str(), r.code), ("app", 11_142));
        // Non-canonical BlobTx: app/11147.
        let r = check(b"\x1a\x04BLOB\x78\x01".to_vec()).unwrap_or_default();
        assert_eq!((r.codespace.as_str(), r.code), ("app", 11_147));
        // A BlobTx without blobs and an undecodable tx are Go errors
        // (ResponseException).
        assert!(check(b"\x0a\x02\x0a\x00\x1a\x04BLOB".to_vec()).is_err());
        assert!(check(vec![0xff, 0xff]).is_err());
    }
}
