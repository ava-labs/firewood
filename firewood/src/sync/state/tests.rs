// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "tests use plain arithmetic on small constants"
)]

use firewood_storage::SeededRng;
use test_case::test_case;

use super::complete::Completed;
use super::complete::ProofExtent;
use super::*;

fn ep(bytes: &[u8]) -> Endpoint {
    Endpoint::Key(bytes.into())
}

fn th(n: u8) -> TrieHash {
    TrieHash::from([n; 32])
}

fn limit(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

fn state(task_limit: usize) -> SyncState {
    SyncState::new(th(7), limit(task_limit))
}

fn take_work(state: &mut SyncState) -> (WorkId, Range<Endpoint>, bool) {
    match state.next_work() {
        NextWork::Work {
            id,
            range,
            wakeup_neighbor,
        } => (id, range, wakeup_neighbor),
        other => panic!("expected Work, got {other:?}"),
    }
}

fn take_continue(completed: Completed) -> (WorkId, Range<Endpoint>, bool) {
    match completed {
        Completed::Continue {
            id,
            range,
            wakeup_neighbor,
        } => (id, range, wakeup_neighbor),
        Completed::Exhausted => panic!("expected Continue, got Exhausted"),
    }
}

fn coverage_entries(state: &SyncState) -> Vec<(Range<Endpoint>, TrieHash)> {
    state
        .coverage
        .iter()
        .map(|(range, hash)| (range.clone(), hash.clone()))
        .collect()
}

/// `(coverage entries, requested ranges, outstanding item count)`.
type Snapshot = (
    Vec<(Range<Endpoint>, TrieHash)>,
    Vec<Range<Endpoint>>,
    usize,
);

fn snapshot(state: &SyncState) -> Snapshot {
    (
        coverage_entries(state),
        state.requested.iter().cloned().collect(),
        state.work.len(),
    )
}

fn ranges_overlap(a: &Range<Endpoint>, b: &Range<Endpoint>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Structural invariants that must hold after every operation.
fn assert_invariants(state: &SyncState) {
    let whole = Endpoint::whole_keyspace();
    // 1. `requested` is disjoint from coverage.
    for range in state.requested.iter() {
        assert!(
            !state.coverage.overlaps(range),
            "requested {range:?} overlaps coverage"
        );
    }
    // 2. `requested` equals the union of work ranges; all non-empty.
    let mut union = RangeSet::new();
    for item in state.work.values() {
        assert!(
            item.range.start < item.range.end,
            "empty in-flight range {:?}",
            item.range
        );
        union.insert(item.range.clone());
    }
    assert!(
        union.iter().eq(state.requested.iter()),
        "requested set diverged from the work map"
    );
    // 3. Outstanding item count is capped.
    assert!(
        state.work.len() <= state.task_limit.get(),
        "task limit exceeded"
    );
    // 4. Seed phase: everything occupied lies left of the seed cursor.
    if state.seed_cursor < Endpoint::Max {
        for range in state.requested.iter() {
            assert!(range.end <= state.seed_cursor, "request beyond seed cursor");
        }
        for (range, _) in state.coverage.iter() {
            assert!(
                range.end <= state.seed_cursor,
                "coverage beyond seed cursor"
            );
        }
    }
    // 5. Complete coverage implies nothing in flight.
    if state.coverage.gaps(&whole).next().is_none() {
        assert!(state.work.is_empty(), "complete coverage with live work");
        assert!(
            state.requested.is_empty(),
            "complete coverage with requests"
        );
    }
}

#[test_case(1, Endpoint::Max; "one slice is the whole keyspace")]
#[test_case(5, ep(&[0x33]); "prime count")]
#[test_case(16, ep(&[0x10]); "nibble boundaries")]
fn seed_slices_tile_the_keyspace(task_limit: usize, first_end: Endpoint) {
    let mut state = state(task_limit);
    let mut previous_end = ep(&[]);
    for i in 1..=task_limit {
        let (_, range, wakeup) = take_work(&mut state);
        assert_eq!(range.start, previous_end, "slice {i} must abut");
        assert!(range.start < range.end, "slice {i} must be non-empty");
        if i == 1 {
            assert_eq!(range.end, first_end, "1/task_limit of the keyspace");
        }
        assert_eq!(wakeup, i < task_limit, "cold space remains after slice {i}");
        previous_end = range.end;
        assert_invariants(&state);
    }
    assert_eq!(previous_end, Endpoint::Max);
    assert!(state.cold_holes().is_empty(), "no gap and no overlap");
}

#[test]
fn full_extent_coalesces_adjacent_coverage() {
    let mut state = state(2);
    let (id_a, range_a, _) = take_work(&mut state);
    let (id_b, range_b, _) = take_work(&mut state);
    assert_eq!(range_a.end, range_b.start, "seed slices abut");

    assert!(matches!(
        state.complete(id_a, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    assert!(matches!(
        state.complete(id_b, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    assert_eq!(
        coverage_entries(&state),
        vec![(Endpoint::whole_keyspace(), th(7))],
        "abutting equal-hash coverage coalesces to one entry"
    );
    assert!(matches!(state.next_work(), NextWork::CoverageComplete));
    assert_invariants(&state);
}

#[test]
fn truncated_records_succ_inclusive_coverage() {
    let mut state = state(1);
    let (id0, range0, _) = take_work(&mut state);
    assert_eq!(range0, Endpoint::whole_keyspace());

    let completed = state
        .complete(
            id0,
            ProofExtent::Truncated {
                last_key: Box::from(&[0x40][..]),
                cut: None,
            },
        )
        .unwrap();
    let (id1, range1, wakeup) = take_continue(completed);
    assert_ne!(id1, id0, "continuation gets a fresh id");
    assert!(!wakeup);
    assert_eq!(range1, ep(&[0x40, 0x00])..Endpoint::Max);
    assert_eq!(
        coverage_entries(&state),
        vec![(ep(&[])..ep(&[0x40, 0x00]), th(7))],
        "coverage is inclusive of the last proven key"
    );
    assert!(state.work_item(id0).is_none(), "old id is retired");
    assert_invariants(&state);

    assert!(matches!(
        state.complete(id1, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    assert_eq!(
        coverage_entries(&state),
        vec![(Endpoint::whole_keyspace(), th(7))],
        "the continuation's coverage coalesces with the truncated part"
    );
    assert!(matches!(state.next_work(), NextWork::CoverageComplete));
    assert_invariants(&state);
}

#[test]
fn truncation_on_right_edge_is_exhausted() {
    // Seed slicing never produces a right edge of the form `L ++ 0x00`,
    // so reserve the region directly.
    let mut state = state(1);
    state.seed_cursor = Endpoint::Max;
    let range = ep(&[0x10])..ep(&[0x10, 0x40, 0x00]);
    let id = state.reserve(range.clone());

    let completed = state
        .complete(
            id,
            ProofExtent::Truncated {
                last_key: Box::from(&[0x10, 0x40][..]),
                cut: None,
            },
        )
        .unwrap();
    // succ(L) == range.end: nothing remains to reserve.
    assert_eq!(completed, Completed::Exhausted);
    assert_eq!(coverage_entries(&state), vec![(range, th(7))]);
    assert!(state.requested.is_empty());
    assert!(state.work.is_empty());
    assert_invariants(&state);
}

/// Truncate the left seed slice `[[], [0x80])` at `[0x10]`, with the right
/// slice still in flight (`idle == false`) or already done.
#[test_case(true, Some(ep(&[0x40])), ep(&[0x40]); "idle worker: shed at the cut")]
#[test_case(false, Some(ep(&[0x40])), ep(&[0x80]); "no idle worker: keep the remainder")]
#[test_case(true, None, ep(&[0x80]); "no cut")]
#[test_case(true, Some(ep(&[0x10])), ep(&[0x80]); "cut below the resume key")]
#[test_case(true, Some(ep(&[0x10, 0x00])), ep(&[0x80]); "cut at the resume key")]
#[test_case(true, Some(ep(&[0x80])), ep(&[0x80]); "cut at the end")]
#[test_case(true, Some(Endpoint::Max), ep(&[0x80]); "cut past the end")]
fn truncation_sheds_only_at_a_usable_cut_for_an_idle_worker(
    idle: bool,
    cut: Option<Endpoint>,
    kept_end: Endpoint,
) {
    let mut state = state(2);
    let (id, _, _) = take_work(&mut state);
    let (other, _, _) = take_work(&mut state);
    if idle {
        assert!(matches!(
            state.complete(other, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
    }
    let (_, range, wakeup) = take_continue(
        state
            .complete(
                id,
                ProofExtent::Truncated {
                    last_key: Box::from(&[0x10][..]),
                    cut,
                },
            )
            .unwrap(),
    );
    let end = ep(&[0x80]);
    let shed = kept_end != end;
    assert_eq!(range, ep(&[0x10, 0x00])..kept_end.clone());
    assert_eq!(wakeup, shed, "a shed must wake a parked helper");
    let expected_cold = if shed { vec![kept_end..end] } else { vec![] };
    assert_eq!(state.cold_holes(), expected_cold);
    assert_invariants(&state);
}

#[test]
fn refeed_hands_out_the_widest_cold_hole_whole() {
    // Steady-state cold space arises only from shedding; building the
    // maps directly keeps the scenario small.
    let mut state = state(4);
    state.seed_cursor = Endpoint::Max;
    state.coverage.insert(ep(&[0x10])..ep(&[0x80]), th(7));
    state.coverage.insert(ep(&[0xc0])..Endpoint::Max, th(7));
    // Holes: [[], [0x10]) (width 0x10) and [[0x80], [0xc0]) (width 0x40).
    let (_, range, wakeup) = take_work(&mut state);
    assert_eq!(range, ep(&[0x80])..ep(&[0xc0]));
    assert!(wakeup, "the smaller hole stays cold");
    assert_eq!(state.cold_holes(), vec![ep(&[])..ep(&[0x10])]);
    assert_invariants(&state);
}

#[test]
fn cold_holes_is_gaps_minus_requested() {
    let mut state = state(4);
    state.seed_cursor = Endpoint::Max;
    state.coverage.insert(ep(&[0x20])..ep(&[0x40]), th(7));
    state.coverage.insert(ep(&[0x80])..ep(&[0xa0]), th(7));
    // An in-flight island strictly inside the middle coverage gap.
    state.reserve(ep(&[0x50])..ep(&[0x60]));
    assert_eq!(
        state.cold_holes(),
        vec![
            ep(&[])..ep(&[0x20]),
            ep(&[0x40])..ep(&[0x50]),
            ep(&[0x60])..ep(&[0x80]),
            ep(&[0xa0])..Endpoint::Max,
        ],
        "cold space is exactly gaps(coverage) minus requested"
    );
    assert_invariants(&state);
}

#[test]
fn wait_when_everything_in_flight() {
    let mut state = state(2);
    take_work(&mut state);
    let (id_b, _, _) = take_work(&mut state);
    // Capacity full.
    assert!(matches!(state.next_work(), NextWork::Wait));
    // Capacity free, but the only coverage gap is fully in flight.
    assert!(matches!(
        state.complete(id_b, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    assert!(!state.coverage_complete());
    assert!(matches!(state.next_work(), NextWork::Wait));
    assert_invariants(&state);
}

#[test]
fn stale_id_rejected_without_mutation() {
    let mut state = state(2);
    let (id_a, _, _) = take_work(&mut state);
    take_work(&mut state);

    // Never-issued id.
    let before = snapshot(&state);
    assert!(matches!(
        state.complete(WorkId::from(9999), ProofExtent::Full),
        Err(SyncError::StaleWorkId(id)) if id.as_u64() == 9999
    ));
    assert_eq!(snapshot(&state), before, "stale complete must not mutate");

    // Already-retired id.
    assert!(matches!(
        state.complete(id_a, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    let after_retire = snapshot(&state);
    assert!(matches!(
        state.complete(id_a, ProofExtent::Full),
        Err(SyncError::StaleWorkId(_))
    ));
    assert_eq!(snapshot(&state), after_retire);
    assert_invariants(&state);
}

#[test]
fn truncation_key_out_of_range_returns_range_to_cold() {
    let mut state = state(2);
    let (id_a, range_a, _) = take_work(&mut state);
    let (id_b, range_b, _) = take_work(&mut state);
    assert_eq!(range_a, ep(&[])..ep(&[0x80]));
    assert_eq!(range_b, ep(&[0x80])..Endpoint::Max);

    // The end is exclusive, so a key equal to it is out of range.
    assert!(matches!(
        state.complete(
            id_a,
            ProofExtent::Truncated {
                last_key: Box::from(&[0x80][..]),
                cut: None,
            }
        ),
        Err(SyncError::TruncationKeyOutOfRange(bad_id)) if bad_id == id_a
    ));
    // The empty key is below the second slice's start.
    assert!(matches!(
        state.complete(
            id_b,
            ProofExtent::Truncated {
                last_key: Box::default(),
                cut: None,
            }
        ),
        Err(SyncError::TruncationKeyOutOfRange(bad_id)) if bad_id == id_b
    ));

    assert!(state.work_item(id_a).is_none(), "the id is retired");
    assert!(state.work_item(id_b).is_none(), "the id is retired");
    assert!(state.coverage.is_empty(), "nothing is covered");
    assert_eq!(state.cold_holes(), vec![Endpoint::whole_keyspace()]);
    assert_invariants(&state);
}

#[test]
fn invalid_proof_reservation_survives() {
    // The whole InvalidProof story at the pure layer: look the item up,
    // do NOT call complete. The reservation must survive untouched.
    let mut state = state(2);
    let (id_a, range_a, _) = take_work(&mut state);

    let (looked_range, looked_hash) = state.work_item(id_a).unwrap();
    assert_eq!(looked_range, range_a);
    assert_eq!(looked_hash, th(7));

    // The reserved region is never handed out again...
    let (id_b, range_b, _) = take_work(&mut state);
    assert_ne!(id_b, id_a);
    assert!(!ranges_overlap(&range_a, &range_b));
    assert!(matches!(state.next_work(), NextWork::Wait));

    // ...the id stays valid, and a later complete succeeds.
    assert!(state.work_item(id_a).is_some());
    assert!(matches!(
        state.complete(id_a, ProofExtent::Full),
        Ok(Completed::Exhausted)
    ));
    assert_invariants(&state);
}

fn assert_no_overlap(active: &[(WorkId, Range<Endpoint>)], range: &Range<Endpoint>) {
    for (id, reserved) in active {
        assert!(
            !ranges_overlap(reserved, range),
            "handed-out {range:?} overlaps live {id:?} ({reserved:?})"
        );
    }
}

/// A random truncation key within the item's request bounds:
/// `range.start <= Key(L) < range.end`.
fn draw_truncation_key(rng: &SeededRng, range: &Range<Endpoint>) -> Box<[u8]> {
    let Endpoint::Key(base) = &range.start else {
        panic!("work ranges never start at Max");
    };
    let mut candidates: Vec<Box<[u8]>> = vec![base.clone()];
    for extra in 1..=2usize {
        let candidate: Box<[u8]> = base
            .iter()
            .copied()
            .chain((0..extra).map(|_| rng.random()))
            .collect();
        if Endpoint::Key(candidate.clone()) < range.end {
            candidates.push(candidate);
        }
    }
    let pick = rng.random_range(0..candidates.len());
    candidates.swap_remove(pick)
}

fn run_random_ops(rng: &SeededRng, ops: usize) {
    let task_limit = rng.random_range(1..=8usize);
    let mut state = SyncState::new(th(7), limit(task_limit));
    // Live reservations, mirrored to check that handouts never overlap.
    let mut active: Vec<(WorkId, Range<Endpoint>)> = Vec::new();

    for _ in 0..ops {
        if active.is_empty() || rng.random_range(0..2u8) == 0 {
            match state.next_work() {
                NextWork::Work { id, range, .. } => {
                    assert_no_overlap(&active, &range);
                    active.push((id, range));
                }
                NextWork::Wait => {}
                NextWork::CoverageComplete => break,
            }
        } else {
            let pick = rng.random_range(0..active.len());
            let (id, range) = active.swap_remove(pick);
            let extent = if rng.random_range(0..3u8) == 0 {
                ProofExtent::Full
            } else {
                ProofExtent::Truncated {
                    last_key: draw_truncation_key(rng, &range),
                    cut: (rng.random_range(0..2u8) == 0)
                        .then(|| Endpoint::Key(draw_truncation_key(rng, &range))),
                }
            };
            match state.complete(id, extent).unwrap() {
                Completed::Exhausted => {}
                Completed::Continue { id, range, .. } => {
                    assert_no_overlap(&active, &range);
                    active.push((id, range));
                }
            }
        }
        assert_invariants(&state);
        assert_eq!(active.len(), state.work.len(), "mirror diverged");
    }

    // Drive to completion with Full completions; refeed hands out whole
    // holes, so each handout retires one.
    let mut guard = 0usize;
    loop {
        guard += 1;
        assert!(guard < 200_000, "sync failed to converge");
        if let Some((id, _)) = active.pop() {
            assert!(matches!(
                state.complete(id, ProofExtent::Full),
                Ok(Completed::Exhausted)
            ));
            assert_invariants(&state);
            continue;
        }
        match state.next_work() {
            NextWork::Work { id, range, .. } => {
                assert_no_overlap(&active, &range);
                active.push((id, range));
            }
            NextWork::Wait => panic!("Wait with nothing outstanding"),
            NextWork::CoverageComplete => break,
        }
        assert_invariants(&state);
    }
    assert!(state.coverage_complete());
    assert_invariants(&state);
}

#[test]
fn random_ops_uphold_invariants() {
    let rng = SeededRng::from_env_or_random();
    for _ in 0..10 {
        run_random_ops(&rng, 300);
    }
}
