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
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::Mutex;

<<<<<<<< HEAD:storage/src/revisions/filestore.rs
use super::{ReadableStorage, WritableStorage};

/// FileStore represents a synchronous file layer for I/O operations.
#[derive(Debug)]
pub struct FileStore {
========
#[derive(Debug)]
pub struct FileBacked {
>>>>>>>> e66d9c59b7d4806d44ee4f22febb7a9362ca8df0:storage/src/revisions/filebacked.rs
    fd: Mutex<File>,
}

impl FileStore {
    /// Create or open a file at a given path
    pub fn new(path: PathBuf, truncate: bool) -> Result<Self, Error> {
        let fd = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(truncate)
            .open(path)?;

        Ok(Self { fd: Mutex::new(fd) })
    }
}

<<<<<<<< HEAD:storage/src/revisions/filestore.rs
impl ReadableStorage for FileStore {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, Error> {
========
impl FileBacked {
    pub fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, Error> {
>>>>>>>> e66d9c59b7d4806d44ee4f22febb7a9362ca8df0:storage/src/revisions/filebacked.rs
        let mut fd = self.fd.lock().expect("p");
        fd.seek(std::io::SeekFrom::Start(addr))?;
        Ok(Box::new(fd.try_clone().expect("poisoned lock")))
    }

    pub fn size(&self) -> Result<u64, Error> {
        self.fd
            .lock()
            .expect("poisoned lock")
            .seek(std::io::SeekFrom::End(0))
    }
}

impl WritableStorage for FileStore {
    /// Write to the backend filestore. This does not implement [crate::WriteLinearStore]
    /// because we don't want someone accidentally writing nodes directly to disk
<<<<<<<< HEAD:storage/src/revisions/filestore.rs
    fn write(&self, offset: u64, object: &[u8]) -> Result<usize, Error> {
========
    pub fn write(&self, offset: u64, object: &[u8]) -> Result<usize, Error> {
>>>>>>>> e66d9c59b7d4806d44ee4f22febb7a9362ca8df0:storage/src/revisions/filebacked.rs
        self.fd
            .lock()
            .expect("poisoned lock")
            .write_at(object, offset)
    }
}
