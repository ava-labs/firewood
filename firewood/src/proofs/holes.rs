// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Spans of key space for post-merge hole detection.
//!
//! A [`KeySpan`] names a contiguous span of the key space by its nibble
//! prefix — the shape a trie's sealed sibling stubs commit to. Spans convert
//! to byte-key ranges ([`KeySpan::as_key_range`]) and to the byte prefixes a
//! [`BatchOp::DeleteRange`] accepts ([`KeySpan::delete_prefixes`]). A
//! [`Hole`] labels a span or a single key with the remedy that brings the
//! local trie into agreement with the target there.
//!
//! [`BatchOp::DeleteRange`]: crate::api::BatchOp::DeleteRange

use firewood_storage::{Children, PathBuf, PathComponent, TriePathAsPackedBytes, prefix_successor};

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
    pub(crate) const fn new(prefix: PathBuf) -> Self {
        Self { prefix }
    }

    /// The nibble prefix naming this span.
    pub(crate) fn prefix(&self) -> &[PathComponent] {
        &self.prefix
    }

    /// The nibble prefix naming this span, by value.
    pub(crate) fn into_prefix(self) -> PathBuf {
        self.prefix
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
#[expect(
    clippy::large_enum_variant,
    reason = "`Children<Box<[u8]>>` is a fixed 16-slot array (one fat pointer per nibble), so \
              `PerNibble` is unavoidably larger than `Whole`; boxing it would not trade away an \
              allocation (the odd arm already performs sixteen), only enum footprint, and a \
              directly pattern-matchable shape serves callers better than a smaller enum"
)]
pub enum DeletePrefixes {
    /// The nibble prefix is even-length and packs to a single byte prefix.
    Whole(Box<[u8]>),
    /// The nibble prefix is odd-length: one byte prefix per completing nibble,
    /// each completion being even-length and therefore packable.
    PerNibble(Children<Box<[u8]>>),
}

/// One classified span or point of the key space outside a proven range.
///
/// Post-merge hole detection compares the sealed sibling hashes a verified
/// boundary proof carries against the local trie and labels every part of
/// the key space the proof did not cover. Each variant names the remedy that
/// brings the local trie into agreement with the target there. A consumer
/// must apply every remedy it is handed or its local state diverges, which is
/// why this enum is closed: adding a label is a breaking change for every
/// correct consumer, and a wildcard arm would turn that break into a silent
/// root-hash mismatch.
///
/// Two labels overlap only when both are deletions, so the order in which
/// remedies are applied does not matter: deleting a span twice is idempotent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hole {
    /// The target holds keys under this span and the local trie holds none.
    /// Remedy: fetch the span, with a range proof.
    Missing(KeySpan),
    /// Both tries hold keys under this span and their contents differ.
    /// Remedy: fetch the span.
    Stale(KeySpan),
    /// The target provably holds no keys under this span and the local trie
    /// holds some. Remedy: delete the span locally through
    /// [`KeySpan::delete_prefixes`]; the absence is authenticated, so nothing
    /// is fetched. The span may extend past the range the proof was applied
    /// to: the target is empty over its whole extent, so deleting all of it
    /// cannot remove a key the proof wrote.
    Surplus(KeySpan),
    /// The local subtree hash under this span equals the target's sealed
    /// sibling hash, which verifies the local content equal to the target's
    /// with the strength of a range proof over the span. Remedy: record the
    /// span as synchronized at this root and do not fetch it again.
    Synced(KeySpan),
    /// An exact key whose target value is in hand from a verified proof node.
    /// Remedy: write the value locally; nothing is fetched.
    PointFix {
        /// The key.
        key: Box<[u8]>,
        /// The target's value for the key.
        value: Box<[u8]>,
    },
    /// An exact key whose target value differs from the local one but is
    /// known only by digest: under the MerkleDB hashing scheme, a value of 32
    /// bytes or more appears in a proof as its hash. Remedy: fetch the key.
    PointStale {
        /// The key.
        key: Box<[u8]>,
    },
    /// An exact key the target provably lacks and the local trie holds.
    /// Remedy: delete the key locally.
    PointSurplus {
        /// The key.
        key: Box<[u8]>,
    },
}

/// The key space strictly between two nibble paths, decomposed into the
/// pieces a hole-detection walk labels: nibble-prefix spans plus the byte
/// keys that are proper prefixes of the upper bound.
///
/// A proper prefix of a key sorts before it but lies under no span that
/// excludes it, so those keys are reported separately as points. Only
/// even-length prefixes are keys, since keys are byte strings.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct OpenInterval {
    /// Pairwise incomparable: no span's prefix is a prefix of another's.
    pub(crate) spans: Vec<KeySpan>,
    /// Byte keys inside the interval that no span covers.
    pub(crate) points: Vec<Box<[u8]>>,
}

/// Decomposes the open interval `(lower_exclusive, upper_exclusive)` of the
/// key space into spans and points whose union is exactly that interval.
///
/// With no lower bound the interval is everything below `upper_exclusive`,
/// the shape an empty start proof leaves below the requested start key. With
/// both bounds it is the gap between a proven right edge and the first
/// target key above it. An empty interval — `lower_exclusive` at or above
/// `upper_exclusive` — yields nothing.
///
/// The decomposition follows the two bounds' nibble paths: at the first
/// offset where they disagree, the nibbles strictly between them; above the
/// lower bound, at each deeper offset, the nibbles above its own; below the
/// upper bound, at each deeper offset, the nibbles below its own; and every
/// nibble extending the lower bound itself, since those keys all sort above
/// it. Each piece is a span. The upper bound's proper prefixes deeper than
/// the divergence are the points.
pub(crate) fn open_interval(
    lower_exclusive: Option<&[PathComponent]>,
    upper_exclusive: &[PathComponent],
) -> OpenInterval {
    let mut interval = OpenInterval::default();
    let mut span = |prefix: &[PathComponent], depth: usize, last: PathComponent| {
        let mut path: PathBuf = prefix.iter().take(depth).copied().collect();
        path.push(last);
        interval.spans.push(KeySpan::new(path));
    };

    // `spans_from`: the offset from which the upper bound's own path is
    // walked. `points_from`: the offset from which its proper prefixes lie
    // strictly above the lower bound.
    let (spans_from, points_from) = match lower_exclusive {
        None => (0, 0),
        Some(lower) => {
            let common = lower
                .iter()
                .zip(upper_exclusive)
                .take_while(|(a, b)| a == b)
                .count();
            // `common` never exceeds the shorter bound's length, so stepping
            // past it cannot overflow.
            debug_assert!(common <= lower.len().min(upper_exclusive.len()));
            let next = common.wrapping_add(1);
            match (lower.get(common), upper_exclusive.get(common)) {
                // The lower bound is a proper prefix of the upper bound: every
                // key between them extends the lower bound and sorts below the
                // upper, which the upper-bound walk from `common` covers. The
                // prefix at `common` is the lower bound itself, not a point.
                (None, Some(_)) => (common, next),
                // The bounds diverge with the lower one smaller.
                (Some(&low), Some(&high)) if low < high => {
                    for n in PathComponent::ALL
                        .into_iter()
                        .filter(|n| *n > low && *n < high)
                    {
                        span(upper_exclusive, common, n);
                    }
                    for (offset, &own) in lower.iter().enumerate().skip(next) {
                        for n in PathComponent::ALL.into_iter().filter(|n| *n > own) {
                            span(lower, offset, n);
                        }
                    }
                    for n in PathComponent::ALL {
                        span(lower, lower.len(), n);
                    }
                    (next, next)
                }
                // Equal bounds, or the lower bound at or above the upper one.
                _ => return interval,
            }
        }
    };

    for (offset, &own) in upper_exclusive.iter().enumerate().skip(spans_from) {
        if offset >= points_from && offset.is_multiple_of(2) {
            let prefix: PathBuf = upper_exclusive.iter().take(offset).copied().collect();
            interval.points.push(prefix.as_packed_bytes().collect());
        }
        for n in PathComponent::ALL.into_iter().filter(|n| *n < own) {
            span(upper_exclusive, offset, n);
        }
    }
    interval
}

impl IntoIterator for DeletePrefixes {
    type Item = Box<[u8]>;
    type IntoIter = Box<dyn Iterator<Item = Box<[u8]>> + Send>;

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
        let completions: Vec<_> = completions.into_iter().collect();
        assert_eq!(completions.len(), 16, "one completion per nibble");
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

    /// The union of the returned byte prefixes covers exactly
    /// `as_key_range`'s half-open interval, checked exhaustively over all 0-,
    /// 1-, and 2-byte keys plus 3-byte spot keys.
    fn assert_prefixes_match_range(span: &KeySpan) {
        let (lower, upper) = span.as_key_range();
        let prefixes: Vec<Box<[u8]>> = span.delete_prefixes().into_iter().collect();

        let mut in_range_count = 0usize;
        for key in key_universe() {
            let in_range =
                key.as_slice() >= &*lower && upper.as_deref().is_none_or(|u| key.as_slice() < u);
            let covered = prefixes.iter().any(|p| key.starts_with(p));
            assert_eq!(in_range, covered, "key {key:02x?}");
            if in_range {
                in_range_count = in_range_count.saturating_add(1);
            }
        }

        // Without this, a span whose bounds sit outside the key universe above
        // passes vacuously: every assertion compares `false == false` and the
        // call advertises coverage it does not have. A span deeper than four
        // nibbles has three-byte bounds, and the only three-byte keys here are
        // stepped and end in a pinned byte, so unreachable spans are easy to
        // add by accident.
        assert!(
            in_range_count > 0,
            "no key in the test universe falls inside this span, so every \
             assertion above passed vacuously"
        );
    }

    #[test]
    fn delete_prefixes_union_equals_key_range_even() {
        assert_prefixes_match_range(&span(&[]));
        assert_prefixes_match_range(&span(&[0xA, 0x7]));
        assert_prefixes_match_range(&span(&[0x0, 0x0]));
        assert_prefixes_match_range(&span(&[0xF, 0xF]));
        // Four nibbles: both bounds are two bytes (`a713`..`a714`), so the
        // exhaustive two-byte universe brackets the boundary exactly. This is
        // the deep mid-byte-divergence case; its coverage depends on neither
        // the three-byte step nor that key's pinned final byte.
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

    fn components(nibbles: &[u8]) -> Vec<PathComponent> {
        nibbles
            .iter()
            .map(|&n| PathComponent::try_new(n).expect("test nibble in range"))
            .collect()
    }

    fn key_nibbles(key: &[u8]) -> Vec<PathComponent> {
        key.iter()
            .flat_map(|&b| [b >> 4, b & 0xF])
            .map(|n| PathComponent::try_new(n).expect("a nibble is in range"))
            .collect()
    }

    /// The same key universe `assert_prefixes_match_range` uses.
    fn key_universe() -> Vec<Vec<u8>> {
        let mut keys: Vec<Vec<u8>> = vec![Vec::new()];
        keys.extend((0u8..=u8::MAX).map(|b| vec![b]));
        keys.extend((0u16..=u16::MAX).map(|k| k.to_be_bytes().to_vec()));
        keys.extend((0u16..=u16::MAX).step_by(16).map(|k| {
            let mut key = k.to_be_bytes().to_vec();
            key.push(0x5A);
            key
        }));
        keys
    }

    /// Every key strictly between the bounds lies in exactly one span or is
    /// exactly one point, no key outside the bounds is covered, and the spans
    /// are pairwise incomparable. `expect_keys` says whether the universe is
    /// expected to hold any key inside the interval, so an empty interval can
    /// be asserted deliberately rather than passing vacuously.
    fn assert_open_interval_partitions(lower: Option<&[u8]>, upper: &[u8], expect_keys: bool) {
        let lower = lower.map(components);
        let upper = components(upper);
        let interval = open_interval(lower.as_deref(), &upper);

        let spans: Vec<Vec<PathComponent>> = interval
            .spans
            .iter()
            .map(|s| s.prefix.iter().copied().collect())
            .collect();
        for (i, a) in spans.iter().enumerate() {
            for (j, b) in spans.iter().enumerate() {
                assert!(
                    i == j || !a.starts_with(b),
                    "span {a:?} lies under span {b:?}"
                );
            }
        }

        let strictly_inside = |path: &[PathComponent]| {
            lower.as_ref().is_none_or(|lo| path > lo.as_slice()) && path < upper.as_slice()
        };
        // Points are checked directly as well as through the universe, so a
        // spurious point longer than any universe key cannot slip through.
        for point in &interval.points {
            assert!(
                strictly_inside(&key_nibbles(point)),
                "point {point:02x?} lies outside the bounds"
            );
        }

        let mut inside_count = 0usize;
        for key in key_universe() {
            let path = key_nibbles(&key);
            let inside = strictly_inside(&path);
            let span_hits = spans.iter().filter(|s| path.starts_with(s)).count();
            let point_hits = interval.points.iter().filter(|p| ***p == *key).count();
            assert_eq!(
                usize::from(inside),
                span_hits.saturating_add(point_hits),
                "key {key:02x?}: inside={inside} spans={span_hits} points={point_hits}"
            );
            if inside {
                inside_count = inside_count.saturating_add(1);
            }
        }
        assert_eq!(
            expect_keys,
            inside_count > 0,
            "the key universe holds {inside_count} keys inside the interval"
        );
    }

    #[test]
    fn open_interval_below_an_anchor_covers_everything_under_it() {
        // The empty-start-proof shape: everything below the requested start key,
        // including the empty key and the anchor's own even-length prefixes.
        assert_open_interval_partitions(None, &[0xA, 0x7, 0x1, 0x1], true);
        assert_open_interval_partitions(None, &[0x0, 0x0], true);
        assert_open_interval_partitions(None, &[0xF, 0xF, 0xF, 0xF], true);
    }

    #[test]
    fn open_interval_between_two_keys() {
        // The proven-right-edge gap: local 0x26 lies between a proven edge of
        // 0x25 and a target key of 0x28 and must land in exactly one span.
        assert_open_interval_partitions(Some(&[0x2, 0x5]), &[0x2, 0x8], true);
        // Divergence deep inside shared structure.
        assert_open_interval_partitions(Some(&[0xA, 0x7, 0x1, 0x1]), &[0xA, 0x7, 0x7, 0x7], true);
        // Carry across a byte boundary.
        assert_open_interval_partitions(Some(&[0xA, 0xF, 0xF, 0xF]), &[0xB, 0x0, 0x0, 0x0], true);
        // Adjacent keys: only the lower key's extensions lie between them.
        assert_open_interval_partitions(Some(&[0xA, 0x7]), &[0xA, 0x8], true);
    }

    #[test]
    fn open_interval_above_a_prefix_of_the_upper_bound() {
        // The lower bound is a proper prefix of the upper bound, so the lower
        // bound itself is excluded while the upper bound's proper prefixes
        // deeper than it are points.
        assert_open_interval_partitions(Some(&[0xA, 0x7]), &[0xA, 0x7, 0x7, 0x7], true);
        // The empty key as lower bound: everything below the upper bound
        // except the empty key.
        assert_open_interval_partitions(Some(&[]), &[0xB, 0x0], true);
    }

    #[test]
    fn open_interval_is_empty_for_equal_or_inverted_bounds() {
        assert_open_interval_partitions(None, &[], false);
        assert_open_interval_partitions(Some(&[0xA, 0x7]), &[0xA, 0x7], false);
        assert_open_interval_partitions(Some(&[0xA, 0x8]), &[0xA, 0x7], false);
        assert_open_interval_partitions(Some(&[0xA, 0x7, 0x1]), &[0xA, 0x7], false);
        assert_eq!(open_interval(None, &[]), OpenInterval::default());
    }

    #[test]
    fn open_interval_reports_the_empty_key_as_a_point() {
        let interval = open_interval(None, &components(&[0xA, 0x7]));
        assert_eq!(interval.points, vec![Box::<[u8]>::from([])]);
        let interval = open_interval(Some(&[]), &components(&[0xA, 0x7]));
        assert!(interval.points.is_empty());
    }
}
