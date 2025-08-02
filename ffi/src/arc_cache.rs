// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::sync::{Arc, Mutex};

use crate::factory::ValueFactory;

#[derive(Debug)]
pub struct ArcCache<K, V: ?Sized> {
    cache: Mutex<Option<(K, Arc<V>)>>,
}

impl<K: PartialEq, V: ?Sized> ArcCache<K, V> {
    pub const fn new() -> Self {
        ArcCache {
            cache: Mutex::new(None),
        }
    }

    pub fn get_or_try_insert_with<E>(
        &self,
        key: K,
        factory: impl FnOnce(&K) -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_key, value)) = cache.as_ref() {
            if *cached_key == key {
                return Ok(Arc::clone(value));
            }
        }

        // clear the cache before running the factory in case it fails
        *cache = None;

        let value = factory.create(&key)?;
        *cache = Some((key, Arc::clone(&value)));

        Ok(value)
    }

    pub fn clear(&self) {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

impl<K: PartialEq, T: ?Sized> Default for ArcCache<K, T> {
    fn default() -> Self {
        Self::new()
    }
}
