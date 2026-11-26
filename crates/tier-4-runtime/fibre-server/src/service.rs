//! The `celestia.fibre.v1.Fibre` handlers (Go `server_upload.go`,
//! `server_download.go`): the order of checks, status codes and status
//! messages are Go's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use celestia_proto::celestia::fibre::v1::{
    BlobShard, DownloadShardRequest, DownloadShardResponse, PaymentPromise, UploadShardRequest,
    UploadShardResponse,
};
use prost::Message;
use tokio_util::sync::CancellationToken;
use tonic::{Code, Status};

use crate::assignment::assign;
use crate::metrics::{RejectReason, Rpc};
use crate::params::{BlobConfig, BlobId, Commitment};
use crate::proto::{RETRY_INFO_TYPE_URL, RetryInfo, RpcStatus};
use crate::server::Inner;
use crate::store::{self, StoreError};
use crate::verify::ShardRow;

/// Go `ed25519.SignatureSize`.
const SIGNATURE_SIZE: usize = 64;

/// The cancellation state of one call (Go's request `context.Context`):
/// cancelled when the client goes away (the handler future is dropped)
/// or when the client's `grpc-timeout` deadline passes.
#[derive(Debug, Clone, Default)]
pub struct CallContext {
    token: CancellationToken,
    deadline_hit: Arc<AtomicBool>,
}

impl CallContext {
    /// A context without deadline.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A context that expires after `timeout` (Go `context.WithTimeout`).
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        let ctx = Self::default();
        let token = ctx.token.clone();
        let hit = Arc::clone(&ctx.deadline_hit);
        tokio::spawn(async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = tokio::time::sleep(timeout) => {
                    hit.store(true, Ordering::SeqCst);
                    token.cancel();
                }
            }
        });
        ctx
    }

    /// Cancels the call (Go `cancel()`).
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// The token cancelled with the call.
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    fn is_done(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Go `ctx.Err()` text.
    fn err_text(&self) -> &'static str {
        if self.deadline_hit.load(Ordering::SeqCst) {
            "context deadline exceeded"
        } else {
            "context canceled"
        }
    }

    /// Go `cancellationCode`.
    fn code(&self) -> Code {
        if self.deadline_hit.load(Ordering::SeqCst) {
            Code::DeadlineExceeded
        } else {
            Code::Cancelled
        }
    }
}

/// A verified promise (the results of Go `verifyPromise`).
struct Verified {
    promise: PaymentPromise,
    commitment: Commitment,
    blob_cfg: BlobConfig,
    hash: Vec<u8>,
    prune_at: SystemTime,
}

/// `google.protobuf.Timestamp` as `SystemTime` (absent: Go's zero time).
pub(crate) fn timestamp(ts: Option<&pbjson_types::Timestamp>) -> SystemTime {
    let ts = ts.copied().unwrap_or(pbjson_types::Timestamp {
        seconds: -62_135_596_800,
        nanos: 0,
    });
    let nanos = Duration::from_nanos(u64::try_from(ts.nanos.clamp(0, 999_999_999)).unwrap_or(0));
    let base = if ts.seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(ts.seconds.unsigned_abs()))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(ts.seconds.unsigned_abs()))
    };
    base.and_then(|t| t.checked_add(nanos))
        .unwrap_or(UNIX_EPOCH)
}

/// Go `shardPruneAt`: `max(expires_at, creation + retention)`, anchored on
/// the creation timestamp so re-puts are idempotent.
#[must_use]
pub fn shard_prune_at(
    creation: SystemTime,
    expires_at: SystemTime,
    retention: Duration,
) -> SystemTime {
    let floor = creation.checked_add(retention).unwrap_or(creation);
    if expires_at > floor {
        expires_at
    } else {
        floor
    }
}

/// Go `retryAfterHint`: one prune interval plus up to half an interval of
/// jitter.
pub(crate) fn retry_after_hint(prune_interval: Duration) -> Duration {
    let mut b = [0u8; 8];
    let r = getrandom::fill(&mut b).map_or(0, |()| u64::from_le_bytes(b));
    let half = u64::try_from(prune_interval.as_nanos() / 2)
        .unwrap_or(u64::MAX)
        .max(1);
    prune_interval + Duration::from_nanos(r % half)
}

/// The `ResourceExhausted` budget rejection with its `RetryInfo` detail.
pub(crate) fn budget_exceeded(prune_interval: Duration) -> Status {
    let delay = retry_after_hint(prune_interval);
    let message = "fibre storage budget exceeded";
    let details = RpcStatus {
        code: Code::ResourceExhausted as i32,
        message: message.into(),
        details: vec![pbjson_types::Any {
            type_url: RETRY_INFO_TYPE_URL.into(),
            value: RetryInfo {
                retry_delay: Some(pbjson_types::Duration {
                    seconds: i64::try_from(delay.as_secs()).unwrap_or(i64::MAX),
                    nanos: i32::try_from(delay.subsec_nanos()).unwrap_or(0),
                }),
            }
            .encode_to_vec()
            .into(),
        }],
    };
    Status::with_details(
        Code::ResourceExhausted,
        message,
        Bytes::from(details.encode_to_vec()),
    )
}

/// Runs a blocking closure on the blocking pool; a panic there becomes an
/// error text.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("blocking task failed: {e}"))
}

impl Inner {
    /// Go `Server.UploadShard`.
    pub(crate) async fn upload_shard(
        &self,
        ctx: &CallContext,
        req: UploadShardRequest,
    ) -> Result<UploadShardResponse, Status> {
        let timer = self.metrics.start_rpc(Rpc::Upload);
        let res = self.upload_shard_inner(ctx, req).await;
        timer.finish(res.as_ref().map_or_else(Status::code, |_| Code::Ok));
        res
    }

    #[allow(clippy::too_many_lines)]
    async fn upload_shard_inner(
        &self,
        ctx: &CallContext,
        req: UploadShardRequest,
    ) -> Result<UploadShardResponse, Status> {
        let UploadShardRequest { promise, shard } = req;
        let v = match self.verify_promise(promise).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "payment promise verification failed");
                self.metrics.reject(RejectReason::PromiseInvalid);
                return Err(Status::invalid_argument(format!(
                    "payment promise verification failed: {e}"
                )));
            }
        };
        let commitment_hex = v.commitment.to_string();
        let height = v.promise.height;

        let Some(shard) = shard else {
            tracing::warn!(
                blob_commitment = %commitment_hex,
                promise_height = height,
                error = "shard is required",
                "missing shard in upload request"
            );
            self.metrics.reject(RejectReason::ShardMissing);
            return Err(Status::invalid_argument("shard is required"));
        };
        let shard = Arc::new(shard);

        if let Err(e) = self.verify_assignment(&v, &shard).await {
            tracing::warn!(
                blob_commitment = %commitment_hex,
                promise_height = height,
                error = %e,
                "shard assignment verification failed"
            );
            self.metrics.reject(RejectReason::AssignmentInvalid);
            return Err(Status::invalid_argument(format!(
                "shard assignment verification failed: {e}"
            )));
        }

        if let Err(e) = self.verify_shard(ctx, &v, &shard).await {
            tracing::warn!(
                blob_commitment = %commitment_hex,
                promise_height = height,
                error = %e,
                "shard verification failed"
            );
            self.metrics.reject(RejectReason::ShardInvalid);
            return Err(Status::invalid_argument(format!(
                "shard verification failed: {e}"
            )));
        }

        // Serialize identical uploads so concurrent duplicates can't each
        // reserve occupancy for a single stored shard.
        let _guard = tokio::select! {
            biased;
            g = self.uploads.acquire(&v.hash) => g,
            () = ctx.token().cancelled() => {
                return Err(Status::new(
                    ctx.code(),
                    format!("waiting for existing upload: {}", ctx.err_text()),
                ));
            }
        };

        let store = self.store()?;
        let status = {
            let store = Arc::clone(&store);
            let (c, h) = (v.commitment.0, v.hash.clone());
            blocking(move || store.shard_status(&c, &h))
                .await
                .and_then(|r| r.map_err(|e| e.to_string()))
        };
        let status = match status {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    blob_commitment = %commitment_hex,
                    promise_height = height,
                    error = %e,
                    "failed to check store for existing shard"
                );
                return Err(Status::internal(format!(
                    "failed to check if store has the commitment: {e}"
                )));
            }
        };

        if !status.has {
            let size = store::shard_binary_size(&shard);
            let new_reservation = !status.accounted;
            if new_reservation && !self.occupancy.reserve(size) {
                self.metrics.reject(RejectReason::BudgetExceeded);
                return Err(crate::service::budget_exceeded(
                    self.cfg.tuning.prune_interval,
                ));
            }

            let put_start = Instant::now();
            let put = {
                let store = Arc::clone(&store);
                let shard = Arc::clone(&shard);
                let promise = v.promise.clone();
                let prune_at = v.prune_at;
                let token = ctx.token().clone();
                blocking(move || store.put(&promise, &shard, prune_at, &token))
                    .await
                    .unwrap_or_else(|e| Err(StoreError::Other(e)))
            };
            if let Err(e) = put {
                if new_reservation {
                    self.occupancy.release(size);
                }
                self.metrics.observe_store_put(false, put_start.elapsed());
                if ctx.is_done() {
                    tracing::warn!(
                        blob_commitment = %commitment_hex,
                        promise_height = height,
                        error = %e,
                        "store upload aborted by client cancellation"
                    );
                    return Err(Status::new(
                        ctx.code(),
                        format!("store upload aborted: {e}"),
                    ));
                }
                tracing::error!(
                    blob_commitment = %commitment_hex,
                    promise_height = height,
                    error = %e,
                    "failed to store upload data"
                );
                return Err(Status::internal(format!(
                    "failed to store upload data: {e}"
                )));
            }
            self.metrics.observe_store_put(true, put_start.elapsed());
        }

        let sign_start = Instant::now();
        let signature = self.sign_promise(&v.promise).await;
        self.metrics
            .observe_sign(signature.is_ok(), sign_start.elapsed());
        let signature = match signature {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    blob_commitment = %commitment_hex,
                    promise_height = height,
                    error = %e,
                    "failed to sign payment promise"
                );
                return Err(Status::internal(format!(
                    "failed to sign payment promise: {e}"
                )));
            }
        };

        let rows = shard.rows.len();
        let row_size = shard.rows.first().map_or(0, |r| r.data.len());
        let shard_bytes = (rows as u64) * (row_size as u64);
        self.metrics.add_upload_bytes(shard_bytes);
        tracing::debug!(
            blob_commitment = %commitment_hex,
            promise_height = height,
            upload_size = v.promise.blob_size,
            shard_bytes,
            rows_count = rows,
            row_size,
            "successful upload"
        );
        Ok(UploadShardResponse {
            validator_signature: signature,
        })
    }

    /// Go `SignPaymentPromiseValidator`.
    async fn sign_promise(&self, promise: &PaymentPromise) -> Result<Vec<u8>, String> {
        let stripped = x_fibre::promise::stripped_sign_bytes(promise);
        let sig = self
            .signer
            .sign_raw_bytes(
                &promise.chain_id,
                x_fibre::promise::SIGN_BYTES_PREFIX,
                &stripped,
            )
            .await
            .map_err(|e| format!("signing payment promise: {e}"))?;
        if sig.len() != SIGNATURE_SIZE {
            return Err(format!(
                "invalid signature length: expected {SIGNATURE_SIZE}, got {}",
                sig.len()
            ));
        }
        Ok(sig)
    }

    /// Go `verifyPromise`: stateless checks, the local minimum, then the
    /// stateful query.
    async fn verify_promise(&self, promise: Option<PaymentPromise>) -> Result<Verified, String> {
        let Some(promise) = promise else {
            return Err("nil payment promise".into());
        };
        x_fibre::msgs::validate_basic_promise(&promise)
            .map_err(|e| format!("invalid payment promise: {}", e.log))?;
        x_fibre::promise::from_proto_check(&promise)
            .map_err(|e| format!("invalid payment promise proto: {e}"))?;
        let chain_id = self.chain_id();
        if promise.chain_id != chain_id {
            return Err(format!(
                "payment promise chain ID mismatch: expected {chain_id}, got {}",
                promise.chain_id
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        let blob_cfg = BlobConfig::for_version(promise.blob_version as u8)
            .map_err(|e| format!("unsupported blob version {}: {e}", promise.blob_version))?;
        x_fibre::promise::validate(&promise)
            .map_err(|e| format!("payment promise validation failed: {e}"))?;
        let min = self.min_upload_size();
        #[allow(clippy::cast_sign_loss)]
        if u64::from(promise.blob_size) < min as u64 {
            return Err(format!(
                "upload size {} is below local minimum {min} bytes",
                promise.blob_size
            ));
        }
        let verified = self
            .state
            .verify_promise(&promise)
            .await
            .map_err(|e| format!("stateful validation: {e}"))?;
        let hash = x_fibre::promise::hash(&promise)
            .map_err(|e| format!("computing payment promise hash: {e}"))?;
        let commitment = Commitment::from_bytes(&promise.commitment)?;
        let prune_at = shard_prune_at(
            timestamp(promise.creation_timestamp.as_ref()),
            verified.expires_at,
            verified.shard_retention,
        );
        Ok(Verified {
            promise,
            commitment,
            blob_cfg,
            hash,
            prune_at,
        })
    }

    /// Go `verifyAssignment`.
    async fn verify_assignment(&self, v: &Verified, shard: &BlobShard) -> Result<(), String> {
        #[allow(clippy::cast_sign_loss)]
        let height = v.promise.height as u64;
        let set = self
            .state
            .validator_set(height)
            .await
            .map_err(|e| format!("getting validator set at height {height}: {e}"))?;
        let pub_key = self
            .signer
            .public_key()
            .await
            .map_err(|e| format!("getting validator public key: {e}"))?;
        let address = crypto::ed25519::address(&pub_key);
        let Some((index, _)) = set.get_by_address(&address) else {
            return Err(format!(
                "validator {} not in set at height {height}",
                hex::encode_upper(address)
            ));
        };
        let rows: Vec<u32> = shard.rows.iter().map(|r| r.index).collect();
        let shard_map = assign(
            &v.commitment.0,
            &set.voting_powers(),
            v.blob_cfg.total_rows(),
            v.blob_cfg.original_rows,
            self.cfg.min_rows_per_validator,
            self.cfg.liveness_threshold,
        );
        shard_map.verify(index, &rows).map_err(|e| e.to_string())
    }

    /// Go `verifyShard`: waits for a verification slot (cancellable), then
    /// verifies on the blocking pool.
    async fn verify_shard(
        &self,
        ctx: &CallContext,
        v: &Verified,
        shard: &Arc<BlobShard>,
    ) -> Result<(), String> {
        let permit = tokio::select! {
            biased;
            p = Arc::clone(&self.verify_slots).acquire_owned() => {
                p.map_err(|e| format!("acquiring verifier: {e}"))?
            }
            () = ctx.token().cancelled() => {
                return Err(format!("acquiring verifier: {}", ctx.err_text()));
            }
        };
        let verifier = self.verifier.clone();
        let shard = Arc::clone(shard);
        let commitment = v.commitment.0;
        let upload_size = v.promise.blob_size;
        blocking(move || {
            let _permit = permit;
            let rows: Vec<ShardRow<'_>> = shard
                .rows
                .iter()
                .map(|r| ShardRow {
                    index: r.index,
                    data: &r.data,
                    proof: &r.proof,
                })
                .collect();
            verifier
                .verify_shard(&commitment, upload_size, &rows, &shard.rlcs)
                .map_err(|e| e.to_string())
        })
        .await?
    }

    /// Go `Server.DownloadShard`.
    pub(crate) async fn download_shard(
        &self,
        req: DownloadShardRequest,
    ) -> Result<DownloadShardResponse, Status> {
        let timer = self.metrics.start_rpc(Rpc::Download);
        let res = self.download_shard_inner(req).await;
        timer.finish(res.as_ref().map_or_else(Status::code, |_| Code::Ok));
        res
    }

    async fn download_shard_inner(
        &self,
        req: DownloadShardRequest,
    ) -> Result<DownloadShardResponse, Status> {
        let id = match BlobId::from_bytes(&req.blob_id) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, "invalid blob ID");
                return Err(Status::invalid_argument(format!("invalid blob ID: {e}")));
            }
        };
        if let Err(e) = id.blob_config() {
            tracing::error!(version = id.version(), error = %e, "unsupported blob version");
            return Err(Status::invalid_argument(format!(
                "unsupported blob version: {e}"
            )));
        }
        let commitment = id.commitment();
        let store = self.store()?;
        let get_start = Instant::now();
        let got = blocking(move || store.get(&commitment.0))
            .await
            .unwrap_or_else(|e| Err(StoreError::Other(e)));
        self.metrics
            .observe_store_get(got.is_ok(), get_start.elapsed());
        let shard = match got {
            Ok(shard) => shard,
            Err(e) if e.is_not_found() => {
                tracing::debug!(blob_commitment = %commitment, "no blob shard found for commitment");
                return Err(Status::not_found(format!(
                    "no blob shard found for commitment {commitment}"
                )));
            }
            Err(e) => {
                tracing::error!(blob_commitment = %commitment, error = %e, "failed to retrieve blob shard");
                return Err(Status::internal(format!(
                    "failed to retrieve blob shard: {e}"
                )));
            }
        };
        let row_size = shard.rows.first().map_or(0, |r| r.data.len());
        let shard_size: u64 = shard.rows.iter().map(|r| r.data.len() as u64).sum();
        self.metrics.add_download_bytes(shard_size);
        tracing::info!(
            blob_commitment = %commitment,
            rows = shard.rows.len(),
            row_size,
            "download successful"
        );
        Ok(DownloadShardResponse { shard: Some(shard) })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // Go TestShardPruneAt.
    #[test]
    fn prune_at_never_before_expiry() {
        let creation = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let hour = Duration::from_secs(3600);
        // Retention longer than the promise: the floor wins.
        assert_eq!(
            shard_prune_at(creation, creation + hour, 2 * hour),
            creation + 2 * hour
        );
        // Promise valid longer than retention: expiry wins.
        assert_eq!(
            shard_prune_at(creation, creation + 3 * hour, hour),
            creation + 3 * hour
        );
        // Equal: either.
        assert_eq!(
            shard_prune_at(creation, creation + hour, hour),
            creation + hour
        );
        // Zero retention.
        assert_eq!(
            shard_prune_at(creation, creation + hour, Duration::ZERO),
            creation + hour
        );
    }

    #[test]
    fn retry_hint_range() {
        let i = Duration::from_secs(60);
        for _ in 0..100 {
            let d = retry_after_hint(i);
            assert!(d >= i && d < i + i / 2, "{d:?}");
        }
    }

    #[test]
    fn budget_status_carries_retry_info() {
        let st = budget_exceeded(Duration::from_secs(60));
        assert_eq!(st.code(), Code::ResourceExhausted);
        assert_eq!(st.message(), "fibre storage budget exceeded");
        let details = RpcStatus::decode(st.details()).unwrap();
        assert_eq!(details.code, 8);
        assert_eq!(details.details.len(), 1);
        assert_eq!(details.details[0].type_url, RETRY_INFO_TYPE_URL);
        let ri = RetryInfo::decode(details.details[0].value.as_ref()).unwrap();
        let d = ri.retry_delay.unwrap();
        assert!((60..90).contains(&d.seconds));
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            timestamp(Some(&pbjson_types::Timestamp {
                seconds: 10,
                nanos: 5
            })),
            UNIX_EPOCH + Duration::new(10, 5)
        );
        assert!(timestamp(None) < UNIX_EPOCH);
    }
}
