// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Header Module
//!
//! This module defines the nodestore header structure and validation logic for ensuring
//! database compatibility across different versions and configurations.
//!
//! ## Header Structure
//!
//! The `NodeStoreHeader` is stored at the beginning of every nodestore file and contains:
//!
//! - **Version String** - Human-readable firewood version (e.g., "firewood 0.1.0")
//! - **Endianness Test** - Detects byte order mismatches between platforms
//! - **Root Address** - Points to the merkle trie root node (if any)
//! - **Storage Size** - Total allocated storage space
//! - **Free Lists** - Array of free space linked list heads for each area size
//!
//! ## Storage Layout
//!
//! The header occupies the first 2048 bytes of storage:
//! - Fixed size for alignment with disk block boundaries
//! - Zero-padded to full size for consistent layout
//! - Uses C-compatible representation for cross-language access
//!

use bytemuck_derive::{Pod, Zeroable};
use std::io::{Error, ErrorKind, Read};

use super::alloc::FreeLists;
use super::primitives::{LinearAddress, area_size_hash};
use crate::linear::FileIoError;
use crate::logger::{debug, trace};
use crate::{ReadableStorage, WritableStorage};

/// Can be used by filesystem tooling such as "file" to identify
/// the version of firewood used to create this `NodeStore` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Zeroable, Pod)]
#[repr(transparent)]
pub struct Version {
    bytes: [u8; 16],
}

impl Version {
    const SIZE: usize = size_of::<Self>();

    /// Version >= 0.0.4
    ///
    /// Increase as needed to set the minimum required version of `firewood-storage` for
    /// compatibility checks.
    ///
    /// We may want to add migrations if we need to add a breaking change.
    const BASE_VERSION: semver::Comparator = semver::Comparator {
        op: semver::Op::GreaterEq,
        major: 0,
        minor: Some(0),
        patch: Some(4),
        pre: semver::Prerelease::EMPTY,
    };

    /// Validates that the version identifier is valid and compatible with the current
    /// build of firewood.
    ///
    /// # Errors
    ///
    /// - If the token contains invalid utf-8 bytes (nul is allowed).
    /// - If the token does not start with "firewood ".
    /// - If the version is not parsable by [`semver::Version`].
    /// - If the version is not compatible with the current build of firewood.
    ///   - Currently, the minimum required version is 0.0.4.
    pub fn validate(&self) -> Result<(), Error> {
        let version = std::str::from_utf8(&self.bytes).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Invalid database version: invalid utf-8: {e} (original: [{:032x}])",
                    u128::from_be_bytes(self.bytes)
                ),
            )
        })?;

        // strip trailling nuls as they're only for padding
        let version = version.trim_end_matches('\0');

        // strip magic prefix or error
        let version = version.strip_prefix("firewood ").ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Invalid database version: does not start with magic 'firewood ': {version}",
                ),
            )
        })?;

        // Version strings from CARGO_PKG_VERSION are guaranteed to be parsable by
        // semver (cargo uses the same library).
        let version = semver::Version::parse(version).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Invalid version string: unable to parse `{version}` as a semver string: {e}"
                ),
            )
        })?;

        // verify base compatibility version
        if !Self::BASE_VERSION.matches(&version) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Database was created with firewood version {version}; however, this build of firewood requires version {}",
                    Self::BASE_VERSION,
                ),
            ));
        }

        debug!(
            "Database version is valid: {version} {}",
            Self::BASE_VERSION
        );
        Ok(())
    }

    /// Construct a [`Version`] instance for the current build of firewood.
    pub fn new() -> Self {
        // Note that with this magic token of 9 bytes, we can store a version string of
        // up to 7 bytes. If we always include the major, minor, and patch versions,
        // then no more than two of three can be 2 digits long.
        const VERSION_STR: &str = concat!("firewood ", env!("CARGO_PKG_VERSION"));
        const {
            assert!(
                VERSION_STR.len() <= Version::SIZE,
                concat!(
                    "Database version string `firewood ",
                    env!("CARGO_PKG_VERSION"),
                    "` is too long for the Version struct! Update Cargo.toml or modify this code.",
                ),
            );
        }

        // pad with nul bytes
        let mut bytes = [0u8; Version::SIZE];
        bytes
            .get_mut(..VERSION_STR.len())
            .expect("must fit")
            .copy_from_slice(VERSION_STR.as_bytes());

        Self { bytes }
    }
}

/// Persisted metadata for a `NodeStore`.
/// The [`NodeStoreHeader`] is at the start of the `ReadableStorage`.
#[derive(Copy, Debug, PartialEq, Eq, Clone, Zeroable, Pod)]
#[repr(C)]
pub struct NodeStoreHeader {
    /// Identifies the version of firewood used to create this `NodeStore`.
    version: Version,
    /// always "1"; verifies endianness
    endian_test: u64,
    /// Total allocated storage size (high water mark). New nodes are allocated
    /// at this offset when the free list is empty, then this value is incremented.
    size: u64,
    /// Heads of the free lists for each area size class.
    ///
    /// Element `i` points to the first free area of size `AREA_SIZES[i]`. Each free area
    /// contains a pointer to the next free area of the same size, forming a linked list.
    ///
    /// When allocating a new node, the allocator first checks the appropriate free list.
    /// If empty, it falls back to bumping `size`. Free lists are populated at commit time
    /// during revision reaping, when deleted nodes from evicted revisions are reclaimed.
    free_lists: FreeLists,
    /// Disk address of the merkle trie root node, or `None` for an empty trie.
    ///
    /// This is updated at commit time to point to the new root after changes are persisted.
    /// The address is a direct file offset, not a hash-based reference.
    root_address: Option<LinearAddress>,
    /// The hash of the area sizes used in this database to prevent someone from changing the
    /// area sizes and trying to read old databases with the wrong area sizes.
    area_size_hash: [u8; 32],
    /// Whether ethhash was enabled when this database was created.
    ethhash: u64,
}

impl Default for NodeStoreHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeStoreHeader {
    /// The first SIZE bytes of the `ReadableStorage` are reserved for the
    /// [`NodeStoreHeader`].
    /// We also want it aligned to a disk block
    pub const SIZE: u64 = 2048;

    // Compile-time assertion that SIZE is large enough for the header
    const _ASSERT_SIZE: () = assert!(Self::SIZE as usize >= std::mem::size_of::<NodeStoreHeader>());

    /// Deserialize a `NodeStoreHeader` from bytes using bytemuck
    fn from_bytes(bytes: &[u8]) -> &Self {
        bytemuck::from_bytes(bytes)
    }

    /// Creates a new header with default values and no root address.
    ///
    /// # Panics
    ///
    /// Will panic if ther `area_size_hash` cannot be computed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // The store just contains the header at this point
            size: Self::SIZE,
            endian_test: 1,
            root_address: None,
            version: Version::new(),
            free_lists: Default::default(),
            area_size_hash: area_size_hash()
                .as_slice()
                .try_into()
                .expect("sizes should match"),
            #[cfg(feature = "ethhash")]
            ethhash: 1,
            #[cfg(not(feature = "ethhash"))]
            ethhash: 0,
        }
    }

    /// Read and create a header from storage, or return a new empty header if
    /// storage is empty.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the header cannot be read or validated.
    pub fn with_storage<S: ReadableStorage>(storage: &S) -> Result<Self, FileIoError> {
        let mut stream = storage.stream_from(0)?;
        let mut header_bytes = vec![0u8; std::mem::size_of::<Self>()];

        if let Err(e) = stream.read_exact(&mut header_bytes) {
            if e.kind() == ErrorKind::UnexpectedEof {
                // Storage is empty, return a fresh header
                return Ok(Self::new());
            }
            return Err(storage.file_io_error(e, 0, Some("header read".to_string())));
        }

        drop(stream);

        let header = *Self::from_bytes(&header_bytes);
        header
            .validate()
            .map_err(|e| storage.file_io_error(e, 0, Some("header validation".to_string())))?;

        Ok(header)
    }

    fn validate(&self) -> Result<(), Error> {
        trace!("Checking version...");
        self.version.validate()?;

        trace!("Checking endianness...");
        self.validate_endian_test()?;

        trace!("Checking area size hash...");
        self.validate_area_size_hash()?;

        trace!("Checking if db ethhash flag matches build feature...");
        self.validate_ethhash()?;

        Ok(())
    }

    /// Get the size of the nodestore
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Set the size of the nodestore
    pub const fn set_size(&mut self, size: u64) {
        self.size = size;
    }

    /// Get the free lists
    #[must_use]
    pub const fn free_lists(&self) -> &FreeLists {
        &self.free_lists
    }

    /// Get mutable access to the free lists
    pub const fn free_lists_mut(&mut self) -> &mut FreeLists {
        &mut self.free_lists
    }

    /// Get the root address
    #[must_use]
    pub const fn root_address(&self) -> Option<LinearAddress> {
        self.root_address
    }

    /// Set the root address
    pub const fn set_root_address(&mut self, root_address: Option<LinearAddress>) {
        self.root_address = root_address;
    }

    /// Get the offset of the `free_lists` field for use with `offset_of`!
    #[must_use]
    pub const fn free_lists_offset() -> u64 {
        std::mem::offset_of!(NodeStoreHeader, free_lists) as u64
    }

    fn validate_endian_test(&self) -> Result<(), Error> {
        if self.endian_test == 1 {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidData,
                "Database cannot be opened due to difference in endianness",
            ))
        }
    }

    fn validate_area_size_hash(&self) -> Result<(), Error> {
        if self.area_size_hash == area_size_hash().as_slice() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidData,
                "Database cannot be opened due to difference in area size hash",
            ))
        }
    }

    #[cfg(not(feature = "ethhash"))]
    fn validate_ethhash(&self) -> Result<(), Error> {
        if self.ethhash == 0 {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidData,
                "Database cannot be opened as it was created with ethhash enabled",
            ))
        }
    }

    #[cfg(feature = "ethhash")]
    fn validate_ethhash(&self) -> Result<(), Error> {
        if self.ethhash == 1 {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidData,
                "Database cannot be opened as it was created without ethhash enabled",
            ))
        }
    }

    /// Persist this header to storage.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the header cannot be written.
    pub fn flush_to<S: WritableStorage>(&self, storage: &S) -> Result<(), FileIoError> {
        let header_bytes = bytemuck::bytes_of(self);
        storage.write(0, header_bytes)?;
        Ok(())
    }

    /// Persist this header to storage including all the padding.
    /// This is only done the first time we write the header.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the header cannot be written.
    pub fn flush_with_padding_to<S: WritableStorage>(
        &self,
        storage: &S,
    ) -> Result<(), FileIoError> {
        let mut header_bytes = bytemuck::bytes_of(self).to_vec();
        header_bytes.resize(Self::SIZE as usize, 0);
        debug_assert_eq!(header_bytes.len(), Self::SIZE as usize);

        storage.write(0, &header_bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test]
    fn test_version_new_is_valid() {
        Version::new()
            .validate()
            .expect("Version::new() should always be valid");
    }

    #[test_case(*b"invalid\0\0\0\0\0\0\0\0\0")]
    #[test_case(*b"avalanche 0.1.0\0")]
    #[test_case(*b"firewood 0.0.1\0\0")]
    fn test_invalid_version_strings(bytes: [u8; 16]) {
        assert!(Version { bytes }.validate().is_err());
    }

    #[test]
    fn test_header_new() {
        let header = NodeStoreHeader::new();

        // Check the header is correctly initialized.
        assert_eq!(header.version, Version::new());
        let empty_free_list: FreeLists = Default::default();
        assert_eq!(*header.free_lists(), empty_free_list);
    }
}
