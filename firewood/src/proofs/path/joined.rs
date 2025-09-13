// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Nibbles;

#[derive(Clone, Copy)]
pub(crate) struct JoinedPath<A, B> {
    a: A,
    b: B,
}

impl<T: Nibbles, U: Nibbles> JoinedPath<T, U> {
    pub(super) const fn new(a: T, b: U) -> Self {
        Self { a, b }
    }
}

impl<T: Nibbles, U: Nibbles> Nibbles for JoinedPath<T, U> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.a.nibbles_iter().chain(self.b.nibbles_iter())
    }

    fn len(&self) -> usize {
        self.a.len().saturating_add(self.b.len())
    }

    fn is_empty(&self) -> bool {
        self.a.is_empty() && self.b.is_empty()
    }
}
