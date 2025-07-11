// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Serializable;
use crate::{
    Node,
    nodestore::{
        AreaIndex,
        alloc::{Area, FreeArea, StoredArea},
    },
};
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
        self.write_to(&mut vec);
        Ok(vec)
    }
}

impl<T: Serializable + ?Sized> SerializeToVec for T {}

impl Serializable for AreaIndex {
    fn write_to<W: super::ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl Serializable for StoredArea<Area<Node, FreeArea>> {
    fn write_to<W: super::ExtendableBytes>(&self, vec: &mut W) {
        todo!()
    }

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}
