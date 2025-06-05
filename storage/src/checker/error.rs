// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::LinearAddress;

use thiserror::Error;

/// Errors returned by the checker
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CheckerError {
    /// The root node was not found
    #[error("root node not found")]
    RootNodeNotFound,

    /// The file size is not valid
    #[error("file size {0} is not valid")]
    InvalidFileSize(u64),

    /// The address is out of bounds
    #[error("stored area at {start} with size {size} is out of bounds")]
    AreaOutOfBounds {
        /// Start of the StoredArea
        start: LinearAddress,
        /// Size of the StoredArea
        size: u64,
    },

    /// Stored areas intersect
    #[error("stored area at {start} with size {size} intersects with another stored area")]
    AreaIntersects {
        /// Start of the StoredArea
        start: LinearAddress,
        /// Size of the StoredArea
        size: u64,
    },

    /// IO error
    #[error("IO error")]
    IO(#[from] std::io::Error),
}
