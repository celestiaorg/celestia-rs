//! 06-solomachine (ibc-go v8.7.2 `modules/light-clients/06-solomachine`):
//! header and misbehaviour verification, membership proofs over
//! `TimestampedSignatureData` / `SignBytes`, and client recovery.
//!
//! Signatures are checked with the SDK public keys (`x-ante`): secp256k1,
//! secp256r1, ed25519 and `LegacyAminoPubKey` multisig.

use celestia_proto::Any;
use celestia_proto::cosmos::tx::signing::v1beta1::signature_descriptor::{Data, data};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::commitment::v1::MerklePath;
use celestia_proto::ibc::lightclients::solomachine::v3 as sm;
use prost::Message;
use sdk::{Result, SdkError};
use x_ante::pubkey::PubKey;
use x_ante::txview::SigData;

use crate::errors::{client as cerr, host as herr, solomachine as serr};

/// `SentinelHeaderPath`.
pub const SENTINEL_HEADER_PATH: &str = "solomachine:header";

/// A decoded 06-solomachine client message.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage {
    /// `Header`.
    Header(Box<sm::Header>),
    /// `Misbehaviour`.
    Misbehaviour(Box<sm::Misbehaviour>),
}

impl ClientMessage {
    /// Go `%T` of the message.
    #[must_use]
    pub fn go_type(&self) -> &'static str {
        match self {
            Self::Header(_) => "*solomachine.Header",
            Self::Misbehaviour(_) => "*solomachine.Misbehaviour",
        }
    }

    /// The packed form.
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Header(h) => Any::pack(h.as_ref()),
            Self::Misbehaviour(m) => Any::pack(m.as_ref()),
        }
    }
}

/// `UnpackClientMessage` for the solo machine types (`None` for others).
#[must_use]
pub fn unpack_client_message(any: &Any) -> Option<Result<ClientMessage>> {
    let dec = |e: prost::DecodeError| crate::errors::ibc::unpack_any(e.to_string());
    if let Some(r) = any.unpack::<sm::Header>() {
        return Some(r.map(|h| ClientMessage::Header(Box::new(h))).map_err(dec));
    }
    if let Some(r) = any.unpack::<sm::Misbehaviour>() {
        return Some(
            r.map(|m| ClientMessage::Misbehaviour(Box::new(m)))
                .map_err(dec),
        );
    }
    None
}

/// The public key packed in `any` (`GetCachedValue().(cryptotypes.PubKey)`).
fn unpack_pub_key(any: &Any) -> Option<PubKey> {
    PubKey::from_any(any).ok()
}

/// `len(publicKey.Bytes()) == 0`.
fn empty_key(pk: &PubKey) -> bool {
    match pk {
        PubKey::Secp256k1(k) | PubKey::Secp256r1(k) | PubKey::Ed25519(k) => k.is_empty(),
        PubKey::Multisig { .. } => false,
    }
}

/// `ConsensusState.GetPubKey`.
fn consensus_pub_key(c: &sm::ConsensusState) -> Result<PubKey> {
    let Some(any) = c.public_key.as_ref() else {
        return Err(cerr::invalid_consensus(
            "consensus state PublicKey cannot be nil",
        ));
    };
    unpack_pub_key(any).ok_or_else(|| {
        cerr::invalid_consensus("consensus state PublicKey is not cryptotypes.PubKey")
    })
}

/// `ConsensusState.ValidateBasic`.
pub fn validate_consensus_state(c: &sm::ConsensusState) -> Result<()> {
    if c.timestamp == 0 {
        return Err(cerr::invalid_consensus("timestamp cannot be 0"));
    }
    if !c.diversifier.is_empty() && c.diversifier.trim().is_empty() {
        return Err(cerr::invalid_consensus(
            "diversifier cannot contain only spaces",
        ));
    }
    match consensus_pub_key(c) {
        Ok(pk) if !empty_key(&pk) => Ok(()),
        _ => Err(cerr::invalid_consensus("public key cannot be empty")),
    }
}

/// `Header.ValidateBasic`.
pub fn validate_header(h: &sm::Header) -> Result<()> {
    if h.timestamp == 0 {
        return Err(cerr::invalid_header("timestamp cannot be zero"));
    }
    if !h.new_diversifier.is_empty() && h.new_diversifier.trim().is_empty() {
        return Err(cerr::invalid_header(
            "diversifier cannot contain only spaces",
        ));
    }
    if h.signature.is_empty() {
        return Err(cerr::invalid_header("signature cannot be empty"));
    }
    match h.new_public_key.as_ref().and_then(unpack_pub_key) {
        Some(pk) if !empty_key(&pk) => Ok(()),
        _ => Err(cerr::invalid_header("new public key cannot be empty")),
    }
}

/// `SignatureAndData.ValidateBasic`.
fn validate_signature_and_data(sd: &sm::SignatureAndData) -> Result<()> {
    if sd.signature.is_empty() {
        return Err(serr::invalid_signature_and_data(
            "signature cannot be empty",
        ));
    }
    if sd.data.is_empty() {
        return Err(serr::invalid_signature_and_data(
            "data for signature cannot be empty",
        ));
    }
    if sd.path.is_empty() {
        return Err(serr::invalid_signature_and_data(
            "path for signature cannot be empty",
        ));
    }
    if sd.timestamp == 0 {
        return Err(serr::invalid_signature_and_data("timestamp cannot be 0"));
    }
    Ok(())
}

fn nil_deref() -> SdkError {
    sdk::error::go_panic("runtime error: invalid memory address or nil pointer dereference")
}

/// `Misbehaviour.ValidateBasic`.
pub fn validate_misbehaviour(m: &sm::Misbehaviour) -> Result<()> {
    if m.sequence == 0 {
        return Err(cerr::invalid_misbehaviour("sequence cannot be 0"));
    }
    let one = m.signature_one.as_ref().ok_or_else(nil_deref)?;
    validate_signature_and_data(one)
        .map_err(|e| e.wrap("signature one failed basic validation"))?;
    let two = m.signature_two.as_ref().ok_or_else(nil_deref)?;
    validate_signature_and_data(two)
        .map_err(|e| e.wrap("signature two failed basic validation"))?;
    if one.signature == two.signature {
        return Err(cerr::invalid_misbehaviour(
            "misbehaviour signatures cannot be equal",
        ));
    }
    if one.path == two.path && one.data == two.data {
        return Err(cerr::invalid_misbehaviour(
            "misbehaviour signature data must be signed over different messages",
        ));
    }
    Ok(())
}

/// `ClientMessage.ValidateBasic`.
pub fn validate_client_message(m: &ClientMessage) -> Result<()> {
    match m {
        ClientMessage::Header(h) => validate_header(h),
        ClientMessage::Misbehaviour(mb) => validate_misbehaviour(mb),
    }
}

/// `signing.SignatureDataFromProto` (a nil `Sum` panics in Go).
fn signature_data_from_proto(d: &Data) -> Result<SigData> {
    match &d.sum {
        Some(data::Sum::Single(s)) => Ok(SigData::Single {
            mode: s.mode,
            signature: s.signature.clone(),
        }),
        Some(data::Sum::Multi(m)) => Ok(SigData::Multi {
            bitarray: m.bitarray.clone(),
            signatures: m
                .signatures
                .iter()
                .map(signature_data_from_proto)
                .collect::<Result<_>>()?,
        }),
        None => Err(sdk::error::go_panic("unexpected case <nil>")),
    }
}

/// `UnmarshalSignatureData`.
fn unmarshal_signature_data(bz: &[u8]) -> Result<SigData> {
    let d = Data::decode(bz).map_err(|e| {
        crate::errors::unregistered(format!(
            "failed to unmarshal proof into type *signing.SignatureDescriptor_Data: {e}"
        ))
    })?;
    signature_data_from_proto(&d)
}

fn ante_fail(f: x_ante::Fail) -> SdkError {
    match f {
        x_ante::Fail::Err(e) | x_ante::Fail::ZeroCtx(e) => e,
        x_ante::Fail::Panic(p) => sdk::error::go_panic(p),
    }
}

/// `VerifySignature`.
fn verify_signature(pk: &PubKey, sign_bytes: &[u8], sig: &SigData) -> Result<()> {
    if matches!(pk, PubKey::Multisig { .. }) {
        let SigData::Multi { .. } = sig else {
            return Err(serr::signature_verification_failed(
                "invalid signature data type, expected *signing.MultiSignatureData, got *signing.MultiSignatureData",
            ));
        };
        return x_ante::signing::verify_multisig_with(pk, sig, &mut |_| {
            Ok(Ok(sign_bytes.to_vec()))
        })
        .map_err(ante_fail)?
        .map_err(crate::errors::unregistered);
    }
    let SigData::Single { signature, .. } = sig else {
        return Err(serr::signature_verification_failed(
            "invalid signature data type, expected *signing.SingleSignatureData, got *signing.SingleSignatureData",
        ));
    };
    if pk
        .verify_signature(sign_bytes, signature)
        .map_err(ante_fail)?
    {
        Ok(())
    } else {
        Err(serr::signature_verification_failed(""))
    }
}

fn consensus(cs: &sm::ClientState) -> Result<&sm::ConsensusState> {
    cs.consensus_state.as_ref().ok_or_else(nil_deref)
}

/// `VerifyClientMessage` for a solo machine message.
pub fn verify_client_message(cs: &sm::ClientState, msg: &ClientMessage) -> Result<()> {
    match msg {
        ClientMessage::Header(h) => verify_header(cs, h),
        ClientMessage::Misbehaviour(m) => {
            verify_signature_and_data(cs, m, m.signature_one.as_ref())
                .map_err(|e| e.wrap("failed to verify signature one"))?;
            verify_signature_and_data(cs, m, m.signature_two.as_ref())
                .map_err(|e| e.wrap("failed to verify signature two"))
        }
    }
}

fn verify_header(cs: &sm::ClientState, h: &sm::Header) -> Result<()> {
    let cons = consensus(cs)?;
    if h.timestamp < cons.timestamp {
        return Err(cerr::invalid_header(format!(
            "header timestamp is less than to the consensus state timestamp ({} < {})",
            h.timestamp, cons.timestamp
        )));
    }
    let data = sm::HeaderData {
        new_pub_key: h.new_public_key.clone(),
        new_diversifier: h.new_diversifier.clone(),
    }
    .encode_to_vec();
    let sign_bytes = sm::SignBytes {
        sequence: cs.sequence,
        timestamp: h.timestamp,
        diversifier: cons.diversifier.clone(),
        path: SENTINEL_HEADER_PATH.as_bytes().to_vec(),
        data,
    }
    .encode_to_vec();
    let sig = unmarshal_signature_data(&h.signature)?;
    let pk = consensus_pub_key(cons)?;
    verify_signature(&pk, &sign_bytes, &sig).map_err(|e| serr::invalid_header(e.log))
}

fn verify_signature_and_data(
    cs: &sm::ClientState,
    m: &sm::Misbehaviour,
    sd: Option<&sm::SignatureAndData>,
) -> Result<()> {
    let sd = sd.ok_or_else(nil_deref)?;
    MerklePath::decode(sd.path.as_slice())
        .map_err(|e| crate::errors::unregistered(e.to_string()))?;
    let cons = consensus(cs)?;
    let sign_bytes = sm::SignBytes {
        sequence: m.sequence,
        timestamp: sd.timestamp,
        diversifier: cons.diversifier.clone(),
        path: sd.path.clone(),
        data: sd.data.clone(),
    }
    .encode_to_vec();
    let sig = unmarshal_signature_data(&sd.signature)?;
    let pk = consensus_pub_key(cons)?;
    verify_signature(&pk, &sign_bytes, &sig)
}

/// `UpdateState`: the new consensus state from the header; returns the
/// consensus heights.
#[must_use]
pub fn update_state(cs: &sm::ClientState, h: &sm::Header) -> (sm::ClientState, Vec<Height>) {
    let mut cs = cs.clone();
    cs.sequence = cs.sequence.wrapping_add(1);
    cs.consensus_state = Some(sm::ConsensusState {
        public_key: h.new_public_key.clone(),
        diversifier: h.new_diversifier.clone(),
        timestamp: h.timestamp,
    });
    let heights = vec![Height {
        revision_number: 0,
        revision_height: cs.sequence,
    }];
    (cs, heights)
}

/// `UpdateStateOnMisbehaviour`.
#[must_use]
pub fn update_state_on_misbehaviour(cs: &sm::ClientState) -> sm::ClientState {
    let mut cs = cs.clone();
    cs.is_frozen = true;
    cs
}

/// `VerifyMembership` / `VerifyNonMembership` (`value = None`): returns the
/// client state with the sequence consumed.
pub fn verify_membership(
    cs: &sm::ClientState,
    proof: &[u8],
    path: &MerklePath,
    value: Option<&[u8]>,
) -> Result<sm::ClientState> {
    // produceVerificationArgs (the `proof == nil` check is the caller's).
    let ts = sm::TimestampedSignatureData::decode(proof).map_err(|e| {
        crate::errors::unregistered(format!(
            "failed to unmarshal proof into type solomachine.TimestampedSignatureData: {e}"
        ))
    })?;
    if ts.signature_data.is_empty() {
        return Err(serr::invalid_proof("signature data cannot be empty"));
    }
    let sig = unmarshal_signature_data(&ts.signature_data)?;
    let cons = consensus(cs)?;
    if cons.timestamp > ts.timestamp {
        return Err(serr::invalid_proof(format!(
            "the consensus state timestamp is greater than the signature timestamp ({} >= {})",
            cons.timestamp, ts.timestamp
        )));
    }
    let pk = consensus_pub_key(cons)?;
    if path.key_path.len() != 2 {
        return Err(herr::invalid_path(format!(
            "path must be of length 2: [{}]",
            path.key_path.join(" ")
        )));
    }
    let sign_bytes = sm::SignBytes {
        sequence: cs.sequence,
        timestamp: ts.timestamp,
        diversifier: cons.diversifier.clone(),
        path: path.key_path[1].as_bytes().to_vec(),
        data: value.map(<[u8]>::to_vec).unwrap_or_default(),
    }
    .encode_to_vec();
    verify_signature(&pk, &sign_bytes, &sig)?;
    let mut cs = cs.clone();
    cs.sequence = cs.sequence.wrapping_add(1);
    if let Some(c) = cs.consensus_state.as_mut() {
        c.timestamp = ts.timestamp;
    }
    Ok(cs)
}

/// `CheckSubstituteAndUpdateState`: the recovered subject client state.
pub fn check_substitute_and_update_state(
    subject: &sm::ClientState,
    substitute: Option<&sm::ClientState>,
    substitute_go_type: &str,
) -> Result<sm::ClientState> {
    let Some(sub) = substitute else {
        return Err(cerr::invalid_client_type(format!(
            "substitute client state type {substitute_go_type}, expected  *solomachine.ClientState"
        )));
    };
    let subject_pk = consensus_pub_key(consensus(subject)?)
        .map_err(|e| e.wrap("failed to get consensus public key"))?;
    let sub_pk = consensus_pub_key(consensus(sub)?)
        .map_err(|e| e.wrap("failed to get substitute client public key"))?;
    if subject_pk == sub_pk {
        return Err(cerr::invalid_header(
            "subject and substitute have the same public key",
        ));
    }
    let mut cs = subject.clone();
    cs.sequence = sub.sequence;
    cs.consensus_state.clone_from(&sub.consensus_state);
    cs.is_frozen = false;
    Ok(cs)
}
