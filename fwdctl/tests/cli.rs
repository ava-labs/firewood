// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use anyhow::{anyhow, Result};
use assert_cmd::Command;
use predicates::prelude::*;
use serial_test::serial;
use std::fs::{self, remove_file};
use std::path::PathBuf;

const PRG: &str = "fwdctl";
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Removes the firewood database on disk
fn fwdctl_delete_db() -> Result<()> {
    if let Err(e) = remove_file(tmpdb::path()) {
        eprintln!("failed to delete testing dir: {e}");
        return Err(anyhow!(e));
    }

    Ok(())
}

#[test]
#[serial]
fn fwdctl_prints_version() -> Result<()> {
    let expected_version_output: String = format!("{PRG} {VERSION}\n");

    // version is defined and succeeds with the desired output
    Command::cargo_bin(PRG)?
        .args(["-V"])
        .assert()
        .success()
        .stdout(predicate::str::contains(expected_version_output));

    Ok(())
}

#[test]
#[serial]
fn fwdctl_creates_database() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_insert_successful() -> Result<()> {
    // Create db
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    // Insert data
    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["year"])
        .args(["2023"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("year"));

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_get_successful() -> Result<()> {
    // Create db and insert data
    Command::cargo_bin(PRG)?
        .arg("create")
        .args([tmpdb::path()])
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["year"])
        .args(["2023"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("year"));

    // Get value back out
    Command::cargo_bin(PRG)?
        .arg("get")
        .args(["year"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2023"));

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_delete_successful() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["year"])
        .args(["2023"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("year"));

    // Delete key -- prints raw data of deleted value
    Command::cargo_bin(PRG)?
        .arg("delete")
        .args(["year"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("key year deleted successfully"));

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_root_hash() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["year"])
        .args(["2023"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("year"));

    // Get root
    Command::cargo_bin(PRG)?
        .arg("root")
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_dump() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["year"])
        .args(["2023"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("year"));

    // Get root
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2023"));

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_dump_with_start_stop_and_max() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["a"])
        .args(["1"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("a"));

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["b"])
        .args(["2"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("b"));

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["c"])
        .args(["3"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("c"));

    // Test stop in the middle
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--stop-key"])
        .arg("b")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Next key is c, resume with \"--start-key=c\".",
        ));

    // Test stop in the end
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--stop-key"])
        .arg("c")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "There is no next key. Data dump completed.",
        ));

    // Test start in the middle
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--start-key"])
        .arg("b")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("\'b"));

    // Test start and stop
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--start-key"])
        .arg("b")
        .args(["--stop-key"])
        .arg("b")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("\'b"))
        .stdout(predicate::str::contains(
            "Next key is c, resume with \"--start-key=c\".",
        ));

    // Test start and stop
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--start-key"])
        .arg("b")
        .args(["--max-key-count"])
        .arg("1")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("\'b"))
        .stdout(predicate::str::contains(
            "Next key is c, resume with \"--start-key=c\".",
        ));

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

#[test]
#[serial]
fn fwdctl_dump_with_csv_and_json() -> Result<()> {
    Command::cargo_bin(PRG)?
        .arg("create")
        .arg(tmpdb::path())
        .assert()
        .success();

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["a"])
        .args(["1"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("a"));

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["b"])
        .args(["2"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("b"));

    Command::cargo_bin(PRG)?
        .arg("insert")
        .args(["c"])
        .args(["3"])
        .args(["--db"])
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::contains("c"));

    // Test output csv
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--output-format"])
        .arg("csv")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let contents = fs::read_to_string("dump.csv").expect("Should read dump.csv file");
    assert_eq!(contents, "a,1\nb,2\nc,3\n");
    fs::remove_file("dump.csv").expect("Should remove dump.csv file");

    // Test output json
    Command::cargo_bin(PRG)?
        .arg("dump")
        .args(["--output-format"])
        .arg("json")
        .args([tmpdb::path()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let contents = fs::read_to_string("dump.json").expect("Should read dump.json file");
    assert_eq!(
        contents,
        "{\n  \"a\": \"1\",\n  \"b\": \"2\",\n  \"c\": \"3\"\n}\n"
    );
    fs::remove_file("dump.json").expect("Should remove dump.json file");

    fwdctl_delete_db().map_err(|e| anyhow!(e))?;

    Ok(())
}

// A module to create a temporary database name for use in
// tests. The directory will be one of:
// - cargo's compile-time CARGO_TARGET_TMPDIR, if that exists
// - the value of the TMPDIR environment, if that is set, or
// - fallback to /tmp

// using cargo's CARGO_TARGET_TMPDIR ensures that multiple runs
// of this in different directories will have different databases

mod tmpdb {
    use super::*;

    const FIREWOOD_TEST_DB_NAME: &str = "test_firewood";
    const TARGET_TMP_DIR: Option<&str> = option_env!("CARGO_TARGET_TMPDIR");

    pub fn path() -> PathBuf {
        TARGET_TMP_DIR
            .map(PathBuf::from)
            .or_else(|| std::env::var("TMPDIR").ok().map(PathBuf::from))
            .unwrap_or(std::env::temp_dir())
            .join(FIREWOOD_TEST_DB_NAME)
    }
}
