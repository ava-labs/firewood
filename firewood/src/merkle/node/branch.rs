// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    merkle::{nibbles_to_bytes_iter, Path},
    nibbles::Nibbles,
    shale::DiskAddress,
};
use bincode::{Error, Options};
use serde::de::Error as DeError;
use std::{
    fmt::{Debug, Error as FmtError, Formatter},
    io::{Cursor, Write},
    mem::size_of,
};

type PathLen = u8;
pub type ValueLen = u32;
pub type EncodedChildLen = u8;

const MAX_CHILDREN: usize = 16;

#[derive(PartialEq, Eq, Clone)]
pub struct BranchNode {
    pub(crate) partial_path: Path,
    pub(crate) children: [Option<DiskAddress>; MAX_CHILDREN],
    pub(crate) value: Option<Vec<u8>>,
    pub(crate) children_encoded: [Option<Vec<u8>>; MAX_CHILDREN],
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

        for (i, c) in self.children_encoded.iter().enumerate() {
            if let Some(c) = c {
                write!(f, " ({i:x} {:?})", c)?;
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

    pub const fn value(&self) -> &Option<Vec<u8>> {
        &self.value
    }

    pub const fn chd(&self) -> &[Option<DiskAddress>; Self::MAX_CHILDREN] {
        &self.children
    }

    pub fn chd_mut(&mut self) -> &mut [Option<DiskAddress>; Self::MAX_CHILDREN] {
        &mut self.children
    }

    pub const fn chd_encode(&self) -> &[Option<Vec<u8>>; Self::MAX_CHILDREN] {
        &self.children_encoded
    }

    pub fn chd_encoded_mut(&mut self) -> &mut [Option<Vec<u8>>; Self::MAX_CHILDREN] {
        &mut self.children_encoded
    }

    pub(super) fn decode(buf: &[u8]) -> Result<Self, Error> {
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
            partial_path: path,
            children: [None; Self::MAX_CHILDREN],
            value,
            children_encoded: chd_encoded,
        })
    }

    pub(super) fn encode<S>(&self) -> Vec<u8> {
        todo!()
    }
}

fn optional_value_len<Len, T: AsRef<[u8]>>(value: Option<T>) -> u64 {
    size_of::<Len>() as u64
        + value
            .as_ref()
            .map_or(0, |value| value.as_ref().len() as u64)
}
