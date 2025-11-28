use std::env;
use std::path::{Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{Oid, Repository};

use crate::build::run_cargo_build_with_progress;

fn get_temp_clone_path(repo_name: &str) -> PathBuf {
    let mut temp_path = env::temp_dir();
    temp_path.push(repo_name);
    temp_path
}

pub fn binary_search_commits() {
    clone_and_walk_commits_git2(
        "https://github.com/ava-labs/firewood.git",
        get_temp_clone_path("firewood").as_path(),
    )
    .unwrap();
}

fn checkout_branch(
    repo: &Repository,
    branch_name: &str,
    directory: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Point HEAD at the requested branch and check it out
    repo.set_head(&format!("refs/heads/{}", branch_name))?;
    let mut checkout_opts = CheckoutBuilder::new();
    checkout_opts.force();
    if let Some(dir) = directory {
        // checkout only the specified directory
        checkout_opts.path(dir);
    }
    repo.checkout_head(Some(&mut checkout_opts))?;

    Ok(())
}

fn checkout_replay_code(repo: &Repository) -> Result<(), Box<dyn std::error::Error>> {
    let b = repo.revparse_single(&format!("remotes/origin/amin/performance-replay"))?;
    let commit = b.peel_to_commit()?;
    let commit_obj = commit.into_object();

    // Point HEAD at the requested branch and check it out
    let mut checkout_opts = CheckoutBuilder::new();
    checkout_opts.force();
    checkout_opts.path("replay");
    repo.checkout_tree(&commit_obj, Some(&mut checkout_opts))?;

    Ok(())
}

fn clone_and_walk_commits_git2(
    repo_url: &str,
    clone_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = if !clone_path.exists() {
        Repository::clone(repo_url, clone_path)?
    } else {
        let repo = Repository::open(clone_path)?;
        // it's possible for the repo to be on another head already, let's force checkout main.
        checkout_branch(&repo, "main", None)?;
        repo
    };

    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;

    // Collect all commits
    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_commit.id())?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    let commit_ids: Vec<Oid> = revwalk.collect::<Result<Vec<_>, _>>()?;
    // Walk through each commit
    for commit_id in commit_ids {
        let commit = repo.find_commit(commit_id)?;
        println!(
            "Processing commit: {} - {}",
            commit.id(),
            commit.summary().unwrap_or("No message")
        );

        // Checkout the commit
        let obj = repo.find_object(commit_id, None)?;
        let mut checkout_opts = CheckoutBuilder::new();
        checkout_opts.force();
        repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
        repo.set_head_detached(commit_id)?;

        checkout_replay_code(&repo)?;

        run_custom_commands(clone_path, &commit_id.to_string())?;
        break;
    }

    Ok(())
}

fn run_custom_commands(
    repo_path: &Path,
    commit_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    run_cargo_build_with_progress(repo_path)?;
    Ok(())
}
