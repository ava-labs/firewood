// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{merkle::path::Path, nibbles::Nibbles, storage::node::LinearAddress};
use bincode::{Error, Options};
use serde::de::Error as DeError;
use std::{
    fmt::{Debug, Error as FmtError, Formatter},
    mem::size_of,
};

const MAX_CHILDREN: usize = 16;

#[derive(PartialEq, Eq, Clone)]
pub struct BranchNode {
    pub(crate) path: Path,
    pub(crate) value: Option<Box<[u8]>>,
    pub(crate) children: [Option<LinearAddress>; MAX_CHILDREN],
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Branch")?;
        write!(f, r#" path="{:?}""#, self.path)?;

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
    pub const MAX_CHILDREN: usize = MAX_CHILDREN;
    pub const MSIZE: usize = Self::MAX_CHILDREN + 2;

    pub(super) fn _decode(buf: &[u8]) -> Result<Self, Error> {
        let mut items: Vec<Vec<u8>> = bincode::DefaultOptions::new().deserialize(buf)?;

        let path = items.pop().ok_or(Error::custom("Invalid Branch Node"))?;
        let path = Nibbles::<0>::new(&path);
        let path = Path::from_nibbles(path.into_iter());

        // we've already validated the size, that's why we can safely unwrap
        #[allow(clippy::unwrap_used)]
        let value = items.pop().unwrap();
        // Extract the value of the branch node and set to None if it's an empty Vec
        let value = Some(value).filter(|value| !value.is_empty());

        // encode all children.
        let mut chd_encoded: [Option<Vec<u8>>; Self::MAX_CHILDREN] = Default::default();

        // we popped the last element, so their should only be NBRANCH items left
        for (i, chd) in items.into_iter().enumerate() {
            #[allow(clippy::indexing_slicing)]
            (chd_encoded[i] = Some(chd).filter(|value| !value.is_empty()));
        }

        Ok(BranchNode {
            path,
            children: [None; Self::MAX_CHILDREN],
            value: value.map(|value| value.into_boxed_slice()),
        })
    }

    pub(super) fn _encode(&self) -> Vec<u8> {
        todo!()
    }
}

fn _optional_value_len<Len, T: AsRef<[u8]>>(value: Option<T>) -> u64 {
    size_of::<Len>() as u64
        + value
            .as_ref()
            .map_or(0, |value| value.as_ref().len() as u64)
}
