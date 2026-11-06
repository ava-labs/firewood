// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use test_case::test_case;

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
