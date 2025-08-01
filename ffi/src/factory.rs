// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub trait ValueFactory<K, V, E> {
    fn create(self, key: &K) -> Result<V, E>;
}

impl<F, K, V, E> ValueFactory<K, V, E> for F
where
    F: FnOnce(&K) -> Result<V, E>,
{
    fn create(self, key: &K) -> Result<V, E> {
        self(key)
    }
}
