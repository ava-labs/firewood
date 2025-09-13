// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod bytes_iter;
mod guard;
mod joined;
mod nibbles;
#[cfg(not(feature = "branch_factor_256"))]
mod packed;
mod split;
mod widened;

pub(crate) use self::bytes_iter::BytesIter;
#[cfg(not(feature = "branch_factor_256"))]
pub(crate) use self::packed::PackedPath;
pub(crate) use self::widened::WidenedPath;
#[cfg(feature = "branch_factor_256")]
pub(crate) type PackedPath<'a> = WidenedPath<'a>;
pub(crate) use self::guard::PathGuard;
pub(crate) use self::joined::JoinedPath;
pub(super) use self::nibbles::SplitNibbles;
pub(crate) use self::nibbles::{CollectedNibbles, Nibbles, PathNibble};
pub(super) use self::split::SplitPath;
