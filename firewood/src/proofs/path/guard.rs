// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Nibbles;

pub(crate) struct PathGuard<'a> {
    nibbles: &'a mut Vec<u8>,
    reset_len: usize,
}

impl Drop for PathGuard<'_> {
    fn drop(&mut self) {
        self.nibbles.truncate(self.reset_len);
    }
}

impl<'a> PathGuard<'a> {
    pub const fn new(nibbles: &'a mut Vec<u8>) -> Self {
        let reset_len = nibbles.len();
        Self { nibbles, reset_len }
    }

    pub const fn fork(&mut self) -> PathGuard<'_> {
        PathGuard::new(self.nibbles)
    }

    pub fn fork_push(&mut self, nibbles: impl Nibbles) -> PathGuard<'_> {
        let mut this = self.fork();
        this.extend(nibbles.nibbles_iter());
        this
    }
}

impl std::ops::Deref for PathGuard<'_> {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        self.nibbles
    }
}

impl std::ops::DerefMut for PathGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.nibbles
    }
}

impl Nibbles for PathGuard<'_> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.nibbles.iter().copied()
    }

    fn len(&self) -> usize {
        self.nibbles.len()
    }

    fn is_empty(&self) -> bool {
        self.nibbles.is_empty()
    }
}
