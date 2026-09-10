// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Size-targeted proof generation: a range/change proof whose compressed
//! wire size targets the specified byte budget.

use std::num::{NonZeroU32, NonZeroU64};

use firewood_metrics::{HistogramExt, firewood_histogram};
use firewood_storage::{HashedNodeReader, TrieReader};
use integer_encoding::VarInt;

use super::{Key, Merkle, Value};
use crate::api::{self, FrozenChangeProof, FrozenProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::changes::DiffMerkleNodeStream;
use crate::proofs::frame::MAX_DECOMPRESSED_LEN;
use crate::proofs::{ChangeProof, Proof, RangeProof};

/// Cap on the uncompressed payload of one chunk, leaving the rest of the
/// decoder's body limit for the edge proofs.
const MAX_PAYLOAD: usize = MAX_DECOMPRESSED_LEN / 2;
/// Cap on ratio-correction (grow) passes.
const MAX_RATIO_CORRECTION_PASSES: usize = 6;
/// Stop growing once the wire reaches this percentage of the budget.
const SUFFICIENT_FILL_PERCENT: usize = 95;

/// Compressed/uncompressed size fraction of a proof body, in fixed point
/// with [`Self::SCALE`] meaning 1:1. Pass the previous chunk's ratio as the
/// next request's `ratio_hint` so its first size estimate is calibrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CompressionRatio(NonZeroU32);

impl CompressionRatio {
    /// Fixed-point scale: the representation of a 1:1 ratio.
    const SCALE: u32 = 1 << 16;
    /// Assumed before anything is measured; roughly 2:1.
    pub(crate) const INITIAL_ESTIMATE: Self = Self::from_scaled(Self::SCALE * 52 / 100);
    /// Measurements are clamped into `MIN_EXPECTED..=MAX_EXPECTED`, keeping
    /// a pathological chunk from skewing the next estimate.
    const MIN_EXPECTED: Self = Self::from_scaled(Self::SCALE / 20);
    const MAX_EXPECTED: Self = Self::from_scaled(Self::SCALE * 2);

    /// Zero is stored as the smallest representable ratio.
    const fn from_scaled(scaled: u32) -> Self {
        Self(match NonZeroU32::new(scaled) {
            Some(scaled) => scaled,
            None => NonZeroU32::MIN,
        })
    }

    /// `compressed / uncompressed`, clamped into the expected range.
    pub(crate) fn measured(compressed: usize, uncompressed: usize) -> Self {
        let uncompressed = NonZeroU64::new(uncompressed as u64).unwrap_or(NonZeroU64::MIN);
        let scaled = (compressed as u64).saturating_mul(u64::from(Self::SCALE)) / uncompressed;
        Self::from_scaled(u32::try_from(scaled).unwrap_or(u32::MAX))
            .clamp(Self::MIN_EXPECTED, Self::MAX_EXPECTED)
    }

    /// Uncompressed bytes expected to compress into `compressed` bytes.
    fn uncompressed_for(self, compressed: usize) -> usize {
        let estimate =
            (compressed as u64).saturating_mul(u64::from(Self::SCALE)) / NonZeroU64::from(self.0);
        usize::try_from(estimate).unwrap_or(usize::MAX)
    }
}

/// A sized proof `P` with its compressed `wire` bytes.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no in-crate caller until the `Db` API exposes sized proofs"
    )
)]
pub(crate) struct SizedProof<P> {
    pub proof: P,
    pub wire: Vec<u8>,
    /// True once paging has reached the end of the keyspace/diff.
    pub natural_end: bool,
    /// Measured compression of this chunk; pass it as `ratio_hint`
    /// when requesting the next chunk.
    pub ratio: CompressionRatio,
}

/// Body bytes of a length-prefixed byte sequence.
fn seq_len(bytes: &[u8]) -> usize {
    bytes.len().required_space().saturating_add(bytes.len())
}

/// One proof flavor for [`stream_sized`]: what an item costs in uncompressed
/// body bytes, how to assemble a chunk proof, and how to serialize it.
trait ChunkBuilder {
    type Item;
    type Proof;

    /// Uncompressed body bytes `item` contributes to the payload.
    fn item_cost(item: &Self::Item) -> usize;

    /// The chunk proof (payload plus right edge) for `items`;
    /// `at_natural_end` is true when `items` reached the end of the stream.
    fn build(&self, items: &[Self::Item], at_natural_end: bool) -> Result<Self::Proof, api::Error>;

    /// Compressed wire bytes for `proof`.
    fn wire(proof: &Self::Proof) -> Result<Vec<u8>, api::Error>;
}

/// Assembles a proof from a prefix of `items`, sized to approach
/// `budget` compressed wire bytes without exceeding it unless a single
/// item alone does.
///
/// The proof is grown/shrunk by estimate, and is decided by real
/// serialized length. The output is near-budget, not the exact
/// largest fitting prefix.
fn stream_sized<B: ChunkBuilder>(
    builder: &B,
    items: impl Iterator<Item = Result<B::Item, api::Error>>,
    budget: usize,
    ratio_hint: Option<CompressionRatio>,
) -> Result<SizedProof<B::Proof>, api::Error> {
    let mut ratio = ratio_hint.unwrap_or(CompressionRatio::INITIAL_ESTIMATE);
    let mut items = items.peekable();
    let mut kept: Vec<B::Item> = Vec::new();
    let mut body = 0usize; // summed item_cost of `kept`
    let mut natural = true;

    // estimate edge overhead
    // TODO(AminR443): the 6KiB constant is very rough estimate. use a better estimate/method.
    let mut proof = builder.build(&[], true)?;
    let mut wire = B::wire(&proof)?;
    let fixed = wire.len().saturating_add(wire.len().max(6 * 1024)); // 6KiB

    let sufficient_fill = budget.saturating_mul(SUFFICIENT_FILL_PERCENT) / 100;

    for _ in 0..=MAX_RATIO_CORRECTION_PASSES {
        let budget_body = ratio
            .uncompressed_for(budget)
            .saturating_sub(fixed)
            .min(MAX_PAYLOAD);
        let before = kept.len();
        while let Some(peeked) = items.peek() {
            if let Ok(item) = peeked
                && !kept.is_empty()
                && body.saturating_add(B::item_cost(item)) > budget_body
            {
                break;
            }
            let Some(item) = items.next() else { break };
            let item = item?;
            body = body.saturating_add(B::item_cost(&item));
            kept.push(item);
        }
        if kept.len() == before {
            break;
        }
        natural = items.peek().is_none();
        proof = builder.build(&kept, natural)?;
        wire = B::wire(&proof)?;
        if natural || wire.len() >= sufficient_fill {
            break;
        }
        ratio = CompressionRatio::measured(wire.len(), body.saturating_add(fixed));
    }

    // Shrink: drop entries until the wire fits, but never below one so
    // paging progresses.
    while wire.len() > budget && kept.len() > 1 {
        // Entries whose share of the wire covers the overshoot, halved so a
        // heavier-than-average tail does not shrink far past the budget.
        let over = wire.len().saturating_sub(budget);
        let drop = kept
            .len()
            .saturating_mul(over)
            .div_ceil(wire.len().saturating_mul(2));
        kept.truncate(kept.len().saturating_sub(drop).max(1));
        natural = false;
        proof = builder.build(&kept, natural)?;
        wire = B::wire(&proof)?;
    }

    // Report the measured ratio so the caller can seed the next chunk.
    if !kept.is_empty() {
        let body = kept.iter().map(B::item_cost).fold(0, usize::saturating_add);
        ratio = CompressionRatio::measured(wire.len(), body.saturating_add(fixed));
    }
    Ok(SizedProof {
        proof,
        wire,
        natural_end: natural,
        ratio,
    })
}

struct RangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: &'a FrozenProof,
}

impl<T: TrieReader> ChunkBuilder for RangeChunkBuilder<'_, T> {
    type Item = (Key, Value);
    type Proof = FrozenRangeProof;

    fn item_cost((key, value): &Self::Item) -> usize {
        seq_len(key).saturating_add(seq_len(value))
    }

    fn build(&self, kvs: &[Self::Item], at_natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match kvs.last() {
            Some((last, _)) if !at_natural_end => {
                self.merkle.prove(last).map_err(api::Error::from)?
            }
            _ => Proof::default(),
        };
        Ok(RangeProof::with_hash_mode(
            self.start_proof.clone(),
            end,
            kvs.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn wire(proof: &Self::Proof) -> Result<Vec<u8>, api::Error> {
        let mut out = Vec::new();
        proof.write_to_vec(&mut out)?;
        Ok(out)
    }
}

struct ChangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: &'a FrozenProof,
}

impl<T: HashedNodeReader> ChunkBuilder for ChangeChunkBuilder<'_, T> {
    type Item = BatchOp<Key, Value>;
    type Proof = FrozenChangeProof;

    /// 1-byte tag + key, + value for `Put`.
    fn item_cost(op: &Self::Item) -> usize {
        let tag_and_key = seq_len(op.key()).saturating_add(1);
        match op {
            BatchOp::Put { value, .. } => tag_and_key.saturating_add(seq_len(value)),
            _ => tag_and_key,
        }
    }

    fn build(&self, ops: &[Self::Item], _at_natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match ops.last() {
            Some(op) => self.merkle.prove(op.key()).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        Ok(ChangeProof::with_hash_mode(
            self.start_proof.clone(),
            end,
            ops.to_vec().into_boxed_slice(),
            self.merkle.nodestore().node_hash_algorithm(),
        ))
    }

    fn wire(proof: &Self::Proof) -> Result<Vec<u8>, api::Error> {
        let mut out = Vec::new();
        proof.write_to_vec(&mut out)?;
        Ok(out)
    }
}

impl<T: TrieReader> Merkle<T> {
    /// Generates a range proof sized to target `budget` compressed
    /// wire bytes without exceeding it.
    ///
    /// # Errors
    ///
    /// * [`api::Error::RangeProofOnEmptyTrie`] - if the trie is empty and
    ///   `start_key` is `None`, matching [`Merkle::range_proof`].
    /// * Any error from proof generation ([`Merkle::prove`]) or iteration.
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
        budget: usize,
        ratio_hint: Option<CompressionRatio>,
    ) -> Result<SizedProof<FrozenRangeProof>, api::Error> {
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let items = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .map(|r| r.map_err(api::Error::from));

        let sized = stream_sized(
            &RangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint,
        )?;
        if start_key.is_none() && sized.proof.key_values().is_empty() {
            return Err(api::Error::RangeProofOnEmptyTrie);
        }
        firewood_histogram!(PROOF_KEYS, "kind" => "range")
            .record_integer(sized.proof.key_values().len());
        Ok(sized)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// Generates a change proof sized to target `budget` compressed
    /// wire bytes without exceeding it.
    ///
    /// # Errors
    ///
    /// Any error from proof generation or diff iteration.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no in-crate caller until the `Db` API exposes sized proofs"
        )
    )]
    pub(crate) fn change_proof_sized(
        &self,
        source_trie: &T,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<CompressionRatio>,
    ) -> Result<SizedProof<FrozenChangeProof>, api::Error> {
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let items = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )
        .map_err(api::Error::from)?
        .map(|r| r.map_err(api::Error::from));

        let sized = stream_sized(
            &ChangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint,
        )?;
        firewood_histogram!(PROOF_KEYS, "kind" => "change")
            .record_integer(sized.proof.batch_ops().len());
        Ok(sized)
    }
}
