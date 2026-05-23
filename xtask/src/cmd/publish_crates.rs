use std::collections::{HashMap, HashSet};

use anyhow::Context;
use cargo_metadata::{Metadata, Package, PackageId};
use clap::Args;
use topological_sort::TopologicalSort;

use crate::cargo::{load_metadata, run_cargo};

#[derive(Debug, Args)]
pub struct Command {
    /// Print what would be published without actually publishing.
    #[arg(long)]
    dry_run: bool,
}

impl Command {
    pub fn run(&self) -> anyhow::Result<()> {
        let metadata = load_metadata()?;
        let crates = crates_in_publish_order(&metadata)?;

        for pkg in crates {
            let name = &pkg.name;
            let version = pkg.version.to_string();

            // publish = false means the crate is not meant for crates.io
            if pkg.publish.as_deref() == Some(&[]) {
                eprintln!("{name}@{version}: skip — publish = false");
                continue;
            }

            if has_git_dep(&metadata, &pkg.id) {
                eprintln!("{name}@{version}: skip — transitive git-patched dependency");
                continue;
            }

            if is_published(name, &version)? {
                eprintln!("{name}@{version}: skip — already published on crates.io");
                continue;
            }

            if self.dry_run {
                eprintln!("{name}@{version}: would publish (dry run)");
            } else {
                eprintln!("Publishing {name}@{version}...");
                let status = run_cargo(["publish", "-p", name])?;
                if !status.success() {
                    return Err(anyhow::anyhow!(
                        "failed to publish {name}: exit status {status}"
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Returns workspace member packages in topological order (deps before dependents).
fn crates_in_publish_order(metadata: &Metadata) -> anyhow::Result<Vec<&Package>> {
    let ws_ids: HashSet<&PackageId> = metadata.workspace_members.iter().collect();
    let id_to_pkg: HashMap<&PackageId, &Package> =
        metadata.packages.iter().map(|p| (&p.id, p)).collect();

    let resolve = metadata
        .resolve
        .as_ref()
        .context("missing `resolve` in cargo metadata")?;

    let mut topo: TopologicalSort<&PackageId> = TopologicalSort::new();
    for node in &resolve.nodes {
        if !ws_ids.contains(&node.id) {
            continue;
        }
        topo.insert(&node.id);
        for dep in &node.dependencies {
            if ws_ids.contains(dep) {
                // dep must be published before node
                topo.add_dependency(dep, &node.id);
            }
        }
    }

    let mut result = Vec::new();
    while !topo.is_empty() {
        let batch = topo.pop_all();
        anyhow::ensure!(
            !batch.is_empty(),
            "cycle detected in workspace dependency graph"
        );
        for id in batch {
            result.push(
                id_to_pkg
                    .get(id)
                    .copied()
                    .context("package ID in resolve missing from packages")?,
            );
        }
    }

    Ok(result)
}

/// Returns true if the package has any transitive dependency with a git source.
fn has_git_dep(metadata: &Metadata, pkg_id: &PackageId) -> bool {
    let git_ids: HashSet<&PackageId> = metadata
        .packages
        .iter()
        .filter(|p| {
            p.source
                .as_ref()
                .is_some_and(|s| s.to_string().starts_with("git+"))
        })
        .map(|p| &p.id)
        .collect();

    let Some(resolve) = &metadata.resolve else {
        return false;
    };

    let node_deps: HashMap<&PackageId, &[PackageId]> = resolve
        .nodes
        .iter()
        .map(|n| (&n.id, n.dependencies.as_slice()))
        .collect();

    let mut visited: HashSet<&PackageId> = HashSet::new();
    let mut queue = vec![pkg_id];
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        if git_ids.contains(id) {
            return true;
        }
        if let Some(deps) = node_deps.get(id) {
            queue.extend(*deps);
        }
    }

    false
}

/// Returns true if the given crate version is already published on crates.io.
fn is_published(name: &str, version: &str) -> anyhow::Result<bool> {
    let url = format!("https://crates.io/api/v1/crates/{name}/{version}");
    match ureq::get(&url)
        .header(
            "User-Agent",
            "firewood-xtask/1.0 (github.com/ava-labs/firewood)",
        )
        .call()
    {
        Ok(_) => Ok(true),
        Err(ureq::Error::StatusCode(404)) => Ok(false),
        Err(e) => Err(anyhow::anyhow!(
            "crates.io check for {name}@{version} failed: {e}"
        )),
    }
}
