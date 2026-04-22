// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(feature = "ethhash")]
pub(crate) mod ethhash;
#[cfg(not(feature = "ethhash"))]
mod merkledb;
