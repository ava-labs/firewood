use std::process::Command;

fn main() {
    // Get the git commit SHA
    let git_sha = match Command::new("git").args(["rev-parse", "HEAD"]).output() {
        Ok(output) => {
            if output.status.success() {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                let error_msg = String::from_utf8_lossy(&output.stderr);
                format!("git error: {}", error_msg.trim())
            }
        }
        Err(e) => {
            format!("git not found: {}", e)
        }
    };

    // Make the git SHA available to the main.rs file
    println!("cargo:rustc-env=GIT_COMMIT_SHA={}", git_sha);

    // Re-run this build script if the git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
