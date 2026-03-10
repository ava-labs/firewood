// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Serialization for remote storage types.
//!
//! Provides `write_to_vec` and `from_slice` for [`TruncatedTrie`],
//! enabling wire transport over gRPC.
//!
//! Format uses variable-length integers (LEB128) and is versioned.

use firewood_storage::{
    BranchNode, Child, Children, HashType, LeafNode, MaybePersistedNode, Node, Path, SharedNode,
    TrieHash,
};
use integer_encoding::VarInt;

use super::truncated_trie::TruncatedTrie;
use crate::proofs::reader::{ProofReader, ReadError};

/// Magic bytes for truncated trie: `b"fwdtrtri"`
const TRIE_MAGIC: &[u8; 8] = b"fwdtrtri";
/// Current version
const VERSION: u8 = 0;

// -- Node type tags --
const NODE_BRANCH: u8 = 0;
const NODE_LEAF: u8 = 1;

// -- Child type tags --
const CHILD_NONE: u8 = 0;
const CHILD_PROXY: u8 = 1;
const CHILD_NODE: u8 = 2;
const CHILD_MAYBE_PERSISTED: u8 = 3;

// ==================== Helper traits ====================

trait PushVarInt {
    fn push_var_int<VI: VarInt>(&mut self, v: VI);
}

impl PushVarInt for Vec<u8> {
    fn push_var_int<VI: VarInt>(&mut self, v: VI) {
        let mut buf = [0u8; 10];
        let n = v.encode_var(&mut buf);
        #[expect(clippy::indexing_slicing)]
        self.extend_from_slice(&buf[..n]);
    }
}

fn write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    out.push_var_int(data.len());
    out.extend_from_slice(data);
}

fn write_hash_type(out: &mut Vec<u8>, hash: &HashType) {
    #[cfg(not(feature = "ethhash"))]
    {
        // Without ethhash, HashType is just TrieHash
        out.extend_from_slice(hash.as_ref());
    }
    #[cfg(feature = "ethhash")]
    match hash {
        HashType::Hash(h) => {
            out.push(0);
            out.extend_from_slice(h.as_ref());
        }
        HashType::Rlp(r) => {
            out.push(1);
            write_bytes(out, r);
        }
    }
}

fn read_hash_type(reader: &mut ProofReader<'_>) -> Result<HashType, ReadError> {
    #[cfg(not(feature = "ethhash"))]
    {
        // Without ethhash, HashType is just TrieHash
        let hash = reader.read_item::<TrieHash>()?;
        Ok(hash)
    }
    #[cfg(feature = "ethhash")]
    {
        match reader.read_item::<u8>()? {
            0 => {
                let hash = reader.read_item::<TrieHash>()?;
                Ok(HashType::Hash(hash))
            }
            1 => {
                let data = reader.read_item::<&[u8]>()?;
                Ok(HashType::Rlp(data.into()))
            }
            found => {
                Err(reader.invalid_item("hash type discriminant", "0 (hash) or 1 (rlp)", found))
            }
        }
    }
}

// ==================== Node serialization ====================

fn write_node(out: &mut Vec<u8>, node: &Node) {
    match node {
        Node::Branch(branch) => {
            out.push(NODE_BRANCH);
            write_branch(out, branch);
        }
        Node::Leaf(leaf) => {
            out.push(NODE_LEAF);
            write_leaf(out, leaf);
        }
    }
}

fn write_branch(out: &mut Vec<u8>, branch: &BranchNode) {
    // Partial path
    write_bytes(out, &branch.partial_path);
    // Value
    if let Some(ref value) = branch.value {
        out.push(1);
        write_bytes(out, value);
    } else {
        out.push(0);
    }
    // Children: write each child slot
    for (_idx, child_opt) in &branch.children {
        write_child(out, child_opt.as_ref());
    }
}

fn write_child(out: &mut Vec<u8>, child: Option<&Child>) {
    match child {
        None => out.push(CHILD_NONE),
        Some(Child::Proxy(hash)) => {
            out.push(CHILD_PROXY);
            write_hash_type(out, hash);
        }
        Some(Child::Node(node)) => {
            out.push(CHILD_NODE);
            write_node(out, node);
        }
        Some(Child::MaybePersisted(maybe, hash)) => {
            // Serialize as CHILD_MAYBE_PERSISTED: hash + inline node
            out.push(CHILD_MAYBE_PERSISTED);
            write_hash_type(out, hash);
            // Unwrap the MaybePersisted to get the node.
            // For truncated tries, these are always Unpersisted.
            if let Ok(shared) = maybe.as_unpersisted_node() {
                write_node(out, &shared);
            } else {
                // Fallback: write an empty leaf if we can't read the node.
                // This shouldn't happen for truncated tries.
                out.push(NODE_LEAF);
                write_bytes(out, &[]);
                write_bytes(out, &[]);
            }
        }
        Some(Child::AddressWithHash(_, hash)) => {
            // Can't serialize address-based children; downgrade to Proxy
            out.push(CHILD_PROXY);
            write_hash_type(out, hash);
        }
    }
}

fn write_leaf(out: &mut Vec<u8>, leaf: &LeafNode) {
    write_bytes(out, &leaf.partial_path);
    write_bytes(out, &leaf.value);
}

fn read_node(reader: &mut ProofReader<'_>) -> Result<Node, ReadError> {
    match reader
        .read_item::<u8>()
        .map_err(|err| err.set_item("node type tag"))?
    {
        NODE_BRANCH => read_branch(reader).map(|b| Node::Branch(Box::new(b))),
        NODE_LEAF => read_leaf(reader).map(Node::Leaf),
        found => Err(reader.invalid_item("node type tag", "0 (branch) or 1 (leaf)", found)),
    }
}

fn read_path(reader: &mut ProofReader<'_>) -> Result<Path, ReadError> {
    let path_bytes = reader.read_item::<&[u8]>()?;
    if path_bytes.iter().any(|&b| b > 0x0F) {
        return Err(reader.invalid_item(
            "path nibble",
            "bytes in range 0x00..=0x0F",
            "byte > 0x0F",
        ));
    }
    Ok(Path::from(path_bytes))
}

fn read_branch(reader: &mut ProofReader<'_>) -> Result<BranchNode, ReadError> {
    // Partial path
    let partial_path = read_path(reader)?;

    // Value
    let has_value = reader.read_item::<u8>()?;
    let value = if has_value == 1 {
        Some(reader.read_item::<Box<[u8]>>()?)
    } else {
        None
    };

    // Children (16 slots for branch factor 16)
    let mut children = Children::new();
    for (_idx, child_slot) in &mut children {
        *child_slot = read_child(reader)?;
    }

    Ok(BranchNode {
        partial_path,
        value,
        children,
    })
}

fn read_child(reader: &mut ProofReader<'_>) -> Result<Option<Child>, ReadError> {
    match reader
        .read_item::<u8>()
        .map_err(|err| err.set_item("child type tag"))?
    {
        CHILD_NONE => Ok(None),
        CHILD_PROXY => {
            let hash = read_hash_type(reader)?;
            Ok(Some(Child::Proxy(hash)))
        }
        CHILD_NODE => {
            let node = read_node(reader)?;
            Ok(Some(Child::Node(node)))
        }
        CHILD_MAYBE_PERSISTED => {
            let hash = read_hash_type(reader)?;
            let node = read_node(reader)?;
            let shared = SharedNode::new(node);
            let maybe = MaybePersistedNode::from(shared);
            Ok(Some(Child::MaybePersisted(maybe, hash)))
        }
        found => Err(reader.invalid_item(
            "child type tag",
            "0 (none), 1 (proxy), 2 (node), or 3 (maybe_persisted)",
            found,
        )),
    }
}

fn read_leaf(reader: &mut ProofReader<'_>) -> Result<LeafNode, ReadError> {
    let partial_path = read_path(reader)?;
    let value = reader.read_item::<Box<[u8]>>()?;

    Ok(LeafNode {
        partial_path,
        value,
    })
}

// ==================== TruncatedTrie ====================

impl TruncatedTrie {
    /// Serializes the truncated trie into the provided byte vector.
    ///
    /// # Format
    ///
    /// - 8 bytes: magic `b"fwdtrtri"`
    /// - 1 byte: version (0)
    /// - varint: `truncation_depth`
    /// - 1 byte: `has_root` (0 or 1)
    /// - If `has_root`:
    ///   - 32 bytes: `root_hash`
    ///   - serialized root node tree
    pub fn write_to_vec(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(TRIE_MAGIC);
        out.push(VERSION);
        out.push_var_int(self.truncation_depth());

        match (self.root_hash(), self.root()) {
            (Some(hash), Some(root)) => {
                out.push(1);
                out.extend_from_slice(hash.as_ref());
                write_node(out, root);
            }
            _ => {
                out.push(0);
            }
        }
    }

    /// Deserializes a truncated trie from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] if the data is invalid.
    pub fn from_slice(data: &[u8]) -> Result<Self, ReadError> {
        let mut reader = ProofReader::new(data);

        // Magic
        let magic = reader
            .read_chunk::<8>()
            .map_err(|err| err.set_item("trie magic"))?;
        if magic != TRIE_MAGIC {
            return Err(reader.invalid_item("trie magic", "b\"fwdtrtri\"", format!("{magic:?}")));
        }

        // Version
        let version = reader.read_item::<u8>()?;
        if version != VERSION {
            return Err(reader.invalid_item("trie version", "0", version));
        }

        // Truncation depth
        let truncation_depth = reader
            .read_item::<usize>()
            .map_err(|err| err.set_item("truncation depth"))?;

        // Has root?
        let has_root = reader.read_item::<u8>()?;
        if has_root == 0 {
            return if reader.remainder().is_empty() {
                Ok(TruncatedTrie::new(truncation_depth))
            } else {
                Err(reader.invalid_item(
                    "trailing bytes",
                    "no data after empty trie",
                    format!("{} bytes", reader.remainder().len()),
                ))
            };
        }

        // Root hash
        let root_hash = reader.read_item::<TrieHash>()?;

        // Root node
        let root = read_node(&mut reader)?;

        if !reader.remainder().is_empty() {
            return Err(reader.invalid_item(
                "trailing bytes",
                "no data after trie",
                format!("{} bytes", reader.remainder().len()),
            ));
        }

        Ok(TruncatedTrie::from_parts(
            Some(root_hash),
            Some(root),
            truncation_depth,
        ))
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;
    use crate::merkle::Merkle;
    use firewood_storage::{HashedNodeReader, ImmutableProposal, MemStore, NodeStore};
    use std::sync::Arc;

    type ImmutableMerkle = Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>;

    fn create_test_trie(keys: &[(&[u8], &[u8])]) -> ImmutableMerkle {
        let memstore = MemStore::default();
        let nodestore = NodeStore::new_empty_proposal(Arc::new(memstore));
        let mut merkle = Merkle::from(nodestore);
        for (key, value) in keys {
            merkle
                .insert(key, value.to_vec().into_boxed_slice())
                .unwrap();
        }
        merkle.try_into().unwrap()
    }

    #[test]
    fn test_truncated_trie_roundtrip() {
        let trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
            (b"cherry", b"dark"),
            (b"date", b"brown"),
            (b"elderberry", b"purple"),
        ]);
        let expected_hash = trie.nodestore().root_hash().unwrap();

        for depth in [1, 2, 4] {
            let truncated = TruncatedTrie::from_trie(trie.nodestore(), depth).unwrap();

            let mut buf = Vec::new();
            truncated.write_to_vec(&mut buf);
            assert!(!buf.is_empty());

            let deserialized = TruncatedTrie::from_slice(&buf).unwrap();

            assert_eq!(deserialized.truncation_depth(), depth);
            assert_eq!(*deserialized.root_hash().unwrap(), expected_hash);
            assert!(deserialized.verify_root_hash(&expected_hash));
        }
    }

    #[test]
    fn test_truncated_trie_empty_roundtrip() {
        let truncated = TruncatedTrie::new(4);

        let mut buf = Vec::new();
        truncated.write_to_vec(&mut buf);

        let deserialized = TruncatedTrie::from_slice(&buf).unwrap();

        assert_eq!(deserialized.truncation_depth(), 4);
        assert!(deserialized.root_hash().is_none());
        assert!(deserialized.root().is_none());
    }
}
