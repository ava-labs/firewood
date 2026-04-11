// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{MemStore, Mutable, NodeStore, Propose, ValueDigest};

use crate::{ProofError, ProofNode, Value, merkle::Merkle};

/// Reconciles a branch proof node against the in-memory proving merkle.
///
/// This helper never overwrites an existing branch value. It only
/// creates missing branch structure and inserts a value when the
/// branch exists without one.
///
/// ## Arguments
///
/// * `proof_node` - A branch proof node containing the key (as nibble
///   path components) and an optional value digest to reconcile.
///
/// ## Errors
///
/// Returns an error if an existing value conflicts with the proof.
/// Reconciles a proof node's value against the proving trie.
///
/// When the proof's value and the trie's value conflict (different values,
/// or one has a value and the other doesn't), `on_conflict` is called with
/// the proof node. It returns the value to store (`Some`) or `None` to
/// clear the value. If the conflict cannot be resolved, it returns `Err`.
impl Merkle<NodeStore<Mutable<Propose>, MemStore>> {
    pub(crate) fn reconcile_branch_proof_node(
        &mut self,
        proof_node: &ProofNode,
        on_conflict: impl FnOnce(&ProofNode) -> Result<Option<Value>, ProofError>,
    ) -> Result<(), ProofError> {
        let key_nibbles: Box<[u8]> = proof_node
            .key
            .iter()
            .map(|component| component.as_u8())
            .collect();

        if !key_nibbles.len().is_multiple_of(2)
            && matches!(proof_node.value_digest, Some(ValueDigest::Value(_)))
        {
            return Err(ProofError::ValueAtOddNibbleLength);
        }

        self.insert_branch_from_nibbles(&key_nibbles)?;

        // insert_branch_from_nibbles guarantees a branch exists at this path
        // and that all nodes along it are in-memory, so this cannot fail.
        let branch = Self::get_branch_from_nibbles_mut(self.nodestore.root_mut(), &proof_node.key)
            .ok_or(ProofError::NodeNotInTrie)?;

        let proof_value = match proof_node.value_digest.as_ref() {
            Some(ValueDigest::Value(v)) => Some(v.as_ref()),
            _ => None,
        };

        if proof_value == branch.value.as_deref() {
            return Ok(());
        }

        // Values differ — let the caller decide what to do.
        branch.value = on_conflict(proof_node)?;
        Ok(())
    }
}
