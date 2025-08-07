// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(test)]
mod tests;

use crate::proof::{Proof, ProofCollection, ProofError, ProofNode};
use crate::range_proof::RangeProof;
use crate::stream::{MerkleKeyValueStream, PathIterator};
use crate::v2::api::{self, FrozenProof, FrozenRangeProof, KeyType, ValueType};
use firewood_storage::{
    BranchNode, Child, FileIoError, HashType, HashedNodeReader, ImmutableProposal, IntoHashType,
    LeafNode, MaybePersistedNode, MutableProposal, NibblesIterator, Node, NodeStore, Parentable,
    Path, ReadableStorage, SharedNode, TrieHash, TrieReader, ValueDigest,
};

use futures::{StreamExt, TryStreamExt};
use metrics::counter;
use std::collections::HashSet;
use std::fmt::{Debug, Write};
use std::future::ready;
use std::io::Error;
use std::iter::once;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, SendError, Sender};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

/// Keys are boxed u8 slices
pub type Key = Box<[u8]>;

/// Values are boxed u8 slices
pub type Value = Box<[u8]>;

// convert a set of nibbles into a printable string
// panics if there is a non-nibble byte in the set
#[cfg(not(feature = "branch_factor_256"))]
fn nibbles_formatter<X: IntoIterator<Item = u8>>(nib: X) -> String {
    nib.into_iter()
        .map(|c| {
            *b"0123456789abcdef"
                .get(c as usize)
                .expect("requires nibbles") as char
        })
        .collect::<String>()
}

#[cfg(feature = "branch_factor_256")]
fn nibbles_formatter<X: IntoIterator<Item = u8>>(nib: X) -> String {
    let collected: Box<[u8]> = nib.into_iter().collect();
    hex::encode(&collected)
}

macro_rules! write_attributes {
    ($writer:ident, $node:expr, $value:expr) => {
        if !$node.partial_path.0.is_empty() {
            write!(
                $writer,
                " pp={}",
                nibbles_formatter($node.partial_path.0.clone())
            )
            .map_err(|e| FileIoError::from_generic_no_file(e, "write attributes"))?;
        }
        if !$value.is_empty() {
            match std::str::from_utf8($value) {
                Ok(string) if string.chars().all(char::is_alphanumeric) => {
                    write!($writer, " val={}", string)
                        .map_err(|e| FileIoError::from_generic_no_file(e, "write attributes"))?;
                }
                _ => {
                    let hex = hex::encode($value);
                    if hex.len() > 6 {
                        write!($writer, " val={:.6}...", hex).map_err(|e| {
                            FileIoError::from_generic_no_file(e, "write attributes")
                        })?;
                    } else {
                        write!($writer, " val={}", hex).map_err(|e| {
                            FileIoError::from_generic_no_file(e, "write attributes")
                        })?;
                    }
                }
            }
        }
    };
}

/// Returns the value mapped to by `key` in the subtrie rooted at `node`.
fn get_helper<T: TrieReader>(
    nodestore: &T,
    node: &Node,
    key: &[u8],
) -> Result<Option<SharedNode>, FileIoError> {
    // 4 possibilities for the position of the `key` relative to `node`:
    // 1. The node is at `key`
    // 2. The key is above the node (i.e. its ancestor)
    // 3. The key is below the node (i.e. its descendant)
    // 4. Neither is an ancestor of the other
    let path_overlap = PrefixOverlap::from(key, node.partial_path());
    let unique_key = path_overlap.unique_a;
    let unique_node = path_overlap.unique_b;

    match (
        unique_key.split_first().map(|(index, path)| (*index, path)),
        unique_node.split_first(),
    ) {
        (_, Some(_)) => {
            // Case (2) or (4)
            Ok(None)
        }
        (None, None) => Ok(Some(node.clone().into())), // 1. The node is at `key`
        (Some((child_index, remaining_key)), None) => {
            // 3. The key is below the node (i.e. its descendant)
            match node {
                Node::Leaf(_) => Ok(None),
                Node::Branch(node) => match node
                    .children
                    .get(child_index as usize)
                    .expect("index is in bounds")
                {
                    None => Ok(None),
                    Some(Child::Node(child)) => get_helper(nodestore, child, remaining_key),
                    Some(Child::AddressWithHash(addr, _)) => {
                        let child = nodestore.read_node(*addr)?;
                        get_helper(nodestore, &child, remaining_key)
                    }
                    Some(Child::MaybePersisted(maybe_persisted, _)) => {
                        let child = maybe_persisted.as_shared_node(nodestore)?;
                        get_helper(nodestore, &child, remaining_key)
                    }
                },
            }
        }
    }
}

#[derive(Debug)]
/// Parallel compatible wrapper for a Merkle trie
pub struct MerkleParallel<T, S> {
    merkle_arc: Option<Arc<T>>,
    root_node: Option<Node>,
    worker_pool: WorkerPool<S>,
}

impl<S: ReadableStorage + 'static> MerkleParallel<Merkle<NodeStore<MutableProposal, S>>, S> {
    /// Constructor for `MerkleParallel`.
    pub fn new(mut m: Merkle<NodeStore<MutableProposal, S>>) -> Self {
        let root_node = std::mem::take(m.nodestore.mut_root());
        let merkle_arc = Arc::new(m);
        let worker_pool = WorkerPool::new(merkle_arc.clone());
        MerkleParallel {
            merkle_arc: Some(merkle_arc),
            root_node,
            worker_pool,
        }
    }

    /// Paralle insert
    ///
    /// ## Panics
    ///
    /// Can panic if `merkle_arc` is None or `insert_parallel` returns an error (TODO: handle this correctly)
    pub fn insert(&mut self, key: &[u8], value: Value) {
        let mut merkle_arc: Arc<Merkle<NodeStore<MutableProposal, S>>> =
            self.merkle_arc.take().expect("merkle arc option is none");
        let insert_result =
            merkle_arc.insert_parallel(self.root_node.take(), &self.worker_pool, key, value);

        // TODO: handle error
        match insert_result.expect("insert_parallel returned an error") {
            ParallelInsertReturn::Performed(node) => {
                self.root_node = Some(node);
                self.merkle_arc = Some(merkle_arc);
            }
            ParallelInsertReturn::RetryNonThreaded(node, value) => {
                let mut merkle = self.wait_params(node, merkle_arc);

                // Perform non-parallel Merkle insert
                // TODO: Handle error
                merkle
                    .insert(key, value)
                    .expect("merkle insert returned an error");

                // Set all of the parameters that were taken previously.
                self.root_node = std::mem::take(merkle.nodestore.mut_root());
                merkle_arc = Arc::new(merkle);
                self.worker_pool.set_merkle(merkle_arc.clone());
                self.merkle_arc = Some(merkle_arc);
            }
        }
    }

    fn wait_params(
        &mut self,
        mut node: Node,
        merkle_arc: Arc<Merkle<NodeStore<MutableProposal, S>>>,
    ) -> Merkle<NodeStore<MutableProposal, S>> {
        let mut child_nodes: Vec<Option<Node>> =
            self.worker_pool.clear_merkle().expect("file io error");
        for (i, child_node_opt) in child_nodes.iter_mut().enumerate() {
            // If child_nodes is not empty, then node must be a branch
            if let Some(_child_node) = child_node_opt {
                if let Node::Branch(ref mut branch_node) = node {
                    branch_node.update_child(
                        i as u8,
                        Some(Child::Node(
                            child_node_opt
                                .take()
                                .expect("child_node_opt should not be none"),
                        )),
                    );
                } else {
                    // TODO: assert or log error
                }
            }
        }
        // All but one of the references to merkle_arc should be gone. Extract
        // the inner Merkle should we can perform a mut operations on it.
        let mut merkle = Arc::into_inner(merkle_arc).expect("merkle_arc reference count is not 1");
        *merkle.nodestore.mut_root() = Some(node);
        merkle
    }

    /// Wait until workers have completed
    ///
    /// ## Panics
    ///
    /// Can panic if root node is none
    pub fn wait(&mut self) -> Merkle<NodeStore<MutableProposal, S>> {
        let merkle_arc = self.merkle_arc.take().expect("merkle arc option is none");
        // The root can be None if the trie is empty. In this case, there should not be any
        // outstanding requests being handled by workers as long as the invariant that the
        // root cannot be removed while there are workers active. For this case, we just 
        // convert the merkle_arc back to a merkle and return it back to the caller.
        if let Some(node) = self.root_node.take() {
            self.wait_params(node, merkle_arc)
        } else {
            // TODO: Add counter to verify that there are no outstanding requests.
            let merkle_arc = self.merkle_arc.take().expect("merkle_arc is None");
            Arc::into_inner(merkle_arc).expect("merkle_arc reference count is not 1")
        }
    }
}

#[derive(Debug)]
/// Merkle operations against a nodestore
pub struct Merkle<T> {
    nodestore: T,
}

impl<T> Merkle<T> {
    pub(crate) fn into_inner(self) -> T {
        self.nodestore
    }
}

impl<T> From<T> for Merkle<T> {
    fn from(nodestore: T) -> Self {
        Merkle { nodestore }
    }
}

impl<T: TrieReader> Merkle<T> {
    pub(crate) fn root(&self) -> Option<SharedNode> {
        self.nodestore.root_node()
    }

    #[cfg(test)]
    pub(crate) const fn nodestore(&self) -> &T {
        &self.nodestore
    }

    /// Returns a proof that the given key has a certain value,
    /// or that the key isn't in the trie.
    ///
    /// ## Errors
    ///
    /// Returns an error if the trie is empty or an error occurs while reading from storage.
    pub fn prove(&self, key: &[u8]) -> Result<FrozenProof, ProofError> {
        let Some(root) = self.root() else {
            return Err(ProofError::Empty);
        };

        // Get the path to the key
        let path_iter = self.path_iter(key)?;
        let mut proof = Vec::new();
        for node in path_iter {
            let node = node?;
            proof.push(ProofNode::from(node));
        }

        if proof.is_empty() {
            // No nodes, even the root, are before `key`.
            // The root alone proves the non-existence of `key`.
            // TODO reduce duplicate code with ProofNode::from<PathIterItem>
            let child_hashes = if let Some(branch) = root.as_branch() {
                branch.children_hashes()
            } else {
                BranchNode::empty_children()
            };

            proof.push(ProofNode {
                key: root.partial_path().bytes(),
                #[cfg(feature = "ethhash")]
                partial_len: root.partial_path().0.len(),
                value_digest: root
                    .value()
                    .map(|value| ValueDigest::Value(value.to_vec().into_boxed_slice())),
                child_hashes,
            });
        }

        Ok(Proof::new(proof.into_boxed_slice()))
    }

    /// Verify that a range proof is valid for the specified key range and root hash.
    ///
    /// This method validates a range proof by constructing a partial trie from the proof data
    /// and verifying that it produces the expected root hash. The proof may contain fewer
    /// key-value pairs than requested if the peer chose to limit the response size.
    ///
    /// # Parameters
    ///
    /// * `first_key` - The requested start of the range (inclusive).
    ///   - If `Some(key)`, verifies the proof covers keys >= this key
    ///   - If `None`, verifies the proof starts from the beginning of the trie
    ///
    /// * `last_key` - The requested end of the range (inclusive).
    ///   - If `Some(key)`, represents the upper bound that was requested
    ///   - If `None`, indicates no upper bound was specified
    ///   - Note: The proof may contain fewer keys than requested if the peer limited the response
    ///
    /// * `root_hash` - The expected root hash of the trie. The constructed partial trie
    ///   from the proof must produce this exact hash for the proof to be valid.
    ///
    /// * `proof` - The range proof to verify, containing:
    ///   - Start proof: Merkle proof for the lower boundary
    ///   - End proof: Merkle proof for the upper boundary
    ///   - Key-value pairs: The actual entries within the range
    ///
    /// # Returns
    ///
    /// Returns the constructed [`Merkle<Arc<ImmutableProposal>, _>`] that was built and
    /// verified from the proof data, if the proof is valid.
    ///
    /// # Verification Process
    ///
    /// The verification follows these steps:
    /// 1. **Structural validation**: Verify the proof structure is well-formed
    ///    - Check that start/end proofs are consistent with the key range
    ///    - Ensure key-value pairs are in the correct order
    ///    - Validate that boundary proofs correctly bound the key-value pairs
    ///
    /// 2. **Proposal construction**: Build a proposal trie containing the proof data
    ///    - Insert all key-value pairs from the proof
    ///    - Incorporate nodes from the start and end proofs
    ///    - Handle edge cases for empty ranges or partial proofs
    ///
    /// 3. **Hash verification**: Compute the root hash of the constructed proposal
    ///    - The computed hash must match the provided `root_hash` exactly
    ///    - Any mismatch indicates an invalid or tampered proof
    ///
    /// # Errors
    ///
    /// * [`api::Error::ProofError`] - The proof structure is malformed or inconsistent
    /// * [`api::Error::InvalidRange`] - The proof boundaries don't match the requested range
    /// * [`api::Error::IncorrectRootHash`] - The computed root hash doesn't match the expected hash
    /// * [`api::Error`] - Other errors during proposal construction or verification
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Verify a range proof received from a peer
    /// let verified_proposal = merkle.verify_range_proof(
    ///     Some(b"alice"),
    ///     Some(b"charlie"),
    ///     &expected_root_hash,
    ///     &range_proof
    /// )?;
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// - Structural validation is performed first to avoid expensive proposal construction
    ///   for obviously invalid proofs
    /// - The method is designed to handle partial proofs where the peer provides less
    ///   data than requested, which is common for large ranges
    /// - Future optimization: Consider caching partial verification results for
    ///   incremental range proof verification
    pub fn verify_range_proof(
        &self,
        _first_key: Option<impl KeyType>,
        _last_key: Option<impl KeyType>,
        _root_hash: &TrieHash,
        _proof: &RangeProof<impl KeyType, impl ValueType, impl ProofCollection>,
    ) -> Result<(), api::Error> {
        todo!()
    }

    pub(crate) fn path_iter<'a>(
        &self,
        key: &'a [u8],
    ) -> Result<PathIterator<'_, 'a, T>, FileIoError> {
        PathIterator::new(&self.nodestore, key)
    }

    pub(super) fn key_value_iter(&self) -> MerkleKeyValueStream<'_, T> {
        MerkleKeyValueStream::from(&self.nodestore)
    }

    pub(super) fn key_value_iter_from_key<K: AsRef<[u8]>>(
        &self,
        key: K,
    ) -> MerkleKeyValueStream<'_, T> {
        // TODO danlaine: change key to &[u8]
        MerkleKeyValueStream::from_key(&self.nodestore, key.as_ref())
    }

    /// Generate a cryptographic proof for a range of key-value pairs in the Merkle trie.
    ///
    /// This method creates a range proof that can be used to verify the existence (or absence)
    /// of a contiguous set of keys within the trie. The proof includes boundary proofs and
    /// the actual key-value pairs within the specified range.
    ///
    /// # Parameters
    ///
    /// * `start_key` - The optional lower bound of the range (inclusive).
    ///   - If `Some(key)`, the proof will include all keys >= this key
    ///   - If `None`, the proof starts from the beginning of the trie
    ///
    /// * `end_key` - The optional upper bound of the range (inclusive).
    ///   - If `Some(key)`, the proof will include all keys <= this key
    ///   - If `None`, the proof extends to the end of the trie
    ///
    /// * `limit` - Optional maximum number of key-value pairs to include in the proof.
    ///   - If `Some(n)`, at most n key-value pairs will be included
    ///   - If `None`, all key-value pairs in the range will be included
    ///   - Useful for paginating through large ranges
    ///   - **NOTE**: avalanchego's limit is based on the entire packet size and not the
    ///     number of key-value pairs. Currently, we only limit by the number of pairs.
    ///
    /// # Returns
    ///
    /// A `FrozenRangeProof` containing:
    /// - Start proof: Merkle proof for the first key in the range
    /// - End proof: Merkle proof for the last key in the range
    /// - Key-value pairs: All entries within the specified bounds (up to the limit)
    ///
    /// # Errors
    ///
    /// * `api::Error::InvalidRange` - If `start_key` > `end_key` when both are provided.
    ///   This ensures the range bounds are logically consistent.
    ///
    /// * `api::Error::RangeProofOnEmptyTrie` - If the trie is empty and the caller
    ///   requests a proof for the entire trie (both `start_key` and `end_key` are `None`).
    ///   This prevents generating meaningless proofs for non-existent data.
    ///
    /// * `api::Error` - Various other errors can occur during proof generation, such as:
    ///   - I/O errors when reading nodes from storage
    ///   - Corrupted trie structure
    ///   - Invalid node references
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Prove all keys between "alice" and "charlie"
    /// let proof = merkle.range_proof(
    ///     Some(b"alice"),
    ///     Some(b"charlie"),
    ///     None
    /// ).await?;
    ///
    /// // Prove the first 100 keys starting from "alice"
    /// let proof = merkle.range_proof(
    ///     Some(b"alice"),
    ///     None,
    ///     Some(NonZeroUsize::new(100).unwrap())
    /// ).await?;
    ///
    /// // Prove that no keys exist in a range
    /// let proof = merkle.range_proof(
    ///     Some(b"aardvark"),
    ///     Some(b"aaron"),
    ///     None
    /// ).await?;
    /// ```
    pub(super) async fn range_proof(
        &self,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenRangeProof, api::Error> {
        if let (Some(k1), Some(k2)) = (&start_key, &end_key) {
            if k1 > k2 {
                return Err(api::Error::InvalidRange {
                    start_key: k1.to_vec().into(),
                    end_key: k2.to_vec().into(),
                });
            }
        }

        let mut stream = match start_key {
            // TODO: fix the call-site to force the caller to do the allocation
            Some(key) => self.key_value_iter_from_key(key.to_vec().into_boxed_slice()),
            None => self.key_value_iter(),
        };

        // fetch the first key from the stream
        let first_result = stream.next().await;

        // transpose the Option<Result<T, E>> to Result<Option<T>, E>
        // If this is an error, the ? operator will return it
        let Some((first_key, first_value)) = first_result.transpose()? else {
            // The trie is empty.
            if start_key.is_none() && end_key.is_none() {
                // The caller requested a range proof over an empty trie.
                return Err(api::Error::RangeProofOnEmptyTrie);
            }

            let start_proof = start_key
                .map(|start_key| self.prove(start_key))
                .transpose()?
                .unwrap_or_default();

            let end_proof = end_key
                .map(|end_key| self.prove(end_key))
                .transpose()?
                .unwrap_or_default();

            return Ok(RangeProof::new(start_proof, end_proof, Box::new([])));
        };

        let start_proof = self.prove(&first_key)?;
        let limit = limit.map(|old_limit| old_limit.get().saturating_sub(1));

        let mut key_values = vec![(first_key, first_value)];

        // we stop streaming if either we hit the limit or the key returned was larger
        // than the largest key requested
        key_values.extend(
            stream
                .take(limit.unwrap_or(usize::MAX))
                .take_while(|kv| {
                    // no last key asked for, so keep going
                    let Some(last_key) = end_key else {
                        return ready(true);
                    };

                    // return the error if there was one
                    let Ok(kv) = kv else {
                        return ready(true);
                    };

                    // keep going if the key returned is less than the last key requested
                    ready(&*kv.0 <= last_key)
                })
                .try_collect::<Vec<(Key, Value)>>()
                .await?,
        );

        let end_proof = key_values
            .last()
            .map(|(largest_key, _)| self.prove(largest_key))
            .transpose()?
            .unwrap_or_default();

        Ok(RangeProof::new(
            start_proof,
            end_proof,
            key_values.into_boxed_slice(),
        ))
    }

    pub(crate) fn get_value(&self, key: &[u8]) -> Result<Option<Value>, FileIoError> {
        let Some(node) = self.get_node(key)? else {
            return Ok(None);
        };
        Ok(node.value().map(|v| v.to_vec().into_boxed_slice()))
    }

    pub(crate) fn get_node(&self, key: &[u8]) -> Result<Option<SharedNode>, FileIoError> {
        let Some(root) = self.root() else {
            return Ok(None);
        };

        let key = Path::from_nibbles_iterator(NibblesIterator::new(key));
        get_helper(&self.nodestore, &root, &key)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// Dump a node, recursively, to a dot file
    pub(crate) fn dump_node(
        &self,
        node: &MaybePersistedNode,
        hash: Option<&HashType>,
        seen: &mut HashSet<String>,
        writer: &mut dyn Write,
    ) -> Result<(), FileIoError> {
        writeln!(writer, "  {node}[label=\"{node}")
            .map_err(Error::other)
            .map_err(|e| FileIoError::new(e, None, 0, None))?;
        if let Some(hash) = hash {
            write!(writer, " H={hash:.6?}")
                .map_err(Error::other)
                .map_err(|e| FileIoError::new(e, None, 0, None))?;
        }

        match &*node.as_shared_node(&self.nodestore)? {
            Node::Branch(b) => {
                write_attributes!(writer, b, &b.value.clone().unwrap_or(Box::from([])));
                writeln!(writer, "\"]")
                    .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                for (childidx, child) in b.children.iter().enumerate() {
                    let (child, child_hash) = match child {
                        None => continue,
                        Some(node) => (node.as_maybe_persisted_node(), node.hash()),
                    };

                    let inserted = seen.insert(format!("{child}"));
                    if inserted {
                        writeln!(writer, "  {node} -> {child}[label=\"{childidx:x}\"]")
                            .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                        self.dump_node(&child, child_hash, seen, writer)?;
                    } else {
                        // We have already seen this child, which shouldn't happen.
                        // Indicate this with a red edge.
                        writeln!(
                            writer,
                            "  {node} -> {child}[label=\"{childidx:x} (dup)\" color=red]"
                        )
                        .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                    }
                }
            }
            Node::Leaf(l) => {
                write_attributes!(writer, l, &l.value);
                writeln!(writer, "\" shape=rect]")
                    .map_err(|e| FileIoError::from_generic_no_file(e, "write leaf"))?;
            }
        }
        Ok(())
    }

    /// Dump the trie to a dot file.
    ///
    /// This function is primarily used in testing, but also has an API implementation
    ///
    /// Dot files can be rendered using `dot -Tpng -o output.png input.dot`
    /// or online at <https://dreampuf.github.io/GraphvizOnline>
    pub(crate) fn dump(&self) -> Result<String, Error> {
        let root = self.nodestore.root_as_maybe_persisted_node();

        let mut result = String::new();
        writeln!(result, "digraph Merkle {{\n  rankdir=LR;").map_err(Error::other)?;
        if let (Some(root), Some(root_hash)) = (root, self.nodestore.root_hash()) {
            writeln!(result, " root -> {root}")
                .map_err(Error::other)
                .map_err(|e| FileIoError::new(e, None, 0, None))
                .map_err(Error::other)?;
            let mut seen = HashSet::new();
            self.dump_node(
                &root,
                Some(&root_hash.into_hash_type()),
                &mut seen,
                &mut result,
            )
            .map_err(Error::other)?;
        }
        write!(result, "}}")
            .map_err(Error::other)
            .map_err(|e| FileIoError::new(e, None, 0, None))
            .map_err(Error::other)?;

        Ok(result)
    }
}

impl<F: Parentable, S: ReadableStorage> Merkle<NodeStore<F, S>> {
    /// Forks the current Merkle trie into a new mutable proposal.
    ///
    /// ## Errors
    ///
    /// Returns an error if the nodestore cannot be created. See [`NodeStore::new`].
    pub fn fork(&self) -> Result<Merkle<NodeStore<MutableProposal, S>>, FileIoError> {
        NodeStore::new(&self.nodestore).map(Into::into)
    }
}

impl<S: ReadableStorage> TryFrom<Merkle<NodeStore<MutableProposal, S>>>
    for Merkle<NodeStore<Arc<ImmutableProposal>, S>>
{
    type Error = FileIoError;
    fn try_from(m: Merkle<NodeStore<MutableProposal, S>>) -> Result<Self, Self::Error> {
        Ok(Merkle {
            nodestore: m.nodestore.try_into()?,
        })
    }
}

#[derive(Debug)]
/// Different operations that can be sent to the worker pool
pub enum MerkleOp<S> {
    /// Operaiton to call ``insert_helper`` at the specified node.
    InsertData(Box<Option<Node>>, Box<[u8]>, Box<[u8]>),

    /// Operation to termine the worker threads in the thread pool.
    Terminate,

    /// Clearing Merkle Arc
    ClearMerkle,

    /// Setting Merkle Arc
    SetMerkle(Box<Arc<Merkle<NodeStore<MutableProposal, S>>>>),
    //SetMerkle(Option<Arc<Mutex<Merkle<NodeStore<MutableProposal, S>>>>>)
}

//type HostReceiver = std::sync::mpsc::Receiver<Result<Node, FileIoError>>;
//type HostReceiver = std::sync::mpsc::Receiver<WorkerReturn>;

type WorkerData<S> = (Sender<MerkleOp<S>>, Receiver<WorkerReturn>, JoinHandle<()>);

enum WorkerReturn {
    NodeResult(Result<Option<Node>, FileIoError>),
    //MerkleClearComplete,
}

#[derive(Debug)]
/// Worker pool to issue concurrent inserts to a Merkle trie
pub struct WorkerPool<S> {
    //merkle: Arc<Merkle<NodeStore<MutableProposal, S>>>,
    workers_data: Vec<WorkerData<S>>,
}

impl<S: ReadableStorage + 'static> WorkerPool<S> {
    fn create_one_worker(
        merkle: Arc<Merkle<NodeStore<MutableProposal, S>>>,
        //index: usize,
    ) -> (Sender<MerkleOp<S>>, Receiver<WorkerReturn>, JoinHandle<()>) {
        let (host_sender, thread_receiver) = mpsc::channel::<MerkleOp<S>>();
        let (thread_sender, host_receiver) = mpsc::channel::<WorkerReturn>();
        let mut merkle: Option<Arc<Merkle<NodeStore<MutableProposal, S>>>> = Some(merkle);

        let handle = thread::spawn(move || {
            let mut inserted_root = None;
            //let thread_id = thread::current().id();
            loop {
                let Ok(v) = thread_receiver.recv() else {
                    return; // Thread is unable to recv data from parent
                };
                match v {
                    MerkleOp::InsertData(sub_trie_root, key, value) => {
                        // There are four possible cases to consider here.
                        // 1.   inserted_root is None and subtrie_root is None. This means that we need to
                        //      create a sub-trie. We create a Leaf node and set it to inserted_root and
                        //      continue from the loop.
                        // 2.   inserted_root is None and subtrie_root is *not* None. This means we are adding
                        //      to an existing trie rooted at subtrie_root. We set node to sub_trie_root.
                        // 3.   inserted_root is *not* None and node_opt is None. Same as case 2 except
                        //      we are adding to the sub-trie rooted at inserted_root.
                        // 4.   Both insert_root and subtrie_root are *not* None. This case should not happen
                        //      as subtrie_root is taken from the children array and any subsequent insert to
                        //      that branch should set sub_trie_root to None.
                        let node = match *sub_trie_root {
                            Some(node_from_sub_trie_root) => {
                                assert!(
                                    inserted_root.is_none(),
                                    "inserted_root should be None if sub_trie_root is not None"
                                );
                                node_from_sub_trie_root
                                /*
                                if let Some(node_from_inserted) = inserted_root {
                                    println!("############### Do we every get to this case? ###############");
                                    node_from_inserted
                                } else {
                                    node_from_sub_trie_root
                                }
                                */
                            }
                            None => {
                                if let Some(node_from_inserted) = inserted_root {
                                    node_from_inserted
                                } else {
                                    inserted_root = Some(Node::Leaf(LeafNode {
                                        partial_path: key.into(),
                                        value,
                                    }));
                                    continue;
                                }
                            }
                        };

                        let merkle_arc: Arc<Merkle<NodeStore<MutableProposal, S>>> =
                            merkle.expect("Merkle is not set before insert");

                        let path: Path = key.into();
                        let insert_result = merkle_arc.insert_helper(node, path.as_ref(), value);
                        inserted_root = match insert_result {
                            Ok(n) => Some(n),
                            // TODO: Send back error message to the main thread. Currently just breaks out of this
                            //       loop and termintes the thread. Should there be a separate error channel, or
                            //       should the same channel be used for sending errors?
                            Err(_) => {
                                break;
                            }
                        };
                        merkle = Some(merkle_arc);
                    }
                    MerkleOp::Terminate => {
                        break;
                    }
                    MerkleOp::ClearMerkle => {
                        merkle = None; // Decrement the Arc counter
                        let _ = thread_sender.send(WorkerReturn::NodeResult(Ok(inserted_root)));
                        inserted_root = None;
                    }
                    MerkleOp::SetMerkle(m) => {
                        merkle = Some(*m);
                    }
                }
            }
        });
        (host_sender, host_receiver, handle)
    }

    /// Test
    ///
    /// ## Panics
    ///
    /// Test
    #[must_use]
    pub fn new(merkle: Arc<Merkle<NodeStore<MutableProposal, S>>>) -> Self {
        let workers_data: Vec<WorkerData<S>> = (0..BranchNode::MAX_CHILDREN)
            .map(|_| WorkerPool::create_one_worker(merkle.clone()))
            .collect();

        WorkerPool { workers_data }
    }

    /// Send an insert to the worker threads
    ///
    /// ## Panics
    ///
    /// Can panic if workerpool vector is incorrectly initialized.
    ///
    /// ## Errors
    ///
    /// ``FileIOError`` from reading a node.
    pub fn insert(
        &self,
        node: Option<Node>,
        child_index: usize,
        key: &[u8],
        value: Value,
    ) -> Result<(), SendError<MerkleOp<S>>> {
        //println!("In workerpool insert");
        self.workers_data
            .get(child_index)
            .expect("out of bounds on vector")
            .0
            .send(MerkleOp::InsertData(Box::new(node), key.into(), value))
    }

    /// Setting the Merkle trie for the woker pool. Must be cleared first.
    ///
    /// ## Panics
    ///
    /// Can panic if workerpool trie is incorrectly initialized.
    pub fn set_merkle(&self, merkle: Arc<Merkle<NodeStore<MutableProposal, S>>>) {
        for i in 0..BranchNode::MAX_CHILDREN {
            let _ = self
                .workers_data
                .get(i)
                .expect("empty vector")
                .0
                .send(MerkleOp::SetMerkle(Box::new(merkle.clone())));
        }
    }

    /// Clears the Merkle trie so that it can be moved out of the Arc
    ///
    /// ## Panics
    ///
    /// Can panic if workerpool vector is incorrectly initialized.
    ///
    /// ## Errors
    ///
    /// Can return a `FileIoError` that came from a previous insert.
    pub fn clear_merkle(&self) -> Result<Vec<Option<Node>>, FileIoError> {
        for i in 0..BranchNode::MAX_CHILDREN {
            let _ = self
                .workers_data
                .get(i)
                .expect("empty vector")
                .0
                .send(MerkleOp::ClearMerkle);
        }

        //let mut ret_vec = vec![];
        let mut ret_vec = Vec::with_capacity(BranchNode::MAX_CHILDREN);
        for i in 0..BranchNode::MAX_CHILDREN {
            // Blocks until result completes for now in this initial prototype.
            let WorkerReturn::NodeResult(result) = self
                .workers_data
                .get(i)
                .expect("empty vector")
                .1
                .recv()
                .expect("recv error");
            // TODO: Handle this error better
            ret_vec.push(result.expect("File IO error"));
        }
        Ok(ret_vec)
    }
}

#[derive(Debug)]
/// Return value for calling `insert_helper_parallel`.
pub enum ParallelInsertReturn {
    /// Return this if operation cannot be performed while there are other operations outstanding.
    RetryNonThreaded(Node, Value),

    /// Return this if operation was performed in paralle.
    Performed(Node),
}

#[expect(clippy::missing_errors_doc)]
impl<S: ReadableStorage + 'static> Merkle<NodeStore<MutableProposal, S>> {
    /// Convert a merkle backed by an `MutableProposal` into an `ImmutableProposal`
    ///
    /// This function is only used in benchmarks and tests
    ///
    /// ## Panics
    ///
    /// Panics if the conversion fails. This should only be used in tests or benchmarks.
    #[must_use]
    pub fn hash(self) -> Merkle<NodeStore<Arc<ImmutableProposal>, S>> {
        self.try_into().expect("failed to convert")
    }

    /*
    /// Insert using a worker pool
    pub fn insert_worker_pool<T: ReadableStorage + 'static>(
        &self,
        root: Option<Node>,
        worker_pool: &WorkerPool<T>,
        child_index: usize,
        key: &[u8],
        value: Value,
    ) -> Result<(), SendError<MerkleOp<T>>> {
        worker_pool.insert(root, child_index, key, value)?;
        //let root_node = self.insert_helper(root_node, key.as_ref(), value)?;
        Ok(())
    }
    */

    fn fetch_or_create_root(&mut self, key: &[u8], value: Value) -> Option<(Node, Path, Value)> {
        let key = Path::from_nibbles_iterator(NibblesIterator::new(key));

        let root = self.nodestore.mut_root();

        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. Create a new leaf node with `value` and set
            // it as the root.
            let root_node = Node::Leaf(LeafNode {
                partial_path: key,
                value,
            });
            *root = root_node.into();
            return None;
        };
        Some((root_node, key, value))
    }

    /// Map `key` to `value` in the trie.
    /// Each element of key is 2 nibbles.
    pub fn insert(&mut self, key: &[u8], value: Value) -> Result<(), FileIoError> {
        if let Some(root_tuple) = self.fetch_or_create_root(key, value) {
            let (node, key, value) = root_tuple;
            let root_node = self.insert_helper(node, key.as_ref(), value)?;
            *self.nodestore.mut_root() = root_node.into();
        }
        Ok(())
    }

    /// Parallel insertion
    pub fn insert_parallel(
        &self,
        root: Option<Node>,
        worker_pool: &WorkerPool<S>,
        key: &[u8],
        value: Value,
    ) -> Result<ParallelInsertReturn, FileIoError> {
        // If root is None, create the root node and return.
        let path_key = Path::from_nibbles_iterator(NibblesIterator::new(key));
        let Some(root_node) = root else {
            return Ok(ParallelInsertReturn::Performed(Node::Leaf(LeafNode {
                partial_path: path_key,
                value,
            })));
        };
        let insert_return =
            self.insert_parallel_helper(worker_pool, root_node, path_key.as_ref(), value)?;
        Ok(insert_return)
    }

    /// Map `key` to `value` into the subtrie rooted at `node`.
    /// Each element of `key` is 1 nibble.
    /// Returns the new root of the subtrie.
    pub fn insert_parallel_helper(
        &self,
        worker_pool: &WorkerPool<S>,
        mut node: Node,
        key: &[u8],
        value: Value,
    ) -> Result<ParallelInsertReturn, FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`
        // 2. The key is above the node (i.e. its ancestor)
        // 3. The key is below the node (i.e. its descendant)
        // 4. Neither is an ancestor of the other
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                //.map(|(index, path)| (*index, path.into())),
                .map(|(index, path)| (*index, <&[u8] as std::convert::Into<Path>>::into(path))),
            unique_node
                .split_first()
                .map(|(index, path)| (*index, <&[u8] as std::convert::Into<Path>>::into(path))),
            //.map(|(index, path)| (*index, path.into())),
        ) {
            (None, None) => {
                //Ok(ParallelInsertReturn::RetryNonThreaded(node, value))
                // 1. The node is at `key`
                node.update_value(value);
                counter!("firewood.insert", "merkle" => "update").increment(1);
                Ok(ParallelInsertReturn::Performed(node))
            }
            (None, Some((_child_index, _partial_path))) => {
                // Rotates the root. Should not be applied until the outstanding operations
                // have been merged into the root. Return a special type to tell the caller
                // to block until all operations are done and then perform a non-threaded
                // insert for this key/value
                Ok(ParallelInsertReturn::RetryNonThreaded(node, value))
            }
            (Some((child_index, partial_path)), None) => {
                // 3. The key is below the node (i.e. its descendant)
                //    ...                         ...
                //     |                           |
                //    node         -->            node
                //     |                           |
                //    ... (key may be below)       ... (key is below)
                match node {
                    Node::Branch(ref mut branch) => {
                        #[expect(clippy::indexing_slicing)]
                        let child = match std::mem::take(&mut branch.children[child_index as usize])
                        {
                            None => {
                                //return Ok(ParallelInsertReturn::RetryNonThreaded(node, value))
                                // Not handling this case correctly. Need to send this to the worker thread!!

                                //println!("&&&&& inserted key:{:?} value:{value:?} child_index {child_index:?}", partial_path.as_ref());
                                /*
                                // TODO: Send a None to the worker pool? Or just perform the insert here?
                                // There is no child at this index.
                                // Create a new leaf and put it here.
                                let new_leaf = Node::Leaf(LeafNode {
                                    value,
                                    partial_path,
                                });
                                branch.update_child(child_index, Some(Child::Node(new_leaf)));
                                counter!("firewood.insert", "merkle"=>"below").increment(1);
                                return Ok(ParallelInsertReturn::Performed(node));
                                */
                                let _ = worker_pool.insert(
                                    None,
                                    child_index as usize,
                                    partial_path.as_ref(),
                                    value,
                                );
                                //let _ = worker_pool.insert(None, child_index as usize, key, value);
                                return Ok(ParallelInsertReturn::Performed(node));
                            }
                            Some(Child::Node(child)) => child,
                            Some(Child::AddressWithHash(addr, _)) => {
                                // TODO: Send this address to the worker pool.
                                self.nodestore.read_for_update(addr.into())?
                            }
                            Some(Child::MaybePersisted(maybe_persisted, _)) => {
                                // TODO: send this maybe_persisted to the worker pool.
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        //Ok(ParallelInsertReturn::RetryNonThreaded(node, value))

                        //println!(
                        //    "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! inserting key {key:?} at child_index {child_index:?} with partial path {:?}",
                        //    partial_path.as_ref()
                        //);
                        //let child = self.insert_helper(child, partial_path.as_ref(), value)?;
                        //branch.update_child(child_index, Some(Child::Node(child)));
                        // TODO: Currently there is only one thread. Once we go multi-thread, we
                        //       should have child_index as a parameter to insert.
                        let _ = worker_pool.insert(
                            Some(child),
                            child_index as usize,
                            partial_path.as_ref(),
                            value,
                        );
                        Ok(ParallelInsertReturn::Performed(node))
                    }
                    Node::Leaf(ref mut _leaf) => {
                        // Rotates the root. Should not be applied until the outstanding operations
                        // have been merged into the root. Return a special type to tell the caller
                        // to block until all operations are done and then perform a non-threaded
                        // insert for this key/value
                        Ok(ParallelInsertReturn::RetryNonThreaded(node, value))
                    }
                }
            }
            (Some((_key_index, _key_partial_path)), Some((_node_index, _node_partial_path))) => {
                // 4. Neither is an ancestor of the other
                //    ...                         ...
                //     |                           |
                //    node         -->            branch
                //     |                           |    \
                //                               node   key
                // Make a branch node that has both the current node and a new leaf node as children.
                // (When called from the root): Rotates the root. Should block until all parallel
                // operations have completed.
                //
                // Rotates the root. Should not be applied until the outstanding operations
                // have been merged into the root. Return a special type to tell the caller
                // to block until all operations are done and then perform a non-threaded
                // insert for this key/value
                Ok(ParallelInsertReturn::RetryNonThreaded(node, value))
            }
        }
    }

    /// Map `key` to `value` into the subtrie rooted at `node`.
    /// Each element of `key` is 1 nibble.
    /// Returns the new root of the subtrie.
    pub fn insert_helper(
        &self,
        mut node: Node,
        key: &[u8],
        value: Value,
    ) -> Result<Node, FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`
        // 2. The key is above the node (i.e. its ancestor)
        // 3. The key is below the node (i.e. its descendant)
        // 4. Neither is an ancestor of the other
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, path.into())),
            unique_node
                .split_first()
                .map(|(index, path)| (*index, path.into())),
        ) {
            (None, None) => {
                println!("###### Branch 1");
                // 1. The node is at `key`
                node.update_value(value);
                counter!("firewood.insert", "merkle" => "update").increment(1);
                Ok(node)
            }
            (None, Some((child_index, partial_path))) => {
                // 2. The key is above the node (i.e. its ancestor)
                // Make a new branch node and insert the current node as a child.
                //    ...                ...
                //     |     -->          |
                //    node               key
                //                        |
                //                       node
                //println!("###### Branch 2");
                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: Some(value),
                    children: BranchNode::empty_children(),
                };
                // Shorten the node's partial path since it has a new parent.
                node.update_partial_path(partial_path);
                branch.update_child(child_index, Some(Child::Node(node)));
                counter!("firewood.insert", "merkle"=>"above").increment(1);

                Ok(Node::Branch(Box::new(branch)))
            }
            (Some((child_index, partial_path)), None) => {
                // 3. The key is below the node (i.e. its descendant)
                //    ...                         ...
                //     |                           |
                //    node         -->            node
                //     |                           |
                //    ... (key may be below)       ... (key is below)
                //println!("###### Branch 3");
                match node {
                    Node::Branch(ref mut branch) => {
                        #[expect(clippy::indexing_slicing)]
                        let child = match std::mem::take(&mut branch.children[child_index as usize])
                        {
                            None => {
                                // There is no child at this index.
                                // Create a new leaf and put it here.
                                let new_leaf = Node::Leaf(LeafNode {
                                    value,
                                    partial_path,
                                });
                                branch.update_child(child_index, Some(Child::Node(new_leaf)));
                                counter!("firewood.insert", "merkle"=>"below").increment(1);
                                return Ok(node);
                            }
                            Some(Child::Node(child)) => child,
                            Some(Child::AddressWithHash(addr, _)) => {
                                self.nodestore.read_for_update(addr.into())?
                            }
                            Some(Child::MaybePersisted(maybe_persisted, _)) => {
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        let child = self.insert_helper(child, partial_path.as_ref(), value)?;
                        branch.update_child(child_index, Some(Child::Node(child)));
                        Ok(node)
                    }
                    Node::Leaf(ref mut leaf) => {
                        // Turn this node into a branch node and put a new leaf as a child.
                        let mut branch = BranchNode {
                            partial_path: std::mem::replace(&mut leaf.partial_path, Path::new()),
                            value: Some(std::mem::take(&mut leaf.value)),
                            children: BranchNode::empty_children(),
                        };

                        let new_leaf = Node::Leaf(LeafNode {
                            value,
                            partial_path,
                        });

                        branch.update_child(child_index, Some(Child::Node(new_leaf)));

                        counter!("firewood.insert", "merkle"=>"split").increment(1);
                        Ok(Node::Branch(Box::new(branch)))
                    }
                }
            }
            (Some((key_index, key_partial_path)), Some((node_index, node_partial_path))) => {
                // 4. Neither is an ancestor of the other
                //    ...                         ...
                //     |                           |
                //    node         -->            branch
                //     |                           |    \
                //                               node   key
                // Make a branch node that has both the current node and a new leaf node as children.
                //println!("###### Branch 4");
                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: None,
                    children: BranchNode::empty_children(),
                };

                node.update_partial_path(node_partial_path);
                branch.update_child(node_index, Some(Child::Node(node)));

                let new_leaf = Node::Leaf(LeafNode {
                    value,
                    partial_path: key_partial_path,
                });
                branch.update_child(key_index, Some(Child::Node(new_leaf)));

                counter!("firewood.insert", "merkle" => "split").increment(1);
                Ok(Node::Branch(Box::new(branch)))
            }
        }
    }

    /// Removes the value associated with the given `key`.
    /// Returns the value that was removed, if any.
    /// Otherwise returns `None`.
    /// Each element of `key` is 2 nibbles.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Value>, FileIoError> {
        let key = Path::from_nibbles_iterator(NibblesIterator::new(key));

        let root = self.nodestore.mut_root();
        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. There is nothing to remove.
            counter!("firewood.remove", "prefix" => "false", "result" => "nonexistent")
                .increment(1);
            return Ok(None);
        };

        let (root_node, removed_value) = self.remove_helper(root_node, &key)?;
        *self.nodestore.mut_root() = root_node;
        if removed_value.is_some() {
            counter!("firewood.remove", "prefix" => "false", "result" => "success").increment(1);
        } else {
            counter!("firewood.remove", "prefix" => "false", "result" => "nonexistent")
                .increment(1);
        }
        Ok(removed_value)
    }

    /// Removes the value associated with the given `key` from the subtrie rooted at `node`.
    /// Returns the new root of the subtrie and the value that was removed, if any.
    /// Each element of `key` is 1 nibble.
    #[expect(clippy::too_many_lines)]
    fn remove_helper(
        &self,
        mut node: Node,
        key: &[u8],
    ) -> Result<(Option<Node>, Option<Value>), FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`
        // 2. The key is above the node (i.e. its ancestor)
        // 3. The key is below the node (i.e. its descendant)
        // 4. Neither is an ancestor of the other
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, Path::from(path))),
            unique_node.split_first(),
        ) {
            (_, Some(_)) => {
                // Case (2) or (4)
                Ok((Some(node), None))
            }
            (None, None) => {
                // 1. The node is at `key`
                match &mut node {
                    Node::Branch(branch) => {
                        let Some(removed_value) = branch.value.take() else {
                            // The branch has no value. Return the node as is.
                            return Ok((Some(node), None));
                        };

                        // This branch node has a value.
                        // If it has multiple children, return the node as is.
                        // Otherwise, its only child becomes the root of this subtrie.
                        let mut children_iter =
                            branch
                                .children
                                .iter_mut()
                                .enumerate()
                                .filter_map(|(index, child)| {
                                    child.as_mut().map(|child| (index, child))
                                });

                        let (child_index, child) = children_iter
                            .next()
                            .expect("branch node must have children");

                        if children_iter.next().is_some() {
                            // The branch has more than 1 child so it can't be removed.
                            Ok((Some(node), Some(removed_value)))
                        } else {
                            // The branch's only child becomes the root of this subtrie.
                            let mut child = match child {
                                Child::Node(child_node) => std::mem::take(child_node),
                                Child::AddressWithHash(addr, _) => {
                                    self.nodestore.read_for_update((*addr).into())?
                                }
                                Child::MaybePersisted(maybe_persisted, _) => {
                                    self.nodestore.read_for_update(maybe_persisted.clone())?
                                }
                            };

                            // The child's partial path is the concatenation of its (now removed) parent,
                            // its (former) child index, and its partial path.
                            match child {
                                Node::Branch(ref mut child_branch) => {
                                    let partial_path = Path::from_nibbles_iterator(
                                        branch
                                            .partial_path
                                            .iter()
                                            .copied()
                                            .chain(once(child_index as u8))
                                            .chain(child_branch.partial_path.iter().copied()),
                                    );
                                    child_branch.partial_path = partial_path;
                                }
                                Node::Leaf(ref mut leaf) => {
                                    let partial_path = Path::from_nibbles_iterator(
                                        branch
                                            .partial_path
                                            .iter()
                                            .copied()
                                            .chain(once(child_index as u8))
                                            .chain(leaf.partial_path.iter().copied()),
                                    );
                                    leaf.partial_path = partial_path;
                                }
                            }

                            let node_partial_path =
                                std::mem::replace(&mut branch.partial_path, Path::new());

                            let partial_path = Path::from_nibbles_iterator(
                                branch
                                    .partial_path
                                    .iter()
                                    .chain(once(&(child_index as u8)))
                                    .chain(node_partial_path.iter())
                                    .copied(),
                            );

                            node.update_partial_path(partial_path);

                            Ok((Some(child), Some(removed_value)))
                        }
                    }
                    Node::Leaf(leaf) => {
                        let removed_value = std::mem::take(&mut leaf.value);
                        Ok((None, Some(removed_value)))
                    }
                }
            }
            (Some((child_index, child_partial_path)), None) => {
                // 3. The key is below the node (i.e. its descendant)
                match node {
                    // we found a non-matching leaf node, so the value does not exist
                    Node::Leaf(_) => Ok((Some(node), None)),
                    Node::Branch(ref mut branch) => {
                        #[expect(clippy::indexing_slicing)]
                        let child = match std::mem::take(&mut branch.children[child_index as usize])
                        {
                            None => {
                                return Ok((Some(node), None));
                            }
                            Some(Child::Node(node)) => node,
                            Some(Child::AddressWithHash(addr, _)) => {
                                self.nodestore.read_for_update(addr.into())?
                            }
                            Some(Child::MaybePersisted(maybe_persisted, _)) => {
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        let (child, removed_value) =
                            self.remove_helper(child, child_partial_path.as_ref())?;

                        if let Some(child) = child {
                            branch.update_child(child_index, Some(Child::Node(child)));
                        } else {
                            branch.update_child(child_index, None);
                        }

                        let mut children_iter =
                            branch
                                .children
                                .iter_mut()
                                .enumerate()
                                .filter_map(|(index, child)| {
                                    child.as_mut().map(|child| (index, child))
                                });

                        let Some((child_index, child)) = children_iter.next() else {
                            // The branch has no children. Turn it into a leaf.
                            let leaf = Node::Leaf(LeafNode {
                                    value: branch.value.take().expect(
                                        "branch node must have a value if it previously had only 1 child",
                                    ),
                                    partial_path: branch.partial_path.clone(), // TODO remove clone
                                });
                            return Ok((Some(leaf), removed_value));
                        };

                        // if there is more than one child or the branch has a value, return it
                        if branch.value.is_some() || children_iter.next().is_some() {
                            return Ok((Some(node), removed_value));
                        }

                        // The branch has only 1 child. Remove the branch and return the child.
                        let mut child = match child {
                            Child::Node(child_node) => std::mem::replace(
                                child_node,
                                Node::Leaf(LeafNode {
                                    value: Box::default(),
                                    partial_path: Path::new(),
                                }),
                            ),
                            Child::AddressWithHash(addr, _) => {
                                self.nodestore.read_for_update((*addr).into())?
                            }
                            Child::MaybePersisted(maybe_persisted, _) => {
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        // The child's partial path is the concatenation of its (now removed) parent,
                        // its (former) child index, and its partial path.
                        let child_partial_path = Path::from_nibbles_iterator(
                            branch
                                .partial_path
                                .iter()
                                .chain(once(&(child_index as u8)))
                                .chain(child.partial_path().iter())
                                .copied(),
                        );
                        child.update_partial_path(child_partial_path);

                        Ok((Some(child), removed_value))
                    }
                }
            }
        }
    }

    /// Removes any key-value pairs with keys that have the given `prefix`.
    /// Returns the number of key-value pairs removed.
    pub fn remove_prefix(&mut self, prefix: &[u8]) -> Result<usize, FileIoError> {
        let prefix = Path::from_nibbles_iterator(NibblesIterator::new(prefix));

        let root = self.nodestore.mut_root();
        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. There is nothing to remove.
            counter!("firewood.remove", "prefix" => "true", "result" => "nonexistent").increment(1);
            return Ok(0);
        };

        let mut deleted = 0;
        let root_node = self.remove_prefix_helper(root_node, &prefix, &mut deleted)?;
        counter!("firewood.remove", "prefix" => "true", "result" => "success")
            .increment(deleted as u64);
        *self.nodestore.mut_root() = root_node;
        Ok(deleted)
    }

    fn remove_prefix_helper(
        &self,
        mut node: Node,
        key: &[u8],
        deleted: &mut usize,
    ) -> Result<Option<Node>, FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`, in which case we need to delete this node and all its children.
        // 2. The key is above the node (i.e. its ancestor), so the parent needs to be restructured (TODO).
        // 3. The key is below the node (i.e. its descendant), so continue traversing the trie.
        // 4. Neither is an ancestor of the other, in which case there's no work to do.
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, Path::from(path))),
            unique_node.split_first(),
        ) {
            (None, _) => {
                // 1. The node is at `key`, or we're just above it
                // so we can start deleting below here
                match &mut node {
                    Node::Branch(branch) => {
                        if branch.value.is_some() {
                            // a KV pair was in the branch itself
                            *deleted = deleted.saturating_add(1);
                        }
                        self.delete_children(branch, deleted)?;
                    }
                    Node::Leaf(_) => {
                        // the prefix matched only a leaf, so we remove it and indicate only one item was removed
                        *deleted = deleted.saturating_add(1);
                    }
                }
                Ok(None)
            }
            (_, Some(_)) => {
                // Case (2) or (4)
                Ok(Some(node))
            }
            (Some((child_index, child_partial_path)), None) => {
                // 3. The key is below the node (i.e. its descendant)
                match node {
                    Node::Leaf(_) => Ok(Some(node)),
                    Node::Branch(ref mut branch) => {
                        #[expect(clippy::indexing_slicing)]
                        let child = match std::mem::take(&mut branch.children[child_index as usize])
                        {
                            None => {
                                return Ok(Some(node));
                            }
                            Some(Child::Node(node)) => node,
                            Some(Child::AddressWithHash(addr, _)) => {
                                self.nodestore.read_for_update(addr.into())?
                            }
                            Some(Child::MaybePersisted(maybe_persisted, _)) => {
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        let child =
                            self.remove_prefix_helper(child, child_partial_path.as_ref(), deleted)?;

                        if let Some(child) = child {
                            branch.update_child(child_index, Some(Child::Node(child)));
                        } else {
                            branch.update_child(child_index, None);
                        }

                        let mut children_iter =
                            branch
                                .children
                                .iter_mut()
                                .enumerate()
                                .filter_map(|(index, child)| {
                                    child.as_mut().map(|child| (index, child))
                                });

                        let Some((child_index, child)) = children_iter.next() else {
                            // The branch has no children. Turn it into a leaf.
                            let leaf = Node::Leaf(LeafNode {
                                    value: branch.value.take().expect(
                                        "branch node must have a value if it previously had only 1 child",
                                    ),
                                    partial_path: branch.partial_path.clone(), // TODO remove clone
                                });
                            return Ok(Some(leaf));
                        };

                        // if there is more than one child or the branch has a value, return it
                        if branch.value.is_some() || children_iter.next().is_some() {
                            return Ok(Some(node));
                        }

                        // The branch has only 1 child. Remove the branch and return the child.
                        let mut child = match child {
                            Child::Node(child_node) => std::mem::replace(
                                child_node,
                                Node::Leaf(LeafNode {
                                    value: Box::default(),
                                    partial_path: Path::new(),
                                }),
                            ),
                            Child::AddressWithHash(addr, _) => {
                                self.nodestore.read_for_update((*addr).into())?
                            }
                            Child::MaybePersisted(maybe_persisted, _) => {
                                self.nodestore.read_for_update(maybe_persisted.clone())?
                            }
                        };

                        // The child's partial path is the concatenation of its (now removed) parent,
                        // its (former) child index, and its partial path.
                        let child_partial_path = Path::from_nibbles_iterator(
                            branch
                                .partial_path
                                .iter()
                                .chain(once(&(child_index as u8)))
                                .chain(child.partial_path().iter())
                                .copied(),
                        );
                        child.update_partial_path(child_partial_path);

                        Ok(Some(child))
                    }
                }
            }
        }
    }

    /// Recursively deletes all children of a branch node.
    fn delete_children(
        &self,
        branch: &mut BranchNode,
        deleted: &mut usize,
    ) -> Result<(), FileIoError> {
        if branch.value.is_some() {
            // a KV pair was in the branch itself
            *deleted = deleted.saturating_add(1);
        }
        for children in &mut branch.children {
            // read the child node
            let child = match children {
                Some(Child::Node(node)) => node,
                Some(Child::AddressWithHash(addr, _)) => {
                    &mut self.nodestore.read_for_update((*addr).into())?
                }
                Some(Child::MaybePersisted(maybe_persisted, _)) => {
                    // For MaybePersisted, we need to get the node to update it
                    // We can't get a mutable reference from SharedNode, so we need to handle this differently
                    // For now, we'll skip this child since we can't modify it
                    let _shared_node = maybe_persisted.as_shared_node(&self.nodestore)?;
                    continue;
                }
                None => continue,
            };
            match child {
                Node::Branch(child_branch) => {
                    self.delete_children(child_branch, deleted)?;
                }
                Node::Leaf(_) => {
                    *deleted = deleted.saturating_add(1);
                }
            }
        }
        Ok(())
    }
}

/// Returns an iterator where each element is the result of combining
/// 2 nibbles of `nibbles`. If `nibbles` is odd length, panics in
/// debug mode and drops the final nibble in release mode.
pub fn nibbles_to_bytes_iter(nibbles: &[u8]) -> impl Iterator<Item = u8> {
    debug_assert_eq!(nibbles.len() & 1, 0);
    #[expect(clippy::indexing_slicing)]
    nibbles.chunks_exact(2).map(|p| (p[0] << 4) | p[1])
}

/// The [`PrefixOverlap`] type represents the _shared_ and _unique_ parts of two potentially overlapping slices.
/// As the type-name implies, the `shared` property only constitues a shared *prefix*.
/// The `unique_*` properties, [`unique_a`][`PrefixOverlap::unique_a`] and [`unique_b`][`PrefixOverlap::unique_b`]
/// are set based on the argument order passed into the [`from`][`PrefixOverlap::from`] constructor.
#[derive(Debug)]
struct PrefixOverlap<'a, T> {
    shared: &'a [T],
    unique_a: &'a [T],
    unique_b: &'a [T],
}

impl<'a, T: PartialEq> PrefixOverlap<'a, T> {
    fn from(a: &'a [T], b: &'a [T]) -> Self {
        let split_index = a
            .iter()
            .zip(b)
            .position(|(a, b)| *a != *b)
            .unwrap_or_else(|| std::cmp::min(a.len(), b.len()));

        let (shared, unique_a) = a.split_at(split_index);
        let unique_b = b.get(split_index..).expect("");

        Self {
            shared,
            unique_a,
            unique_b,
        }
    }
}
