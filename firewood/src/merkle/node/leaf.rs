// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    merkle::{nibbles_to_bytes_iter, Path},
    nibbles::Nibbles,
    shale::{ShaleError::InvalidCacheView, Storable},
};
use bytemuck::{Pod, Zeroable};
use std::{
    fmt::{Debug, Error as FmtError, Formatter},
    io::{Cursor, Write},
    mem::size_of,
};

pub const SIZE: usize = 2;

type PathLen = u8;
type ValueLen = u32;

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

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C, packed)]
struct Meta {
    path_len: PathLen,
    value_len: ValueLen,
}

impl Meta {
    const SIZE: usize = size_of::<Self>();
}

impl Storable for LeafNode {
    fn serialized_len(&self) -> u64 {
        let meta_len = size_of::<Meta>() as u64;
        let path_len = self.partial_path.serialized_len();
        let value_len = self.value.len() as u64;

        meta_len + path_len + value_len
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), crate::shale::ShaleError> {
        let mut cursor = Cursor::new(to);

        let path = &self.partial_path.encode();
        let path = nibbles_to_bytes_iter(path);
        let value = &self.value;

        let path_len = self.partial_path.serialized_len() as PathLen;
        let value_len = value.len() as ValueLen;

        let meta = Meta {
            path_len,
            value_len,
        };

        cursor.write_all(bytemuck::bytes_of(&meta))?;

        for nibble in path {
            cursor.write_all(&[nibble])?;
        }

        cursor.write_all(value)?;

        Ok(())
    }

    fn deserialize<T: crate::shale::CachedStore>(
        offset: usize,
        mem: &T,
    ) -> Result<Self, crate::shale::ShaleError>
    where
        Self: Sized,
    {
        let node_header_raw = mem
            .get_view(offset, Meta::SIZE as u64)
            .ok_or(InvalidCacheView {
                offset,
                size: Meta::SIZE as u64,
            })?
            .as_deref();

        let offset = offset + Meta::SIZE;
        let Meta {
            path_len,
            value_len,
        } = *bytemuck::from_bytes(&node_header_raw);
        let size = path_len as u64 + value_len as u64;

        let remainder = mem
            .get_view(offset, size)
            .ok_or(InvalidCacheView { offset, size })?
            .as_deref();

        let (path, value) = remainder.split_at(path_len as usize);

        let path = {
            let nibbles = Nibbles::<0>::new(path).into_iter();
            Path::from_nibbles(nibbles).0
        };

        let value = value.to_vec();

        Ok(Self::new(path, value))
    }
}
