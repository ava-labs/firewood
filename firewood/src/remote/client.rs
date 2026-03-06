// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Client-side remote storage access with proof verification.
//!
//! The [`RemoteClient`] wraps a [`TruncatedTrie`] and a [`RemoteTransport`]
//! implementation. Every read is verified against the truncated trie's root
//! hash using Merkle inclusion/exclusion proofs.

use crate::proofs::types::ProofError;
use crate::remote::TruncatedTrie;
use crate::v2::api::FrozenProof;
use firewood_storage::TrieHash;

/// A value with its Merkle proof for verification.
pub type ValueWithProof = (Option<Box<[u8]>>, FrozenProof);

/// Transport trait for communicating with a remote Firewood server.
///
/// Implementors provide the actual network communication (e.g., gRPC).
pub trait RemoteTransport {
    /// The error type returned by transport operations.
    type Error: std::error::Error;

    /// Fetches a value and its proof for the given key from the server.
    ///
    /// Returns `(value, proof)` where `value` is `None` if the key does not
    /// exist, and `proof` is a single-key Merkle proof.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport operation fails.
    fn get_value(&self, key: &[u8]) -> Result<ValueWithProof, Self::Error>;

    /// Sends a batch of operations to the server to create a proposal.
    ///
    /// Returns a proposal ID and the new root hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport operation fails.
    fn create_proposal(&self, batch_ops: &[BatchOp])
    -> Result<(ProposalId, TrieHash), Self::Error>;

    /// Commits a proposal on the server.
    ///
    /// Returns the witness proof bytes for client-side verification.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport operation fails.
    fn commit_proposal(&self, proposal_id: ProposalId) -> Result<Box<[u8]>, Self::Error>;

    /// Fetches a truncated trie from the server for bootstrapping.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport operation fails.
    fn get_truncated_trie(
        &self,
        root_hash: &TrieHash,
        depth: usize,
    ) -> Result<TruncatedTrie, Self::Error>;
}

/// A batch operation to be applied to the trie.
#[derive(Debug, Clone)]
pub enum BatchOp {
    /// Insert or update a key-value pair.
    Put {
        /// The key to insert.
        key: Box<[u8]>,
        /// The value to associate with the key.
        value: Box<[u8]>,
    },
    /// Delete a key.
    Delete {
        /// The key to delete.
        key: Box<[u8]>,
    },
}

/// An opaque identifier for a proposal on the server.
pub type ProposalId = u64;

/// Error type for remote client operations.
#[derive(Debug, thiserror::Error)]
pub enum RemoteClientError<T: std::error::Error> {
    /// The proof verification failed.
    #[error("proof verification failed: {0}")]
    ProofError(#[from] ProofError),

    /// A transport-level error occurred.
    #[error("transport error: {0}")]
    TransportError(T),

    /// The client has no root hash (not yet bootstrapped).
    #[error("client not bootstrapped: no root hash available")]
    NotBootstrapped,
}

/// A client that reads from a remote Firewood server with proof verification.
///
/// The client holds a [`TruncatedTrie`] with the top K levels of the trie and
/// verifies every read against the root hash using Merkle proofs.
#[derive(Debug)]
pub struct RemoteClient<T: RemoteTransport> {
    trie: TruncatedTrie,
    transport: T,
}

impl<T: RemoteTransport> RemoteClient<T> {
    /// Creates a new `RemoteClient` with the given truncated trie and transport.
    pub const fn new(trie: TruncatedTrie, transport: T) -> Self {
        Self { trie, transport }
    }

    /// Returns a reference to the underlying truncated trie.
    pub const fn trie(&self) -> &TruncatedTrie {
        &self.trie
    }

    /// Returns the current root hash, if the client has been bootstrapped.
    pub const fn root_hash(&self) -> Option<&TrieHash> {
        self.trie.root_hash()
    }

    /// Fetches a value for the given key, verifying it against the root hash.
    ///
    /// 1. Calls the transport to get the value and proof from the server
    /// 2. Verifies the proof against the client's trusted root hash
    /// 3. Returns the verified value
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The client has not been bootstrapped (no root hash)
    /// - The transport fails
    /// - The proof verification fails
    pub fn get(&self, key: &[u8]) -> Result<Option<Box<[u8]>>, RemoteClientError<T::Error>> {
        let root_hash = self
            .trie
            .root_hash()
            .ok_or(RemoteClientError::NotBootstrapped)?;

        let (value, proof) = self
            .transport
            .get_value(key)
            .map_err(RemoteClientError::TransportError)?;

        // Verify the proof against our trusted root hash
        proof.verify(key, value.as_deref(), root_hash)?;

        Ok(value)
    }

    /// Bootstraps the client from a remote server using a trusted root hash.
    ///
    /// 1. Fetches the truncated trie from the server
    /// 2. Verifies that the trie's root hash matches the trusted hash
    /// 3. Stores the verified trie
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transport fails
    /// - The trie's root hash doesn't match the trusted hash
    pub fn bootstrap(
        &mut self,
        trusted_root_hash: &TrieHash,
        depth: usize,
    ) -> Result<(), RemoteClientError<T::Error>> {
        let trie = self
            .transport
            .get_truncated_trie(trusted_root_hash, depth)
            .map_err(RemoteClientError::TransportError)?;

        if !trie.verify_root_hash(trusted_root_hash) {
            return Err(RemoteClientError::ProofError(ProofError::UnexpectedHash));
        }

        self.trie = trie;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;
    use crate::merkle::Merkle;
    use firewood_storage::{HashedNodeReader, ImmutableProposal, MemStore, NodeStore};
    use std::sync::Arc;

    /// A test transport that wraps a real Merkle trie and generates proofs.
    struct TestTransport {
        merkle: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
    }

    #[derive(Debug)]
    struct TestError(String);
    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TestError: {}", self.0)
        }
    }
    impl std::error::Error for TestError {}

    impl RemoteTransport for TestTransport {
        type Error = TestError;

        fn get_value(&self, key: &[u8]) -> Result<ValueWithProof, TestError> {
            let proof = self
                .merkle
                .prove(key)
                .map_err(|e| TestError(e.to_string()))?;

            // Get the actual value
            let value = self
                .merkle
                .get_value(key)
                .map_err(|e| TestError(e.to_string()))?;

            Ok((value, proof))
        }

        fn create_proposal(
            &self,
            _batch_ops: &[BatchOp],
        ) -> Result<(ProposalId, TrieHash), TestError> {
            Err(TestError("not implemented".into()))
        }

        fn commit_proposal(&self, _proposal_id: ProposalId) -> Result<Box<[u8]>, TestError> {
            Err(TestError("not implemented".into()))
        }

        fn get_truncated_trie(
            &self,
            _root_hash: &TrieHash,
            depth: usize,
        ) -> Result<TruncatedTrie, TestError> {
            TruncatedTrie::from_trie(self.merkle.nodestore(), depth)
                .map_err(|e| TestError(e.to_string()))
        }
    }

    fn create_test_transport(keys: &[(&[u8], &[u8])]) -> TestTransport {
        let memstore = MemStore::default();
        let nodestore = NodeStore::new_empty_proposal(Arc::new(memstore));
        let mut merkle = Merkle::from(nodestore);
        for (key, value) in keys {
            merkle
                .insert(key, value.to_vec().into_boxed_slice())
                .unwrap();
        }
        let hashed: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
            merkle.try_into().unwrap();
        TestTransport { merkle: hashed }
    }

    #[test]
    fn test_get_existing_key() {
        let transport = create_test_transport(&[(b"apple", b"red"), (b"banana", b"yellow")]);
        let trie = TruncatedTrie::from_trie(transport.merkle.nodestore(), 4).unwrap();
        let client = RemoteClient::new(trie, transport);

        let value = client.get(b"apple").unwrap();
        assert_eq!(value.as_deref(), Some(b"red".as_slice()));

        let value = client.get(b"banana").unwrap();
        assert_eq!(value.as_deref(), Some(b"yellow".as_slice()));
    }

    #[test]
    fn test_get_nonexistent_key() {
        let transport = create_test_transport(&[(b"apple", b"red")]);

        let trie = TruncatedTrie::from_trie(transport.merkle.nodestore(), 4).unwrap();
        let client = RemoteClient::new(trie, transport);

        let value = client.get(b"cherry").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn test_get_without_bootstrap_fails() {
        let transport = create_test_transport(&[(b"apple", b"red")]);
        let client = RemoteClient::new(TruncatedTrie::new(4), transport);

        let result = client.get(b"apple");
        assert!(matches!(result, Err(RemoteClientError::NotBootstrapped)));
    }

    #[test]
    fn test_bootstrap() {
        let transport = create_test_transport(&[(b"key", b"value")]);
        let root_hash = transport.merkle.nodestore().root_hash().unwrap();

        let mut client = RemoteClient::new(TruncatedTrie::new(4), transport);
        assert!(client.root_hash().is_none());

        client.bootstrap(&root_hash, 4).unwrap();
        assert_eq!(*client.root_hash().unwrap(), root_hash);

        // Should now be able to read
        let value = client.get(b"key").unwrap();
        assert_eq!(value.as_deref(), Some(b"value".as_slice()));
    }

    #[test]
    fn test_bootstrap_with_wrong_hash_fails() {
        let transport = create_test_transport(&[(b"key", b"value")]);
        let wrong_hash = TrieHash::empty();

        let mut client = RemoteClient::new(TruncatedTrie::new(4), transport);
        let result = client.bootstrap(&wrong_hash, 4);
        assert!(matches!(result, Err(RemoteClientError::ProofError(_))));
    }
}
