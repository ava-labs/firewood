// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};

use crate::{LinearAddress, Path};
use std::fmt::{Debug, Error as FmtError, Formatter};

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
/// A branch node
pub struct BranchNode {
    /// The partial path for this branch
    pub partial_path: Path,

    /// The value of the data for this branch, if any
    pub value: Option<Box<[u8]>>,

    /// The children of this branch
    pub children: [Option<LinearAddress>; Self::MAX_CHILDREN],
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Branch")?;
        write!(f, r#" path="{:?}""#, self.partial_path)?;

        for (i, c) in self.children.iter().enumerate() {
            if let Some(c) = c {
                write!(f, " ({i:x} {c:?})")?;
            }
        }

        write!(
            f,
            " v={}]",
            match &self.value {
                Some(v) => hex::encode(&**v),
                None => "nil".to_string(),
            }
        )
    }
}

impl BranchNode {
    /// The maximum number of children in a [BranchNode]
    pub const MAX_CHILDREN: usize = 16;

    /// Obtain a mutable reference to a child address within a branch
    /// This convenience method takes a u8 as the nibble offset
    /// It panics if passed a value more than 0xf
    pub fn mut_child(&mut self, child_index: u8) -> &mut Option<LinearAddress> {
        self.children.get_mut(child_index as usize).expect("nibble")
    }
}
