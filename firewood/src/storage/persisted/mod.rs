// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// TODO: remove this once we have code that uses it
#![allow(dead_code)]

use bytes::Bytes;
use std::io::{Error, SeekFrom};
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct StoreWrite {
    offset: u64,
    data: Bytes,
}

pub struct PersistedStore {
    path: PathBuf,
    fd: Mutex<File>,
}

impl PersistedStore {
    pub async fn new(path: PathBuf) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .create(true)
            .mode(0o600)
            .open(&path)
            .await?;
        Ok(Self {
            path,
            fd: Mutex::new(file),
        })
    }

    // TODO: impl ReadLinearStore when it is async
    pub async fn stream_from(&self, addr: u64, length: usize) -> Result<Bytes, Error> {
        let mut file = self.fd.lock().await;
        file.seek(SeekFrom::Start(addr)).await?;
        let mut buffer = vec![0; length];
        file.read_exact(&mut buffer).await?;
        Ok(Bytes::from(buffer))
    }

    pub async fn write_all(&mut self, writes: &[StoreWrite]) -> Result<(), Error> {
        let mut file = self.fd.lock().await;

        for write in writes {
            file.seek(SeekFrom::Start(write.offset)).await?; // Seek to the specified offset
            file.write_all(&write.data).await?; // Write data to the file at the specified offset
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    #[tokio::test]
    async fn test_persisted_store() {
        // Create a temporary directory to store the file
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_persisted_file");

        let mut persisted_store = PersistedStore::new(file_path.clone()).await.unwrap();
        let first_data = "Hello, ";
        let second_data = "world";

        let writes = vec![
            StoreWrite {
                offset: 0,
                data: Bytes::from(first_data),
            },
            StoreWrite {
                offset: first_data.len() as u64,
                data: Bytes::from(second_data),
            },
        ];

        persisted_store.write_all(&writes).await.unwrap();

        let read_data = persisted_store
            .stream_from(0, first_data.len() + second_data.len())
            .await
            .unwrap();
        let read_str = String::from_utf8(read_data.to_vec()).unwrap();

        // Verify that the read data matches the expected content
        assert_eq!(read_str, first_data.to_owned() + second_data);

        // Clean up by deleting the temporary file
        fs::remove_file(file_path)
            .await
            .expect("Failed to delete temporary file");
    }
}
