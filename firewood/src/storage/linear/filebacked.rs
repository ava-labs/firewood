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

use super::ReadLinearStore;

#[derive(Debug)]
pub(crate) struct FileBacked {
    fd: Mutex<File>,
}

impl FileBacked {
    pub(crate) fn new(path: PathBuf) -> Result<Self, Error> {
        let fd = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        Ok(Self { fd: Mutex::new(fd) })
    }
}

impl ReadLinearStore for FileBacked {
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
}

impl FileBacked {
    pub(crate) fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error> {
        self.fd
            .lock()
            .expect("poisoned lock")
            .write_at(object, offset)
    }
}
