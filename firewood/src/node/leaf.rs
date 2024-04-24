// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::{nibbles_to_bytes_iter, path::Path};
use bincode::Options;
use bytemuck::{Pod, Zeroable};
use std::{
    fmt::{Debug, Error as FmtError, Formatter},
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

    pub(super) fn _encode(&self) -> Vec<u8> {
        #[allow(clippy::unwrap_used)]
        bincode::DefaultOptions::new()
            .serialize(
                [
                    nibbles_to_bytes_iter(&self.partial_path.encode()).collect(),
                    self.value.to_vec(),
                ]
                .as_slice(),
            )
            .unwrap()
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C, packed)]
struct Meta {
    path_len: PathLen,
    value_len: ValueLen,
}

impl Meta {
    const _SIZE: usize = size_of::<Self>();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use test_case::test_case;

    // these tests will fail if the encoding mechanism changes and should be updated accordingly
    //
    // Even length so ODD_LEN flag is not set so flag byte is 0b0000_0000
    #[test_case(0x00, vec![0x12, 0x34], vec![1, 2, 3, 4]; "even length")]
    // Odd length so ODD_LEN flag is set so flag byte is 0b0000_0001
    // This is combined with the first nibble of the path (0b0000_0010) to become 0b0001_0010
    #[test_case(0b0001_0010, vec![0x34], vec![2, 3, 4]; "odd length")]
    fn encode_regression_test(prefix: u8, path: Vec<u8>, nibbles: Vec<u8>) {
        let value = vec![5, 6, 7, 8];

        let serialized_path = [vec![prefix], path.clone()].concat();
        let serialized_path = [vec![serialized_path.len() as u8], serialized_path].concat();
        let serialized_value = [vec![value.len() as u8], value.clone()].concat();

        let serialized = [vec![2], serialized_path, serialized_value].concat();

        let node = LeafNode::new(nibbles, value.clone());

        assert_eq!(node._encode(), serialized);
    }
}
