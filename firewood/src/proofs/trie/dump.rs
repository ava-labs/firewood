// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::borrow::Cow;

use firewood_storage::{BranchNode, ValueDigest};

use crate::proofs::{
    path::{Nibbles, PathGuard, PathNibble},
    trie::{TrieNode, iter::Child},
};

pub(super) struct DumpTrie<T>(T);

impl<'a, T: TrieNode<'a>> DumpTrie<T> {
    pub const fn new(node: T) -> Self {
        Self(node)
    }
}

impl<'a, T: TrieNode<'a>> std::fmt::Debug for DumpTrie<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        dump_trie(f, self.0)
    }
}

impl<'a, T: TrieNode<'a>> std::fmt::Display for DumpTrie<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        dump_trie(f, self.0)
    }
}

/// Dumps a trie to the given writer, for debugging purposes.
///
/// The output format is not stable and is only for human readable consumption.
///
/// This will walk the trie in pre-order and print each node's partial path, value
/// digest (if any), previously computed hash (if any), and the children recursively.
///
/// Children in the trie may either be by reference (hash only) or by value (full
/// node) and full nodes will be dumped recursively.
fn dump_trie<'a, W, T>(w: &mut W, node: T) -> std::fmt::Result
where
    W: std::fmt::Write + ?Sized,
    T: TrieNode<'a>,
{
    fn inner_dump<'a, W, T>(
        w: &mut W,
        node: T,
        depth: usize,
        mut path: PathGuard<'_>,
    ) -> std::fmt::Result
    where
        W: std::fmt::Write + ?Sized,
        T: TrieNode<'a>,
    {
        const INDENT: usize = 2;

        #[cfg(not(feature = "branch_factor_256"))]
        const NIBBLE_WIDTH: usize = 1;
        #[cfg(feature = "branch_factor_256")]
        const NIBBLE_WIDTH: usize = 2;

        // empty string, the width is used to pad with `depth` spaces
        const WS: &str = "";

        path.extend(node.partial_path().nibbles_iter());
        writeln!(w, "{WS:>depth$}Path: {}", path.display())?;

        if let Some(vd) = node.value_digest() {
            match vd {
                ValueDigest::Value(value) => {
                    let value = str::from_utf8(value)
                        .map_or_else(|_| hex::encode(value).into(), Cow::Borrowed);
                    writeln!(w, "{WS:>depth$}- Value: {value}")?;
                }
                #[cfg(not(feature = "ethhash"))]
                ValueDigest::Hash(digest) => {
                    writeln!(w, "{WS:>depth$}- Value Digest: {digest}")?;
                }
            }
        }

        if let Some(ch) = node.computed_hash() {
            writeln!(w, "{WS:>depth$}- Computed Hash: {ch:.64}")?;
        }

        let mut n_children = 0_usize;
        for (idx, slot) in node.children().into_iter().enumerate() {
            let Some(slot) = slot else {
                continue;
            };

            n_children = n_children.wrapping_add(1);

            let path = path.fork_push(PathNibble(idx as u8));
            write!(w, "{WS:>depth$}- Child {idx:0NIBBLE_WIDTH$x} ")?;
            match slot {
                Child::Unhashed(node) => {
                    writeln!(w, "(unhashed):")?;
                    inner_dump(w, node, depth.wrapping_add(INDENT), path)?;
                }
                Child::Hashed(hash, node) => {
                    writeln!(w, "(hashed: {hash}):")?;
                    inner_dump(w, node, depth.wrapping_add(INDENT), path)?;
                }
                Child::Remote(hash) => {
                    writeln!(w, "(remote: {hash})")?;
                }
            }
        }

        if n_children == 0 {
            writeln!(w, "{WS:>depth$}- <no children>")?;
        } else if n_children == BranchNode::MAX_CHILDREN {
            writeln!(w, "{WS:>depth$}- <all {n_children} children present>")?;
        }

        Ok(())
    }

    inner_dump(w, node, 0, PathGuard::new(&mut Vec::new()))
}
