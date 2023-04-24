// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Copied from CedrusDB

use std::io::ErrorKind;
use std::os::fd::IntoRawFd;
pub(crate) use std::os::unix::io::RawFd as Fd;
use std::path::{Path, PathBuf};

use nix::unistd::close;

pub struct File {
    fd: Fd,
}

impl File {
    pub fn open_file(rootpath: PathBuf, fname: &str, truncate: bool) -> Result<Fd, std::io::Error> {
        let mut filepath = rootpath;
        filepath.push(fname);
        Ok(std::fs::File::options()
            .truncate(truncate)
            .read(true)
            .write(true)
            .open(filepath)?
            .into_raw_fd())
        // TODO: file permissions?
    }

    pub fn create_file(rootpath: PathBuf, fname: &str) -> Result<Fd, std::io::Error> {
        let mut filepath = rootpath;
        filepath.push(fname);
        Ok(std::fs::File::options()
            .create(true)
            .read(true)
            .write(true)
            .open(filepath)?
            .into_raw_fd())
        // TODO: file permissions?
    }

    fn _get_fname(fid: u64) -> String {
        format!("{fid:08x}.fw")
    }

    pub fn new<P: AsRef<Path>>(fid: u64, flen: u64, rootdir: P) -> Result<Self, std::io::Error> {
        let fname = Self::_get_fname(fid);
        let fd = match Self::open_file(rootdir.as_ref().to_path_buf(), &fname, false) {
            Ok(fd) => fd,
            Err(e) => match e.kind() {
                ErrorKind::NotFound => {
                    let fd = Self::create_file(rootdir.as_ref().to_path_buf(), &fname)?;
                    nix::unistd::ftruncate(fd, flen as nix::libc::off_t)?;
                    fd
                }
                _ => return Err(e),
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

pub fn touch_dir(dirname: &str, rootdir: PathBuf) -> Result<PathBuf, std::io::Error> {
    let mut path = rootdir;
    path.push(dirname);
    if let Err(e) = std::fs::create_dir(&path) {
        // ignore already-exists error
        if e.kind() != ErrorKind::AlreadyExists {
            return Err(e);
        }
    }
    Ok(path)
}

pub fn open_dir<P: AsRef<Path>>(
    path: P,
    truncate: bool,
) -> Result<(PathBuf, bool), std::io::Error> {
    let mut reset_header = truncate;
    if truncate {
        let _ = std::fs::remove_dir_all(path.as_ref());
    }
    match std::fs::create_dir(path.as_ref()) {
        Err(e) => {
            if truncate || e.kind() != ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        Ok(_) => {
            // the DB did not exist
            reset_header = true
        }
    }
    Ok((PathBuf::from(path.as_ref()), reset_header))
}

// TODO: add some tests
