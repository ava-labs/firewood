// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use semver::Version;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let crate_version = Version::parse(VERSION).expect("failed to parse CARGO_PKG_VERSION");

    // Set FIREWOOD_DB_VERSION to the major version of the crate. E.g., if the version
    // is 1.2.3, this will set it to 1. If the version is 0.0.1 or 0.1.0, it will be set
    // to 0.
    println!(
        "cargo::rustc-env=FIREWOOD_DB_VERSION={}",
        crate_version.major
    )
}
