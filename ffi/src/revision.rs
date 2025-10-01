// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::v2::api::{ArcDynDbView, HashKey};

#[derive(Debug)]
pub struct RevisionHandle {
    root: HashKey,
    view: ArcDynDbView,
}

impl RevisionHandle {
    pub(crate) fn new(root: HashKey, view: ArcDynDbView) -> RevisionHandle {
        RevisionHandle {
            root,
            view,
        }
    }
}

#[derive(Debug)]
pub struct GetRevisionResult {
    pub handle: RevisionHandle,
}
