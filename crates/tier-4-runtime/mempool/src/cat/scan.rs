//! `mempool/cat/scan.go`: counts the entries of a `Txs` message before the
//! real decode, so a message sized for one max-size tx cannot be packed
//! with empty entries.

use celestia_core_proto::tendermint::mempool::TxCountMessage;
use prost::Message as _;

use crate::MAX_TXS_PER_MESSAGE;

/// `errTooManyTxs`.
pub const ERR_TOO_MANY_TXS: &str = "txs message contains too many transactions";

/// `validateMempoolBytes`.
pub fn validate_mempool_bytes(msg: &[u8]) -> Result<(), String> {
    let stub =
        TxCountMessage::decode(msg).map_err(|e| format!("malformed mempool message: {e}"))?;
    let Some(txs) = stub.txs else {
        // SeenTx / WantTx: nothing to count.
        return Ok(());
    };
    let n = txs.txs.len();
    if n > MAX_TXS_PER_MESSAGE {
        return Err(format!(
            "{ERR_TOO_MANY_TXS} (got {n}, max {MAX_TXS_PER_MESSAGE})"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]
mod tests {
    use celestia_core_proto::tendermint::mempool::{Message, SeenTx, Txs, WantTx, message::Sum};

    use super::*;

    fn txs_msg(txs: Vec<Vec<u8>>) -> Vec<u8> {
        Message {
            sum: Some(Sum::Txs(Txs { txs })),
        }
        .encode_to_vec()
    }

    #[test]
    fn validate() {
        assert!(validate_mempool_bytes(&txs_msg(vec![b"a valid transaction".to_vec()])).is_ok());
        let e = validate_mempool_bytes(&txs_msg(vec![b"tx one".to_vec(), b"tx two".to_vec()]))
            .unwrap_err();
        assert!(e.starts_with(ERR_TOO_MANY_TXS));
        let e = validate_mempool_bytes(&txs_msg(vec![Vec::new(); 10_000])).unwrap_err();
        assert!(e.starts_with(ERR_TOO_MANY_TXS));
        let seen = Message {
            sum: Some(Sum::SeenTx(SeenTx {
                tx_key: vec![0; 32],
                ..SeenTx::default()
            })),
        };
        assert!(validate_mempool_bytes(&seen.encode_to_vec()).is_ok());
        let want = Message {
            sum: Some(Sum::WantTx(WantTx {
                tx_key: vec![0; 32],
            })),
        };
        assert!(validate_mempool_bytes(&want.encode_to_vec()).is_ok());
        assert!(validate_mempool_bytes(&[]).is_ok());
        assert!(validate_mempool_bytes(&txs_msg(Vec::new())).is_ok());
        assert!(validate_mempool_bytes(&txs_msg(vec![Vec::new()])).is_ok());
        assert!(validate_mempool_bytes(&[0x78, 0x01]).is_ok());
    }

    #[test]
    fn malformed() {
        for m in [
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ][..],
            &[0x0a],
            &[0x0a, 0x02, 0x0a],
            &[0x00],
        ] {
            assert!(validate_mempool_bytes(m).is_err(), "{m:x?}");
        }
    }

    #[test]
    fn matches_reactor_limit() {
        let mut txs = vec![b"tx".to_vec(); MAX_TXS_PER_MESSAGE];
        assert!(validate_mempool_bytes(&txs_msg(txs.clone())).is_ok());
        txs.push(b"one too many".to_vec());
        assert!(validate_mempool_bytes(&txs_msg(txs)).is_err());
    }

    #[test]
    fn go_wire_vectors() {
        // TestMempoolVectors
        assert_eq!(hex::encode(txs_msg(vec![vec![123]])), "0a030a017b");
        assert_eq!(
            hex::encode(txs_msg(vec![b"proto encoding in mempool".to_vec()])),
            "0a1b0a1970726f746f20656e636f64696e6720696e206d656d706f6f6c"
        );
    }
}
