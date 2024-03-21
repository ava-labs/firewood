// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use crate::{
    merkle::{nibbles_to_bytes_iter, to_nibble_array, Path, TRIE_HASH_LEN},
    shale::{DiskAddress, ShaleError, Storable},
};
use std::{
    fmt::{Debug, Error as FmtError, Formatter},
    io::{Cursor, Read, Write},
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
    pub(crate) children_encoded: [Option<[u8; TRIE_HASH_LEN]>; MAX_CHILDREN],
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

    pub const fn chd_encode(&self) -> &[Option<[u8; TRIE_HASH_LEN]>; Self::MAX_CHILDREN] {
        &self.children_encoded
    }

    pub fn chd_encoded_mut(&mut self) -> &mut [Option<[u8; TRIE_HASH_LEN]>; Self::MAX_CHILDREN] {
        &mut self.children_encoded
    }
}

impl Storable for BranchNode {
    fn serialized_len(&self) -> u64 {
        let children_len = Self::MAX_CHILDREN as u64 * DiskAddress::SERIALIZED_LEN;
        let value_len = optional_value_len::<ValueLen, _>(self.value.as_deref());
        let children_encoded_len = self.children_encoded.iter().fold(0, |len, child| {
            len + optional_value_len::<EncodedChildLen, _>(child.as_ref())
        });
        let path_len_size = size_of::<PathLen>() as u64;
        let path_len = self.partial_path.serialized_len();

        children_len + value_len + children_encoded_len + path_len_size + path_len
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), crate::shale::ShaleError> {
        let mut cursor = Cursor::new(to);

        let path: Vec<u8> = nibbles_to_bytes_iter(&self.partial_path.encode()).collect();
        cursor.write_all(&[path.len() as PathLen])?;
        cursor.write_all(&path)?;

        for child in &self.children {
            let bytes = child.map(|addr| addr.to_le_bytes()).unwrap_or_default();
            cursor.write_all(&bytes)?;
        }

        let (value_len, value) = self
            .value
            .as_ref()
            .map(|val| (val.len() as ValueLen, &**val))
            .unwrap_or((ValueLen::MAX, &[]));

        cursor.write_all(&value_len.to_le_bytes())?;
        cursor.write_all(value)?;

        for child_encoded in &self.children_encoded {
            let (child_len, child) = child_encoded
                .as_ref()
                .map(|child| (child.len() as EncodedChildLen, child.as_slice()))
                .unwrap_or((EncodedChildLen::MIN, &[]));

            cursor.write_all(&child_len.to_le_bytes())?;
            cursor.write_all(child)?;
        }

        Ok(())
    }

    fn deserialize<T: crate::shale::CachedStore>(
        mut addr: usize,
        mem: &T,
    ) -> Result<Self, crate::shale::ShaleError> {
        const PATH_LEN_SIZE: u64 = size_of::<PathLen>() as u64;
        const VALUE_LEN_SIZE: usize = size_of::<ValueLen>();
        const BRANCH_HEADER_SIZE: u64 =
            BranchNode::MAX_CHILDREN as u64 * DiskAddress::SERIALIZED_LEN + VALUE_LEN_SIZE as u64;

        let path_len = mem
            .get_view(addr, PATH_LEN_SIZE)
            .ok_or(ShaleError::InvalidCacheView {
                offset: addr,
                size: PATH_LEN_SIZE,
            })?
            .as_deref();

        addr += PATH_LEN_SIZE as usize;

        let path_len = {
            let mut buf = [0u8; PATH_LEN_SIZE as usize];
            let mut cursor = Cursor::new(path_len);
            cursor.read_exact(buf.as_mut())?;

            PathLen::from_le_bytes(buf) as u64
        };

        let path = mem
            .get_view(addr, path_len)
            .ok_or(ShaleError::InvalidCacheView {
                offset: addr,
                size: path_len,
            })?
            .as_deref();

        addr += path_len as usize;

        let path: Vec<u8> = path.into_iter().flat_map(to_nibble_array).collect();
        let path = Path::decode(&path);

        let node_raw =
            mem.get_view(addr, BRANCH_HEADER_SIZE)
                .ok_or(ShaleError::InvalidCacheView {
                    offset: addr,
                    size: BRANCH_HEADER_SIZE,
                })?;

        addr += BRANCH_HEADER_SIZE as usize;

        let mut cursor = Cursor::new(node_raw.as_deref());
        let mut children = [None; BranchNode::MAX_CHILDREN];
        let mut buf = [0u8; DiskAddress::SERIALIZED_LEN as usize];

        for child in &mut children {
            cursor.read_exact(&mut buf)?;
            *child = Some(usize::from_le_bytes(buf))
                .filter(|addr| *addr != 0)
                .map(DiskAddress::from);
        }

        let raw_len = {
            let mut buf = [0; VALUE_LEN_SIZE];
            cursor.read_exact(&mut buf)?;
            Some(ValueLen::from_le_bytes(buf))
                .filter(|len| *len != ValueLen::MAX)
                .map(|len| len as u64)
        };

        let value = match raw_len {
            Some(len) => {
                let value = mem
                    .get_view(addr, len)
                    .ok_or(ShaleError::InvalidCacheView {
                        offset: addr,
                        size: len,
                    })?;

                addr += len as usize;

                Some(value.as_deref())
            }
            None => None,
        };

        let mut children_encoded: [Option<[u8; TRIE_HASH_LEN]>; BranchNode::MAX_CHILDREN] =
            Default::default();

        for child in &mut children_encoded {
            const ENCODED_CHILD_LEN_SIZE: u64 = size_of::<EncodedChildLen>() as u64;

            let len_raw = mem
                .get_view(addr, ENCODED_CHILD_LEN_SIZE)
                .ok_or(ShaleError::InvalidCacheView {
                    offset: addr,
                    size: ENCODED_CHILD_LEN_SIZE,
                })?
                .as_deref();

            let mut cursor = Cursor::new(len_raw);

            let len = {
                let mut buf = [0; ENCODED_CHILD_LEN_SIZE as usize];
                cursor.read_exact(buf.as_mut())?;
                EncodedChildLen::from_le_bytes(buf) as u64
            };

            addr += ENCODED_CHILD_LEN_SIZE as usize;

            if len == 0 {
                continue;
            }

            let encoded = mem
                .get_view(addr, len)
                .ok_or(ShaleError::InvalidCacheView {
                    offset: addr,
                    size: len,
                })?
                .as_deref();

            let encoded: [u8; TRIE_HASH_LEN] = encoded.try_into().expect("TODO");

            addr += len as usize;

            *child = Some(encoded);
        }

        let node = BranchNode {
            partial_path: path,
            children,
            value,
            children_encoded,
        };

        Ok(node)
    }
}

fn optional_value_len<Len, T: AsRef<[u8]>>(value: Option<T>) -> u64 {
    size_of::<Len>() as u64
        + value
            .as_ref()
            .map_or(0, |value| value.as_ref().len() as u64)
}
