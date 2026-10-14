//! Replays one devnet scenario and prints wall time, peak RSS and (for the
//! redb backend) on-disk size. Run one process per measurement:
//!
//! ```text
//! cargo run -p devnet-conformance --example replay_stats -- <mem|redb> <scenario> [keep_recent]
//! ```

#![allow(clippy::print_stdout, clippy::cast_precision_loss)]

use std::time::Instant;

use app::{App, AppOptions};
use devnet_conformance::replay::replay_into;

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let backend = args.get(1).map_or("mem", String::as_str);
    let scenario = args.get(2).map_or("staking", String::as_str);
    let keep_recent: u64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(100);
    let home = std::env::temp_dir().join(format!("replay-stats-{}", std::process::id()));
    let start_rss = peak_rss_kib();
    let start = Instant::now();
    let mut app = match backend {
        "redb" => {
            let mut options = AppOptions::default();
            options.store.pruning.keep_recent = keep_recent;
            App::open_with(&home, options)?
        }
        _ => App::new(),
    };
    let report = replay_into(scenario, &mut app, 0, None)?;
    let elapsed = start.elapsed();
    let disk = std::fs::metadata(app::state_db_path(&home)).map_or(0, |m| m.len());
    let cache = app.multistore().root().node_cache().stats();
    println!(
        "scenario={scenario} backend={backend} heights={} parity={} time_ms={} peak_rss_mib={:.1} rss_growth_mib={:.1} disk_kib={} cache_nodes={} hits={} misses={}",
        report.matched_heights.len(),
        report.first_mismatch.is_none() && report.errors.is_empty(),
        elapsed.as_millis(),
        peak_rss_kib() as f64 / 1024.0,
        (peak_rss_kib() - start_rss) as f64 / 1024.0,
        disk / 1024,
        cache.len,
        cache.hits,
        cache.misses,
    );
    drop(app);
    let _ = std::fs::remove_dir_all(&home);
    Ok(())
}
