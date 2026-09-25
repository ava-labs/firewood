// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Range and change proofs sized to a serialized wire byte budget.

use std::num::{NonZeroU32, NonZeroU128};

use firewood_metrics::{HistogramExt, firewood_histogram};
use firewood_storage::{HashedNodeReader, TrieReader};
use integer_encoding::VarInt;

use super::{Key, Merkle, Value};
use crate::api::{self, FrozenChangeProof, FrozenProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::changes::DiffMerkleNodeStream;
use crate::proofs::header::Header;
use crate::proofs::ser::write_framed_body;
use crate::proofs::{ChangeProof, MAX_DECOMPRESSED_LEN, Proof, ProofError, ProofType, RangeProof};

/// Probes after the first candidate, each growing or shrinking it. One
/// more may follow to settle on a single item.
const MAX_CORRECTION_PASSES: usize = 8;
/// Wire size a grow pass aims for and a shrink pass cuts back to, as a
/// share of the budget. Aiming inside the acceptance window rather than at
/// the budget keeps ordinary compression variance from forcing another
/// probe.
const TARGET_FILL_PERCENT: usize = 99;
/// Growth stops once the wire reaches this share of the budget.
const SUFFICIENT_FILL_PERCENT: usize = 97;
/// Body bytes assumed for the edge proofs and length prefixes until a hint
/// or probe measures them. C-Chain edges run 9-15 KiB.
const DEFAULT_EDGE_BYTES: usize = 16 * 1024;
/// Extra body bytes cut when a body exceeds the decoder's limit, so the
/// rebuilt chunk lands under it despite a different right edge.
const BODY_LIMIT_MARGIN: usize = 16 * 1024;

/// Compressed/uncompressed body size fraction in fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CompressionRatio(NonZeroU32);

impl CompressionRatio {
    const SCALE: u32 = 1 << 16;
    /// An initial compressed/uncompressed estimate of 0.52.
    const INITIAL_ESTIMATE: Self = Self::from_scaled(Self::SCALE * 52 / 100);
    // Limit cross-page predictions to 20x compression or 2x expansion so one
    // unusual page cannot dominate the next. In-page measurements are unclamped.
    const MIN_EXPECTED: Self = Self::from_scaled(Self::SCALE / 20);
    const MAX_EXPECTED: Self = Self::from_scaled(Self::SCALE * 2);

    const fn from_scaled(scaled: u32) -> Self {
        Self(match NonZeroU32::new(scaled) {
            Some(value) => value,
            None => NonZeroU32::MIN,
        })
    }

    fn from_lengths(compressed: usize, uncompressed: usize) -> Self {
        let uncompressed = NonZeroU128::new(uncompressed as u128).unwrap_or(NonZeroU128::MIN);
        let scaled = (compressed as u128).saturating_mul(u128::from(Self::SCALE)) / uncompressed;
        Self::from_scaled(u32::try_from(scaled).unwrap_or(u32::MAX))
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

/// What a chunk measured about its data, calibrating the next request's
/// first candidate: pass a chunk's [`SizedProof::hint`] when requesting the
/// chunk that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SizingHint {
    /// Compressed/uncompressed ratio of the body.
    pub(super) ratio: CompressionRatio,
    /// Body bytes the edge proofs and length prefixes took; `None` assumes
    /// [`DEFAULT_EDGE_BYTES`] until the first probe measures them.
    pub(super) edges: Option<usize>,
}

/// A budget-targeted proof and its serialized wire representation.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no in-crate caller until the Db API exposes sized proofs"
    )
)]
pub(super) struct SizedProof<P> {
    /// The selected prefix and its boundary proofs.
    pub(super) proof: P,
    /// The bytes produced by serializing `proof`.
    pub(super) wire: Vec<u8>,
    /// True once paging has reached the end of the keyspace/diff.
    pub(super) natural_end: bool,
    /// What this chunk measured, for the next request's `hint`.
    pub(super) hint: SizingHint,
}

impl<T: TrieReader> Merkle<T> {
    /// Generates a range proof from the inclusive lower bound whose wire
    /// approaches, and never exceeds, `budget` bytes. Resume strictly above
    /// the last emitted key and pass the returned [`SizedProof::hint`] as the
    /// next request's `hint`.
    ///
    /// # Errors
    ///
    /// * [`api::Error::RangeProofOnEmptyTrie`] - if the trie is empty and
    ///   `start_key` is `None`, matching [`Merkle::range_proof`].
    /// * [`api::Error::ProofOverBudget`] - one entry, or the edge proofs
    ///   alone, serialize past `budget`.
    /// * A bounded request on an empty trie returns [`ProofError::Empty`].
    /// * Proof generation, iteration, body-size, or compression errors.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no in-crate caller until the `Db` API exposes sized proofs"
        )
    )]
    pub(super) fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        budget: usize,
        hint: Option<SizingHint>,
    ) -> Result<SizedProof<FrozenRangeProof>, api::Error> {
        if start_key.is_none() && self.root().is_none() {
            return Err(api::Error::RangeProofOnEmptyTrie);
        }
        let start_proof = match start_key {
            Some(key) => self.prove(key)?,
            None => Proof::default(),
        };
        let items = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .map(|r| r.map_err(api::Error::from));

        let sized = build_sized_chunk(
            &RangeChunkBuilder {
                merkle: self,
                start_proof,
            },
            items,
            budget,
            hint,
        )?;
        firewood_histogram!(PROOF_KEYS, "kind" => "range_sized")
            .record_integer(sized.proof.key_values().len());
        Ok(sized)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// Generates a change proof from the inclusive lower bound whose wire
    /// approaches, and never exceeds, `budget` bytes. Resume strictly above
    /// the last emitted key and pass the returned [`SizedProof::hint`] as the
    /// next request's `hint`.
    ///
    /// # Errors
    ///
    /// * [`api::Error::ProofOverBudget`] - one operation, or the edge proofs
    ///   alone, serialize past `budget`.
    /// * [`ProofError::Empty`] - this trie is empty, as in
    ///   [`Self::change_proof`].
    /// * Proof generation, diff iteration, body-size, or compression errors.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no in-crate caller until the `Db` API exposes sized proofs"
        )
    )]
    pub(super) fn change_proof_sized(
        &self,
        source_trie: &impl HashedNodeReader,
        start_key: Option<&[u8]>,
        budget: usize,
        hint: Option<SizingHint>,
    ) -> Result<SizedProof<FrozenChangeProof>, api::Error> {
        let start_proof = match start_key {
            Some(key) => self.prove(key)?,
            None => Proof::default(),
        };
        let items = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )?
        .map(|r| r.map_err(api::Error::from));

        let sized = build_sized_chunk(
            &ChangeChunkBuilder {
                merkle: self,
                start_proof,
            },
            items,
            budget,
            hint,
        )?;
        firewood_histogram!(PROOF_KEYS, "kind" => "change_sized")
            .record_integer(sized.proof.batch_ops().len());
        Ok(sized)
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
/// [`MAX_CORRECTION_PASSES`] limits the number of attempts. If every pass
/// overshoots, try a single item. If that item, or the edge proofs alone,
/// exceeds the budget, return [`api::Error::ProofOverBudget`]: peers would
/// reject a chunk larger than the message limit.
fn build_sized_chunk<B: ChunkBuilder>(
    builder: &B,
    items: impl Iterator<Item = Result<B::Item, api::Error>>,
    budget: usize,
    hint: Option<SizingHint>,
) -> Result<SizedProof<B::Proof>, api::Error> {
    let target = percent_of(budget, TARGET_FILL_PERCENT);
    let sufficient_fill = percent_of(budget, SUFFICIENT_FILL_PERCENT);

    let mut items = items.peekable();
    let mut kept: Vec<B::Item> = Vec::new();
    // `costs[n]` is the summed item cost of `kept[..n]`.
    let mut costs = vec![0usize];
    let mut body = Vec::new();
    let mut wire = Vec::new();
    let mut ratio = hint.map_or(CompressionRatio::INITIAL_ESTIMATE, |h| h.ratio);
    let mut edges = hint.and_then(|h| h.edges).unwrap_or(DEFAULT_EDGE_BYTES);
    let mut count = 0usize;
    let mut proof = None;
    let mut natural_end = false;
    // Body bytes the last probe overshot by; `None` grows instead.
    let mut excess: Option<usize> = None;
    let mut probes = 0usize;

    for pass in 0..=MAX_CORRECTION_PASSES {
        let shrinking = excess.is_some();
        count = if let Some(excess) = excess.take() {
            shrunk_count(&costs, count, excess)
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
        proof = Some((candidate, terminal));
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
        if shrinking || terminal || wire.len() >= sufficient_fill {
            break;
        }
    }

    let (proof, terminal) = match proof {
        Some(fitting) if excess.is_none() => fitting,
        _ => {
            // Passes exhausted while overshooting: the smallest chunk must fit.
            count = kept.len().min(1);
            let terminal = natural_end && count == kept.len();
            probes = probes.saturating_add(1);
            let candidate = builder.build(kept.get(..count).unwrap_or_default(), terminal)?;
            builder.serialize(&candidate, &mut body, &mut wire)?;
            if wire.len() > budget {
                return Err(api::Error::ProofOverBudget {
                    wire: wire.len(),
                    budget,
                });
            }
            edges = edge_bytes(body.len(), &costs, count);
            (candidate, terminal)
        }
    };
    firewood_histogram!(SIZED_PROOF_PROBES, "kind" => B::KIND.name()).record_integer(probes);
    Ok(SizedProof {
        proof,
        hint: SizingHint {
            ratio: CompressionRatio::measured(
                wire.len().saturating_sub(size_of::<Header>()),
                body.len(),
            ),
            edges: Some(edges),
        },
        wire,
        natural_end: terminal,
    })
}

/// One proof flavor: what an item costs in body bytes, how a candidate
/// chunk is assembled, and how it is serialized.
trait ChunkBuilder {
    type Item;
    type Proof;
    const KIND: ProofType;

    /// Body bytes `item` contributes to the payload.
    fn item_cost(item: &Self::Item) -> usize;
    /// The chunk proof for `items`; `natural_end` is true when `items`
    /// reached the end of the stream.
    fn build(&self, items: &[Self::Item], natural_end: bool) -> Result<Self::Proof, api::Error>;
    /// Serializes `proof`: its canonical body into `body` and its framed
    /// wire into `wire`, both cleared first.
    fn serialize(
        &self,
        proof: &Self::Proof,
        body: &mut Vec<u8>,
        wire: &mut Vec<u8>,
    ) -> Result<(), ProofError>;
}

struct RangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: FrozenProof,
}

impl<T: TrieReader> ChunkBuilder for RangeChunkBuilder<'_, T> {
    type Item = (Key, Value);
    type Proof = FrozenRangeProof;
    const KIND: ProofType = ProofType::Range;

    fn item_cost((key, value): &Self::Item) -> usize {
        encoded_sequence_len(key).saturating_add(encoded_sequence_len(value))
    }

    /// At the natural end the right edge stays open, as
    /// [`Merkle::range_proof`] leaves it when its limit is not hit.
    fn build(&self, kvs: &[Self::Item], natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match kvs.last() {
            Some((last, _)) if !natural_end => self.merkle.prove(last)?,
            _ => Proof::default(),
        };
        Ok(RangeProof::with_hash_mode(
            self.start_proof.clone(),
            end,
            kvs.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn serialize(
        &self,
        proof: &Self::Proof,
        body: &mut Vec<u8>,
        wire: &mut Vec<u8>,
    ) -> Result<(), ProofError> {
        body.clear();
        proof.write_body_to_vec(body);
        wire.clear();
        write_framed_body(body, Self::KIND, proof.hash_mode(), wire)
    }
}

struct ChangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: FrozenProof,
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

    /// The right edge is always the last operation's key, as
    /// [`Merkle::change_proof`] proves it when no end key is requested.
    fn build(&self, ops: &[Self::Item], _natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match ops.last() {
            Some(op) => self.merkle.prove(op.key())?,
            None => Proof::default(),
        };
        Ok(ChangeProof::with_hash_mode(
            self.start_proof.clone(),
            end,
            ops.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn serialize(
        &self,
        proof: &Self::Proof,
        body: &mut Vec<u8>,
        wire: &mut Vec<u8>,
    ) -> Result<(), ProofError> {
        body.clear();
        proof.write_body_to_vec(body);
        wire.clear();
        write_framed_body(body, Self::KIND, proof.hash_mode(), wire)
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
