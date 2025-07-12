// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    BranchNode, Node,
    nodestore::alloc::{Area, FreeArea},
    serialization::{ExtendableBytes, ExtendableBytesExt, Serializable},
};
use integer_encoding::{VarInt, VarIntReader};
use std::io::Read;

pub(crate) trait ReaderExt: Read {
    /// Reads the next `T` from the reader.
    fn read_next<T: Serializable>(&mut self) -> std::io::Result<T> {
        T::from_reader(self)
    }
}

impl<T: Read + ?Sized> ReaderExt for T {}

pub(crate) trait SerializeToVec: Serializable {
    /// Serializes the object to a `Vec<u8>`.
    // NOTE: `Result<_, Infallible>` is used here for parity with the
    // `SerializeToVec` trait when the `serde` feature is enabled.
    fn serialize_to_vec(&self) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut vec = Vec::new();
        if let Some(hint) = self.encoded_len_hint() {
            vec.reserve(hint);
        }
        self.write_to(&mut vec);
        Ok(vec)
    }
}

impl<T: Serializable + ?Sized> SerializeToVec for T {}

impl Serializable for Box<[u8]> {
    fn encoded_len_hint(&self) -> Option<usize> {
        self.len().checked_add(VarInt::required_space(self.len()))
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        vec.extend_from_varint(self.len());
        vec.extend_from_slice(self);
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let len = reader.read_varint::<usize>()?;
        let mut buf = vec![0_u8; len].into_boxed_slice();
        reader.read_exact(&mut buf)?;
        Ok(buf)
    }
}

impl<N: Serializable, F: Serializable> Serializable for Area<N, F> {
    fn encoded_len_hint(&self) -> Option<usize> {
        todo!()
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl Serializable for FreeArea {
    fn encoded_len_hint(&self) -> Option<usize> {
        todo!()
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl Serializable for Node {
    fn encoded_len_hint(&self) -> Option<usize> {
        todo!()
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl Serializable for BranchNode {
    fn encoded_len_hint(&self) -> Option<usize> {
        todo!()
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

macro_rules! impl_serializable_for_varint {
    ($($Type:ty),* $(,)?) => {
        $(
            impl Serializable for $Type {
                fn encoded_len_hint(&self) -> Option<usize> {
                    Some(self.required_space())
                }

                fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
                    vec.extend_from_varint(*self);
                }

                fn from_reader<R: Read>(mut reader: R) -> ::std::io::Result<Self> {
                    reader.read_varint::<Self>()
                }
            }
        )*
    }
}

impl_serializable_for_varint!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

#[cfg(test)]
mod tests {
    use super::*;
}
