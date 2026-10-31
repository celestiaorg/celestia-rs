//! Checkpoints signed by harness-held Hyperlane validators.

use anyhow::{Context as _, bail};
use celestia_proto::Any;
use celestia_proto::hyperlane::core::interchain_security::v1 as pb;
use k256::ecdsa::SigningKey;
use prost::Message as _;
use x_hyperlane::ism::checkpoint_digest;
use x_hyperlane::util::{branch_root, keccak256};

pub(super) struct Verifier {
    name: String,
    keys: Vec<SigningKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rejection {
    MissingSignature,
    DuplicateSignature,
    ReversedSignatures,
    RecoveryId,
    MerkleProof,
    SignedIndex,
}

impl Rejection {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::MissingSignature => "missing-signature",
            Self::DuplicateSignature => "duplicate-signature",
            Self::ReversedSignatures => "reversed-signatures",
            Self::RecoveryId => "recovery-id",
            Self::MerkleProof => "merkle-proof",
            Self::SignedIndex => "signed-index",
        }
    }

    pub(super) fn error_marker(self) -> &'static str {
        match self {
            Self::MissingSignature => "threshold can not be reached",
            Self::DuplicateSignature | Self::ReversedSignatures | Self::MerkleProof => {
                "ism verification failed"
            }
            Self::RecoveryId => "failed to recover validator signature",
            Self::SignedIndex => "invalid signed index",
        }
    }

    fn mutate(self, metadata: &mut Vec<u8>) {
        let offset = metadata.len() - 130;
        match self {
            Self::MissingSignature => metadata.truncate(metadata.len() - 65),
            Self::DuplicateSignature => metadata.copy_within(offset..offset + 65, offset + 65),
            Self::ReversedSignatures => metadata[offset..].rotate_left(65),
            Self::RecoveryId => metadata[offset + 64] = 0,
            Self::MerkleProof => metadata[68] ^= 1,
            Self::SignedIndex => {
                metadata[32..36].copy_from_slice(&1u32.to_be_bytes());
                metadata[1092..1096].copy_from_slice(&0u32.to_be_bytes());
            }
        }
    }

    pub(super) fn messages(self, valid: &[Any]) -> anyhow::Result<Vec<Any>> {
        use celestia_proto::hyperlane::core::v1::MsgProcessMessage;
        let mut invalid = valid.to_vec();
        let last = invalid.last_mut().context("missing incoming message")?;
        anyhow::ensure!(
            last.type_url == <MsgProcessMessage as prost::Name>::type_url(),
            "expected incoming MsgProcessMessage"
        );
        let mut process = MsgProcessMessage::decode(last.value.as_slice())?;
        let mut metadata = hex::decode(process.metadata.trim_start_matches("0x"))?;
        let minimum = if matches!(self, Self::MerkleProof | Self::SignedIndex) {
            1226
        } else {
            198
        };
        anyhow::ensure!(metadata.len() >= minimum, "missing signed metadata");
        self.mutate(&mut metadata);
        process.metadata = format!("0x{}", hex::encode(metadata));
        *last = Any::pack(&process);
        Ok(invalid)
    }
}

impl Default for Verifier {
    fn default() -> Self {
        Self {
            name: "noop".into(),
            keys: Vec::new(),
        }
    }
}

fn address(key: &SigningKey) -> [u8; 20] {
    let point = key.verifying_key().to_sec1_point(false);
    let hash = keccak256(&[&point.as_bytes()[1..]]);
    let mut address = [0; 20];
    address.copy_from_slice(&hash[12..]);
    address
}

impl Verifier {
    pub(super) fn from_env() -> anyhow::Result<Self> {
        let name = match std::env::var("DELIVER_HL_ISM") {
            Ok(name) => name,
            Err(std::env::VarError::NotPresent) => "noop".into(),
            Err(error) => return Err(error.into()),
        };
        Self::named(&name)
    }

    fn named(name: &str) -> anyhow::Result<Self> {
        if !matches!(name, "noop" | "message-id" | "merkle-root") {
            bail!("DELIVER_HL_ISM must be noop, message-id or merkle-root; got {name:?}");
        }
        let mut keys = Vec::new();
        if name != "noop" {
            for seed in 1u8..=3 {
                keys.push(SigningKey::from_slice(&[seed; 32]).context("Hyperlane test key")?);
            }
            // Signatures must follow decoded Ethereum-address order.
            keys.sort_by_key(address);
        }
        Ok(Self {
            name: name.into(),
            keys,
        })
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn rejections(&self) -> &'static [Rejection] {
        use Rejection::{
            DuplicateSignature, MerkleProof, MissingSignature, RecoveryId, ReversedSignatures,
            SignedIndex,
        };
        match self.name.as_str() {
            "message-id" => &[
                MissingSignature,
                DuplicateSignature,
                ReversedSignatures,
                RecoveryId,
            ],
            "merkle-root" => &[
                MissingSignature,
                DuplicateSignature,
                ReversedSignatures,
                RecoveryId,
                MerkleProof,
                SignedIndex,
            ],
            _ => &[],
        }
    }

    pub(super) fn create(&self, creator: String) -> Any {
        let validators = self
            .keys
            .iter()
            .map(|key| format!("0x{}", hex::encode(address(key))))
            .collect();
        match self.name.as_str() {
            "message-id" => Any::pack(&pb::MsgCreateMessageIdMultisigIsm {
                creator,
                validators,
                threshold: 2,
            }),
            "merkle-root" => Any::pack(&pb::MsgCreateMerkleRootMultisigIsm {
                creator,
                validators,
                threshold: 2,
            }),
            _ => Any::pack(&pb::MsgCreateNoopIsm { creator }),
        }
    }

    pub(super) fn metadata(&self, origin: u32, message: &[u8], index: u32) -> Vec<u8> {
        if self.name == "noop" {
            return Vec::new();
        }
        let hook = [4; 32];
        let message_id = keccak256(&[message]);
        let mut metadata = hook.to_vec();
        let root = if self.name == "merkle-root" {
            let proof = [[0; 32]; 32];
            metadata.extend_from_slice(&index.to_be_bytes());
            metadata.extend_from_slice(&message_id);
            for sibling in &proof {
                metadata.extend_from_slice(sibling);
            }
            metadata.extend_from_slice(&index.to_be_bytes());
            branch_root(message_id, &proof, index)
        } else {
            let root = [8; 32];
            metadata.extend_from_slice(&root);
            metadata.extend_from_slice(&index.to_be_bytes());
            root
        };
        let digest = checkpoint_digest(origin, &hook, &root, index, &message_id);
        for key in self.keys.iter().take(2) {
            let (signature, recovery) = key.sign_prehash_recoverable(&digest);
            metadata.extend_from_slice(&signature.to_bytes());
            metadata.push(recovery.to_byte() + 27);
        }
        metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{Context, ExecMode, Header, MultiStore};
    use x_hyperlane::ism::Ism;
    use x_hyperlane::util::{HexAddress, HyperlaneMessage};

    #[test]
    fn checkpoints_verify_and_reject_changed_messages_and_signatures() {
        for name in ["message-id", "merkle-root"] {
            let verifier = Verifier::named(name).unwrap();
            let validators = verifier
                .keys
                .iter()
                .map(|key| format!("0x{}", hex::encode(address(key))))
                .collect();
            let ism = if name == "message-id" {
                Ism::MessageIdMultisig {
                    id: HexAddress::default(),
                    owner: String::new(),
                    validators,
                    threshold: 2,
                }
            } else {
                Ism::MerkleRootMultisig {
                    id: HexAddress::default(),
                    owner: String::new(),
                    validators,
                    threshold: 2,
                }
            };
            let message = HyperlaneMessage {
                origin: 88,
                body: vec![1, 2, 3],
                ..Default::default()
            };
            let mut store = MultiStore::new();
            let mut context = Context::new(&mut store, Header::default(), ExecMode::Finalize);
            for index in [0, 1, 3, u32::MAX] {
                let metadata = verifier.metadata(88, &message.bytes(), index);
                assert!(ism.verify(&mut context, &metadata, &message).unwrap());
                for &case in verifier.rejections() {
                    use celestia_proto::hyperlane::core::v1::MsgProcessMessage;
                    let process = MsgProcessMessage {
                        message: format!("0x{}", hex::encode(message.bytes())),
                        metadata: format!("0x{}", hex::encode(&metadata)),
                        ..Default::default()
                    };
                    let prefix =
                        Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend::default());
                    let invalid = case
                        .messages(&[prefix.clone(), Any::pack(&process)])
                        .unwrap();
                    assert_eq!(invalid[0], prefix);
                    let invalid = MsgProcessMessage::decode(invalid[1].value.as_slice()).unwrap();
                    assert_eq!(invalid.message, process.message);
                    let mutated = hex::decode(invalid.metadata.trim_start_matches("0x")).unwrap();
                    let result = ism.verify(&mut context, &mutated, &message);
                    match case {
                        Rejection::DuplicateSignature
                        | Rejection::ReversedSignatures
                        | Rejection::MerkleProof => assert!(!result.unwrap(), "{}", case.name()),
                        _ => assert!(
                            result
                                .unwrap_err()
                                .to_string()
                                .contains(case.error_marker()),
                            "{}",
                            case.name()
                        ),
                    }
                    assert!(ism.verify(&mut context, &metadata, &message).unwrap());
                }
                let mut changed = message.clone();
                changed.body.push(4);
                assert!(!ism.verify(&mut context, &metadata, &changed).unwrap());
                let mut reversed = metadata.clone();
                let offset = reversed.len() - 130;
                reversed[offset..].rotate_left(65);
                assert!(!ism.verify(&mut context, &reversed, &message).unwrap());
                let truncated = &metadata[..metadata.len() - 65];
                assert!(ism.verify(&mut context, truncated, &message).is_err());
                if name == "merkle-root" {
                    let mut invalid_proof = metadata.clone();
                    invalid_proof[68] ^= 1;
                    assert!(!ism.verify(&mut context, &invalid_proof, &message).unwrap());
                }
            }
        }
    }

    #[test]
    fn unknown_mode_is_rejected() {
        assert!(Verifier::named("merkle-rooot").is_err());
        assert!(
            Verifier::named("noop")
                .unwrap()
                .metadata(88, b"message", 0)
                .is_empty()
        );
    }

    #[test]
    fn rejection_requires_process_message_and_sufficient_metadata() {
        use celestia_proto::hyperlane::core::v1::MsgProcessMessage;
        assert!(Rejection::MissingSignature.messages(&[]).is_err());
        let mut message = Any::pack(&MsgProcessMessage {
            metadata: format!("0x{}", "00".repeat(198)),
            ..Default::default()
        });
        assert!(Rejection::SignedIndex.messages(&[message.clone()]).is_err());
        message.type_url = "unexpected".into();
        assert!(Rejection::MissingSignature.messages(&[message]).is_err());
    }
}
