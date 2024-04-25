// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use typed_builder::TypedBuilder;

/// Database configuration.
#[derive(Clone, TypedBuilder, Debug)]
pub struct DbConfig {
    /// Whether to truncate the DB when opening it. If set, the DB will be reset and all its
    /// existing contents will be lost.
    #[builder(default = false)]
    pub truncate: bool,
}
