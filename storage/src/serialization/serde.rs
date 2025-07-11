// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use bincode::{DefaultOptions, Options};
use serde::{Serialize, de::DeserializeOwned};
use std::io::Read;

fn serializer() -> impl Options {
    DefaultOptions::new().with_varint_encoding()
}

pub(crate) trait ReaderExt: Read {
    /// Reads the next `T` from the reader.
    fn read_next<T: DeserializeOwned>(&mut self) -> bincode::Result<T> {
        serializer().deserialize_from(self)
    }
}

impl<T: Read + ?Sized> ReaderExt for T {}

pub(crate) trait SerializeToVec: Serialize {
    /// Serializes the object to a `Vec<u8>`.
    fn serialize_to_vec(&self) -> bincode::Result<Vec<u8>> {
        serializer().serialize(self)
    }
}

impl<T: Serialize + ?Sized> SerializeToVec for T {}
