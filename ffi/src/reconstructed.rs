// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::v2::api::{
    self, BoxKeyValueIter, DbView, HashKey, IntoBatchIter, Reconstructible as _,
};
use firewood_storage::BranchNode;
use rayon::ThreadPoolBuilder;

use crate::{IteratorHandle, iterator::CreateIteratorResult, metrics::MetricsContextExt};

/// An opaque wrapper around a reconstructed view.
#[derive(Debug)]
pub struct ReconstructedHandle {
    reconstructed: firewood::db::Reconstructed,
    metrics_context: firewood_metrics::MetricsContext,
}

impl DbView for ReconstructedHandle {
    type Iter<'view>
        = <firewood::db::Reconstructed as DbView>::Iter<'view>
    where
        Self: 'view;

    fn root_hash(&self) -> Option<HashKey> {
        self.reconstructed.root_hash()
    }

    fn val<K: api::KeyType>(&self, key: K) -> Result<Option<firewood::merkle::Value>, api::Error> {
        self.reconstructed.val(key)
    }

    fn single_key_proof<K: api::KeyType>(&self, key: K) -> Result<api::FrozenProof, api::Error> {
        self.reconstructed.single_key_proof(key)
    }

    fn range_proof<K: api::KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<std::num::NonZeroUsize>,
    ) -> Result<api::FrozenRangeProof, api::Error> {
        self.reconstructed.range_proof(first_key, last_key, limit)
    }

    fn iter_option<K: api::KeyType>(
        &self,
        first_key: Option<K>,
    ) -> Result<Self::Iter<'_>, api::Error> {
        self.reconstructed.iter_option(first_key)
    }

    fn dump_to_string(&self) -> Result<String, api::Error> {
        self.reconstructed.dump_to_string()
    }
}

impl ReconstructedHandle {
    pub(crate) const fn new(
        reconstructed: firewood::db::Reconstructed,
        metrics_context: firewood_metrics::MetricsContext,
    ) -> Self {
        Self {
            reconstructed,
            metrics_context,
        }
    }

    /// Creates an iterator on the reconstructed view starting from the given key.
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn iter_from(&self, first_key: Option<&[u8]>) -> CreateIteratorResult<'_> {
        let it = self
            .iter_option(first_key)
            .expect("infallible; see issue #1329");
        CreateIteratorResult(IteratorHandle::new(
            self.reconstructed.view(),
            Box::new(it) as BoxKeyValueIter<'_>,
            self.metrics_context(),
        ))
    }

    /// Consume this reconstructed view and apply a new batch of operations.
    ///
    /// # Errors
    ///
    /// Returns an error if reconstruction fails.
    pub fn reconstruct(self, values: impl IntoBatchIter) -> Result<Self, api::Error> {
        let pool = ThreadPoolBuilder::new()
            .num_threads(BranchNode::MAX_CHILDREN)
            .build()
            .map_err(|e| api::Error::IO(std::io::Error::other(e)))?;
        let next = self.reconstructed.reconstruct(values, &pool)?;
        Ok(Self::new(next, self.metrics_context))
    }
}

impl crate::MetricsContextExt for ReconstructedHandle {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        Some(self.metrics_context)
    }
}
