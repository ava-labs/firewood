// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for post-merge hole detection: the walk over a verified proof's
//! boundary proofs that labels the key space outside the proven range.
//!
//! Most tests build a target trie `M` and a local trie `L` from flat maps,
//! take a range or change proof from `M`, verify it, run the walk against
//! `L`, and check every label against the maps directly. The checker also
//! asserts coverage: every key of either map outside the applied range lies
//! in exactly one label, and any label reaching into the applied range is a
//! deletion.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood_macros::hash_mode;
use firewood_storage::{
    Committed, EthHash, HashMode, HashType, HashedNodeReader, MemStore, MerkleDbHash,
    NodeReader as _, NodeStore, PathComponent, TriePathFromPackedBytes, replace_list_field,
};

use super::accounts::{
    account_storage_key, empty_code_hash, reopen_as_legacy, rlp_encode_account, rlp_encode_storage,
};
use super::init_merkle_in;
use crate::api::{self, BatchOp, Db as _, DbView as _, HashKey, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::merkle::Merkle;
use crate::merkle::descend::subtree_hash;
use crate::merkle::holes::{
    find_holes_after_change_proof, find_holes_after_range_proof, merge_labels,
};
use crate::proofs::holes::KeySpan;
use crate::{Hole, ProofError, VerifiedChangeProof, VerifiedRangeProof};

type Map = BTreeMap<Vec<u8>, Vec<u8>>;
type Trie<H> = Merkle<NodeStore<Committed, MemStore, H>>;

fn map(pairs: &[(&[u8], &[u8])]) -> Map {
    pairs
        .iter()
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}

fn trie<H: HashMode>(m: &Map) -> Trie<H> {
    init_merkle_in::<H, _, _, _>(m.iter().map(|(k, v)| (k.clone(), v.clone())))
}

fn root<H: HashMode>(t: &Trie<H>) -> HashKey {
    HashedNodeReader::root_hash(t.nodestore()).unwrap()
}

fn nibbles(key: &[u8]) -> Vec<PathComponent> {
    Vec::<PathComponent>::path_from_packed_bytes(key)
}

fn components(nibbles: &[u8]) -> Vec<PathComponent> {
    nibbles
        .iter()
        .map(|&n| PathComponent::try_new(n).expect("test nibble in range"))
        .collect()
}

fn span(nibbles: &[u8]) -> KeySpan {
    KeySpan::new(components(nibbles).into_iter().collect())
}

/// Take a range proof from `target`, verify it under `H`, and run the walk
/// against `local`.
fn range_holes<H: HashMode>(
    target: &Trie<H>,
    local: &Trie<H>,
    start: Option<&[u8]>,
    end: Option<&[u8]>,
    limit: Option<NonZeroUsize>,
) -> (VerifiedRangeProof, Vec<Hole>) {
    let proof = target.range_proof(start, end, limit).unwrap();
    let verified = VerifiedRangeProof::verify(
        Arc::new(proof),
        root(target),
        start,
        end,
        H::ALGORITHM,
        limit,
    )
    .unwrap();
    let holes = find_holes_after_range_proof::<H, _>(&verified, local.nodestore()).unwrap();
    (verified, holes)
}

/// The applied range of a verified range proof, as the walk infers it. A
/// proof whose start proof is empty labels nothing below its start key, so
/// no lower bound is reported and the oracle's coverage check applies only
/// above the right edge.
fn applied_range(verified: &VerifiedRangeProof) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let ctx = verified.verification();
    let start = if verified.proof().start_proof().is_empty() {
        None
    } else {
        ctx.start_key().map(<[u8]>::to_vec)
    };
    (start, ctx.right_edge_key().map(<[u8]>::to_vec))
}

fn in_applied(key: &[u8], (start, end): &(Option<Vec<u8>>, Option<Vec<u8>>)) -> bool {
    start.as_deref().is_none_or(|s| key >= s) && end.as_deref().is_none_or(|e| key <= e)
}

fn sub(m: &Map, prefix: &[PathComponent]) -> Map {
    m.iter()
        .filter(|(k, _)| nibbles(k).starts_with(prefix))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn is_deletion(hole: &Hole) -> bool {
    matches!(hole, Hole::Surplus(_) | Hole::PointSurplus { .. })
}

/// Whether `hole` covers `key`: the key lies under the span, or is the point.
fn covers(hole: &Hole, key: &[u8]) -> bool {
    match hole {
        Hole::Missing(s) | Hole::Stale(s) | Hole::Surplus(s) | Hole::Synced(s) => {
            nibbles(key).starts_with(s.prefix())
        }
        Hole::PointFix { key: k, .. }
        | Hole::PointStale { key: k }
        | Hole::PointSurplus { key: k } => **k == *key,
    }
}

/// Whether two values for `key` count as equal to the walk: byte equality,
/// except that under the Ethereum mode an account value (32-byte key) is
/// compared with its `storageRoot` field masked, since hashing derives that
/// field and the walk reports storage differences through spans.
fn values_equal<H: HashMode>(key: &[u8], a: &[u8], b: &[u8]) -> bool {
    if H::ALGORITHM.is_ethereum()
        && key.len() == 32
        && let (Ok(a), Ok(b)) = (
            replace_list_field(a, 2, &[0; 32]),
            replace_list_field(b, 2, &[0; 32]),
        )
    {
        return a == b;
    }
    // A value that is not well-formed account RLP is compared as-is, as the
    // walk does.
    a == b
}

/// Whether two maps, already restricted to the same key span by the caller,
/// hold equal content by [`values_equal`].
fn equal_under<H: HashMode>(t: &Map, l: &Map) -> bool {
    t.len() == l.len()
        && t.iter()
            .all(|(k, v)| l.get(k).is_some_and(|w| values_equal::<H>(k, v, w)))
}

/// Check every label against the flat maps and check coverage of the
/// complement of the applied range.
fn assert_matches_truth<H: HashMode>(
    target: &Map,
    local: &Map,
    applied: &(Option<Vec<u8>>, Option<Vec<u8>>),
    holes: &[Hole],
) {
    for hole in holes {
        match hole {
            Hole::Missing(s) | Hole::Stale(s) | Hole::Surplus(s) | Hole::Synced(s) => {
                let t = sub(target, s.prefix());
                let l = sub(local, s.prefix());
                let expected = match (t.is_empty(), l.is_empty()) {
                    (false, true) => "Missing",
                    (true, false) => "Surplus",
                    (false, false) if equal_under::<H>(&t, &l) => "Synced",
                    (false, false) => "Stale",
                    (true, true) => "silent",
                };
                let actual = match hole {
                    Hole::Missing(_) => "Missing",
                    Hole::Stale(_) => "Stale",
                    Hole::Surplus(_) => "Surplus",
                    _ => "Synced",
                };
                assert_eq!(actual, expected, "span {:?}", s.prefix());
            }
            Hole::PointFix { key, value } => {
                let t = target.get(&**key).expect("the target holds a fixed key");
                assert!(
                    values_equal::<H>(key, t, value),
                    "{key:02x?}: target {t:02x?}, label {value:02x?}"
                );
                assert!(
                    !local
                        .get(&**key)
                        .is_some_and(|l| values_equal::<H>(key, l, value)),
                    "{key:02x?}"
                );
            }
            Hole::PointStale { key } => {
                let t = target.get(&**key).expect("the target holds a stale key");
                assert!(t.len() >= 32, "a digest-only value is at least 32 bytes");
                assert_ne!(local.get(&**key), Some(t), "{key:02x?}");
            }
            Hole::PointSurplus { key } => {
                assert!(target.get(&**key).is_none(), "{key:02x?}");
                assert!(local.get(&**key).is_some(), "{key:02x?}");
            }
        }
    }

    for key in target.keys().chain(local.keys()) {
        let covering: Vec<&Hole> = holes.iter().filter(|h| covers(h, key)).collect();
        if in_applied(key, applied) {
            assert!(
                covering.iter().all(|h| is_deletion(h)),
                "key {key:02x?} inside the applied range is covered by {covering:?}"
            );
        } else {
            // The walk probes two kinds of point: byte keys that are proper
            // prefixes of the start key, and the end proof's terminal when it
            // lies above the proven edge, which is the target's first key past
            // it. A probed point whose value both sides share is silent, so
            // only those keys may go uncovered; every other key outside the
            // applied range is covered exactly once.
            let (start, end) = applied;
            let prefix_point = start
                .as_deref()
                .is_some_and(|s| s.starts_with(key) && key.len() < s.len());
            let successor_point = end.as_deref().is_some_and(|e| {
                target
                    .keys()
                    .find(|k| k.as_slice() > e)
                    .is_some_and(|k| k.as_slice() == key)
            });
            let both_equal = match (target.get(key), local.get(key)) {
                (Some(t), Some(l)) => values_equal::<H>(key, t, l),
                _ => false,
            };
            if (prefix_point || successor_point) && both_equal && covering.is_empty() {
                continue;
            }
            assert_eq!(
                covering.len(),
                1,
                "key {key:02x?} outside the applied range is covered by {covering:?}"
            );
        }
    }

    let mut lower_bounds: Vec<Vec<u8>> = holes
        .iter()
        .map(|h| match h {
            Hole::Missing(s) | Hole::Stale(s) | Hole::Surplus(s) | Hole::Synced(s) => {
                s.as_key_range().0.to_vec()
            }
            Hole::PointFix { key, .. } | Hole::PointStale { key } | Hole::PointSurplus { key } => {
                key.to_vec()
            }
        })
        .collect();
    let sorted = lower_bounds.clone();
    lower_bounds.sort();
    assert_eq!(sorted, lower_bounds, "labels are sorted by lowest key");
}

fn spans(holes: &[Hole]) -> Vec<(&'static str, Vec<u8>)> {
    holes
        .iter()
        .filter_map(|h| {
            let (label, s) = match h {
                Hole::Missing(s) => ("Missing", s),
                Hole::Stale(s) => ("Stale", s),
                Hole::Surplus(s) => ("Surplus", s),
                Hole::Synced(s) => ("Synced", s),
                _ => return None,
            };
            Some((label, s.prefix().iter().map(|c| c.as_u8()).collect()))
        })
        .collect()
}

fn fetch_labels(holes: &[Hole]) -> usize {
    holes
        .iter()
        .filter(|h| {
            matches!(
                h,
                Hole::Missing(_) | Hole::Stale(_) | Hole::PointStale { .. }
            )
        })
        .count()
}

/// The target of the worked examples: a branch at `[A,7]` with leaves
/// `0xA711`, `0xA777`, `0xA7FF`, and a leaf `0xB0` under the root.
fn worked_target() -> Map {
    map(&[
        (&[0xA7, 0x11], b"one"),
        (&[0xA7, 0x77], b"two"),
        (&[0xA7, 0xFF], b"three"),
        (&[0xB0], b"four"),
    ])
}

#[hash_mode]
#[test]
fn missing_hole<H: HashMode>() {
    // A fresh client holding a stale `0xB0` synced `[MIN, 0xA74F]` and
    // received `{0xA711}`. The end proof is an exclusion at `[A,7]` whose slot
    // 4 is absent; the walk reports the two sibling leaves it never received
    // and the stale `B` subtree, and nothing else.
    let target = worked_target();
    let local = map(&[(&[0xA7, 0x11], b"one"), (&[0xB0], b"OLD")]);
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA7, 0x4F]), None);

    assert_eq!(
        spans(&holes),
        vec![
            ("Missing", vec![0xA, 0x7, 0x7]),
            ("Missing", vec![0xA, 0x7, 0xF]),
            ("Stale", vec![0xB]),
        ]
    );
    assert_eq!(holes.len(), 3);
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn stale_hole<H: HashMode>() {
    // One wrong value under a sibling stub.
    let target = worked_target();
    let mut local = target.clone();
    local.insert(vec![0xA7, 0xFF], b"wrong".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA7, 0x4F]), None);

    assert!(spans(&holes).contains(&("Stale", vec![0xA, 0x7, 0xF])));
    assert!(spans(&holes).contains(&("Synced", vec![0xA, 0x7, 0x7])));
    assert!(spans(&holes).contains(&("Synced", vec![0xB])));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn synced_span_skipped<H: HashMode>() {
    // Local content equal to the target under every stub: only Synced labels,
    // and a second round over any Synced span finds nothing to fetch.
    let target = worked_target();
    let (t, l) = (trie::<H>(&target), trie::<H>(&target));
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA7, 0x4F]), None);

    assert_eq!(fetch_labels(&holes), 0);
    assert!(holes.iter().all(|h| matches!(h, Hole::Synced(_))));
    assert_matches_truth::<H>(&target, &target, &applied_range(&verified), &holes);

    // Re-request one Synced span: the proof over it labels nothing.
    let (_, again) = range_holes(&t, &l, Some(&[0xB0]), Some(&[0xBF]), None);
    assert_eq!(fetch_labels(&again), 0);
}

#[hash_mode]
#[test]
fn surplus_hole<H: HashMode>() {
    // Local keys under an absent child of the root (`[C]`) and under a branch
    // the target's compressed path `[7]` at `A` implies empty (`[A,8]`).
    let target = map(&[(&[0xA7, 0x11], b"one"), (&[0xA7, 0x77], b"two")]);
    let mut local = target.clone();
    local.insert(vec![0xC0, 0x00], b"extra".to_vec());
    local.insert(vec![0xA8, 0x00], b"extra".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA7, 0x4F]), None);

    assert!(spans(&holes).contains(&("Surplus", vec![0xC])));
    assert!(spans(&holes).contains(&("Surplus", vec![0xA, 0x8])));
    assert_eq!(
        fetch_labels(&holes),
        0,
        "the local trie holds every target key"
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn mid_edge_adjustment<H: HashMode>() {
    // The local trie holds only `0xA711`, as a single leaf with partial path
    // `[A,7,1,1]`. The left proof for `0xA777` carries the stub for `[A,7,1]`,
    // which commits to a leaf with partial path `[1]`. The probe lands inside
    // the local leaf's edge; re-encoding with the adjusted split matches the
    // stub, where the stored hash of the whole leaf would not.
    let target = map(&[
        (&[0xA7, 0x11], b"one"),
        (&[0xA7, 0x77], b"two"),
        (&[0xB0, 0x55], b"three"),
    ]);
    let local = map(&[(&[0xA7, 0x11], b"one")]);
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, Some(&[0xA7, 0x77]), None, None);

    assert!(spans(&holes).contains(&("Synced", vec![0xA, 0x7, 0x1])));
    assert!(!spans(&holes).iter().any(|(label, _)| *label == "Stale"));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);

    // The adjustment has teeth only where the partial path is part of the
    // preimage. MerkleDB hashes the full path, so the re-encoded hash equals
    // the stored one; the Ethereum mode hashes the partial path, so it must
    // differ from the stored hash of the whole leaf while matching the stub.
    let stored = HashedNodeReader::root_hash(l.nodestore()).map(HashType::from);
    let probed = subtree_hash::<H, _>(l.nodestore(), &components(&[0xA, 0x7, 0x1])).unwrap();
    if H::ALGORITHM.is_ethereum() {
        assert_ne!(probed, stored);
    } else {
        assert_eq!(probed, stored);
    }
}

#[hash_mode]
#[test]
fn prefix_value_surplus<H: HashMode>() {
    // A local value at `0xA7`, a proper prefix of the start key, where the
    // target's node at `[A,7]` carries no value.
    let target = map(&[(&[0xA7, 0x11], b"one"), (&[0xA7, 0x77], b"two")]);
    let mut local = target.clone();
    local.insert(vec![0xA7], b"prefix".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, Some(&[0xA7, 0x11]), None, None);

    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::PointSurplus { key } if **key == [0xA7]))
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn path_value_repair<H: HashMode>() {
    // The target's path node at `[A,7]` carries a value at key `0xA7`, a
    // proper prefix of the start key; the proof carries it in full, so the
    // local side is repaired without a fetch whether it is absent or wrong.
    let target = map(&[
        (&[0xA7], b"value"),
        (&[0xA7, 0x11], b"one"),
        (&[0xA7, 0x77], b"two"),
    ]);
    let mut local = target.clone();
    local.remove(&vec![0xA7]);
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, Some(&[0xA7, 0x11]), None, None);
    assert!(holes.iter().any(
        |h| matches!(h, Hole::PointFix { key, value } if **key == [0xA7] && &**value == b"value")
    ));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);

    local.insert(vec![0xA7], b"wrong".to_vec());
    let l = trie::<H>(&local);
    let (verified, holes) = range_holes(&t, &l, Some(&[0xA7, 0x11]), None, None);
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::PointFix { key, .. } if **key == [0xA7]))
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[test]
fn hashed_path_value_is_point_stale() {
    // Proofs built here carry values in full, but a MerkleDB proof from
    // another implementation carries only the digest of a value of 32 bytes
    // or more. The path node at `0xA7` is below the start key, so the
    // verifier does not check its value against the key-value pairs and the
    // hashed form verifies. A differing local value then needs a fetch:
    // PointStale, since there is no value in hand to write.
    type H = MerkleDbHash;
    let long = vec![0x5A; 40];
    let target = map(&[
        (&[0xA7], long.as_slice()),
        (&[0xA7, 0x11], b"one"),
        (&[0xA7, 0x77], b"two"),
    ]);
    let mut local = target.clone();
    local.insert(vec![0xA7], b"wrong".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let start: &[u8] = &[0xA7, 0x11];

    let honest = t.range_proof(Some(start), None, None).unwrap();
    let mut start_nodes: Vec<crate::ProofNode> = honest.start_proof().as_ref().to_vec();
    let path_node = start_nodes
        .iter_mut()
        .find(|n| n.key.as_slice() == components(&[0xA, 0x7]).as_slice())
        .expect("the start proof passes through the path node");
    let digest = path_node
        .value_digest
        .take()
        .expect("the path node carries a value");
    path_node.value_digest = Some(digest.make_hash(H::ALGORITHM).map(Box::from));
    let hashed = crate::RangeProof::with_hash_mode(
        crate::Proof::new(start_nodes.into_boxed_slice()),
        honest.end_proof().clone(),
        honest.key_values().to_vec().into_boxed_slice(),
        H::ALGORITHM,
    );
    let verified =
        VerifiedRangeProof::verify(hashed, root(&t), Some(start), None, H::ALGORITHM, None)
            .expect("a hashed out-of-range value verifies");
    let holes = find_holes_after_range_proof::<H, _>(&verified, l.nodestore()).unwrap();
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::PointStale { key } if **key == [0xA7])),
        "{holes:?}"
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn straddle_terminal_surplus<H: HashMode>() {
    // Absent-child terminal: the end proof for `0xA74F` ends at `[A,7]` with
    // slot 4 absent, so `[A,7,4]` is proven empty over its whole extent. Local
    // keys there on both sides of the boundary are Surplus, including the
    // ones inside the applied range.
    let target = map(&[(&[0xA7, 0x11], b"one"), (&[0xA7, 0x77], b"two")]);
    let mut local = target.clone();
    local.insert(vec![0xA7, 0x40], b"in".to_vec());
    local.insert(vec![0xA7, 0x4F], b"edge".to_vec());
    local.insert(vec![0xA7, 0x4A], b"out".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA7, 0x4F]), None);
    assert!(spans(&holes).contains(&("Surplus", vec![0xA, 0x7, 0x4])));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);

    // Divergence terminal: the end proof for `0xA800` diverges inside `A`'s
    // partial path `[7]`, so `[A,8]` is proven empty. The applied range ends
    // at the requested `0xA800`, and local keys `0xA800` and `0xA8FF` straddle
    // it.
    let mut local = target.clone();
    local.insert(vec![0xA8, 0x00], b"in".to_vec());
    local.insert(vec![0xA8, 0xFF], b"out".to_vec());
    let l = trie::<H>(&local);
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xA8, 0x00]), None);
    assert!(spans(&holes).contains(&("Surplus", vec![0xA, 0x8])));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn kb_exhausted_mid_edge<H: HashMode>() {
    // The requested end `0xB0` ends inside the partial path `[0,5,5]` of the
    // branch under `B`, which carries no value, so the right edge stays at the
    // requested key and the terminal is the mid-edge case: the node's
    // continuation `[B,0,5]` is reported above via a re-encoded hash and the
    // other continuations are empty.
    let target = map(&[
        (&[0xA7, 0x11], b"one"),
        (&[0xA7, 0x77], b"two"),
        (&[0xB0, 0x55, 0x11], b"three"),
        (&[0xB0, 0x55, 0x22], b"four"),
    ]);
    let t = trie::<H>(&target);

    let l = trie::<H>(&target);
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xB0]), None);
    assert_eq!(verified.verification().right_edge_key(), Some(&[0xB0][..]));
    assert!(spans(&holes).contains(&("Synced", vec![0xB, 0x0, 0x5])));
    assert_matches_truth::<H>(&target, &target, &applied_range(&verified), &holes);

    let local = map(&[(&[0xA7, 0x11], b"one"), (&[0xA7, 0x77], b"two")]);
    let l = trie::<H>(&local);
    let (verified, holes) = range_holes(&t, &l, None, Some(&[0xB0]), None);
    assert!(spans(&holes).contains(&("Missing", vec![0xB, 0x0, 0x5])));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn empty_start_proof_emits_nothing_below_start<H: HashMode>() {
    // A proof generated for an unbounded start has an empty start proof. The
    // verifier accepts it for a requested start key as well, but it adopts
    // end-proof node values below that key without checking them against
    // the key-value pairs, so the space below the start key is not
    // authenticated empty and the walk labels none of it.
    let target = map(&[(&[0x20], b"a"), (&[0x30], b"b"), (&[0x40], b"c")]);
    let mut local = target.clone();
    local.insert(vec![0x05], b"low".to_vec());
    local.insert(vec![0x0A, 0x0A], b"low".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));

    let proof = t.range_proof(None, Some(&[0x30]), None).unwrap();
    assert!(proof.start_proof().is_empty());
    let start: &[u8] = &[0x10];
    let verified = VerifiedRangeProof::verify(
        Arc::new(proof),
        root(&t),
        Some(start),
        Some(&[0x30]),
        H::ALGORITHM,
        None,
    )
    .unwrap();
    let holes = find_holes_after_range_proof::<H, _>(&verified, l.nodestore()).unwrap();

    for key in local.keys().filter(|k| k.as_slice() < start) {
        assert!(
            holes.iter().all(|h| !covers(h, key)),
            "{key:02x?} below the start key is labelled by {holes:?}"
        );
    }
    assert!(spans(&holes).contains(&("Synced", vec![0x4])));
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[test]
fn tampered_partial_len_does_not_steer_the_walk() {
    // Under MerkleDB the hash preimage carries a node's full path but not
    // its partial length, so a proof whose `partial_len` was altered still
    // verifies. The walk takes structure from the authenticated key, so the
    // labels match those of the untampered proof. The Ethereum preimage
    // includes the partial path, so the tamper is rejected there and this
    // test is MerkleDB-only.
    type H = MerkleDbHash;
    let target = worked_target();
    let local = map(&[(&[0xB0], b"stale")]);
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let end: &[u8] = &[0xA7, 0x4F];
    let (_, expected) = range_holes(&t, &l, None, Some(end), None);

    let honest = t.range_proof(None, Some(end), None).unwrap();
    let mut end_nodes: Vec<crate::ProofNode> = honest.end_proof().as_ref().to_vec();
    assert!(
        end_nodes.len() >= 2,
        "the end proof descends below the root"
    );
    for node in end_nodes.iter_mut().skip(1) {
        node.partial_len = 0;
    }
    let tampered = crate::RangeProof::with_hash_mode(
        honest.start_proof().clone(),
        crate::Proof::new(end_nodes.into_boxed_slice()),
        honest.key_values().to_vec().into_boxed_slice(),
        H::ALGORITHM,
    );
    let verified =
        VerifiedRangeProof::verify(tampered, root(&t), None, Some(end), H::ALGORITHM, None)
            .expect("MerkleDB does not authenticate partial_len");
    let holes = find_holes_after_range_proof::<H, _>(&verified, l.nodestore()).unwrap();
    assert_eq!(holes, expected);
}

#[hash_mode]
#[test]
fn out_of_range_interval_covered<H: HashMode>() {
    // Target `{0x10, 0x28}`, local `{0x10, 0x26, 0x28}`, request `[0x00,
    // 0x25]`. The end proof's terminal is the leaf for `0x28`, outside the
    // request, so the proven right edge falls back to `0x25`; `0x26` lies in
    // the interval `(0x25, 0x28)` that neither the merge nor the walk above
    // `0x28` touches, and must still be labelled.
    let target = map(&[(&[0x10], b"a"), (&[0x28], b"b")]);
    let local = map(&[(&[0x10], b"a"), (&[0x26], b"x"), (&[0x28], b"b")]);
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));
    let (verified, holes) = range_holes(&t, &l, Some(&[0x00]), Some(&[0x25]), None);

    assert_eq!(verified.verification().right_edge_key(), Some(&[0x25][..]));
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::Surplus(_)) && covers(h, &[0x26]))
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);

    // The terminal key itself is probed as a point: a differing local value
    // is repaired from the proof.
    let mut local = local.clone();
    local.insert(vec![0x28], b"old".to_vec());
    let l = trie::<H>(&local);
    let (verified, holes) = range_holes(&t, &l, Some(&[0x00]), Some(&[0x25]), None);
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::PointFix { key, .. } if **key == [0x28]))
    );
    assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
}

#[hash_mode]
#[test]
fn holes_partition<H: HashMode>() {
    // A larger shaped pair: the local trie drops a span, alters values, and
    // adds surplus keys. Every label must match the maps and the complement
    // of the applied range must be covered exactly once.
    let mut target = Map::new();
    for i in 0u8..48 {
        target.insert(vec![i.wrapping_mul(5), i ^ 0x3C], vec![i; 3]);
    }
    let mut local = target.clone();
    let keys: Vec<Vec<u8>> = target.keys().cloned().collect();
    for key in keys.iter().step_by(7) {
        local.remove(key);
    }
    for key in keys.iter().skip(3).step_by(11) {
        local.insert(key.clone(), b"altered".to_vec());
    }
    local.insert(vec![0x33, 0x33, 0x33], b"surplus".to_vec());
    local.insert(vec![0xFE], b"surplus".to_vec());
    let (t, l) = (trie::<H>(&target), trie::<H>(&local));

    for (start, end, limit) in [
        (Some(&keys[10][..]), Some(&keys[25][..]), None),
        (None, Some(&keys[25][..]), NonZeroUsize::new(5)),
        (Some(&keys[30][..]), None, NonZeroUsize::new(3)),
        (Some(&[0x00][..]), Some(&[0xFF][..]), NonZeroUsize::new(9)),
    ] {
        let (verified, holes) = range_holes(&t, &l, start, end, limit);
        assert_matches_truth::<H>(&target, &local, &applied_range(&verified), &holes);
    }
}

#[hash_mode]
#[test]
fn fully_synced_emits_nothing<H: HashMode>() {
    // `L == M`: random boundaries yield only Synced spans and silence.
    let mut target = Map::new();
    for i in 0u8..40 {
        target.insert(vec![i.wrapping_mul(7), i], vec![i; 2]);
    }
    let keys: Vec<Vec<u8>> = target.keys().cloned().collect();
    let (t, l) = (trie::<H>(&target), trie::<H>(&target));
    for (start, end, limit) in [
        (Some(&keys[5][..]), Some(&keys[20][..]), None),
        (None, Some(&keys[33][..]), NonZeroUsize::new(4)),
        (Some(&keys[2][..]), None, NonZeroUsize::new(6)),
        (None, None, NonZeroUsize::new(1)),
    ] {
        let (_, holes) = range_holes(&t, &l, start, end, limit);
        assert!(!holes.is_empty(), "a bounded request leaves synced space");
        assert!(
            holes.iter().all(|h| matches!(h, Hole::Synced(_))),
            "{holes:?}"
        );
    }
}

#[hash_mode]
#[test]
fn change_proof_labels_match_range_proof_labels<H: HashMode>() {
    // The same target/local pair classified through a change proof's
    // boundary proofs yields the same span labels as the range-proof path
    // over the same bounds.
    let source = map(&[(&[0xA7, 0x11], b"one"), (&[0xB0], b"four")]);
    let target = worked_target();
    let local = map(&[(&[0xA7, 0xFF], b"three"), (&[0xB0], b"OLD")]);
    let (s, t, l) = (trie::<H>(&source), trie::<H>(&target), trie::<H>(&local));

    let start: &[u8] = &[0xA7, 0x20];
    let end: &[u8] = &[0xA7, 0x80];
    let proof = t
        .change_proof(Some(start), Some(end), s.nodestore(), None)
        .unwrap();
    let verified = VerifiedChangeProof::verify(
        Arc::new(proof),
        root(&t),
        Some(start),
        Some(end),
        H::ALGORITHM,
        None,
    )
    .unwrap();
    let change = find_holes_after_change_proof::<H, _>(&verified, l.nodestore()).unwrap();
    let (_, range) = range_holes(&t, &l, Some(start), Some(end), None);

    assert_eq!(spans(&change), spans(&range));
    assert_eq!(
        spans(&change),
        vec![
            ("Missing", vec![0xA, 0x7, 0x1]),
            ("Synced", vec![0xA, 0x7, 0xF]),
            ("Stale", vec![0xB]),
        ]
    );
}

#[hash_mode]
#[test]
fn change_proof_empty_start_emits_nothing<H: HashMode>() {
    // A change proof with an empty start proof and a requested start key
    // verifies, and the walk emits nothing below the start key. The
    // verifier's root check hashes the proposal's own content there and
    // strips off-path content under intermediate branches without
    // authenticating it, so a root match proves neither that the local
    // content below the start key equals the target's nor that it is
    // absent; no label is justified.
    let source = map(&[(&[0xA7, 0x11], b"one")]);
    let target = worked_target();
    let mut local = target.clone();
    local.insert(vec![0x05], b"below".to_vec());
    local.insert(vec![0x50], b"below".to_vec());
    let (s, t, l) = (trie::<H>(&source), trie::<H>(&target), trie::<H>(&local));

    let end: &[u8] = &[0xFF];
    let generated = t
        .change_proof(None, Some(end), s.nodestore(), None)
        .unwrap();
    assert!(generated.start_proof().is_empty());
    let proof = crate::ChangeProof::with_hash_mode(
        crate::Proof::new(Box::<[crate::ProofNode]>::default()),
        generated.end_proof().clone(),
        generated.batch_ops().to_vec().into_boxed_slice(),
        H::ALGORITHM,
    );
    let start: &[u8] = &[0x60];
    let verified = VerifiedChangeProof::verify(
        Arc::new(proof),
        root(&t),
        Some(start),
        Some(end),
        H::ALGORITHM,
        None,
    )
    .unwrap();
    let holes = find_holes_after_change_proof::<H, _>(&verified, l.nodestore()).unwrap();

    assert!(
        holes
            .iter()
            .all(|h| !covers(h, &[0x05]) && !covers(h, &[0x50])),
        "{holes:?}"
    );
}

#[test]
fn hash_mode_mismatch_rejected() {
    let target = worked_target();
    let t = trie::<MerkleDbHash>(&target);
    let proof = t.range_proof(None, Some(&[0xA7, 0x4F]), None).unwrap();
    let verified = VerifiedRangeProof::verify(
        Arc::new(proof),
        root(&t),
        None,
        Some(&[0xA7, 0x4F]),
        MerkleDbHash::ALGORITHM,
        None,
    )
    .unwrap();
    let result = find_holes_after_range_proof::<EthHash, _>(&verified, t.nodestore());
    assert!(matches!(
        result,
        Err(api::Error::ProofError(ProofError::HashModeMismatch { .. }))
    ));
}

#[test]
fn overlap_invariant_errors() {
    let key = |k: &[u8]| Box::<[u8]>::from(k);
    // Two deletions may overlap; redundant deletes are idempotent.
    assert!(
        merge_labels(vec![
            Hole::Surplus(span(&[0xA])),
            Hole::Surplus(span(&[0xA, 0x7]))
        ])
        .is_ok()
    );
    assert!(
        merge_labels(vec![
            Hole::Surplus(span(&[0xA])),
            Hole::PointSurplus { key: key(&[0xA7]) },
        ])
        .is_ok()
    );
    // Disjoint labels of any kind are fine.
    assert!(
        merge_labels(vec![
            Hole::Synced(span(&[0xA])),
            Hole::Missing(span(&[0xB]))
        ])
        .is_ok()
    );
    // An authenticated-empty span over content the other label asserts.
    for other in [
        Hole::Synced(span(&[0xA, 0x7])),
        Hole::Missing(span(&[0xA, 0x7])),
        Hole::Stale(span(&[0xA, 0x7])),
        Hole::PointFix {
            key: key(&[0xA7]),
            value: key(b"v"),
        },
    ] {
        assert!(
            matches!(
                merge_labels(vec![Hole::Surplus(span(&[0xA])), other.clone()]),
                Err(api::Error::InternalError(_))
            ),
            "{other:?}"
        );
    }
    // Sorted by lowest key.
    let sorted = merge_labels(vec![
        Hole::Missing(span(&[0xB])),
        Hole::PointStale { key: key(&[0x05]) },
        Hole::Synced(span(&[0xA, 0x7])),
    ])
    .unwrap();
    assert!(matches!(sorted[0], Hole::PointStale { .. }));
    assert!(matches!(&sorted[1], Hole::Synced(s) if s.prefix() == components(&[0xA, 0x7])));
    assert!(matches!(&sorted[2], Hole::Missing(s) if s.prefix() == components(&[0xB])));
}

fn new_db<H: HashMode>() -> (Db<H>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::<H>::new_with_hash_mode(
        dir.path(),
        DbConfig::builder()
            .node_hash_algorithm(H::ALGORITHM)
            .build(),
    )
    .unwrap();
    (db, dir)
}

fn commit<H: HashMode>(db: &Db<H>, m: &Map) -> HashKey {
    let batch: Vec<BatchOp<&[u8], &[u8]>> = m
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_slice(),
            value: v.as_slice(),
        })
        .collect();
    db.propose(batch).unwrap().commit().unwrap();
    db.root_hash().unwrap()
}

#[hash_mode]
#[test]
fn erased_views_reach_the_walk<H: HashMode>() {
    // The public surface: a committed view and a reconstructed view of the
    // local database each classify the same proof the way the generic walk
    // over an in-memory trie does. The reconstructed view's root is hashed
    // lazily, so this also exercises the `root_hash()` pre-call.
    let target = worked_target();
    let local = map(&[(&[0xA7, 0x11], b"one"), (&[0xB0], b"OLD")]);
    let (target_db, _target_dir) = new_db::<H>();
    let (local_db, _local_dir) = new_db::<H>();
    let target_root = commit(&target_db, &target);

    // Commit the local state without `0xB0`, then reconstruct it with the
    // `0xB0` write so the reconstructed view holds the full local map.
    let mut local_base = local.clone();
    local_base.remove(&vec![0xB0]);
    let local_root = commit(&local_db, &local_base);

    let end: &[u8] = &[0xA7, 0x4F];
    let proof = target_db
        .revision(target_root.clone())
        .unwrap()
        .range_proof(None, Some(end), None)
        .unwrap();
    let verified = VerifiedRangeProof::verify(
        Arc::new(proof),
        target_root,
        None,
        Some(end),
        H::ALGORITHM,
        None,
    )
    .unwrap();

    let committed = local_db.committed_view(local_root).unwrap();
    let from_committed = committed.find_holes_after_range_proof(&verified).unwrap();
    assert_eq!(
        spans(&from_committed),
        vec![
            ("Missing", vec![0xA, 0x7, 0x7]),
            ("Missing", vec![0xA, 0x7, 0xF]),
            ("Missing", vec![0xB]),
        ]
    );

    let reconstructed = local_db
        .reconstruct_from_view(
            &committed,
            vec![BatchOp::Put {
                key: &[0xB0][..],
                value: &b"OLD"[..],
            }],
        )
        .unwrap();
    let from_reconstructed = reconstructed
        .find_holes_after_range_proof(&verified)
        .unwrap();
    let expected = {
        let (t, l) = (trie::<H>(&target), trie::<H>(&local));
        let (_, holes) = range_holes(&t, &l, None, Some(end), None);
        holes
    };
    assert_eq!(from_reconstructed, expected);
    assert!(spans(&from_reconstructed).contains(&("Stale", vec![0xB])));
}

// Account-shaped fixtures. These run under the Ethereum mode only: depth 64
// is the account boundary there and has no meaning under MerkleDB.

const ACCOUNT_A: [u8; 32] = [0x11; 32];
const ACCOUNT_B: [u8; 32] = [0x22; 32];

fn account(nonce: u64, storage_root: u8) -> Vec<u8> {
    rlp_encode_account(nonce, 1_000, &[storage_root; 32], &empty_code_hash()).into_vec()
}

/// Account A with storage in slots 1, 2, and 3, and account B with none. The
/// inserted `storageRoot` placeholders are deliberately wrong: hashing
/// derives the real field.
fn account_target() -> Map {
    let mut m = Map::new();
    m.insert(ACCOUNT_A.to_vec(), account(1, 0xAA));
    for slot in [0x10, 0x20, 0x30] {
        m.insert(
            account_storage_key(&ACCOUNT_A, slot).into_vec(),
            rlp_encode_storage(&[slot; 32]),
        );
    }
    m.insert(ACCOUNT_B.to_vec(), account(7, 0xBB));
    m
}

/// A range covering exactly A's slot-1 entry, so both boundary keys run
/// through account A: the Below walk probes A's own key as a point, and the
/// Above walk compares A's remaining storage slots as spans.
fn through_account_a<'a>() -> (Option<&'a [u8]>, Option<&'a [u8]>) {
    static SLOT_ONE: std::sync::LazyLock<Box<[u8]>> =
        std::sync::LazyLock::new(|| account_storage_key(&ACCOUNT_A, 0x10));
    (Some(&SLOT_ONE), Some(&SLOT_ONE))
}

fn point_labels(holes: &[Hole]) -> Vec<&Hole> {
    holes
        .iter()
        .filter(|h| {
            matches!(
                h,
                Hole::PointFix { .. } | Hole::PointStale { .. } | Hole::PointSurplus { .. }
            )
        })
        .collect()
}

#[test]
fn account_storage_differs_fields_agree() {
    // The target's account value and the local one differ only in the
    // storage root hashing derives, because slot 3 is missing locally. The
    // point probe at the account key is silent; the span over slot 3 reports
    // the storage difference.
    let target = account_target();
    let mut local = target.clone();
    let slot_three = account_storage_key(&ACCOUNT_A, 0x30);
    local.remove(&*slot_three);
    let (t, l) = (trie::<EthHash>(&target), trie::<EthHash>(&local));
    let (start, end) = through_account_a();
    let (verified, holes) = range_holes(&t, &l, start, end, None);

    assert!(point_labels(&holes).is_empty(), "{holes:?}");
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::Missing(_)) && covers(h, &slot_three)),
        "{holes:?}"
    );
    assert_matches_truth::<EthHash>(&target, &local, &applied_range(&verified), &holes);
}

#[test]
fn account_fields_differ_storage_agrees() {
    // Same storage on both sides, different nonce: exactly one `PointFix`
    // carrying the target's account value, and nothing to fetch.
    let target = account_target();
    let mut local = target.clone();
    local.insert(ACCOUNT_A.to_vec(), account(2, 0xCC));
    let (t, l) = (trie::<EthHash>(&target), trie::<EthHash>(&local));
    let (start, end) = through_account_a();
    let (verified, holes) = range_holes(&t, &l, start, end, None);

    // The label carries the target's stored value, whose `storageRoot` is
    // the one hashing derived, not the inserted placeholder.
    let points = point_labels(&holes);
    assert_eq!(points.len(), 1, "{holes:?}");
    assert!(
        matches!(
            points[0],
            Hole::PointFix { key, value }
                if **key == ACCOUNT_A && values_equal::<EthHash>(key, value, &account(1, 0xAA))
        ),
        "{points:?}"
    );
    assert_eq!(fetch_labels(&holes), 0, "{holes:?}");
    assert_matches_truth::<EthHash>(&target, &local, &applied_range(&verified), &holes);
}

#[test]
fn account_fields_and_storage_differ() {
    // Both differ: the point fix for the account fields and the span for the
    // storage, each reported once.
    let target = account_target();
    let mut local = target.clone();
    local.insert(ACCOUNT_A.to_vec(), account(2, 0xCC));
    let slot_three = account_storage_key(&ACCOUNT_A, 0x30);
    local.insert(slot_three.to_vec(), rlp_encode_storage(&[0xFF; 32]));
    let (t, l) = (trie::<EthHash>(&target), trie::<EthHash>(&local));
    let (start, end) = through_account_a();
    let (verified, holes) = range_holes(&t, &l, start, end, None);

    assert_eq!(point_labels(&holes).len(), 1, "{holes:?}");
    assert!(
        matches!(point_labels(&holes)[0], Hole::PointFix { key, .. } if **key == ACCOUNT_A),
        "{holes:?}"
    );
    assert_eq!(fetch_labels(&holes), 1, "{holes:?}");
    assert!(
        holes
            .iter()
            .any(|h| matches!(h, Hole::Stale(_)) && covers(h, &slot_three)),
        "{holes:?}"
    );
    assert_matches_truth::<EthHash>(&target, &local, &applied_range(&verified), &holes);
}

#[test]
fn legacy_header_fully_synced_emits_nothing() {
    // A pre-hfix local database stores account values with a stale
    // `storageRoot` while its node hashes are canonical. The walk does not
    // read the header flag: with `L == M`, boundaries through both accounts
    // yield only Synced spans and silence.
    let target = account_target();
    let (t, l) = (trie::<EthHash>(&target), trie::<EthHash>(&target));
    let legacy = reopen_as_legacy(&l, &[&ACCOUNT_A, &ACCOUNT_B]);
    assert!(legacy.must_recompute_storage_hash());
    assert_eq!(HashedNodeReader::root_hash(&legacy), Some(root(&t)));
    // Non-vacuous: the stored account value really is stale now.
    assert_ne!(
        Merkle::from(&legacy).get_value(&ACCOUNT_A).unwrap(),
        t.get_value(&ACCOUNT_A).unwrap()
    );

    let slot_two = account_storage_key(&ACCOUNT_A, 0x20);
    for (start, end) in [
        through_account_a(),
        (Some(&ACCOUNT_B[..]), Some(&ACCOUNT_B[..])),
        (Some(&slot_two[..]), None),
        (None, Some(&ACCOUNT_A[..])),
    ] {
        let proof = t.range_proof(start, end, None).unwrap();
        let verified = VerifiedRangeProof::verify(
            Arc::new(proof),
            root(&t),
            start,
            end,
            EthHash::ALGORITHM,
            None,
        )
        .unwrap();
        let holes = find_holes_after_range_proof::<EthHash, _>(&verified, &legacy).unwrap();
        assert!(
            holes.iter().all(|h| matches!(h, Hole::Synced(_))),
            "{start:02x?}..{end:02x?}: {holes:?}"
        );
        // The legacy store must walk exactly as the post-hfix store does, so
        // "all Synced" cannot pass by emitting nothing.
        let expected =
            find_holes_after_range_proof::<EthHash, _>(&verified, l.nodestore()).unwrap();
        assert_eq!(holes, expected, "{start:02x?}..{end:02x?}");
    }
    let (start, end) = through_account_a();
    let proof = t.range_proof(start, end, None).unwrap();
    let verified =
        VerifiedRangeProof::verify(proof, root(&t), start, end, EthHash::ALGORITHM, None).unwrap();
    assert!(
        !find_holes_after_range_proof::<EthHash, _>(&verified, &legacy)
            .unwrap()
            .is_empty(),
        "a boundary through account A leaves synced storage slots"
    );
}

#[test]
fn malformed_account_value_is_compared_as_is() {
    // A 32-byte key whose value is not account RLP cannot be masked, so it is
    // compared byte for byte: equal values are silent and a difference is a
    // PointFix, the same as any other point.
    const JUNK: [u8; 32] = [0x33; 32];
    let slot = account_storage_key(&JUNK, 0x10);
    let target = map(&[(&JUNK, b"junk"), (&slot, b"slot")]);
    let t = trie::<EthHash>(&target);
    let bounds = (Some(&slot[..]), Some(&slot[..]));

    let l = trie::<EthHash>(&target);
    let (verified, holes) = range_holes(&t, &l, bounds.0, bounds.1, None);
    assert!(point_labels(&holes).is_empty(), "{holes:?}");
    assert_matches_truth::<EthHash>(&target, &target, &applied_range(&verified), &holes);

    let mut local = target.clone();
    local.insert(JUNK.to_vec(), b"other".to_vec());
    let l = trie::<EthHash>(&local);
    let (verified, holes) = range_holes(&t, &l, bounds.0, bounds.1, None);
    assert_eq!(point_labels(&holes).len(), 1, "{holes:?}");
    assert!(
        matches!(point_labels(&holes)[0], Hole::PointFix { key, value } if **key == JUNK && &**value == b"junk"),
        "{holes:?}"
    );
    assert_matches_truth::<EthHash>(&target, &local, &applied_range(&verified), &holes);
}

// The lone-storage-child fold. Live hashing stores an account's single
// storage child folded as a standalone storage-trie root, so a child's stored
// hash depends on its sibling count; the walk re-hashes the local child under
// the target's convention when the counts straddle one.

/// Account A holding the given storage slots, each valued by its slot byte.
fn account_with_slots(slots: &[u8]) -> Map {
    let mut m = Map::new();
    m.insert(ACCOUNT_A.to_vec(), account(1, 0xAA));
    for &slot in slots {
        m.insert(
            account_storage_key(&ACCOUNT_A, slot).into_vec(),
            rlp_encode_storage(&[slot; 32]),
        );
    }
    m
}

/// Apply every label's remedy to `local` the way a caller would: delete the
/// keys under deletion labels, write the carried value for a point fix, and
/// replace everything a fetch label covers with the target's content. Only
/// the complement of the applied range is modelled; the key-value pairs the
/// proof itself carried are not written.
fn apply_remedies(target: &Map, local: &Map, holes: &[Hole]) -> Map {
    let mut out = local.clone();
    for hole in holes {
        match hole {
            Hole::Surplus(_) | Hole::PointSurplus { .. } => {
                out.retain(|k, _| !covers(hole, k));
            }
            Hole::Missing(_) | Hole::Stale(_) | Hole::PointStale { .. } => {
                out.retain(|k, _| !covers(hole, k));
                for (k, v) in target.iter().filter(|(k, _)| covers(hole, k)) {
                    out.insert(k.clone(), v.clone());
                }
            }
            Hole::PointFix { key, value } => {
                out.insert(key.to_vec(), value.to_vec());
            }
            Hole::Synced(_) => {}
        }
    }
    out
}

/// Walk `local` against a proof from `target` over `[start, end]`, check the
/// labels, then apply them and assert the second walk finds only agreement.
fn assert_converges(
    target: &Map,
    local: &Map,
    start: Option<&[u8]>,
    end: Option<&[u8]>,
) -> Vec<Hole> {
    let (t, l) = (trie::<EthHash>(target), trie::<EthHash>(local));
    let (verified, holes) = range_holes(&t, &l, start, end, None);
    assert_matches_truth::<EthHash>(target, local, &applied_range(&verified), &holes);

    let repaired = apply_remedies(target, local, &holes);
    let r = trie::<EthHash>(&repaired);
    let (verified, again) = range_holes(&t, &r, start, end, None);
    assert!(
        !again.is_empty(),
        "the second walk still labels the complement"
    );
    assert!(
        again.iter().all(|h| matches!(h, Hole::Synced(_))),
        "after remedies: {again:?}"
    );
    assert_matches_truth::<EthHash>(target, &repaired, &applied_range(&verified), &again);
    holes
}

#[test]
fn fold_target_one_child_local_two() {
    // The target stores slot 1 folded, the local trie stores it unfolded
    // beside a surplus slot 2. Identical content under slot 1 is Synced, not
    // Stale; slot 2 is the only remedy.
    let target = account_with_slots(&[0x10]);
    let local = account_with_slots(&[0x10, 0x20]);
    let holes = assert_converges(&target, &local, Some(&ACCOUNT_A), Some(&ACCOUNT_A));

    let labels = spans(&holes);
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Synced" && p.ends_with(&[0x1])),
        "{holes:?}"
    );
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Surplus" && p.ends_with(&[0x2])),
        "{holes:?}"
    );
    assert!(!labels.iter().any(|(l, _)| *l == "Stale"), "{holes:?}");
}

#[test]
fn fold_target_two_children_local_one() {
    // The reverse: the local trie stores its single slot folded while the
    // target stores both slots unfolded. Slot 1 is Synced and slot 2 Missing.
    let target = account_with_slots(&[0x10, 0x20]);
    let local = account_with_slots(&[0x10]);
    let holes = assert_converges(&target, &local, Some(&ACCOUNT_A), Some(&ACCOUNT_A));

    let labels = spans(&holes);
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Synced" && p.ends_with(&[0x1])),
        "{holes:?}"
    );
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Missing" && p.ends_with(&[0x2])),
        "{holes:?}"
    );
    assert!(!labels.iter().any(|(l, _)| *l == "Stale"), "{holes:?}");
}

#[test]
fn fold_sibling_through_storage_boundary() {
    // The boundary runs through slot 1 of a two-slot target account; the
    // local account holds only slot 2, folded. The sibling comparison at
    // slot 2 crosses the fold and still reads Synced.
    let target = account_with_slots(&[0x10, 0x20]);
    let local = account_with_slots(&[0x20]);
    let slot_one = account_storage_key(&ACCOUNT_A, 0x10);
    let holes = assert_converges(&target, &local, Some(&slot_one), Some(&slot_one));

    let labels = spans(&holes);
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Synced" && p.ends_with(&[0x2])),
        "{holes:?}"
    );
    assert!(!labels.iter().any(|(l, _)| *l == "Stale"), "{holes:?}");
}

#[test]
fn fold_sibling_below_the_boundary() {
    // The same crossing on the Below side: the boundary runs through slot 2
    // of a two-slot target account and the local account holds only slot 1,
    // folded. The start proof's sibling comparison at slot 1 re-hashes under
    // the target's convention and reads Synced.
    let target = account_with_slots(&[0x10, 0x20]);
    let local = account_with_slots(&[0x10]);
    let slot_two = account_storage_key(&ACCOUNT_A, 0x20);
    let holes = assert_converges(&target, &local, Some(&slot_two), Some(&slot_two));

    let labels = spans(&holes);
    assert!(
        labels
            .iter()
            .any(|(l, p)| *l == "Synced" && p.ends_with(&[0x1])),
        "{holes:?}"
    );
    assert!(!labels.iter().any(|(l, _)| *l == "Stale"), "{holes:?}");
}

#[test]
fn fold_same_count_needs_no_normalization() {
    // One slot on both sides: both stored hashes are folded and compare
    // directly. Pins that the normalization stays out of the way.
    let target = account_with_slots(&[0x10]);
    let holes = assert_converges(&target, &target, Some(&ACCOUNT_A), Some(&ACCOUNT_A));
    assert!(
        holes.iter().all(|h| matches!(h, Hole::Synced(_))),
        "{holes:?}"
    );
}
