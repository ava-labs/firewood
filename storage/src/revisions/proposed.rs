// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use crate::{LinearAddress, Node};

use super::NodeStoreParent;

#[derive(Debug)]
pub struct Immutable;
#[derive(Debug)]
pub struct Mutable ;
/// A shortcut for a [`Proposed<Mutable>`]
pub type ProposedMutable = Proposed<Mutable>;
/// A shortcut for a [`Proposed<Immutable>`]
pub type ProposedImmutable = Proposed<Immutable>;

#[derive(Debug)]
pub struct Proposed<T> {
    new_nodes: HashMap<LinearAddress, Node>,
    parent: Arc<NodeStoreParent>,
    mutability: PhantomData<T>,
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
