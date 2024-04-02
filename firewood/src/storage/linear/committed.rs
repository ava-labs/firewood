// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{LinearStore, ReadLinearStore};

/// A linear store used for historical revisions
///
/// A [Committed] [LinearStore] supports read operations only
#[derive(Debug)]
struct Committed<P: ReadLinearStore> {
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Arc<LinearStore<P>>,
}
