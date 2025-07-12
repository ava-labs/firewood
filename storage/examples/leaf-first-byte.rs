use bincode::{DefaultOptions, Options};
use bitfield::bitfield;
use firewood_storage::{LinearAddress, Node};

bitfield! {
    struct LeafFirstByte(u8);
    impl Debug;
    impl new;
    u8;
    is_leaf, set_is_leaf: 0, 0;
    partial_path_length, set_partial_path_length: 7, 1;
}

#[repr(u8)]
#[derive(PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Area<T, U> {
    Node(T),
    Free(U) = 255, // this is magic: no node starts with a byte of 255
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct StoredArea<T> {
    /// Index in [`AREA_SIZES`] of this area's size
    area_size_index: u8,
    area: T,
}

impl<T> StoredArea<T> {
    const fn new(area_size_index: u8, area: T) -> Self {
        Self {
            area_size_index,
            area,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FreeArea {
    next_free_block: Option<LinearAddress>,
}

impl FreeArea {
    const fn new(next_free_block: Option<LinearAddress>) -> Self {
        Self { next_free_block }
    }
}

#[expect(clippy::unwrap_used)]
fn main() {
    for b in 0..=u8::MAX {
        let first_byte = LeafFirstByte(b);
        let is_leaf = first_byte.is_leaf();
        let pp_len = first_byte.partial_path_length();
        println!(
            "{b:#04x} ({b:#010b}): is_leaf={is_leaf:01b}, partial_path_length={pp_len:#09b} ({pp_len:#04x})",
        );
    }

    println!();

    for b in 0..=u8::MAX {
        let first_byte = LeafFirstByte::new(1, b);
        let b = first_byte.0;
        let is_leaf = first_byte.is_leaf();
        let pp_len = first_byte.partial_path_length();
        println!(
            "{b:#04x} ({b:#010b}): is_leaf={is_leaf:01b}, partial_path_length={pp_len:#09b} ({pp_len:#04x})",
        );
    }

    let node = StoredArea::new(
        12,
        Area::<Node, _>::Free(FreeArea::new(LinearAddress::new(42))),
    );
    let mut buf = Vec::new();
    DefaultOptions::new()
        .with_fixint_encoding()
        .serialize_into(&mut buf, &node)
        .unwrap();
    println!("Serialized StoredArea: {buf:#04x?}");

    buf.clear();
    DefaultOptions::new()
        .with_varint_encoding()
        .serialize_into(&mut buf, &node)
        .unwrap();
    println!("VarInt Serialized StoredArea: {buf:#04x?}");
}
