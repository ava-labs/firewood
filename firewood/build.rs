// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Captures the git commit sha and tag (if any) at build time so they can be
//! exported via the `firewood_build_info` metric.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn main() {
    // Falls back to "unknown" when building outside a git checkout
    // (e.g. from a published crate).
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    // Empty when no tag points at HEAD.
    let tag = git(&["describe", "--tags", "--exact-match", "HEAD"]).unwrap_or_default();

    println!("cargo::rustc-env=FIREWOOD_GIT_SHA={sha}");
    println!("cargo::rustc-env=FIREWOOD_GIT_TAG={tag}");

    // Rebuild when HEAD or the tag set changes. HEAD covers both the detached
    // case (contains the sha) and the branch case (the symbolic ref rewrites on
    // checkout); packed-refs and refs/tags cover tag creation and deletion.
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo::rerun-if-changed={git_dir}/HEAD");
        println!("cargo::rerun-if-changed={git_dir}/packed-refs");
        println!("cargo::rerun-if-changed={git_dir}/refs/tags");
    }
}
