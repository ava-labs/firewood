// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Deferred persistence for committed revisions.
//!
//! This module decouples commit operations from disk I/O by offloading persistence
//! to a background thread. Commits return immediately after updating in-memory state,
//! while disk writes happen asynchronously.
//!
//! [`PersistWorker`] is the main entry point. It spawns a background thread and provides
//! methods to send revisions for persistence, with built-in backpressure to limit
//! the number of unpersisted commits.
//!
//! The diagram below shows how commits are handled under deferred persistence.
//! The main thread updates shared state and signals the background thread via
//! condition variables. Backpressure is enforced by waiting when all permits
//! are exhausted.
//!
//! Below is an example when `commit_count` is set to 10:
//!
//! ```mermaid
//! sequenceDiagram
//!     participant Caller
//!     participant Main as Main Thread
//!     participant BG as Background Thread
//!     participant Disk
//!
//!     loop Commits 1-4
//!         Caller->>Main: commit()
//!         Main->>Main: store latest revision, count -= 1
//!         Note right of BG: Waiting (count > threshold)
//!     end
//!
//!     Caller->>Main: commit() (5th)
//!     Main->>Main: store latest revision, count -= 1
//!     Main->>BG: notify persist_ready
//!     BG->>Disk: persist latest revision
//!     Note right of Disk: Sub-interval (10/2) reached
//!
//!     loop Commits 6-8
//!         Caller->>Main: commit()
//!         Main->>Main: store latest revision, count -= 1
//!         Note right of BG: Waiting (count > threshold)
//!     end
//!
//!     Caller->>Main: close()
//!     Main->>BG: shutdown signal
//!     BG->>Disk: persist last committed revision
//!     Note right of Disk: Latest committed revision is persisted
//! ```

use std::{
    num::NonZeroU64,
    panic::resume_unwind,
    sync::{Arc, OnceLock},
    thread::{self, JoinHandle},
};

use firewood_storage::{
    Committed, FileBacked, FileIoError, HashedNodeReader, NodeStore, NodeStoreHeader,
};
use parking_lot::{Condvar, Mutex, MutexGuard};

use crate::{manager::CommittedRevision, root_store::RootStore};

use firewood_storage::logger::error;

/// Error type for persistence operations.
#[derive(Clone, Debug, thiserror::Error)]
pub enum PersistError {
    #[error("IO error during persistence: {0}")]
    FileIo(#[from] Arc<FileIoError>),
    #[error("RootStore error during persistence: {0}")]
    RootStore(#[source] Arc<dyn std::error::Error + Send + Sync>),
    #[error("Failed to signal background thread after shutdown")]
    ChannelDisconnected,
}

/// Handle for managing the background persistence thread.
#[derive(Debug)]
pub(crate) struct PersistWorker {
    /// The background thread responsible for persisting commits async.
    handle: Mutex<Option<JoinHandle<Result<(), PersistError>>>>,

    /// Shared state with background thread.
    shared: Arc<SharedState>,
}

impl PersistWorker {
    /// Creates a new `PersistWorker` and starts the background thread.
    ///
    /// Returns the worker for sending messages to the background thread.
    #[allow(clippy::large_types_passed_by_value)]
    pub(crate) fn new(
        commit_count: NonZeroU64,
        header: NodeStoreHeader,
        root_store: Option<Arc<RootStore>>,
    ) -> Self {
        let persist_interval = NonZeroU64::new(commit_count.get().div_ceil(2))
            .expect("a nonzero div_ceil(2) is always positive");
        let persist_threshold = commit_count.get().saturating_sub(persist_interval.get());

        let shared = Arc::new(SharedState {
            error: OnceLock::new(),
            commit_not_full: Condvar::new(),
            persist_ready: Condvar::new(),
            max_permits: commit_count,
            persist_threshold,
            root_store,
            header: Mutex::new(header),
            persist_on_shutdown: OnceLock::new(),
            state: Mutex::new(PersistState {
                count: commit_count.get(),
                latest_revision: None,
                pending_reaps: Vec::new(),
                shutdown: false,
            }),
        });

        let persist_loop = PersistLoop {
            shared: shared.clone(),
        };

        let handle = thread::spawn(move || persist_loop.run());

        Self {
            handle: Mutex::new(Some(handle)),
            shared,
        }
    }

    /// Sends `committed` to the background thread for persistence. This call
    /// blocks if the limit of unpersisted commits has been reached.
    pub(crate) fn persist(&self, committed: CommittedRevision) -> Result<(), PersistError> {
        let mut state = self.shared.state.lock();
        // Backpressure: block only when all permits are exhausted.
        while state.count == 0 && !state.shutdown {
            self.shared.commit_not_full.wait(&mut state);
        }

        if state.shutdown {
            return Err(PersistError::ChannelDisconnected);
        }

        state.latest_revision = Some(committed);
        state.count = state.count.saturating_sub(1);

        // Wake the persister once we reach the threshold (or on any reap).
        if state.count <= self.shared.persist_threshold {
            self.shared.persist_ready.notify_one();
        }

        Ok(())
    }

    /// Sends `nodestore` to the background thread for reaping if archival mode
    /// is disabled. Otherwise, the `nodestore` is dropped.
    pub(crate) fn reap(
        &self,
        nodestore: NodeStore<Committed, FileBacked>,
    ) -> Result<(), PersistError> {
        if self.shared.root_store.is_none() {
            // Always send the reap message, even for empty tries. Even if this
            // revision wasn't persisted, it could be the case that this
            // revision carries deleted nodes from a previously persisted revision.
            let mut state = self.shared.state.lock();
            if state.shutdown {
                return Err(PersistError::ChannelDisconnected);
            }

            state.pending_reaps.push(nodestore);
            self.shared.persist_ready.notify_one();
        }

        Ok(())
    }

    /// Get a lock to the header of the database.
    pub(crate) fn locked_header(&self) -> MutexGuard<'_, NodeStoreHeader> {
        self.shared.header.lock()
    }

    /// Check if the persist worker has errored.
    pub(crate) fn check_error(&self) -> Result<(), PersistError> {
        match self.shared.error.get() {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }

    /// Close the persist worker.
    ///
    /// This method is idempotent for compatibility with `drop` (as `drop` is
    /// called after this function).
    pub(crate) fn close(
        &mut self,
        latest_committed_revision: CommittedRevision,
    ) -> Result<(), PersistError> {
        // Ignore the error if already set.
        let _ = self
            .shared
            .persist_on_shutdown
            .set(latest_committed_revision);

        {
            let mut state = self.shared.state.lock();
            state.shutdown = true;
            self.shared.persist_ready.notify_one();
            self.shared.commit_not_full.notify_all();
        }

        self.join_handle();
        self.check_error()
    }

    /// Joins the background thread if the handle is still available.
    ///
    /// This is a no-op if the handle was already taken (e.g., by a prior call
    /// to `close()`), which guarantees idempotency.
    ///
    /// # Panics
    ///
    /// Propagates the panic if the background thread panicked.
    fn join_handle(&self) {
        if let Some(handle) = self.handle.lock().take()
            && let Err(payload) = handle.join()
        {
            resume_unwind(payload);
        }
    }

    /// Wait until all pending commits have been persisted.
    #[cfg(test)]
    pub(crate) fn wait_persisted(&self) {
        self.shared.wait_all_released();
    }
}

/// Shared state between `PersistWorker` and `PersistLoop` for coordination.
#[derive(Debug)]
struct SharedState {
    /// Shared error state that can be checked without joining the thread.
    error: OnceLock<PersistError>,
    /// Condition variable to wake blocked committers when space is available.
    commit_not_full: Condvar,
    /// Condition variable to wake the persister when persistence is needed.
    persist_ready: Condvar,
    /// Maximum number of unpersisted commits allowed.
    max_permits: NonZeroU64,
    /// Persist when remaining permits are at or below this threshold.
    persist_threshold: u64,
    /// Persisted metadata for the database.
    /// Updated on persists or when revisions are reaped.
    header: Mutex<NodeStoreHeader>,
    /// Optional persistent store for historical root addresses.
    root_store: Option<Arc<RootStore>>,
    /// Unpersisted revision to persist on shutdown.
    persist_on_shutdown: OnceLock<CommittedRevision>,
    /// Coordination state for commit and persist operations.
    state: Mutex<PersistState>,
}

#[derive(Debug)]
struct PersistState {
    count: u64,
    latest_revision: Option<CommittedRevision>,
    pending_reaps: Vec<NodeStore<Committed, FileBacked>>,
    shutdown: bool,
}

/// The background persistence loop that runs in a separate thread.
struct PersistLoop {
    /// Shared state for coordination with `PersistWorker`.
    shared: Arc<SharedState>,
}

impl PersistLoop {
    /// Runs the persistence loop until shutdown or error.
    ///
    /// If the event loop exits with an error, it is stored in shared state
    /// so the main thread can observe it without joining.
    fn run(mut self) -> Result<(), PersistError> {
        let result = self.event_loop();
        if let Err(ref err) = result {
            self.shared.error.set(err.clone()).expect("should be empty");
            let mut state = self.shared.state.lock();
            state.shutdown = true;
            self.shared.persist_ready.notify_one();
            self.shared.commit_not_full.notify_all();
        }
        result
    }

    /// Processes pending work until shutdown or an error occurs.
    fn event_loop(&mut self) -> Result<(), PersistError> {
        loop {
            let (revision, reaps, permits_to_release, shutting_down) = {
                let mut state = self.shared.state.lock();
                // Sleep until there is work to do (reaps or a pending persist), or shutdown.
                while !state.shutdown
                    && state.pending_reaps.is_empty()
                    && (state.count > self.shared.persist_threshold
                        || state.latest_revision.is_none())
                {
                    self.shared.persist_ready.wait(&mut state);
                }

                let reaps = std::mem::take(&mut state.pending_reaps);
                let should_persist =
                    state.count <= self.shared.persist_threshold && state.latest_revision.is_some();
                let revision = if should_persist {
                    state.latest_revision.take()
                } else {
                    None
                };
                let permits_to_release = if should_persist {
                    // Release all permits for unpersisted commits observed at this point.
                    self.shared.max_permits.get().saturating_sub(state.count)
                } else {
                    0
                };
                let shutting_down = state.shutdown;

                (revision, reaps, permits_to_release, shutting_down)
            };

            for nodestore in reaps {
                self.reap(nodestore)?;
            }

            if let Some(revision) = revision {
                self.persist_and_release(&revision, permits_to_release)?;
            }

            if shutting_down {
                break;
            }
        }

        let outstanding_commits = {
            let state = self.shared.state.lock();
            self.shared.max_permits.get().saturating_sub(state.count)
        };

        // Persist the last unpersisted revision on shutdown
        if let Some(revision) = self.shared.persist_on_shutdown.get().cloned()
            && outstanding_commits > 0
        {
            self.persist_and_release(&revision, outstanding_commits)?;
        }

        Ok(())
    }

    /// Performs the actual persistence and releases semaphore permits.
    fn persist_and_release(
        &self,
        revision: &CommittedRevision,
        permits_to_release: u64,
    ) -> Result<(), PersistError> {
        let result = self
            .persist_to_disk(revision)
            .and_then(|()| self.save_to_root_store(revision));
        self.release_permits(permits_to_release);
        result
    }

    /// Persists the revision to disk.
    fn persist_to_disk(&self, revision: &CommittedRevision) -> Result<(), PersistError> {
        let mut header = self.shared.header.lock();
        revision.persist(&mut header).map_err(|e| {
            error!("Failed to persist revision: {e}");
            PersistError::FileIo(Arc::new(e))
        })
    }

    /// Releases permits for commits since last persist if any.
    fn release_permits(&self, permits_to_release: u64) {
        if permits_to_release == 0 {
            return;
        }

        let mut state = self.shared.state.lock();
        state.count = state
            .count
            .saturating_add(permits_to_release)
            .min(self.shared.max_permits.get());
        self.shared.commit_not_full.notify_all();
    }

    /// Add the nodes of this revision to the free lists.
    fn reap(&self, nodestore: NodeStore<Committed, FileBacked>) -> Result<(), PersistError> {
        nodestore
            .reap_deleted(&mut self.shared.header.lock())
            .map_err(|e| PersistError::FileIo(Arc::new(e)))
    }

    /// Saves the revision's root address to `RootStore` if configured.
    fn save_to_root_store(&self, revision: &CommittedRevision) -> Result<(), PersistError> {
        if let Some(ref store) = self.shared.root_store
            && let (Some(hash), Some(addr)) = (revision.root_hash(), revision.root_address())
        {
            store.add_root(&hash, &addr).map_err(|e| {
                error!("Failed to persist revision address to RootStore: {e}");
                PersistError::RootStore(e.into())
            })?;
        }

        Ok(())
    }
}

impl SharedState {
    #[cfg(test)]
    #[allow(clippy::arithmetic_side_effects)]
    fn wait_all_released(&self) {
        use firewood_storage::logger::warn;
        use std::time::Duration;

        const WARN_INTERVAL: Duration = Duration::from_secs(60);

        let mut state = self.state.lock();
        let mut elapsed_secs = 0;
        while state.count < self.max_permits.get() {
            let result = self.commit_not_full.wait_for(&mut state, WARN_INTERVAL);
            if result.timed_out() && state.count < self.max_permits.get() {
                elapsed_secs += WARN_INTERVAL.as_secs();
                warn!("all permits have not been released back after {elapsed_secs}s");
            }
        }
    }
}
