// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::v2::api::{self, ArcDynDbView, HashKey};
use metrics::counter;

use crate::DatabaseHandle;

impl DatabaseHandle<'_> {
    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        // construct outside of the closure so that when the closure is dropped
        // without executing, it triggers the hit counter
        let token = CachedViewHitOnDrop;
        self.cached_view.get_or_try_insert_with(root, move |key| {
            token.miss();
            self.db.view(HashKey::clone(key))
        })
    }
}

/// A RAII metrics helper that tracks cache hits and misses for database views.
///
/// This type uses the drop pattern to automatically record cache metrics:
/// - By default, dropping this type records a cache hit
/// - Calling [`Self::miss`] before dropping records a cache miss instead
///
/// This ensures that every cache lookup is properly tracked in metrics without
/// requiring manual instrumentation at each call site.
///
/// # Metrics Recorded
///
/// - `firewood.ffi.cached_view.hit` - Incremented when the cache lookup succeeds
/// - `firewood.ffi.cached_view.miss` - Incremented when the cache lookup fails
struct CachedViewHitOnDrop;

impl Drop for CachedViewHitOnDrop {
    fn drop(&mut self) {
        counter!("firewood.ffi.cached_view.hit").increment(1);
    }
}

impl CachedViewHitOnDrop {
    pub fn miss(self) {
        std::mem::forget(self);
        counter!("firewood.ffi.cached_view.miss").increment(1);
    }
}
