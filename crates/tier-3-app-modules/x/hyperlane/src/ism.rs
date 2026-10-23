//! Interchain security submodule (`x/core/01_interchain_security`).
//!
//! Store: `0x01 0x00 || u64` ISMs (as `Any`), `0x01 0x02 || u64 ||
//! len-prefixed validator || u64` validator storage locations.

use celestia_proto::Any;
use celestia_proto::hyperlane::core::interchain_security::v1 as pb;
use prost::{Message, Name};
use sdk::codec::{key, length_prefixed, u64_key};
use sdk::{Context, Result};

use crate::core::{self as corek, consume_gas};
use crate::errors::{internal, ism as err};
use crate::events::{JsonValue as J, typed_event};
use crate::router::{Deps, InterchainSecurityModule};
use crate::util::{
    HexAddress, HyperlaneMessage, TREE_DEPTH, acc_address_from_bech32, branch_root, decode_eth_hex,
    encode_eth_hex, eth_signing_hash, keccak256, recover_eth_address,
};
use crate::{ISMS_KEY, STORAGE_LOCATIONS_KEY, STORE_KEY};

/// `INTERCHAIN_SECURITY_MODULE_TYPE_UNUSED` (noop ISM).
pub const TYPE_UNUSED: u8 = 0;
/// `INTERCHAIN_SECURITY_MODULE_TYPE_ROUTING`.
pub const TYPE_ROUTING: u8 = 1;
/// `INTERCHAIN_SECURITY_MODULE_TYPE_MERKLE_ROOT_MULTISIG`.
pub const TYPE_MERKLE_ROOT_MULTISIG: u8 = 4;
/// `INTERCHAIN_SECURITY_MODULE_TYPE_MESSAGE_ID_MULTISIG`.
pub const TYPE_MESSAGE_ID_MULTISIG: u8 = 5;

/// A routing ISM route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Target ISM.
    pub ism: HexAddress,
    /// Origin domain.
    pub domain: u32,
}

/// A stored core ISM (`HyperlaneInterchainSecurityModule`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ism {
    /// `NoopISM`.
    Noop {
        /// Id.
        id: HexAddress,
        /// Owner.
        owner: String,
    },
    /// `MessageIdMultisigISM`.
    MessageIdMultisig {
        /// Id.
        id: HexAddress,
        /// Owner.
        owner: String,
        /// Validator eth addresses (hex).
        validators: Vec<String>,
        /// Threshold.
        threshold: u32,
    },
    /// `MerkleRootMultisigISM`.
    MerkleRootMultisig {
        /// Id.
        id: HexAddress,
        /// Owner.
        owner: String,
        /// Validator eth addresses (hex).
        validators: Vec<String>,
        /// Threshold.
        threshold: u32,
    },
    /// `RoutingISM`.
    Routing {
        /// Id.
        id: HexAddress,
        /// Owner.
        owner: String,
        /// Routes.
        routes: Vec<Route>,
    },
}

fn hex(s: &str) -> std::result::Result<HexAddress, String> {
    HexAddress::from_proto(s)
}

impl Ism {
    /// `GetId`.
    #[must_use]
    pub fn id(&self) -> HexAddress {
        match self {
            Self::Noop { id, .. }
            | Self::MessageIdMultisig { id, .. }
            | Self::MerkleRootMultisig { id, .. }
            | Self::Routing { id, .. } => *id,
        }
    }

    /// `ModuleType`.
    #[must_use]
    pub fn module_type(&self) -> u8 {
        match self {
            Self::Noop { .. } => TYPE_UNUSED,
            Self::MessageIdMultisig { .. } => TYPE_MESSAGE_ID_MULTISIG,
            Self::MerkleRootMultisig { .. } => TYPE_MERKLE_ROOT_MULTISIG,
            Self::Routing { .. } => TYPE_ROUTING,
        }
    }

    /// Packs into `Any` (`codec.CollInterfaceValue`).
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Noop { id, owner } => Any::pack(&pb::NoopIsm {
                id: id.to_string(),
                owner: owner.clone(),
            }),
            Self::MessageIdMultisig {
                id,
                owner,
                validators,
                threshold,
            } => Any::pack(&pb::MessageIdMultisigIsm {
                id: id.to_string(),
                owner: owner.clone(),
                validators: validators.clone(),
                threshold: *threshold,
            }),
            Self::MerkleRootMultisig {
                id,
                owner,
                validators,
                threshold,
            } => Any::pack(&pb::MerkleRootMultisigIsm {
                id: id.to_string(),
                owner: owner.clone(),
                validators: validators.clone(),
                threshold: *threshold,
            }),
            Self::Routing { id, owner, routes } => Any::pack(&pb::RoutingIsm {
                id: id.to_string(),
                owner: owner.clone(),
                routes: routes
                    .iter()
                    .map(|r| pb::Route {
                        ism: r.ism.to_string(),
                        domain: r.domain,
                    })
                    .collect(),
            }),
        }
    }

    /// Unpacks from `Any`.
    pub fn from_any(a: &Any) -> std::result::Result<Self, String> {
        let de = |e: prost::DecodeError| e.to_string();
        let url = a.type_url.as_str();
        if url == pb::NoopIsm::type_url() {
            let m = pb::NoopIsm::decode(a.value.as_slice()).map_err(de)?;
            Ok(Self::Noop {
                id: hex(&m.id)?,
                owner: m.owner,
            })
        } else if url == pb::MessageIdMultisigIsm::type_url() {
            let m = pb::MessageIdMultisigIsm::decode(a.value.as_slice()).map_err(de)?;
            Ok(Self::MessageIdMultisig {
                id: hex(&m.id)?,
                owner: m.owner,
                validators: m.validators,
                threshold: m.threshold,
            })
        } else if url == pb::MerkleRootMultisigIsm::type_url() {
            let m = pb::MerkleRootMultisigIsm::decode(a.value.as_slice()).map_err(de)?;
            Ok(Self::MerkleRootMultisig {
                id: hex(&m.id)?,
                owner: m.owner,
                validators: m.validators,
                threshold: m.threshold,
            })
        } else if url == pb::RoutingIsm::type_url() {
            let m = pb::RoutingIsm::decode(a.value.as_slice()).map_err(de)?;
            Ok(Self::Routing {
                id: hex(&m.id)?,
                owner: m.owner,
                routes: m
                    .routes
                    .iter()
                    .map(|r| {
                        Ok(Route {
                            ism: hex(&r.ism)?,
                            domain: r.domain,
                        })
                    })
                    .collect::<std::result::Result<_, String>>()?,
            })
        } else {
            Err(format!("unsupported type {url}"))
        }
    }

    /// `Verify` of the stored ISM itself (routing is resolved by
    /// [`RoutingIsmHandler`]).
    pub fn verify(
        &self,
        ctx: &mut Context<'_>,
        metadata: &[u8],
        message: &HyperlaneMessage,
    ) -> Result<bool> {
        match self {
            Self::Noop { .. } => Ok(true),
            Self::MessageIdMultisig {
                validators,
                threshold,
                ..
            } => {
                let md = MessageIdMultisigMetadata::parse(metadata).map_err(internal)?;
                let digest = checkpoint_digest(
                    message.origin,
                    &md.merkle_tree_hook,
                    &md.merkle_root,
                    md.merkle_index,
                    &message.id().0,
                );
                charge_signatures(ctx, md.signatures.len())?;
                if crate::is_v9(ctx) {
                    return verify_multisig_v1_1(validators, *threshold, &md.signatures, &digest);
                }
                verify_multisig(validators, *threshold, &md.signatures, &digest)
            }
            Self::MerkleRootMultisig {
                validators,
                threshold,
                ..
            } => {
                let md = MerkleRootMultisigMetadata::parse(metadata).map_err(internal)?;
                if md.message_index > md.signed_index {
                    return Err(internal("invalid signed index"));
                }
                let signed_root = branch_root(message.id().0, &md.merkle_proof, md.message_index);
                let digest = checkpoint_digest(
                    message.origin,
                    &md.merkle_tree_hook,
                    &signed_root,
                    md.signed_index,
                    &md.signed_message_id,
                );
                charge_signatures(ctx, md.signatures.len())?;
                if crate::is_v9(ctx) {
                    return verify_multisig_v1_1(validators, *threshold, &md.signatures, &digest);
                }
                verify_multisig(validators, *threshold, &md.signatures, &digest)
            }
            Self::Routing { .. } => Err(err::unexpected_error(
                "Verify should not be called on RoutingISM",
            )),
        }
    }
}

/// celestia's hyperlane-cosmos v1.3.0 (app version 10) charges 1000 gas per
/// multisig signature; upstream v1.1.0 (app version 9) charges nothing.
fn charge_signatures(ctx: &mut Context<'_>, n: usize) -> Result<()> {
    if crate::is_v9(ctx) {
        return Ok(());
    }
    consume_gas(ctx, 1000 * n as u64, "ism signature verification")
}

fn ism_key(id: u64) -> Vec<u8> {
    key(&[&ISMS_KEY, &u64_key(id)])
}

fn storage_location_prefix(mailbox: u64, validator: &[u8]) -> Vec<u8> {
    key(&[
        &STORAGE_LOCATIONS_KEY,
        &u64_key(mailbox),
        &length_prefixed(validator),
    ])
}

fn storage_location_key(mailbox: u64, validator: &[u8], index: u64) -> Vec<u8> {
    let mut k = storage_location_prefix(mailbox, validator);
    k.extend_from_slice(&u64_key(index));
    k
}

/// `isms.Get` (`None` when absent).
pub fn get_ism(ctx: &mut Context<'_>, internal_id: u64) -> Result<Option<Ism>> {
    ctx.get(STORE_KEY, &ism_key(internal_id))?
        .map(|b| {
            Any::decode(b.as_slice())
                .map_err(|e| e.to_string())
                .and_then(|a| Ism::from_any(&a))
                .map_err(internal)
        })
        .transpose()
}

/// `isms.Set`.
pub fn set_ism(ctx: &mut Context<'_>, internal_id: u64, ism: &Ism) -> Result<()> {
    ctx.set(
        STORE_KEY,
        ism_key(internal_id),
        ism.to_any().encode_to_vec(),
    )
}

fn not_found(id: u64) -> sdk::SdkError {
    internal(format!("collections: not found: key '{id}'"))
}

/// Handler for the noop and multisig ISMs (the ISM keeper itself).
#[derive(Debug, Clone, Copy, Default)]
pub struct CoreIsmHandler;

impl InterchainSecurityModule for CoreIsmHandler {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, ism_id: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &ism_key(ism_id.internal_id()))
    }

    fn verify(
        &self,
        ctx: &mut Context<'_>,
        _: &Deps<'_>,
        ism_id: &HexAddress,
        metadata: &[u8],
        message: &HyperlaneMessage,
    ) -> Result<bool> {
        let ism =
            get_ism(ctx, ism_id.internal_id())?.ok_or_else(|| not_found(ism_id.internal_id()))?;
        ism.verify(ctx, metadata, message)
    }
}

/// `RoutingISMHandler`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoutingIsmHandler;

impl InterchainSecurityModule for RoutingIsmHandler {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, ism_id: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &ism_key(ism_id.internal_id()))
    }

    fn verify(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        ism_id: &HexAddress,
        metadata: &[u8],
        message: &HyperlaneMessage,
    ) -> Result<bool> {
        let route = self
            .route(ctx, ism_id, message)?
            .ok_or_else(|| internal("routing ISM did not return a route"))?;
        corek::verify(ctx, deps, &route, metadata, message)
    }

    fn route(
        &self,
        ctx: &mut Context<'_>,
        ism_id: &HexAddress,
        message: &HyperlaneMessage,
    ) -> Result<Option<HexAddress>> {
        let ism =
            get_ism(ctx, ism_id.internal_id())?.ok_or_else(|| not_found(ism_id.internal_id()))?;
        let Ism::Routing { routes, .. } = ism else {
            return Err(err::invalid_ism_type(format!(
                "ISM {ism_id} is not a routing ISM"
            )));
        };
        let Some(route) = routes.iter().find(|r| r.domain == message.origin) else {
            return Err(err::no_route_found(format!(
                "no route found for domain {}",
                message.origin
            )));
        };
        Ok(Some(route.ism))
    }
}

/// `MessageIdMultisigMetadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageIdMultisigMetadata {
    /// Origin merkle tree hook.
    pub merkle_tree_hook: [u8; 32],
    /// Signed checkpoint root.
    pub merkle_root: [u8; 32],
    /// Signed checkpoint index.
    pub merkle_index: u32,
    /// 65-byte signatures.
    pub signatures: Vec<Vec<u8>>,
}

fn arr32(b: &[u8]) -> [u8; 32] {
    let mut a = [0u8; 32];
    a.copy_from_slice(&b[..32]);
    a
}

fn be32(b: &[u8]) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[..4]);
    u32::from_be_bytes(a)
}

fn split_signatures(metadata: &[u8], offset: usize) -> std::result::Result<Vec<Vec<u8>>, String> {
    if metadata.len() < offset {
        return Err(format!(
            "invalid metadata length: got {}, expected at least {offset} bytes",
            metadata.len()
        ));
    }
    let sigs = &metadata[offset..];
    if !sigs.len().is_multiple_of(65) {
        return Err("invalid signatures length in metadata".into());
    }
    Ok(sigs.chunks(65).map(<[u8]>::to_vec).collect())
}

impl MessageIdMultisigMetadata {
    /// `NewMessageIdMultisigMetadata`.
    pub fn parse(metadata: &[u8]) -> std::result::Result<Self, String> {
        let signatures = split_signatures(metadata, 68)?;
        Ok(Self {
            merkle_tree_hook: arr32(metadata),
            merkle_root: arr32(&metadata[32..]),
            merkle_index: be32(&metadata[64..]),
            signatures,
        })
    }
}

/// `MerkleRootMultisigMetadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleRootMultisigMetadata {
    /// Origin merkle tree hook.
    pub merkle_tree_hook: [u8; 32],
    /// Index of the message in the tree.
    pub message_index: u32,
    /// Merkle proof.
    pub merkle_proof: [[u8; 32]; TREE_DEPTH],
    /// Signed checkpoint index.
    pub signed_index: u32,
    /// Signed checkpoint message id.
    pub signed_message_id: [u8; 32],
    /// 65-byte signatures.
    pub signatures: Vec<Vec<u8>>,
}

impl MerkleRootMultisigMetadata {
    /// `NewMerkleRootMultisigMetadata`.
    pub fn parse(metadata: &[u8]) -> std::result::Result<Self, String> {
        let signatures = split_signatures(metadata, 1096)?;
        let mut merkle_proof = [[0u8; 32]; TREE_DEPTH];
        for (i, p) in merkle_proof.iter_mut().enumerate() {
            *p = arr32(&metadata[68 + i * 32..]);
        }
        Ok(Self {
            merkle_tree_hook: arr32(metadata),
            message_index: be32(&metadata[32..]),
            merkle_proof,
            signed_index: be32(&metadata[1092..]),
            signed_message_id: arr32(&metadata[36..]),
            signatures,
        })
    }
}

fn domain_hash(origin: u32, merkle_tree_hook: &[u8; 32]) -> [u8; 32] {
    keccak256(&[&origin.to_be_bytes(), merkle_tree_hook, b"HYPERLANE"])
}

/// `checkpointDigest`.
#[must_use]
pub fn checkpoint_digest(
    origin: u32,
    merkle_tree_hook: &[u8; 32],
    checkpoint_root: &[u8; 32],
    checkpoint_index: u32,
    message_id: &[u8; 32],
) -> [u8; 32] {
    let h = domain_hash(origin, merkle_tree_hook);
    let inner = keccak256(&[
        &h,
        checkpoint_root,
        &checkpoint_index.to_be_bytes(),
        message_id,
    ]);
    eth_signing_hash(&inner)
}

fn decode_validators(validators: &[String]) -> std::result::Result<Vec<[u8; 20]>, String> {
    let mut out: Vec<[u8; 20]> = Vec::with_capacity(validators.len());
    for v in validators {
        let raw = decode_eth_hex(v).map_err(|_| format!("invalid validator address: {v}"))?;
        if raw.len() != 20 {
            return Err("invalid validator address: must be 20 bytes".into());
        }
        let mut a = [0u8; 20];
        a.copy_from_slice(&raw);
        if out.contains(&a) {
            return Err(format!("duplicate validator address: {v}"));
        }
        out.push(a);
    }
    Ok(out)
}

/// `VerifyMultisig`.
pub fn verify_multisig(
    validators: &[String],
    threshold: u32,
    signatures: &[Vec<u8>],
    digest: &[u8; 32],
) -> Result<bool> {
    if signatures.len() < threshold as usize {
        return Err(internal("threshold can not be reached"));
    }
    let addrs = decode_validators(validators)
        .map_err(|e| internal(format!("invalid multisig validator set: {e}")))?;
    let mut idx = 0;
    for sig in signatures.iter().take(threshold as usize) {
        let signer = recover_eth_address(digest, sig)
            .map_err(|e| internal(format!("failed to recover validator signature: {e}")))?;
        while idx < addrs.len() && signer != addrs[idx] {
            idx += 1;
        }
        if idx >= addrs.len() {
            return Ok(false);
        }
        idx += 1;
    }
    Ok(true)
}

/// `VerifyMultisig` of hyperlane-cosmos v1.1.0 (app version 9): the
/// recovered signer as lower-case `0x` hex against each stored validator
/// string lower-cased (`strings.ToLower`; only an ASCII string can equal
/// the signer), no decoding of the stored set.
pub fn verify_multisig_v1_1(
    validators: &[String],
    threshold: u32,
    signatures: &[Vec<u8>],
    digest: &[u8; 32],
) -> Result<bool> {
    if signatures.len() < threshold as usize {
        return Err(internal("threshold can not be reached"));
    }
    let mut idx = 0;
    for sig in signatures.iter().take(threshold as usize) {
        let signer = recover_eth_address(digest, sig)
            .map_err(|e| internal(format!("failed to recover validator signature: {e}")))?;
        let signer = format!("0x{}", hex::encode(signer));
        while idx < validators.len()
            && !(validators[idx].is_ascii() && validators[idx].to_ascii_lowercase() == signer)
        {
            idx += 1;
        }
        if idx >= validators.len() {
            return Ok(false);
        }
        idx += 1;
    }
    Ok(true)
}

/// `ValidateNewMultisig` of hyperlane-cosmos v1.1.0 (app version 9): the
/// strings sorted byte-wise (`slices.IsSorted`), then each decoded (20
/// bytes) and checked for exact duplicates.
pub fn validate_new_multisig_v1_1(
    validators: &[String],
    threshold: u32,
) -> std::result::Result<(), String> {
    if threshold == 0 {
        return Err("threshold must be greater than zero".into());
    }
    if validators.len() < threshold as usize {
        return Err("validator addresses less than threshold".into());
    }
    if !validators
        .windows(2)
        .all(|w| gostr::to_go_bytes(&w[0]) <= gostr::to_go_bytes(&w[1]))
    {
        return Err("validator addresses are not sorted correctly in ascending order".into());
    }
    let mut seen: Vec<&str> = Vec::with_capacity(validators.len());
    for v in validators {
        let raw = decode_eth_hex(v).map_err(|_| format!("invalid validator address: {v}"))?;
        if raw.len() != 20 {
            return Err("invalid validator address: must be 20 bytes".into());
        }
        if seen.contains(&v.as_str()) {
            return Err(format!("duplicate validator address: {v}"));
        }
        seen.push(v);
    }
    Ok(())
}

/// `ValidateNewMultisig`.
pub fn validate_new_multisig(
    validators: &[String],
    threshold: u32,
) -> std::result::Result<(), String> {
    if threshold == 0 {
        return Err("threshold must be greater than zero".into());
    }
    if validators.len() < threshold as usize {
        return Err("validator addresses less than threshold".into());
    }
    let addrs = decode_validators(validators)?;
    if !addrs.windows(2).all(|w| w[0] <= w[1]) {
        return Err("validator addresses are not sorted correctly in ascending order".into());
    }
    Ok(())
}

fn next_ism_id(ctx: &mut Context<'_>, deps: &Deps<'_>, ty: u8) -> Result<HexAddress> {
    deps.isms
        .next_id(ctx, ty)
        .map_err(|e| err::unexpected_error(e.log))
}

/// `Keeper.CreateNoopIsm`.
pub fn create_noop_ism(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    creator: &str,
) -> Result<HexAddress> {
    let id = next_ism_id(ctx, deps, TYPE_UNUSED)?;
    let ism = Ism::Noop {
        id,
        owner: creator.to_string(),
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    ctx.events.emit(typed_event::<pb::EventCreateNoopIsm>(vec![
        ("ism_id", J::addr(&id)),
        ("owner", J::Str(creator.to_string())),
    ]));
    Ok(id)
}

/// `Keeper.CreateMessageIdMultisigIsm` / `CreateMerkleRootMultisigIsm`.
pub fn create_multisig_ism(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    merkle_root: bool,
    creator: &str,
    validators: Vec<String>,
    threshold: u32,
) -> Result<HexAddress> {
    let ty = if merkle_root {
        TYPE_MERKLE_ROOT_MULTISIG
    } else {
        TYPE_MESSAGE_ID_MULTISIG
    };
    let id = next_ism_id(ctx, deps, ty)?;
    if crate::is_v9(ctx) {
        validate_new_multisig_v1_1(&validators, threshold)
    } else {
        validate_new_multisig(&validators, threshold)
    }
    .map_err(err::invalid_multisig_configuration)?;
    let ev_validators = validators.clone();
    let ism = if merkle_root {
        Ism::MerkleRootMultisig {
            id,
            owner: creator.to_string(),
            validators,
            threshold,
        }
    } else {
        Ism::MessageIdMultisig {
            id,
            owner: creator.to_string(),
            validators,
            threshold,
        }
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    let fields = vec![
        ("ism_id", J::addr(&id)),
        ("owner", J::Str(creator.to_string())),
        ("validators", J::StrList(ev_validators)),
        ("threshold", J::Num(u64::from(threshold))),
    ];
    ctx.events.emit(if merkle_root {
        typed_event::<pb::EventCreateMerkleRootMultisigIsm>(fields)
    } else {
        typed_event::<pb::EventCreateMessageIdMultisigIsm>(fields)
    });
    Ok(id)
}

/// `Keeper.CreateRoutingIsm`.
pub fn create_routing_ism(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    creator: &str,
    routes: Vec<Route>,
) -> Result<HexAddress> {
    let id = next_ism_id(ctx, deps, TYPE_ROUTING)?;
    let mut seen = std::collections::BTreeSet::new();
    for r in &routes {
        if !seen.insert(r.domain) {
            return Err(err::duplicated_domains(format!(
                "multiple ISMs for domain {} not allowed",
                r.domain
            )));
        }
        if !matches!(corek::ism_exists(ctx, deps, &r.ism), Ok(true)) {
            return Err(err::unknown_ism_id(format!("ISM {} not found", r.ism)));
        }
    }
    let ism = Ism::Routing {
        id,
        owner: creator.to_string(),
        routes: routes.clone(),
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    ctx.events
        .emit(typed_event::<pb::EventCreateRoutingIsm>(vec![
            ("ism_id", J::addr(&id)),
            ("owner", J::Str(creator.to_string())),
        ]));
    for r in &routes {
        ctx.events
            .emit(typed_event::<pb::EventSetRoutingIsmDomain>(vec![
                ("owner", J::Str(creator.to_string())),
                ("ism_id", J::addr(&id)),
                ("route_ism_id", J::addr(&r.ism)),
                ("route_domain", J::Num(u64::from(r.domain))),
            ]));
    }
    Ok(id)
}

/// `getRoutingIsm`: loads a routing ISM owned by `owner`.
fn get_routing_ism(
    ctx: &mut Context<'_>,
    ism_id: &HexAddress,
    owner: &str,
) -> Result<(HexAddress, String, Vec<Route>)> {
    let Some(ism) = get_ism(ctx, ism_id.internal_id())? else {
        return Err(err::unknown_ism_id(format!("ISM {ism_id} not found")));
    };
    let Ism::Routing {
        id,
        owner: o,
        routes,
    } = ism
    else {
        return Err(err::invalid_ism_type(format!(
            "ISM {ism_id} is not a routing ISM"
        )));
    };
    if o != owner {
        return Err(err::unauthorized(format!(
            "owner {owner} is not the owner of the ism {id}"
        )));
    }
    Ok((id, o, routes))
}

/// `Keeper.UpdateRoutingIsmOwner`.
pub fn update_routing_ism_owner(
    ctx: &mut Context<'_>,
    ism_id: &HexAddress,
    owner: &str,
    new_owner: &str,
    renounce_ownership: bool,
) -> Result<()> {
    let (id, _, routes) = get_routing_ism(ctx, ism_id, owner)?;
    if !new_owner.is_empty() && acc_address_from_bech32(new_owner).is_err() {
        return Err(err::invalid_owner("invalid new owner"));
    }
    if renounce_ownership && !new_owner.is_empty() {
        return Err(err::invalid_owner(
            "cannot set new owner and renounce ownership at the same time",
        ));
    }
    if !renounce_ownership && new_owner.is_empty() {
        return Err(err::invalid_owner(
            "cannot set owner to empty address without renouncing ownership",
        ));
    }
    let ism = Ism::Routing {
        id,
        owner: new_owner.to_string(),
        routes,
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    ctx.events.emit(typed_event::<pb::EventSetRoutingIsm>(vec![
        ("owner", J::Str(owner.to_string())),
        ("ism_id", J::addr(&id)),
        ("new_owner", J::Str(new_owner.to_string())),
        ("renounce_ownership", J::Bool(renounce_ownership)),
    ]));
    Ok(())
}

/// `Keeper.RemoveRoutingIsmDomain`.
pub fn remove_routing_ism_domain(
    ctx: &mut Context<'_>,
    ism_id: &HexAddress,
    owner: &str,
    domain: u32,
) -> Result<()> {
    let (id, o, mut routes) = get_routing_ism(ctx, ism_id, owner)?;
    if let Some(pos) = routes.iter().position(|r| r.domain == domain) {
        routes.remove(pos);
    }
    let ism = Ism::Routing {
        id,
        owner: o,
        routes,
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    ctx.events
        .emit(typed_event::<pb::EventRemoveRoutingIsmDomain>(vec![
            ("owner", J::Str(owner.to_string())),
            ("ism_id", J::addr(ism_id)),
            ("route_domain", J::Num(u64::from(domain))),
        ]));
    Ok(())
}

/// `Keeper.SetRoutingIsmDomain`.
pub fn set_routing_ism_domain(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    ism_id: &HexAddress,
    owner: &str,
    route: Route,
) -> Result<()> {
    let (id, o, mut routes) = get_routing_ism(ctx, ism_id, owner)?;
    if !matches!(corek::ism_exists(ctx, deps, &route.ism), Ok(true)) {
        return Err(err::unknown_ism_id(format!("ISM {} not found", route.ism)));
    }
    if let Some(r) = routes.iter_mut().find(|r| r.domain == route.domain) {
        // hyperlane-cosmos v1.1.0 (app version 9) `SetDomain` assigns to
        // the loop's copy of the route: an existing domain keeps its ISM.
        if !crate::is_v9(ctx) {
            r.ism = route.ism;
        }
    } else {
        routes.push(route.clone());
    }
    let ism = Ism::Routing {
        id,
        owner: o,
        routes,
    };
    set_ism(ctx, id.internal_id(), &ism).map_err(|e| err::unexpected_error(e.log))?;
    ctx.events
        .emit(typed_event::<pb::EventSetRoutingIsmDomain>(vec![
            ("owner", J::Str(owner.to_string())),
            ("ism_id", J::addr(ism_id)),
            ("route_ism_id", J::addr(&route.ism)),
            ("route_domain", J::Num(u64::from(route.domain))),
        ]));
    Ok(())
}

/// `GetAnnouncementDigest`.
#[must_use]
pub fn announcement_digest(storage_location: &str, domain: u32, mailbox: &[u8]) -> [u8; 32] {
    let domain_hash = keccak256(&[&domain.to_be_bytes(), mailbox, b"HYPERLANE_ANNOUNCEMENT"]);
    keccak256(&[&domain_hash, storage_location.as_bytes()])
}

/// Storage locations of `validator` for `mailbox`, in index order.
pub fn storage_locations(
    ctx: &mut Context<'_>,
    mailbox: u64,
    validator: &[u8],
) -> Result<Vec<String>> {
    let prefix = storage_location_prefix(mailbox, validator);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    let mut out = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        out.push(String::from_utf8(v).map_err(|e| err::unexpected_error(e.to_string()))?);
    }
    Ok(out)
}

/// Writes a storage location (`storageLocations.Set`).
pub fn set_storage_location(
    ctx: &mut Context<'_>,
    mailbox: u64,
    validator: &[u8],
    index: u64,
    location: &str,
) -> Result<()> {
    ctx.set(
        STORE_KEY,
        storage_location_key(mailbox, validator, index),
        location.as_bytes().to_vec(),
    )
}

/// `msgServer.AnnounceValidator`.
pub fn announce_validator(
    ctx: &mut Context<'_>,
    validator: &str,
    storage_location: &str,
    signature: &str,
    mailbox_id: &HexAddress,
    creator: &str,
) -> Result<()> {
    if validator.is_empty() {
        return Err(err::invalid_announce("validator cannot be empty"));
    }
    if storage_location.is_empty() {
        return Err(err::invalid_announce("storage location cannot be empty"));
    }
    if signature.is_empty() {
        return Err(err::invalid_announce("signature cannot be empty"));
    }
    let sig = decode_eth_hex(signature).map_err(|_| err::invalid_announce("invalid signature"))?;
    if !matches!(corek::mailbox_id_exists(ctx, mailbox_id), Ok(true)) {
        return Err(err::mailbox_does_not_exist(format!(
            "failed to find mailbox with id: {mailbox_id}"
        )));
    }
    let local_domain =
        corek::local_domain(ctx, mailbox_id).map_err(|e| err::unexpected_error(e.log))?;
    let digest = announcement_digest(storage_location, local_domain, mailbox_id.bytes());
    let signing_hash = eth_signing_hash(&digest);
    let recovered = recover_eth_address(&signing_hash, &sig).map_err(err::invalid_signature)?;
    let validator_address = decode_eth_hex(validator)
        .map_err(|_| err::invalid_announce("invalid validator address"))?;
    if recovered.as_slice() != validator_address.as_slice() {
        return Err(err::invalid_signature(format!(
            "validator {} doesn't match signature. recovered address: {}",
            encode_eth_hex(&validator_address),
            encode_eth_hex(&recovered)
        )));
    }
    let mailbox = mailbox_id.internal_id();
    let exists = ctx.has(
        STORE_KEY,
        &storage_location_key(mailbox, &validator_address, 0),
    )?;
    let mut index = 0u64;
    if exists {
        let locations = storage_locations(ctx, mailbox, &validator_address)?;
        if locations.iter().any(|l| l == storage_location) {
            return Err(err::invalid_announce(format!(
                "validator {validator} already announced storage location {storage_location}"
            )));
        }
        index = locations.len() as u64;
    }
    set_storage_location(ctx, mailbox, &validator_address, index, storage_location)?;
    ctx.events
        .emit(typed_event::<pb::EventAnnounceStorageLocation>(vec![
            ("sender", J::Str(creator.to_string())),
            ("validator", J::Str(encode_eth_hex(&validator_address))),
            ("mailbox_id", J::addr(mailbox_id)),
            ("storage_location", J::Str(storage_location.to_string())),
        ]));
    Ok(())
}

/// Parses a proto route.
pub fn route_from_proto(r: &pb::Route) -> std::result::Result<Route, String> {
    Ok(Route {
        ism: hex(&r.ism)?,
        domain: r.domain,
    })
}

/// `InitGenesis` of the ISM submodule.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &pb::GenesisState) -> Result<()> {
    let isms = gs
        .isms
        .iter()
        .map(Ism::from_any)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(internal)?;
    for ism in &isms {
        set_ism(ctx, ism.id().internal_id(), ism)?;
    }
    for loc in &gs.validator_storage_locations {
        let validator = decode_eth_hex(&loc.validator_address).map_err(internal)?;
        set_storage_location(
            ctx,
            loc.mailbox_id,
            &validator,
            loc.index,
            &loc.storage_location,
        )?;
    }
    Ok(())
}

/// `ExportGenesis` of the ISM submodule.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let mut isms = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &ISMS_KEY)?;
    while let Some((_, v)) = it.next(ctx)? {
        let a = Any::decode(v.as_slice()).map_err(|e| internal(e.to_string()))?;
        // Re-pack so the export is canonical.
        isms.push(Ism::from_any(&a).map_err(internal)?.to_any());
    }
    let mut locations = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &STORAGE_LOCATIONS_KEY)?;
    while let Some((k, v)) = it.next(ctx)? {
        let rest = &k[STORAGE_LOCATIONS_KEY.len()..];
        let bad = || internal("invalid storage location key");
        let mailbox = sdk::codec::decode_u64(rest.get(..8).ok_or_else(bad)?).ok_or_else(bad)?;
        let vlen = usize::from(*rest.get(8).ok_or_else(bad)?);
        let validator = rest.get(9..9 + vlen).ok_or_else(bad)?;
        let index =
            sdk::codec::decode_u64(rest.get(9 + vlen..).ok_or_else(bad)?).ok_or_else(bad)?;
        locations.push(pb::GenesisValidatorStorageLocationWrapper {
            mailbox_id: mailbox,
            validator_address: encode_eth_hex(validator),
            index,
            storage_location: String::from_utf8(v).map_err(|e| internal(e.to_string()))?,
        });
    }
    Ok(pb::GenesisState {
        isms,
        validator_storage_locations: locations,
    })
}
