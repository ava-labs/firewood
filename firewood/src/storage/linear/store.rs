// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// during development only

#![allow(dead_code)]

use tokio::fs::{File, OpenOptions};
pub struct StoreWrite {
    offset: u64,
    data: Bytes,
}

pub struct PersistedStore {
    path: PathBuf,
    fd: Mutex<File>,
}

impl ReadLinearStore for PersistedStore {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error> {
        let mut fd = self.fd.lock().expect("p");
        fd.seek(std::io::SeekFrom::Start(addr))?;
        Ok(fd.try_clone().expect("poisoned lock"))
    }
}

impl PersistedStore {
    async fn write_all(&mut self, writes: &[StoreWrite]) -> Result<(), Error> {
        self.fd
        .lock()
        .expect("poisoned lock")
        .seek(std::io::SeekFrom::End(0))
    }
}