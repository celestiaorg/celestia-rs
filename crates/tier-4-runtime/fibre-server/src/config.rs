//! Server configuration (Go `fibre.ServerConfig` without the parts an
//! embedded server does not need: the app and signer gRPC addresses,
//! object storage, telemetry exporters).

use std::time::Duration;

use crate::params::{DEFAULT_PROTOCOL_PARAMS, Fraction, ProtocolParams};
use crate::store::{StoreConfig, StoreError};

/// Go `fibregrpc.DefaultMaxConnections`.
pub const DEFAULT_MAX_CONNECTIONS: i64 = 16;
/// Go `fibregrpc.DefaultMaxConcurrentStreams`.
pub const DEFAULT_MAX_CONCURRENT_STREAMS: i64 = 13;
/// Default `max_inflight_upload_bytes` (2 GiB): about seven maximum
/// uploads at once, safe on the 16 GB reference machine.
pub const DEFAULT_MAX_INFLIGHT_UPLOAD_BYTES: i64 = 2 << 30;
/// Go's default `server_listen_address`.
pub const DEFAULT_LISTEN_ADDRESS: &str = "0.0.0.0:7980";

/// The `[health]` table (Go `HealthConfig`). Durations use Go syntax such
/// as `10s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthConfig {
    /// How often the checks run (default `10s`).
    pub check_interval: String,
    /// Deadline of each check (default `3s`).
    pub probe_timeout: String,
    /// Maximum age of the latest block before the chain counts as stalled
    /// (default `2m`).
    pub max_block_age: String,
}

impl Default for HealthConfig {
    /// Go `DefaultHealthConfig`.
    fn default() -> Self {
        Self {
            check_interval: "10s".into(),
            probe_timeout: "3s".into(),
            max_block_age: "2m".into(),
        }
    }
}

/// Parsed [`HealthConfig`] (Go `healthSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthSettings {
    /// Interval between check cycles.
    pub check_interval: Duration,
    /// Deadline of each check.
    pub probe_timeout: Duration,
    /// Maximum block age.
    pub max_block_age: Duration,
}

impl HealthConfig {
    /// Go `HealthConfig.parse`.
    pub fn parse(&self) -> Result<HealthSettings, String> {
        Ok(HealthSettings {
            check_interval: parse_positive_duration("check_interval", &self.check_interval)?,
            probe_timeout: parse_positive_duration("probe_timeout", &self.probe_timeout)?,
            max_block_age: parse_positive_duration("max_block_age", &self.max_block_age)?,
        })
    }
}

fn parse_positive_duration(name: &str, value: &str) -> Result<Duration, String> {
    match crate::godur::parse(value) {
        Ok(d) if d > 0 => Ok(Duration::from_nanos(d.unsigned_abs())),
        _ => Err(format!(
            "health.{name} must be a positive duration, got {}",
            gostr::quote(value)
        )),
    }
}

/// Timings Go hard-codes (`internal/grpc/server.go`, `server_prune.go`).
/// Not part of Go's configuration; tests shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct Tuning {
    /// Go `connectionTimeout`: TLS handshake deadline.
    pub connection_timeout: Duration,
    /// Go `keepAliveMaxConnIdle`.
    pub max_connection_idle: Duration,
    /// Go `keepAliveMaxConnAge` (jittered by ±10% per connection).
    pub max_connection_age: Duration,
    /// Go `keepAliveMaxConnAgeGrace`.
    pub max_connection_age_grace: Duration,
    /// Go `keepAlivePingTime`.
    pub keepalive_ping_time: Duration,
    /// Go `keepAlivePingTimeout`.
    pub keepalive_ping_timeout: Duration,
    /// Go `pruneInterval`.
    pub prune_interval: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            connection_timeout: Duration::from_secs(15),
            max_connection_idle: Duration::from_secs(30),
            max_connection_age: Duration::from_secs(5 * 60),
            max_connection_age_grace: Duration::from_secs(2 * 60),
            keepalive_ping_time: Duration::from_secs(2 * 60),
            keepalive_ping_timeout: Duration::from_secs(20),
            prune_interval: Duration::from_secs(60),
        }
    }
}

/// Configuration of the Fibre [`crate::Server`] (Go `ServerConfig`).
///
/// The node fills the operator-facing fields (Go's TOML keys, named the
/// same) and keeps the protocol-derived ones from [`ServerConfig::default`].
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// TCP address to listen on (`server_listen_address`, default
    /// `0.0.0.0:7980`).
    pub server_listen_address: String,
    /// Local minimum padded upload size in bytes, header included and
    /// parity excluded (`min_upload_size`, default 262144).
    pub min_upload_size: i64,
    /// Cap on concurrent shard verifications (`upload_verify_workers`,
    /// default: available parallelism, Go's `GOMAXPROCS`).
    pub upload_verify_workers: i64,
    /// Cap on concurrent connections (`max_connections`, default 16).
    pub max_connections: i64,
    /// Cap on concurrent streams per connection (`max_concurrent_streams`,
    /// default 13).
    pub max_concurrent_streams: i64,
    /// Process-wide cap on memory held by in-flight uploads
    /// (`max_inflight_upload_bytes`, default 2 GiB; not in Go). Each
    /// upload reserves twice the length its gRPC frame declares (receive
    /// buffer plus decoded copy) before its body is read, and keeps the
    /// reservation until its handler finishes; uploads that do not fit are
    /// refused with `ResourceExhausted`.
    pub max_inflight_upload_bytes: i64,
    /// The `[health]` table.
    pub health: HealthConfig,
    /// Store directory and backend (`storage_backend`; the path is not a
    /// TOML key in Go: the node sets it).
    pub store: StoreConfig,
    /// Disables the storage limiter (`unlimited_budget`).
    pub unlimited_budget: bool,
    /// Fraction of stake needed for reconstruction (protocol).
    pub liveness_threshold: Fraction,
    /// Minimum rows per validator (protocol).
    pub min_rows_per_validator: usize,
    /// K (protocol).
    pub original_rows: usize,
    /// Maximum shard size, for the budget provisioning warning (protocol).
    pub max_shard_size: usize,
    /// Maximum gRPC message size, both directions (protocol, ~132 MiB).
    pub max_message_size: usize,
    /// Go's hard-coded timings.
    #[doc(hidden)]
    pub tuning: Tuning,
}

impl Default for ServerConfig {
    /// Go `DefaultServerConfig` (store path empty: the node sets it).
    fn default() -> Self {
        Self::from_params(&DEFAULT_PROTOCOL_PARAMS)
    }
}

impl ServerConfig {
    /// Go `NewServerConfigFromParams`.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn from_params(p: &ProtocolParams) -> Self {
        Self {
            server_listen_address: DEFAULT_LISTEN_ADDRESS.into(),
            min_upload_size: (p.rows * p.min_row_size) as i64,
            upload_verify_workers: std::thread::available_parallelism()
                .map_or(1, std::num::NonZero::get) as i64,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_concurrent_streams: DEFAULT_MAX_CONCURRENT_STREAMS,
            max_inflight_upload_bytes: DEFAULT_MAX_INFLIGHT_UPLOAD_BYTES,
            health: HealthConfig::default(),
            store: StoreConfig::default(),
            unlimited_budget: false,
            liveness_threshold: p.liveness_threshold,
            min_rows_per_validator: p.min_rows_per_validator(),
            original_rows: p.rows,
            max_shard_size: p.max_shard_size(),
            max_message_size: p.max_message_size(),
            tuning: Tuning::default(),
        }
    }

    /// Go `ServerConfig.Validate` (the checks that apply when embedded),
    /// with Go's error texts. Returns the parsed health settings.
    pub fn validate(&self) -> Result<HealthSettings, ConfigError> {
        if self.server_listen_address.is_empty() {
            return Err(ConfigError("server listen address is required".into()));
        }
        let health = self
            .health
            .parse()
            .map_err(|e| ConfigError(format!("health config: {e}")))?;
        self.store.validate().map_err(|e| {
            ConfigError(format!(
                "store config: {}",
                match e {
                    StoreError::Config(m) => m,
                    other => other.to_string(),
                }
            ))
        })?;
        #[allow(clippy::cast_possible_wrap)]
        let max_blob = DEFAULT_PROTOCOL_PARAMS.max_blob_size as i64;
        if self.min_upload_size < 1 || self.min_upload_size > max_blob {
            return Err(ConfigError(format!(
                "min_upload_size must be between 1 and {max_blob} bytes, got {}",
                self.min_upload_size
            )));
        }
        if self.upload_verify_workers < 1 {
            return Err(ConfigError(format!(
                "upload_verify_workers must be at least 1, got {}",
                self.upload_verify_workers
            )));
        }
        if self.max_connections < 1 {
            return Err(ConfigError(format!(
                "max_connections must be at least 1, got {}",
                self.max_connections
            )));
        }
        if self.max_concurrent_streams < 1 {
            return Err(ConfigError(format!(
                "max_concurrent_streams must be at least 1, got {}",
                self.max_concurrent_streams
            )));
        }
        if self.max_concurrent_streams > i64::from(u32::MAX) {
            return Err(ConfigError(format!(
                "max_concurrent_streams must not exceed {}, got {}",
                u32::MAX,
                self.max_concurrent_streams
            )));
        }
        let min_budget = 2 * self.max_message_size as u64;
        if u64::try_from(self.max_inflight_upload_bytes).unwrap_or(0) < min_budget {
            return Err(ConfigError(format!(
                "max_inflight_upload_bytes must be at least {min_budget} (twice the maximum message size), got {}",
                self.max_inflight_upload_bytes
            )));
        }
        Ok(health)
    }
}

/// An invalid [`ServerConfig`]; the text is Go's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn valid() -> ServerConfig {
        ServerConfig {
            store: StoreConfig::new("/tmp/fibre-config-test"),
            ..ServerConfig::default()
        }
    }

    #[test]
    fn defaults_match_go() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.server_listen_address, "0.0.0.0:7980");
        assert_eq!(cfg.min_upload_size, 256 << 10);
        assert_eq!(cfg.max_connections, 16);
        assert_eq!(cfg.max_concurrent_streams, 13);
        assert_eq!(cfg.health, HealthConfig::default());
        assert_eq!(cfg.original_rows, 4096);
        assert_eq!(
            cfg.max_message_size,
            DEFAULT_PROTOCOL_PARAMS.max_message_size()
        );
        assert!(cfg.upload_verify_workers >= 1);
        assert!(!cfg.unlimited_budget);
        let h = valid().validate().unwrap();
        assert_eq!(h.check_interval, Duration::from_secs(10));
        assert_eq!(h.probe_timeout, Duration::from_secs(3));
        assert_eq!(h.max_block_age, Duration::from_secs(120));
    }

    // Go TestServerConfigValidateConnectionCaps.
    #[test]
    fn connection_caps() {
        let mut cfg = valid();
        cfg.max_connections = 0;
        assert!(
            cfg.validate()
                .unwrap_err()
                .0
                .contains("max_connections must be at least 1")
        );
        cfg.max_connections = DEFAULT_MAX_CONNECTIONS;
        cfg.max_concurrent_streams = 0;
        assert!(
            cfg.validate()
                .unwrap_err()
                .0
                .contains("max_concurrent_streams must be at least 1")
        );
        cfg.max_concurrent_streams = i64::from(u32::MAX) + 1;
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "max_concurrent_streams must not exceed 4294967295, got 4294967296"
        );
    }

    // Go TestServerConfigMinUploadSize.
    #[test]
    fn min_upload_size() {
        for size in [-1, 0, 1, 256 << 10, 32 << 20, 128 << 20, (128 << 20) + 1] {
            let mut cfg = valid();
            cfg.min_upload_size = size;
            let res = cfg.validate();
            if (1..=128 << 20).contains(&size) {
                res.unwrap();
            } else {
                assert_eq!(
                    res.unwrap_err().0,
                    format!("min_upload_size must be between 1 and 134217728 bytes, got {size}")
                );
            }
        }
    }

    #[test]
    fn other_rules() {
        let mut cfg = valid();
        cfg.upload_verify_workers = 0;
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "upload_verify_workers must be at least 1, got 0"
        );
        let mut cfg = valid();
        cfg.server_listen_address.clear();
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "server listen address is required"
        );
        let mut cfg = valid();
        cfg.health.probe_timeout = "0s".into();
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "health config: health.probe_timeout must be a positive duration, got \"0s\""
        );
        let mut cfg = valid();
        cfg.health.check_interval = "soon".into();
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "health config: health.check_interval must be a positive duration, got \"soon\""
        );
        let mut cfg = valid();
        cfg.store.path = std::path::PathBuf::new();
        assert_eq!(
            cfg.validate().unwrap_err().0,
            "store config: store path is required"
        );
    }
}
