// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod borrowed;
mod display_hex;
mod hash_key;
mod kvp;
mod owned;
pub mod rendered_metrics;
pub(crate) mod results;

pub use self::borrowed::BorrowedBatchOps;
pub use self::borrowed::BorrowedBytes;
pub use self::borrowed::BorrowedBytes2D;
pub use self::borrowed::BorrowedSlice;
use self::display_hex::DisplayHex;
pub use self::hash_key::HashKey;
pub use self::kvp::BatchOp;
pub use self::kvp::OwnedKeyValueBatch;
pub use self::kvp::OwnedKeyValuePair;
pub use self::owned::OwnedBytes;
pub use self::owned::OwnedSlice;
pub use self::rendered_metrics::OwnedBucket;
pub use self::rendered_metrics::OwnedBucketSpan;
pub use self::rendered_metrics::OwnedClassicHistogram;
pub use self::rendered_metrics::OwnedLabelPair;
pub use self::rendered_metrics::OwnedMetric;
pub use self::rendered_metrics::OwnedMetricFamily;
pub use self::rendered_metrics::OwnedMetricValue;
pub use self::rendered_metrics::OwnedNativeHistogram;
pub use self::rendered_metrics::OwnedQuantile;
pub use self::rendered_metrics::OwnedRenderedMetrics;
pub use self::rendered_metrics::OwnedSummary;
pub(crate) use self::results::CResult;
pub use self::results::ChangeProofResult;
pub use self::results::CodeIteratorResult;
pub use self::results::EthProofResult;
pub use self::results::HandleResult;
pub use self::results::HashResult;
pub use self::results::IteratorResult;
pub use self::results::KeyValueBatchResult;
pub use self::results::KeyValueResult;
pub use self::results::NextKeyRangeResult;
pub(crate) use self::results::NullHandleResult;
pub use self::results::ProposalResult;
pub use self::results::RangeProofResult;
pub use self::results::ReconstructedResult;
pub use self::results::RenderedMetricsResult;
pub use self::results::RevisionResult;
pub use self::results::ValueResult;
pub use self::results::VoidResult;

/// Maybe is a C-compatible optional type using a tagged union pattern.
///
/// FFI methods and types can use this to represent optional values where `Optional<T>`
/// does not work due to it not having a C-compatible layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C, usize)]
pub enum Maybe<T> {
    /// No value present.
    None,
    /// A value is present.
    Some(T),
}

impl<T> Maybe<T> {
    /// Returns true if the `Maybe` contains a value.
    pub const fn is_some(&self) -> bool {
        matches!(self, Maybe::Some(_))
    }

    /// Returns true if the `Maybe` does not contain a value.
    pub const fn is_none(&self) -> bool {
        matches!(self, Maybe::None)
    }

    /// Converts from `&Maybe<T>` to `Maybe<&T>`.
    pub const fn as_ref(&self) -> Maybe<&T> {
        match self {
            Maybe::None => Maybe::None,
            Maybe::Some(v) => Maybe::Some(v),
        }
    }

    /// Converts from `&mut Maybe<T>` to `Maybe<&mut T>`.
    pub const fn as_mut(&mut self) -> Maybe<&mut T> {
        match self {
            Maybe::None => Maybe::None,
            Maybe::Some(v) => Maybe::Some(v),
        }
    }

    /// Maps a `Maybe<T>` to `Maybe<U>` by applying a function to a contained value.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Maybe<U> {
        match self {
            Maybe::None => Maybe::None,
            Maybe::Some(v) => Maybe::Some(f(v)),
        }
    }

    /// Converts from `Maybe<T>` to `Option<T>`.
    pub fn into_option(self) -> Option<T> {
        match self {
            Maybe::None => None,
            Maybe::Some(v) => Some(v),
        }
    }
}

impl<T> From<Option<T>> for Maybe<T> {
    fn from(opt: Option<T>) -> Self {
        match opt {
            None => Maybe::None,
            Some(v) => Maybe::Some(v),
        }
    }
}
