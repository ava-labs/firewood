// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// during development only
#![allow(dead_code)]

// This synchronous file layer is a simple implementation of what we
// want to do for I/O. This uses a [Mutex] lock around a simple `File`
// object. Instead, we probably should use an IO system that can perform multiple
// read/write operations at once

use futures::Future;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeekExt};
use tokio::task::spawn_blocking;

use super::{ReadLinearStore, WriteLinearStore};
use std::io::Error;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::pin::Pin;

#[derive(Debug)]
pub(super) struct FileBacked {
    path: PathBuf,
    fd: File,
}

impl ReadLinearStore for FileBacked {
    fn stream_from(&self, addr: u64) -> Result<Pin<Box<dyn AsyncRead + '_>>, Error> {
        let mut fd = self.fd;
        fd.seek(std::io::SeekFrom::Start(addr));
        Ok(Box::pin(fd))
    }

    fn size(&self) -> Pin<Box<dyn Future<Output = Result<u64, Error>>>> {
        Pin::from(Box::from(self.fd.seek(std::io::SeekFrom::End(0))))
    }
}

impl WriteLinearStore for FileBacked {
    async fn write(&mut self, offset: u64, object: Box<[u8]>) -> Result<usize, Error> {
        let std_file = self.fd.try_clone().await?.into_std().await;
        spawn_blocking(move || {
            std_file.write_at(&object, offset).unwrap();
        })
        .await
        .expect("Failed to run blocking task");
        Ok(0)
    }
}
