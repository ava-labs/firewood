// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Implementation of a background thread responsible for persisting revisions.

use derive_where::derive_where;
use std::{
    sync::{Arc, OnceLock},
    thread::{self, JoinHandle},
};

use firewood_storage::{
    Committed, FileBacked, FileIoError, HashedNodeReader, NodeStore, NodeStoreHeader, RootReader,
};
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
    #[error("Background thread panicked")]
    ThreadPanic,
    #[error("Failed to send message to background thread: channel disconnected")]
    ChannelDisconnected,
}

enum PersistMessage {
    /// A committed revision that may be persisted.
    Persist(CommittedRevision),
    /// A persisted revision to be reaped.
    Reap(NodeStore<Committed, FileBacked>),
    /// The background thread should shutdown.
    Shutdown,
}

/// Handle for managing the background persistence thread.
///
/// `PersistWorker` is the main-thread interface for:
/// - Checking for errors from the background thread
/// - Implementing backpressure via the `PersistSemaphore`.
/// - Waiting for graceful shutdown
#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub(crate) struct PersistWorker {
    /// The background thread responsible for persisting commits async.
    handle: Mutex<Option<JoinHandle<Result<(), PersistError>>>>,

    sender: Sender<PersistMessage>,

    /// Shared state with background thread.
    shared: Arc<SharedState>,
}

impl PersistWorker {
    /// Creates a new `PersistWorker` and starts the background thread.
    ///
    /// Returns the worker and a sender for sending messages to the background thread.
    ///
    /// # Arguments
    ///
    /// * `commit_count` - Maximum number of commits that can be replayed on crash.
    ///   The worker will persist every `commit_count / 2` commits.
    /// * `header` - Shared header for persistence operations.
    /// * `root_store` - Optional root store for saving root addresses.
    #[allow(clippy::large_types_passed_by_value)]
    pub(crate) fn new(
        commit_count: usize,
        header: NodeStoreHeader,
        root_store: Option<Arc<RootStore>>,
    ) -> Self {
        let (sender, receiver) = channel::unbounded();
        // Persist every sub_interval commits. With commit_count=1, persist every commit.
        let sub_interval = (commit_count / 2).max(1);

        let shared = Arc::new(SharedState {
            error: OnceLock::new(),
            semaphore: PersistSemaphore::new(commit_count),
            latest_committed: Mutex::new(None),
            root_store,
            header: Mutex::new(header),
        });

        let persist_loop = PersistLoop {
            receiver,
            sub_interval,
            shared: shared.clone(),
            last_persisted_commit: 0,
        };

        let handle = thread::spawn(move || persist_loop.run());

        Self {
            handle: Mutex::new(Some(handle)),
            sender,
            shared,
        }
    }

    /// Persist this revision to disk.
    pub(crate) fn persist(&self, committed: CommittedRevision) -> Result<(), PersistError> {
        // Acquire a permit before sending. Blocks if we've hit the limit of
        // unpersisted commits.
        self.shared.semaphore.acquire();

        // Check for errors after potentially blocking
        self.check_error()?;

        // Track the latest committed revision for use in close()
        *self.shared.latest_committed.lock() = Some(committed.clone());

        self.sender
            .send(PersistMessage::Persist(committed))
            .map_err(|_| PersistError::ChannelDisconnected)
    }

    /// Reap a revision's deleted nodes.
    pub(crate) fn reap(
        &self,
        nodestore: NodeStore<Committed, FileBacked>,
    ) -> Result<(), PersistError> {
        let is_persisted = nodestore
            .root_as_maybe_persisted_node()
            .is_some_and(|node| node.unpersisted().is_none());

        if is_persisted {
            self.sender
                .send(PersistMessage::Reap(nodestore))
                .map_err(|_| PersistError::ChannelDisconnected)?;
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
    pub(crate) fn close(&self) -> Result<(), PersistError> {
        // Signal the background thread to shutdown before waiting for it.
        // This is necessary because the thread is blocked on recv(), which will
        // only unblock when a message is sent or the channel is closed.
        //
        // If the send fails, the background thread has already exited unexpectedly
        // (due to error or panic). We'll still try to join to get the actual error.
        let send_failed = self.sender.send(PersistMessage::Shutdown).is_err();

        // Wait for the background thread to finish and return its result.
        match self.handle.lock().take() {
            Some(handle) => handle.join().unwrap_or(Err(PersistError::ThreadPanic))?,
            None if send_failed => {
                // Thread exited and handle was already taken - check for stored error
                // or report channel disconnection
                self.check_error()
                    .and(Err(PersistError::ChannelDisconnected))?;
            }
            None => self.check_error()?,
        }

        // Persist the latest committed revision to ensure no data is lost.
        if let Some(committed) = self.shared.latest_committed.lock().as_ref() {
            let has_unpersisted = committed
                .root_as_maybe_persisted_node()
                .is_some_and(|node| node.unpersisted().is_some());

            if has_unpersisted {
                let mut header = self.shared.header.lock();
                committed
                    .persist(&mut header)
                    .map_err(|e| PersistError::FileIo(e.into()))?;

                // Save to root store if configured
                if let Some(store) = &self.shared.root_store
                    && let (Some(hash), Some(address)) =
                        (committed.root_hash(), committed.root_address())
                {
                    store
                        .add_root(&hash, &address)
                        .map_err(|e| PersistError::RootStore(e.into()))?;
                }
            }
        }

        Ok(())
    }

    /// Wait until all pending commits have been persisted.
    #[cfg(test)]
    pub(crate) fn wait_persisted(&self) {
        self.shared.semaphore.wait_all_released();
    }
}

/// A semaphore for controlling the rate of commits relative to persistence.
///
/// Unlike standard semaphores where acquire returns a guard that auto-releases:
/// - `acquire()` takes exactly 1 permit and blocks if none available
/// - `release(n)` returns `n` permits at once (called when persist completes)
///
/// This design allows the persist loop to release multiple permits at once
/// based on how many commits were persisted in a batch.
struct PersistSemaphore {
    state: Mutex<usize>,
    condvar: Condvar,
    max_permits: usize,
}

impl PersistSemaphore {
    /// Creates a new semaphore with the given number of permits.
    const fn new(max_permits: usize) -> Self {
        Self {
            state: Mutex::new(max_permits),
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
        // Safe: loop guarantees permits > 0
        *permits = permits.saturating_sub(1);
    }

    /// Releases `count` permits back to the semaphore.
    ///
    /// The number of permits will not exceed the maximum configured at creation.
    #[inline]
    fn release(&self, count: usize) {
        if count == 0 {
            return;
        }
        let mut permits = self.state.lock();
        *permits = permits.saturating_add(count).min(self.max_permits);
        self.condvar.notify_all();
    }

    /// Waits until all permits have been released back to the semaphore.
    #[cfg(test)]
    fn wait_all_released(&self) {
        let mut permits = self.state.lock();
        while *permits < self.max_permits {
            self.condvar.wait(&mut permits);
        }
    }
}

/// Shared state between `PersistWorker` and `PersistLoop` for coordination.
///
/// Wrapped in a single `Arc` to minimize allocations and reference count operations
/// when cloning, and to improve cache locality.
struct SharedState {
    /// Shared error state that can be checked without joining the thread.
    error: OnceLock<PersistError>,
    /// Semaphore for backpressure - limits unpersisted commits.
    semaphore: PersistSemaphore,
    /// The latest committed revision, used by `close()` to persist any remaining data.
    latest_committed: Mutex<Option<CommittedRevision>>,
    header: Mutex<NodeStoreHeader>,
    root_store: Option<Arc<RootStore>>,
}

/// The background persistence loop that runs in a separate thread.
///
/// This struct encapsulates all the state and logic needed to:
/// - Receive committed revisions from the channel
/// - Persist revisions at regular intervals
/// - Release semaphore permits when persistence completes
struct PersistLoop {
    /// Channel for receiving messages.
    receiver: Receiver<PersistMessage>,
    /// Persist every `sub_interval` commits.
    sub_interval: usize,
    /// Shared state for coordination with `PersistWorker`.
    shared: Arc<SharedState>,
    /// The commit number of the last successful persist (for calculating permits to release).
    last_persisted_commit: usize,
}

impl PersistLoop {
    /// Runs the persistence loop until shutdown or error.
    ///
    /// This method:
    /// 1. Waits for messages from the channel
    /// 2. On `Commit`: tracks the revision and persists every `sub_interval` commits
    /// 3. On `Shutdown` or channel close: exits gracefully
    fn run(mut self) -> Result<(), PersistError> {
        let mut num_commits = 0usize;

        loop {
            // An error indicates that the channel is closed.
            let message = self.receiver.recv().unwrap_or(PersistMessage::Shutdown);

            match message {
                PersistMessage::Shutdown => return Ok(()),
                PersistMessage::Reap(nodestore) => self.reap(nodestore)?,
                PersistMessage::Persist(revision) => {
                    num_commits = num_commits.wrapping_add(1);

                    if num_commits.is_multiple_of(self.sub_interval) {
                        self.persist(&revision, num_commits)?;
                    }
                }
            }
        }
    }

    /// Persists the given revision and releases semaphore permits.
    fn persist(
        &mut self,
        revision: &CommittedRevision,
        num_commits: usize,
    ) -> Result<(), PersistError> {
        // Persist the revision
        let mut header = self.shared.header.lock();
        let result = revision.persist(&mut header);
        drop(header);

        if let Err(e) = result {
            error!("Failed to persist revision: {e}");

            let err = PersistError::FileIo(Arc::new(e));
            self.shared.error.set(err.clone()).expect("should be empty");
            // Release permits even on error to unblock waiting threads
            self.release_permits(num_commits);

            return Err(err);
        }

        // Save to root store if configured
        if let Err(e) = self.save_to_root_store(revision) {
            self.shared.error.set(e.clone()).expect("should be empty");
            // Release permits even on error to unblock waiting threads
            self.release_permits(num_commits);
            return Err(e);
        }

        // Release permits for all commits that were persisted
        self.release_permits(num_commits);
        Ok(())
    }

    /// Releases semaphore permits for commits up to `num_commits`.
    fn release_permits(&mut self, num_commits: usize) {
        let permits_to_release = num_commits.saturating_sub(self.last_persisted_commit);
        self.shared.semaphore.release(permits_to_release);
        self.last_persisted_commit = num_commits;
    }

    /// Note: no need to check if we're running in archival mode as this should
    /// be the responsibility of the message sender.
    fn reap(&self, nodestore: NodeStore<Committed, FileBacked>) -> Result<(), PersistError> {
        if let Err(e) = nodestore.reap_deleted(&mut self.shared.header.lock()) {
            let err = PersistError::FileIo(Arc::new(e));
            self.shared.error.set(err.clone()).expect("should be empty");
            return Err(err);
        }

        Ok(())
    }

    /// Saves the revision's root address to the root store if configured.
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
