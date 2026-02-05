// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{
    num::NonZeroU64,
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

/// Message type that is sent to the background thread.
enum PersistMessage {
    /// A committed revision that may be persisted.
    Persist(CommittedRevision),
    /// A persisted revision to be reaped.
    Reap(NodeStore<Committed, FileBacked>),
    /// The background thread should shutdown.
    Shutdown,
}

/// Handle for managing the background persistence thread.
#[derive(Debug)]
pub(crate) struct PersistWorker {
    /// The background thread responsible for persisting commits async.
    handle: Mutex<Option<JoinHandle<Result<(), PersistError>>>>,

    /// Channel for sending messages to the background thread.
    sender: Sender<PersistMessage>,

    /// Shared state with background thread.
    shared: Arc<SharedState>,
}

impl PersistWorker {
    /// Creates a new `PersistWorker` and starts the background thread.
    ///
    /// Returns the worker for sending messages to the background thread.
    #[allow(clippy::large_types_passed_by_value)]
    pub(crate) fn new(header: NodeStoreHeader, root_store: Option<Arc<RootStore>>) -> Self {
        let (sender, receiver) = channel::unbounded();
        let persist_interval = NonZeroU64::new(1).expect("should be nonzero");

        let shared = Arc::new(SharedState {
            error: OnceLock::new(),
            semaphore: PersistSemaphore::new(persist_interval),
            root_store,
            header: Mutex::new(header),
        });

        let persist_loop = PersistLoop {
            receiver,
            persist_interval,
            shared: shared.clone(),
            last_persisted_commit: None,
        };

        let handle = thread::spawn(move || persist_loop.run());

        Self {
            handle: Mutex::new(Some(handle)),
            sender,
            shared,
        }
    }

    /// Sends `committed` to the background thread for persistence. This call
    /// blocks if the limit of unpersisted commits has been reached.
    pub(crate) fn persist(&self, committed: CommittedRevision) -> Result<(), PersistError> {
        self.shared.semaphore.acquire();

        // Check for errors after potentially blocking
        self.check_error()?;

        self.sender
            .send(PersistMessage::Persist(committed))
            .map_err(|_| PersistError::ChannelDisconnected)
    }

    /// Sends `nodestore` to the background thread for reaping if persisted and
    /// if archival mode is disabled. Otherwise, the `nodestore` is dropped.
    pub(crate) fn reap(
        &self,
        nodestore: NodeStore<Committed, FileBacked>,
    ) -> Result<(), PersistError> {
        let is_persisted = nodestore
            .root_as_maybe_persisted_node()
            .is_some_and(|node| node.unpersisted().is_none());

        if is_persisted && self.shared.root_store.is_none() {
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
    ///
    /// This method is idempotent for compatibility with `drop` (as `drop` is
    /// called after this function).
    pub(crate) fn close(&self) -> Result<(), PersistError> {
        // Signal to the background thread to exit.
        // We ignore any errors here as the background thread may already have exited.
        let _ = self.sender.send(PersistMessage::Shutdown);

        // Wait for the background thread to finish and return its result.
        match self.handle.lock().take() {
            Some(handle) => handle.join().unwrap_or(Err(PersistError::ThreadPanic)),
            None => self.check_error(),
        }
    }

    /// Wait until all pending commits have been persisted.
    #[cfg(test)]
    pub(crate) fn wait_persisted(&self) {
        self.shared.semaphore.wait_all_released();
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
    fn wait_all_released(&self) {
        let mut permits = self.state.lock();
        while *permits < self.max_permits.get() {
            self.condvar.wait(&mut permits);
        }
    }
}

/// Shared state between `PersistWorker` and `PersistLoop` for coordination.
#[derive(Debug)]
struct SharedState {
    /// Shared error state that can be checked without joining the thread.
    error: OnceLock<PersistError>,
    /// Semaphore for limiting the number of unpersisted commits.
    semaphore: PersistSemaphore,
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
}

impl PersistLoop {
    /// Runs the persistence loop until shutdown or error.
    ///
    /// Upon receiving a message, `run()` can do one of three things:
    /// - On `Shutdown` or channel close: exits gracefully.
    /// - On `Reap`: drops the revision
    ///   If persisted, the revision's nodes are added to the free lists only if
    ///   not running in archival mode.
    /// - On `Commit`: persists the revision if the number of revisions recieved
    ///   modulo `persist_interval` is zero.
    fn run(mut self) -> Result<(), PersistError> {
        let mut num_commits = NonZeroU64::new(1).expect("should be nonzero");

        loop {
            // An error indicates that the channel is closed.
            let message = self.receiver.recv().unwrap_or(PersistMessage::Shutdown);

            match message {
                PersistMessage::Shutdown => return Ok(()),
                PersistMessage::Reap(nodestore) => self.reap(nodestore)?,
                PersistMessage::Persist(revision) => {
                    if num_commits
                        .get()
                        .is_multiple_of(self.persist_interval.get())
                    {
                        self.persist(&revision, num_commits)?;
                    }

                    num_commits = num_commits.saturating_add(1);
                }
            }
        }
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
            self.shared.error.set(err.clone()).expect("should be empty");
            // Release permits even on error to unblock waiting threads
            self.release_permits(num_commits);

            return Err(err);
        }

        // Save to root store if configured
        if let Err(e) = self.save_to_root_store(revision) {
            error!("Failed to persist revision address to RootStore: {e}");

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
    fn release_permits(&mut self, num_commits: NonZeroU64) {
        let last = self.last_persisted_commit.map_or(0, NonZeroU64::get);
        let permits_to_release = num_commits.get().saturating_sub(last);
        if let Some(count) = NonZeroU64::new(permits_to_release) {
            self.shared.semaphore.release(count);
            self.last_persisted_commit = Some(num_commits);
        }
    }

    /// Add the nodes of this revision to the free lists.
    fn reap(&self, nodestore: NodeStore<Committed, FileBacked>) -> Result<(), PersistError> {
        if let Err(e) = nodestore.reap_deleted(&mut self.shared.header.lock()) {
            let err = PersistError::FileIo(Arc::new(e));
            self.shared.error.set(err.clone()).expect("should be empty");
            return Err(err);
        }

        Ok(())
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
