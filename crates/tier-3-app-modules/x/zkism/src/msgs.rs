//! x/zkism Msg server (`keeper/msg_server.go`), `ValidateBasic`
//! (`types/msgs.go`, `types/keys.go`), public values (`types/public_values.go`)
//! and the ISM registered on the hyperlane ISM router (`keeper/keeper.go`).

use celestia_proto::celestia::zkism::v1::{
    EventCreateInterchainSecurityModule, EventSubmitMessages, EventUpdateInterchainSecurityModule,
    InterchainSecurityModule, MsgCreateInterchainSecurityModule,
    MsgCreateInterchainSecurityModuleResponse, MsgSubmitMessages, MsgSubmitMessagesResponse,
    MsgUpdateInterchainSecurityModule, MsgUpdateInterchainSecurityModuleResponse,
};
use prost::Message;
use sdk::error::internal;
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};
use x_hyperlane::{Deps, HexAddress, HyperlaneMessage};

use crate::groth16::{self, PROOF_BYTES_LEN, VerifyError};
use crate::{
    MESSAGES_PREFIX, MODULE_NAME, STORE_KEY, encode_hex_address, ism_key, set_ism, set_message,
    set_submission,
};

/// `ModuleTypeZkISM`.
pub const MODULE_TYPE_ZK_ISM: u8 = 42;
/// `MinStateBytes`.
pub const MIN_STATE_BYTES: usize = 32;
/// `MaxStateBytes`.
pub const MAX_STATE_BYTES: usize = 2048;
/// `MaxStateTransitionValuesBytes`.
pub const MAX_STATE_TRANSITION_VALUES_BYTES: usize = 2 * 8 + 2 * MAX_STATE_BYTES;
/// `MaxMessageIdsCount`.
pub const MAX_MESSAGE_IDS_COUNT: u64 = 1_000_000;
/// `MaxStateMembershipValuesBytes`.
pub const MAX_STATE_MEMBERSHIP_VALUES_BYTES: usize = 32 + 32 + 8 + 1_000_000 * 32;
/// `Groth16VkeySize`.
pub const GROTH16_VKEY_SIZE: usize = 492;
/// `DefaultProofVerifyCostGroth16`.
pub const PROOF_VERIFY_COST_GROTH16: u64 = 6000;

const VKEY_POINT_OFFSETS: [usize; 6] = [0, 32, 64, 128, 192, 224];
const VKEY_CURVE_POINTS_SIZE: usize = 288;
const VKEY_G1K_LENGTH: u32 = 6;
const VKEY_COMMITMENT_KEYS_OFFSET: usize = VKEY_CURVE_POINTS_SIZE + 4 + 6 * 32;
const VKEY_PUB_COMMITTED_OFFSET: usize = VKEY_COMMITMENT_KEYS_OFFSET + 4;

fn err(code: u32, desc: &str, log: impl AsRef<str>) -> SdkError {
    let log = log.as_ref();
    SdkError::new(
        MODULE_NAME,
        code,
        if log.is_empty() {
            desc.to_owned()
        } else {
            format!("{log}: {desc}")
        },
    )
}

/// `ErrIsmNotFound`.
fn ism_not_found(log: impl AsRef<str>) -> SdkError {
    err(2, "ism not found", log)
}
/// `ErrInvalidVerifyingKey`.
fn invalid_verifying_key(log: impl AsRef<str>) -> SdkError {
    err(3, "invalid verifying key", log)
}
/// `ErrInvalidProof`.
fn invalid_proof(log: impl AsRef<str>) -> SdkError {
    err(4, "invalid proof", log)
}
/// `ErrInvalidProofLength`.
fn invalid_proof_length(log: impl AsRef<str>) -> SdkError {
    err(5, "invalid proof length", log)
}
/// `ErrInvalidProofPrefix`.
fn invalid_proof_prefix(log: impl AsRef<str>) -> SdkError {
    err(6, "invalid proof prefix", log)
}
/// `ErrInvalidTrustedState`.
fn invalid_trusted_state(log: impl AsRef<str>) -> SdkError {
    err(7, "invalid trusted state", log)
}
/// `ErrInvalidStateRoot`.
fn invalid_state_root(log: impl AsRef<str>) -> SdkError {
    err(8, "invalid state root", log)
}
/// `ErrInvalidMerkleTreeAddress`.
fn invalid_merkle_tree_address(log: impl AsRef<str>) -> SdkError {
    err(9, "invalid merkle tree address", log)
}
/// `ErrMessageProofAlreadySubmitted`.
fn message_proof_already_submitted() -> SdkError {
    err(
        10,
        "message proof already submitted for current state root",
        "",
    )
}
/// `ErrInvalidPublicValuesLength`.
fn invalid_public_values_length(log: impl AsRef<str>) -> SdkError {
    err(11, "invalid public values length", log)
}

/// `EncodeHex`.
#[must_use]
pub fn encode_hex(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

/// celestia-app v9.0.8 `ValidateGroth16Vkey` (SP1 v5: 2 public inputs,
/// G1.K of 3 points, 396 bytes, no compressed-point check).
fn validate_groth16_vkey_v9(v: &[u8]) -> core::result::Result<(), String> {
    const SIZE: usize = 396;
    const G1K: u32 = 3;
    const CK_OFFSET: usize = VKEY_CURVE_POINTS_SIZE + 4 + 3 * 32;
    if v.len() != SIZE {
        return Err(format!(
            "groth16 vkey must be exactly {SIZE} bytes, got {}",
            v.len()
        ));
    }
    let be = |o: usize| u32::from_be_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
    let g1k = be(VKEY_CURVE_POINTS_SIZE);
    if g1k != G1K {
        return Err(format!("groth16 vkey G1.K length must be {G1K}, got {g1k}"));
    }
    let ck = be(CK_OFFSET);
    if ck != 0 {
        return Err(format!(
            "groth16 vkey CommitmentKeys length must be 0, got {ck}"
        ));
    }
    let pc = be(CK_OFFSET + 4);
    if pc != 0 {
        return Err(format!(
            "groth16 vkey PublicAndCommitmentCommitted length must be 0, got {pc}"
        ));
    }
    Ok(())
}

/// Whether the executing app version is 9 (SP1 v5 proofs, ADR 0004).
fn sp1_v5() -> bool {
    sdk::version::is_v9(sdk::version::app_version())
}

/// `ValidateGroth16Vkey`.
pub fn validate_groth16_vkey(v: &[u8]) -> core::result::Result<(), String> {
    if sp1_v5() {
        return validate_groth16_vkey_v9(v);
    }
    if v.len() != GROTH16_VKEY_SIZE {
        return Err(format!(
            "groth16 vkey must be exactly {GROTH16_VKEY_SIZE} bytes, got {}",
            v.len()
        ));
    }
    for off in VKEY_POINT_OFFSETS {
        if v[off] & (0b11 << 6) == 0 {
            return Err(format!(
                "groth16 vkey curve point at offset {off} must be compressed"
            ));
        }
    }
    let be = |o: usize| u32::from_be_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
    let g1k = be(VKEY_CURVE_POINTS_SIZE);
    if g1k != VKEY_G1K_LENGTH {
        return Err(format!(
            "groth16 vkey G1.K length must be {VKEY_G1K_LENGTH}, got {g1k}"
        ));
    }
    let ck = be(VKEY_COMMITMENT_KEYS_OFFSET);
    if ck != 0 {
        return Err(format!(
            "groth16 vkey CommitmentKeys length must be 0, got {ck}"
        ));
    }
    let pc = be(VKEY_PUB_COMMITTED_OFFSET);
    if pc != 0 {
        return Err(format!(
            "groth16 vkey PublicAndCommitmentCommitted length must be 0, got {pc}"
        ));
    }
    Ok(())
}

fn decode_id(s: &str) -> Result<HexAddress> {
    HexAddress::decode(s).map_err(errs::tx_decode)
}

/// `MsgCreateInterchainSecurityModule.ValidateBasic`.
pub fn validate_basic_create(m: &MsgCreateInterchainSecurityModule) -> Result<()> {
    if m.state.len() < MIN_STATE_BYTES {
        return Err(invalid_trusted_state(format!(
            "initial trusted state must be at least {MIN_STATE_BYTES} bytes"
        )));
    }
    if m.state.len() > MAX_STATE_BYTES {
        return Err(invalid_trusted_state(format!(
            "initial trusted state must be no greater than {MAX_STATE_BYTES} bytes"
        )));
    }
    if m.merkle_tree_address.len() != 32 {
        return Err(invalid_merkle_tree_address(
            "merkle tree address must be 32 bytes",
        ));
    }
    validate_groth16_vkey(&m.groth16_vkey).map_err(invalid_verifying_key)?;
    groth16::new_verifying_key(&m.groth16_vkey)
        .map_err(|_| invalid_verifying_key("invalid groth16 verifying key"))?;
    if m.state_transition_vkey.len() != 32 || m.state_membership_vkey.len() != 32 {
        return Err(invalid_verifying_key(
            "program verifying key commitment must be exactly 32 bytes",
        ));
    }
    Ok(())
}

fn validate_basic_proof(id: &str, proof: &[u8], values: &[u8], max_values: usize) -> Result<()> {
    if decode_id(id)?.is_zero() {
        return Err(errs::invalid_request("ism identifier must be non-zero"));
    }
    // v9 (SP1 v5): prefix + proof, no metadata.
    let want = if sp1_v5() {
        groth16::PREFIX_LEN + groth16::PROOF_SIZE
    } else {
        PROOF_BYTES_LEN
    };
    if proof.len() != want {
        return Err(invalid_proof_length(format!(
            "expected {want}, got {}",
            proof.len()
        )));
    }
    if values.len() > max_values {
        return Err(invalid_public_values_length(format!(
            "public values must not exceed {max_values} bytes"
        )));
    }
    Ok(())
}

/// `MsgUpdateInterchainSecurityModule.ValidateBasic`.
pub fn validate_basic_update(m: &MsgUpdateInterchainSecurityModule) -> Result<()> {
    validate_basic_proof(
        &m.id,
        &m.proof,
        &m.public_values,
        MAX_STATE_TRANSITION_VALUES_BYTES,
    )
}

/// `MsgSubmitMessages.ValidateBasic`.
pub fn validate_basic_submit(m: &MsgSubmitMessages) -> Result<()> {
    validate_basic_proof(
        &m.id,
        &m.proof,
        &m.public_values,
        MAX_STATE_MEMBERSHIP_VALUES_BYTES,
    )
}

/// `StateTransitionValues`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionValues {
    /// Trusted state the proof starts from.
    pub state: Vec<u8>,
    /// New trusted state.
    pub new_state: Vec<u8>,
}

fn read_u64_le(buf: &mut &[u8]) -> core::result::Result<u64, String> {
    if buf.len() < 8 {
        return Err(if buf.is_empty() {
            "EOF".into()
        } else {
            "unexpected EOF".into()
        });
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[..8]);
    *buf = &buf[8..];
    Ok(u64::from_le_bytes(b))
}

fn read_exact<'a>(buf: &mut &'a [u8], n: usize) -> core::result::Result<&'a [u8], String> {
    if buf.len() < n {
        return Err(if buf.is_empty() && n > 0 {
            "EOF".into()
        } else {
            "unexpected EOF".into()
        });
    }
    let (a, b) = buf.split_at(n);
    *buf = b;
    Ok(a)
}

/// `StateTransitionValues.Unmarshal` (trailing bytes are ignored, as in Go).
pub fn unmarshal_state_transition(
    data: &[u8],
) -> core::result::Result<StateTransitionValues, String> {
    let mut buf = data;
    let read_state = |buf: &mut &[u8]| -> core::result::Result<Vec<u8>, String> {
        let l = read_u64_le(buf)?;
        if l < MIN_STATE_BYTES as u64 || l > MAX_STATE_BYTES as u64 {
            return Err(format!(
                "invalid state length, must be between {MIN_STATE_BYTES} and {MAX_STATE_BYTES}"
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(read_exact(buf, l as usize)?.to_vec())
    };
    let state = read_state(&mut buf)?;
    let new_state = read_state(&mut buf)?;
    Ok(StateTransitionValues { state, new_state })
}

/// `StateMembershipValues`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMembershipValues {
    /// State root the messages are proven against.
    pub state_root: [u8; 32],
    /// Hyperlane merkle tree hook address.
    pub merkle_tree_address: [u8; 32],
    /// Authorized message ids.
    pub message_ids: Vec<[u8; 32]>,
}

/// `StateMembershipValues.Unmarshal`.
pub fn unmarshal_state_membership(
    data: &[u8],
) -> core::result::Result<StateMembershipValues, String> {
    let mut buf = data;
    let mut state_root = [0u8; 32];
    state_root
        .copy_from_slice(read_exact(&mut buf, 32).map_err(|e| format!("read StateRoot: {e}"))?);
    let mut merkle_tree_address = [0u8; 32];
    merkle_tree_address.copy_from_slice(
        read_exact(&mut buf, 32).map_err(|e| format!("read MerkleTreeAddress: {e}"))?,
    );
    let count = read_u64_le(&mut buf).map_err(|e| format!("read message ids length: {e}"))?;
    if count > MAX_MESSAGE_IDS_COUNT {
        return Err(format!(
            "message ids count {count} exceeds maximum allowed {MAX_MESSAGE_IDS_COUNT}"
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    if buf.len() as u64 != count * 32 {
        return Err(format!(
            "buffer length mismatch: need exactly {}, have {}",
            count * 32,
            buf.len()
        ));
    }
    let message_ids = buf
        .chunks(32)
        .map(|c| {
            let mut id = [0u8; 32];
            id.copy_from_slice(c);
            id
        })
        .collect();
    Ok(StateMembershipValues {
        state_root,
        merkle_tree_address,
        message_ids,
    })
}

/// `isms.Get`.
pub fn get_ism(ctx: &mut Context<'_>, internal: u64) -> Result<Option<InterchainSecurityModule>> {
    ctx.get(STORE_KEY, &ism_key(internal))?
        .map(|b| InterchainSecurityModule::decode(b.as_slice()).map_err(|e| internal_err(&e)))
        .transpose()
}

fn internal_err(e: &dyn std::fmt::Display) -> SdkError {
    internal(e.to_string())
}

fn emit<T: prost::Name + serde::Serialize>(ctx: &mut Context<'_>, ev: &T) -> Result<()> {
    let e = sdk::event::typed_event(ev).map_err(|e| errs::json_marshal(e.to_string()))?;
    ctx.events.emit(e);
    Ok(())
}

fn verify_proof(
    ctx: &mut Context<'_>,
    ism: &InterchainSecurityModule,
    proof: &[u8],
    program_vk: &[u8],
    values: &[u8],
) -> Result<()> {
    let vk =
        groth16::new_verifying_key(&ism.groth16_vkey).map_err(|_| invalid_verifying_key(""))?;
    ctx.gas
        .consume(PROOF_VERIFY_COST_GROTH16, "groth16 verify")
        .map_err(|_| errs::out_of_gas(""))?;
    let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    let verify = if sdk::version::is_v9(app_version) {
        groth16::sp1_verify_v5
    } else {
        groth16::sp1_verify
    };
    verify(&vk, &ism.groth16_vkey, proof, program_vk, values).map_err(|e| match e {
        VerifyError::Length(l) => invalid_proof_length(l),
        VerifyError::Prefix(l) => invalid_proof_prefix(l),
        VerifyError::Invalid(l) => invalid_proof(l),
        VerifyError::Decode(l) => internal(l),
    })
}

/// `CreateInterchainSecurityModule`.
pub fn create_ism(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    m: &MsgCreateInterchainSecurityModule,
) -> Result<MsgCreateInterchainSecurityModuleResponse> {
    let id = deps.isms.next_id(ctx, MODULE_TYPE_ZK_ISM)?;
    let ism = InterchainSecurityModule {
        id: id.to_string(),
        owner: m.creator.clone(),
        state: m.state.clone(),
        merkle_tree_address: m.merkle_tree_address.clone(),
        groth16_vkey: m.groth16_vkey.clone(),
        state_transition_vkey: m.state_transition_vkey.clone(),
        state_membership_vkey: m.state_membership_vkey.clone(),
    };
    set_ism(ctx, &ism)?;
    emit(
        ctx,
        &EventCreateInterchainSecurityModule {
            id: id.to_string(),
            owner: ism.owner.clone(),
            state: encode_hex(&ism.state),
            merkle_tree_address: encode_hex(&ism.merkle_tree_address),
            groth16_vkey: encode_hex(&ism.groth16_vkey),
            state_transition_vkey: encode_hex(&ism.state_transition_vkey),
            state_membership_vkey: encode_hex(&ism.state_membership_vkey),
        },
    )?;
    Ok(MsgCreateInterchainSecurityModuleResponse { id: id.to_string() })
}

/// `UpdateInterchainSecurityModule`.
pub fn update_ism(
    ctx: &mut Context<'_>,
    m: &MsgUpdateInterchainSecurityModule,
) -> Result<MsgUpdateInterchainSecurityModuleResponse> {
    let id = decode_id(&m.id)?;
    let Some(mut ism) = get_ism(ctx, id.internal_id())? else {
        return Err(ism_not_found(format!("failed to get ism: {id}")));
    };
    let values = unmarshal_state_transition(&m.public_values).map_err(errs::invalid_type)?;
    if values.state.len() < 32 || values.new_state.len() < 32 {
        return Err(invalid_trusted_state("state must be at least 32 bytes"));
    }
    if ism.state != values.state {
        return Err(invalid_trusted_state(format!(
            "expected {}, got {}",
            hex::encode(&ism.state),
            hex::encode(&values.state)
        )));
    }
    let program_vk = ism.state_transition_vkey.clone();
    verify_proof(ctx, &ism, &m.proof, &program_vk, &m.public_values)?;
    let root_changed = ism.state.get(..32) != values.new_state.get(..32);
    ism.state = values.new_state;
    set_ism(ctx, &ism)?;
    let internal = decode_id(&ism.id)?.internal_id();
    if root_changed {
        set_submission(ctx, internal, false)?;
    }
    emit(
        ctx,
        &EventUpdateInterchainSecurityModule {
            id: encode_hex_address(&decode_id(&ism.id)?.0),
            state: encode_hex(&ism.state),
        },
    )?;
    Ok(MsgUpdateInterchainSecurityModuleResponse { state: ism.state })
}

fn submission_key(internal: u64) -> Vec<u8> {
    let mut k = vec![crate::SUBMISSIONS_PREFIX];
    k.extend_from_slice(&sdk::codec::u64_key(internal));
    k
}

/// `SubmitMessages`.
pub fn submit_messages(
    ctx: &mut Context<'_>,
    m: &MsgSubmitMessages,
) -> Result<MsgSubmitMessagesResponse> {
    let id = decode_id(&m.id)?;
    let Some(ism) = get_ism(ctx, id.internal_id())? else {
        return Err(ism_not_found(format!("failed to get ism: {id}")));
    };
    let values = unmarshal_state_membership(&m.public_values).map_err(errs::invalid_type)?;
    let root = ism.state.get(..32).unwrap_or(&ism.state);
    if values.state_root[..] != *root {
        return Err(invalid_state_root(format!(
            "expected {}, got {}",
            hex::encode(root),
            hex::encode(values.state_root)
        )));
    }
    let internal = decode_id(&ism.id)?.internal_id();
    // `submissions.Get`: a missing or undecodable entry reads as false.
    let submitted = ctx
        .get(STORE_KEY, &submission_key(internal))?
        .is_some_and(|b| b == [1]);
    if submitted {
        return Err(message_proof_already_submitted());
    }
    if values.merkle_tree_address[..] != ism.merkle_tree_address[..] {
        return Err(invalid_merkle_tree_address(format!(
            "expected {}, got {}",
            hex::encode(&ism.merkle_tree_address),
            hex::encode(values.merkle_tree_address)
        )));
    }
    let program_vk = ism.state_membership_vkey.clone();
    verify_proof(ctx, &ism, &m.proof, &program_vk, &m.public_values)?;
    let mut messages = Vec::with_capacity(values.message_ids.len());
    for mid in &values.message_ids {
        set_message(ctx, internal, mid)?;
        messages.push(encode_hex(mid));
    }
    set_submission(ctx, internal, true)?;
    emit(
        ctx,
        &EventSubmitMessages {
            id: encode_hex_address(&decode_id(&ism.id)?.0),
            state_root: encode_hex(root),
            messages: messages.clone(),
        },
    )?;
    Ok(MsgSubmitMessagesResponse {
        state_root: encode_hex(&values.state_root),
        messages,
    })
}

/// The zkism keeper as a hyperlane ISM (router type 42).
#[derive(Debug, Clone, Copy, Default)]
pub struct ZkIsm;

fn message_key(internal: u64, id: &[u8]) -> Vec<u8> {
    let mut k = vec![MESSAGES_PREFIX];
    k.extend_from_slice(&sdk::codec::u64_key(internal));
    k.extend_from_slice(id);
    k
}

impl x_hyperlane::InterchainSecurityModule for ZkIsm {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, ism_id: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &ism_key(ism_id.internal_id()))
    }

    fn verify(
        &self,
        ctx: &mut Context<'_>,
        _: &Deps<'_>,
        ism_id: &HexAddress,
        _metadata: &[u8],
        message: &HyperlaneMessage,
    ) -> Result<bool> {
        let Some(ism) = get_ism(ctx, ism_id.internal_id())? else {
            return Err(ism_not_found(format!(
                "collections: not found: key '{}' of type uint64",
                ism_id.internal_id()
            )));
        };
        let internal = decode_id(&ism.id)?.internal_id();
        let key = message_key(internal, &message.id().0);
        let authorized = ctx.has(STORE_KEY, &key)?;
        if authorized {
            ctx.delete(STORE_KEY, &key)?;
        }
        Ok(authorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    const VK: &[u8] = include_bytes!("../testdata/groth16_vk.bin");
    const ST_PROOF: &[u8] = include_bytes!("../testdata/state_transition/proof.bin");
    const ST_VALUES: &[u8] = include_bytes!("../testdata/state_transition/public_values.bin");
    const SM_PROOF: &[u8] = include_bytes!("../testdata/state_membership/proof.bin");
    const SM_VALUES: &[u8] = include_bytes!("../testdata/state_membership/public_values.bin");

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap_or_default()
    }

    #[derive(Debug)]
    struct Bank;
    impl x_hyperlane::BankKeeper for Bank {
        fn send_coins_from_account_to_module(
            &self,
            _: &mut Context<'_>,
            _: &[u8],
            _: &str,
            _: &x_hyperlane::Coins,
        ) -> Result<()> {
            Ok(())
        }
        fn send_coins_from_module_to_account(
            &self,
            _: &mut Context<'_>,
            _: &str,
            _: &[u8],
            _: &x_hyperlane::Coins,
        ) -> Result<()> {
            Ok(())
        }
        fn mint_coins(&self, _: &mut Context<'_>, _: &str, _: &x_hyperlane::Coins) -> Result<()> {
            Ok(())
        }
        fn burn_coins(&self, _: &mut Context<'_>, _: &str, _: &x_hyperlane::Coins) -> Result<()> {
            Ok(())
        }
        fn has_supply(&self, _: &mut Context<'_>, _: &str) -> Result<bool> {
            Ok(false)
        }
    }

    #[test]
    fn create_update_submit_and_verify_with_go_testdata() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        ms.mount(x_hyperlane::STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let bank = Bank;
        let mut deps = Deps::new(&bank);
        deps.isms.register(MODULE_TYPE_ZK_ISM, &ZkIsm)?;

        let st = unmarshal_state_transition(ST_VALUES).map_err(internal)?;
        let sm = unmarshal_state_membership(SM_VALUES).map_err(internal)?;
        let create = MsgCreateInterchainSecurityModule {
            creator: "celestia1x".into(),
            state: st.state.clone(),
            merkle_tree_address: sm.merkle_tree_address.to_vec(),
            groth16_vkey: VK.to_vec(),
            state_transition_vkey: h(
                "004ac29c473e811dece0f8dd76c8eda80f886d263efb393ec81f54173e54f160",
            ),
            state_membership_vkey: h(
                "00982fb21526d096c8bf58eda36b5e293ee9ea0f36df441f6a996a974f8feb63",
            ),
        };
        validate_basic_create(&create)?;
        let id = create_ism(&mut ctx, &deps, &create)?.id;
        assert_eq!(
            id,
            "0x726f757465725f69736d000000000000000000000000002a0000000000000000"
        );
        // Membership before the transition: wrong state root.
        let submit = MsgSubmitMessages {
            id: id.clone(),
            proof: SM_PROOF.to_vec(),
            public_values: SM_VALUES.to_vec(),
            signer: "celestia1x".into(),
        };
        let e = submit_messages(&mut ctx, &submit).err();
        assert_eq!(e.map(|e| (e.codespace, e.code)), Some(("zkism", 8)));

        let update = MsgUpdateInterchainSecurityModule {
            id: id.clone(),
            proof: ST_PROOF.to_vec(),
            public_values: ST_VALUES.to_vec(),
            signer: "celestia1x".into(),
        };
        validate_basic_update(&update)?;
        let gas0 = ctx.gas.consumed();
        assert_eq!(update_ism(&mut ctx, &update)?.state, st.new_state);
        assert!(ctx.gas.consumed() >= gas0 + PROOF_VERIFY_COST_GROTH16);
        // Replaying the transition fails (state moved on).
        let e = update_ism(&mut ctx, &update).err();
        assert_eq!(e.map(|e| (e.codespace, e.code)), Some(("zkism", 7)));

        validate_basic_submit(&submit)?;
        let r = submit_messages(&mut ctx, &submit)?;
        assert_eq!(r.messages, vec![encode_hex(&sm.message_ids[0])]);
        let e = submit_messages(&mut ctx, &submit).err();
        assert_eq!(e.map(|e| (e.codespace, e.code)), Some(("zkism", 10)));

        // Tampered proof.
        let mut bad = update.clone();
        bad.proof[300] ^= 1;
        assert!(update_ism(&mut ctx, &bad).is_err());
        Ok(())
    }

    #[test]
    fn public_values_parsing() {
        assert!(unmarshal_state_transition(&[]).is_err());
        let mut v = 31u64.to_le_bytes().to_vec();
        v.extend([0u8; 31]);
        assert!(unmarshal_state_transition(&v).is_err());
        let mut sm = SM_VALUES.to_vec();
        sm.push(0);
        assert!(unmarshal_state_membership(&sm).is_err());
        assert!(validate_groth16_vkey(&VK[..491]).is_err());
        assert!(validate_groth16_vkey(VK).is_ok());
    }
}
