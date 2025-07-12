use integer_encoding::VarInt;
use std::io::Read;

use crate::{
    LeafNode,
    node::LeafFirstByte,
    serialization::{ExtendableBytes, ExtendableBytesExt, Serializable},
};

const SHORT_PATH_LEN: usize = 0xff;

impl Serializable for LeafFirstByte {
    fn encoded_len_hint(&self) -> Option<usize> {
        Some(size_of::<LeafFirstByte>())
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        vec.push(self.0);
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let mut buf = [0_u8; size_of::<LeafFirstByte>()];
        reader.read_exact(&mut buf)?;
        Ok(LeafFirstByte(buf[0]))
    }
}

impl Serializable for LeafNode {
    fn encoded_len_hint(&self) -> Option<usize> {
        let pp_len = self.partial_path.len();
        size_of::<LeafFirstByte>()
            .checked_add(if pp_len < SHORT_PATH_LEN {
                0
            } else {
                pp_len.required_space()
            })?
            .checked_add(pp_len)?
            .checked_add(self.value.encoded_len_hint()?)
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        let pp_len = self.partial_path.len();
        let first_byte = LeafFirstByte::new(1, Ord::min(pp_len, SHORT_PATH_LEN) as u8);

        first_byte.write_to(vec);

        // encode the partial path, including the length if it didn't fit above
        if pp_len >= SHORT_PATH_LEN {
            vec.extend_from_varint(pp_len);
        }
        vec.extend_from_slice(&self.partial_path);

        // encode the value
        self.value.write_to(vec);
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let first_byte = LeafFirstByte::from_reader(&mut reader)?;

        if first_byte.is_leaf() & 1 == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Expected a LeafFirstByte, but found a non-leaf node",
            ));
        }

        todo!();
    }
}
