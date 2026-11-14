// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Spans of key space, named by nibble prefix.
//!
//! A [`KeySpan`] names a contiguous span of the key space by its nibble
//! prefix, for example the key space under one child edge of a trie node.
//! Spans convert to byte-key ranges ([`KeySpan::as_key_range`]) and to the
//! byte prefixes a [`BatchOp::DeleteRange`] accepts
//! ([`KeySpan::delete_prefixes`]).
//!
//! [`BatchOp::DeleteRange`]: crate::api::BatchOp::DeleteRange

use firewood_storage::Children;
use firewood_storage::PathBuf;
use firewood_storage::TriePathAsPackedBytes;
use firewood_storage::prefix_upper_bound;

/// A contiguous span of key space: all keys carrying a nibble prefix.
///
/// Nibble prefixes may be odd-length (a branch child edge adds one nibble to
/// its parent's even- or odd-length path), and an odd-length nibble prefix
/// has no byte-prefix representation — which is why this type owns both
/// conversions instead of exposing the prefix and leaving them to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpan {
    prefix: PathBuf,
}

impl KeySpan {
    /// Creates a span from its nibble prefix.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "exercised only by tests until the hole-detection walk calls it"
        )
    )]
    pub(crate) const fn new(prefix: PathBuf) -> Self {
        Self { prefix }
    }

    /// Half-open byte-key range `[lower, upper)` covering exactly the keys
    /// carrying this span's nibble prefix.
    ///
    /// The lower bound is the smallest byte key carrying the prefix: the
    /// packed prefix itself when even-length, the prefix zero-padded to even
    /// length otherwise. The upper bound is the packed
    /// [`prefix_upper_bound`] (zero-padded the same way when odd), or `None`
    /// when the prefix is empty or all-`F` and the span is unbounded above.
    ///
    /// The upper bound is **exclusive**. Do not pass it as an inclusive
    /// bound (such as `Db::merge_key_value_range`'s `last_key`), which would
    /// extend the range by one key.
    #[must_use]
    pub fn as_key_range(&self) -> (Box<[u8]>, Option<Box<[u8]>>) {
        (
            self.prefix.as_packed_bytes().collect(),
            prefix_upper_bound(&self.prefix).map(|bound| bound.as_packed_bytes().collect()),
        )
    }

    /// The **byte** prefixes whose union is exactly this span, ready to hand
    /// to [`BatchOp::DeleteRange`](crate::api::BatchOp::DeleteRange).
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
/// one completion per nibble, stored as [`Children`] (one slot per possible
/// completing nibble).
///
/// Deliberately not `#[non_exhaustive]`. The two arms carry different deletion
/// geometry with no sensible default branch, so callers must match both; and no
/// third shape is possible, because a nibble prefix is either even-length or it
/// is not.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "`PerNibble` holds sixteen boxed slices inline; boxing it would only shrink the \
              enum, since the odd arm already allocates sixteen times, and callers match the \
              plain shape"
)]
pub enum DeletePrefixes {
    /// The nibble prefix is even-length and packs to a single byte prefix.
    Whole(Box<[u8]>),
    /// The nibble prefix is odd-length: one byte prefix per completing nibble,
    /// each completion being even-length and therefore packable.
    PerNibble(Children<Box<[u8]>>),
}

impl IntoIterator for DeletePrefixes {
    type Item = Box<[u8]>;
    type IntoIter = Box<dyn Iterator<Item = Box<[u8]>> + Send>;

    /// Yields every prefix, whichever variant this is.
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
    use firewood_storage::PathComponent;

    use super::*;

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
        // Lower: pack([A,7,1] ++ [0]). Upper: bound [A,7,2] is odd, so
        // pack([A,7,2] ++ [0]) — truncating instead would under-cover.
        let (lower, upper) = span(&[0xA, 0x7, 0x1]).as_key_range();
        assert_eq!(&*lower, &[0xA7, 0x10]);
        assert_eq!(upper.as_deref(), Some([0xA7, 0x20].as_slice()));
    }

    #[test]
    fn trailing_max_component_flips_successor_parity() {
        // `prefix_upper_bound([A,F])` is `[B]` (odd), so the upper bound is
        // pack([B,0]).
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
        let completions: Vec<_> = completions.into_iter().collect();
        assert_eq!(completions.len(), 16, "one completion per nibble");
        for (nibble, prefix) in completions {
            assert_eq!(&*prefix, &[0xA7, 0x10 | nibble.as_u8()]);
        }
    }

    #[test]
    fn empty_prefix_deletes_everything() {
        // The empty span is the whole key space, so its single prefix is empty
        // and matches every key.
        let DeletePrefixes::Whole(prefix) = span(&[]).delete_prefixes() else {
            panic!("the empty prefix is even-length");
        };
        assert!(prefix.is_empty());
    }

    /// The union of the returned byte prefixes covers exactly
    /// `as_key_range`'s half-open interval, checked exhaustively over all 0-,
    /// 1-, and 2-byte keys plus 3-byte spot keys.
    fn assert_prefixes_match_range(span: &KeySpan) {
        let (lower, upper) = span.as_key_range();
        let prefixes: Vec<Box<[u8]>> = span.delete_prefixes().into_iter().collect();

        let mut keys: Vec<Vec<u8>> = vec![Vec::new()];
        keys.extend((0u8..=u8::MAX).map(|b| vec![b]));
        keys.extend((0u16..=u16::MAX).map(|k| k.to_be_bytes().to_vec()));
        keys.extend((0u16..=u16::MAX).step_by(16).map(|k| {
            let mut key = k.to_be_bytes().to_vec();
            key.push(0x5A);
            key
        }));

        let mut in_range_count = 0usize;
        for key in keys {
            let in_range =
                key.as_slice() >= &*lower && upper.as_deref().is_none_or(|u| key.as_slice() < u);
            let covered = prefixes.iter().any(|p| key.starts_with(p));
            assert_eq!(in_range, covered, "key {key:02x?}");
            if in_range {
                in_range_count = in_range_count.saturating_add(1);
            }
        }
        assert!(
            in_range_count > 0,
            "no key in the test universe lies inside this span"
        );
    }

    #[test]
    fn delete_prefixes_union_equals_key_range_even() {
        assert_prefixes_match_range(&span(&[]));
        assert_prefixes_match_range(&span(&[0xA, 0x7]));
        assert_prefixes_match_range(&span(&[0x0, 0x0]));
        assert_prefixes_match_range(&span(&[0xF, 0xF]));
        // Four nibbles: both bounds are two bytes (`a713`..`a714`), so the
        // exhaustive two-byte keys bracket the boundary exactly, independent
        // of the sampled three-byte keys.
        assert_prefixes_match_range(&span(&[0xA, 0x7, 0x1, 0x3]));
    }

    #[test]
    fn delete_prefixes_union_equals_key_range_odd() {
        assert_prefixes_match_range(&span(&[0xA]));
        assert_prefixes_match_range(&span(&[0xA, 0x7, 0x1]));
        assert_prefixes_match_range(&span(&[0xF]));
        assert_prefixes_match_range(&span(&[0xA, 0xF, 0xF]));
        assert_prefixes_match_range(&span(&[0x0]));
    }
}
