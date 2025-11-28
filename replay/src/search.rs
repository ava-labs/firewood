use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use std::{env, fs};

use git2::build::CheckoutBuilder;
use git2::{Oid, Repository};

use crate::build::run_cargo_build_with_progress;

fn get_temp_clone_path(repo_name: &str) -> PathBuf {
    let mut temp_path = env::temp_dir();
    temp_path.push(repo_name);
    temp_path
}

pub fn binary_search_commits() -> Result<(), Box<dyn std::error::Error>> {
    let repo = clone_or_get_repo(
        "https://github.com/ava-labs/firewood.git",
        get_temp_clone_path("firewood").as_path(),
    )?;

    let commits = all_commits(&repo)?;
    let start_commit = Oid::from_str("80dbeee04a06154967a31deaafb8fce15aa7afff")?;
    let start_index = commits
        .iter()
        .position(|&x| x == start_commit)
        .ok_or("start commit not found in history")?;
    if start_index == 0 {
        println!("start commit {start_commit} is the most recent commit; nothing to search");
        return Ok(());
    }
    let commits = &commits[..start_index];

    // binary search for the latest commit that fails to build
    let mut left = 0; // 0 is the most recent commit
    let mut right = commits.len() - 1; // right is the oldest commit we consider
    let mut first_failing: Option<usize> = None;
    while left <= right {
        let mid = left + (right - left) / 2;
        let commit = commits[mid];
        println!("Checking commit: {}", commit);
        checkout_commit(&repo, commit)?;
        let res = build_replay_cli(&repo, commit);
        if res.is_err() {
            // This commit fails to build; remember it and look for an earlier
            // failing commit closer to HEAD.
            first_failing = Some(mid);
            if mid == 0 {
                break;
            }
            right = mid - 1;
        } else {
            // This commit builds; any failing commit must be older.
            left = mid + 1;
        }
    }

    match first_failing {
        Some(idx) => {
            println!("first failing commit: {}", commits[idx]);
        }
        None => {
            println!(
                "all commits between HEAD and {start_commit} built successfully; \
                 no failing commit found in this range"
            );
        }
    }

    Ok(())
}

pub fn binary_search_performance() -> Result<(), Box<dyn std::error::Error>> {
    let repo = clone_or_get_repo(
        "https://github.com/ava-labs/firewood.git",
        get_temp_clone_path("firewood").as_path(),
    )?;

    let commits = all_commits(&repo)?;
    let start_commit = Oid::from_str("80dbeee04a06154967a31deaafb8fce15aa7afff")?;
    let start_index = commits
        .iter()
        .position(|&x| x == start_commit)
        .ok_or("start commit not found in history")?;
    if start_index == 0 {
        println!("start commit {start_commit} is the most recent commit; nothing to search");
        return Ok(());
    }
    let commits = &commits[..start_index];

    // binary search for the latest commit that fails to build
    let mut left = 0; // 0 is the most recent commit
    let mut right = commits.len() - 1; // right is the oldest commit we consider

    // checkout left and right too
    for idx in [left, right] {
        let commit = commits[idx];
        checkout_commit(&repo, commit)?;
        build_replay_cli(&repo, commit)?;
        run_replay_cli(&repo, commit)?;
    }

    let mut first_failing: Option<usize> = None;
    while left <= right {
        let mid = left + (right - left) / 2;
        let commit = commits[mid];
        println!("Checking commit: {}", commit);
        checkout_commit(&repo, commit)?;
        let res = build_replay_cli(&repo, commit);
        if res.is_err() {
            // This commit fails to build; remember it and look for an earlier
            // failing commit closer to HEAD.
            first_failing = Some(mid);
            if mid == 0 {
                break;
            }
            right = mid - 1;
        } else {
            // This commit builds; any failing commit must be older.
            left = mid + 1;
        }
    }

    match first_failing {
        Some(idx) => {
            println!("first failing commit: {}", commits[idx]);
        }
        None => {
            println!(
                "all commits between HEAD and {start_commit} built successfully; \
                 no failing commit found in this range"
            );
        }
    }

    Ok(())
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

fn clone_or_get_repo(
    repo_url: &str,
    clone_path: &Path,
) -> Result<Repository, Box<dyn std::error::Error>> {
    let repo = if !clone_path.exists() {
        Repository::clone(repo_url, clone_path)?
    } else {
        let repo = Repository::open(clone_path)?;
        // it's possible for the repo to be on another head already, let's force checkout main.
        checkout_branch(&repo, "main", None)?;
        repo
    };
    Ok(repo)
}

fn all_commits(repo: &Repository) -> Result<Vec<Oid>, Box<dyn std::error::Error>> {
    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_commit.id())?;
    revwalk.set_sorting(git2::Sort::TIME)?;
    let commit_ids: Vec<Oid> = revwalk.collect::<Result<Vec<_>, _>>()?;
    Ok(commit_ids)
}

fn checkout_commit(repo: &Repository, commit_id: Oid) -> Result<(), Box<dyn std::error::Error>> {
    let obj = repo.find_object(commit_id, None)?;
    let mut checkout_opts = CheckoutBuilder::new();
    checkout_opts.force();
    repo.checkout_tree(&obj, Some(&mut checkout_opts))?;
    repo.set_head_detached(commit_id)?;
    Ok(())
}

fn build_replay_cli(repo: &Repository, _commit: Oid) -> Result<(), Box<dyn std::error::Error>> {
    checkout_replay_code(&repo)?;
    let mut cargo_toml = fs::read_to_string(repo.workdir().unwrap().join("Cargo.toml"))?;
    cargo_toml = cargo_toml.replacen("\"triehash\",", "\"triehash\", \"replay\",", 1);
    fs::write(repo.workdir().unwrap().join("Cargo.toml"), cargo_toml)?;
    run_cargo_build_with_progress(repo.workdir().unwrap())?;
    Ok(())
}

fn run_replay_cli(repo: &Repository, commit: Oid) -> Result<(), Box<dyn std::error::Error>> {
    let binary = repo.workdir().unwrap().join("target/debug/firewood-replay");
    let db_path = "/Volumes/Workspace/FirewoodForest/firewood-replay-simple/replay_new_db";
    let log_path = "/Volumes/Workspace/FirewoodForest/firewood-replay-simple/replay_100k_log_db";
    let metrics_path = format!(
        "/Volumes/Workspace/FirewoodForest/firewood-replay-simple/run-metrics/{}.txt",
        commit
    );
    let mut child = Command::new(binary)
        .arg("re-execute")
        .arg("--db")
        .arg(db_path)
        .arg("--log")
        .arg(log_path)
        .arg("--metrics")
        .arg(metrics_path.clone())
        .spawn()?;
    child.wait()?;
    println!(
        "average propose+commit time for {}: {}ns",
        commit,
        replay_time_ns_from_metrics(Path::new(&metrics_path))?
    );
    Ok(())
}

fn replay_time_ns_from_metrics(metrics_path: &Path) -> Result<f64, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(metrics_path)?;

    fn metric_value(contents: &str, name: &str) -> Result<f64, Box<dyn std::error::Error>> {
        for line in contents.lines() {
            if line.starts_with('#') {
                continue;
            }
            if !line.starts_with(&format!("{} ", name)) {
                continue;
            }
            if let Some(value_str) = line.split_whitespace().last() {
                let value: f64 = value_str.parse()?;
                return Ok(value);
            }
        }
        Err(format!("metric {name} not found").into())
    }

    let propose = metric_value(&contents, "firewood_replay_propose")?;
    let propose_ns = metric_value(&contents, "firewood_replay_propose_ns")?;
    let commit = metric_value(&contents, "firewood_replay_commit")?;
    let commit_ns = metric_value(&contents, "firewood_replay_commit_ns")?;

    if propose == 0.0 || commit == 0.0 {
        return Err("replay metrics have zero counts; cannot compute average time".into());
    }

    Ok(propose_ns / propose + commit_ns / commit)
}
