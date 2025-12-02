// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::ops::ControlFlow;
use std::os::fd::RawFd;
use std::path::PathBuf;

use hashbrown::{HashTable, hash_table::Entry as HashTableEntry};
use io_uring::IoUring;
use parking_lot::Mutex;

use crate::{FileIoError, firewood_counter, logger::trace};

/// The amount of time (in milliseconds) that the io-uring kernel thread
/// processing the submission queue should spin before idling to sleep.
///
/// A higher value increases CPU utilization when idle but reduces latency from
/// submissions to the queue. A lower value may increase the latency when
/// processing many small batches with gaps of idleness in between, but reduces
/// wasted CPU cycles from the busy wait loop.
const SUBMISSION_QUEUE_IDLE_TIME_MS: u32 = 1000;

/// The amount of entries in the shared submission queue.
///
/// The kernel copies entries from this queue to its own internal queue once it
/// begins processing them freeing us to add more entries while it processes
/// existing ones. This means the submission queue is effectively double this
/// size.
const SUBMISSION_QUEUE_SIZE: u32 = 32;

/// The amount of entries in the shared completion queue.
///
/// The kernel copies entries to this queue as operations complete, failure
/// or success. The kernel requires this queue to be strictly larger than the
/// submission queue and will round up to the next power of two if necessary.
///
/// We must ensure the completion queue is continuously worked to prevent the
/// kernel from stalling when it cannot push completed entries to it.
const COMPLETION_QUEUE_SIZE: u32 = SUBMISSION_QUEUE_SIZE * 2;

/// A thread-safe proxy around an io-uring instance.
pub struct IoUringProxy(Mutex<IoUring>);

/// A collection of errors encountered during an io-uring write batch.
///
/// A least one error occured for this to be returned. Incurable errors can be
/// observed via [`BatchErrors::incurable_error`].
#[must_use]
pub struct BatchErrors {
    errors: Vec<BatchError>,
}

/// An error encountered during an io-uring write operation.
pub struct BatchError {
    /// The offset of the original write operation that caused the error.
    ///
    /// This value is meaningful only if `incurable` is true.
    pub batch_offset: u64,

    /// The offset at which the error occurred, which may be different from
    /// `batch_offset` if a prior write operation partially succeeded.
    ///
    /// This value is meaningful only if `incurable` is false.
    pub error_offset: u64,

    /// The underlying I/O error.
    pub err: std::io::Error,

    /// Incurable errors are those that prevent us from communicating with the
    /// kernel via io-uring and force us to abort the entire batch immediately.
    ///
    /// The data is likely corrupted and should not be trusted. These errors
    /// likely should abort the process instead of being handled gracefully;
    /// however, we return them here so the abort can happen at a higher level.
    ///
    /// Curable errors are those that affect only a single write operation and
    /// may be recoverable, depending on the operation and context.
    incurable: bool,
}

/// The state of an ongoing io-uring write batch.
struct WriteBatch<'batch, 'ring, I> {
    /// The file descriptor all writes are applied to.
    ///
    /// There is no requirement that all entries write to the same file;
    /// however, this implementation assumes that is the case for simplicity.
    fd: RawFd,

    /// The io-uring submission interface.
    ///
    /// This is a higher-level interface for actually interacting with the
    /// kernel, after we have prepared our submission queue entries and after
    /// the kernel has processed some completion queue entries.
    submitter: io_uring::Submitter<'ring>,

    /// The kernel submission queue.
    ///
    /// The kernel keeps its own internal submission queue and copies entries
    /// from this queue to its own when it begins processing them. This allows
    /// us to continue adding entries to the queue while the kernel is busy.
    sq: io_uring::squeue::SubmissionQueue<'ring>,

    /// The kernel completion queue.
    ///
    /// We must continuously work this queue to prevent the kernel from stalling.
    cq: io_uring::cqueue::CompletionQueue<'ring>,

    /// A table of outstanding entries that have been submitted to the kernel
    /// but have not yet completed.
    ///
    /// A raw table is used here instead of a `HashMap` to avoid unnecessary
    /// indirection for the embedded key; and, to avoid unnecessary hashing
    /// because our keys are u64 offsets and do not require protection from
    /// denial-of-service attacks.
    outstanding: HashTable<QueueEntry<'batch>>,

    /// A backlog of entries that were taken from the `entries` iterator but
    /// could not be submitted to the kernel yet.
    ///
    /// This also contains entries for items that were partially written and
    /// need to be re-submitted.
    ///
    /// New entries are added to the back of the queue, while partially written
    /// entries are re-added to the front of the queue for priority processing.
    backlog: std::collections::VecDeque<QueueEntry<'batch>>,

    /// The undiscovered, unsubmitted entries in the batch.
    entries: I,

    /// The total number of bytes successfully written so far.
    ///
    /// An error is returned if the batch causes an overflow; however, the batch
    /// will continue processing all entries in the batch after such an error.
    written: usize,
}

/// An entry in the io-uring write batch.
#[derive(Debug, PartialEq)]
struct QueueEntry<'batch> {
    /// The original offset of the entry.
    original_offset: u64,

    /// The current offset of the entry.
    ///
    /// If different from `original_offset`, this entry has been partially written.
    offset: u64,

    /// A pointer to the current position in the buffer to write from.
    ///
    /// This is taken from the input and is adjusted as data is written. The
    /// pointer is guaranteed to be valid for the `'batch` lifetime and for
    /// `length` bytes. We do not directly dereference this pointer; however,
    /// it is passed to the kernel to copy data from.
    pointer: *const u8,

    /// The number of bytes remaining to write from the buffer.
    length: u32,

    /// ZST marker to bind the buffer's lifetime after replacing the slice with
    /// a raw pointer.
    lifetime: std::marker::PhantomData<&'batch [u8]>,
}

impl IoUringProxy {
    const fn from_ring(ring: IoUring) -> Self {
        Self(Mutex::new(ring))
    }

    pub fn new() -> std::io::Result<Self> {
        io_uring::IoUring::builder()
            // don't give the ring to child processes
            .dontfork()
            // Use a kernel thread to poll the submission queue. This avoids
            // context switching on submit; but, does add a significant
            // amount of overhead for small writes and after the thread sleeps.
            .setup_sqpoll(SUBMISSION_QUEUE_IDLE_TIME_MS)
            // do not stop processing the submission queue on error, submit
            // all events and report errors in the completion queue
            .setup_submit_all()
            .setup_single_issuer()
            // .setup_defer_taskrun()
            // completion queue must be greater than the submission queue
            // and is rounded up to the next power of two if necessary
            .setup_cqsize(COMPLETION_QUEUE_SIZE)
            .build(SUBMISSION_QUEUE_SIZE)
            .map(Self::from_ring)
    }

    /// Writes a batch of buffers to the given file descriptor at the specified
    /// offsets.
    ///
    /// We do not guarantee the ordering of writes in the batch; however, we
    /// do guarantee that all writes are attempted even if some writes fail;
    /// except if an incurable error occurs that prevents further communication
    /// with the kernel.
    ///
    /// If all writes succeed, the total number of bytes written is returned. If
    /// any writes fail, a collection of errors is returned. If an incurable
    /// error occurs, it will be indicated in the returned errors.
    pub fn write_batch<'a>(
        &self,
        fd: RawFd,
        writes: impl IntoIterator<Item = (u64, &'a [u8])>,
    ) -> Result<usize, BatchErrors> {
        let mut errors = Vec::<BatchError>::new();
        macro_rules! bail {
            ($reason:expr) => {{
                errors.push(BatchError {
                    batch_offset: 0,
                    error_offset: 0,
                    err: $reason,
                    incurable: true,
                });
                // guaranteed non-empty since we just pushed the incurable error
                return Err(BatchErrors { errors });
            }};
        }

        trace!("starting io-uring write batch");
        let mut ring = self.0.lock();
        let mut batch = WriteBatch::new(&mut ring, fd, writes.into_iter().map(QueueEntry::new));

        batch.seed_submission_queue();

        while !batch.is_finished() {
            trace!(
                "io-uring write batch status: outstanding={}, backlog={}, sq_len={}, sq_full={}, cq_len={}, cq_full={}, iter_size_hint={:?}",
                batch.outstanding.len(),
                batch.backlog.len(),
                batch.sq.len(),
                batch.sq.is_full(),
                batch.cq.len(),
                batch.cq.is_full(),
                batch.entries.size_hint(),
            );

            if let Err(err) = batch.sync_completion_queue() {
                bail!(err);
            }

            if let Err(err) = batch.flush_submission_queue() {
                bail!(err);
            }

            batch.flush_completion_queue(&mut errors);
        }

        if errors.is_empty() {
            Ok(batch.written)
        } else {
            Err(BatchErrors { errors })
        }
    }
}

impl<'a> QueueEntry<'a> {
    fn new((offset, buffer): (u64, &'a [u8])) -> Self {
        // trivial check because we never allocate nodes larger than 16 MiB (AREA_SIZES)
        debug_assert!(u32::try_from(buffer.len()).is_ok());
        Self {
            original_offset: offset,
            offset,
            pointer: buffer.as_ptr(),
            length: buffer.len() as u32,
            lifetime: std::marker::PhantomData,
        }
    }

    fn build_submission_queue_entry(&self, fd: RawFd) -> io_uring::squeue::Entry {
        io_uring::opcode::Write::new(io_uring::types::Fd(fd), self.pointer, self.length)
            .offset(self.offset)
            .build()
            .user_data(self.original_offset)
    }

    /// Handles the result of a completed write operation.
    ///
    /// If the operation was successful, returns the number of bytes written. If
    /// the operation failed, returns a `BatchError` describing the failure.
    fn handle_result(&mut self, res: i32) -> Result<usize, BatchError> {
        #![expect(clippy::arithmetic_side_effects, clippy::cast_sign_loss)]

        if res < 0 {
            let err = std::io::Error::from_raw_os_error(-res);
            // rust stdlib maps EAGIN to WouldBlock; re-submit in that case
            if matches!(err.kind(), std::io::ErrorKind::WouldBlock) {
                trace!(
                    "io-uring write at offset {} returned EAGIN, re-submitting",
                    self.original_offset
                );
                firewood_counter!(
                    "ring.eagin_write_retry",
                    "amount of io-uring write entries that have been re-submitted due to EAGIN io error"
                ).increment(1);
                return Ok(0);
            }

            return Err(BatchError {
                batch_offset: self.original_offset,
                error_offset: self.offset,
                err,
                incurable: false,
            });
        }

        // This check is to prevent infinite loops. The kernel should never
        // return Ok(0) for a write operation via this API unless the storage
        // driver is broken (or we weren't writing to a file--how did we get here?)
        if res == 0 {
            return Err(BatchError {
                batch_offset: self.original_offset,
                error_offset: self.offset,
                err: std::io::Error::new(std::io::ErrorKind::WriteZero, "kernel wrote zero bytes"),
                incurable: false,
            });
        }

        let written = res as u32;
        if written > self.length {
            return Err(BatchError {
                batch_offset: self.original_offset,
                error_offset: self.offset,
                err: std::io::Error::other("kernel wrote more data than requested"),
                incurable: false,
            });
        }

        // if this is non-zero, we need to re-submit the remaining data
        self.length -= written;
        self.pointer = self.pointer.wrapping_add(written as usize);
        self.offset += u64::from(written);

        Ok(written as usize)
    }

    const fn hashtable_hash(&self) -> u64 {
        // use the original offset as the hash to avoid unnecessary hashing
        self.original_offset
    }
}

impl<'batch, 'ring, I: Iterator<Item = QueueEntry<'batch>>> WriteBatch<'batch, 'ring, I> {
    fn new(ring: &'ring mut IoUring, fd: RawFd, entries: I) -> Self {
        let (submitter, sq, cq) = ring.split();
        Self {
            fd,
            submitter,
            sq,
            cq,
            outstanding: HashTable::with_capacity(COMPLETION_QUEUE_SIZE as usize),
            backlog: std::collections::VecDeque::new(),
            entries,
            written: 0,
        }
    }

    fn is_finished(&self) -> bool {
        // we never take from the iterator without submitting it to the kernel
        // or adding it to the backlog, so we don't need to check the iterator
        self.backlog.is_empty() && self.outstanding.is_empty()
    }

    fn hashtable_insert(&mut self, entry: QueueEntry<'batch>) {
        self.outstanding
            .insert_unique(entry.hashtable_hash(), entry, QueueEntry::hashtable_hash);
    }

    fn hashtable_remove(&mut self, offset: u64) -> Option<QueueEntry<'batch>> {
        match self.outstanding.entry(
            offset,
            |needle| needle.original_offset == offset,
            QueueEntry::hashtable_hash,
        ) {
            HashTableEntry::Occupied(entry) => {
                let (entry, _) = entry.remove();
                Some(entry)
            }
            HashTableEntry::Vacant(_) => None,
        }
    }

    fn pop_next_entry(&mut self) -> Option<QueueEntry<'batch>> {
        if let Some(entry) = self.backlog.pop_front() {
            Some(entry)
        } else {
            self.entries.next()
        }
    }

    fn add_entry_to_backlog(&mut self, entry: QueueEntry<'batch>) {
        if entry.offset == entry.original_offset {
            // new entry, queue at the back
            self.backlog.push_back(entry);
        } else {
            // partially written entry, re-queue at the front for priority
            self.backlog.push_front(entry);
        }
    }

    fn sync_completion_queue(&mut self) -> std::io::Result<()> {
        trace!("syncing io-uring completion queue");
        match self.submitter.submit_and_wait(1) {
            Ok(_) => {}
            Err(ref err) if ignore_poll_error(err) => {}
            Err(err) => return Err(err),
        }

        self.cq.sync();

        Ok(())
    }

    fn enqueue_single_op(&mut self) -> ControlFlow<()> {
        let Some(entry) = self.pop_next_entry() else {
            return ControlFlow::Break(());
        };

        // debug_assert_eq so it `Debug` prints the entry on failure
        debug_assert_eq!(
            self.hashtable_remove(entry.original_offset),
            None,
            "attempting to enqueue duplicate io-uring write entry at offset {}",
            entry.original_offset,
        );

        let sqe = entry.build_submission_queue_entry(self.fd);
        #[expect(unsafe_code)]
        // SAFETY: rust borrowing rules ensure the buffer lives for `'batch`
        // lifetime which must outlive the `write_batch` method that created
        // this `WriteBatch` instance. We also ensure that the method does
        // not return until all entries have been processed by the kernel.
        if unsafe { self.sq.push(&sqe) }.is_err() {
            self.add_entry_to_backlog(entry);
            trace!("io-uring submission queue is full");
            firewood_counter!(
                "ring.full",
                "amount of io-uring full submission queue breakpoints"
            )
            .increment(1);
            ControlFlow::Break(())
        } else {
            self.hashtable_insert(entry);
            ControlFlow::Continue(())
        }
    }

    fn seed_submission_queue(&mut self) {
        while self.enqueue_single_op().is_continue() {}
        self.sq.sync();
    }

    fn flush_submission_queue(&mut self) -> std::io::Result<()> {
        trace!("flushing io-uring submission queue");
        loop {
            if self.sq.is_full() {
                match self.submitter.submit() {
                    Ok(_) => {}
                    Err(ref err) if ignore_poll_error(err) => {
                        // kernel is busy, move onto completions
                        return Ok(());
                    }
                    Err(err) => return Err(err),
                }
            }
            self.sq.sync();

            if self.enqueue_single_op().is_break() {
                return Ok(());
            }
        }
    }

    fn flush_completion_queue(&mut self, errors: &mut Vec<BatchError>) {
        trace!("flushing io-uring completion queue");
        while let Some(entry) = self.cq.next() {
            let offset = entry.user_data();
            let Some(mut write_entry) = self.hashtable_remove(offset) else {
                errors.push(BatchError {
                    batch_offset: offset,
                    error_offset: offset,
                    err: std::io::Error::other("completion event for unknown offset"),
                    incurable: false,
                });
                continue;
            };
            match write_entry.handle_result(entry.result()) {
                Ok(written) => {
                    // copy offset off the entry before adding it back to the backlog
                    let error_offset = write_entry.offset;
                    if write_entry.length > 0 {
                        // not fully written, re-queue
                        self.add_entry_to_backlog(write_entry);
                        if written != 0 {
                            // if zero, we would have already logged EAGIN above
                            trace!(
                                "io-uring write at offset {offset} partially completed, re-queuing"
                            );
                            firewood_counter!(
                                "ring.partial_write_retry",
                                "amount of io-uring write entries that have been re-submitted due to partial writes"
                            ).increment(1);
                        }
                    }
                    if let Some(total_written) = self.written.checked_add(written) {
                        self.written = total_written;
                    } else {
                        errors.push(BatchError {
                            batch_offset: offset,
                            error_offset,
                            err: std::io::Error::other("overflow when summing total bytes written"),
                            incurable: false,
                        });
                    }
                }
                Err(err) => {
                    errors.push(err);
                }
            }
        }
    }
}

impl std::fmt::Debug for IoUringProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoUringProxy").finish_non_exhaustive()
    }
}

impl BatchError {
    pub const fn is_incurable(&self) -> bool {
        self.incurable
    }
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.batch_offset == self.error_offset {
            write!(f, "{} at offset {}", self.err, self.batch_offset)
        } else {
            write!(
                f,
                "{} at offset {} (after writing up to offset {})",
                self.err, self.error_offset, self.batch_offset
            )
        }
    }
}

impl BatchErrors {
    pub fn into_file_io_error(self, path: Option<PathBuf>) -> FileIoError {
        if self.errors.len() == 1 {
            let only = self.errors.into_iter().next().expect("just checked length");
            FileIoError::new(
                only.err,
                path,
                only.error_offset,
                Some("io_uring write error".to_string()),
            )
        } else {
            FileIoError::new(
                std::io::Error::other(self),
                path,
                0,
                Some("multiple io_uring write errors".to_string()),
            )
        }
    }

    pub fn errors(&self) -> &[BatchError] {
        &self.errors
    }

    pub fn incurable_error(&self) -> Option<&std::io::Error> {
        // if we have an incurable error, it will always be the last one added
        self.errors()
            .last()
            .and_then(|e| if e.is_incurable() { Some(&e.err) } else { None })
    }
}

impl std::fmt::Debug for BatchErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for BatchErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::error::Error;
        match self.errors() {
            [] => write!(f, "no errors (this is unexpected)"),
            [only] => std::fmt::Display::fmt(&only, f),
            [
                rest @ ..,
                incurable @ BatchError {
                    incurable: true, ..
                },
            ] if f.alternate() => {
                writeln!(
                    f,
                    "Encountered an incurable error after other errors: {incurable}"
                )?;
                for err in ErrorChain(incurable.err.source()) {
                    writeln!(f, "  Caused by: {err}")?;
                }
                writeln!(f, "Other errors:")?;
                for (i, err) in rest.iter().enumerate() {
                    writeln!(f, "  {}: {err}", i.wrapping_add(1))?;
                    for err in ErrorChain(err.err.source()) {
                        writeln!(f, "    Caused by: {err}")?;
                    }
                }
                Ok(())
            }
            [
                rest @ ..,
                incurable @ BatchError {
                    incurable: true, ..
                },
            ] => {
                write!(
                    f,
                    "Encountered an incurable error after {} other errors: {incurable}",
                    rest.len()
                )
            }
            [first, rest @ ..] if f.alternate() => {
                writeln!(f, "Multiple errors encountered:")?;
                for (i, err) in std::iter::once(first).chain(rest.iter()).enumerate() {
                    writeln!(f, "  {}: {err}", i.wrapping_add(1))?;
                    for err in ErrorChain(err.err.source()) {
                        writeln!(f, "    Caused by: {err}")?;
                    }
                }
                Ok(())
            }
            [first, rest @ ..] => {
                write!(f, "{first} (and {} more errors)", rest.len())
            }
        }
    }
}

impl std::error::Error for BatchErrors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // if we have an incurable error, prefer that as the source
        self.incurable_error().map(|e| e as &_).or_else(|| {
            // Source only lets us return one error, so return the first.
            self.errors.first().map(|e| &e.err as &_)
        })
    }
}

struct ErrorChain<'a>(Option<&'a (dyn std::error::Error + 'static)>);

impl<'a> std::iter::Iterator for ErrorChain<'a> {
    type Item = &'a (dyn std::error::Error + 'static);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.take().inspect(|err| self.0 = err.source())
    }
}

fn ignore_poll_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ResourceBusy
    )
}
