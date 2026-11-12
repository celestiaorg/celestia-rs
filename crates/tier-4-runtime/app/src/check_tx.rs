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
        let version = self.execution_version();
        sdk::version::set_app_version(version);
        if crate::stores::is_v9(version) {
            return self.check_tx_v9(req);
        }
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
            Some(Err(
                BlobTxError::NonCanonical | BlobTxError::Nested | BlobTxError::NonCanonicalBlob(_),
            )) => {
                let e = SdkError::new("app", 11_147, "blob tx is not canonically encoded");
                return Ok(error_response(&e, 0, 0));
            }
            Some(Err(BlobTxError::Invalid(e))) => return Err(e),
            Some(Ok(u)) => return self.check_blob_tx(req, &u, crate::txpipe::decode_tx),
            None => {}
        }
        let sdk_tx = txdecode::decode(raw).map_err(|e| e.log)?;
        let pfb = MsgPayForBlobs::type_url();
        if sdk_tx.msgs().iter().any(|m| m.type_url == pfb) {
            return Ok(error_response(&x_blob::errors::no_blobs(), 0, 0));
        }
        let header = self.check_header.clone();
        let mut ctx = Context::new(&mut self.ms, header, ExecMode::Check);
        let count = count_executable_msgs(Some(&mut ctx), sdk_tx.gogo_msgs()).unwrap_or_default();
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
        self.forward_check_tx(req.r#type, raw, &sdk_tx, crate::txpipe::decode_tx)
    }

    /// celestia-app v9.0.8 `CheckTx` (app version 9, ADR 0004).
    fn check_tx_v9(&mut self, req: &RequestCheckTx) -> Result<ResponseCheckTx, String> {
        let raw = &req.tx;
        if raw.len() > MAX_TX_SIZE {
            let e = SdkError::new(
                "app",
                11_142,
                format!(
                    "tx size {} bytes is larger than the application's configured MaxTxSize of {MAX_TX_SIZE} bytes for version 9: transaction size exceeds maximum allowed limit",
                    raw.len()
                ),
            );
            return Ok(error_response(&e, 0, 0));
        }
        let blob = match txdecode::unmarshal_blob_tx_v9(raw) {
            Some(Err(BlobTxError::Invalid(e))) => return Err(e),
            Some(Err(e)) => return Err(e.to_string()),
            Some(Ok(u)) => Some(u),
            None => None,
        };
        // Txs repeating a top-level TxRaw field are rejected with an error.
        let sdk_bytes = blob.as_ref().map_or(raw.as_slice(), |u| u.tx.as_slice());
        if txdecode::has_duplicate_tx_raw_field(sdk_bytes) {
            return Err("transaction contains a duplicate TxRaw field".into());
        }
        if let Some(u) = blob {
            return self.check_blob_tx(req, &u, crate::txpipe::decode_tx_v9);
        }
        let sdk_tx = txdecode::decode_v9(raw).map_err(|e| e.log)?;
        let pfb = MsgPayForBlobs::type_url();
        if sdk_tx.msgs().iter().any(|m| m.type_url == pfb) {
            return Ok(error_response(&x_blob::errors::no_blobs(), 0, 0));
        }
        let count = sdk_tx.msgs().len();
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
        self.forward_check_tx(req.r#type, raw, &sdk_tx, crate::txpipe::decode_tx_v9)
    }

    /// `handleBlobCheckTx`.
    fn check_blob_tx(
        &mut self,
        req: &RequestCheckTx,
        u: &UnwrappedBlobTx,
        decode: tx::DecodeFn,
    ) -> Result<ResponseCheckTx, String> {
        match CheckTxType::try_from(req.r#type) {
            Ok(CheckTxType::New) => {
                let inner = decode(&u.tx).map_err(|e| e.log)?;
                validate_blob_tx(&inner, u).map_err(|e| e.log)?;
            }
            Ok(CheckTxType::Recheck) => {}
            Err(_) => return Err(format!("unknown RequestCheckTx type: {}", req.r#type)),
        }
        let sdk_tx = decode(&u.tx).map_err(|e| e.log)?;
        let response = self.forward_check_tx(req.r#type, &u.tx, &sdk_tx, decode)?;
        if req.r#type == i32::from(CheckTxType::New) && response.code == 0 {
            self.blob_tx_cache
                .remember(self.execution_version(), &req.tx);
        }
        Ok(response)
    }

    /// `forwardCheckTx`: `BaseApp.CheckTx` plus the priority-mempool
    /// signer fields.
    fn forward_check_tx(
        &mut self,
        check_type: i32,
        tx_bytes: &[u8],
        sdk_tx: &Tx,
        decode: tx::DecodeFn,
    ) -> Result<ResponseCheckTx, String> {
        let mode = match CheckTxType::try_from(check_type) {
            Ok(CheckTxType::New) => ExecMode::Check,
            Ok(CheckTxType::Recheck) => ExecMode::ReCheck,
            Err(_) => return Err(format!("unknown RequestCheckTx type: {check_type}")),
        };
        let res = self.run_check_tx(mode, tx_bytes, decode);
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
    fn run_check_tx(
        &mut self,
        mode: ExecMode,
        tx_bytes: &[u8],
        decode: tx::DecodeFn,
    ) -> ResponseCheckTx {
        let header = self.check_header.clone();
        let deps = x_ante::StoreDeps::new(self.node_min_gas_price.clone())
            .with_ibc(crate::ibc::ANTE_HOOKS);
        let mut ctx = Context::new(&mut self.ms, header, mode);
        ctx.consensus_params = x_consensus::get_params(&mut ctx)
            .ok()
            .flatten()
            .unwrap_or_default();
        ctx.tx_bytes = tx_bytes.to_vec();
        let decoded = match decode(tx_bytes) {
            Ok(t) => t,
            Err(e) => return error_response(&e, 0, ctx.gas.consumed()),
        };
        if let Err(e) = tx::check_msgs(&decoded) {
            return error_response(&e, 0, ctx.gas.consumed());
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

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn reject_changed_blob(
        app: &mut App,
        proposal: &celestia_proto::celestia_core_proto::abci::RequestProcessProposal,
    ) -> TestResult<bool> {
        use celestia_proto::celestia_core_proto::abci::response_process_proposal::ProposalStatus;
        let version = app.execution_version();
        let Some((index, mut blob)) = proposal.txs.iter().enumerate().find_map(|(i, raw)| {
            if !app.blob_tx_cache.contains(version, raw) {
                return None;
            }
            txdecode::unmarshal_blob_tx_v9(raw)?
                .ok()
                .map(|blob| (i, blob))
        }) else {
            return Ok(false);
        };
        assert_eq!(
            app.process_proposal(proposal)?.status,
            i32::from(ProposalStatus::Accept)
        );
        let data = blob
            .blobs
            .iter_mut()
            .find(|b| !b.data.is_empty())
            .ok_or("nonempty blob required")?;
        data.data[0] ^= 1;
        let mut altered = proposal.clone();
        altered.txs[index] = txdecode::marshal_blob_tx(&blob.tx, &blob.blobs);
        let (size, root) = crate::proposal::block_data_root(version, &altered.txs)?;
        altered.square_size = size;
        altered.data_root_hash = root;
        assert!(!app.blob_tx_cache.contains(version, &altered.txs[index]));
        assert_eq!(
            app.process_proposal(&altered)?.status,
            i32::from(ProposalStatus::Reject)
        );
        Ok(true)
    }

    fn reject_failed_signature(app: &mut App, raw: &[u8], version: u64) -> TestResult {
        use celestia_proto::cosmos::tx::v1beta1::TxRaw;
        use prost::Message;
        let mut blob = txdecode::unmarshal_blob_tx_v9(raw)
            .ok_or("blob required")?
            .map_err(|e| e.to_string())?;
        let mut inner = TxRaw::decode(blob.tx.as_slice())?;
        inner.signatures[0][0] ^= 1;
        blob.tx = inner.encode_to_vec();
        let invalid = txdecode::marshal_blob_tx(&blob.tx, &blob.blobs);
        // Valid blob commitments must not authorize failed SDK/ante validation.
        for check_type in [CheckTxType::New, CheckTxType::Recheck] {
            let failed = app.check_tx(&RequestCheckTx {
                tx: invalid.clone(),
                r#type: i32::from(check_type),
            });
            assert!(!matches!(failed, Ok(ref r) if r.code == 0));
            assert!(!app.blob_tx_cache.contains(version, &invalid));
        }
        Ok(())
    }

    #[test]
    fn recorded_blob_admission_and_proposals_respect_cache_boundaries() -> TestResult {
        use abci_server::Application;
        use celestia_proto::celestia_core_proto::abci::{request, response};
        for fixture in ["blob", "v9-blob"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../conformance/fixtures/devnet")
                .join(fixture)
                .join("abci.rec");
            let records = abci_server::record::parse(&std::fs::read(path)?)?;
            let adapter = crate::AbciApp::new(App::new());
            let shared = adapter.shared();
            shared.lock().map_err(|e| e.to_string())?.blob_tx_cache =
                crate::blob_cache::BlobTxCache::with_capacity(10_000);
            let (mut admitted, mut mutated) = (0, 0);
            for exchange in abci_server::record::exchanges(&records)? {
                let Some(req) = exchange.request.value else {
                    continue;
                };
                if let request::Value::ProcessProposal(proposal) = &req {
                    let mut app = shared.lock().map_err(|e| e.to_string())?;
                    mutated += usize::from(reject_changed_blob(&mut app, proposal)?);
                }
                let check = if let request::Value::CheckTx(check) = &req {
                    let app = shared.lock().map_err(|e| e.to_string())?;
                    Some((
                        check.clone(),
                        app.execution_version(),
                        app.blob_tx_cache
                            .contains(app.execution_version(), &check.tx),
                    ))
                } else {
                    None
                };
                let result = adapter.handle(req);
                if let Some((check, version, before)) = check {
                    if txdecode::unmarshal_blob_tx_v9(&check.tx).is_none() {
                        continue;
                    }
                    let mut app = shared.lock().map_err(|e| e.to_string())?;
                    let after = app.blob_tx_cache.contains(version, &check.tx);
                    if check.r#type == i32::from(CheckTxType::New)
                        && matches!(result, response::Value::CheckTx(ref r) if r.code == 0)
                    {
                        assert!(after);
                        admitted += 1;
                        reject_failed_signature(&mut app, &check.tx, version)?;
                    } else {
                        assert_eq!(
                            after, before,
                            "failed or rechecked tx must not populate cache"
                        );
                    }
                }
            }
            assert!(admitted > 0, "{fixture}: successful admission exercised");
            assert!(
                mutated > 0,
                "{fixture}: warm proposal and changed blob exercised"
            );
        }
        Ok(())
    }
}
