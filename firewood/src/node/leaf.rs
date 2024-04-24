// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};

use crate::node::Path;
use std::fmt::{Debug, Error as FmtError, Formatter};

pub const SIZE: usize = 2;

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct LeafNode {
    pub(crate) partial_path: Path,
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
