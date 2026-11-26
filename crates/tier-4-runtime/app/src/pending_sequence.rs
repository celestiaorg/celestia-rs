//! Mempool-aware account sequences for the v12 in-process app.

use std::fmt;
use std::sync::Arc;

use celestia_proto::celestia_core_proto::abci::ResponseQuerySequence;
use sdk::{Context, ExecMode};

use crate::App;

pub(crate) struct PendingSequenceSource(Arc<x_ante::PendingNextSequence<'static>>);

impl fmt::Debug for PendingSequenceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingSequenceSource")
            .finish_non_exhaustive()
    }
}

impl App {
    /// Attaches a reader for the highest admitted CAT sequence. The reader
    /// must take only the transaction store lock, including during recheck.
    pub fn set_pending_sequencer(
        &mut self,
        highest: impl Fn(&[u8]) -> Option<u64> + Send + Sync + 'static,
    ) {
        self.pending_sequencer = Some(PendingSequenceSource(Arc::new(move |signer| {
            highest(signer)?.checked_add(1)
        })));
    }

    pub(crate) fn pending_next_sequence_reader(
        &self,
    ) -> Option<Arc<x_ante::PendingNextSequence<'static>>> {
        self.pending_sequencer
            .as_ref()
            .map(|source| source.0.clone())
    }

    /// `QuerySequence` (celestia-core mempool). From v12 with a local
    /// mempool, use the higher of the committed sequence and the next
    /// pending sequence. Otherwise read check state. A cosmos-sdk v0.46
    /// state (no mempool: CheckTx is not supported) answers 0.
    pub fn query_sequence(&mut self, signer: &[u8]) -> ResponseQuerySequence {
        if self.is_sdk_v046_state() {
            return ResponseQuerySequence { sequence: 0 };
        }
        if sdk::version::is_v12(self.execution_version())
            && self.last_height > 0
            && let Some(source) = &self.pending_sequencer
        {
            // Read pending first so a transaction removed after Commit is
            // visible in the committed state read below.
            let pending_next = (source.0)(signer);
            match x_auth::get_account_at_version(&self.ms, self.last_height, signer) {
                Ok(account) => {
                    let committed = account.map_or(0, |account| account.sequence());
                    return ResponseQuerySequence {
                        sequence: committed.max(pending_next.unwrap_or(0)),
                    };
                }
                Err(error) => {
                    tracing::error!(error = %error.log, "QuerySequence falls back to check state");
                }
            }
        }

        let mut ctx = Context::new(&mut self.ms, self.check_header.clone(), ExecMode::Check);
        ResponseQuerySequence {
            sequence: x_auth::get_account(&mut ctx, signer)
                .ok()
                .flatten()
                .map_or(0, |account| account.sequence()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::{AtomicU64, Ordering};

    use celestia_proto::cosmos::auth::v1beta1::BaseAccount;
    use sdk::Header;

    use super::*;

    const SIGNER: [u8; 20] = [1; 20];

    fn write_sequence(app: &mut App, mode: ExecMode, sequence: u64) {
        let mut ctx = Context::new(&mut app.ms, Header::default(), mode);
        let account = x_auth::Account::Base(BaseAccount {
            address: x_auth::bech32(&SIGNER).unwrap(),
            sequence,
            ..Default::default()
        });
        x_auth::set_account(&mut ctx, &account).unwrap();
    }

    fn commit(app: &mut App, height: i64) {
        app.finalize_header = Some(Header {
            height,
            ..Default::default()
        });
        app.commit().unwrap();
    }

    #[test]
    fn query_sequence_is_independent_of_recheck_and_uncommitted_state() {
        let mut app = App::new();
        app.execution_version = Some(12);
        write_sequence(&mut app, ExecMode::Check, 1);
        let highest_pending = Arc::new(AtomicU64::new(7));
        let pending = highest_pending.clone();
        app.set_pending_sequencer(move |signer| {
            (signer == SIGNER).then(|| pending.load(Ordering::Relaxed))
        });
        assert_eq!(app.query_sequence(&SIGNER).sequence, 1);

        write_sequence(&mut app, ExecMode::Finalize, 3);
        commit(&mut app, 1);
        write_sequence(&mut app, ExecMode::Check, 4);
        write_sequence(&mut app, ExecMode::Finalize, 10);
        app.ms.flush().unwrap();
        assert_eq!(app.query_sequence(&SIGNER).sequence, 8);
        assert_eq!(app.query_sequence(&[2; 20]).sequence, 0);

        highest_pending.store(u64::MAX, Ordering::Relaxed);
        assert_eq!(app.query_sequence(&SIGNER).sequence, 3);
        commit(&mut app, 2);
        assert_eq!(app.query_sequence(&SIGNER).sequence, 10);
    }

    #[test]
    fn reopened_app_uses_loaded_height_before_its_first_commit() {
        let db = Arc::new(crate::kvdb::MemBackend::new());
        let mut app = App::open_backend(db.clone(), &crate::AppOptions::default()).unwrap();
        write_sequence(&mut app, ExecMode::Finalize, 2);
        commit(&mut app, 1);
        drop(app);

        let mut app = App::open_backend(db, &crate::AppOptions::default()).unwrap();
        app.set_pending_sequencer(|signer| (signer == SIGNER).then_some(5));
        write_sequence(&mut app, ExecMode::Check, 3);
        assert_eq!(app.query_sequence(&SIGNER).sequence, 3);

        // Test the v12 hook against reopened state; release selection for a
        // real v12 executable is outside this fixture.
        app.execution_version = Some(12);
        assert_eq!(app.query_sequence(&SIGNER).sequence, 6);
    }

    #[test]
    fn v12_out_of_process_keeps_using_check_state() {
        let mut app = App::new();
        app.execution_version = Some(12);
        write_sequence(&mut app, ExecMode::Finalize, 3);
        commit(&mut app, 1);
        write_sequence(&mut app, ExecMode::Check, 4);
        assert_eq!(app.query_sequence(&SIGNER).sequence, 4);
    }
}
