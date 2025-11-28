use std::collections::HashMap;
use std::ffi::OsStr;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use std::{env, fs};

use git2::build::CheckoutBuilder;
use git2::{Oid, Repository};
use plotly::common::{Mode, Title};
use plotly::{Plot, Scatter};

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

    for (idx, commit) in commits.into_iter().enumerate() {
        println!("commits: {}/{}", idx + 1, commits.len());
        checkout_commit(&repo, *commit)?;
        build_replay_cli(&repo, *commit)?;
        run_replay_cli(&repo, *commit)?;
    }

    // checkout left and right too
    // let mut threshold = 0.0;
    // for idx in [left, right] {
    //     let commit = commits[idx];
    //     checkout_commit(&repo, commit)?;
    //     build_replay_cli(&repo, commit)?;
    //     threshold += run_replay_cli(&repo, commit)?;
    // }
    // threshold /= 2.0;

    // let mut first_failing: Option<usize> = None;
    // while left <= right {
    //     let mid = left + (right - left) / 2;
    //     let commit = commits[mid];
    //     println!("Checking commit: {}", commit);
    //     checkout_commit(&repo, commit)?;
    //     build_replay_cli(&repo, commit)?;
    //     let p = run_replay_cli(&repo, commit)?;
    //     if p > threshold {
    //         // This commit performs bad; remember it and look for an older
    //         // failing commit closer to HEAD.
    //         first_failing = Some(mid);
    //         if mid == 0 {
    //             break;
    //         }
    //         right = mid - 1;
    //     } else {
    //         // This commit builds; any failing commit must be older.
    //         left = mid + 1;
    //     }
    // }

    // match first_failing {
    //     Some(idx) => {
    //         println!("first failing commit: {}", commits[idx]);
    //     }
    //     None => {
    //         println!(
    //             "all commits between HEAD and {start_commit} built successfully; \
    //              no failing commit found in this range"
    //         );
    //     }
    // }

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

fn run_replay_cli(repo: &Repository, commit: Oid) -> Result<f64, Box<dyn std::error::Error>> {
    let binary = repo.workdir().unwrap().join("target/debug/firewood-replay");
    let db_path = "/Volumes/Workspace/FirewoodForest/firewood-replay-simple/replay_new_db";
    if fs::exists(db_path).unwrap_or(false) {
        fs::remove_file(db_path)?;
    }
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
        .arg("--max-commits")
        .arg("10000")
        .arg("--metrics")
        .arg(metrics_path.clone())
        .spawn()?;
    child.wait()?;
    let replay_time = replay_time_ns_from_metrics(Path::new(&metrics_path))?;
    println!(
        "average propose+commit time for {}: {}ns",
        commit, replay_time
    );
    Ok(replay_time)
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

pub fn plot_replay_times_from_dir(
    metrics_dir: &Path,
    output_html: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dmap: HashMap<String, f64> = HashMap::new();

    for entry in fs::read_dir(metrics_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension() != Some(OsStr::new("txt")) {
            continue;
        }

        let label = path
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let value_ns = replay_time_ns_from_metrics(&path)?;
        dmap.insert(label.clone(), value_ns / 1_000_000.0);
    }

    if dmap.is_empty() {
        return Err("no metrics files found to plot".into());
    }

    let repo = clone_or_get_repo(
        "https://github.com/ava-labs/firewood.git",
        get_temp_clone_path("firewood").as_path(),
    )?;

    let mut labels = Vec::new();
    let mut values_ms = Vec::new();

    let mut commits = all_commits(&repo)?;
    commits.reverse();
    for c in commits {
        if let Some(value) = dmap.get(&c.to_string()) {
            labels.push(c.to_string());
            values_ms.push(*value);
        }
    }

    let trace = Scatter::new(labels, values_ms).mode(Mode::LinesMarkers);
    let mut plot = Plot::new();
    plot.add_trace(trace);
    // plot.(Title::new("Replay time per metrics log (ms)"));
    plot.write_html(output_html);

    Ok(())
}
