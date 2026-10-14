//! Prometheus metrics (`[instrumentation] prometheus = true`,
//! `prometheus_listen_addr`, `namespace`), named like celestia-core's
//! (`cometbft_consensus_height`, `cometbft_p2p_peers`,
//! `cometbft_mempool_size`, …) with the `chain_id` label Go adds, so Go
//! dashboards and alerts work unchanged.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

type Probe = Box<dyn Fn() -> f64 + Send + Sync>;

/// `stdprometheus.DefBuckets`.
const DEF_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
struct Histogram {
    counts: [u64; DEF_BUCKETS.len()],
    sum: f64,
    count: u64,
}

#[derive(Default)]
struct State {
    /// `(subsystem_name, labels)` → value.
    gauges: BTreeMap<(String, String), f64>,
    counters: BTreeMap<(String, String), f64>,
    histograms: BTreeMap<String, Histogram>,
    last_block_time: Option<i64>,
}

/// The node's metrics registry.
pub struct Metrics {
    namespace: String,
    chain_id: String,
    state: Mutex<State>,
    probes: Mutex<Vec<(String, Probe)>>,
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

/// What the consensus loop reports for a committed block.
#[derive(Debug, Clone, Default)]
pub struct BlockReport {
    /// Height.
    pub height: i64,
    /// Commit round.
    pub round: i32,
    /// Block time (ns since the epoch).
    pub time_ns: i64,
    /// Number of txs.
    pub num_txs: usize,
    /// Block size (protobuf bytes).
    pub size_bytes: usize,
    /// Validators of the block (count, total power).
    pub validators: (usize, i64),
    /// Validators missing from the block's last commit (count, power).
    pub missing: (usize, i64),
    /// Evidence in the block (count, power).
    pub byzantine: (usize, i64),
    /// Our validator: (address hex, power, signed the last commit).
    pub own: Option<(String, i64, bool)>,
}

impl Metrics {
    /// A registry for `chain_id` under `namespace` (Go default `cometbft`).
    #[must_use]
    pub fn new(namespace: &str, chain_id: &str) -> Arc<Self> {
        Arc::new(Self {
            namespace: if namespace.is_empty() {
                "cometbft".into()
            } else {
                namespace.to_owned()
            },
            chain_id: chain_id.to_owned(),
            state: Mutex::new(State::default()),
            probes: Mutex::new(Vec::new()),
        })
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets gauge `name` (e.g. `consensus_height`).
    pub fn set(&self, name: &str, v: f64) {
        self.st().gauges.insert((name.to_owned(), String::new()), v);
    }

    /// Sets a gauge with extra labels (`key="value"` list).
    pub fn set_labeled(&self, name: &str, labels: &str, v: f64) {
        self.st()
            .gauges
            .insert((name.to_owned(), labels.to_owned()), v);
    }

    /// Adds to counter `name`.
    pub fn add(&self, name: &str, labels: &str, v: f64) {
        *self
            .st()
            .counters
            .entry((name.to_owned(), labels.to_owned()))
            .or_default() += v;
    }

    /// Observes a histogram value.
    pub fn observe(&self, name: &str, v: f64) {
        let mut st = self.st();
        let h = st.histograms.entry(name.to_owned()).or_default();
        for (i, b) in DEF_BUCKETS.iter().enumerate() {
            if v <= *b {
                h.counts[i] += 1;
            }
        }
        h.sum += v;
        h.count += 1;
    }

    /// Registers a gauge sampled at scrape time (peers, mempool size).
    pub fn probe(&self, name: &str, f: impl Fn() -> f64 + Send + Sync + 'static) {
        self.probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((name.to_owned(), Box::new(f)));
    }

    /// Records a committed block (`recordMetrics` in celestia-core).
    #[allow(clippy::cast_precision_loss)]
    pub fn block(&self, r: &BlockReport) {
        self.set("consensus_height", (r.height + 1) as f64);
        self.set("consensus_latest_block_height", r.height as f64);
        self.set("consensus_rounds", f64::from(r.round));
        self.set("consensus_num_txs", r.num_txs as f64);
        self.add("consensus_total_txs", "", r.num_txs as f64);
        self.set("consensus_block_size_bytes", r.size_bytes as f64);
        self.set("consensus_validators", r.validators.0 as f64);
        self.set("consensus_validators_power", r.validators.1 as f64);
        self.set("consensus_missing_validators", r.missing.0 as f64);
        self.set("consensus_missing_validators_power", r.missing.1 as f64);
        self.set("consensus_byzantine_validators", r.byzantine.0 as f64);
        self.set("consensus_byzantine_validators_power", r.byzantine.1 as f64);
        if let Some((addr, power, signed)) = &r.own {
            let l = format!("validator_address=\"{addr}\"");
            self.set_labeled("consensus_validator_power", &l, *power as f64);
            if *signed {
                self.set_labeled(
                    "consensus_validator_last_signed_height",
                    &l,
                    r.height as f64,
                );
            } else {
                self.add("consensus_validator_missed_blocks", &l, 1.0);
            }
        }
        let prev = self.st().last_block_time.replace(r.time_ns);
        if let Some(p) = prev {
            self.observe(
                "consensus_block_interval_seconds",
                (r.time_ns - p) as f64 / 1e9,
            );
        }
    }

    /// Prometheus text exposition.
    #[must_use]
    pub fn render(&self) -> String {
        let ns = &self.namespace;
        let chain = format!("chain_id=\"{}\"", self.chain_id);
        let labels = |extra: &str| {
            if extra.is_empty() {
                format!("{{{chain}}}")
            } else {
                format!("{{{chain},{extra}}}")
            }
        };
        let mut out = String::new();
        let mut gauges: BTreeMap<(String, String), f64> = self.st().gauges.clone();
        for (name, f) in self
            .probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            gauges.insert((name.clone(), String::new()), f());
        }
        let mut last = String::new();
        for ((name, l), v) in &gauges {
            if *name != last {
                let _ = writeln!(out, "# TYPE {ns}_{name} gauge");
                last.clone_from(name);
            }
            let _ = writeln!(out, "{ns}_{name}{} {v}", labels(l));
        }
        let st = self.st();
        last.clear();
        for ((name, l), v) in &st.counters {
            if *name != last {
                let _ = writeln!(out, "# TYPE {ns}_{name} counter");
                last.clone_from(name);
            }
            let _ = writeln!(out, "{ns}_{name}{} {v}", labels(l));
        }
        for (name, h) in &st.histograms {
            let _ = writeln!(out, "# TYPE {ns}_{name} histogram");
            for (i, b) in DEF_BUCKETS.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "{ns}_{name}_bucket{} {}",
                    labels(&format!("le=\"{b}\"")),
                    h.counts[i]
                );
            }
            let _ = writeln!(
                out,
                "{ns}_{name}_bucket{} {}",
                labels("le=\"+Inf\""),
                h.count
            );
            let _ = writeln!(out, "{ns}_{name}_sum{} {}", labels(""), h.sum);
            let _ = writeln!(out, "{ns}_{name}_count{} {}", labels(""), h.count);
        }
        out
    }
}

/// Serves `/metrics` on `listener` (spawned on the current runtime).
pub fn serve(listener: tokio::net::TcpListener, metrics: Arc<Metrics>) {
    let app = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || {
            let m = metrics.clone();
            async move {
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    m.render(),
                )
            }
        }),
    );
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "metrics server failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposition() {
        let m = Metrics::new("", "c");
        m.probe("p2p_peers", || 3.0);
        m.block(&BlockReport {
            height: 5,
            time_ns: 1_000_000_000,
            own: Some(("AB".into(), 10, false)),
            ..BlockReport::default()
        });
        m.block(&BlockReport {
            height: 6,
            time_ns: 7_000_000_000,
            own: Some(("AB".into(), 10, true)),
            ..BlockReport::default()
        });
        let s = m.render();
        assert!(
            s.contains("cometbft_consensus_height{chain_id=\"c\"} 7"),
            "{s}"
        );
        assert!(s.contains("cometbft_p2p_peers{chain_id=\"c\"} 3"));
        assert!(s.contains(
            "cometbft_consensus_validator_missed_blocks{chain_id=\"c\",validator_address=\"AB\"} 1"
        ));
        assert!(s.contains(
            "cometbft_consensus_block_interval_seconds_bucket{chain_id=\"c\",le=\"10\"} 1"
        ));
        assert!(s.contains("cometbft_consensus_block_interval_seconds_sum{chain_id=\"c\"} 6"));
    }
}
