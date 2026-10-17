//! celestia-app v10 `x/zkism`: genesis state, Msg server and the ZK ISM
//! registered on the hyperlane ISM router ([`msgs`]), SP1 Groth16 proof
//! verification ([`groth16`]).
//!
//! Store layout (collections):
//! - `0x00 || u64(internal id)` → `InterchainSecurityModule`
//! - `0x01 || u64(internal id) || message id` → empty (key set)
//! - `0x02 || u64(internal id)` → bool (`0x01` / `0x00`)
//!
//! ISM ids are hyperlane `HexAddress`es: their gogoproto binary form is the
//! lowercase `0x`-prefixed hex string, and the internal id is bytes 24..32.

pub mod groth16;
pub mod msgs;

use celestia_proto::celestia::zkism::v1::{GenesisState, InterchainSecurityModule};
use prost::Message;
use sdk::address::undefined;
use sdk::codec::u64_key;
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "zkism";
/// Module name.
pub const MODULE_NAME: &str = "zkism";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `IsmsKeyPrefix`.
pub const ISMS_PREFIX: u8 = 0x00;
/// `MessageKeyPrefix`.
pub const MESSAGES_PREFIX: u8 = 0x01;
/// `MessageProofSubmittedPrefix`.
pub const SUBMISSIONS_PREFIX: u8 = 0x02;

/// `ErrIsmNotFound`.
#[must_use]
pub fn err_ism_not_found() -> SdkError {
    SdkError::new(MODULE_NAME, 2, "ism not found")
}

/// A decoded hyperlane `HexAddress` (32 bytes).
pub type HexAddress = [u8; 32];

/// `util.DecodeHexAddress`.
pub fn decode_hex_address(s: &str) -> Result<HexAddress> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        return Err(undefined("invalid hex address length"));
    }
    let b = hex::decode(s).map_err(|e| undefined(e.to_string()))?;
    <HexAddress>::try_from(b.as_slice()).map_err(|_| undefined("invalid hex address length"))
}

/// `HexAddress.String()` / its gogoproto binary form.
#[must_use]
pub fn encode_hex_address(a: &HexAddress) -> String {
    format!("0x{}", hex::encode(a))
}

/// `HexAddress.GetInternalId()`.
#[must_use]
pub fn internal_id(a: &HexAddress) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&a[24..32]);
    u64::from_be_bytes(b)
}

pub(crate) fn ism_key(id: u64) -> Vec<u8> {
    let mut k = vec![ISMS_PREFIX];
    k.extend_from_slice(&u64_key(id));
    k
}

/// `isms.Set` (the id is re-encoded in canonical lowercase form).
pub fn set_ism(ctx: &mut Context<'_>, ism: &InterchainSecurityModule) -> Result<()> {
    let id = decode_hex_address(&ism.id)?;
    let mut ism = ism.clone();
    ism.id = encode_hex_address(&id);
    ctx.set(STORE_KEY, ism_key(internal_id(&id)), ism.encode_to_vec())
}

/// `isms.Has`.
pub fn has_ism(ctx: &mut Context<'_>, internal: u64) -> Result<bool> {
    ctx.has(STORE_KEY, &ism_key(internal))
}

/// `messages.Set`.
pub fn set_message(ctx: &mut Context<'_>, internal: u64, message_id: &[u8]) -> Result<()> {
    let mut k = vec![MESSAGES_PREFIX];
    k.extend_from_slice(&u64_key(internal));
    k.extend_from_slice(message_id);
    ctx.set(STORE_KEY, k, Vec::new())
}

/// `submissions.Set`.
pub fn set_submission(ctx: &mut Context<'_>, internal: u64, submitted: bool) -> Result<()> {
    let mut k = vec![SUBMISSIONS_PREFIX];
    k.extend_from_slice(&u64_key(internal));
    ctx.set(STORE_KEY, k, vec![u8::from(submitted)])
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for ism in &gs.isms {
        set_ism(ctx, ism)?;
    }
    for m in &gs.messages {
        let id = decode_hex_address(&m.id)?;
        if !has_ism(ctx, internal_id(&id))? {
            return Err(err_ism_not_found().wrap(format!(
                "messages defined for unknown ism {}",
                encode_hex_address(&id)
            )));
        }
        for msg in &m.messages {
            let mid = hex::decode(msg.strip_prefix("0x").unwrap_or(msg))
                .map_err(|e| undefined(format!("invalid message id {}: {e}", gostr::quote(msg))))?;
            set_message(ctx, internal_id(&id), &mid)?;
        }
    }
    for s in &gs.submissions {
        let id = decode_hex_address(&s.id)?;
        if !has_ism(ctx, internal_id(&id))? {
            return Err(err_ism_not_found().wrap(format!(
                "message proof submitted entry for unknown ism {}",
                encode_hex_address(&id)
            )));
        }
        set_submission(ctx, internal_id(&id), s.submitted)?;
    }
    Ok(())
}

/// `ExportGenesis` (`keeper/genesis.go`): ISMs in key order, their
/// message ids (sorted `0x…` strings) and proof submissions, both sorted
/// by ISM id.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    use celestia_proto::celestia::zkism::v1::{GenesisMessages, GenesisProofSubmission};
    let mut isms = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[ISMS_PREFIX])?;
    while let Some((_, v)) = it.next(ctx)? {
        isms.push(
            InterchainSecurityModule::decode(v.as_slice()).map_err(|e| undefined(e.to_string()))?,
        );
    }
    let mut messages = Vec::new();
    let v9 = sdk::version::is_v9(ctx.consensus_params.version.as_ref().map_or(0, |v| v.app));
    let mut by_internal = std::collections::BTreeMap::new();
    for ism in &isms {
        let id = decode_hex_address(&ism.id)?;
        let internal = internal_id(&id);
        by_internal.insert(internal, (id, ism.id.clone()));
        let mut prefix = vec![MESSAGES_PREFIX];
        prefix.extend_from_slice(&u64_key(internal));
        let mut msgs = Vec::new();
        let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
        while let Some((k, _)) = it.next(ctx)? {
            // celestia-app v9.0.8 collects them with `CollectionPaginate`
            // and a nil page request: the first `DefaultLimit` (100) in
            // key order.
            if v9 && msgs.len() == 100 {
                break;
            }
            msgs.push(format!("0x{}", hex::encode(&k[prefix.len()..])));
        }
        if msgs.is_empty() {
            continue;
        }
        msgs.sort();
        messages.push((
            id,
            GenesisMessages {
                id: ism.id.clone(),
                messages: msgs,
            },
        ));
    }
    messages.sort_by_key(|a| a.0);
    let mut submissions = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[SUBMISSIONS_PREFIX])?;
    while let Some((k, v)) = it.next(ctx)? {
        let internal =
            sdk::codec::decode_u64(&k[1..]).ok_or_else(|| undefined("invalid submission key"))?;
        let (id, s) = by_internal.get(&internal).cloned().ok_or_else(|| {
            err_ism_not_found().wrap(format!(
                "message proof submitted entry for unknown ism {internal}"
            ))
        })?;
        submissions.push((
            id,
            GenesisProofSubmission {
                id: s,
                submitted: v.first().copied().unwrap_or(0) != 0,
            },
        ));
    }
    submissions.sort_by_key(|a| a.0);
    Ok(GenesisState {
        isms,
        messages: messages.into_iter().map(|(_, m)| m).collect(),
        submissions: submissions.into_iter().map(|(_, s)| s).collect(),
    })
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia::zkism::v1::{GenesisMessages, GenesisProofSubmission};
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[test]
    fn genesis_layout() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(
            &mut ctx,
            &serde_json::json!({"isms": [], "messages": [], "submissions": []}),
        )?;
        let mut id = [0u8; 32];
        id[31] = 7;
        let upper = format!("0x{}", hex::encode_upper(id));
        let gs = GenesisState {
            isms: vec![InterchainSecurityModule {
                id: upper.clone(),
                owner: "o".into(),
                ..Default::default()
            }],
            messages: vec![GenesisMessages {
                id: upper.clone(),
                messages: vec!["0xabcd".into()],
            }],
            submissions: vec![GenesisProofSubmission {
                id: upper,
                submitted: true,
            }],
        };
        init_genesis(&mut ctx, &gs)?;
        let stored = ctx.get(STORE_KEY, &ism_key(7))?.unwrap_or_default();
        let ism = InterchainSecurityModule::decode(stored.as_slice())
            .map_err(|e| undefined(e.to_string()))?;
        assert_eq!(ism.id, encode_hex_address(&id));
        assert!(ctx.has(STORE_KEY, &[1, 0, 0, 0, 0, 0, 0, 0, 7, 0xab, 0xcd])?);
        assert_eq!(
            ctx.get(STORE_KEY, &[2, 0, 0, 0, 0, 0, 0, 0, 7])?,
            Some(vec![1])
        );

        let mut bad = gs.clone();
        bad.isms.clear();
        let mut ms2 = MultiStore::new();
        ms2.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx2 = Context::new(&mut ms2, Header::default(), ExecMode::Finalize);
        assert_eq!(
            init_genesis(&mut ctx2, &bad).map_err(|e| (e.codespace, e.code)),
            Err((MODULE_NAME, 2))
        );
        Ok(())
    }
}
