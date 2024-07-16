// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::Debug;
use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use crate::{LinearAddress, Node};

use super::{NodeStoreParent, ReadChangedNode};

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
    pub new_nodes: HashMap<LinearAddress, (Arc<Node>, u8)>,
    pub(crate) parent: Arc<NodeStoreParent>,
    mutability: PhantomData<T>,
}

impl<T: Debug + Send + Sync> ReadChangedNode for Proposed<T> {
    fn read_changed_node(&self, addr: LinearAddress) -> Option<Arc<Node>> {
        if let Some((node, _)) = self.new_nodes.get(&addr) {
            Some(node.clone())
        } else {
            match self.parent.as_ref() {
                NodeStoreParent::Proposed(parent) => parent.read_changed_node(addr),
                NodeStoreParent::Committed(_) => None,
            }
        }
    }
}

impl Proposed<Mutable> {
    pub(crate) fn new(parent: Arc<NodeStoreParent>) -> Self {
        Proposed {
            new_nodes: Default::default(),
            parent: parent.clone(),
            mutability: Default::default(),
        }
    }
    /// Freeze a mutable proposal, consuming it, and making it immutable
    pub fn freeze(self) -> ProposedImmutable {
        Proposed {
            new_nodes: self.new_nodes,
            parent: self.parent,
            mutability: Default::default(),
        }
    }
}
