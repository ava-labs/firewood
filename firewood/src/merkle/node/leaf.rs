// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Data;
use crate::{
    merkle::{from_nibbles_even, PartialPath},
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
type DataLen = u32;

#[derive(PartialEq, Eq, Clone)]
pub struct LeafNode {
    pub(crate) path: PartialPath,
    pub(crate) data: Data,
}

impl Debug for LeafNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Leaf {:?} {}]", self.path, hex::encode(&*self.data))
    }
}

impl LeafNode {
    pub fn new<P: Into<PartialPath>, D: Into<Data>>(path: P, data: D) -> Self {
        Self {
            path: path.into(),
            data: data.into(),
        }
    }

    pub const fn path(&self) -> &PartialPath {
        &self.path
    }

    pub const fn data(&self) -> &Data {
        &self.data
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C, packed)]
struct Meta {
    path_len: PathLen,
    data_len: DataLen,
}

impl Meta {
    const SIZE: usize = size_of::<Self>();
}

impl Storable for LeafNode {
    fn serialized_len(&self) -> u64 {
        let meta_len = size_of::<Meta>() as u64;
        let path_len = self.path.serialized_len();
        let data_len = self.data.len() as u64;

        meta_len + path_len + data_len
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), crate::shale::ShaleError> {
        let mut cursor = Cursor::new(to);

        let path = &self.path.encode();
        let path = from_nibbles_even(path);
        let data = &self.data;

        let path_len = self.path.serialized_len() as PathLen;
        let data_len = data.len() as DataLen;

        let meta = Meta { path_len, data_len };

        cursor.write_all(bytemuck::bytes_of(&meta))?;

        for nibble in path {
            cursor.write_all(&[nibble])?;
        }

        cursor.write_all(data)?;

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
        let Meta { path_len, data_len } = *bytemuck::from_bytes(&node_header_raw);
        let size = path_len as u64 + data_len as u64;

        let remainder = mem
            .get_view(offset, size)
            .ok_or(InvalidCacheView { offset, size })?
            .as_deref();

        let (path, data) = remainder.split_at(path_len as usize);

        let path = {
            let nibbles = Nibbles::<0>::new(path).into_iter();
            PartialPath::from_nibbles(nibbles).0
        };

        let data = Data(data.to_vec());

        Ok(Self::new(path, data))
    }
}
