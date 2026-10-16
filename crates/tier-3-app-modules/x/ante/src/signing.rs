//! Signature verification: sign-mode handlers (`x/tx/signing` direct,
//! direct-aux, aminojson; textual is not enabled in celestia-app v10:
//! `authtx.DefaultSignModes`), `authsigning.VerifySignature` and the key
//! types' `VerifySignature`.

use std::collections::HashMap;
use std::sync::OnceLock;

use celestia_proto::Any;
use celestia_proto::cosmos::tx::v1beta1::{SignDoc, SignDocDirectAux};
use prost::Message;
use prost_reflect::{DescriptorPool, Value};
use sdk::SdkError;
use sdk::tx::Tx;

use crate::pubkey::PubKey;
use crate::txview::{self, SigData};
use crate::{R, idx, panic};

/// `SIGN_MODE_UNSPECIFIED`.
pub const SIGN_MODE_UNSPECIFIED: i32 = 0;
/// `SIGN_MODE_DIRECT`.
pub const SIGN_MODE_DIRECT: i32 = 1;
/// `SIGN_MODE_TEXTUAL`.
pub const SIGN_MODE_TEXTUAL: i32 = 2;
/// `SIGN_MODE_DIRECT_AUX`.
pub const SIGN_MODE_DIRECT_AUX: i32 = 3;
/// `SIGN_MODE_LEGACY_AMINO_JSON`.
pub const SIGN_MODE_LEGACY_AMINO_JSON: i32 = 127;

static DESCRIPTORS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptors.bin"));

/// The descriptor pool of every vendored proto (built by `build.rs`).
pub fn pool() -> &'static DescriptorPool {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    POOL.get_or_init(|| DescriptorPool::decode(DESCRIPTORS).unwrap_or_default())
}

/// Message type URLs whose `cosmos.msg.v1.signer` field is a
/// `cosmos.ValidatorAddressString` (decoded with the validator codec).
fn validator_signer_types() -> &'static HashMap<String, bool> {
    static MAP: OnceLock<HashMap<String, bool>> = OnceLock::new();
    MAP.get_or_init(|| {
        let p = pool();
        let mut m = HashMap::new();
        let (Some(signer_ext), Some(scalar_ext)) = (
            p.get_extension_by_name("cosmos.msg.v1.signer"),
            p.get_extension_by_name("cosmos_proto.scalar"),
        ) else {
            return m;
        };
        for msg in p.all_messages() {
            let opts = msg.options();
            if !opts.has_extension(&signer_ext) {
                continue;
            }
            let Value::List(fields) = opts.get_extension(&signer_ext).into_owned() else {
                continue;
            };
            let validator = fields.iter().any(|f| {
                let Some(name) = f.as_str() else { return false };
                msg.get_field_by_name(name).is_some_and(|fd| {
                    fd.options()
                        .get_extension(&scalar_ext)
                        .as_str()
                        .is_some_and(|s| s == "cosmos.ValidatorAddressString")
                })
            });
            m.insert(format!("/{}", msg.full_name()), validator);
        }
        m
    })
}

/// Whether `type_url`'s signer field uses the validator address codec.
#[must_use]
pub fn is_validator_signer(type_url: &str) -> bool {
    validator_signer_types()
        .get(type_url)
        .copied()
        .unwrap_or(false)
}

/// `txsigning.SignerData`.
#[derive(Debug, Clone)]
pub struct SignerData {
    /// Bech32 signer address.
    pub address: String,
    /// Chain ID.
    pub chain_id: String,
    /// Account number (0 at genesis).
    pub account_number: u64,
    /// Account sequence.
    pub sequence: u64,
    /// Account public key (canonical `Any`).
    pub pub_key: Any,
}

/// `wrapper.GetSigningTxData`: panics like Go on signer infos without a
/// public key or multi mode infos without a bit array.
pub fn check_signing_tx_data(tx: &Tx) -> R<()> {
    fn adapt(mi: &celestia_proto::cosmos::tx::v1beta1::ModeInfo) -> R<()> {
        use celestia_proto::cosmos::tx::v1beta1::mode_info::Sum;
        if let Some(Sum::Multi(m)) = &mi.sum {
            for inner in &m.mode_infos {
                adapt(inner)?;
            }
            if m.bitarray.is_none() {
                return panic("runtime error: invalid memory address or nil pointer dereference");
            }
        }
        Ok(())
    }
    for si in &tx.auth_info.signer_infos {
        if let Some(mi) = &si.mode_info {
            adapt(mi)?;
        }
        if si.public_key.is_none() {
            return panic("signerInfo.PublicKey cannot be nil");
        }
    }
    Ok(())
}

/// `HandlerMap.GetSignBytes`.
pub fn get_sign_bytes(mode: i32, sd: &SignerData, tx: &Tx) -> R<Result<Vec<u8>, String>> {
    match mode {
        SIGN_MODE_DIRECT => Ok(Ok(SignDoc {
            body_bytes: tx.body_bytes.clone(),
            auth_info_bytes: tx.auth_info_bytes.clone(),
            chain_id: sd.chain_id.clone(),
            account_number: sd.account_number,
        }
        .encode_to_vec())),
        SIGN_MODE_DIRECT_AUX => direct_aux_sign_bytes(sd, tx),
        SIGN_MODE_LEGACY_AMINO_JSON => Ok(crate::aminojson::sign_bytes(sd, tx)),
        m => Ok(Err(format!("unsupported sign mode {}", sign_mode_name(m)))),
    }
}

fn sign_mode_name(m: i32) -> String {
    match m {
        SIGN_MODE_UNSPECIFIED => "SIGN_MODE_UNSPECIFIED".into(),
        SIGN_MODE_TEXTUAL => "SIGN_MODE_TEXTUAL".into(),
        191 => "SIGN_MODE_EIP_191".into(),
        other => other.to_string(),
    }
}

fn direct_aux_sign_bytes(sd: &SignerData, tx: &Tx) -> R<Result<Vec<u8>, String>> {
    let fee = txview::fee(tx)?;
    let fee_payer = if fee.payer.is_empty() {
        if tx.body.messages.is_empty() {
            return Ok(Err("no signer found".into()));
        }
        let signers = match txview::tx_msg_signers(tx, 0) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e.log)),
        };
        txview::bytes_to_bech32(idx(&signers, 0)?)
    } else {
        fee.payer.clone()
    };
    if fee_payer == sd.address {
        return Ok(Err(format!(
            "fee payer {fee_payer} cannot sign with SIGN_MODE_DIRECT_AUX: unauthorized"
        )));
    }
    Ok(Ok(SignDocDirectAux {
        body_bytes: tx.body_bytes.clone(),
        public_key: Some(sd.pub_key.clone()),
        chain_id: sd.chain_id.clone(),
        account_number: sd.account_number,
        sequence: sd.sequence,
        #[allow(deprecated)]
        tip: None,
    }
    .encode_to_vec()))
}

/// `internalSignModeToAPI`.
fn api_sign_mode(m: i32) -> Result<i32, String> {
    match m {
        SIGN_MODE_DIRECT
        | SIGN_MODE_LEGACY_AMINO_JSON
        | SIGN_MODE_TEXTUAL
        | SIGN_MODE_DIRECT_AUX => Ok(m),
        other => Err(format!("unsupported sign mode {}", sign_mode_name(other))),
    }
}

/// `authsigning.VerifySignature`. The inner `Result` carries Go's returned
/// error (always wrapped as `ErrUnauthorized` by the caller).
pub fn verify_signature(
    pk: &PubKey,
    sd: &SignerData,
    data: Option<&SigData>,
    tx: &Tx,
) -> R<Result<(), String>> {
    match data {
        Some(SigData::Single { mode, signature }) => {
            let m = match api_sign_mode(*mode) {
                Ok(m) => m,
                Err(e) => return Ok(Err(e)),
            };
            let bytes = match get_sign_bytes(m, sd, tx)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            if pk.verify_signature(&bytes, signature)? {
                Ok(Ok(()))
            } else {
                Ok(Err("unable to verify single signer signature".into()))
            }
        }
        Some(multi @ SigData::Multi { .. }) => {
            if !matches!(pk, PubKey::Multisig { .. }) {
                return Ok(Err(format!(
                    "expected multisig.PubKey, got {}",
                    pk.go_type()
                )));
            }
            verify_multisignature(pk, multi, sd, tx)
        }
        None => Ok(Err("unexpected SignatureData <nil>".into())),
    }
}

/// `LegacyAminoPubKey.VerifyMultisignature` over the tx sign bytes.
fn verify_multisignature(
    pk: &PubKey,
    data: &SigData,
    sd: &SignerData,
    tx: &Tx,
) -> R<Result<(), String>> {
    verify_multisig_with(pk, data, &mut |mode| match api_sign_mode(mode) {
        Ok(m) => get_sign_bytes(m, sd, tx),
        Err(e) => Ok(Err(e)),
    })
}

/// `LegacyAminoPubKey.VerifyMultisignature` with Go's `getSignBytes`
/// callback (sign mode -> bytes, or the error it returns).
pub fn verify_multisig_with(
    pk: &PubKey,
    data: &SigData,
    get_sign_bytes: &mut dyn FnMut(i32) -> R<Result<Vec<u8>, String>>,
) -> R<Result<(), String>> {
    let (
        PubKey::Multisig {
            threshold, keys, ..
        },
        SigData::Multi {
            bitarray,
            signatures,
        },
    ) = (pk, data)
    else {
        return Ok(Err("not a multisig".into()));
    };
    let ba = bitarray.as_ref();
    let size = txview::bit_count(ba);
    if keys.len() as i64 != size {
        return Ok(Err(format!(
            "bit array size is incorrect, expecting: {}",
            keys.len()
        )));
    }
    let n = signatures.len() as i64;
    if n < i64::from(*threshold) || n > size {
        return Ok(Err(format!("signature size is incorrect {n}")));
    }
    let set = txview::bit_num_true_before(ba, size)?;
    if set < i64::from(*threshold) {
        return Ok(Err(format!(
            "not enough signatures set, have {set}, expected {threshold}"
        )));
    }
    let mut sig_index = 0usize;
    for i in 0..size {
        if !txview::bit_get(ba, i)? {
            continue;
        }
        let key = idx(keys, usize::try_from(i).unwrap_or(usize::MAX))?;
        match idx(signatures, sig_index)? {
            Some(SigData::Single { mode, signature }) => {
                let bytes = match get_sign_bytes(*mode)? {
                    Ok(b) => b,
                    Err(e) => return Ok(Err(e)),
                };
                if !key.verify_signature(&bytes, signature)? {
                    return Ok(Err(format!("unable to verify signature at index {i}")));
                }
            }
            Some(nested @ SigData::Multi { .. }) => {
                if !matches!(key, PubKey::Multisig { .. }) {
                    return Ok(Err(format!("unable to parse pubkey of index {i}")));
                }
                if let Err(e) = verify_multisig_with(key, nested, get_sign_bytes)? {
                    return Ok(Err(e));
                }
            }
            None => {
                return Ok(Err(format!(
                    "improper signature data type for index {sig_index}"
                )));
            }
        }
        sig_index += 1;
    }
    Ok(Ok(()))
}

/// secp256k1 `PubKey.VerifySignature` (cosmos-sdk `secp256k1_nocgo.go`):
/// `r` and `s` are reduced modulo the group order (decred
/// `ModNScalar.SetByteSlice`), high-S signatures are rejected.
#[must_use]
pub fn verify_secp256k1(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    if sig.len() != 64 {
        return false;
    }
    // A tx's signature is checked by CheckTx, PrepareProposal,
    // ProcessProposal and FinalizeBlock: remember the valid ones.
    let digest = hash::sha256(msg);
    let entry = verified_cache_key(key, &digest, sig);
    if VERIFIED.contains(&entry) {
        return true;
    }
    let valid = verify_secp256k1_prehashed(key, &digest, sig);
    if valid {
        VERIFIED.insert(entry);
    }
    valid
}

/// Identifies one `(key, sha256(msg), sig)` verification.
fn verified_cache_key(key: &[u8], digest: &[u8; 32], sig: &[u8]) -> [u8; 32] {
    let mut h = hash::Sha256Hasher::new();
    h.update(&(key.len() as u64).to_be_bytes());
    h.update(key);
    h.update(digest);
    h.update(sig);
    h.finalize()
}

/// Verifications that succeeded, so repeats skip the curve arithmetic.
/// Verification is a pure function of its inputs and only successes are
/// kept, so every result is the one a fresh verification returns.
static VERIFIED: std::sync::LazyLock<VerifiedCache> =
    std::sync::LazyLock::new(|| VerifiedCache::new(1 << 16));

/// A bounded set (oldest entries evicted first).
struct VerifiedCache {
    capacity: usize,
    inner: std::sync::Mutex<VerifiedSet>,
}

/// The entries, and their insertion order for eviction.
#[derive(Default)]
struct VerifiedSet(
    std::collections::HashSet<[u8; 32]>,
    std::collections::VecDeque<[u8; 32]>,
);

impl VerifiedCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: std::sync::Mutex::new(VerifiedSet::default()),
        }
    }

    fn contains(&self, entry: &[u8; 32]) -> bool {
        self.inner.lock().is_ok_and(|g| g.0.contains(entry))
    }

    fn insert(&self, entry: [u8; 32]) {
        if let Ok(mut g) = self.inner.lock() {
            let VerifiedSet(set, order) = &mut *g;
            if set.insert(entry) {
                order.push_back(entry);
                while order.len() > self.capacity {
                    if let Some(old) = order.pop_front() {
                        set.remove(&old);
                    }
                }
            }
        }
    }
}

/// [`verify_secp256k1`] of a message with SHA-256 digest `digest`
/// (libsecp256k1; the Go-equivalence rules are in
/// [`crypto::secp256k1::verify_go_prehashed`]).
fn verify_secp256k1_prehashed(key: &[u8], digest: &[u8; 32], sig: &[u8]) -> bool {
    crypto::secp256k1::verify_go_prehashed(key, digest, sig)
}

/// `elliptic.UnmarshalCompressed(P256)` succeeds.
#[must_use]
pub fn p256_point_valid(key: &[u8]) -> bool {
    key.len() == 33 && matches!(key[0], 2 | 3) && p256::PublicKey::from_sec1_bytes(key).is_ok()
}

/// secp256r1 `PubKey.VerifySignature` (cosmos-sdk `internal/ecdsa`):
/// raw `r || s`, low-S required, Go `ecdsa.Verify`.
#[must_use]
pub fn verify_secp256r1(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    if sig.len() != 64 || !p256_point_valid(key) {
        return false;
    }
    let Ok(vk) = p256::ecdsa::VerifyingKey::from_sec1_bytes(key) else {
        return false;
    };
    let Ok(signature) = p256::ecdsa::Signature::from_slice(sig) else {
        return false;
    };
    if signature.normalize_s().is_some() {
        return false;
    }
    let digest = hash::sha256(msg);
    p256::ecdsa::signature::hazmat::PrehashVerifier::verify_prehash(&vk, &digest, &signature)
        .is_ok()
}

/// Maps a sign-bytes/verification failure to the `SigVerificationDecorator`
/// error.
#[must_use]
pub fn unauthorized(msg: String) -> SdkError {
    sdk::error::sdk::unauthorized(msg)
}
