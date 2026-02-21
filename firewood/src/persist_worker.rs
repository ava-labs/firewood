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
            root_store,
            header: Mutex::new(header),
            persist_on_shutdown: OnceLock::new(),
            channel: PersistChannel::new(commit_count, persist_threshold),
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
        self.shared.channel.push(committed)
    }

    /// Sends `nodestore` to the background thread for reaping if archival mode
    /// is disabled. Otherwise, the `nodestore` is dropped.
    pub(crate) fn reap(
        &self,
        nodestore: NodeStore<Committed, FileBacked>,
    ) -> Result<(), PersistError> {
        if self.shared.root_store.is_none() {
            self.shared.channel.reap(nodestore)?;
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

        self.shared.channel.close();
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

#[derive(Debug)]
struct PersistChannel<T> {
    state: Mutex<PersistChannelState<T>>,
    /// Condition variable to wake blocked committers when space is available.
    commit_not_full: Condvar,
    /// Condition variable to wake the persister when persistence is needed.
    persist_ready: Condvar,
}

impl<T> PersistChannel<T> {
    const fn new(max_permits: NonZeroU64, persist_threshold: u64) -> Self {
        Self {
            state: Mutex::new(PersistChannelState {
                permits_available: max_permits.get(),
                max_permits,
                persist_threshold,
                shutdown: false,
                pending_reaps: Vec::new(),
                data: None,
            }),
            commit_not_full: Condvar::new(),
            persist_ready: Condvar::new(),
        }
    }

    fn empty(&self) -> bool {
        let state = self.state.lock();
        state.permits_available == state.max_permits.get()
    }

    /// Reaping is only performed as part of the next persist operation
    fn reap(&self, nodestore: NodeStore<Committed, FileBacked>) -> Result<(), PersistError> {
        let mut state = self.state.lock();
        if state.shutdown {
            return Err(PersistError::ChannelDisconnected);
        }
        state.pending_reaps.push(nodestore);
        // Note that there is no need to call notify here since reaps
        // are only performed as part of a persists.
        Ok(())
    }

    fn push(&self, item: T) -> Result<(), PersistError> {
        let mut state = self.state.lock();
        while state.permits_available == 0 {
            self.commit_not_full.wait(&mut state);
        }

        if state.shutdown {
            return Err(PersistError::ChannelDisconnected);
        }

        state.data = Some(item);
        state.permits_available = state.permits_available.saturating_sub(1);

        // Wake the persister once we reach the threshold (or on any reap).
        if state.permits_available <= state.persist_threshold {
            self.persist_ready.notify_one();
        }
        Ok(())
    }

    fn pop(&self) -> Result<PersistDataWrapper<'_, T>, PersistError> {
        let mut state = self.state.lock();
        // Sleep until we have reached the required threshold of pending commits
        // Only performs reaping the next time that we persist.
        //
        // NOTE: The is None check is unnecessary if there is only one
        //       thread performing the
        while !state.shutdown
            && (state.permits_available > state.persist_threshold || state.data.is_none())
        {
            self.persist_ready.wait(&mut state);
        }

        if state.shutdown {
            return Err(PersistError::ChannelDisconnected);
        }

        Ok(PersistDataWrapper {
            channel: self,
            permits_to_release: state
                .max_permits
                .get()
                .saturating_sub(state.permits_available),
            pending_reaps: std::mem::take(&mut state.pending_reaps),
            data: state.data.take(),
        })
    }

    fn close(&self) {
        let mut state = self.state.lock();
        state.shutdown = true;
        self.persist_ready.notify_all();
        self.commit_not_full.notify_all();
    }

    #[cfg(test)]
    #[allow(clippy::arithmetic_side_effects)]
    fn wait_all_released(&self) {
        use firewood_storage::logger::warn;
        use std::time::Duration;

        const WARN_INTERVAL: Duration = Duration::from_secs(60);

        let mut state = self.state.lock();
        let mut elapsed_secs = 0;
        while state.permits_available < state.max_permits.get() {
            let result = self.commit_not_full.wait_for(&mut state, WARN_INTERVAL);
            if result.timed_out() && state.permits_available < state.max_permits.get() {
                elapsed_secs += WARN_INTERVAL.as_secs();
                warn!("all permits have not been released back after {elapsed_secs}s");
            }
        }
    }
}

#[derive(Debug)]
struct PersistChannelState<T> {
    permits_available: u64,
    /// Maximum number of unpersisted commits allowed.
    max_permits: NonZeroU64,
    /// Persist when remaining permits are at or below this threshold.
    persist_threshold: u64,
    shutdown: bool,
    pending_reaps: Vec<NodeStore<Committed, FileBacked>>,
    data: Option<T>,
}

struct PersistDataWrapper<'a, T> {
    channel: &'a PersistChannel<T>,
    permits_to_release: u64,
    pending_reaps: Vec<NodeStore<Committed, FileBacked>>,
    data: Option<T>,
}

impl<T> Drop for PersistDataWrapper<'_, T> {
    /// Handle permit release on drop of the wrapper
    fn drop(&mut self) {
        if self.permits_to_release == 0 {
            return;
        }

        let mut state = self.channel.state.lock();
        state.permits_available = state
            .permits_available
            .saturating_add(self.permits_to_release)
            .min(state.max_permits.get());

        self.channel.commit_not_full.notify_all();
    }
}

/// Shared state between `PersistWorker` and `PersistLoop` for coordination.
#[derive(Debug)]
struct SharedState {
    /// Shared error state that can be checked without joining the thread.
    error: OnceLock<PersistError>,
    /// Persisted metadata for the database.
    /// Updated on persists or when revisions are reaped.
    header: Mutex<NodeStoreHeader>,
    /// Optional persistent store for historical root addresses.
    root_store: Option<Arc<RootStore>>,
    /// Unpersisted revision to persist on shutdown.
    persist_on_shutdown: OnceLock<CommittedRevision>,
    /// Channel for coordinating persist and reap operations.
    channel: PersistChannel<CommittedRevision>,
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
            self.shared.channel.close();
        }
        result
    }

    /// Processes pending work until shutdown or an error occurs.
    fn event_loop(&mut self) -> Result<(), PersistError> {
        loop {
            let Ok(mut persist_data) = self.shared.channel.pop() else {
                break; // Channel failed due to a shutdown. Break to handle shutdown gracefully.
            };

            for nodestore in std::mem::take(&mut persist_data.pending_reaps) {
                self.reap(nodestore)?;
            }

            if let Some(revision) = persist_data.data.take() {
                self.persist_to_disk(&revision)
                    .and_then(|()| self.save_to_root_store(&revision))?;
            }
        }

        // Persist the last unpersisted revision on shutdown
        if let Some(revision) = self.shared.persist_on_shutdown.get().cloned()
            && !self.shared.channel.empty()
        {
            self.persist_to_disk(&revision)
                .and_then(|()| self.save_to_root_store(&revision))?;
        }

        Ok(())
    }

    /// Persists the revision to disk.
    fn persist_to_disk(&self, revision: &CommittedRevision) -> Result<(), PersistError> {
        let mut header = self.shared.header.lock();
        revision.persist(&mut header).map_err(|e| {
            error!("Failed to persist revision: {e}");
            PersistError::FileIo(Arc::new(e))
        })
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
        self.channel.wait_all_released();
    }
}
