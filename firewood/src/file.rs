// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Copied from CedrusDB

use std::fs;
pub(crate) use std::os::unix::io::RawFd as Fd;
use std::path::Path;

use growthring::oflags;
use nix::errno::Errno;
use nix::fcntl::{open, openat, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, mkdir};

pub struct File {
    fd: Fd,
}

impl File {
    pub fn open_file(rootfd: Fd, fname: &str, truncate: bool) -> nix::Result<Fd> {
        openat(
            rootfd,
            fname,
            (if truncate {
                OFlag::O_TRUNC
            } else {
                OFlag::empty()
            }) | OFlag::O_RDWR,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
    }

    pub fn create_file(rootfd: Fd, fname: &str) -> nix::Result<Fd> {
        openat(
            rootfd,
            fname,
            OFlag::O_CREAT | OFlag::O_RDWR,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
    }

    fn _get_fname(fid: u64) -> String {
        format!("{fid:08x}.fw")
    }

    pub fn new(fid: u64, flen: u64, rootfd: Fd) -> nix::Result<Self> {
        let fname = Self::_get_fname(fid);
        let fd = match Self::open_file(rootfd, &fname, false) {
            Ok(fd) => fd,
            Err(e) => match e {
                Errno::ENOENT => {
                    let fd = Self::create_file(rootfd, &fname)?;
                    nix::unistd::ftruncate(fd, flen as nix::libc::off_t)?;
                    fd
                }
                e => return Err(e),
            },
        };
        Ok(File { fd })
    }

    pub fn get_fd(&self) -> Fd {
        self.fd
    }
}

impl Drop for File {
    fn drop(&mut self) {
        close(self.fd).unwrap();
    }
}

pub fn touch_dir<P: AsRef<Path>>(
    dir_name: &str,
    path: P,
) -> Result<std::path::PathBuf, std::io::Error> {
    let file_path = path.as_ref().join(dir_name);
    if !file_path.exists() {
        fs::create_dir_all(file_path);
    }
    Ok(file_path)
}

pub fn open_dir<P: AsRef<Path>>(path: P, truncate: bool) -> Result<(P, bool), std::io::Error> {
    let mut reset_header = truncate;
    if truncate {
        std::fs::remove_dir_all(&path)?;
    }

    if !path.as_ref().exists() {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        reset_header = true;
    }

    Ok((path, reset_header))
}
#[test]
/// This test simulates a filesystem error: for example the specified path
/// does not exist when creating a file.
fn test_create_file() {
    if let Err(e) = File::create_file(0, "/badpath/baddir") {
        assert_eq!(e.desc(), "No such file or directory")
    }
}
