// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Range and change proofs sized to a serialized wire byte budget.

use std::fmt;
use std::mem::size_of;
use std::num::NonZeroU32;
use std::num::NonZeroU128;

use bytemuck_derive::Pod;
use bytemuck_derive::Zeroable;
use firewood_metrics::HistogramExt;
use firewood_metrics::firewood_histogram;
use firewood_storage::HashedNodeReader;
use firewood_storage::NodeHashAlgorithm;
use firewood_storage::TrieReader;
use integer_encoding::VarInt;

use super::Key;
use super::Merkle;
use super::Value;
use crate::api::FrozenChangeProof;
use crate::api::FrozenProof;
use crate::api::FrozenRangeProof;
use crate::api::{self};
use crate::db::BatchOp;
use crate::merkle::changes::DiffMerkleNodeStream;
use crate::proofs::ChangeProof;
use crate::proofs::MAX_DECOMPRESSED_LEN;
use crate::proofs::Proof;
use crate::proofs::ProofError;
use crate::proofs::ProofType;
use crate::proofs::RangeProof;
use crate::proofs::header::Header;
use crate::proofs::ser::write_framed_body;

/// Probes after the first candidate, each growing or shrinking it. One
/// more may follow to settle on a single item.
pub(super) const MAX_CORRECTION_PASSES: usize = 8;
/// Wire size a grow pass aims for and a shrink pass cuts back to, as a
/// share of the budget. Aiming inside the acceptance window rather than at
/// the budget keeps ordinary compression variance from forcing another
/// probe.
const TARGET_FILL_PERCENT: usize = 99;
/// Growth stops once the wire reaches this share of the budget.
const SUFFICIENT_FILL_PERCENT: usize = 97;
/// Body bytes assumed for the edge proofs and length prefixes until a hint
/// or probe measures them (C-Chain edges run 9-15 KiB), capped at half the
/// target so the first estimate still admits items for small budgets.
const DEFAULT_EDGE_BYTES: usize = 16 * 1024;
/// Extra body bytes cut when a body exceeds the decoder's limit, so the
/// rebuilt chunk lands under it despite a different right edge.
const BODY_LIMIT_MARGIN: usize = 16 * 1024;

/// Compressed/uncompressed body size ratio in fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CompressionRatio(NonZeroU32);

impl CompressionRatio {
    /// The fixed-point representation of a 1:1 ratio.
    const SCALE: u32 = 1 << 16;
    /// An initial compressed/uncompressed estimate of 0.52.
    const INITIAL_ESTIMATE: Self = Self::const_scaled(Self::SCALE * 52 / 100);
    // Clamp cross-page predictions to 64x compression or 2x expansion so one
    // unusual page cannot dominate the next. Flushed-block frames of constant
    // values reach ~40x, so the floor must sit above that.
    const MIN_EXPECTED: Self = Self::const_scaled(Self::SCALE / 64);
    const MAX_EXPECTED: Self = Self::const_scaled(Self::SCALE * 2);

    /// A constant ratio from a non-zero literal.
    const fn const_scaled(scaled: u32) -> Self {
        Self(NonZeroU32::new(scaled).expect("constant ratios are non-zero"))
    }

    /// The unclamped ratio of one probe; a zero result rounds up to the
    /// smallest ratio.
    fn from_lengths(compressed: usize, uncompressed: usize) -> Self {
        let uncompressed = NonZeroU128::new(uncompressed as u128).unwrap_or(NonZeroU128::MIN);
        let scaled = (compressed as u128).saturating_mul(u128::from(Self::SCALE)) / uncompressed;
        let scaled = u32::try_from(scaled).unwrap_or(u32::MAX);
        Self(NonZeroU32::new(scaled).unwrap_or(NonZeroU32::MIN))
    }

    /// A measured body ratio, clamped for use as the next page's hint.
    pub(super) fn measured(compressed: usize, uncompressed: usize) -> Self {
        Self::from_lengths(compressed, uncompressed).clamp(Self::MIN_EXPECTED, Self::MAX_EXPECTED)
    }

    fn uncompressed_for(self, compressed: usize) -> usize {
        let estimate = (compressed as u128).saturating_mul(u128::from(Self::SCALE))
            / NonZeroU128::from(self.0);
        usize::try_from(estimate).unwrap_or(usize::MAX)
    }
}

/// The [`SizingHint::new`] budget when none is given: 95% of the 2 MiB p2p
/// message limit.
pub const DEFAULT_SIZED_PROOF_BUDGET: usize = 2 * 1024 * 1024 * 95 / 100;

/// Byte length of the stable encoding used to carry a [`SizingHint`] across an
/// FFI boundary.
pub const SIZING_HINT_ENCODING_LEN: usize = size_of::<SizingHintEncoding>();

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct SizingHintEncoding {
    version: [u8; 1],
    state: [u8; 1],
    budget: [u8; 8],
    ratio: [u8; 4],
    edges: [u8; 8],
}

/// A sized request's byte budget and, once a chunk was built, what that
/// chunk measured to calibrate the next one. Start with [`SizingHint::new`]
/// or [`SizingHint::default`] and pass each [`SizedProof::hint`] into the
/// request that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizingHint(SizingHintState);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizingHintState {
    /// No chunk built yet.
    Initial { budget: usize },
    /// What the previous chunk measured: the body ratio, and the body bytes
    /// its edge proofs and length prefixes took (`None` assumes a default).
    Computed {
        budget: usize,
        ratio: CompressionRatio,
        edges: Option<usize>,
    },
}

impl SizingHint {
    /// A hint for chunks whose serialized size never exceeds `budget` bytes.
    #[must_use]
    pub const fn new(budget: usize) -> Self {
        Self(SizingHintState::Initial { budget })
    }

    pub(super) const fn computed(
        budget: usize,
        ratio: CompressionRatio,
        edges: Option<usize>,
    ) -> Self {
        Self(SizingHintState::Computed {
            budget,
            ratio,
            edges,
        })
    }

    const fn budget(self) -> usize {
        match self.0 {
            SizingHintState::Initial { budget } | SizingHintState::Computed { budget, .. } => {
                budget
            }
        }
    }

    /// The ratio to start from, bounded like a measurement.
    fn ratio(self) -> CompressionRatio {
        match self.0 {
            SizingHintState::Initial { .. } => CompressionRatio::INITIAL_ESTIMATE,
            SizingHintState::Computed { ratio, .. } => ratio.clamp(
                CompressionRatio::MIN_EXPECTED,
                CompressionRatio::MAX_EXPECTED,
            ),
        }
    }

    const fn edges(self) -> Option<usize> {
        match self.0 {
            SizingHintState::Initial { .. } => None,
            SizingHintState::Computed { edges, .. } => edges,
        }
    }
}

impl Default for SizingHint {
    fn default() -> Self {
        Self::new(DEFAULT_SIZED_PROOF_BUDGET)
    }
}

/// A byte array did not contain a valid sizing hint encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSizingHintEncoding;

impl fmt::Display for InvalidSizingHintEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid sizing hint encoding")
    }
}

impl std::error::Error for InvalidSizingHintEncoding {}

impl From<SizingHint> for [u8; SIZING_HINT_ENCODING_LEN] {
    fn from(hint: SizingHint) -> Self {
        let (state, ratio, edges) = match hint.0 {
            SizingHintState::Initial { .. } => (0, [0; 4], [0; 8]),
            SizingHintState::Computed { ratio, edges, .. } => (
                if edges.is_some() { 2 } else { 1 },
                ratio.0.get().to_le_bytes(),
                edges.map_or([0; 8], |edges| {
                    u64::try_from(edges)
                        .expect("sizing hint edge size fits in u64")
                        .to_le_bytes()
                }),
            ),
        };
        let encoding = SizingHintEncoding {
            version: [1],
            state: [state],
            budget: u64::try_from(hint.budget())
                .expect("sizing hint budget fits in u64")
                .to_le_bytes(),
            ratio,
            edges,
        };
        bytemuck::bytes_of(&encoding)
            .try_into()
            .expect("encoding size matches SIZING_HINT_ENCODING_LEN")
    }
}

impl TryFrom<[u8; SIZING_HINT_ENCODING_LEN]> for SizingHint {
    type Error = InvalidSizingHintEncoding;

    fn try_from(bytes: [u8; SIZING_HINT_ENCODING_LEN]) -> Result<Self, Self::Error> {
        let encoding = bytemuck::try_from_bytes::<SizingHintEncoding>(&bytes)
            .map_err(|_| InvalidSizingHintEncoding)?;
        if encoding.version != [1] {
            return Err(InvalidSizingHintEncoding);
        }
        let budget = usize::try_from(u64::from_le_bytes(encoding.budget))
            .map_err(|_| InvalidSizingHintEncoding)?;
        match encoding.state {
            [0] if encoding.ratio == [0; 4] && encoding.edges == [0; 8] => Ok(Self::new(budget)),
            [1] if encoding.edges == [0; 8] => {
                let ratio = u32::from_le_bytes(encoding.ratio);
                let ratio = NonZeroU32::new(ratio)
                    .map(CompressionRatio)
                    .ok_or(InvalidSizingHintEncoding)?;
                Ok(Self::computed(budget, ratio, None))
            }
            [2] => {
                let ratio = u32::from_le_bytes(encoding.ratio);
                let ratio = NonZeroU32::new(ratio)
                    .map(CompressionRatio)
                    .ok_or(InvalidSizingHintEncoding)?;
                let edges = usize::try_from(u64::from_le_bytes(encoding.edges))
                    .map_err(|_| InvalidSizingHintEncoding)?;
                Ok(Self::computed(budget, ratio, Some(edges)))
            }
            _ => Err(InvalidSizingHintEncoding),
        }
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn sizing_hint_encoding_round_trips_all_states() {
        let hints = [
            SizingHint::new(2048),
            SizingHint::computed(4096, CompressionRatio::measured(52, 100), None),
            SizingHint::computed(8192, CompressionRatio::measured(25, 100), Some(512)),
        ];

        for hint in hints {
            let encoded = <[u8; SIZING_HINT_ENCODING_LEN]>::from(hint);
            assert_eq!(SizingHint::try_from(encoded), Ok(hint));
        }
    }

    #[test]
    fn sizing_hint_encoding_rejects_invalid_state() {
        let mut encoded = <[u8; SIZING_HINT_ENCODING_LEN]>::from(SizingHint::new(2048));
        encoded[0] = 0;
        assert_eq!(
            SizingHint::try_from(encoded),
            Err(InvalidSizingHintEncoding)
        );
    }
}

/// A budget-targeted proof and its serialized wire representation.
#[derive(Clone)]
#[must_use]
pub struct SizedProof<P> {
    proof: P,
    wire: Vec<u8>,
    natural_end: bool,
    hint: SizingHint,
}

impl<P> fmt::Debug for SizedProof<P> {
    // Omits the proof and wire payloads, which can be megabytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SizedProof")
            .field("wire_len", &self.wire.len())
            .field("natural_end", &self.natural_end)
            .field("hint", &self.hint)
            .finish_non_exhaustive()
    }
}

impl<P> SizedProof<P> {
    /// The selected prefix and its boundary proofs.
    #[must_use]
    pub const fn proof(&self) -> &P {
        &self.proof
    }

    /// The bytes produced by serializing the proof, never longer than the
    /// requested budget.
    #[must_use]
    pub const fn wire(&self) -> &[u8] {
        self.wire.as_slice()
    }

    /// True once paging has reached the end of the keyspace/diff. This is
    /// out-of-band metadata, not part of the serialized proof: a receiver
    /// must verify the proof before trusting it as the end of a sync.
    #[must_use]
    pub const fn natural_end(&self) -> bool {
        self.natural_end
    }

    /// What this chunk measured, for the next request's hint.
    #[must_use]
    pub const fn hint(&self) -> SizingHint {
        self.hint
    }

    /// The proof and its serialization, so the bytes can be sent as they are.
    #[must_use]
    pub fn into_parts(self) -> (P, Vec<u8>) {
        (self.proof, self.wire)
    }
}

impl<T: TrieReader> Merkle<T> {
    /// Generates a range proof over `[start_key, end_key]` whose wire
    /// approaches, and never exceeds, the hint's budget; the chunk that
    /// reaches `end_key` proves it, as [`Merkle::range_proof`] does. Resume strictly above
    /// the last emitted key and pass the returned [`SizedProof::hint`] as the
    /// next request's `hint`.
    ///
    /// # Errors
    ///
    /// * [`api::Error::RangeProofOnEmptyTrie`] - if the trie is empty and
    ///   `start_key` is `None`, matching [`Merkle::range_proof`].
    /// * [`api::Error::ProofOverBudget`] - one entry, or the edge proofs
    ///   alone, serialize past `budget`.
    /// * [`api::Error::ProofError`] with [`ProofError::BodyTooLarge`] - the
    ///   smallest chunk's body exceeds the decoder's limit, so no budget
    ///   can send it.
    /// * A bounded request on an empty trie returns [`ProofError::Empty`].
    /// * Proof generation, iteration, or compression errors.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no in-crate caller until the `Db` API exposes sized proofs"
        )
    )]
    pub(crate) fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        hint: SizingHint,
    ) -> Result<SizedProof<FrozenRangeProof>, api::Error> {
        check_range(start_key, end_key)?;
        if start_key.is_none() && end_key.is_none() && self.root().is_none() {
            return Err(api::Error::RangeProofOnEmptyTrie);
        }
        let start_proof = match start_key {
            Some(key) => self.prove(key)?,
            None => Proof::default(),
        };
        let items = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .stop_after_key(end_key)
            .map(|r| r.map_err(api::Error::from));

        let sized = build_sized_chunk(
            &RangeChunkBuilder {
                merkle: self,
                start_proof,
                end_key,
            },
            items,
            hint,
        )?;
        firewood_histogram!(PROOF_KEYS, "kind" => sized_kind_label(ProofType::Range))
            .record_integer(sized.proof.key_values().len());
        Ok(sized)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// Generates a change proof over `[start_key, end_key]` whose wire
    /// approaches, and never exceeds, the hint's budget; the chunk that
    /// reaches `end_key` proves it, as [`Merkle::change_proof`] does. Resume strictly above
    /// the last emitted key and pass the returned [`SizedProof::hint`] as the
    /// next request's `hint`.
    ///
    /// # Errors
    ///
    /// * [`api::Error::ProofOverBudget`] - one operation, or the edge proofs
    ///   alone, serialize past `budget`.
    /// * [`api::Error::ProofError`] with [`ProofError::BodyTooLarge`] - the
    ///   smallest chunk's body exceeds the decoder's limit, so no budget
    ///   can send it.
    /// * [`ProofError::Empty`] - this trie is empty, as in
    ///   [`Self::change_proof`].
    /// * Proof generation, diff iteration, or compression errors.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no in-crate caller until the `Db` API exposes sized proofs"
        )
    )]
    pub(crate) fn change_proof_sized(
        &self,
        source_trie: &impl HashedNodeReader,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        hint: SizingHint,
    ) -> Result<SizedProof<FrozenChangeProof>, api::Error> {
        check_range(start_key, end_key)?;
        let start_proof = match start_key {
            Some(key) => self.prove(key)?,
            None => Proof::default(),
        };
        let items = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )?
        .map_while(move |op| match op {
            Ok(op) => end_key
                .is_none_or(|end| op.key().as_ref() <= end)
                .then_some(Ok(op)),
            Err(err) => Some(Err(err)),
        })
        .map(|r| r.map_err(api::Error::from));

        let sized = build_sized_chunk(
            &ChangeChunkBuilder {
                merkle: self,
                start_proof,
                end_key,
            },
            items,
            hint,
        )?;
        firewood_histogram!(PROOF_KEYS, "kind" => sized_kind_label(ProofType::Change))
            .record_integer(sized.proof.batch_ops().len());
        Ok(sized)
    }
}

/// Rejects a lower bound above the upper one, as the plain proofs do.
fn check_range(start_key: Option<&[u8]>, end_key: Option<&[u8]>) -> Result<(), api::Error> {
    match (start_key, end_key) {
        (Some(start), Some(end)) if start > end => Err(api::Error::InvalidRange {
            start_key: start.to_vec().into(),
            end_key: end.to_vec().into(),
        }),
        _ => Ok(()),
    }
}

/// Selects a prefix of `items` whose serialized wire size gets close to
/// `budget` without exceeding it.
///
/// Compression means we can't know the wire size until we serialize, so
/// we estimate, measure, and adjust:
///
/// 1. Estimate the body bytes that fit in 99% of the budget using the
///    hinted compression ratio and edge-proof size, or their defaults.
///    Add items until their total cost reaches that estimate.
/// 2. Build and serialize the candidate. Use its measured ratio and edge
///    bytes for the next pass and as the hint for the next chunk.
/// 3. Stop if the wire size reaches 97% of the budget, we run out of
///    items, or the decoder's body limit leaves no room for another item.
/// 4. If the wire size is under 97%, grow using the updated estimate.
/// 5. If the wire size or body exceeds its limit, estimate the excess in
///    body bytes and drop tail items until their costs cover it. A shrunk
///    candidate that fits is final.
///
/// [`MAX_CORRECTION_PASSES`] limits the number of attempts. A shrink that
/// would land at or below a candidate that already fit stops there and
/// returns it, as does running out of attempts; if nothing fit, try a
/// single item.
/// If that item, or the edge proofs alone, exceeds the budget, return
/// [`api::Error::ProofOverBudget`]: peers would reject a chunk larger than
/// the message limit.
pub(super) fn build_sized_chunk<B: ChunkBuilder>(
    builder: &B,
    items: impl Iterator<Item = Result<B::Item, api::Error>>,
    hint: SizingHint,
) -> Result<SizedProof<B::Proof>, api::Error> {
    let budget = hint.budget();
    let target = percent_of(budget, TARGET_FILL_PERCENT);
    let sufficient_fill = percent_of(budget, SUFFICIENT_FILL_PERCENT);

    let mut items = items.peekable();
    let mut kept: Vec<B::Item> = Vec::new();
    // `costs[n]` is the summed item cost of `kept[..n]`.
    let mut costs = vec![0usize];
    let mut body = Vec::new();
    let mut wire = Vec::new();
    let mut ratio = hint.ratio();
    let mut edges = hint.edges().unwrap_or(DEFAULT_EDGE_BYTES.min(target / 2));
    let mut count = 0usize;
    // The largest candidate that fit the budget.
    let mut fit: Option<Fit<B::Proof>> = None;
    let mut natural_end = false;
    // Body bytes the last probe overshot by; `None` grows instead.
    let mut excess: Option<usize> = None;
    let mut probes = 0usize;

    for pass in 0..=MAX_CORRECTION_PASSES {
        let shrinking = excess.is_some();
        count = if let Some(excess) = excess.take() {
            let shrunk = shrunk_count(&costs, count, excess);
            if fit.as_ref().is_some_and(|fit| shrunk <= fit.count) {
                // Shrinking would not beat a candidate that already fit.
                break;
            }
            shrunk
        } else {
            let allowance = ratio
                .uncompressed_for(target.saturating_sub(size_of::<Header>()))
                .min(MAX_DECOMPRESSED_LEN)
                .saturating_sub(edges);
            let end = grow_prefix::<B>(&mut items, &mut kept, &mut costs, allowance)?;
            if kept.len() == count && pass > 0 {
                // The estimate admits nothing more: the last probe stands.
                break;
            }
            natural_end = end;
            kept.len()
        };
        let terminal = natural_end && count == kept.len();
        probes = probes.saturating_add(1);
        let candidate = builder.build(kept.get(..count).unwrap_or_default(), terminal)?;
        match builder.serialize(&candidate, &mut body, &mut wire) {
            Ok(()) => {}
            Err(ProofError::BodyTooLarge { len, .. }) if count > 1 => {
                // The body is exact: cut the excess plus a margin.
                excess = Some(
                    len.saturating_sub(MAX_DECOMPRESSED_LEN)
                        .saturating_add(BODY_LIMIT_MARGIN),
                );
                continue;
            }
            Err(err) => return Err(err.into()),
        }
        ratio = CompressionRatio::from_lengths(
            wire.len().saturating_sub(size_of::<Header>()),
            body.len(),
        );
        edges = edge_bytes(body.len(), &costs, count);
        if wire.len() > budget {
            if count <= 1 {
                return Err(api::Error::ProofOverBudget {
                    wire: wire.len(),
                    budget,
                });
            }
            excess = Some(ratio.uncompressed_for(wire.len().saturating_sub(target)));
            continue;
        }
        let wire_len = wire.len();
        fit = Some(Fit {
            proof: candidate,
            count,
            wire: std::mem::take(&mut wire),
            body_len: body.len(),
            edges,
            terminal,
        });
        if shrinking || terminal || wire_len >= sufficient_fill {
            break;
        }
    }

    let fit = if let Some(fit) = fit {
        fit
    } else {
        // Every pass overshot: the smallest chunk must fit.
        probes = probes.saturating_add(1);
        single_item_fit(builder, &kept, &costs, natural_end, budget, body)?
    };
    firewood_histogram!(SIZED_PROOF_PROBES, "kind" => sized_kind_label(B::KIND))
        .record_integer(probes);
    Ok(SizedProof {
        proof: fit.proof,
        hint: SizingHint::computed(
            budget,
            CompressionRatio::measured(
                fit.wire.len().saturating_sub(size_of::<Header>()),
                fit.body_len,
            ),
            Some(fit.edges),
        ),
        wire: fit.wire,
        natural_end: fit.terminal,
    })
}

/// The `kind` label the sized-proof metrics share, distinct from the plain
/// proofs' `range` and `change`.
const fn sized_kind_label(kind: ProofType) -> &'static str {
    match kind {
        ProofType::Range => "range_sized",
        ProofType::Change => "change_sized",
        ProofType::Single => "single_sized",
    }
}

/// A candidate that fit the budget, with what its probe measured.
struct Fit<P> {
    proof: P,
    count: usize,
    wire: Vec<u8>,
    body_len: usize,
    edges: usize,
    terminal: bool,
}

/// The chunk of the first item of `kept` (or of no item), which must fit
/// `budget` or the request fails.
fn single_item_fit<B: ChunkBuilder>(
    builder: &B,
    kept: &[B::Item],
    costs: &[usize],
    natural_end: bool,
    budget: usize,
    mut body: Vec<u8>,
) -> Result<Fit<B::Proof>, api::Error> {
    let count = kept.len().min(1);
    let terminal = natural_end && count == kept.len();
    let candidate = builder.build(kept.get(..count).unwrap_or_default(), terminal)?;
    let mut wire = Vec::new();
    builder.serialize(&candidate, &mut body, &mut wire)?;
    if wire.len() > budget {
        return Err(api::Error::ProofOverBudget {
            wire: wire.len(),
            budget,
        });
    }
    Ok(Fit {
        proof: candidate,
        count,
        wire,
        body_len: body.len(),
        edges: edge_bytes(body.len(), costs, count),
        terminal,
    })
}

/// One proof flavor: what an item costs in body bytes, how a candidate
/// chunk is assembled, and how it is serialized.
pub(super) trait ChunkBuilder {
    type Item;
    type Proof;
    const KIND: ProofType;

    /// Body bytes `item` contributes to the payload.
    fn item_cost(item: &Self::Item) -> usize;
    /// The chunk proof for `items`; `natural_end` is true when `items`
    /// reached the end of the stream.
    fn build(&self, items: &[Self::Item], natural_end: bool) -> Result<Self::Proof, api::Error>;
    /// Writes `proof`'s canonical body.
    fn write_body(proof: &Self::Proof, out: &mut Vec<u8>);
    fn hash_mode(proof: &Self::Proof) -> NodeHashAlgorithm;

    /// Serializes `proof`: its canonical body into `body` and its framed
    /// wire into `wire`, both cleared first.
    fn serialize(
        &self,
        proof: &Self::Proof,
        body: &mut Vec<u8>,
        wire: &mut Vec<u8>,
    ) -> Result<(), ProofError> {
        body.clear();
        Self::write_body(proof, body);
        wire.clear();
        write_framed_body(body, Self::KIND, Self::hash_mode(proof), wire)
    }
}

struct RangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: FrozenProof,
    end_key: Option<&'a [u8]>,
}

impl<T: TrieReader> ChunkBuilder for RangeChunkBuilder<'_, T> {
    type Item = (Key, Value);
    type Proof = FrozenRangeProof;
    const KIND: ProofType = ProofType::Range;

    fn item_cost((key, value): &Self::Item) -> usize {
        encoded_sequence_len(key).saturating_add(encoded_sequence_len(value))
    }

    /// At the natural end the right edge is the requested end key, or stays
    /// open without one, as [`Merkle::range_proof`] leaves it when its limit
    /// is not hit.
    fn build(&self, kvs: &[Self::Item], natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end_key = if natural_end {
            self.end_key
        } else {
            kvs.last().map(|(last, _)| last.as_ref())
        };
        let end = match end_key {
            Some(key) => self.merkle.prove(key)?,
            None => Proof::default(),
        };
        Ok(RangeProof::new(
            self.start_proof.clone(),
            end,
            kvs.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn write_body(proof: &Self::Proof, out: &mut Vec<u8>) {
        proof.write_body_to_vec(out);
    }

    fn hash_mode(proof: &Self::Proof) -> NodeHashAlgorithm {
        proof.hash_mode()
    }
}

struct ChangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: FrozenProof,
    end_key: Option<&'a [u8]>,
}

impl<T: HashedNodeReader> ChunkBuilder for ChangeChunkBuilder<'_, T> {
    type Item = BatchOp<Key, Value>;
    type Proof = FrozenChangeProof;
    const KIND: ProofType = ProofType::Change;

    /// One tag byte and the key, plus the value of a `Put`.
    fn item_cost(op: &Self::Item) -> usize {
        let tag_and_key = encoded_sequence_len(op.key()).saturating_add(1);
        match op {
            BatchOp::Put { value, .. } => tag_and_key.saturating_add(encoded_sequence_len(value)),
            BatchOp::Delete { .. } | BatchOp::DeleteRange { .. } => tag_and_key,
        }
    }

    /// The right edge is the requested end key once the natural end is
    /// reached, and otherwise the last operation's key, as
    /// [`Merkle::change_proof`] proves it.
    fn build(&self, ops: &[Self::Item], natural_end: bool) -> Result<Self::Proof, api::Error> {
        let last = ops.last().map(|op| op.key().as_ref());
        let end_key = if natural_end {
            self.end_key.or(last)
        } else {
            last
        };
        let end = match end_key {
            Some(key) => self.merkle.prove(key)?,
            None => Proof::default(),
        };
        Ok(ChangeProof::new(
            self.start_proof.clone(),
            end,
            ops.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn write_body(proof: &Self::Proof, out: &mut Vec<u8>) {
        proof.write_body_to_vec(out);
    }

    fn hash_mode(proof: &Self::Proof) -> NodeHashAlgorithm {
        proof.hash_mode()
    }
}

/// Admits items while their summed cost stays within `allowance`, always
/// admitting one when `kept` is empty so paging progresses. Returns whether
/// the stream is then exhausted.
fn grow_prefix<B: ChunkBuilder>(
    items: &mut std::iter::Peekable<impl Iterator<Item = Result<B::Item, api::Error>>>,
    kept: &mut Vec<B::Item>,
    costs: &mut Vec<usize>,
    allowance: usize,
) -> Result<bool, api::Error> {
    while let Some(peeked) = items.peek() {
        let cost = if let Ok(item) = peeked {
            B::item_cost(item)
        } else {
            items.next().transpose()?;
            continue;
        };
        let next_cost = costs
            .last()
            .copied()
            .unwrap_or_default()
            .saturating_add(cost);
        let payload_len = next_cost.saturating_add(kept.len().saturating_add(1).required_space());
        if !kept.is_empty() && (payload_len > MAX_DECOMPRESSED_LEN || payload_len > allowance) {
            break;
        }
        if let Some(item) = items.next().transpose()? {
            kept.push(item);
            costs.push(next_cost);
        }
    }
    Ok(items.peek().is_none())
}

/// The prefix length below `count` whose dropped items cover `excess` body
/// bytes, walking real item costs off the tail; at least one item is
/// dropped and one is kept.
fn shrunk_count(costs: &[usize], count: usize, excess: usize) -> usize {
    let total = costs.get(count).copied().unwrap_or_default();
    let mut kept = count.saturating_sub(1);
    while kept > 1 && total.saturating_sub(costs.get(kept).copied().unwrap_or_default()) < excess {
        kept = kept.saturating_sub(1);
    }
    kept.max(1)
}

/// Body bytes of a probe beyond its items and their count prefix: the edge
/// proofs and length prefixes.
fn edge_bytes(body_len: usize, costs: &[usize], count: usize) -> usize {
    body_len.saturating_sub(
        costs
            .get(count)
            .copied()
            .unwrap_or_default()
            .saturating_add(count.required_space()),
    )
}

fn encoded_sequence_len(bytes: &[u8]) -> usize {
    bytes.len().required_space().saturating_add(bytes.len())
}

fn percent_of(bytes: usize, percent: usize) -> usize {
    usize::try_from((bytes as u128).saturating_mul(percent as u128) / 100).unwrap_or(usize::MAX)
}
