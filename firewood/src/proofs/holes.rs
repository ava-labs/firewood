// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Spans of key space for post-merge hole detection.
//!
//! A [`KeySpan`] names a contiguous span of the key space by its nibble
//! prefix — the shape a trie's sealed sibling stubs commit to. Spans convert
//! to byte-key ranges ([`KeySpan::as_key_range`]) and to the byte prefixes a
//! [`BatchOp::DeleteRange`] accepts ([`KeySpan::delete_prefixes`]).
//!
//! [`BatchOp::DeleteRange`]: crate::api::BatchOp::DeleteRange

use firewood_storage::{Children, PathBuf, TriePathAsPackedBytes, prefix_successor};

/// A contiguous span of key space: all keys carrying a nibble prefix.
///
/// Nibble prefixes may be odd-length (a branch child edge adds one nibble to
/// its parent's even- or odd-length path), and an odd-length nibble prefix
/// has no byte-prefix representation — which is why this type owns both
/// conversions instead of exposing the prefix and leaving them to callers.
/// `#[non_exhaustive]` here is belt-and-braces: the private field and
/// `pub(crate)` constructor already block external construction and exhaustive
/// matching. It is kept because the merge gates require it, and because it
/// documents that the internal representation is not part of the contract.
/// Do not delete it as redundant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeySpan {
    prefix: PathBuf,
}

impl KeySpan {
    /// Creates a span from its nibble prefix.
    // No production caller exists yet: PR2's hole-detection walk constructs
    // these. Tests already call this, so drop the allow once that walk lands.
    #[allow(dead_code)]
    pub(crate) const fn new(prefix: PathBuf) -> Self {
        Self { prefix }
    }

    /// Half-open byte-key range `[lower, upper)` covering exactly the keys
    /// carrying this span's nibble prefix.
    ///
    /// The lower bound is the smallest byte key carrying the prefix: the
    /// packed prefix itself when even-length, the prefix zero-padded to even
    /// length otherwise. The upper bound is the packed nibble-prefix
    /// successor (zero-padded the same way when odd), or `None` when the
    /// prefix is empty or all-`F` and the span is unbounded above.
    ///
    /// The upper bound is **exclusive**. Do not pass it as an inclusive
    /// bound (such as `Db::merge_key_value_range`'s `last_key`), which would
    /// extend the range by one key.
    #[must_use]
    pub fn as_key_range(&self) -> (Box<[u8]>, Option<Box<[u8]>>) {
        (
            self.prefix.as_packed_bytes().collect(),
            prefix_successor(&self.prefix).map(|successor| successor.as_packed_bytes().collect()),
        )
    }

    /// The **byte** prefixes whose union is exactly this span, ready to hand
    /// to `BatchOp::DeleteRange`.
    ///
    /// This is the supported way to apply a span-shaped deletion through the
    /// write API: the byte range from [`Self::as_key_range`] does not
    /// unambiguously encode the nibble prefix, so callers cannot rebuild
    /// these prefixes from it.
    #[must_use]
    pub fn delete_prefixes(&self) -> DeletePrefixes {
        if self.prefix.len().is_multiple_of(2) {
            DeletePrefixes::Whole(self.prefix.as_packed_bytes().collect())
        } else {
            DeletePrefixes::PerNibble(Children::from_fn(|completion| {
                let mut completed = self.prefix.clone();
                completed.push(completion);
                completed.as_packed_bytes().collect()
            }))
        }
    }
}

/// The byte prefixes covering a [`KeySpan`], shaped by the parity of its
/// nibble prefix.
///
/// An odd-length nibble prefix has no byte-prefix form, so it decomposes into
/// one completion per nibble — which is why the odd arm is a
/// [`Children`] rather than a list: there is exactly one entry per possible
/// completing nibble, and the type says so.
///
/// Deliberately not `#[non_exhaustive]`. The two arms carry different deletion
/// geometry with no sensible default branch, so callers must match both; and no
/// third shape is possible, because a nibble prefix is either even-length or it
/// is not.
#[derive(Debug, Clone, PartialEq, Eq)]
// `Children<Box<[u8]>>` is a fixed 16-slot array (one fat pointer per nibble),
// so `PerNibble` is unavoidably larger than `Whole`. Boxing it would trade a
// one-time allocation for a smaller enum footprint solely to satisfy this
// lint; that trade isn't worth making for a value returned once per call.
#[allow(clippy::large_enum_variant)]
pub enum DeletePrefixes {
    /// The nibble prefix is even-length and packs to a single byte prefix.
    Whole(Box<[u8]>),
    /// The nibble prefix is odd-length: one byte prefix per completing nibble,
    /// each completion being even-length and therefore packable.
    PerNibble(Children<Box<[u8]>>),
}

impl IntoIterator for DeletePrefixes {
    type Item = Box<[u8]>;
    type IntoIter = Box<dyn Iterator<Item = Box<[u8]>>>;

    /// Yields the prefixes regardless of arm, for callers that only want to
    /// apply every one of them.
    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Whole(prefix) => Box::new(std::iter::once(prefix)),
            Self::PerNibble(completions) => {
                Box::new(completions.into_iter().map(|(_nibble, prefix)| prefix))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firewood_storage::PathComponent;

    fn span(nibbles: &[u8]) -> KeySpan {
        KeySpan::new(
            nibbles
                .iter()
                .map(|&n| PathComponent::try_new(n).expect("test nibble in range"))
                .collect(),
        )
    }

    #[test]
    fn even_prefix_range_is_packed_bytes_to_packed_successor() {
        let (lower, upper) = span(&[0xA, 0x7]).as_key_range();
        assert_eq!(&*lower, &[0xA7]);
        assert_eq!(upper.as_deref(), Some([0xA8].as_slice()));
    }

    #[test]
    fn odd_prefix_range_zero_pads_both_bounds() {
        // Lower: pack([A,7,1] ++ [0]). Upper: successor [A,7,2] is odd, so
        // pack([A,7,2] ++ [0]) — truncating instead would under-cover.
        let (lower, upper) = span(&[0xA, 0x7, 0x1]).as_key_range();
        assert_eq!(&*lower, &[0xA7, 0x10]);
        assert_eq!(upper.as_deref(), Some([0xA7, 0x20].as_slice()));
    }

    #[test]
    fn trailing_max_component_flips_successor_parity() {
        // succ([A,F]) = [B] (odd), so the upper bound is pack([B,0]).
        let (lower, upper) = span(&[0xA, 0xF]).as_key_range();
        assert_eq!(&*lower, &[0xAF]);
        assert_eq!(upper.as_deref(), Some([0xB0].as_slice()));
    }

    #[test]
    fn all_max_prefix_is_unbounded_above() {
        let (lower, upper) = span(&[0xF, 0xF]).as_key_range();
        assert_eq!(&*lower, &[0xFF]);
        assert_eq!(upper, None);
    }

    #[test]
    fn empty_prefix_covers_the_whole_key_space() {
        let (lower, upper) = span(&[]).as_key_range();
        assert!(lower.is_empty());
        assert_eq!(upper, None);
    }

    #[test]
    fn even_prefix_deletes_as_one_byte_prefix() {
        let DeletePrefixes::Whole(prefix) = span(&[0xA, 0x7]).delete_prefixes() else {
            panic!("an even-length nibble prefix packs to a single byte prefix");
        };
        assert_eq!(&*prefix, &[0xA7]);
    }

    #[test]
    fn odd_prefix_deletes_as_one_completion_per_nibble() {
        let DeletePrefixes::PerNibble(completions) = span(&[0xA, 0x7, 0x1]).delete_prefixes()
        else {
            panic!("an odd-length nibble prefix has no single byte-prefix form");
        };
        for (nibble, prefix) in completions {
            assert_eq!(&*prefix, &[0xA7, 0x10 | nibble.as_u8()]);
        }
    }

    #[test]
    fn empty_prefix_deletes_everything() {
        // The empty prefix is even-length, so this is a single empty byte
        // prefix — which as a DeleteRange argument matches every key. That is
        // correct (the span *is* the whole key space) and load-bearing enough
        // to pin: a caller handing this to DeleteRange wipes the database, so
        // the emptiness must be a deliberate, tested property rather than an
        // emergent one a future refactor could quietly change.
        let DeletePrefixes::Whole(prefix) = span(&[]).delete_prefixes() else {
            panic!("the empty prefix is even-length");
        };
        assert!(prefix.is_empty());
    }

    /// The gate property (merge-gates § PR1): the union of the returned byte
    /// prefixes covers exactly `as_key_range`'s half-open interval, checked
    /// exhaustively over all 0-, 1-, and 2-byte keys plus 3-byte spot keys.
    fn assert_prefixes_match_range(span: &KeySpan) {
        let (lower, upper) = span.as_key_range();
        let prefixes: Vec<Box<[u8]>> = span.delete_prefixes().into_iter().collect();

        let mut keys: Vec<Vec<u8>> = vec![Vec::new()];
        keys.extend((0u8..=u8::MAX).map(|b| vec![b]));
        keys.extend((0u16..=u16::MAX).map(|k| k.to_be_bytes().to_vec()));
        keys.extend((0u16..=u16::MAX).step_by(251).map(|k| {
            let mut key = k.to_be_bytes().to_vec();
            key.push(0x5A);
            key
        }));

        for key in keys {
            let in_range =
                key.as_slice() >= &*lower && upper.as_deref().is_none_or(|u| key.as_slice() < u);
            let covered = prefixes.iter().any(|p| key.starts_with(p));
            assert_eq!(in_range, covered, "key {key:02x?}");
        }
    }

    #[test]
    fn delete_prefixes_union_equals_key_range_even() {
        assert_prefixes_match_range(&span(&[]));
        assert_prefixes_match_range(&span(&[0xA, 0x7]));
        assert_prefixes_match_range(&span(&[0x0, 0x0]));
        assert_prefixes_match_range(&span(&[0xF, 0xF]));
    }

    #[test]
    fn delete_prefixes_union_equals_key_range_odd() {
        assert_prefixes_match_range(&span(&[0xA]));
        assert_prefixes_match_range(&span(&[0xA, 0x7, 0x1]));
        assert_prefixes_match_range(&span(&[0xF]));
        assert_prefixes_match_range(&span(&[0xA, 0xF, 0xF]));
    }
}
