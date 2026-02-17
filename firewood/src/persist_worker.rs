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
//! The main thread validates and updates in-memory state, then hands off the
//! committed revision to the background thread for disk I/O. A semaphore provides
//! backpressure when the background thread falls behind.
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
//!         Main->>BG: queue revision
//!         Note right of BG: Waiting...
//!     end
//!
//!     Caller->>Main: commit() (5th)
//!     Main->>BG: queue revision
//!     BG->>Disk: persist revision 5
//!     Note right of Disk: Sub-interval (10/2) reached
//!
//!     loop Commits 6-8
//!         Caller->>Main: commit()
//!         Main->>BG: queue revision
//!         Note right of BG: Waiting...
//!     end
//!
//!     Caller->>Main: close()
//!     Main->>BG: shutdown + persist last committed revision
//!     BG->>Disk: persist revision 8
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
use nonzero_ext::nonzero;
use parking_lot::{Condvar, Mutex, MutexGuard};

use crate::{manager::CommittedRevision, root_store::RootStore};
use crossbeam::channel::{self, Receiver, Sender};

use firewood_storage::logger::error;

/// Error type for persistence operations.
#[derive(Clone, Debug, thiserror::Error)]
pub enum PersistError {
    #[error("IO error during persistence: {0}")]
    FileIo(#[from] Arc<FileIoError>),
    #[error("RootStore error during persistence: {0}")]
    RootStore(#[source] Arc<dyn std::error::Error + Send + Sync>),
    #[error("Failed to send message after background thread channel disconnected")]
    ChannelDisconnected,
}

/// Message type that is sent to the background thread.
enum PersistMessage {
    /// A committed revision that may be persisted.
    Persist(CommittedRevision),
    /// A persisted revision to be reaped.
    Reap(NodeStore<Committed, FileBacked>),
}

/// Handle for managing the background persistence thread.
#[derive(Debug)]
pub(crate) struct PersistWorker {
    /// The background thread responsible for persisting commits async.
    handle: Mutex<Option<JoinHandle<Result<(), PersistError>>>>,

    /// Channel for sending messages to the background thread.
    ///
    /// Wrapped in `Option` so that `close()` can drop the sender to signal
    /// shutdown to the background thread via channel close.
    sender: Option<Sender<PersistMessage>>,

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
        let (sender, receiver) = channel::unbounded();
        let persist_interval = NonZeroU64::new(commit_count.get().div_ceil(2))
            .expect("a nonzero div_ceil(2) is always positive");

        let shared = Arc::new(SharedState {
            error: OnceLock::new(),
            commit_throttle: PersistSemaphore::new(commit_count),
            root_store,
            header: Mutex::new(header),
        });

        let persist_loop = PersistLoop {
            receiver,
            persist_interval,
            shared: shared.clone(),
            last_persisted_commit: None,
            last_committed_revision: None,
        };

        let handle = thread::spawn(move || persist_loop.run());

        Self {
            handle: Mutex::new(Some(handle)),
            sender: Some(sender),
            shared,
        }
    }

    /// Sends `committed` to the background thread for persistence. This call
    /// blocks if the limit of unpersisted commits has been reached.
    pub(crate) fn persist(&self, committed: CommittedRevision) -> Result<(), PersistError> {
        self.shared.commit_throttle.acquire();

        self.sender
            .as_ref()
            .ok_or(PersistError::ChannelDisconnected)?
            .send(PersistMessage::Persist(committed))
            .map_err(|_| self.resolve_worker_error())
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
            self.sender
                .as_ref()
                .ok_or(PersistError::ChannelDisconnected)?
                .send(PersistMessage::Reap(nodestore))
                .map_err(|_| self.resolve_worker_error())?;
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
    pub(crate) fn close(&mut self) -> Result<(), PersistError> {
        // Drop the sender to close the channel, signaling the background
        // thread to exit.
        drop(self.sender.take());

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

    /// Joins the background thread and returns the error that caused it to exit.
    ///
    /// Returns the stored error if one was set, or
    /// `PersistError::ChannelDisconnected` as a fallback if the thread exited
    /// cleanly without storing an error (e.g., `persist()` or `reap()` called
    /// after `close()`).
    ///
    /// # Panics
    ///
    /// Propagates the panic if the background thread panicked.
    fn resolve_worker_error(&self) -> PersistError {
        self.join_handle();
        self.check_error()
            .err()
            .unwrap_or(PersistError::ChannelDisconnected)
    }

    /// Wait until all pending commits have been persisted.
    #[cfg(test)]
    pub(crate) fn wait_persisted(&self) {
        self.shared.commit_throttle.wait_all_released();
    }
}

/// A semaphore for controlling the rate of commits relative to persistence.
///
/// Unlike standard semaphores where `acquire()` returns a guard that auto-releases:
/// - `acquire()` takes exactly 1 permit and blocks if none available
/// - `release(n)` returns `n` permits at once (called when persist completes)
///
/// This design allows the persist loop to release multiple permits at once
/// based on how many commits were persisted in a batch.
#[derive(Debug)]
struct PersistSemaphore {
    state: Mutex<u64>,
    condvar: Condvar,
    max_permits: NonZeroU64,
}

impl PersistSemaphore {
    /// Creates a new semaphore with `max_permits`.
    const fn new(max_permits: NonZeroU64) -> Self {
        Self {
            state: Mutex::new(max_permits.get()),
            condvar: Condvar::new(),
            max_permits,
        }
    }

    /// Acquires one permit. Blocks if no permits are available.
    #[inline]
    fn acquire(&self) {
        let mut permits = self.state.lock();
        while *permits == 0 {
            self.condvar.wait(&mut permits);
        }
        // The background loop guarantees permits > 0
        *permits = permits.saturating_sub(1);
    }

    /// Releases `count` permits back to the semaphore.
    ///
    /// The number of permits will not exceed `max_permits`.
    #[inline]
    fn release(&self, count: NonZeroU64) {
        let mut permits = self.state.lock();
        *permits = permits
            .saturating_add(count.get())
            .min(self.max_permits.get());
        self.condvar.notify_all();
    }

    /// Waits until all permits have been released back to the semaphore.
    #[cfg(test)]
    #[allow(clippy::arithmetic_side_effects)]
    fn wait_all_released(&self) {
        use firewood_storage::logger::warn;
        use std::time::Duration;

        const WARN_INTERVAL: Duration = Duration::from_secs(60);

        let mut permits = self.state.lock();
        let mut elapsed_secs = 0;
        while *permits < self.max_permits.get() {
            let result = self.condvar.wait_for(&mut permits, WARN_INTERVAL);
            if result.timed_out() && *permits < self.max_permits.get() {
                elapsed_secs += WARN_INTERVAL.as_secs();
                warn!("all permits have not been released back after {elapsed_secs}s");
            }
        }
    }
}

/// Shared state between `PersistWorker` and `PersistLoop` for coordination.
#[derive(Debug)]
struct SharedState {
    /// Shared error state that can be checked without joining the thread.
    error: OnceLock<PersistError>,
    /// Semaphore for limiting the number of unpersisted commits.
    commit_throttle: PersistSemaphore,
    /// Persisted metadata for the database.
    /// Updated on persists or when revisions are reaped.
    header: Mutex<NodeStoreHeader>,
    /// Optional persistent store for historical root addresses.
    root_store: Option<Arc<RootStore>>,
}

/// The background persistence loop that runs in a separate thread.
struct PersistLoop {
    /// Channel for receiving messages from `PersistWorker`.
    receiver: Receiver<PersistMessage>,
    /// Persist every `persist_interval` commits.
    persist_interval: NonZeroU64,
    /// Shared state for coordination with `PersistWorker`.
    shared: Arc<SharedState>,
    /// The commit number of the last successful persist (for calculating permits to release).
    last_persisted_commit: Option<NonZeroU64>,
    /// The last committed revision and its commit number.
    last_committed_revision: Option<(CommittedRevision, NonZeroU64)>,
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
        }
        result
    }

    /// Processes messages until the channel is closed or an error occurs.
    ///
    /// Upon receiving a message, this can do one of two things:
    /// - On `Reap`: drops the revision.
    ///   If persisted, the revision's nodes are added to the free lists only if
    ///   not running in archival mode.
    /// - On `Persist`: persists the revision if the number of revisions received
    ///   modulo `persist_interval` is zero.
    fn event_loop(&mut self) -> Result<(), PersistError> {
        let mut num_commits = nonzero!(1u64);

        while let Ok(message) = self.receiver.recv() {
            match message {
                PersistMessage::Reap(nodestore) => self.reap(nodestore)?,
                PersistMessage::Persist(revision) => {
                    if num_commits
                        .get()
                        .is_multiple_of(self.persist_interval.get())
                    {
                        self.persist(&revision, num_commits)?;
                    }

                    self.last_committed_revision = Some((revision, num_commits));
                    num_commits = num_commits.saturating_add(1);
                }
            }
        }

        if let Some((revision, last_commit_num)) = self.last_committed_revision.take()
            && Some(last_commit_num) != self.last_persisted_commit
        {
            self.persist(&revision, last_commit_num)?;
        }

        Ok(())
    }

    /// Persists the given revision and releases semaphore permits.
    fn persist(
        &mut self,
        revision: &CommittedRevision,
        num_commits: NonZeroU64,
    ) -> Result<(), PersistError> {
        // Persist the revision
        let mut header = self.shared.header.lock();
        let result = revision.persist(&mut header);
        drop(header);

        if let Err(e) = result {
            error!("Failed to persist revision: {e}");

            let err = PersistError::FileIo(Arc::new(e));
            // Release permits even on error to unblock waiting threads
            self.release_permits(num_commits);

            return Err(err);
        }

        // Save to root store if configured
        if let Err(e) = self.save_to_root_store(revision) {
            error!("Failed to persist revision address to RootStore: {e}");

            // Release permits even on error to unblock waiting threads
            self.release_permits(num_commits);
            return Err(e);
        }

        // Release permits for all commits that were persisted
        self.release_permits(num_commits);
        Ok(())
    }

    /// Releases semaphore permits for commits up to `num_commits`.
    fn release_permits(&mut self, num_commits: NonZeroU64) {
        let last = self.last_persisted_commit.map_or(0, NonZeroU64::get);
        let permits_to_release = num_commits
            .get()
            .checked_sub(last)
            .expect("should be positive");
        if let Some(count) = NonZeroU64::new(permits_to_release) {
            self.shared.commit_throttle.release(count);
            self.last_persisted_commit = Some(num_commits);
        }
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
            store
                .add_root(&hash, &addr)
                .map_err(|e| PersistError::RootStore(e.into()))?;
        }

        Ok(())
    }
}
