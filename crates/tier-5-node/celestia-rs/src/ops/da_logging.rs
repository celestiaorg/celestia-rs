//! Runtime DA logging controls for the authenticated DA RPC.
use anyhow::{Context as _, Result};
use std::sync::OnceLock;
use tracing_subscriber::{EnvFilter, Registry, prelude::*, reload};

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Install reload support when the operator starts a DA RPC service.
pub fn init(level: &str, format: &str) -> Result<()> {
    let base = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(super::log_filter(level)));
    let (layer, handle) = reload::Layer::new(base);
    let subscriber = tracing_subscriber::registry().with(layer);
    let ansi = super::use_ansi(
        std::io::IsTerminal::is_terminal(&std::io::stdout()),
        std::env::var("NO_COLOR").ok(),
    );
    let fmt = tracing_subscriber::fmt::layer().with_ansi(ansi);
    if format == "json" {
        subscriber.with(fmt.json()).try_init()?;
    } else {
        subscriber.with(fmt).try_init()?;
    }
    FILTER
        .set(handle)
        .map_err(|_| anyhow::anyhow!("DA logging already initialized"))
}

// Use static Rust targets: an arbitrary component name supplied over DA RPC
// must never become an EnvFilter directive for consensus or validator logs.
const COMPONENTS: &[(&[&str], &str)] = &[
    (&["node"], "celestia_rs::bridge"),
    (&["module/node", "rpc"], "celestia_rs::bridge::node_rpc"),
    (&["rpc"], "celestia_rs::bridge::ws"),
    (
        &["edsstore", "core", "pruner/service"],
        "celestia_rs::bridge::index",
    ),
    (&["module/p2p", "shrex"], "celestia_rs::bridge::peer"),
    (&["share/discovery"], "celestia_rs::bridge::peer::network"),
    (&["shrex-sub"], "celestia_rs::bridge::peer::flood"),
    (&["das", "share/light"], "celestia_rs::bridge::peer::light"),
    (
        &["module/header"],
        "celestia_rs::bridge::peer::light::exchange",
    ),
    (&["state", "module/state"], "celestia_rs::bridge::submit"),
    (
        &["state", "state/txclient"],
        "celestia_rs::bridge::peer::light::wallet",
    ),
    (&["fibre"], "celestia_rs::bridge::submit::fibre"),
    (&["fibre"], "celestia_fibre"),
    (&["pubsub"], "libp2p_gossipsub"),
    (&["dht"], "libp2p_kad"),
    (&["swarm2"], "libp2p_swarm"),
];

pub(crate) fn set_level(name: &str, level: &str) -> Result<()> {
    update(FILTER.get().context("DA logging unavailable")?, name, level)
}

fn update(handle: &reload::Handle<EnvFilter, Registry>, name: &str, level: &str) -> Result<()> {
    let level = match level.to_ascii_lowercase().as_str() {
        "debug" => "debug",
        "" | "info" => "info",
        "warning" | "warn" => "warn",
        "error" => "error",
        "dpanic" | "panic" | "fatal" => "off",
        _ => anyhow::bail!("unrecognized level: {level:?}"),
    };
    let directives = COMPONENTS
        .iter()
        .filter(|(aliases, _)| name == "*" || aliases.contains(&name))
        .map(|(_, target)| format!("{target}={level}").parse())
        .collect::<Result<Vec<_>, _>>()?;
    // go-log accepts a setting for an as-yet unused component.
    if !directives.is_empty() {
        handle.modify(|filter| {
            *filter = directives
                .into_iter()
                .fold(filter.clone(), EnvFilter::add_directive);
        })?;
    }
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
