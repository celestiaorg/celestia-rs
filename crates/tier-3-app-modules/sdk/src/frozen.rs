//! celestia-app v11.0.0-mocha/corto frozen-account recovery rules.
//! Lists and recovery addresses are consensus inputs from app commit
//! 58be8ae90d00518800df5506b3a1e988f009c1c6 (app/frozen).

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::{Context, Result, SdkError};

/// The patch release's activation version, independent of v12 mainline rules.
pub const ACTIVATION_VERSION: u64 = 11;
/// Maximum fee in utia paid by a transaction with a listed signer.
pub const MAX_FEE: u64 = 1_000_000;
/// Go's ErrFrozen message.
pub const ERROR: &str = "account is frozen: funds can only move to the whitehat address";

/// A chain's immutable account list and recovery destination.
pub struct Network {
    listed: HashSet<Vec<u8>>,
    /// The exact string used by MsgSetWithdrawAddress's allow-list check.
    pub whitehat_text: &'static str,
    /// The decoded recovery address used for bank sends and withdrawals.
    pub whitehat: Vec<u8>,
}

impl Network {
    /// Whether the chain lists this address.
    #[must_use]
    pub fn has(&self, address: &[u8]) -> bool {
        self.listed.contains(address)
    }
}

#[allow(clippy::expect_used)] // Invalid compiled consensus data must fail like Go mustDecode.
fn decode(text: &str) -> Vec<u8> {
    let (prefix, bytes) =
        encoding::bech32::decode_and_convert(text).expect("valid embedded frozen address");
    assert!(prefix == "celestia" && matches!(bytes.len(), 20 | 32));
    bytes
}

/// Rules for the exact chain ID; all other chains have no restrictions.
#[must_use]
pub fn network(chain_id: &str) -> Option<&'static Network> {
    static NETWORKS: OnceLock<[Network; 3]> = OnceLock::new();
    let index = match chain_id {
        "celestia" => 0,
        "mocha-5" => 1,
        "corto-1" => 2,
        _ => return None,
    };
    Some(
        &NETWORKS.get_or_init(|| {
            [
                (
                    include_str!("frozen/celestia.txt"),
                    "celestia1atuap9kfnt6y8ykvn3437e7j3z8qffh33pzt53",
                ),
                (
                    include_str!("frozen/mocha-5.txt"),
                    "celestia1xk4tasrhcr3w6jyq4xzd8x6aapv9hmfuh8mcxn",
                ),
                (
                    include_str!("frozen/corto-1.txt"),
                    "celestia1yny4dz396zd2vp85fnqynnnq6pnj00lz95xv09",
                ),
            ]
            .map(|(list, whitehat_text)| Network {
                listed: list
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
                    .map(decode)
                    .collect(),
                whitehat_text,
                whitehat: decode(whitehat_text),
            })
        })[index],
    )
}

/// Go deliberately checks committed consensus params, not the executing binary.
#[must_use]
pub fn active(ctx: &Context<'_>) -> bool {
    ctx.consensus_params
        .version
        .as_ref()
        .is_some_and(|v| v.app >= ACTIVATION_VERSION)
}

/// The registered error, optionally wrapped with a Go-style detail prefix.
#[must_use]
pub fn error(detail: &str) -> SdkError {
    SdkError::new(
        "frozen",
        2,
        if detail.is_empty() {
            ERROR.into()
        } else {
            format!("{detail}: {ERROR}")
        },
    )
}

/// Apply the bank restriction at the SDK's normal send-restriction boundary.
pub fn check_send(ctx: &Context<'_>, from: &[u8], to: &[u8]) -> Result<()> {
    if let Some(n) = network(&ctx.header.chain_id)
        && n.has(from)
        && to != n.whitehat
        && to != &hash::sha256(b"fee_collector")[..20]
        && active(ctx)
    {
        return Err(error(""));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_lists_and_network_isolation() {
        for (chain, data, digest, count) in [
            (
                "celestia",
                include_str!("frozen/celestia.txt"),
                "96faa31190aec74db35efec56f3f6aaeb16137c3fb988163b7a95a8a00d8f6a3",
                41963,
            ),
            (
                "mocha-5",
                include_str!("frozen/mocha-5.txt"),
                "bc2841e16bd4219adab31c6b1376363037547d63aae773b51d7dac4f0d39b629",
                3,
            ),
            (
                "corto-1",
                include_str!("frozen/corto-1.txt"),
                "c88aae7e7e0af99ae8e2e45b5dc3cae222f4af3aa2cb1e531fcb139fe456fc11",
                3,
            ),
        ] {
            assert_eq!(
                encoding::hex::encode(&hash::sha256(data.as_bytes())),
                digest
            );
            assert_eq!(network(chain).unwrap().listed.len(), count);
        }
        assert!(network("mocha-4").is_none());
        let n = network("corto-1").unwrap();
        let sender = decode("celestia1g4vjdrx32rq0vd4qckv4d93g05062lsprfdpmy");
        let mut ms = crate::MultiStore::new();
        let mut ctx = Context::new(
            &mut ms,
            crate::Header {
                chain_id: "corto-1".into(),
                ..Default::default()
            },
            crate::ExecMode::Finalize,
        );
        for version in [10, 11, 12] {
            ctx.consensus_params.version = Some(
                celestia_proto::celestia_core_proto::tendermint::types::VersionParams {
                    app: version,
                },
            );
            assert_eq!(check_send(&ctx, &sender, &[7; 20]).is_err(), version >= 11);
            assert!(check_send(&ctx, &sender, &n.whitehat).is_ok());
            assert!(check_send(&ctx, &sender, &hash::sha256(b"fee_collector")[..20]).is_ok());
        }
    }
}
