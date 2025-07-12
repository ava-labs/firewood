use std::io::Read;

use crate::{
    nodestore::{AreaIndex, alloc::StoredArea},
    serialization::{ExtendableBytes, Serializable},
};

impl<T: Serializable> Serializable for StoredArea<T> {
    fn encoded_len_hint(&self) -> Option<usize> {
        self.area_size_index
            .encoded_len_hint()?
            .checked_add(self.area.encoded_len_hint()?)
    }

    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        self.area_size_index.write_to(vec);
        self.area.write_to(vec);
    }

    fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        Ok(Self {
            area_size_index: AreaIndex::from_reader(&mut reader)?,
            area: T::from_reader(reader)?,
        })
    }
}
