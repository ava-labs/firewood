// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Hash Module
//!
//! This module contains all node hashing functionality for the nodestore, including
//! specialized support for Ethereum-compatible hash processing.

use crate::U4;
use crate::hashednode::hash_node;
use crate::linear::FileIoError;
use crate::logger::trace;
use crate::node::{BranchNode, Node};
use crate::rlp::{EMPTY_TRIE_ROOT, RlpItem, encode_list, replace_list_field};
use crate::{
    Child, Children, HashMode, HashType, MaybePersistedNode, NodeStore, Path, ReadableStorage,
    SharedNode, TrieHash,
};
use crate::{HashableShunt, JoinedPath, PathComponent, SplitPath, ValueDigest};
use sha3::{Digest, Keccak256};
use smallvec::SmallVec;

use super::NodeReader;


/// Hashes a finished frame. `path` must already be truncated to the node's own
/// prefix.
fn finish_hash_frame<H: HashMode>(
    mut frame: HashFrame,
    path: &Path,
) -> (MaybePersistedNode, HashType) {
    // For account-depth nodes (branch or leaf), persist the computed storageRoot
    // into the node's RLP-encoded value. Ethereum scheme only.
    if H::ALGORITHM.is_ethereum() {
        update_account_storage_root(&mut frame.node, path);
    }

    let hash = match frame.fake_root_extra_nibble {
        Some(nibble) if H::ALGORITHM.is_ethereum() => {
            hash_node_as_storage_trie_root_for_node::<H>(path.as_components(), nibble, &frame.node)
        }
        _ => hash_node::<H>(&frame.node, path),
    };

    (SharedNode::new(frame.node).into(), hash)
}

/// One node part-way through hashing.
///
/// `cursor` is the next child slot to examine and `pending` is the slot waiting
/// on the hash of the frame above it, so a frame can be suspended and resumed.
/// `prefix_len` is the length of the shared path buffer that belongs to this
/// node, which the buffer is truncated back to before the node is hashed.
struct HashFrame {
    node: Node,
    prefix_len: usize,
    /// Whether the per-branch preparation has run. Deferring it to the loop
    /// keeps it in one place rather than at each push site.
    prepared: bool,
    cursor: u8,
    pending: Option<PathComponent>,
    /// The slot this branch will fold as its storage-trie root, told to the child
    /// when the walk descends into it. Only ever set under the Ethereum scheme.
    make_fake_root: Option<PathComponent>,
    /// What this node's parent told it, if the parent folds this node.
    fake_root_extra_nibble: Option<PathComponent>,
}

impl HashFrame {
    const fn new(
        node: Node,
        prefix_len: usize,
        fake_root_extra_nibble: Option<PathComponent>,
    ) -> Self {
        Self {
            node,
            prefix_len,
            prepared: false,
            cursor: 0,
            pending: None,
            make_fake_root: None,
            fake_root_extra_nibble,
        }
    }
}

/// Classified children for ethereum hash processing
pub(super) struct ClassifiedChildren<'a> {
    pub(super) unhashed: Vec<PathComponent>,
    pub(super) hashed: Vec<(PathComponent, (MaybePersistedNode, &'a mut HashType))>,
}

impl<T, S: ReadableStorage, H: HashMode> NodeStore<T, S, H>
where
    NodeStore<T, S, H>: NodeReader,
{
    /// Helper function to classify children for ethereum hash processing
    /// We have some special cases based on the number of children
    /// and whether they are hashed or unhashed, so we need to classify them.
    pub(super) fn ethhash_classify_children<'a>(
        &self,
        children: &'a mut Children<Option<Child>>,
    ) -> ClassifiedChildren<'a> {
        children.into_iter().fold(
            ClassifiedChildren {
                unhashed: Vec::new(),
                hashed: Vec::new(),
            },
            |mut acc, (idx, child)| {
                match child {
                    None => {}
                    Some(Child::AddressWithHash(a, h)) => {
                        // Convert address to MaybePersistedNode
                        let maybe_persisted_node = MaybePersistedNode::from(*a);
                        acc.hashed.push((idx, (maybe_persisted_node, h)));
                    }
                    Some(Child::Node(_)) => acc.unhashed.push(idx),
                    Some(Child::MaybePersisted(maybe_persisted, h)) => {
                        // For MaybePersisted, it's important to remember that we've already hashed it
                        acc.hashed.push((idx, (maybe_persisted.clone(), h)));
                    }
                }
                acc
            },
        )
    }

    /// Hashes the given `node` and the subtree rooted at it. The `root_path` should be empty
    /// if this is called from the root, or it should include the partial path if this is called
    /// on a subtrie. Returns the hashed node and its hash.
    ///
    /// The walk is iterative. Trie depth follows key length, which is
    /// attacker-controlled during proof verification, so recursing per level would
    /// be a stack-exhaustion sink. Frames live on the heap, and one path buffer is
    /// extended when descending and truncated back to a frame's own prefix when
    /// that frame is hashed.
    ///
    /// # Errors
    ///
    /// Can return a `FileIoError` if it is unable to read a node that it is hashing.
    ///
    /// # Panics
    ///
    /// Panics if the frame stack is empty while the walk is still running, which
    /// cannot happen: a frame is only popped when it is finished, and the walk
    /// returns as soon as popping empties the stack.
    pub fn hash_helper(
        &self,
        node: Node,
        root_path: Path,
    ) -> Result<(MaybePersistedNode, HashType), FileIoError> {
        let mut path = root_path;
        let prefix_len = path.0.len();
        // Only branches take frames, so this holds the branch depth of the current
        // path, which is around seven for a million uniformly distributed keys.
        // Inline capacity covers that without allocating, while staying small
        // enough that the inline buffer is not itself a per-call cost. Deeper tries
        // spill to the heap, which is the point: depth then costs memory rather
        // than stack.
        let mut stack: SmallVec<[HashFrame; 8]> = SmallVec::new();
        stack.push(HashFrame::new(node, prefix_len, None));
        let mut carried: Option<(MaybePersistedNode, HashType)> = None;

        loop {
            let frame = stack.last_mut().expect("the stack is never empty here");

            // Install the hash produced by the child frame that just finished, and
            // restore the path to this node's own prefix.
            if let Some(slot) = frame.pending.take() {
                let (child_node, child_hash) =
                    carried.take().expect("a finished frame yields a hash");
                if let Node::Branch(ref mut b) = frame.node {
                    b.children[slot] = Some(Child::MaybePersisted(child_node, child_hash));
                    trace!("child now {:?}", b.children[slot]);
                }
                path.0.truncate(frame.prefix_len);
            }

            if !frame.prepared {
                frame.prepared = true;
                self.prepare_account_branch(frame, &mut path)?;
            }

            // Find the next child that still needs hashing.
            let mut descend: Option<(PathComponent, Node)> = None;
            if let Node::Branch(ref mut b) = frame.node {
                while frame.cursor < BranchNode::MAX_CHILDREN as u8 {
                    let nibble = PathComponent(U4::new_masked(frame.cursor));
                    debug_assert!(frame.cursor < BranchNode::MAX_CHILDREN as u8);
                    frame.cursor = frame.cursor.wrapping_add(1);
                    // Empty slots are None and already-hashed variants are
                    // Some(None) here, so only Some(Some(node)) descends.
                    let Some(child_node) = b.children[nibble].as_mut().and_then(Child::as_mut_node)
                    else {
                        continue;
                    };
                    // Take the child out; a hashed variant replaces it on the way back.
                    descend = Some((nibble, std::mem::take(child_node)));
                    break;
                }
            }

            let Some((nibble, child_node)) = descend else {
                // Every child is hashed, so this node can be hashed.
                let frame = stack.pop().expect("the stack is never empty here");
                path.0.truncate(frame.prefix_len);
                let hashed = finish_hash_frame::<H>(frame, &path);
                if stack.is_empty() {
                    return Ok(hashed);
                }
                carried = Some(hashed);
                continue;
            };

            // Extend the path to the child. The account fold omits the nibble,
            // matching what live hashing produced for a lone storage child.
            if let Node::Branch(ref b) = frame.node {
                path.0.extend(b.partial_path.0.iter().copied());
            }
            let inherited = frame.make_fake_root;
            if !(H::ALGORITHM.is_ethereum() && inherited.is_some()) {
                path.0.push(nibble.as_u8());
            }

            let child_prefix_len = path.0.len();

            // A leaf has no children to wait on, so it is hashed here rather than
            // costing a frame. Leaves dominate a realistic trie, so this keeps most
            // nodes off the stack entirely.
            if matches!(child_node, Node::Leaf(_)) {
                let leaf = HashFrame::new(child_node, child_prefix_len, inherited);
                let (hashed, hash) = finish_hash_frame::<H>(leaf, &path);
                path.0.truncate(frame.prefix_len);
                if let Node::Branch(ref mut b) = frame.node {
                    b.children[nibble] = Some(Child::MaybePersisted(hashed, hash));
                    trace!("child now {:?}", b.children[nibble]);
                }
                continue;
            }

            frame.pending = Some(nibble);
            stack.push(HashFrame::new(child_node, child_prefix_len, inherited));
        }
    }

    /// Rehashes an account branch's children before its own children are walked.
    ///
    /// An account branch left with a single already-hashed child must have that
    /// child rehashed, because whether it folds as the account's storage-trie root
    /// depends on how many children the account has. A lone unhashed child is
    /// recorded so the walk folds it when it descends.
    ///
    /// Does nothing unless the scheme is Ethereum, which is the only scheme with an
    /// account-branch fold.
    fn prepare_account_branch(
        &self,
        frame: &mut HashFrame,
        path: &mut Path,
    ) -> Result<(), FileIoError> {
        if !H::ALGORITHM.is_ethereum() {
            return Ok(());
        }
        let Node::Branch(ref mut b) = frame.node else {
            return Ok(());
        };
        // Both lengths are nibble counts in a trie path, so their sum cannot
        // overflow on any platform firewood targets.
        if frame.prefix_len.wrapping_add(b.partial_path.0.len()) != 64 {
            return Ok(());
        }
        let ClassifiedChildren {
            unhashed,
            mut hashed,
        } = self.ethhash_classify_children(&mut b.children);
        trace!("hashed {hashed:?} unhashed {unhashed:?}");
        if let [(child_idx, (child_node, child_hash))] = &mut hashed[..] {
            let shared = child_node.as_shared_node(&self)?;
            let restore = path.0.len();
            path.0.extend(b.partial_path.0.iter().copied());
            let hash = if unhashed.is_empty() {
                hash_node_as_storage_trie_root_for_node::<H>(
                    path.as_components(),
                    *child_idx,
                    &shared,
                )
            } else {
                path.0.push(child_idx.as_u8());
                hash_node::<H>(&shared, path)
            };
            path.0.truncate(restore);
            **child_hash = hash;
        }
        frame.make_fake_root = if hashed.is_empty() && unhashed.len() == 1 {
            Some(*unhashed.last().expect("only one"))
        } else {
            None
        };
        Ok(())
    }

    /// Hash `node` at `path_prefix`, applying the Ethereum storage-trie-root
    /// fold for an account branch's single storage child when the scheme is
    /// Ethereum. Under the MerkleDB scheme `H::ALGORITHM.is_ethereum()` is a
    /// compile-time `false`, so this reduces to a plain [`hash_node`].
    pub(crate) fn compute_node_ethhash(
        node: &Node,
        path_prefix: &Path,
        have_peers: bool,
    ) -> HashType {
        let components = path_prefix.as_components();
        // 64 nibbles for the account prefix + 1 for this node's slot in the
        // account branch = 65. !have_peers means this is the only storage child.
        if H::ALGORITHM.is_ethereum() && components.len() == 65 && !have_peers {
            let (branch_nibble, account_prefix) = components
                .split_last()
                .expect("len == 65 implies non-empty");
            hash_node_as_storage_trie_root_for_node::<H>(account_prefix, *branch_nibble, node)
        } else {
            hash_node::<H>(node, path_prefix)
        }
    }
}

/// Convenience wrapper around [`hash_node_as_storage_trie_root_parts`] that
/// extracts that function's parts — the partial path, value digest, and child
/// hashes — from a [`Node`] directly: a branch contributes its value and its
/// children's hashes, a leaf contributes its value and no children.
pub fn hash_node_as_storage_trie_root_for_node<H: HashMode>(
    account_full_prefix: &[PathComponent],
    branch_nibble: PathComponent,
    node: &Node,
) -> HashType {
    let (value_digest, children) = match node {
        Node::Branch(b) => (
            b.value.as_deref().map(ValueDigest::Value),
            b.children_hashes(),
        ),
        Node::Leaf(l) => (Some(ValueDigest::Value(l.value.as_ref())), Children::new()),
    };
    hash_node_as_storage_trie_root_parts::<H, _, _>(
        account_full_prefix,
        branch_nibble,
        node.partial_path().as_components(),
        value_digest,
        children,
    )
}

/// Compute the root hash of an Ethereum storage trie from an account branch's
/// already-hashed children.
///
/// - 0 children → empty trie root (`keccak256(0x80)`)
/// - 1 child → that child's hash directly. The caller is responsible for having
///   produced that hash via `hash_node_as_storage_trie_root_parts` (which folds
///   the account's branch nibble into the child's partial path so the child hashes
///   as a standalone storage-trie root). Only relevant under the `ethhash` feature.
/// - ≥2 children → the 17-element branch RLP, hashed.
///
/// At account depth (64 nibbles) storage keys are 32 bytes, so every child
/// encoding exceeds 32 bytes and the inline-RLP variant of [`HashType`] cannot
/// occur. Without `ethhash`, `HashType` is `TrieHash` and the single-child case
/// returns the child hash unchanged.
#[must_use]
fn compute_storage_trie_root(child_hashes: &Children<Option<HashType>>) -> TrieHash {
    if child_hashes.count() == 0 {
        return TrieHash::from(EMPTY_TRIE_ROOT);
    }
    let mut child_hashes = child_hashes.clone();
    if let Some((_, child)) = child_hashes.take_only_child() {
        return single_child_storage_root(child);
    }
    let mut items: [RlpItem<'_>; BranchNode::MAX_CHILDREN + 1] =
        [RlpItem::Empty; BranchNode::MAX_CHILDREN + 1];
    for ((_, child), slot) in (&child_hashes).into_iter().zip(items.iter_mut()) {
        *slot = child_to_rlp_item(child.as_ref());
    }
    TrieHash::from(Keccak256::digest(encode_list(&items)))
}

/// Given an account node's value and its children's hashes, return the value with the
/// storageRoot field replaced by the computed hash of the storage sub-trie.
///
/// For leaf accounts (no children), the storage root is the empty trie hash.
/// For branch accounts, the storage root is computed from the children's hashes.
///
/// Returns `None` if the value is not well-formed account RLP.
#[must_use]
pub fn fix_account_storage_root_value(
    value: &[u8],
    child_hashes: &Children<Option<HashType>>,
) -> Option<Box<[u8]>> {
    let storage_root = compute_storage_trie_root(child_hashes);
    replace_list_field(value, 2, storage_root.as_slice()).ok()
}

/// Hash a node as the standalone root of an Ethereum storage trie.
///
/// In Ethereum, an account with exactly one storage entry has a storage trie
/// consisting of just that leaf, whose partial path is the full 64-nibble
/// storage key. Firewood stores that leaf as a child of the account branch
/// at depth 64, with only 63 nibbles of partial path — the first nibble is
/// the parent's child-slot index. This function folds the missing nibble
/// back onto the front of the child's partial path and hashes the result as
/// if the child were a standalone root.
///
/// Single source of truth for the storage-trie-root fold; prefer this over
/// inline folding so live hashing and proof verification cannot drift.
pub fn hash_node_as_storage_trie_root_parts<H: HashMode, Prefix: SplitPath, Partial: SplitPath>(
    account_full_prefix: Prefix,
    branch_nibble: PathComponent,
    partial_path: Partial,
    value_digest: Option<ValueDigest<&[u8]>>,
    children: Children<Option<HashType>>,
) -> HashType {
    let folded = JoinedPath::new(std::slice::from_ref(&branch_nibble), partial_path);
    H::to_hash(&HashableShunt::new(
        account_full_prefix,
        folded,
        value_digest,
        children,
    ))
}

/// Persist the computed storageRoot into an account node's RLP-encoded value,
/// in place. Only acts on nodes at account depth (64 nibbles) whose values are
/// well-formed Ethereum account RLP.
///
/// For branch accounts, the storage root is computed from the children's hashes.
/// For leaf accounts (no storage sub-trie), the storage root is the empty trie hash.
fn update_account_storage_root(node: &mut Node, path_prefix: &Path) {
    // Both lengths are usize counts of nibbles in a trie path, so their
    // sum cannot overflow on any platform firewood targets.
    let total_depth = path_prefix
        .0
        .len()
        .wrapping_add(node.partial_path().0.len());
    if total_depth != 64 {
        return;
    }

    match node {
        Node::Branch(b) => {
            let Some(old_value) = b.value.as_ref() else {
                return;
            };
            let child_hashes = b.children_hashes();
            if let Some(new_value) = fix_account_storage_root_value(old_value, &child_hashes) {
                b.value = Some(new_value);
            }
        }
        Node::Leaf(l) => {
            let empty_children: Children<Option<HashType>> = Children::new();
            if let Some(new_value) = fix_account_storage_root_value(&l.value, &empty_children) {
                l.value = new_value;
            }
        }
    }
}

/// Extract the `TrieHash` for the single-storage-child case. At account
/// depth storage child encodings always exceed 32 bytes (32-byte keys), so
/// the inline-RLP variant cannot occur; under the merkledb scheme every child
/// is already a 32-byte hash.
fn single_child_storage_root(child: HashType) -> crate::TrieHash {
    match child {
        HashType::Hash(hash) => hash,
        HashType::Rlp(_) => unreachable!(
            "account-depth single storage child cannot have inline RLP: \
             storage leaf encoding with 32-byte keys always exceeds 32 bytes"
        ),
    }
}

/// Encode one child slot of an account's storage branch as an [`RlpItem`].
/// Mirrors the dispatch the ethhash hasher does inline (see
/// `storage/src/hashers/ethhash.rs::EthHash::write_preimage`).
fn child_to_rlp_item(child: Option<&HashType>) -> RlpItem<'_> {
    match child {
        Some(HashType::Hash(hash)) => RlpItem::Bytes(hash.as_slice()),
        Some(HashType::Rlp(_)) => unreachable!(
            "account-depth storage child cannot have inline RLP: \
             storage node encoding with 32-byte keys always exceeds 32 bytes"
        ),
        None => RlpItem::Empty,
    }
}
