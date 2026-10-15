//! SecretConnection handshake against an arbitrary remote byte stream
//! (ephemeral key message, then the encrypted auth signature).
#![no_main]

use libfuzzer_sys::fuzz_target;
use p2p::NodeKey;
use p2p::secret_connection::SecretConnection;

fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let data = data.to_vec();
    rt.block_on(async move {
        let key = NodeKey::from_seed(&[7u8; 32]);
        let stream = tokio::io::join(std::io::Cursor::new(data), tokio::io::sink());
        let _ = SecretConnection::handshake_with_ephemeral(stream, &key, [9u8; 32]).await;
    });
});
