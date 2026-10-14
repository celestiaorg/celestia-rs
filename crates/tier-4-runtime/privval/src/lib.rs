//! Port of celestia-core v0.42.3 `privval/file.go`.
//!
//! [`FilePv`] signs votes and proposals with an Ed25519 key kept in
//! `priv_validator_key.json` and guards against double signing with the
//! last-sign state in `priv_validator_state.json` (height / round / step plus
//! the last signature and sign bytes). Both files use Go's exact JSON layout
//! (`cmtjson.MarshalIndent`), so a Rust validator can take over a Go
//! validator's home directory and vice versa.
//!
//! The last-sign state is written atomically (temp file, fsync, rename,
//! directory fsync) **before** a new signature is returned. A crash between
//! signing and the vote reaching the consensus WAL is recovered like Go:
//! signing the same height/round/step again returns the stored signature
//! when the sign bytes are identical, or when they differ only in their
//! timestamp (the old timestamp is then restored into the message); any
//! other difference is refused with `conflicting data`.

#![forbid(unsafe_code)]

use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ctypes::PubKey;
use ctypes::canonical::{proposal_sign_bytes, vote_extension_sign_bytes, vote_sign_bytes};
use ctypes::proto::types as pb;
use prost::Message;

/// `stepNone`: the initial state.
pub const STEP_NONE: i8 = 0;
/// `stepPropose`.
pub const STEP_PROPOSE: i8 = 1;
/// `stepPrevote`.
pub const STEP_PREVOTE: i8 = 2;
/// `stepPrecommit`.
pub const STEP_PRECOMMIT: i8 = 3;

/// Amino JSON type name of an Ed25519 public key.
pub const PUB_KEY_ED25519_NAME: &str = "tendermint/PubKeyEd25519";
/// Amino JSON type name of an Ed25519 private key.
pub const PRIV_KEY_ED25519_NAME: &str = "tendermint/PrivKeyEd25519";

/// A signer error. Messages match Go's error strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Result alias.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// `types.PrivValidator`: what consensus needs from a signer.
pub trait PrivValidator: Send {
    /// `GetPubKey`.
    fn pub_key(&self) -> Result<PubKey>;
    /// `SignVote`: sets `signature` (and `extension_signature` for non-nil
    /// precommits); may rewrite `timestamp` when re-signing.
    fn sign_vote(&mut self, chain_id: &str, vote: &mut pb::Vote) -> Result<()>;
    /// `SignProposal`: sets `signature`; may rewrite `timestamp`.
    fn sign_proposal(&mut self, chain_id: &str, proposal: &mut pb::Proposal) -> Result<()>;
    /// `SignRawBytes`: signs `RawBytesMessageSignBytes(chain_id, unique_id,
    /// raw_bytes)` (no last-sign state; used for celestia's compact
    /// blocks).
    fn sign_raw_bytes(&self, chain_id: &str, unique_id: &str, raw_bytes: &[u8]) -> Result<Vec<u8>> {
        let _ = (chain_id, unique_id, raw_bytes);
        Err(Error::new("SignRawBytes is not supported by this signer"))
    }
}

/// `voteToStep`.
fn vote_to_step(vote: &pb::Vote) -> Result<i8> {
    match vote.r#type {
        ctypes::vote::PREVOTE_TYPE => Ok(STEP_PREVOTE),
        ctypes::vote::PRECOMMIT_TYPE => Ok(STEP_PRECOMMIT),
        t => Err(Error::new(format!("Unknown vote type: {t}"))),
    }
}

/// `FilePVKey`: the immutable part of the validator.
#[derive(Clone, PartialEq, Eq)]
pub struct FilePvKey {
    /// Ed25519 private key (`seed || public key`).
    pub priv_key: crypto::ed25519::PrivateKeyBytes,
    /// Public key (derived).
    pub pub_key: crypto::ed25519::PublicKeyBytes,
    /// Address (derived).
    pub address: Vec<u8>,
    file_path: Option<PathBuf>,
}

impl fmt::Debug for FilePvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilePvKey")
            .field("address", &hex::encode_upper(&self.address))
            .finish_non_exhaustive()
    }
}

impl FilePvKey {
    /// Builds the key from a 64-byte `seed || public key` private key.
    pub fn from_priv_key(priv_key: crypto::ed25519::PrivateKeyBytes) -> Result<Self> {
        let pub_key = crypto::ed25519::public_key_from_private_key(&priv_key)
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(Self {
            priv_key,
            pub_key,
            address: crypto::ed25519::address(&pub_key).to_vec(),
            file_path: None,
        })
    }

    /// The consensus public key.
    #[must_use]
    pub fn pub_key(&self) -> PubKey {
        PubKey::Ed25519(self.pub_key)
    }

    /// `cmtjson.MarshalIndent(pvKey, "", "  ")`.
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"address\": \"{}\",\n  \"pub_key\": {{\n    \"type\": \"{PUB_KEY_ED25519_NAME}\",\n    \"value\": \"{}\"\n  }},\n  \"priv_key\": {{\n    \"type\": \"{PRIV_KEY_ED25519_NAME}\",\n    \"value\": \"{}\"\n  }}\n}}",
            hex::encode_upper(&self.address),
            B64.encode(self.pub_key),
            B64.encode(self.priv_key)
        )
    }

    /// Parses `priv_validator_key.json`. Like Go, the public key and
    /// address are recomputed from the private key.
    pub fn from_json(bz: &[u8]) -> Result<Self> {
        let v: serde_json::Value =
            serde_json::from_slice(bz).map_err(|e| Error::new(e.to_string()))?;
        let pk = &v["priv_key"];
        if pk["type"].as_str() != Some(PRIV_KEY_ED25519_NAME) {
            return Err(Error::new(format!(
                "unsupported private key type {}",
                pk["type"]
            )));
        }
        let raw = B64
            .decode(pk["value"].as_str().unwrap_or_default())
            .map_err(|e| Error::new(e.to_string()))?;
        let priv_key = <crypto::ed25519::PrivateKeyBytes>::try_from(raw.as_slice())
            .map_err(|_| Error::new("invalid Ed25519 private key length"))?;
        Self::from_priv_key(priv_key)
    }

    /// `FilePVKey.Save`.
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.file_path else {
            return Err(Error::new(
                "cannot save PrivValidator key: filePath not set",
            ));
        };
        write_file_atomic(path, self.to_json().as_bytes())
    }

    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>> {
        crypto::ed25519::sign(&self.priv_key, msg)
            .map(|s| s.to_vec())
            .map_err(|e| Error::new(e.to_string()))
    }
}

/// `FilePVLastSignState`: the mutable part of the validator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastSignState {
    /// Height of the last signature.
    pub height: i64,
    /// Round of the last signature.
    pub round: i32,
    /// Step of the last signature.
    pub step: i8,
    /// Last signature (`None` = Go nil).
    pub signature: Option<Vec<u8>>,
    /// Last sign bytes (`None` = Go nil).
    pub sign_bytes: Option<Vec<u8>>,
    file_path: Option<PathBuf>,
}

impl LastSignState {
    /// `CheckHRS`: errors on a regression; `Ok(true)` when the HRS equals
    /// the last one and the stored signature can be reused.
    pub fn check_hrs(&self, height: i64, round: i32, step: i8) -> Result<bool> {
        if self.height > height {
            return Err(Error::new(format!(
                "height regression. Got {height}, last height {}",
                self.height
            )));
        }
        if self.height == height {
            if self.round > round {
                return Err(Error::new(format!(
                    "round regression at height {height}. Got {round}, last round {}",
                    self.round
                )));
            }
            if self.round == round {
                if self.step > step {
                    return Err(Error::new(format!(
                        "step regression at height {height} round {round}. Got {step}, last step {}",
                        self.step
                    )));
                } else if self.step == step {
                    if self.sign_bytes.is_some() {
                        if self.signature.is_none() {
                            return Err(Error::new("pv: Signature is nil but SignBytes is not!"));
                        }
                        return Ok(true);
                    }
                    return Err(Error::new("no SignBytes found"));
                }
            }
        }
        Ok(false)
    }

    /// `cmtjson.MarshalIndent(lss, "", "  ")`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut s = format!(
            "{{\n  \"height\": \"{}\",\n  \"round\": {},\n  \"step\": {}",
            self.height, self.round, self.step
        );
        if let Some(sig) = self.signature.as_ref().filter(|s| !s.is_empty()) {
            let _ = write!(s, ",\n  \"signature\": \"{}\"", B64.encode(sig));
        }
        if let Some(sb) = self.sign_bytes.as_ref().filter(|s| !s.is_empty()) {
            let _ = write!(s, ",\n  \"signbytes\": \"{}\"", hex::encode_upper(sb));
        }
        s.push_str("\n}");
        s
    }

    /// Parses `priv_validator_state.json` (`height` as a JSON string, like
    /// cmtjson; a number is accepted too).
    pub fn from_json(bz: &[u8]) -> Result<Self> {
        let v: serde_json::Value =
            serde_json::from_slice(bz).map_err(|e| Error::new(e.to_string()))?;
        let height = match &v["height"] {
            serde_json::Value::String(s) => s
                .parse::<i64>()
                .map_err(|e| Error::new(format!("invalid height: {e}")))?,
            serde_json::Value::Number(n) => {
                n.as_i64().ok_or_else(|| Error::new("invalid height"))?
            }
            serde_json::Value::Null => 0,
            other => return Err(Error::new(format!("invalid height {other}"))),
        };
        let int = |k: &str| -> Result<i64> {
            match &v[k] {
                serde_json::Value::Null => Ok(0),
                serde_json::Value::Number(n) => {
                    n.as_i64().ok_or_else(|| Error::new(format!("invalid {k}")))
                }
                other => Err(Error::new(format!("invalid {k} {other}"))),
            }
        };
        let round = i32::try_from(int("round")?).map_err(|e| Error::new(e.to_string()))?;
        let step = i8::try_from(int("step")?).map_err(|e| Error::new(e.to_string()))?;
        let signature = match v["signature"].as_str() {
            Some(s) => Some(B64.decode(s).map_err(|e| Error::new(e.to_string()))?),
            None => None,
        };
        let sign_bytes = match v["signbytes"].as_str() {
            Some(s) => Some(hex::decode(s).map_err(|e| Error::new(e.to_string()))?),
            None => None,
        };
        Ok(Self {
            height,
            round,
            step,
            signature,
            sign_bytes,
            file_path: None,
        })
    }

    /// `FilePVLastSignState.Save`: atomic write with fsync.
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.file_path else {
            return Err(Error::new(
                "cannot save FilePVLastSignState: filePath not set",
            ));
        };
        write_file_atomic(path, self.to_json().as_bytes())
    }

    fn reset(&mut self) {
        self.height = 0;
        self.round = 0;
        self.step = 0;
        self.signature = None;
        self.sign_bytes = None;
    }
}

/// `tempfile.WriteFileAtomic` (mode 0600): temp file in the same
/// directory, fsync, rename over the target, fsync the directory.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let io = |e: std::io::Error| Error::new(format!("{}: {e}", path.display()));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(io)?;
        f.write_all(data).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)?;
    if let Ok(d) = fs::File::open(dir) {
        // Directory fsync makes the rename durable; not supported everywhere.
        let _ = d.sync_all();
    }
    Ok(())
}

/// `FilePV`.
#[derive(Debug, Clone)]
pub struct FilePv {
    /// Key.
    pub key: FilePvKey,
    /// Last-sign state.
    pub last_sign_state: LastSignState,
}

impl FilePv {
    /// `NewFilePV`.
    #[must_use]
    pub fn new(key: FilePvKey, key_path: Option<PathBuf>, state_path: Option<PathBuf>) -> Self {
        let mut key = key;
        key.file_path = key_path;
        Self {
            key,
            last_sign_state: LastSignState {
                step: STEP_NONE,
                file_path: state_path,
                ..LastSignState::default()
            },
        }
    }

    /// `GenFilePV`: a fresh random key (not saved).
    pub fn generate(key_path: impl AsRef<Path>, state_path: impl AsRef<Path>) -> Result<Self> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| Error::new(e.to_string()))?;
        let key = FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(&seed))?;
        Ok(Self::new(
            key,
            Some(key_path.as_ref().to_path_buf()),
            Some(state_path.as_ref().to_path_buf()),
        ))
    }

    /// `LoadFilePV`.
    pub fn load(key_path: impl AsRef<Path>, state_path: impl AsRef<Path>) -> Result<Self> {
        Self::load_inner(key_path.as_ref(), state_path.as_ref(), true)
    }

    /// `LoadFilePVEmptyState`.
    pub fn load_empty_state(
        key_path: impl AsRef<Path>,
        state_path: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::load_inner(key_path.as_ref(), state_path.as_ref(), false)
    }

    fn load_inner(key_path: &Path, state_path: &Path, load_state: bool) -> Result<Self> {
        let bz =
            fs::read(key_path).map_err(|e| Error::new(format!("{}: {e}", key_path.display())))?;
        let mut key = FilePvKey::from_json(&bz).map_err(|e| {
            Error::new(format!(
                "Error reading PrivValidator key from {}: {e}",
                key_path.display()
            ))
        })?;
        key.file_path = Some(key_path.to_path_buf());
        let mut state = if load_state {
            let bz = fs::read(state_path)
                .map_err(|e| Error::new(format!("{}: {e}", state_path.display())))?;
            LastSignState::from_json(&bz).map_err(|e| {
                Error::new(format!(
                    "Error reading PrivValidator state from {}: {e}",
                    state_path.display()
                ))
            })?
        } else {
            LastSignState::default()
        };
        state.file_path = Some(state_path.to_path_buf());
        Ok(Self {
            key,
            last_sign_state: state,
        })
    }

    /// `LoadOrGenFilePV`.
    pub fn load_or_generate(
        key_path: impl AsRef<Path>,
        state_path: impl AsRef<Path>,
    ) -> Result<Self> {
        if key_path.as_ref().exists() {
            Self::load(key_path, state_path)
        } else {
            let pv = Self::generate(key_path, state_path)?;
            pv.save()?;
            Ok(pv)
        }
    }

    /// `GetAddress`.
    #[must_use]
    pub fn address(&self) -> &[u8] {
        &self.key.address
    }

    /// `Save`: key and state.
    pub fn save(&self) -> Result<()> {
        self.key.save()?;
        self.last_sign_state.save()
    }

    /// `Reset` (unsafe: forgets the double-sign guard).
    pub fn reset(&mut self) -> Result<()> {
        self.last_sign_state.reset();
        self.save()
    }

    /// `signVote`.
    fn sign_vote_inner(&mut self, chain_id: &str, vote: &mut pb::Vote) -> Result<()> {
        let (height, round, step) = (vote.height, vote.round, vote_to_step(vote)?);
        let lss = &self.last_sign_state;
        let same_hrs = lss.check_hrs(height, round, step)?;
        let sign_bytes = vote_sign_bytes(chain_id, vote).map_err(|e| Error::new(e.to_string()))?;

        // Extension signatures are always (re-)signed for non-nil precommits.
        let default_id = pb::BlockId::default();
        let is_nil =
            ctypes::block_id::proto_block_id_is_nil(vote.block_id.as_ref().unwrap_or(&default_id));
        let mut ext_sig = Vec::new();
        if vote.r#type == ctypes::vote::PRECOMMIT_TYPE && !is_nil {
            ext_sig = self.key.sign(&vote_extension_sign_bytes(chain_id, vote))?;
        } else if !vote.extension.is_empty() {
            return Err(Error::new(
                "unexpected vote extension - extensions are only allowed in non-nil precommits",
            ));
        }

        if same_hrs {
            let last = lss.sign_bytes.clone().unwrap_or_default();
            let mut res = Ok(());
            if sign_bytes == last {
                vote.signature = lss.signature.clone().unwrap_or_default();
            } else if let Some(ts) = votes_only_differ_by_timestamp(&last, &sign_bytes)? {
                vote.timestamp = Some(ts);
                vote.signature = lss.signature.clone().unwrap_or_default();
            } else {
                res = Err(Error::new("conflicting data"));
            }
            vote.extension_signature = ext_sig;
            return res;
        }

        let sig = self.key.sign(&sign_bytes)?;
        self.save_signed(height, round, step, sign_bytes, sig.clone())?;
        vote.signature = sig;
        vote.extension_signature = ext_sig;
        Ok(())
    }

    /// `signProposal`.
    fn sign_proposal_inner(&mut self, chain_id: &str, proposal: &mut pb::Proposal) -> Result<()> {
        let (height, round, step) = (proposal.height, proposal.round, STEP_PROPOSE);
        let lss = &self.last_sign_state;
        let same_hrs = lss.check_hrs(height, round, step)?;
        let sign_bytes =
            proposal_sign_bytes(chain_id, proposal).map_err(|e| Error::new(e.to_string()))?;
        if same_hrs {
            let last = lss.sign_bytes.clone().unwrap_or_default();
            if sign_bytes == last {
                proposal.signature = lss.signature.clone().unwrap_or_default();
            } else if let Some(ts) = proposals_only_differ_by_timestamp(&last, &sign_bytes)? {
                proposal.timestamp = Some(ts);
                proposal.signature = lss.signature.clone().unwrap_or_default();
            } else {
                return Err(Error::new("conflicting data"));
            }
            return Ok(());
        }
        let sig = self.key.sign(&sign_bytes)?;
        self.save_signed(height, round, step, sign_bytes, sig.clone())?;
        proposal.signature = sig;
        Ok(())
    }

    /// `saveSigned`: persists (with fsync) before the signature is released.
    fn save_signed(
        &mut self,
        height: i64,
        round: i32,
        step: i8,
        sign_bytes: Vec<u8>,
        sig: Vec<u8>,
    ) -> Result<()> {
        let mut next = self.last_sign_state.clone();
        next.height = height;
        next.round = round;
        next.step = step;
        next.signature = Some(sig);
        next.sign_bytes = Some(sign_bytes);
        if next.file_path.is_some() {
            next.save()?;
        }
        self.last_sign_state = next;
        Ok(())
    }
}

impl PrivValidator for FilePv {
    fn pub_key(&self) -> Result<PubKey> {
        Ok(self.key.pub_key())
    }

    fn sign_vote(&mut self, chain_id: &str, vote: &mut pb::Vote) -> Result<()> {
        self.sign_vote_inner(chain_id, vote)
            .map_err(|e| Error::new(format!("error signing vote: {e}")))
    }

    fn sign_proposal(&mut self, chain_id: &str, proposal: &mut pb::Proposal) -> Result<()> {
        self.sign_proposal_inner(chain_id, proposal)
            .map_err(|e| Error::new(format!("error signing proposal: {e}")))
    }

    fn sign_raw_bytes(&self, chain_id: &str, unique_id: &str, raw_bytes: &[u8]) -> Result<Vec<u8>> {
        let sb = ctypes::canonical::raw_bytes_message_sign_bytes(chain_id, unique_id, raw_bytes)
            .map_err(|e| Error::new(format!("error signing raw bytes: {e}")))?;
        self.key.sign(&sb)
    }
}

impl fmt::Display for FilePv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PrivValidator{{{} LH:{}, LR:{}, LS:{}}}",
            hex::encode_upper(&self.key.address),
            self.last_sign_state.height,
            self.last_sign_state.round,
            self.last_sign_state.step
        )
    }
}

/// `checkVotesOnlyDifferByTimestamp`: the last timestamp when the canonical
/// votes are equal up to their timestamps.
fn votes_only_differ_by_timestamp(
    last: &[u8],
    new: &[u8],
) -> Result<Option<pbjson_types::Timestamp>> {
    let mut a = pb::CanonicalVote::decode_length_delimited(last).map_err(|e| {
        Error::new(format!(
            "LastSignBytes cannot be unmarshalled into vote: {e}"
        ))
    })?;
    let mut b = pb::CanonicalVote::decode_length_delimited(new)
        .map_err(|e| Error::new(format!("signBytes cannot be unmarshalled into vote: {e}")))?;
    let last_time = a.timestamp.unwrap_or_default();
    a.timestamp = None;
    b.timestamp = None;
    Ok((a == b).then_some(last_time))
}

/// `checkProposalsOnlyDifferByTimestamp`.
fn proposals_only_differ_by_timestamp(
    last: &[u8],
    new: &[u8],
) -> Result<Option<pbjson_types::Timestamp>> {
    let mut a = pb::CanonicalProposal::decode_length_delimited(last).map_err(|e| {
        Error::new(format!(
            "LastSignBytes cannot be unmarshalled into proposal: {e}"
        ))
    })?;
    let mut b = pb::CanonicalProposal::decode_length_delimited(new).map_err(|e| {
        Error::new(format!(
            "signBytes cannot be unmarshalled into proposal: {e}"
        ))
    })?;
    let last_time = a.timestamp.unwrap_or_default();
    a.timestamp = None;
    b.timestamp = None;
    Ok((a == b).then_some(last_time))
}

#[cfg(test)]
mod tests;
