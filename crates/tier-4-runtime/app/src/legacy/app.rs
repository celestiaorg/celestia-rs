//! [`App`] on a cosmos-sdk v0.46 state (celestia-app v1 to v3, as the
//! multiplexer runs v3.13.0 for them).
//!
//! A genesis at app version 1, or a database whose latest commit info is
//! a v0.46 release's store set ([`super::release_of`]), mounts that
//! release's stores ([`crate::stores::mount_release`]) and routes
//! InitChain, FinalizeBlock and Commit to the [`Driver`]. A reopened
//! driver takes the chain ID from the first block header. A genesis must
//! be at app version 1; blocks execute at app versions 1 to 3, through
//! the v1 to v2 upgrade (`App::set_v2_upgrade_height`, or the chain's
//! default) and the signalled upgrades. A state whose app version became
//! 4 hands off to the cosmos-sdk v0.50 app at the block of the scheduled
//! `v4` plan (`crate::handoff`); without that plan it stops with an error.
//!
//! celestia-rs replays committed v0.46 blocks only. CheckTx,
//! PrepareProposal, ProcessProposal and queries fail with a
//! not-supported error rather than running v0.50 code on v0.46 state, and
//! no state-sync snapshot is taken of a v0.46 state. A fresh app restores
//! a Go v0.46 snapshot (format 2) of app version 1 to 3 into that
//! release's store set ([`App::legacy_offer_snapshot`]) and continues on
//! the driver from the snapshot height.

use celestia_proto::celestia_core_proto::abci::{
    self, ResponseCommit, ResponseFinalizeBlock, ResponseInitChain,
};
use sdk::version::Release;
use sdk::{Header, Result, SdkError};

use super::{Driver, Modules, V1Modules};
use crate::{App, CHAIN_ID_KEY, store_err, stores};

/// The modules an [`App`]'s v0.46 driver runs.
pub(crate) type AppModules = Box<dyn Modules + Send>;

/// The v0.46 driver of an [`App`].
pub(crate) type AppDriver = Driver<AppModules>;

/// The app version 1 modules (the default of every [`App`]).
pub(crate) fn v1_modules() -> AppModules {
    Box::new(V1Modules)
}

/// `what` is not supported on a v0.46 state at `app_version`.
pub(crate) fn not_supported(what: &str, app_version: u64) -> SdkError {
    sdk::error::sdk::not_supported(format!(
        "{what} at app version {app_version} (cosmos-sdk v0.46 state): celestia-rs only replays committed blocks of app versions 1 to 3"
    ))
}

impl App {
    /// Whether this app holds a cosmos-sdk v0.46 state (app versions 1 to
    /// 3), executed by the legacy driver.
    #[must_use]
    pub fn is_sdk_v046_state(&self) -> bool {
        self.legacy.is_some()
    }

    /// The error of an entry point that a v0.46 state does not support,
    /// or `None` on a v0.50 state.
    pub(crate) fn v046_unsupported(&self, what: &str) -> Option<SdkError> {
        self.legacy
            .as_ref()
            .map(|_| not_supported(what, super::app_version(&self.ms)))
    }

    /// Selects the modules of the v0.46 driver (tests).
    #[cfg(test)]
    pub(crate) fn set_legacy_modules(&mut self, modules: fn() -> AppModules) {
        self.legacy_modules = modules;
        if self.legacy.is_some() {
            self.legacy = Some(self.new_legacy_driver());
        }
    }

    /// A driver for the committed v0.46 state: it expects the stored chain
    /// ID, or takes it from the next block header when none is stored.
    pub(crate) fn new_legacy_driver(&self) -> AppDriver {
        let mut d = Driver::new((self.legacy_modules)());
        d.expect_chain_id(&self.chain_id);
        if let Some(h) = self.v2_upgrade_height {
            d.set_upgrade_height_v2(h);
        }
        d
    }

    /// A fresh driver after the state was reopened or rolled back.
    pub(crate) fn reset_legacy_driver(&mut self) {
        if self.legacy.is_some() {
            self.legacy = Some(self.new_legacy_driver());
        }
    }

    /// InitChain of a genesis at v0.46 app version `app_version`: mounts
    /// the release's stores in place of the v0.50 ones (nothing is
    /// committed yet) and runs the driver's InitChain.
    pub(crate) fn legacy_init_chain(
        &mut self,
        req: &abci::RequestInitChain,
        app_version: u64,
    ) -> Result<ResponseInitChain> {
        if !super::executes(app_version) {
            return Err(not_supported("a genesis", app_version));
        }
        if self.last_height != 0 {
            return Err(sdk::error::sdk::logic(format!(
                "InitChain on a state committed at height {}",
                self.last_height
            )));
        }
        let release = Release::for_app_version(app_version)
            .ok_or_else(|| sdk::error::sdk::logic("unknown app version"))?;
        let remounted = self.legacy.is_none();
        if remounted {
            for name in stores::IAVL_STORES {
                if self.ms.is_mounted(name) {
                    self.ms.unmount_iavl(name).map_err(store_err)?;
                }
            }
            stores::mount_release(&mut self.ms, release).map_err(store_err)?;
        }
        let mut driver = Driver::new((self.legacy_modules)());
        if let Some(h) = self.v2_upgrade_height {
            driver.set_upgrade_height_v2(h);
        }
        // A failed InitChain leaves no writes (the driver discards them)
        // and, on a fresh app, the v0.50 stores mounted again.
        let res = match driver.init_chain(&mut self.ms, req) {
            Ok(res) => res,
            Err(e) => {
                if remounted {
                    self.restore_v050_mounts(release)?;
                }
                return Err(e);
            }
        };
        self.chain_id.clone_from(&req.chain_id);
        self.ms
            .root_mut()
            .set_metadata(CHAIN_ID_KEY, req.chain_id.clone().into_bytes());
        self.legacy = Some(driver);
        self.check_header = Header {
            chain_id: req.chain_id.clone(),
            time: req.time.unwrap_or_default(),
            ..Header::default()
        };
        Ok(res)
    }

    /// Undoes [`Self::legacy_init_chain`]'s mounts: `release`'s stores
    /// out, the v0.50 stores in (none holds a committed version).
    fn restore_v050_mounts(&mut self, release: &Release) -> Result<()> {
        for name in release.stores.iter().chain(release.commit_memory_stores) {
            self.ms.unmount_iavl(name).map_err(store_err)?;
        }
        for (name, kind) in stores::all() {
            self.ms.mount(name, kind).map_err(store_err)?;
        }
        Ok(())
    }

    /// `OfferSnapshot` of a snapshot at v0.46 app version
    /// `req.app_version` (or while such a restore is active): only a
    /// fresh app accepts it, and only in format 2. Accepting mounts the
    /// release's stores in place of the v0.50 ones, as a v0.46 genesis
    /// does; a later offer replaces an unfinished restore.
    pub(crate) fn legacy_offer_snapshot(
        &mut self,
        req: abci::RequestOfferSnapshot,
    ) -> abci::ResponseOfferSnapshot {
        use abci::response_offer_snapshot::Result as R;
        let result = |r: R| abci::ResponseOfferSnapshot { result: r as i32 };
        if let Some(release) = self.snapshot_v046.take() {
            if let Some(m) = &mut self.snapshots
                && let Err(e) = m.abort_restore()
            {
                tracing::error!(error = %e, "failed to abort snapshot restore");
            }
            if let Err(e) = self.restore_v050_mounts(release) {
                tracing::error!(error = %e.log, "failed to restore the v0.50 stores");
                return result(R::Abort);
            }
        }
        let Some(release) = super::is_v046(req.app_version)
            .then(|| Release::for_app_version(req.app_version))
            .flatten()
        else {
            tracing::error!(
                app_version = req.app_version,
                "rejecting snapshot: app version {} has no cosmos-sdk v0.46 release",
                req.app_version
            );
            return result(R::Reject);
        };
        if self.last_height != 0 || self.legacy.is_some() {
            tracing::error!("rejecting snapshot: the app already holds a state");
            return result(R::Reject);
        }
        let (Some(manager), Some(s)) = (&mut self.snapshots, req.snapshot) else {
            return result(R::Abort);
        };
        if s.format != snapshots::FORMAT_V046 {
            // Only this snapshot: the format may still be valid for later
            // heights offered in the same state sync.
            tracing::error!(
                format = s.format,
                app_version = req.app_version,
                "rejecting snapshot: a cosmos-sdk v0.46 state is restored from format 2"
            );
            return result(R::Reject);
        }
        let snapshot =
            match snapshots::Snapshot::from_abci(s.height, s.format, s.chunks, s.hash, &s.metadata)
            {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    tracing::error!(error = %e, "rejecting invalid snapshot");
                    return result(R::Reject);
                }
            };
        if let Err(e) = manager.begin_restore(snapshot) {
            tracing::error!(error = %e, "failed to restore snapshot");
            return result(R::Abort);
        }
        for name in stores::IAVL_STORES {
            if self.ms.is_mounted(name)
                && let Err(e) = self.ms.unmount_iavl(name)
            {
                tracing::error!(store = name, error = %e, "failed to select snapshot stores");
                return result(R::Abort);
            }
        }
        if let Err(e) = stores::mount_release(&mut self.ms, release) {
            tracing::error!(error = %e, "failed to select snapshot stores");
            return result(R::Abort);
        }
        self.snapshot_v046 = Some(release);
        tracing::info!(
            app_version = req.app_version,
            "restoring a cosmos-sdk v0.46 snapshot"
        );
        result(R::Accept)
    }

    /// `ApplySnapshotChunk` of a v0.46 restore. After the last chunk the
    /// app holds the release's committed state at the snapshot height and
    /// continues on the v0.46 driver, as when it reopens such a state; the
    /// capability memory store is rebuilt by the first block's BeginBlock.
    pub(crate) fn legacy_apply_snapshot_chunk(
        &mut self,
        req: abci::RequestApplySnapshotChunk,
    ) -> abci::ResponseApplySnapshotChunk {
        use abci::response_apply_snapshot_chunk::Result as R;
        let respond = |r: R| abci::ResponseApplySnapshotChunk {
            result: r as i32,
            ..Default::default()
        };
        let Some(release) = self.snapshot_v046 else {
            return respond(R::Abort);
        };
        let Some(manager) = &mut self.snapshots else {
            return respond(R::Abort);
        };
        match manager.restore_chunk(self.ms.root_mut(), &req.chunk) {
            Ok(None) => respond(R::Accept),
            Ok(Some(id)) => {
                self.snapshot_v046 = None;
                if let Err(e) = self.load_committed_state() {
                    tracing::error!(error = %e.log, "failed to load restored state");
                    return respond(R::Abort);
                }
                let mut driver = self.new_legacy_driver();
                if let Err(e) = driver.init_memory_stores(&mut self.ms) {
                    tracing::error!(error = %e.log, "failed to rebuild the memory stores");
                    return respond(R::Abort);
                }
                self.legacy = Some(driver);
                self.check_header = self.init_check_header();
                tracing::info!(
                    height = id.version,
                    app_version = super::app_version(&self.ms),
                    "restored cosmos-sdk v0.46 state-sync snapshot"
                );
                respond(R::Accept)
            }
            Err(snapshots::Error::ChunkHashMismatch(_)) => abci::ResponseApplySnapshotChunk {
                result: R::Retry as i32,
                refetch_chunks: vec![req.index],
                reject_senders: vec![req.sender],
            },
            Err(e) => {
                self.snapshot_v046 = None;
                if let Err(cleanup) = manager.abort_restore() {
                    tracing::error!(error = %cleanup, "failed to abort snapshot restore");
                }
                if let Err(cleanup) = self.restore_v050_mounts(release) {
                    tracing::error!(error = %cleanup.log, "failed to restore the v0.50 stores");
                }
                tracing::error!(error = %e, "failed to restore snapshot");
                respond(R::Abort)
            }
        }
    }

    /// FinalizeBlock on a v0.46 state.
    pub(crate) fn legacy_finalize_block(
        &mut self,
        req: &abci::RequestFinalizeBlock,
    ) -> Result<ResponseFinalizeBlock> {
        if self.last_height > 0 {
            // App versions 1 to 3 execute here; the v4 upgrade (app
            // version 4 in the consensus params) runs on the v0.50 app.
            let v = super::app_version(&self.ms);
            if !super::is_v046(v) {
                return Err(not_supported("block execution", v));
            }
        }
        let driver = self
            .legacy
            .as_mut()
            .ok_or_else(|| sdk::error::sdk::logic("no cosmos-sdk v0.46 driver"))?;
        let res = driver.finalize_block(&mut self.ms, req)?;
        if self.chain_id.is_empty()
            && let Some(h) = &req.header
        {
            self.chain_id.clone_from(&h.chain_id);
        }
        self.finalize_header = Some(Header {
            chain_id: self.chain_id.clone(),
            height: req.height,
            time: req.time.unwrap_or_default(),
            proposer_address: req.proposer_address.clone(),
            hash: req.hash.clone(),
            app_hash: self.last_commit_hash.clone(),
            next_validators_hash: req.next_validators_hash.clone(),
        });
        Ok(res)
    }

    /// Commit on a v0.46 state. No state-sync snapshot is taken: the
    /// snapshot format of a v0.46 state is not supported.
    pub(crate) fn legacy_commit(&mut self) -> Result<ResponseCommit> {
        let header = self
            .finalize_header
            .take()
            .ok_or_else(|| sdk::error::sdk::logic("commit without finalized block"))?;
        let driver = self
            .legacy
            .as_mut()
            .ok_or_else(|| sdk::error::sdk::logic("no cosmos-sdk v0.46 driver"))?;
        self.ms
            .root_mut()
            .set_commit_timestamp(crate::multistore_timestamp(&header.time));
        let id = driver.commit(&mut self.ms)?;
        self.proposed = crate::proposal::ProposedSquare::default();
        self.last_commit_hash = id.hash.to_vec();
        self.last_height = id.version;
        self.check_header = header;
        if let (Some(m), Ok(h)) = (&self.snapshots, u64::try_from(id.version))
            && m.should_snapshot(h)
        {
            tracing::debug!(
                height = h,
                "no state-sync snapshot of a cosmos-sdk v0.46 state"
            );
        }
        Ok(ResponseCommit {
            retain_height: self.block_retention_height(id.version),
        })
    }
}

#[cfg(test)]
mod tests;
