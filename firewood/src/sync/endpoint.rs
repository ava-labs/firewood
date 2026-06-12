// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`Endpoint`]: one end of a keyspace range, plus the arithmetic that splits
//! a range into roughly equal parts (see
//! `docs/src/designs/2026-06-11-state-sync.md`, "Endpoint key type").
//!
//! Splitting reads an endpoint as a binary fraction in `[0, 1]`: `Key(bytes)`
//! is `0.bytes` and [`Endpoint::Max`] is `1.0`. Distinct keys can share a
//! fraction (`k` and `k ++ 0x00` are both `0.k`), so a range can contain keys
//! yet have zero [`Width`].

use std::num::NonZeroU64;
use std::num::NonZeroU128;
use std::num::NonZeroUsize;
use std::ops::Div;
use std::ops::Range;
use std::ops::Sub;

use firewood_storage::PathBuf;
use firewood_storage::TriePathAsPackedBytes;
use firewood_storage::prefix_upper_bound;
use nonzero_ext::nonzero;

/// One end of a half-open keyspace range.
///
/// The derived [`Ord`] sorts `Key`s by their bytes and puts `Max` after every
/// key, so the whole keyspace is `Key([])..Max`
/// ([`Endpoint::whole_keyspace`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Endpoint {
    /// A key; the empty key is the start of the keyspace.
    Key(Box<[u8]>),
    /// Sorts after every key.
    Max,
}

impl Endpoint {
    /// The start of the keyspace: the empty key.
    #[must_use]
    pub fn keyspace_start() -> Self {
        Self::Key(Box::default())
    }

    /// The whole keyspace as a half-open [`Range`]: `Key([])..Max`.
    #[must_use]
    pub fn whole_keyspace() -> Range<Self> {
        Self::keyspace_start()..Self::Max
    }
}

impl<T: Into<Box<[u8]>>> From<T> for Endpoint {
    fn from(key: T) -> Self {
        Self::Key(key.into())
    }
}

/// The width of a range, read as a fraction (see the module docs); `Ord`
/// compares widths and the default is zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Width {
    /// The integer part: 1 only for the whole keyspace.
    whole: u8,
    /// The fraction bytes, without trailing zeros so equal widths compare
    /// equal.
    bytes: Box<[u8]>,
}

impl Width {
    fn new(whole: u8, mut bytes: Vec<u8>) -> Self {
        debug_assert!(whole <= 1, "a width is at most the whole keyspace");
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        Self {
            whole,
            bytes: bytes.into(),
        }
    }
}

/// The bytes of `x` zero-extended on the right to `len` bytes.
fn zero_extended(x: &[u8], len: usize) -> Vec<u8> {
    let mut out = x.to_vec();
    out.resize(len.max(x.len()), 0);
    out
}

/// `b - a` is the width of `[a, b)`, or `None` unless `a < b`.
impl Sub for &Endpoint {
    type Output = Option<Width>;

    fn sub(self, a: Self) -> Option<Width> {
        let b = self;
        let (a_bytes, b_whole, b_bytes): (&[u8], u8, &[u8]) = match (a, b) {
            (Endpoint::Key(a), Endpoint::Max) => (a, 1, &[]),
            (Endpoint::Key(a), Endpoint::Key(b)) if a < b => (a, 0, b),
            _ => return None,
        };
        let len = b_bytes.len().max(a_bytes.len());
        let mut diff = zero_extended(b_bytes, len);
        let mut borrow = false;
        for (diff_byte, a_byte) in diff.iter_mut().zip(zero_extended(a_bytes, len)).rev() {
            (*diff_byte, borrow) = diff_byte.borrowing_sub(a_byte, borrow);
        }
        // Byte order `a < b` implies `0.a <= 0.b`, so this cannot go negative.
        Some(Width::new(b_whole.wrapping_sub(u8::from(borrow)), diff))
    }
}

/// Divides a width, rounding down; see [`long_divide`].
impl Div<NonZeroUsize> for Width {
    type Output = Self;

    fn div(self, n: NonZeroUsize) -> Self {
        if n == NonZeroUsize::MIN {
            return self;
        }
        let divisor =
            NonZeroU64::try_from(n).expect("usize is at most 64 bits on supported targets");
        // n >= 2 and whole <= 1, so the integer part is 0.
        Self::new(0, long_divide(self.whole, &self.bytes, divisor))
    }
}

/// `whole.fraction / divisor` as fraction bytes, by base-256 long division
/// rounding down.
///
/// Requires `whole < divisor`, so the quotient is below `1.0` and has no
/// integer digit; `whole` seeds the remainder instead. [`Width`] only ever has
/// a `whole` of 0 or 1, and its [`Div`] impl never divides by 1.
///
/// The dividend is extended by 8 bytes, so a nonzero dividend is at least
/// `256^8 = 2^64` units and, since `divisor < 2^64`, the quotient is nonzero.
fn long_divide(whole: u8, fraction: &[u8], divisor: NonZeroU64) -> Vec<u8> {
    const EXTRA_PRECISION: usize = 8;
    debug_assert!(
        u64::from(whole) < divisor.get(),
        "the quotient must be below 1.0"
    );
    // Widened so `rem * 256 + byte` cannot overflow.
    let divisor = NonZeroU128::from(divisor);
    let dividend = zero_extended(fraction, fraction.len().saturating_add(EXTRA_PRECISION));
    let mut quotient = Vec::with_capacity(dividend.len());
    let mut rem = u128::from(whole);
    for byte in dividend {
        // rem < divisor < 2^64, so this is below 2^72 and never wraps.
        let acc = rem.wrapping_mul(256).wrapping_add(u128::from(byte));
        // acc < divisor * 256, so the quotient byte fits a u8.
        quotient.push((acc / divisor) as u8);
        rem = acc % divisor;
    }
    quotient
}

impl Width {
    /// The bytes of `start + self`. The caller ensures the sum stays below
    /// `1.0`.
    fn add_to(&self, start: &[u8]) -> Vec<u8> {
        debug_assert_eq!(self.whole, 0, "caller ensures the sum stays below 1.0");
        let len = start.len().max(self.bytes.len());
        let mut sum = zero_extended(start, len);
        let mut carry = false;
        for (sum_byte, width_byte) in sum.iter_mut().zip(zero_extended(&self.bytes, len)).rev() {
            (*sum_byte, carry) = sum_byte.carrying_add(width_byte, carry);
        }
        debug_assert!(!carry, "caller ensures the sum stays below 1.0");
        sum
    }
}

impl Endpoint {
    /// The first cut when splitting `[self, end)` into `n_way` roughly equal
    /// parts: the shortest key strictly inside `(self, end)` that is a prefix of
    /// `self + (end - self) / n_way`, or `None` if `n_way` is 1 or no key lies
    /// strictly inside.
    ///
    /// Cutting what remains `N, N-1, …, 2` ways divides `[self, end)` into `N`
    /// parts with no gap or overlap; 16 ways over the whole keyspace gives
    /// `0x10, 0x20, …, 0xf0`.
    pub(crate) fn first_cut(&self, end: &Endpoint, n_way: NonZeroUsize) -> Option<Endpoint> {
        // `end - self` is `None` unless `self < end`, which also rules out
        // `self == Max`, so this returns early for an empty or reversed range
        // and binds `self`'s key.
        let (Endpoint::Key(start), Some(width)) = (self, end - self) else {
            return None;
        };
        if n_way == NonZeroUsize::MIN {
            return None;
        }
        // A prefix of `point` never sorts below a shorter prefix, so the
        // prefixes above `self` are all those from some length on, and the
        // prefixes below `end` are all those up to some length. The shortest
        // prefix inside `(self, end)` is therefore the shortest one above
        // `self`, if that one is also below `end`; otherwise no prefix is
        // inside. Prefixes no longer than the run of bytes `point` shares with
        // `start` are prefixes of `start`, so the shortest candidate is one
        // byte longer than that run.
        //
        // When `width / n_way` is nonzero (see `long_divide`), `point` is
        // numerically inside `(self, end)`, so some prefix is inside and a key
        // is always found.
        let mut point = (width / n_way).add_to(start);
        // `add_to` returns at least `start.len()` bytes; a trailing zero keeps the
        // point's value and makes it longer than `start`.
        point.push(0);
        let shared = point
            .iter()
            .zip(start.iter())
            .take_while(|(p, s)| p == s)
            .count();
        let candidate = point
            .get(..shared.saturating_add(1))
            .expect("shared <= start.len() < point.len()");
        debug_assert!(
            start.as_ref() < candidate,
            "`point` is not below `self`, so its prefix past the shared bytes is above `self`"
        );
        let below_end = match end {
            Endpoint::Key(end) => candidate < end.as_ref(),
            Endpoint::Max => true,
        };
        below_end.then(|| Endpoint::Key(candidate.into()))
    }

    /// The cut splitting `[self, end)` in half; see [`Self::first_cut`].
    pub(crate) fn midpoint(&self, end: &Endpoint) -> Option<Endpoint> {
        self.first_cut(end, nonzero!(2usize))
    }
}

/// Where to split the unfetched rest of a truncated proof's range, which ends
/// at `end`, given the nibble prefixes of the subtrees the proof shows there
/// (`remaining_subtrees_after_range_proof`). `None` when there are none.
///
/// A subtree `d` nibbles deep holds about `16^-d` of a hashed keyspace. With
/// two or more subtrees at the shallowest depth, the cut is the start of the
/// middle one: all deeper subtrees together weigh no more than one shallow
/// subtree. With one, the cut is the midpoint of that subtree, clipped to
/// `end`.
pub(crate) fn proof_cut(subtrees: &[PathBuf], end: &Endpoint) -> Option<Endpoint> {
    let depth = subtrees.iter().map(PathBuf::len).min()?;
    let shallow: Vec<&PathBuf> = subtrees.iter().filter(|p| p.len() == depth).collect();
    let start = |path: &[_]| Endpoint::Key(path.as_packed_bytes().collect());
    if let [only] = shallow.as_slice() {
        let subtree_end = prefix_upper_bound(only).map_or(Endpoint::Max, |bound| start(&bound));
        return start(only).midpoint(&subtree_end.min(end.clone()));
    }
    let middle = shallow
        .get(shallow.len() / 2)
        .expect("the shallowest depth has a subtree");
    Some(start(middle))
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::arithmetic_side_effects,
        reason = "tests use plain arithmetic on small constants"
    )]

    use std::cmp::Ordering;

    use firewood_storage::SeededRng;
    use test_case::test_case;

    use super::*;

    fn endpoint(bytes: &[u8]) -> Endpoint {
        bytes.into()
    }

    fn width(a: &Endpoint, b: &Endpoint) -> Width {
        (b - a).unwrap_or_default()
    }

    fn n(n_way: usize) -> NonZeroUsize {
        NonZeroUsize::new(n_way).expect("tests split at least 1 way")
    }

    /// Asserts no proper prefix of `m` is strictly inside `(a, b)`.
    fn assert_shortest_prefix(m: &Endpoint, a: &Endpoint, b: &Endpoint) {
        let Endpoint::Key(bytes) = m else {
            panic!("split point must be a Key, got {m:?}");
        };
        if let Some(prefix) = (0..bytes.len())
            .map(|len| Endpoint::Key(bytes[..len].into()))
            .find(|prefix| a < prefix && prefix < b)
        {
            panic!("shorter prefix {prefix:?} of {m:?} is strictly inside ({a:?}, {b:?})");
        }
    }

    /// Independent reimplementation of fraction comparison for property
    /// checks (zero-extended bytewise compare; `Max` is `1.0`, above every
    /// finite key fraction).
    fn frac_cmp_ref(a: &Endpoint, b: &Endpoint) -> Ordering {
        match (a, b) {
            (Endpoint::Max, Endpoint::Max) => Ordering::Equal,
            (Endpoint::Max, Endpoint::Key(_)) => Ordering::Greater,
            (Endpoint::Key(_), Endpoint::Max) => Ordering::Less,
            (Endpoint::Key(left), Endpoint::Key(right)) => {
                let width = left.len().max(right.len());
                let left_ext: Vec<u8> = left
                    .iter()
                    .copied()
                    .chain(std::iter::repeat_n(0, width - left.len()))
                    .collect();
                let right_ext: Vec<u8> = right
                    .iter()
                    .copied()
                    .chain(std::iter::repeat_n(0, width - right.len()))
                    .collect();
                left_ext.cmp(&right_ext)
            }
        }
    }

    #[test_case(endpoint(&[]), Endpoint::Max, Some(endpoint(&[0x80])); "whole keyspace -> 0.5")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x20]), Some(endpoint(&[0x18])); "(0x10 + 0x20) / 2 = 0x18")]
    #[test_case(endpoint(&[0x10]), Endpoint::Max, Some(endpoint(&[0x88])); "b == Max: (0x10/256 + 1.0) / 2")]
    #[test_case(endpoint(&[0xff]), Endpoint::Max, Some(endpoint(&[0xff, 0x80])); "b == Max near the top")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10, 0x01]), Some(endpoint(&[0x10, 0x00])); "only key strictly inside")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10, 0x00]), None; "adjacent: zero width, empty interior")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10, 0x00, 0x00]), Some(endpoint(&[0x10, 0x00])); "zero width, non-empty interior")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10]), None; "equal endpoints")]
    #[test_case(endpoint(&[]), endpoint(&[]), None; "equal empty endpoints")]
    #[test_case(Endpoint::Max, Endpoint::Max, None; "equal Max endpoints")]
    #[test_case(endpoint(&[0x20]), endpoint(&[0x10]), None; "reversed endpoints")]
    #[test_case(Endpoint::Max, endpoint(&[0x10]), None; "reversed Max / key")]
    fn midpoint_case(a: Endpoint, b: Endpoint, expected: Option<Endpoint>) {
        assert_eq!(a.midpoint(&b), expected);
    }

    fn paths(nibble_paths: &[&[u8]]) -> Vec<PathBuf> {
        nibble_paths
            .iter()
            .map(|nibbles| {
                nibbles
                    .iter()
                    .map(|&n| firewood_storage::PathComponent::try_new(n).unwrap())
                    .collect()
            })
            .collect()
    }

    #[test_case(&[], Endpoint::Max, None; "no subtrees")]
    #[test_case(&[&[0x3], &[0x7]], Endpoint::Max, Some(endpoint(&[0x70])); "two shallow: the second")]
    #[test_case(&[&[0x1, 0x2, 0x3], &[0x1, 0x3], &[0x1, 0x4], &[0x1, 0x5]], Endpoint::Max, Some(endpoint(&[0x14])); "deeper subtrees ignored")]
    #[test_case(&[&[0x1, 0x2, 0x3], &[0x1, 0x4]], Endpoint::Max, Some(endpoint(&[0x14, 0x80])); "one shallow: its midpoint")]
    #[test_case(&[&[0xa, 0xb, 0xc]], Endpoint::Max, Some(endpoint(&[0xab, 0xc8])); "odd depth pads a zero nibble")]
    #[test_case(&[&[0xf]], Endpoint::Max, Some(endpoint(&[0xf8])); "last subtree ends at Max")]
    #[test_case(&[&[0x1, 0x4]], endpoint(&[0x14, 0x40]), Some(endpoint(&[0x14, 0x20])); "clipped to end")]
    fn proof_cut_case(nibble_paths: &[&[u8]], end: Endpoint, expected: Option<Endpoint>) {
        assert_eq!(proof_cut(&paths(nibble_paths), &end), expected);
    }

    #[test_case(endpoint(&[]), Endpoint::Max; "whole keyspace")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x20]); "exact midpoint")]
    #[test_case(endpoint(&[0xff]), Endpoint::Max; "near the top, b == Max")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10, 0x01]); "narrow sliver")]
    #[test_case(endpoint(&[0x10, 0xff]), endpoint(&[0x12]); "carry across byte boundary")]
    fn midpoint_shortest_prefix(a: Endpoint, b: Endpoint) {
        let m = a.midpoint(&b).unwrap();
        assert!(a < m && m < b, "midpoint {m:?} outside ({a:?}, {b:?})");
        assert_shortest_prefix(&m, &a, &b);
    }

    #[test]
    fn even_slice_nibble_seed() {
        // Splitting what remains 16, 15, ..., 2 ways lands on 0x10, ..., 0xf0.
        let mut prev = Endpoint::keyspace_start();
        let mut boundaries = Vec::new();
        for n_way in (2..=16usize).rev() {
            let m = prev.first_cut(&Endpoint::Max, n(n_way)).unwrap();
            boundaries.push(m.clone());
            prev = m;
        }
        let expected: Vec<Endpoint> = (1u8..=15).map(|i| endpoint(&[i * 0x10])).collect();
        assert_eq!(boundaries, expected);
    }

    #[test_case(5; "5 slices")]
    #[test_case(17; "17 slices")]
    fn even_slice_prime_tiling(n: usize) {
        // Prime slice counts tile with strictly increasing boundaries, all
        // strictly inside (a, b).
        let mut prev = Endpoint::keyspace_start();
        let mut count = 0usize;
        for n_way in (2..=n).rev() {
            let m = prev.first_cut(&Endpoint::Max, self::n(n_way)).unwrap();
            assert!(prev < m, "boundaries must strictly increase");
            assert!(m < Endpoint::Max, "boundary must stay inside the range");
            prev = m;
            count += 1;
        }
        assert_eq!(count, n - 1);
    }

    #[test_case(endpoint(&[0x10]), endpoint(&[0x20]), 1; "1-way split")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10, 0x00]), 4; "zero fraction width")]
    #[test_case(endpoint(&[0x10]), endpoint(&[0x10]), 4; "equal endpoints")]
    #[test_case(endpoint(&[0x20]), endpoint(&[0x10]), 4; "reversed endpoints")]
    fn even_slice_none(a: Endpoint, b: Endpoint, n_way: usize) {
        assert_eq!(a.first_cut(&b, n(n_way)), None);
    }

    #[test]
    fn width_ordering() {
        let whole = width(&endpoint(&[]), &Endpoint::Max); // 1.0
        let wide = width(&endpoint(&[0x10]), &endpoint(&[0x20])); // 0.10
        let medium = width(&endpoint(&[0x10]), &endpoint(&[0x18])); // 0.08
        let narrow = width(&endpoint(&[0xff]), &Endpoint::Max); // 0.01
        let sliver = width(&endpoint(&[0x10]), &endpoint(&[0x10, 0x01])); // 0.0001
        assert!(whole > wide);
        assert!(wide > medium);
        assert!(medium > narrow);
        assert!(narrow > sliver);

        // equal widths at different positions compare equal
        assert_eq!(wide, width(&endpoint(&[0x80]), &endpoint(&[0x90])));

        // zero widths compare equal to each other and less than
        // everything nonzero
        let zero1 = width(&endpoint(&[0x10]), &endpoint(&[0x10, 0x00]));
        let zero2 = width(&endpoint(&[0x20]), &endpoint(&[0x20]));
        let zero3 = width(&Endpoint::Max, &endpoint(&[])); // reversed
        assert_eq!(zero1, zero2);
        assert_eq!(zero1, zero3);
        assert_eq!(zero1, Width::default());
        assert!(zero1 < sliver);

        // width exactly 1.0 is a property of the fractions, not of Key([]):
        // frac([0x00]) == 0.0 too
        assert_eq!(whole, width(&endpoint(&[0x00]), &Endpoint::Max));

        // the whole keyspace dominates even the widest sub-1.0 width
        assert!(whole > width(&endpoint(&[]), &endpoint(&[0xff, 0xff])));
    }

    fn random_key(rng: &SeededRng) -> Box<[u8]> {
        const EDGE: [u8; 8] = [0x00, 0x01, 0x0f, 0x10, 0x7f, 0x80, 0xfe, 0xff];
        let len = rng.random_range(0..=5usize);
        (0..len)
            .map(|_| {
                if rng.random_range(0..2u8) == 0 {
                    EDGE[rng.random_range(0..EDGE.len())]
                } else {
                    rng.random()
                }
            })
            .collect()
    }

    fn random_endpoint(rng: &SeededRng) -> Endpoint {
        if rng.random_range(0..10u8) == 0 {
            Endpoint::Max
        } else {
            Endpoint::Key(random_key(rng))
        }
    }

    fn run_split_properties(rng: &SeededRng, iters: usize) {
        for _ in 0..iters {
            let mut a = random_endpoint(rng);
            let mut b = random_endpoint(rng);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            match a.midpoint(&b) {
                Some(m) => {
                    assert!(a < m && m < b, "midpoint {m:?} outside ({a:?}, {b:?})");
                    assert!(matches!(m, Endpoint::Key(_)), "midpoint may never be Max");
                    assert_shortest_prefix(&m, &a, &b);
                }
                None => assert_ne!(
                    frac_cmp_ref(&a, &b),
                    Ordering::Less,
                    "midpoint None requires zero fraction width: ({a:?}, {b:?})"
                ),
            }
            let n_way = rng.random_range(2..=64usize);
            match a.first_cut(&b, n(n_way)) {
                Some(m) => {
                    assert!(
                        a < m && m < b,
                        "slice point {m:?} ({n_way}-way) outside ({a:?}, {b:?})"
                    );
                    assert_shortest_prefix(&m, &a, &b);
                }
                None => assert_ne!(
                    frac_cmp_ref(&a, &b),
                    Ordering::Less,
                    "slice point None requires zero fraction width: ({a:?}, {b:?})"
                ),
            }
            // width is zero exactly when the fraction width is zero
            let w = width(&a, &b);
            if frac_cmp_ref(&a, &b) == Ordering::Less {
                assert!(
                    w > Width::default(),
                    "nonzero fraction width must give a nonzero Width"
                );
            } else {
                assert_eq!(
                    w,
                    Width::default(),
                    "zero fraction width must give the zero Width"
                );
            }
        }
    }

    fn run_tiling_property(rng: &SeededRng, iters: usize) {
        for _ in 0..iters {
            let bound = random_endpoint(rng);
            let mut prev = Endpoint::keyspace_start();
            let n_start = rng.random_range(2..=40usize);
            for n_way in (2..=n_start).rev() {
                if let Some(m) = prev.first_cut(&bound, n(n_way)) {
                    // monotone strictly-increasing boundaries, inside the range
                    assert!(
                        prev < m && m < bound,
                        "boundary {m:?} outside ({prev:?}, {bound:?})"
                    );
                    prev = m;
                } else {
                    assert_ne!(
                        frac_cmp_ref(&prev, &bound),
                        Ordering::Less,
                        "tiling stopped early with width remaining"
                    );
                    break;
                }
            }
        }
    }

    #[test]
    fn endpoint_split_properties() {
        let rng = SeededRng::from_env_or_random();
        run_split_properties(&rng, 2_000);
        run_tiling_property(&rng, 200);
    }

    #[test]
    fn test_slow_endpoint_split_properties() {
        let rng = SeededRng::from_env_or_random();
        run_split_properties(&rng, 200_000);
        run_tiling_property(&rng, 20_000);
    }
}
