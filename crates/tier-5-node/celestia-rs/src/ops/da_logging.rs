//! Runtime severity overrides for the DA service hosted in this process.
use anyhow::{Context as _, Result};
use std::sync::OnceLock;
use tracing_subscriber::{EnvFilter, Registry, prelude::*, reload};

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

// Component names used by the Go DA node and their Rust tracing targets.
const COMPONENTS: &[(&str, &str)] = &[
    ("node", "celestia_rs::bridge"),
    ("module/p2p", "celestia_rs::bridge::peer"),
    ("module/header", "celestia_rs::bridge::header_rpc"),
    ("das", "celestia_rs::bridge::peer::light"),
    ("module/state", "celestia_rs::bridge::submit"),
    ("shrex", "celestia_rs::bridge::peer::inbound"),
    ("pubsub", "libp2p_gossipsub"),
    ("dht", "libp2p_kad"),
    ("swarm2", "libp2p_swarm"),
];

pub(super) fn init(filter: EnvFilter, ansi: bool, format: &str) {
    let (layer, handle) = reload::Layer::new(filter);
    let registry = tracing_subscriber::registry().with(layer);
    let output = tracing_subscriber::fmt::layer().with_ansi(ansi);
    let installed = if format == "json" {
        registry.with(output.json()).try_init()
    } else {
        registry.with(output).try_init()
    };
    if installed.is_ok() {
        let _ = FILTER.set(handle);
    }
}

pub(crate) fn set(component: &str, level: &str) -> Result<()> {
    let handle = FILTER.get().context("DA logging unavailable")?;
    update(handle, component, level)
}

fn update(
    handle: &reload::Handle<EnvFilter, Registry>,
    component: &str,
    level: &str,
) -> Result<()> {
    let level = match level.to_ascii_lowercase().as_str() {
        "" | "info" => "info",
        "debug" => "debug",
        "warn" => "warn",
        "error" => "error",
        // Rust tracing's highest event severity is ERROR.
        "dpanic" | "panic" | "fatal" => "off",
        _ => anyhow::bail!("unrecognized level: {level}"),
    };
    let directives = COMPONENTS
        .iter()
        .filter(|(alias, target)| component == "*" || component == *alias || component == *target)
        .map(|(_, target)| format!("{target}={level}").parse())
        .collect::<Result<Vec<_>, _>>()?;
    // Go accepts subsystem names before a logger has emitted its first event.
    handle.modify(|filter| {
        let mut next = filter.clone();
        for directive in directives {
            next = next.add_directive(directive);
        }
        *filter = next;
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_da_severity_preserves_the_consensus_filter() {
        let (layer, handle) = reload::Layer::new(EnvFilter::new("error"));
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            let enabled =
                || tracing::enabled!(target: "celestia_rs::bridge::peer", tracing::Level::DEBUG);
            assert!(!enabled());
            update(&handle, "module/p2p", "debug").unwrap();
            assert!(enabled());
            assert!(!tracing::enabled!(target: "consensus", tracing::Level::DEBUG));
            update(&handle, "*", "warn").unwrap();
            assert!(!enabled());
            assert!(tracing::enabled!(target: "celestia_rs::bridge::peer", tracing::Level::WARN));
            assert!(!tracing::enabled!(target: "consensus", tracing::Level::WARN));
        });
    }
}
