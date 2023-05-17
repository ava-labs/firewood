//! Simple and modular write-ahead-logging implementation.
//!
//! # Examples
//!
//! ```
//! use growthring::{WalStoreAio, wal::WalLoader};
//! use futures::executor::block_on;
//! let mut loader = WalLoader::new();
//! loader.file_nbit(9).block_nbit(8);
//!
//!
//! // Start with empty WAL (truncate = true).
//! let store = WalStoreAio::new("/tmp/walfiles", true, None).unwrap();
//! let mut wal = block_on(loader.load(store, |_, _| {Ok(())}, 0)).unwrap();
//! // Write a vector of records to WAL.
//! for f in wal.grow(vec!["record1(foo)", "record2(bar)", "record3(foobar)"]).into_iter() {
//!     let ring_id = block_on(f).unwrap().1;
//!     println!("WAL recorded record to {:?}", ring_id);
//! }
//!
//!
//! // Load from WAL (truncate = false).
//! let store = WalStoreAio::new("/tmp/walfiles", false, None).unwrap();
//! let mut wal = block_on(loader.load(store, |payload, ringid| {
//!     // redo the operations in your application
//!     println!("recover(payload={}, ringid={:?})",
//!              std::str::from_utf8(&payload).unwrap(),
//!              ringid);
//!     Ok(())
//! }, 0)).unwrap();
//! // We saw some log playback, even there is no failure.
//! // Let's try to grow the WAL to create many files.
//! let ring_ids = wal.grow((1..100).into_iter().map(|i| "a".repeat(i)).collect::<Vec<_>>())
//!                   .into_iter().map(|f| block_on(f).unwrap().1).collect::<Vec<_>>();
//! // Then assume all these records are not longer needed. We can tell WalWriter by the `peel`
//! // method.
//! block_on(wal.peel(ring_ids, 0)).unwrap();
//! // There will only be one remaining file in /tmp/walfiles.
//!
//! let store = WalStoreAio::new("/tmp/walfiles", false, None).unwrap();
//! let wal = block_on(loader.load(store, |payload, _| {
//!     println!("payload.len() = {}", payload.len());
//!     Ok(())
//! }, 0)).unwrap();
//! // After each recovery, the /tmp/walfiles is empty.
//! ```

#[macro_use]
extern crate scan_fmt;
pub mod wal;
pub mod walerror;

use async_trait::async_trait;
use firewood_libaio::{AioBuilder, AioManager};
use libc::off_t;
#[cfg(not(target_os = "linux"))]
use nix::fcntl::OFlag;
use nix::unistd::{close, ftruncate};
#[cfg(target_os = "linux")]
use nix::{
    errno::Errno,
    fcntl::{fallocate, posix_fallocate, FallocateFlags, OFlag},
};
use std::fs;
use std::os::fd::IntoRawFd;
use std::os::unix::io::RawFd;
use std::os::unix::prelude::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wal::{WalBytes, WalFile, WalPos, WalStore};
use walerror::WalError;

pub struct WalFileAio {
    fd: RawFd,
    aiomgr: Arc<AioManager>,
}

impl WalFileAio {
    fn new<P: AsRef<Path>>(path: P, aiomgr: Arc<AioManager>) -> Result<Self, WalError> {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .create(true)
            .mode(0o600)
            .open(path)
            .map(|f| {
                let fd = f.into_raw_fd();
                WalFileAio { fd, aiomgr }
            })
            .map_err(|e| WalError::IOError(Arc::new(e)))
    }
}

impl Drop for WalFileAio {
    fn drop(&mut self) {
        close(self.fd).unwrap();
    }
}

#[async_trait(?Send)]
impl WalFile for WalFileAio {
    #[cfg(target_os = "linux")]
    async fn allocate(&self, offset: WalPos, length: usize) -> Result<(), WalError> {
        let (offset, length) = (offset as off_t, length as off_t);
        // TODO: is there any async version of fallocate?
        fallocate(
            self.fd,
            FallocateFlags::FALLOC_FL_ZERO_RANGE,
            offset,
            length,
        )
        .or_else(|err| match err {
            Errno::EOPNOTSUPP => posix_fallocate(self.fd, offset, length),
            _ => {
                eprintln!("fallocate failed with error: {err:?}");
                Err(err)
            }
        })
        .map_err(Into::into)
    }

    #[cfg(not(target_os = "linux"))]
    // TODO: macos support is possible here, but possibly unnecessary
    async fn allocate(&self, _offset: WalPos, _length: usize) -> Result<(), WalError> {
        Ok(())
    }

    async fn truncate(&self, length: usize) -> Result<(), WalError> {
        ftruncate(self.fd, length as off_t).map_err(From::from)
    }

    async fn write(&self, offset: WalPos, data: WalBytes) -> Result<(), WalError> {
        let (res, data) = self.aiomgr.write(self.fd, offset, data, None).await;
        res.map_err(Into::into).and_then(|nwrote| {
            if nwrote == data.len() {
                Ok(())
            } else {
                Err(WalError::Other(format!(
                    "partial write; wrote {nwrote} expected {} for fd {}",
                    data.len(),
                    self.fd
                )))
            }
        })
    }

    async fn read(&self, offset: WalPos, length: usize) -> Result<Option<WalBytes>, WalError> {
        let (res, data) = self.aiomgr.read(self.fd, offset, length, None).await;
        res.map_err(From::from)
            .map(|nread| if nread == length { Some(data) } else { None })
    }
}

pub struct WalStoreAio {
    root_dir: PathBuf,
    aiomgr: Arc<AioManager>,
}

unsafe impl Send for WalStoreAio {}

impl WalStoreAio {
    pub fn new<P: AsRef<Path>>(
        wal_dir: P,
        truncate: bool,
        aiomgr: Option<AioManager>,
    ) -> Result<Self, WalError> {
        let aio = match aiomgr {
            Some(aiomgr) => Arc::new(aiomgr),
            None => Arc::new(AioBuilder::default().build()?),
        };

        if truncate {
            if let Err(e) = fs::remove_dir_all(&wal_dir) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(From::from(e));
                }
            }
            fs::create_dir(&wal_dir)?;
        } else if !wal_dir.as_ref().exists() {
            // create Wal dir
            fs::create_dir(&wal_dir)?;
        }

        Ok(WalStoreAio {
            root_dir: wal_dir.as_ref().to_path_buf(),
            aiomgr: aio,
        })
    }
}

/// Return OS specific open flags for opening files
/// TODO: Switch to a rust idiomatic directory scanning approach
/// TODO: This shouldn't need to escape growth-ring (no pub)
pub fn oflags() -> OFlag {
    #[cfg(target_os = "linux")]
    return OFlag::O_DIRECTORY | OFlag::O_PATH;
    #[cfg(not(target_os = "linux"))]
    return OFlag::O_DIRECTORY;
}

#[async_trait(?Send)]
impl WalStore for WalStoreAio {
    type FileNameIter = std::vec::IntoIter<String>;

    async fn open_file(&self, filename: &str, _touch: bool) -> Result<Box<dyn WalFile>, WalError> {
        let path = self.root_dir.join(filename);
        WalFileAio::new(path, self.aiomgr.clone()).map(|f| Box::new(f) as Box<dyn WalFile>)
    }

    async fn remove_file(&self, filename: String) -> Result<(), WalError> {
        let file_to_remove = self.root_dir.join(filename);
        fs::remove_file(file_to_remove).map_err(From::from)
    }

    fn enumerate_files(&self) -> Result<Self::FileNameIter, WalError> {
        let mut filenames = Vec::new();
        for path in fs::read_dir(&self.root_dir)?.filter_map(|entry| entry.ok()) {
            filenames.push(path.file_name().into_string().unwrap());
        }
        Ok(filenames.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn truncation_makes_a_file_smaller() {
        const HALF_LENGTH: usize = 512;

        let walfile_path = get_walfile_path(file!(), line!());

        tokio::fs::remove_file(&walfile_path).await.ok();

        let aio_manager = AioBuilder::default().build().unwrap();

        let walfile_aio = WalFileAio::new(walfile_path, Arc::new(aio_manager)).unwrap();

        let first_half = vec![1u8; HALF_LENGTH];
        let second_half = vec![2u8; HALF_LENGTH];

        let data = first_half
            .iter()
            .copied()
            .chain(second_half.iter().copied())
            .collect();

        walfile_aio.write(0, data).await.unwrap();
        walfile_aio.truncate(HALF_LENGTH).await.unwrap();

        let result = walfile_aio.read(0, HALF_LENGTH).await.unwrap();

        assert_eq!(result, Some(first_half.into()))
    }

    #[tokio::test]
    async fn truncation_extends_a_file_with_zeros() {
        const LENGTH: usize = 512;

        let walfile_path = get_walfile_path(file!(), line!());

        tokio::fs::remove_file(&walfile_path).await.ok();

        let aio_manager = AioBuilder::default().build().unwrap();

        let walfile_aio = WalFileAio::new(walfile_path, Arc::new(aio_manager)).unwrap();

        walfile_aio
            .write(0, vec![1u8; LENGTH].into())
            .await
            .unwrap();

        walfile_aio.truncate(2 * LENGTH).await.unwrap();

        let result = walfile_aio.read(LENGTH as u64, LENGTH).await.unwrap();

        assert_eq!(result, Some(vec![0u8; LENGTH].into()))
    }

    fn get_walfile_path(file: &str, line: u32) -> PathBuf {
        Path::new("/tmp").join(format!("{}_{}", file.replace('/', "-"), line))
    }
}
