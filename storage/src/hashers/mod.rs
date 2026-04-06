// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod ethhash;
mod merkledb;

use crate::hashednode::{HasUpdate, Hashable};
use crate::{HashType, NodeHashAlgorithm};

pub(crate) fn write_preimage<T: Hashable>(
    node_hash_algorithm: NodeHashAlgorithm,
    hashable: &T,
    buf: &mut impl HasUpdate,
) {
    match node_hash_algorithm {
        NodeHashAlgorithm::MerkleDB => merkledb::write(hashable, buf),
        NodeHashAlgorithm::Ethereum => ethhash::write(hashable, buf),
    }
}

#[must_use]
pub(crate) fn to_hash<T: Hashable>(
    node_hash_algorithm: NodeHashAlgorithm,
    hashable: &T,
) -> HashType {
    match node_hash_algorithm {
        NodeHashAlgorithm::MerkleDB => merkledb::to_hash(hashable),
        NodeHashAlgorithm::Ethereum => ethhash::to_hash(hashable),
    }
}
