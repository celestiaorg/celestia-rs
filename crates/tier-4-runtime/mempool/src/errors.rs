//! Mempool errors (`mempool/errors.go`, `cat.ErrTxInMempool`). Messages
//! match Go's `Error()` strings.

use std::fmt;

/// An error returned by the mempool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MempoolError {
    /// `ErrTxNotFound`.
    TxNotFound,
    /// `ErrTxInCache`.
    TxInCache,
    /// `cat.ErrTxInMempool`.
    TxInMempool,
    /// `ErrTxTooLarge`.
    TxTooLarge {
        /// Configured maximum.
        max: usize,
        /// Actual size.
        actual: usize,
    },
    /// The pre-check filter rejected the tx (Go returns its error as is).
    PreCheck(String),
    /// `rejected bad transaction after post check: ...`.
    PostCheck(String),
    /// The pool is full and nothing of lower priority can be evicted.
    MempoolFull(String),
    /// The application connection failed (`CheckTx` / `Flush` error).
    App(String),
}

impl MempoolError {
    /// `errors.Is(err, ErrTxInMempool)`.
    #[must_use]
    pub fn is_tx_in_mempool(&self) -> bool {
        matches!(self, Self::TxInMempool)
    }
}

impl fmt::Display for MempoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TxNotFound => f.write_str("transaction not found in mempool"),
            Self::TxInCache => f.write_str("tx already exists in cache"),
            Self::TxInMempool => f.write_str("tx already exists in mempool"),
            Self::TxTooLarge { max, actual } => {
                write!(f, "Tx too large. Max size is {max}, but got {actual}")
            }
            Self::PreCheck(e) | Self::MempoolFull(e) | Self::App(e) => f.write_str(e),
            Self::PostCheck(e) => write!(f, "rejected bad transaction after post check: {e}"),
        }
    }
}

impl std::error::Error for MempoolError {}
