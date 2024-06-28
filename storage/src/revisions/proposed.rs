// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::HashMap, marker::PhantomData};

use crate::{LinearAddress, Node, NodeStore};

use super::{filebacked::FileBacked, NodeStoreParent};

#[derive(Debug)]
pub struct Immutable;
#[derive(Debug)]
pub struct Mutable;
/// A shortcut for a [`Proposed<Mutable>`]
pub type ProposedMutable = Proposed<Mutable>;
/// A shortcut for a [`Proposed<Immutable>`]
pub type ProposedImmutable = Proposed<Immutable>;

#[derive(Debug)]
pub struct Proposed<T> {
    node_store: NodeStore<FileBacked>,
    new_nodes: HashMap<LinearAddress, Node>,
    parent: Box<NodeStoreParent>,
    state: PhantomData<T>,
}

impl ProposedMutable {
    /// Freeze a mutable proposal, consuming it, and making it immutable
    pub fn freeze(self) -> ProposedImmutable {
        Proposed {
            node_store: self.node_store,
            new_nodes: self.new_nodes,
            parent: self.parent,
            state: Default::default(),
        }
    }
}
