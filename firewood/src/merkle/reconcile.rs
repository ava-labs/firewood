// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{MemStore, Mutable, NodeStore, Propose, ValueDigest};

use crate::{ProofError, ProofNode, Value, merkle::Merkle};

/// Reconciles a branch proof node against the in-memory proving merkle.
///
/// This helper creates any missing branch structure for the proof node's
/// key, then reconciles the branch value with the proof node's value.
/// If the values already match, it returns successfully without changes.
/// If they differ (including when one side has a value and the other
/// does not), `on_conflict` is invoked to decide the final branch value.
///
/// ## Arguments
///
/// * `proof_node` - A branch proof node containing the key (as nibble
///   path components) and an optional value digest to reconcile.
/// * `on_conflict` - Called only when the proof node's value and the
///   existing branch value conflict. Returning `Ok(Some(value))` stores
///   that value in the branch, `Ok(None)` clears the branch value, and
///   `Err(...)` aborts reconciliation with that error.
///
/// ## Errors
///
/// Returns an error if the proof is structurally invalid or if
/// `on_conflict` returns `Err` while resolving a value conflict.
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
