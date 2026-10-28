//! Checkpoints signed by harness-held Hyperlane validators.

use anyhow::{Context as _, bail};
use celestia_proto::Any;
use celestia_proto::hyperlane::core::interchain_security::v1 as pb;
use k256::ecdsa::SigningKey;
use x_hyperlane::ism::checkpoint_digest;
use x_hyperlane::util::{branch_root, keccak256};

pub(super) struct Verifier {
    name: String,
    keys: Vec<SigningKey>,
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
}
