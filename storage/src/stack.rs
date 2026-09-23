// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Stack headroom for walks whose depth follows key length.
//!
//! Every function that recurses once per trie level passes its recursive
//! call through [`ensure_stack`]. Two recursions are compiler-generated and
//! are written out by hand for the same reason: `Node`'s `Clone` and
//! `BranchNode`'s `Drop`. A new `Child` variant that can own a `Node` must be
//! added to the scan in `BranchNode::drop`.

/// Remaining stack below which a walk continues on a heap segment. It must
/// exceed the stack one level uses between two checks. The largest such level
/// is a round trip through the proof-side root-hash walk, under 2 KiB, so this
/// leaves a wide margin. rustc uses the same figure for its own recursion.
const RED_ZONE: usize = 128 * 1024;
/// Size of each heap segment a walk grows onto. A segment holds about a
/// thousand levels, so the allocation is amortised over that many.
const SEGMENT: usize = 1024 * 1024;

/// Runs `f`, first moving to a fresh heap-allocated stack segment if the
/// current stack is within the red zone. Depth then costs heap memory rather
/// than call-stack space, so a walk over peer-controlled depth cannot abort
/// the process.
pub fn ensure_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, SEGMENT, f)
}
