//! Store errors (Go `ErrStoreNotFound`, `ErrStoreIntegrity`, joined errors
//! and `partialDeleteError`).

use std::fmt;
use std::io;

/// Errors returned by [`super::Store`].
///
/// Go distinguishes errors with `errors.Is`; here the predicates
/// [`Self::is_not_found`], [`Self::is_integrity`] and [`Self::is_cancelled`]
/// look through [`Self::Joined`] and [`Self::PartialDelete`] the same way.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// Go `ErrStoreNotFound`: no readable shard for the commitment.
    #[error("no shard found in store")]
    NotFound,
    /// Go `ErrStoreIntegrity`: stored metadata is invalid (corrupt marker,
    /// malformed key, unavailable backend). The data is kept for an operator.
    #[error("store integrity error: {0}")]
    Integrity(String),
    /// The caller's cancellation token fired (Go `context.Canceled`).
    #[error("operation cancelled")]
    Cancelled,
    /// The configuration asks for something celestia-rs does not implement
    /// (the `object` storage backend).
    #[error("{0}")]
    Unsupported(String),
    /// Invalid store configuration.
    #[error("validating store config: {0}")]
    Config(String),
    /// The store was closed.
    #[error("store is closed")]
    Closed,
    /// A file system operation failed.
    #[error("{context}: {source}")]
    Io {
        /// What the store was doing.
        context: String,
        /// The I/O error.
        #[source]
        source: io::Error,
    },
    /// The metadata database failed.
    #[error("metadata database: {0}")]
    Db(String),
    /// A shard file could not be decoded (Go returns a plain error here,
    /// not `ErrStoreIntegrity`).
    #[error("reading shard payload: {0}")]
    Codec(String),
    /// Any other failure (invalid promise, size overflow, ...).
    #[error("{0}")]
    Other(String),
    /// Several errors (Go `errors.Join`).
    #[error("{}", DisplayJoined(.0))]
    Joined(Vec<StoreError>),
    /// Prune could not delete some payloads (Go `partialDeleteError`). The
    /// metadata of the failed deletions is kept for the next pass.
    #[error("{}", DisplayPartial(.deletion, .integrity.as_deref()))]
    PartialDelete {
        /// The first payload deletion failure.
        deletion: Box<StoreError>,
        /// Corrupt markers skipped in the same batch.
        integrity: Option<Box<StoreError>>,
    },
}

impl StoreError {
    /// Go `errors.Is(err, ErrStoreNotFound)`.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        self.any(&|e| matches!(e, Self::NotFound))
    }

    /// Go `errors.Is(err, ErrStoreIntegrity)`.
    #[must_use]
    pub fn is_integrity(&self) -> bool {
        self.any(&|e| matches!(e, Self::Integrity(_)))
    }

    /// Go `errors.Is(err, context.Canceled)`.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.any(&|e| matches!(e, Self::Cancelled))
    }

    fn any(&self, pred: &dyn Fn(&Self) -> bool) -> bool {
        if pred(self) {
            return true;
        }
        match self {
            Self::Joined(errs) => errs.iter().any(|e| e.any(pred)),
            Self::PartialDelete {
                deletion,
                integrity,
            } => deletion.any(pred) || integrity.as_ref().is_some_and(|e| e.any(pred)),
            _ => false,
        }
    }

    /// Wraps an I/O error with context.
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Go `errors.Join`: `None` when every input is `None`, the error itself
    /// when there is one, [`Self::Joined`] otherwise.
    pub(crate) fn join(errs: impl IntoIterator<Item = Option<Self>>) -> Option<Self> {
        let mut errs: Vec<Self> = errs.into_iter().flatten().collect();
        match errs.len() {
            0 => None,
            1 => errs.pop(),
            _ => Some(Self::Joined(errs)),
        }
    }
}

/// Maps any redb error into [`StoreError::Db`].
pub(crate) fn db_err(err: impl Into<redb::Error>) -> StoreError {
    StoreError::Db(err.into().to_string())
}

struct DisplayJoined<'a>(&'a [StoreError]);

impl fmt::Display for DisplayJoined<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

struct DisplayPartial<'a>(&'a StoreError, Option<&'a StoreError>);

impl fmt::Display for DisplayPartial<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        if let Some(integrity) = self.1 {
            write!(f, "; {integrity}")?;
        }
        Ok(())
    }
}
