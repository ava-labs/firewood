// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};

use crate::node::Path;
use std::fmt::{Debug, Error as FmtError, Formatter};

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct LeafNode {
    // TODO danlaine: should we hardcode generic param of Path to Box<[u8]>?
    pub(crate) partial_path: Path<Box<[u8]>>,
    pub(crate) value: Vec<u8>,
}

impl Debug for LeafNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(
            f,
            "[Leaf {:?} {}]",
            self.partial_path,
            hex::encode(&*self.value)
        )
    }
}
