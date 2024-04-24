// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::path::Path;
use std::fmt::{Debug, Error as FmtError, Formatter};

pub const SIZE: usize = 2;

#[derive(PartialEq, Eq, Clone)]
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

impl LeafNode {
    pub fn new<P: Into<Path>, V: Into<Vec<u8>>>(partial_path: P, value: V) -> Self {
        Self {
            partial_path: partial_path.into(),
            value: value.into(),
        }
    }

    pub const fn path(&self) -> &Path {
        &self.partial_path
    }

    pub const fn value(&self) -> &Vec<u8> {
        &self.value
    }
}
