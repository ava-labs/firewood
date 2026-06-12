// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`Endpoint`]: the keyspace endpoint type for state sync, plus the
//! binary-fraction split arithmetic used to carve the keyspace into work
//! regions (see `docs/plans/state-sync.md`, "Endpoint key type").
//!
//! # Semantic model: endpoints as binary fractions
//!
//! An [`Endpoint`] is interpreted as a **left-aligned binary fraction in
//! `[0, 1]`**: `Key([])` is `0.0`, [`Endpoint::Max`] is `1.0`, and
//! `Key(bytes)` is `0.bytes` with implicit zero-extension (the bytes are the
//! leading bits of the fraction).
//!
//! The fraction map is **not injective**: `Key(k)` and `Key(k ++ 0x00)` are
//! the *same* fraction but *different* keys (lexicographically
//! `k < k ++ 0x00`). One direction always holds: **fraction-order
//! strictly-less implies lexicographic strictly-less** — if the zero-extended
//! expansions first differ at a byte where one side is smaller, that side is
//! also lexicographically smaller, whether the differing position falls
//! inside both keys or in one key's zero-extension.
//!
//! Consequently an interval can be lexicographically non-empty but have
//! **zero fraction width** — e.g. `[0x10, 0x1000)` — and splitting such an
//! interval is impossible in the fraction model: the split functions
//! ([`midpoint`], [`even_slice_point`]) return `None` for it, and [`span`]
//! reports zero width.

use std::cmp::Ordering;
use std::num::NonZeroU128;
use std::ops::Range;

/// One end of a half-open keyspace range.
///
/// The derived [`Ord`] is exactly the intended sentinel semantics: variant
/// order makes every `Key(_)` less than `Max`, and `Key`s order
/// lexicographically on their bytes (standard slice ordering, where a proper
/// prefix sorts first).
///
/// * `Key(Box::default())` — the empty key — is the **start of the
///   keyspace**: no endpoint sorts before it.
/// * `Max` is the **one-past-the-end sentinel**, strictly greater than every
///   real key, so the whole keyspace is the half-open range
///   [`Endpoint::whole_keyspace`] (`Key([])..Max`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Endpoint {
    /// A real key; `Key(Box::default())` (the empty key) is the start of the
    /// keyspace.
    Key(Box<[u8]>),
    /// One-past-the-end sentinel, strictly greater than every real key.
    Max,
}

impl Endpoint {
    /// The whole keyspace as a half-open range: `Key([])..Max`.
    #[must_use]
    pub fn whole_keyspace() -> Range<Self> {
        Self::Key(Box::default())..Self::Max
    }
}

impl From<Box<[u8]>> for Endpoint {
    fn from(key: Box<[u8]>) -> Self {
        Self::Key(key)
    }
}

impl From<&[u8]> for Endpoint {
    fn from(key: &[u8]) -> Self {
        Self::Key(key.into())
    }
}

/// Returns the smallest byte string strictly greater than `key`:
/// `key ++ 0x00`.
///
/// Why it is the smallest: there is no byte string strictly between `key`
/// and `key ++ 0x00`. Any `x > key` either extends `key` — and the smallest
/// proper extension appends the smallest byte, which is `key ++ 0x00` itself
/// — or first differs from `key` at some byte position with a larger byte,
/// which makes `x` greater than `key ++ 0x00` at that same position.
///
/// Used by the orchestrator to turn the last key covered by a truncated
/// proof into a strictly-advancing continuation start.
///
/// Contract delivered: `Key(successor(k))` is strictly greater than `Key(k)`.
/// The result may still **equal a region's end bound** `b` (exactly when
/// `b == k ++ 0x00`), in which case the remainder `[successor(k), b)` is
/// empty and the region is fully covered — callers must treat the empty
/// remainder as completion, not hand it out as work.
#[allow(
    dead_code,
    reason = "consumed by the SyncState machinery in a later commit"
)]
pub(crate) fn successor(key: &[u8]) -> Box<[u8]> {
    key.iter().copied().chain(std::iter::once(0)).collect()
}

/// The binary-fraction midpoint of `(a, b)`, trimmed to the shortest usable
/// key.
///
/// Computes the exact fraction `(a + b) / 2` and returns the **shortest
/// prefix of it** that is lexicographically strictly inside `(a, b)`,
/// biasing the split toward the true midpoint while keeping the emitted key
/// short. The result is always an [`Endpoint::Key`], never
/// [`Endpoint::Max`]. In degenerate intervals the emitted key can be
/// fraction-equal to `a` (e.g. `midpoint([0x10], [0x10, 0x01])` is
/// `[0x10, 0x00]`), leaving the left half a zero-width sliver — that skews
/// balance only; lexicographic progress remains strict.
///
/// `b == Max` is handled exactly as `(a + 1.0) / 2` — shift `a` right one
/// bit and set the top bit — never by approximating `Max` with an `0xff`
/// fill. A midpoint landing between byte boundaries carries the half-bit
/// into an appended byte (e.g. `0x80`); this falls out of computing at one
/// byte more precision than the inputs.
///
/// Returns `None` when the interval has **zero fraction width** (equal or
/// reversed endpoints, or degenerate shapes like `[0x10, 0x1000)` — see the
/// module docs). Zero fraction width does **not** mean lexicographically
/// empty: `[0x10, 0x1000)` still contains `[0x10]` and `[0x10, 0x00]`. A
/// `None` with `a < b` means the hole cannot be split — the caller must hand
/// out the whole hole, never treat it as empty. Conversely,
/// when the fraction width is nonzero the exact midpoint is strictly inside
/// `(a, b)` as a fraction — hence strictly inside lexicographically — so a
/// prefix always qualifies and this returns `Some`.
///
/// Note the trim minimizes length along the midpoint's *own* prefixes; an
/// even shorter key inside `(a, b)` that is not a prefix of the midpoint may
/// exist. This affects only balance, never correctness.
#[allow(
    dead_code,
    reason = "consumed by the SyncState machinery in a later commit"
)]
pub(crate) fn midpoint(a: &Endpoint, b: &Endpoint) -> Option<Endpoint> {
    if frac_cmp(a, b) != Ordering::Less {
        // Zero fraction width (or equal/reversed endpoints): nothing strictly
        // inside in the fraction model.
        return None;
    }
    let Endpoint::Key(a_bytes) = a else {
        // Unreachable: `Max` has the maximal fraction, so `frac(a) < frac(b)`
        // forces `a` to be a `Key`.
        return None;
    };
    // sum = frac(a) + frac(b), as (integer bit, fraction bytes). For
    // `b == Max` the sum is exactly `frac(a) + 1.0`.
    let (int_bit, sum) = match b {
        Endpoint::Max => (1, a_bytes.to_vec()),
        Endpoint::Key(b_bytes) => add_bytes(a_bytes, b_bytes),
    };
    let mid = shift_right_one(int_bit, &sum);
    // The full-precision midpoint is exact and strictly inside (a, b) as a
    // fraction, hence strictly inside lexicographically, so a prefix always
    // qualifies.
    shortest_key_inside(&mid, a, b)
}

/// The first boundary of an `owed`-way even split of `[a, b)`:
/// `a + (b - a) / owed` as binary fractions, trimmed to the shortest key
/// strictly inside `(a, b)`.
///
/// The exact point is computed at working precision: byte-at-a-time long
/// division of `b - a` by `owed`, rounding down, with the quotient precision
/// extended past the inputs' width so it is never rounded to zero (see
/// [`divide`]). Rounding only perturbs *balance*, never correctness — the
/// emission constraint that matters is `a < Key(m) < b`, both strict, biased
/// toward the exact point. `b == Max` is handled exactly:
/// `1.0 - frac(a)` is computed two's-complement style ([`one_minus`]), never
/// by approximating `Max` with an `0xff` fill.
///
/// Returns `None` when `owed < 2` (`owed == 1`: the final slice takes the
/// exact remainder, so the caller uses `b` directly as its boundary;
/// `owed == 0`: nothing is owed at all) or when `[a, b)` has zero fraction
/// width — unsplittable in the fraction model, though possibly still
/// lexicographically nonempty (see [`midpoint`]): the caller must hand out
/// the whole hole rather than treat it as empty. For any
/// nonzero-width interval and `owed >= 2` the full-precision point is
/// strictly inside `(a, b)`, so this returns `Some`.
///
/// Iterating over the *remaining* span ("divide what's left by the slices
/// still owed": `owed = N, N-1, …, 2`) tiles `[a, b)` into `N` slices with
/// no gap or overlap. For `N = 16` over the whole keyspace the boundaries
/// land exactly on the nibble seeds `0x10, 0x20, …, 0xf0`.
#[allow(
    dead_code,
    reason = "consumed by the SyncState machinery in a later commit"
)]
pub(crate) fn even_slice_point(a: &Endpoint, b: &Endpoint, owed: usize) -> Option<Endpoint> {
    if owed < 2 {
        return None;
    }
    // Never `None`: `owed >= 2` was checked above.
    let divisor = NonZeroU128::new(u128::from(owed as u64))?;
    if frac_cmp(a, b) != Ordering::Less {
        // Zero fraction width: genuinely impossible to split.
        return None;
    }
    let Endpoint::Key(a_bytes) = a else {
        // Unreachable: `Max` has the maximal fraction (see `midpoint`).
        return None;
    };
    // d = frac(b) - frac(a) as (integer bit, fraction bytes); the integer bit
    // is 1 only when the width is exactly 1.0 (frac(a) == 0.0 and b == Max).
    let (d_int, d_bytes) = match b {
        Endpoint::Max => one_minus(a_bytes),
        Endpoint::Key(b_bytes) => (0, sub_bytes(b_bytes, a_bytes)),
    };
    let quotient = divide(d_int, &d_bytes, divisor);
    let point = add_no_overflow(a_bytes, &quotient);
    shortest_key_inside(&point, a, b)
}

/// [`Ord`]-comparable magnitude of `b - a` as a binary fraction, produced by
/// [`span`]. Used by hole selection to pick the largest cold hole.
///
/// The derived tuple `Ord` (fields in declaration order) is a total order
/// consistent with fraction width:
///
/// * `is_whole` sorts first. It is `true` only for width exactly `1.0`, in
///   which case the fraction bytes are all zero and normalize to empty — so
///   `(true, [])` is the unique maximum.
/// * `bytes` holds the fraction bytes of the difference with **trailing zero
///   bytes stripped**. Lexicographic comparison of trailing-zero-stripped
///   byte strings matches fraction order exactly: equal fractions normalize
///   to identical byte strings; if one string is a proper prefix of the
///   other, the longer one ends in a nonzero byte and is both the larger
///   fraction and lexicographically larger; and if they first differ at some
///   byte, that byte decides both orders the same way.
///
/// Zero-width spans normalize to `(false, [])` — equal to each other and
/// less than every nonzero span.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[allow(
    dead_code,
    reason = "consumed by the SyncState machinery in a later commit"
)]
pub(crate) struct Span {
    /// `true` only for a span of fraction width exactly `1.0`.
    is_whole: bool,
    /// Fraction bytes of `b - a`, trailing zero bytes stripped.
    bytes: Box<[u8]>,
}

/// The fraction width of `[a, b)` as an [`Ord`]-comparable [`Span`].
///
/// Equal or reversed endpoints, and zero-fraction-width intervals (see the
/// module docs), all yield the canonical zero span.
#[allow(
    dead_code,
    reason = "consumed by the SyncState machinery in a later commit"
)]
pub(crate) fn span(a: &Endpoint, b: &Endpoint) -> Span {
    if frac_cmp(a, b) != Ordering::Less {
        return Span::default();
    }
    let Endpoint::Key(a_bytes) = a else {
        // Unreachable: `Max` has the maximal fraction (see `midpoint`).
        return Span::default();
    };
    let (d_int, mut d_bytes) = match b {
        Endpoint::Max => one_minus(a_bytes),
        Endpoint::Key(b_bytes) => (0, sub_bytes(b_bytes, a_bytes)),
    };
    while d_bytes.last() == Some(&0) {
        d_bytes.pop();
    }
    Span {
        is_whole: d_int == 1,
        bytes: d_bytes.into(),
    }
}

/// Compares two endpoints **as binary fractions** (not lexicographically).
///
/// `Max` (`1.0`) is strictly greater than every `Key`: a finite byte string
/// of length `n` has fraction at most `1 - 256^-n < 1.0`. Two `Key`s compare
/// by their zero-extended byte expansions, which is exactly fraction order.
fn frac_cmp(a: &Endpoint, b: &Endpoint) -> Ordering {
    match (a, b) {
        (Endpoint::Max, Endpoint::Max) => Ordering::Equal,
        (Endpoint::Max, Endpoint::Key(_)) => Ordering::Greater,
        (Endpoint::Key(_), Endpoint::Max) => Ordering::Less,
        (Endpoint::Key(x), Endpoint::Key(y)) => {
            let width = x.len().max(y.len());
            zero_extended(x, width).cmp(zero_extended(y, width))
        }
    }
}

/// Iterates the bytes of `x` zero-extended on the right to `width` bytes.
fn zero_extended(x: &[u8], width: usize) -> impl Iterator<Item = u8> + '_ {
    x.iter()
        .copied()
        .chain(std::iter::repeat_n(0, width.saturating_sub(x.len())))
}

/// `frac(x) + frac(y)` at width `max(x.len(), y.len())`, as
/// `(carry, bytes)` where `carry` (0 or 1) is the integer part of the sum.
fn add_bytes(x: &[u8], y: &[u8]) -> (u8, Vec<u8>) {
    let width = x.len().max(y.len());
    let mut out: Vec<u8> = zero_extended(x, width).collect();
    let ys: Vec<u8> = zero_extended(y, width).collect();
    let mut carry = 0u8;
    // Least-significant (rightmost) byte first.
    for (o, yb) in out.iter_mut().zip(ys).rev() {
        // Each operand is at most 0xFF and carry is at most 1, so the sum is
        // at most 0x1FF and cannot wrap a u16.
        let sum = u16::from(*o)
            .wrapping_add(u16::from(yb))
            .wrapping_add(u16::from(carry));
        *o = (sum & 0xFF) as u8;
        carry = (sum >> 8) as u8;
    }
    (carry, out)
}

/// `frac(b) - frac(a)` at width `max(b.len(), a.len())`.
///
/// The caller must ensure `frac(a) <= frac(b)`; the final borrow is then
/// always zero.
fn sub_bytes(b: &[u8], a: &[u8]) -> Vec<u8> {
    let width = b.len().max(a.len());
    let mut out: Vec<u8> = zero_extended(b, width).collect();
    let subtrahend: Vec<u8> = zero_extended(a, width).collect();
    let mut borrow = false;
    // Least-significant (rightmost) byte first.
    for (o, ab) in out.iter_mut().zip(subtrahend).rev() {
        let (diff, borrow1) = o.overflowing_sub(ab);
        let (diff, borrow2) = diff.overflowing_sub(u8::from(borrow));
        *o = diff;
        borrow = borrow1 || borrow2;
    }
    debug_assert!(!borrow, "caller must ensure frac(a) <= frac(b)");
    out
}

/// `1.0 - frac(x)` at width `x.len()`, as `(int_bit, bytes)`.
///
/// Two's-complement style over the fraction width: bytewise NOT computes
/// `1.0 - frac(x) - 1ulp`, then one ulp is added back with carry
/// propagation. A carry out of the top byte means the result is exactly
/// `1.0` (`int_bit == 1`, all bytes zero), which happens exactly when
/// `frac(x) == 0.0`.
fn one_minus(x: &[u8]) -> (u8, Vec<u8>) {
    let mut out: Vec<u8> = x.iter().map(|&b| !b).collect();
    for byte in out.iter_mut().rev() {
        let (incremented, overflow) = byte.overflowing_add(1);
        *byte = incremented;
        if !overflow {
            return (0, out);
        }
    }
    (1, out)
}

/// Shifts the fixed-point value `int_bit.bytes` right one bit, producing a
/// pure fraction at width `bytes.len() + 1` — the appended byte captures a
/// half-bit shifted out of the last input byte (e.g. a trailing `0x80`).
fn shift_right_one(int_bit: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().saturating_add(1));
    // The bit shifted out of the previous byte, entering this byte's top.
    let mut carry_bit = int_bit;
    for &byte in bytes {
        out.push((carry_bit << 7) | (byte >> 1));
        carry_bit = byte & 1;
    }
    out.push(carry_bit << 7);
    out
}

/// `floor((int_bit.bytes) / divisor)` as fraction bytes at width
/// `bytes.len() + 8`, by byte-at-a-time long division carrying the
/// remainder.
///
/// Why 8 extra bytes always make the quotient nonzero for a nonzero
/// dividend: the dividend is at least one ulp at width `bytes.len()`, so
/// scaled to width `bytes.len() + 8` it is at least `256^8 = 2^64`, which
/// exceeds any `divisor` that fits a `u64` — so at least one quotient byte
/// is nonzero.
fn divide(int_bit: u8, bytes: &[u8], divisor: NonZeroU128) -> Vec<u8> {
    const EXTRA_PRECISION: usize = 8;
    let width = bytes.len().saturating_add(EXTRA_PRECISION);
    let mut rem = u128::from(int_bit);
    zero_extended(bytes, width)
        .map(|byte| {
            // rem < divisor <= u64::MAX, so rem * 256 + byte < 2^72 and
            // cannot wrap a u128.
            let acc = rem.wrapping_mul(256).wrapping_add(u128::from(byte));
            rem = acc % divisor;
            // acc <= (divisor - 1) * 256 + 255 < divisor * 256, so the
            // quotient byte fits a u8.
            (acc / divisor) as u8
        })
        .collect()
}

/// `frac(a) + frac(q)` where the caller guarantees the sum stays strictly
/// below `1.0` (no carry out of the top byte).
fn add_no_overflow(a: &[u8], q: &[u8]) -> Vec<u8> {
    let (carry, out) = add_bytes(a, q);
    debug_assert_eq!(carry, 0, "caller guarantees the sum stays below 1.0");
    out
}

/// The shortest prefix of `point` that is lexicographically strictly inside
/// `(a, b)`, as an [`Endpoint::Key`].
///
/// A prefix of `point` rounds the fraction down, so every candidate is at
/// most `point` — the emitted key is biased toward the exact computed point.
fn shortest_key_inside(point: &[u8], a: &Endpoint, b: &Endpoint) -> Option<Endpoint> {
    (0..=point.len()).find_map(|len| {
        let candidate = point.get(..len)?;
        (endpoint_lt_key(a, candidate) && key_lt_endpoint(candidate, b))
            .then(|| Endpoint::Key(candidate.into()))
    })
}

/// `a < Key(k)`, lexicographically.
fn endpoint_lt_key(a: &Endpoint, k: &[u8]) -> bool {
    match a {
        Endpoint::Key(bytes) => bytes.as_ref() < k,
        Endpoint::Max => false,
    }
}

/// `Key(k) < b`, lexicographically.
fn key_lt_endpoint(k: &[u8], b: &Endpoint) -> bool {
    match b {
        Endpoint::Key(bytes) => k < bytes.as_ref(),
        Endpoint::Max => true,
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::unwrap_used,
        clippy::arithmetic_side_effects,
        reason = "tests may unwrap and use plain arithmetic on small constants"
    )]

    use super::*;
    use firewood_storage::SeededRng;

    fn key(bytes: &[u8]) -> Endpoint {
        Endpoint::Key(bytes.into())
    }

    fn extend(k: &[u8], tail: &[u8]) -> Box<[u8]> {
        k.iter().chain(tail).copied().collect()
    }

    /// Asserts no proper prefix of `m` is strictly inside `(a, b)`.
    fn assert_shortest_prefix(m: &Endpoint, a: &Endpoint, b: &Endpoint) {
        let Endpoint::Key(bytes) = m else {
            panic!("split point must be a Key, got {m:?}");
        };
        for len in 0..bytes.len() {
            let prefix = Endpoint::Key(bytes.get(..len).unwrap().into());
            assert!(
                !(*a < prefix && prefix < *b),
                "shorter prefix {prefix:?} of {m:?} is strictly inside ({a:?}, {b:?})"
            );
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

    #[test]
    fn ord_sanity() {
        assert!(key(&[]) < key(&[0x00]));
        assert!(key(&[0x00]) < key(&[0x00, 0x00]));
        assert!(key(&[0x00, 0x00]) < key(&[0x01]));
        assert!(key(&[0x01]) < Endpoint::Max);
        // prefix-extension ordering
        assert!(key(&[0x10]) < key(&[0x10, 0x00]));
        assert!(key(&[0x10, 0x00]) < key(&[0x10, 0x01]));
        assert!(key(&[0x10, 0xff]) < key(&[0x11]));

        let whole = Endpoint::whole_keyspace();
        assert_eq!(whole.start, key(&[]));
        assert_eq!(whole.end, Endpoint::Max);
        assert!(whole.start < whole.end);
    }

    #[test]
    fn conversions() {
        let boxed: Box<[u8]> = Box::new([0x10, 0x20]);
        assert_eq!(Endpoint::from(boxed), key(&[0x10, 0x20]));
        assert_eq!(Endpoint::from(&[0x10u8, 0x20][..]), key(&[0x10, 0x20]));
    }

    #[test]
    fn successor_minimality() {
        for k in [&[][..], &[0x10][..], &[0xff, 0xff][..]] {
            let s = successor(k);
            // successor is exactly k ++ 0x00
            assert_eq!(s, extend(k, &[0x00]));
            // strictness
            assert!(key(k) < key(&s));
            // minimality: every proper extension of k is >= k ++ 0x00
            for tail in [&[0x00][..], &[0x00, 0x00][..], &[0x01][..], &[0xff][..]] {
                let x = key(&extend(k, tail));
                assert!(x >= key(&s), "{x:?} fell below the successor {s:?}");
            }
        }
    }

    #[test]
    fn midpoint_basic() {
        // whole keyspace -> 0.5
        assert_eq!(midpoint(&key(&[]), &Endpoint::Max), Some(key(&[0x80])));
        // (0x10 + 0x20) / 2 = 0x18
        assert_eq!(midpoint(&key(&[0x10]), &key(&[0x20])), Some(key(&[0x18])));
        // b == Max: (0x10/256 + 1.0) / 2 = 0x88/256
        assert_eq!(midpoint(&key(&[0x10]), &Endpoint::Max), Some(key(&[0x88])));
        // b == Max near the top: (0xff/256 + 1.0) / 2 = 0.ff80
        assert_eq!(
            midpoint(&key(&[0xff]), &Endpoint::Max),
            Some(key(&[0xff, 0x80]))
        );
        // the only key strictly inside ([0x10], [0x10, 0x01])
        assert_eq!(
            midpoint(&key(&[0x10]), &key(&[0x10, 0x01])),
            Some(key(&[0x10, 0x00]))
        );
    }

    #[test]
    fn midpoint_none_cases() {
        // adjacent: zero fraction width, lexicographically empty interior
        assert_eq!(midpoint(&key(&[0x10]), &key(&[0x10, 0x00])), None);
        // zero fraction width with a lexicographically NON-empty interior
        // (degenerate; see the module docs)
        assert_eq!(midpoint(&key(&[0x10]), &key(&[0x10, 0x00, 0x00])), None);
        // equal endpoints
        assert_eq!(midpoint(&key(&[0x10]), &key(&[0x10])), None);
        assert_eq!(midpoint(&key(&[]), &key(&[])), None);
        assert_eq!(midpoint(&Endpoint::Max, &Endpoint::Max), None);
        // reversed endpoints
        assert_eq!(midpoint(&key(&[0x20]), &key(&[0x10])), None);
        assert_eq!(midpoint(&Endpoint::Max, &key(&[0x10])), None);
    }

    #[test]
    fn midpoint_shortest_prefix_examples() {
        let cases = [
            (key(&[]), Endpoint::Max),
            (key(&[0x10]), key(&[0x20])),
            (key(&[0xff]), Endpoint::Max),
            (key(&[0x10]), key(&[0x10, 0x01])),
            (key(&[0x10, 0xff]), key(&[0x12])),
        ];
        for (a, b) in &cases {
            let m = midpoint(a, b).unwrap();
            assert!(*a < m && m < *b, "midpoint {m:?} outside ({a:?}, {b:?})");
            assert_shortest_prefix(&m, a, b);
        }
    }

    #[test]
    fn even_slice_nibble_seed() {
        // "divide the remaining span by slices still owed", owed = 16..=2,
        // lands exactly on the nibble boundaries 0x10, 0x20, ..., 0xf0.
        let whole = Endpoint::whole_keyspace();
        let mut prev = whole.start.clone();
        let mut boundaries = Vec::new();
        for owed in (2..=16usize).rev() {
            let m = even_slice_point(&prev, &whole.end, owed).unwrap();
            boundaries.push(m.clone());
            prev = m;
        }
        let expected: Vec<Endpoint> = (1u8..=15).map(|i| key(&[i * 0x10])).collect();
        assert_eq!(boundaries, expected);
    }

    #[test]
    fn even_slice_prime_tiling() {
        // Prime slice counts tile with strictly increasing boundaries, all
        // strictly inside (a, b).
        for n in [5usize, 17] {
            let mut prev = Endpoint::Key(Box::default());
            let mut count = 0usize;
            for owed in (2..=n).rev() {
                let m = even_slice_point(&prev, &Endpoint::Max, owed).unwrap();
                assert!(prev < m, "boundaries must strictly increase");
                assert!(m < Endpoint::Max, "boundary must stay inside the span");
                prev = m;
                count += 1;
            }
            assert_eq!(count, n - 1);
        }
    }

    #[test]
    fn even_slice_none_cases() {
        let a = key(&[0x10]);
        let b = key(&[0x20]);
        // owed == 0 or 1: caller uses b directly
        assert_eq!(even_slice_point(&a, &b, 0), None);
        assert_eq!(even_slice_point(&a, &b, 1), None);
        // zero fraction width
        assert_eq!(
            even_slice_point(&key(&[0x10]), &key(&[0x10, 0x00]), 4),
            None
        );
        // equal and reversed endpoints
        assert_eq!(even_slice_point(&a, &a, 4), None);
        assert_eq!(even_slice_point(&b, &a, 4), None);
    }

    #[test]
    fn span_ordering() {
        let whole = span(&key(&[]), &Endpoint::Max); // 1.0
        let wide = span(&key(&[0x10]), &key(&[0x20])); // 0.10
        let medium = span(&key(&[0x10]), &key(&[0x18])); // 0.08
        let narrow = span(&key(&[0xff]), &Endpoint::Max); // 0.01
        let sliver = span(&key(&[0x10]), &key(&[0x10, 0x01])); // 0.0001
        assert!(whole > wide);
        assert!(wide > medium);
        assert!(medium > narrow);
        assert!(narrow > sliver);

        // equal-width spans at different positions compare equal
        assert_eq!(wide, span(&key(&[0x80]), &key(&[0x90])));

        // zero-width spans compare equal to each other and less than
        // everything nonzero
        let zero1 = span(&key(&[0x10]), &key(&[0x10, 0x00]));
        let zero2 = span(&key(&[0x20]), &key(&[0x20]));
        let zero3 = span(&Endpoint::Max, &key(&[])); // reversed
        assert_eq!(zero1, zero2);
        assert_eq!(zero1, zero3);
        assert_eq!(zero1, Span::default());
        assert!(zero1 < sliver);

        // width exactly 1.0 is a property of the fractions, not of Key([]):
        // frac([0x00]) == 0.0 too
        assert_eq!(whole, span(&key(&[0x00]), &Endpoint::Max));

        // the whole keyspace dominates even the widest sub-1.0 span
        assert!(whole > span(&key(&[]), &key(&[0xff, 0xff])));
    }

    fn random_key(rng: &SeededRng) -> Box<[u8]> {
        const EDGE: [u8; 8] = [0x00, 0x01, 0x0f, 0x10, 0x7f, 0x80, 0xfe, 0xff];
        let len = rng.random_range(0..=5usize);
        (0..len)
            .map(|_| {
                if rng.random_range(0..2u8) == 0 {
                    *EDGE.get(rng.random_range(0..EDGE.len())).unwrap()
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

    fn run_split_properties(iters: usize) {
        let rng = SeededRng::from_env_or_random();
        for _ in 0..iters {
            let mut a = random_endpoint(&rng);
            let mut b = random_endpoint(&rng);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            match midpoint(&a, &b) {
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
            let owed = rng.random_range(2..=64usize);
            match even_slice_point(&a, &b, owed) {
                Some(m) => {
                    assert!(
                        a < m && m < b,
                        "slice point {m:?} (owed {owed}) outside ({a:?}, {b:?})"
                    );
                    assert_shortest_prefix(&m, &a, &b);
                }
                None => assert_ne!(
                    frac_cmp_ref(&a, &b),
                    Ordering::Less,
                    "slice point None requires zero fraction width: ({a:?}, {b:?})"
                ),
            }
            // span is zero exactly when the fraction width is zero
            let s = span(&a, &b);
            if frac_cmp_ref(&a, &b) == Ordering::Less {
                assert!(s > Span::default(), "nonzero width must give nonzero span");
            } else {
                assert_eq!(s, Span::default(), "zero width must give the zero span");
            }
        }
    }

    fn run_tiling_property(iters: usize) {
        let rng = SeededRng::from_env_or_random();
        for _ in 0..iters {
            let bound = random_endpoint(&rng);
            let mut prev = Endpoint::Key(Box::default());
            let owed_start = rng.random_range(2..=40usize);
            for owed in (2..=owed_start).rev() {
                if let Some(m) = even_slice_point(&prev, &bound, owed) {
                    // monotone strictly-increasing boundaries, inside the span
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
        run_split_properties(2_000);
        run_tiling_property(200);
    }

    #[test]
    fn test_slow_endpoint_split_properties() {
        run_split_properties(200_000);
        run_tiling_property(20_000);
    }
}
