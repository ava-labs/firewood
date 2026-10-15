// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for post-merge hole detection: the walk over a verified proof's
//! boundary proofs that labels the key space outside the proven range.
//!
//! Most tests build a target trie `M` and a local trie `L` from flat maps,
//! take a range proof from `M`, verify it, run the walk against
//! `L`, and check every label against the maps directly. The checker also
//! asserts coverage: every key of either map outside the applied range lies
//! in exactly one label, and any label reaching into the applied range is a
//! deletion.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood_macros::hash_mode;
use firewood_storage::Committed;
use firewood_storage::EthHash;
use firewood_storage::HashMode;
use firewood_storage::HashedNodeReader;
use firewood_storage::MemStore;
use firewood_storage::MerkleDbHash;
use firewood_storage::NodeStore;
use firewood_storage::PathComponent;
use firewood_storage::TriePathFromPackedBytes;

use super::init_merkle;
use crate::Hole;
use crate::VerifiedRangeProof;
use crate::api::HashKey;
use crate::merkle::Merkle;
use crate::merkle::holes::find_holes_after_range_proof;

type Map = BTreeMap<Vec<u8>, Vec<u8>>;
type Trie<H> = Merkle<NodeStore<Committed, MemStore, H>>;

fn map(pairs: &[(&[u8], &[u8])]) -> Map {
    pairs
        .iter()
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}

fn trie<H: HashMode>(m: &Map) -> Trie<H> {
    init_merkle::<H, _, _, _>(m.iter().map(|(k, v)| (k.clone(), v.clone())))
}

fn root<H: HashMode>(t: &Trie<H>) -> HashKey {
    HashedNodeReader::root_hash(t.nodestore()).unwrap()
}

fn nibbles(key: &[u8]) -> Vec<PathComponent> {
    Vec::<PathComponent>::path_from_packed_bytes(key)
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

/// Check every label against the flat maps and check coverage of the
/// complement of the applied range.
fn assert_matches_truth(
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
                    (false, false) if t == l => "Synced",
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
                assert_eq!(
                    target.get(&**key).map(Vec::as_slice),
                    Some(&**value),
                    "{key:02x?}"
                );
                assert_ne!(
                    local.get(&**key).map(Vec::as_slice),
                    Some(&**value),
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
            let both_equal = target.get(key) == local.get(key) && target.get(key).is_some();
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
    assert_matches_truth(&target, &local, &applied_range(&verified), &holes);
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
    assert_matches_truth(&target, &local, &applied_range(&verified), &holes);
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
    assert_matches_truth(&target, &target, &applied_range(&verified), &holes);

    // Re-request one Synced span: a walk that also covers the start side
    // finds nothing to fetch.
    let (verified, again) = range_holes(&t, &l, Some(&[0xB0]), Some(&[0xBF]), None);
    assert_eq!(fetch_labels(&again), 0);
    assert_matches_truth(&target, &target, &applied_range(&verified), &again);
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
    assert_matches_truth(&target, &local, &applied_range(&verified), &holes);
}
