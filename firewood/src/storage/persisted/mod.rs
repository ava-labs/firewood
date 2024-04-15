// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// TODO: remove this once we have code that uses it
#![allow(dead_code)]

use bytes::Bytes;
use std::collections::HashMap;
use std::io::{Error, SeekFrom};
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

// Page should be boxed as to not take up so much stack-space
type Page = Box<[u8; PAGE_SIZE as usize]>;
const PAGE_SIZE: usize = 4096; // Page size in bytes

#[derive(Debug)]
pub struct StoreWrite {
    offset: u64,
    data: Bytes,
}

pub struct PersistedStore {
    path: PathBuf,
    fd: Mutex<File>,
    cache: Mutex<HashMap<u64, Page>>, // Page cache storing page offset and data
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
            cache: Mutex::new(HashMap::new()),
        })
    }

    // TODO: impl ReadLinearStore when it is async
    pub async fn stream_from(&self, addr: u64, length: usize) -> Result<Bytes, Error> {
        let start_page = addr / PAGE_SIZE as u64;
        let end_page = (addr + length as u64 - 1) / PAGE_SIZE as u64;

        let mut buffer = Vec::with_capacity(length);

        for page in start_page..=end_page {
            let page_offset = page * PAGE_SIZE as u64;
            let page_data = self.get_page(page_offset).await?;
            let offset_within_page = if page == start_page {
                (addr % PAGE_SIZE as u64) as usize
            } else {
                0
            };
            let remaining_length = (addr + length as u64 - page_offset) as usize;
            let data_to_copy = std::cmp::min(remaining_length, PAGE_SIZE - offset_within_page);
            buffer.extend_from_slice(
                &page_data[offset_within_page..offset_within_page + data_to_copy],
            );
        }

        Ok(Bytes::from(buffer))
    }

    async fn get_page(&self, page_offset: u64) -> Result<Page, Error> {
        let mut cache = self.cache.lock().await;
        if let Some(data) = cache.get(&page_offset) {
            return Ok(data.clone());
        }

        let mut file = self.fd.lock().await;
        file.seek(SeekFrom::Start(page_offset)).await?; // Seek to the page offset

        let file_size = file.metadata().await?.len(); // Get the size of the file
        let remaining_bytes = file_size.saturating_sub(page_offset); // Calculate remaining bytes in the file
        let page_size = std::cmp::min(PAGE_SIZE as u64, remaining_bytes) as usize; // Choose minimum of PAGE_SIZE and remaining bytes
        let mut page_data = Box::new([0u8; PAGE_SIZE]); // Initialize page_data with zeros

        file.read_exact(&mut page_data[..page_size]).await?; // Read data into page_data

        cache.insert(page_offset, page_data.clone());
        Ok(page_data)
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

        let writes = vec![
            StoreWrite {
                offset: 0,
                data: Bytes::from_static(b"Hello, "),
            },
            StoreWrite {
                offset: 7,
                data: Bytes::from_static(b"world!"),
            },
        ];

        persisted_store.write_all(&writes).await.unwrap();

        let read_data = persisted_store.stream_from(0, 13).await.unwrap();
        let read_str = String::from_utf8(read_data.to_vec()).unwrap();

        // Verify that the read data matches the expected content
        assert_eq!(read_str, "Hello, world!");

        // Clean up by deleting the temporary file
        fs::remove_file(file_path)
            .await
            .expect("Failed to delete temporary file");
    }
}
