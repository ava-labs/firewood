// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::metrics::MetricsContextExt;
use crate::reconstructed::ReconstructedHandle;
use crate::{CreateIteratorResult, IteratorHandle};
use firewood::v2::api;
use firewood::v2::api::{ArcDynDbView, BoxKeyValueIter, DbView, HashKey, Reconstructible as _};
use firewood_metrics::MetricsContext;
use firewood_storage::{BranchNode, Committed, FileBacked, NodeStore};
use rayon::ThreadPoolBuilder;
use std::sync::Arc;

#[derive(Debug)]
pub struct RevisionHandle {
    view: ArcDynDbView,
    historical: Option<Arc<NodeStore<Committed, FileBacked>>>,
    metrics_context: MetricsContext,
}

impl RevisionHandle {
    /// Creates a new revision handle for the provided database view.
    pub(crate) fn new(
        view: ArcDynDbView,
        historical: Option<Arc<NodeStore<Committed, FileBacked>>>,
        metrics_context: MetricsContext,
    ) -> RevisionHandle {
        RevisionHandle {
            view,
            historical,
            metrics_context,
        }
    }

    /// Creates an iterator on the revision starting from the given key.
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn iter_from(&self, first_key: Option<&[u8]>) -> CreateIteratorResult<'_> {
        let it = self
            .view
            .iter_option(first_key)
            .expect("infallible; see issue #1329");
        CreateIteratorResult(IteratorHandle::new(
            self.view.clone(),
            it,
            self.metrics_context(),
        ))
    }

    /// Reconstruct this historical revision with a batch of operations.
    ///
    /// # Errors
    ///
    /// Returns an error if reconstruction fails.
    pub fn reconstruct(
        &self,
        values: impl api::IntoBatchIter,
    ) -> Result<ReconstructedHandle, api::Error> {
        let Some(historical) = self.historical.as_ref() else {
            return Err(api::Error::FeatureNotSupported(
                "reconstruct requires a historical (committed) revision handle".to_string(),
            ));
        };

        let pool = ThreadPoolBuilder::new()
            .num_threads(BranchNode::MAX_CHILDREN)
            .build()
            .map_err(|e| api::Error::IO(std::io::Error::other(e)))?;

        let reconstructed = historical.as_ref().reconstruct(values, &pool)?;
        Ok(ReconstructedHandle::new(
            reconstructed,
            self.metrics_context,
        ))
    }
}

impl DbView for RevisionHandle {
    type Iter<'view>
        = BoxKeyValueIter<'view>
    where
        Self: 'view;

    fn root_hash(&self) -> Option<HashKey> {
        self.view.root_hash()
    }

    fn val<K: api::KeyType>(&self, key: K) -> Result<Option<firewood::merkle::Value>, api::Error> {
        self.view.val(key.as_ref())
    }

    fn single_key_proof<K: api::KeyType>(&self, key: K) -> Result<api::FrozenProof, api::Error> {
        self.view.single_key_proof(key.as_ref())
    }

    fn range_proof<K: api::KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<std::num::NonZeroUsize>,
    ) -> Result<api::FrozenRangeProof, api::Error> {
        self.view.range_proof(
            first_key.as_ref().map(AsRef::as_ref),
            last_key.as_ref().map(AsRef::as_ref),
            limit,
        )
    }

    fn iter_option<K: api::KeyType>(
        &self,
        first_key: Option<K>,
    ) -> Result<Self::Iter<'_>, api::Error> {
        self.view.iter_option(first_key.as_ref().map(AsRef::as_ref))
    }

    fn dump_to_string(&self) -> Result<String, api::Error> {
        self.view.dump_to_string()
    }
}

#[derive(Debug)]
pub struct GetRevisionResult {
    pub handle: RevisionHandle,
    pub root_hash: HashKey,
}

impl crate::MetricsContextExt for RevisionHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        Some(self.metrics_context)
    }
}
