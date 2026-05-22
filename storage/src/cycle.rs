// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{LinearAddress, PartialPath, PathBuf};
use std::fmt;

/// Error returned when a cycle is detected during trie traversal.
///
/// A trie cycle exists when a node's child disk address points to a node that
/// was already visited on the current traversal. This would cause infinite
/// iteration. Iterators that load nodes by [`LinearAddress`] return this error
/// instead of looping.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CycleDetected {
    /// The disk address that was encountered a second time.
    pub address: LinearAddress,
    /// The trie nibble-path at which `address` was first visited.
    pub path_to_first_visit: PathBuf,
    /// The trie nibble-path of the back-edge that would revisit `address`.
    pub path_to_revisit: PathBuf,
}

impl CycleDetected {
    /// Construct a new [`CycleDetected`] error.
    ///
    /// # Arguments
    ///
    /// * `address` — the disk address encountered a second time.
    /// * `path_to_first_visit` — the nibble-path at which `address` was first seen.
    /// * `path_to_revisit` — the nibble-path of the back-edge that would revisit `address`.
    #[must_use]
    pub const fn new(
        address: LinearAddress,
        path_to_first_visit: PathBuf,
        path_to_revisit: PathBuf,
    ) -> Self {
        Self {
            address,
            path_to_first_visit,
            path_to_revisit,
        }
    }
}

impl fmt::Display for CycleDetected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cycle in trie at address {:#x}: first visited at {}, back-edge at {}",
            self.address.get(),
            PartialPath::Borrowed(self.path_to_first_visit.as_slice()),
            PartialPath::Borrowed(self.path_to_revisit.as_slice()),
        )
    }
}

impl std::error::Error for CycleDetected {}
