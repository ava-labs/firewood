// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// during development only
#![allow(dead_code)]

// This synchronous file layer is a simple implementation of what we
// want to do for I/O. This uses a [Mutex] lock around a simple `File`
// object. Instead, we probably should use an IO system that can perform multiple
// read/write operations at once

use std::fs::{File, OpenOptions};
use std::io::{Error, Read, Seek};
use std::num::NonZero;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::fmt::{self, Debug, Formatter};

use io_uring::types::Fd;
use lru::LruCache;
use io_uring::{IoUring, opcode::Write};

use crate::{LinearAddress, Node};

use super::{ReadableStorage, WritableStorage};

/// A [ReadableStorage] backed by a file
pub struct FileBacked {
    fd: Mutex<File>,
    cache: Mutex<LruCache<LinearAddress, Arc<Node>>>,
    ring: Mutex<IoUring>,
    raw_fd: Fd,
}

impl Debug for FileBacked {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileBacked")
            .field("fd", &self.fd)
            .field("cache", &self.cache)
            .finish()
    }
}
struct WriteRequest {
    offset: u64,
    object: Box<[u8]>,
}

impl FileBacked {
    /// Create or open a file at a given path
    pub fn new(
        path: PathBuf,
        node_cache_size: NonZero<usize>,
        truncate: bool,
    ) -> Result<Self, Error> {

        let fd = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(truncate)
            .open(path)?;


        let raw_fd = fd.as_raw_fd();

        Ok(Self {
            fd: Mutex::new(fd),
            cache: Mutex::new(LruCache::new(node_cache_size)),
            ring: Mutex::new(IoUring::new(8)?),
            raw_fd: Fd(raw_fd),
        })
    }
}

impl ReadableStorage for FileBacked {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, Error> {
        let mut fd = self.fd.lock().expect("p");
        fd.seek(std::io::SeekFrom::Start(addr))?;
        Ok(Box::new(fd.try_clone().expect("poisoned lock")))
    }

    fn size(&self) -> Result<u64, Error> {
        self.fd
            .lock()
            .expect("poisoned lock")
            .seek(std::io::SeekFrom::End(0))
    }

    fn read_cached_node(&self, addr: LinearAddress) -> Option<Arc<Node>> {
        let mut guard = self.cache.lock().expect("poisoned lock");
        guard.get(&addr).cloned()
    }
}

impl WritableStorage for FileBacked {
    /// Write to the backend filestore. This does not implement [crate::WritableStorage]
    /// because we don't want someone accidentally writing nodes directly to disk
    fn write(&self, offset: u64, object: &[u8]) -> Result<usize, Error> {
        self.fd
            .lock()
            .expect("poisoned lock")
            .write_at(object, offset)
    }

    fn async_write(&self, offset: u64, object: &[u8]) -> Result<usize, Error> {
        let mut ring = self.ring.lock().expect("poisoned lock");
        let write = Write::new(self.raw_fd, object.as_ptr(), offset as u32).build();
        unsafe { ring.submission().push(&write) }.map_err(|e| Error::new(std::io::ErrorKind::Other, e))?;
        ring.submit()?;
        Ok(object.len())
    }

    fn write_cached_nodes<'a>(
        &self,
        nodes: impl Iterator<Item = (&'a std::num::NonZero<u64>, &'a std::sync::Arc<crate::Node>)>,
    ) -> Result<(), Error> {
        let mut guard = self.cache.lock().expect("poisoned lock");
        for (addr, node) in nodes {
            guard.put(*addr, node.clone());
        }
        Ok(())
    }
}
