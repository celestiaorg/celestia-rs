//! Solo machine public keys: the tx decoder keeps their `Any`s as
//! received, as Go does (ibc-go stores the bytes, and `Initialize`
//! compares the client's consensus state with `reflect.DeepEqual`).

use celestia_proto::ibc::core::client::v1::MsgCreateClient;
use celestia_proto::ibc::lightclients::solomachine::v3 as sm;
use prost::Message as _;

/// A `MsgCreateClient` (nightly fuzz-exec, deliver `ibc-upgrades` seed
/// 3080) whose client state's multisig key has a sub-key with an unknown
/// field (`f8 7f 01`, field 2047) that the consensus state's copy lacks.
/// Go fails the message (the consensus states differ); with normalized
/// keys both copies compared equal.
const TX: &[&str] = &[
    "0af7050af4050a232f6962632e636f72652e636c69656e742e76312e4d7367437265617465436c69656e7412cc050acd",
    "020a2c2f6962632e6c69676874636c69656e74732e736f6c6f6d616368696e652e76332e436c69656e74537461746512",
    "9c0208011a97020a8b020a292f636f736d6f732e63727970746f2e6d756c74697369672e4c6567616379416d696e6f50",
    "75624b657912dd01080312460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a",
    "210238c135df3f2d89a2db5d9a6d9b2f7608d5d4c342e6d2de5530c2cc385bc3c91112490a1f2f636f736d6f732e6372",
    "7970746f2e736563703235366b312e5075624b657912260a210322772495704cdd7fc5e41c40cd09cda3d9b40fe1792d",
    "0b2607ef54d815115ef6f87f0112460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b6579",
    "12230a2102809bbacaafc0faf51fc670a90a7615ca1b4aaa5b34758c0726777daa2fffd4c912056d756c7469180a12c8",
    "020a2f2f6962632e6c69676874636c69656e74732e736f6c6f6d616368696e652e76332e436f6e73656e737573537461",
    "74651294020a88020a292f636f736d6f732e63727970746f2e6d756c74697369672e4c6567616379416d696e6f507562",
    "4b657912da01080312460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2102",
    "38c135df3f2d89a2db5d9a6d9b2f7608d5d4c342e6d2de5530c2cc385bc3c91112460a1f2f636f736d6f732e63727970",
    "746f2e736563703235366b312e5075624b657912230a210322772495704cdd7fc5e41c40cd09cda3d9b40fe1792d0b26",
    "07ef54d815115ef612460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2102",
    "809bbacaafc0faf51fc670a90a7615ca1b4aaa5b34758c0726777daa2fffd4c912056d756c7469180a1a2f63656c6573",
    "746961316d65633076656d6867676c306d7a686b36707964776d7a6a6d3075656e61616b6b76336b673512660a500a46",
    "0a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2102bf815bfa3cf18a8aa85c",
    "fad8ed858f24a56e85f32314c350704a6c407d00efdf12040a020801180512120a0c0a047574696112043436313810da",
    "8b0e1a408e0127e72d944a393dff339b37c311508694cc0eb1de85582657bae43fbac230703c5894ce4759c639f73fd4",
    "41f5f79d5ce0742927d5cf06ae8f1863b72b9075",
];

#[test]
fn solomachine_public_key_any_is_kept_as_received() {
    let raw = hex::decode(TX.concat()).unwrap();
    let tx = txdecode::decode(&raw).unwrap();
    // What the message router hands the handler.
    let routed = txdecode::prost_compatible(&tx.msgs()[0]).unwrap();
    let msg = MsgCreateClient::decode(routed.value.as_slice()).unwrap();
    let client = sm::ClientState::decode(msg.client_state.unwrap().value.as_slice()).unwrap();
    let cons = sm::ConsensusState::decode(msg.consensus_state.unwrap().value.as_slice()).unwrap();
    let key = |c: &sm::ConsensusState| c.public_key.clone().unwrap().value;
    let in_client = key(client.consensus_state.as_ref().unwrap());
    assert!(in_client.windows(3).any(|w| w == [0xf8, 0x7f, 0x01]));
    assert!(!key(&cons).windows(3).any(|w| w == [0xf8, 0x7f, 0x01]));
    assert_ne!(client.consensus_state, Some(cons));
}
