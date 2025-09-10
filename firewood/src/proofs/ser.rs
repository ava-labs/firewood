// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::ValueDigest;
use integer_encoding::VarInt;

use crate::{
    proof::ProofNode,
    proofs::marshaling::{ChildrenMap, Header, ProofType},
    v2::api::FrozenRangeProof,
};

impl FrozenRangeProof {
    /// Serializes this proof into the provided byte vector.
    pub fn write_to_vec(&self, out: &mut Vec<u8>) {
        Header::new(ProofType::Range).write_item(out);
        self.write_item(out);
    }
}

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

trait WriteItem {
    fn write_item(&self, out: &mut Vec<u8>);
}

impl WriteItem for FrozenRangeProof {
    fn write_item(&self, out: &mut Vec<u8>) {
        self.start_proof().write_item(out);
        self.end_proof().write_item(out);
        self.key_values().write_item(out);
    }
}

impl WriteItem for ProofNode {
    fn write_item(&self, out: &mut Vec<u8>) {
        self.key.write_item(out);
        out.push_var_int(self.partial_len);
        self.value_digest.write_item(out);
        ChildrenMap::new(&self.child_hashes).write_item(out);
        for child in self.child_hashes.iter().flatten() {
            child.write_item(out);
        }
    }
}

impl<T: WriteItem> WriteItem for Option<T> {
    fn write_item(&self, out: &mut Vec<u8>) {
        if let Some(v) = self {
            out.push(1);
            v.write_item(out);
        } else {
            out.push(0);
        }
    }
}

impl<T: WriteItem> WriteItem for [T] {
    fn write_item(&self, out: &mut Vec<u8>) {
        out.push_var_int(self.len());
        for item in self {
            item.write_item(out);
        }
    }
}

impl WriteItem for [u8] {
    fn write_item(&self, out: &mut Vec<u8>) {
        out.push_var_int(self.len());
        out.extend_from_slice(self);
    }
}

impl<T: AsRef<[u8]>> WriteItem for ValueDigest<T> {
    fn write_item(&self, out: &mut Vec<u8>) {
        match self.make_hash() {
            ValueDigest::Value(v) => {
                out.push(0);
                v.write_item(out);
            }
            #[cfg(not(feature = "ethhash"))]
            ValueDigest::Hash(h) => {
                out.push(1);
                h.write_item(out);
            }
        }
    }
}

impl WriteItem for Header {
    fn write_item(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(bytemuck::bytes_of(self));
    }
}

impl WriteItem for firewood_storage::TrieHash {
    fn write_item(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_ref());
    }
}

#[cfg(feature = "ethhash")]
impl WriteItem for firewood_storage::HashType {
    fn write_item(&self, out: &mut Vec<u8>) {
        match self {
            firewood_storage::HashType::Hash(h) => {
                out.push(0);
                h.write_item(out);
            }
            firewood_storage::HashType::Rlp(h) => {
                out.push(1);
                h.write_item(out);
            }
        }
    }
}

impl WriteItem for ChildrenMap {
    fn write_item(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(bytemuck::bytes_of(self));
    }
}

impl<K: AsRef<[u8]>, V: AsRef<[u8]>> WriteItem for (K, V) {
    fn write_item(&self, out: &mut Vec<u8>) {
        let (key, value) = self;
        key.as_ref().write_item(out);
        value.as_ref().write_item(out);
    }
}
