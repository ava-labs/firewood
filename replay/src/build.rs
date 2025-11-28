use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

/// Run `cargo build` in `project_dir` and print a progress percentage.
///
/// This parses Cargo's own progress lines (e.g. `304/359`) from stderr and
/// reports `done/total` using the same counters that Cargo displays.
pub fn run_cargo_build_with_progress(project_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // println!("Starting `cargo build` in {:?}", project_dir);

    let mut command = Command::new("cargo");
    command
        .arg("build")
        .current_dir(project_dir)
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to capture cargo stderr")?;
    let reader = BufReader::new(stderr);

    let mut last_progress: Option<(u64, u64)> = None;
    let mut last_pct: f64 = 0.0;

    for line_result in reader.lines() {
        let line = line_result?;
        if let Some((done, total)) = extract_progress(&line) {
            let changed = last_progress.map_or(true, |(d, t)| d != done || t != total);
            if changed && total > 0 {
                last_progress = Some((done, total));
                let pct = (done as f64 / total as f64) * 100.0;
                if pct - last_pct > 10.0 {
                    last_pct = pct;
                    println!("cargo build progress: {done}/{total} ({pct:.1}%)");
                }
            }
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(format!("cargo build failed with status {status}").into());
    }

    Ok(())
}

fn extract_progress(line: &str) -> Option<(u64, u64)> {
    for token in line.split_whitespace() {
        if let Some((done_str, total_str)) = token.split_once('/') {
            let done = done_str.parse().ok()?;
            let total_clean = total_str.trim_end_matches(|c: char| !c.is_ascii_digit());
            let total = total_clean.parse().ok()?;
            return Some((done, total));
        }
    }
    None
}
