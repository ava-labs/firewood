// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Serializable;
#[expect(
    deprecated,
    reason = "Only used for backwards compatibility with v0.0.8 and earlier"
)]
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

// Basic type implementations
impl Serializable for u64 {
    fn write_to<W: super::ExtendableBytes>(&self, vec: &mut W) {
        vec.extend_from_slice(&self.to_le_bytes());
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }
}

impl Serializable for AreaIndex {
    fn write_to<W: super::ExtendableBytes>(&self, vec: &mut W) {
        vec.push(*self);
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf)?;
        Ok(buf[0])
    }
}

#[expect(
    deprecated,
    reason = "Only used for backwards compatibility with v0.0.8 and earlier"
)]
impl Serializable for StoredArea<Area<Node, FreeArea>> {
    fn write_to<W: super::ExtendableBytes>(&self, vec: &mut W) {
        // This is a backwards compatibility implementation only
        // We need to clone the data to access it since fields are private
        let (area_size_index, area) = self.clone().into_parts();

        // Write area size index
        area_size_index.write_to(vec);

        // Write the Area enum
        match &area {
            #[expect(
                deprecated,
                reason = "Only used for backwards compatibility with v0.0.8 and earlier"
            )]
            Area::Free(free_area) => {
                // Write discriminant for Free variant (255)
                vec.push(0xFF);

                // Write the FreeArea data (next_free_block as Option<LinearAddress>)
                match free_area.next_free_block() {
                    Some(addr) => {
                        vec.push(1); // Some variant
                        addr.get().write_to(vec);
                    }
                    None => {
                        vec.push(0); // None variant
                    }
                }
            }
            Area::Node(_) => {
                // This should never happen in practice since Area::Node is never used for free areas
                unreachable!("Area::Node should never be serialized in a FreeArea context")
            }
        }
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        // Read area size index
        let area_size_index = AreaIndex::from_reader(&mut reader)?;

        // Read discriminant
        let mut discriminant = [0u8; 1];
        reader.read_exact(&mut discriminant)?;

        match discriminant[0] {
            0xFF => {
                // Free variant
                let mut has_next = [0u8; 1];
                reader.read_exact(&mut has_next)?;

                let next_free_block = match has_next[0] {
                    1 => {
                        // Some variant - read the LinearAddress
                        let addr = u64::from_reader(&mut reader)?;
                        crate::nodestore::LinearAddress::new(addr)
                    }
                    0 => None, // None variant
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid Option discriminant: {}", has_next[0]),
                        ));
                    }
                };

                let free_area = FreeArea::new(next_free_block);
                #[expect(
                    deprecated,
                    reason = "Only used for backwards compatibility with v0.0.8 and earlier"
                )]
                let area = Area::Free(free_area);
                Ok(StoredArea::new(area_size_index, area))
            }
            0 => {
                // Node variant - this should never happen in practice
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Encountered Area::Node in free area context - this should never happen",
                ))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid Area discriminant: {}", discriminant[0]),
            )),
        }
    }
}
