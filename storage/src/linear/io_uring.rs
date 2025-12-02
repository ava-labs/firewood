// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::os::fd::RawFd;
use std::path::PathBuf;

use io_uring::IoUring;
use parking_lot::Mutex;

use crate::{FileIoError, firewood_counter, logger::trace};

const SUBMISSION_QUEUE_IDLE_TIME_MS: u32 = 1000;

const SUBMISSION_QUEUE_SIZE: u32 = 32;

const COMPLETION_QUEUE_SIZE: u32 = SUBMISSION_QUEUE_SIZE * 2;

pub struct IoUringProxy(Mutex<IoUring>);

pub struct Errors {
    errors: Vec<Error>,
}

struct Error {
    offset: u64,
    err: std::io::Error,
}

struct WriteBatch<'batch, 'ring, I> {
    fd: RawFd,
    submitter: io_uring::Submitter<'ring>,
    sq: io_uring::squeue::SubmissionQueue<'ring>,
    cq: io_uring::cqueue::CompletionQueue<'ring>,
    outstanding: HashMap<u64, QueueEntry<'batch>>,
    backlog: std::collections::VecDeque<QueueEntry<'batch>>,
    entries: I,
    written: usize,
}

struct QueueEntry<'batch> {
    original_offset: u64,
    offset: u64,
    pointer: *const u8,
    length: u32,
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

    pub fn write_batch<'a>(
        &self,
        fd: RawFd,
        writes: impl IntoIterator<Item = (u64, &'a [u8])>,
    ) -> Result<usize, Errors> {
        let mut errors = Vec::<Error>::new();
        macro_rules! bail {
            ($reason:expr) => {{
                errors.push(Error {
                    offset: 0,
                    err: $reason,
                });
                return Err(Errors { errors });
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
            Err(Errors { errors })
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

    fn build_sqe(&self, fd: RawFd) -> io_uring::squeue::Entry {
        io_uring::opcode::Write::new(io_uring::types::Fd(fd), self.pointer, self.length)
            .offset(self.offset)
            .build()
            .user_data(self.original_offset)
    }

    fn handle_result(&mut self, res: i32) -> Result<usize, Error> {
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

            return Err(Error {
                offset: self.original_offset,
                err,
            });
        }

        let written = res as u32;
        if written > self.length {
            return Err(Error {
                offset: self.original_offset,
                err: std::io::Error::other("kernel wrote more data than requested"),
            });
        }

        // if this is non-zero, we need to re-submit the remaining data
        self.length -= written;
        self.pointer = self.pointer.wrapping_add(written as usize);
        self.offset += u64::from(written);

        Ok(written as usize)
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
            outstanding: HashMap::with_capacity(COMPLETION_QUEUE_SIZE as usize),
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

        let sqe = entry.build_sqe(self.fd);
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
            self.outstanding.insert(entry.original_offset, entry);
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

    fn flush_completion_queue(&mut self, errors: &mut Vec<Error>) {
        trace!("flushing io-uring completion queue");
        while let Some(entry) = self.cq.next() {
            let offset = entry.user_data();
            let Some(mut write_entry) = self.outstanding.remove(&offset) else {
                errors.push(Error {
                    offset,
                    err: std::io::Error::other("completion event for unknown offset"),
                });
                continue;
            };
            match write_entry.handle_result(entry.result()) {
                Ok(written) => {
                    if write_entry.length > 0 {
                        // not fully written, re-queue
                        self.add_entry_to_backlog(write_entry);
                        trace!("io-uring write at offset {offset} partially completed, re-queuing");
                        firewood_counter!(
                            "ring.partial_write_retry",
                            "amount of io-uring write entries that have been re-submitted due to partial writes"
                        ).increment(1);
                    }
                    if let Some(total_written) = self.written.checked_add(written) {
                        self.written = total_written;
                    } else {
                        errors.push(Error {
                            offset,
                            err: std::io::Error::other("overflow when summing total bytes written"),
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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at offset {}", self.err, self.offset)
    }
}

impl Errors {
    pub fn into_file_io_error(self, path: Option<PathBuf>) -> FileIoError {
        if self.errors.len() == 1 {
            let only = self.errors.into_iter().next().expect("just checked length");
            FileIoError::new(
                only.err,
                path,
                only.offset,
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
}

impl std::fmt::Debug for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.errors.as_slice() {
            [] => write!(f, "no errors (this is unexpected)"),
            [only] => std::fmt::Display::fmt(only, f),
            many if f.alternate() => {
                writeln!(f, "Encountered {} errors:", many.len())?;
                for (i, error) in many.iter().enumerate() {
                    writeln!(f, "  {}: {error}", i.wrapping_add(1))?;
                }
                Ok(())
            }
            [first, rest @ ..] => {
                write!(f, "{first} (and {} more errors)", rest.len())
            }
        }
    }
}

impl std::error::Error for Errors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Source only lets us return one error, so return the first.
        self.errors.first().map(|e| &e.err as &_)
    }
}

fn ignore_poll_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ResourceBusy
    )
}
