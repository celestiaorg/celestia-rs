//! lumina's Rust Fibre client (`celestia-fibre`, the revision the node
//! pins) uploads to and downloads from the server: a single validator holds
//! all stake, so its signature alone meets the client's safety threshold
//! and its shard alone reconstructs the blob.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use celestia_fibre::{
    BlobConfig, DownloadOptions, EncodedBlob, FibreClientBuilder, FibreClientConfig, FibreError,
    GrpcValidatorConnector, Host, HostRegistry, SetGetter, ValidatorInfo, ValidatorSet,
};
use common::*;
use fibre_server::{Server, Validator};

struct OneValidator {
    set: ValidatorSet,
}

#[async_trait::async_trait]
impl SetGetter for OneValidator {
    async fn head(&self) -> Result<ValidatorSet, FibreError> {
        Ok(self.set.clone())
    }
    async fn get_by_height(&self, _height: u64) -> Result<ValidatorSet, FibreError> {
        Ok(self.set.clone())
    }
}

struct Registry(String);

#[async_trait::async_trait]
impl HostRegistry for Registry {
    async fn get_host(&self, _validator: &ValidatorInfo) -> Result<Host, FibreError> {
        Ok(Host(self.0.clone()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lumina_client_upload_and_download() {
    let seed = [0x42u8; 32];
    let signer = FakeSigner::new(&seed);
    let pk = signer.key.public_key();
    let set = fibre_server::ValidatorSet {
        validators: vec![Validator::new(pk, 100)],
        height: u64::try_from(HEIGHT).unwrap(),
    };
    let dir = tempfile::tempdir().unwrap();
    let chain = FakeChain::new(set);
    let server = Server::new(test_config(dir.path()), chain, signer).unwrap();
    server.start().await.unwrap();
    let addr = server.listen_address();

    let client_set = ValidatorSet::try_new(
        vec![
            ValidatorInfo::try_new(ed25519_dalek::VerifyingKey::from_bytes(&pk).unwrap(), 100)
                .unwrap(),
        ],
        u64::try_from(HEIGHT).unwrap(),
    )
    .unwrap();
    let client = FibreClientBuilder::new()
        .config(FibreClientConfig::new(CHAIN_ID).unwrap())
        .set_getter(OneValidator { set: client_set })
        .connector(GrpcValidatorConnector::new(
            Arc::new(Registry(addr.to_string())),
            CHAIN_ID,
        ))
        .build()
        .unwrap();

    let data: Vec<u8> = (0..300_000u32)
        .map(|i| (i * 7 + 3).to_le_bytes()[0])
        .collect();
    let blob = EncodedBlob::new(&data, BlobConfig::v0()).unwrap();
    let id = blob.id().clone();
    let key = k256::ecdsa::SigningKey::from_slice(&escrow_key()).unwrap();
    let namespace = lumina_types::nmt::Namespace::new_v0(b"fibre-test").unwrap();
    let promise = tokio::time::timeout(
        Duration::from_secs(60),
        client.upload(&key, namespace, blob),
    )
    .await
    .expect("upload finished")
    .unwrap();
    assert_eq!(promise.validator_signatures.len(), 1);
    assert_eq!(server.metrics().store_puts(), 1);

    let got = tokio::time::timeout(
        Duration::from_secs(60),
        client.download(&id, DownloadOptions::default()),
    )
    .await
    .expect("download finished")
    .unwrap();
    assert_eq!(got.data(), data.as_slice());
    server.stop(Duration::from_secs(5)).await.unwrap();
}
