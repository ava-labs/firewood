// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::unwrap_used,
    reason = "integration test binary; unwrap failures surface as test failures"
)]

use predicates::prelude::*;
use std::fmt::Write;
use std::fs;
use std::path::Path;

const PRG: &str = "fwdctl";
const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! cargo_bin_cmd {
    () => {
        ::assert_cmd::cargo::cargo_bin_cmd!("fwdctl")
    };
}

fn with_tmpdir(test: impl FnOnce(&Path)) {
    let tmpdir = tempfile::tempdir().unwrap();
    test(tmpdir.path());
}

fn create_db(db_path: &Path) {
    cargo_bin_cmd!()
        .arg("create")
        .arg("--db")
        .arg(db_path)
        .assert()
        .success();
}

fn insert_key_value(db_path: &Path, key: &str, value: &str) {
    cargo_bin_cmd!()
        .arg("insert")
        .arg("--db")
        .arg(db_path)
        .args([key])
        .args([value])
        .assert()
        .success()
        .stdout(predicate::str::contains(key));
}

#[test]
fn fwdctl_prints_version() {
    let expected_version_output: String = format!("{PRG} {VERSION}");

    // version is defined and succeeds with the desired output
    cargo_bin_cmd!()
        .args(["-V"])
        .assert()
        .success()
        .stdout(predicate::str::contains(expected_version_output));
}

#[test]
fn fwdctl_creates_database() {
    with_tmpdir(create_db);
}

#[test]
fn fwdctl_insert_successful() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "year", "2023");
    });
}

#[test]
fn fwdctl_get_successful() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "year", "2023");

        cargo_bin_cmd!()
            .arg("get")
            .args(["year"])
            .arg("--db")
            .arg(db_path)
            .assert()
            .success()
            .stdout(predicate::str::contains("2023"));
    });
}

#[test]
fn fwdctl_delete_successful() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "year", "2023");

        // Delete key -- prints raw data of deleted value
        cargo_bin_cmd!()
            .arg("delete")
            .args(["year"])
            .arg("--db")
            .arg(db_path)
            .assert()
            .success()
            .stdout(predicate::str::contains("key year deleted successfully"));
    });
}

#[test]
fn fwdctl_root_hash() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "year", "2023");

        cargo_bin_cmd!()
            .arg("root")
            .arg("--db")
            .arg(db_path)
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    });
}

#[test]
fn fwdctl_dump() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "year", "2023");

        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .assert()
            .success()
            .stdout(predicate::str::contains("2023"));
    });
}

#[test]
fn test_slow_fwdctl_dump_with_start_stop_and_max() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "a", "1");
        insert_key_value(db_path, "b", "2");
        insert_key_value(db_path, "c", "3");

        // Test stop in the middle
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--stop-key"])
            .arg("b")
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "Next key is c, resume with \"--start-key=c\"",
            ));

        // Test stop in the end
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--stop-key"])
            .arg("c")
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "There is no next key. Data dump completed.",
            ));

        // Test start in the middle
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--start-key"])
            .arg("b")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("\'b"));

        // Test start and stop
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--start-key"])
            .arg("b")
            .args(["--stop-key"])
            .arg("b")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("\'b"))
            .stdout(predicate::str::contains(
                "Next key is c, resume with \"--start-key=c\"",
            ));

        // Test start and stop
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--start-key"])
            .arg("b")
            .args(["--max-key-count"])
            .arg("1")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("\'b"))
            .stdout(predicate::str::contains(
                "Next key is c, resume with \"--start-key=c\"",
            ));
    });
}

#[test]
fn test_slow_fwdctl_dump_with_csv_and_json() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "a", "1");
        insert_key_value(db_path, "b", "2");
        insert_key_value(db_path, "c", "3");

        // Test output csv
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--output-format"])
            .arg("csv")
            .assert()
            .success()
            .stdout(predicate::str::contains("Dumping to dump.csv"));

        let contents = fs::read_to_string("dump.csv").expect("Should read dump.csv file");
        assert_eq!(contents, "a,1\nb,2\nc,3\n");
        fs::remove_file("dump.csv").expect("Should remove dump.csv file");

        // Test output json
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--output-format"])
            .arg("json")
            .assert()
            .success()
            .stdout(predicate::str::contains("Dumping to dump.json"));

        let contents = fs::read_to_string("dump.json").expect("Should read dump.json file");
        assert_eq!(
            contents,
            "{\n  \"a\": \"1\",\n  \"b\": \"2\",\n  \"c\": \"3\"\n}\n"
        );
        fs::remove_file("dump.json").expect("Should remove dump.json file");
    });
}

#[test]
fn fwdctl_dump_with_file_name() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "a", "1");

        // Test without output format
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--output-file-name"])
            .arg("test")
            .assert()
            .failure()
            .stderr(predicate::str::contains("--output-format"));

        // Test output csv
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--output-format"])
            .arg("csv")
            .args(["--output-file-name"])
            .arg("test")
            .assert()
            .success()
            .stdout(predicate::str::contains("Dumping to test.csv"));

        let contents = fs::read_to_string("test.csv").expect("Should read test.csv file");
        assert_eq!(contents, "a,1\n");
        fs::remove_file("test.csv").expect("Should remove test.csv file");

        // Test output json
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--output-format"])
            .arg("json")
            .args(["--output-file-name"])
            .arg("test")
            .assert()
            .success()
            .stdout(predicate::str::contains("Dumping to test.json"));

        let contents = fs::read_to_string("test.json").expect("Should read test.json file");
        assert_eq!(contents, "{\n  \"a\": \"1\"\n}\n");
        fs::remove_file("test.json").expect("Should remove test.json file");
    });
}

#[test]
fn test_slow_fwdctl_dump_with_hex() {
    with_tmpdir(|db_path| {
        create_db(db_path);
        insert_key_value(db_path, "a", "1");
        insert_key_value(db_path, "b", "2");
        insert_key_value(db_path, "c", "3");

        // Test without output format
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--start-key"])
            .arg("a")
            .args(["--start-key-hex"])
            .arg("61")
            .assert()
            .failure()
            .stderr(predicate::str::contains("--start-key"))
            .stderr(predicate::str::contains("--start-key-hex"));

        // Test start with hex value
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--start-key-hex"])
            .arg("62")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("\'b"));

        // Test stop with hex value
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(db_path)
            .args(["--stop-key-hex"])
            .arg("62")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("\'a"))
            .stdout(predicate::str::contains("Next key is c"))
            .stdout(predicate::str::contains("--start-key=c"))
            .stdout(predicate::str::contains("--start-key-hex=63"));
    });
}

#[test]
fn fwdctl_check_empty_db() {
    with_tmpdir(|db_path| {
        create_db(db_path);

        cargo_bin_cmd!()
            .arg("check")
            .arg("--db")
            .arg(db_path)
            .assert()
            .success();
    });
}

#[test]
fn test_slow_fwdctl_check_db_with_data() {
    use rand::{RngExt, distr::Alphanumeric};

    with_tmpdir(|db_path| {
        let rng = firewood_storage::SeededRng::from_env_or_random();
        let mut sample_iter = rng.sample_iter(Alphanumeric).map(char::from);

        create_db(db_path);

        // TODO(#2047): bulk loading data instead of inserting one by one
        for _ in 0..4 {
            let key = sample_iter.by_ref().take(64).collect::<String>();
            let value = sample_iter.by_ref().take(10).collect::<String>();
            insert_key_value(db_path, &key, &value);
        }

        cargo_bin_cmd!()
            .arg("check")
            .arg("--db")
            .arg(db_path)
            .assert()
            .success();
    });
}

#[test]
fn test_slow_fwdctl_import_csv() {
    with_tmpdir(|tmp_dir| {
        let db_path1 = tmp_dir.join("db1");
        create_db(&db_path1);
        insert_key_value(&db_path1, "a", "1");
        insert_key_value(&db_path1, "b", "2");
        insert_key_value(&db_path1, "c", "3");

        let dump_file = tmp_dir.join("dump.csv");

        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path1)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file)
            .assert()
            .success();

        let db_path2 = tmp_dir.join("db2");
        cargo_bin_cmd!()
            .arg("import")
            .arg("--db")
            .arg(&db_path2)
            .args(["--input-format", "csv"])
            .args(["--input-file-name"])
            .arg(&dump_file)
            .assert()
            .success();

        let dump_file2 = tmp_dir.join("dump2.csv");
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path2)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file2)
            .assert()
            .success();

        let contents1 = fs::read_to_string(&dump_file).expect("Should read dump file");
        let contents2 = fs::read_to_string(&dump_file2).expect("Should read dump file 2");
        assert_eq!(contents1, contents2);
    });
}

#[test]
fn test_slow_fwdctl_import_csv_hex() {
    with_tmpdir(|tmp_dir| {
        let db_path1 = tmp_dir.join("db1");
        create_db(&db_path1);
        insert_key_value(&db_path1, "a", "1");
        insert_key_value(&db_path1, "b", "2");

        let dump_file = tmp_dir.join("dump.csv");

        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path1)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file)
            .arg("--hex")
            .assert()
            .success();

        let db_path2 = tmp_dir.join("db2");
        cargo_bin_cmd!()
            .arg("import")
            .arg("--db")
            .arg(&db_path2)
            .args(["--input-format", "csv"])
            .args(["--input-file-name"])
            .arg(&dump_file)
            .arg("--hex")
            .assert()
            .success();

        let dump_file2 = tmp_dir.join("dump2.csv");
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path2)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file2)
            .arg("--hex")
            .assert()
            .success();

        let contents1 = fs::read_to_string(&dump_file).expect("Should read dump file");
        let contents2 = fs::read_to_string(&dump_file2).expect("Should read dump file 2");
        assert_eq!(contents1, contents2);
    });
}

#[test]
fn test_slow_fwdctl_import_csv_malformed() {
    with_tmpdir(|tmp_dir| {
        let db_path = tmp_dir.join("db");
        create_db(&db_path);

        let dump_file = tmp_dir.join("malformed.csv");
        // Row 1: good
        // Row 2: 1 column (skip)
        // Row 3: good
        // Row 4: 3 columns (skip because strict deserialize enforces exactly 2)
        // Row 5: invalid hex in key (skip)
        fs::write(&dump_file, "61,31\n62\n63,33\n64,34,35\nzz,36\n").unwrap();

        cargo_bin_cmd!()
            .arg("import")
            .arg("--db")
            .arg(&db_path)
            .args(["--input-file-name"])
            .arg(&dump_file)
            .arg("--hex")
            .assert()
            .success()
            .stdout(predicate::str::contains("Successfully imported 2 keys"));
    });
}

#[test]
fn test_slow_fwdctl_import_large_random_database() {
    with_tmpdir(|tmp_dir| {
        //Generate a large random database dump
        let dump_file1 = tmp_dir.join("bulk1.csv");
        let mut csv_content = String::with_capacity(100_000 * 25);
        for i in 0..100_000 {
            // Using standard key/value pairs
            let _ = writeln!(csv_content, "key_{i:06},value_{i:06}");
        }
        fs::write(&dump_file1, csv_content).unwrap();

        // Import it into db1
        let db_path1 = tmp_dir.join("db1");
        cargo_bin_cmd!()
            .arg("import")
            .arg("--db")
            .arg(&db_path1)
            .args(["--input-file-name"])
            .arg(&dump_file1)
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "Successfully imported 100000 keys",
            ));

        // Export db1 to dump_file2
        let dump_file2 = tmp_dir.join("bulk2.csv");
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path1)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file2)
            .assert()
            .success();

        // Import dump_file2 into db2
        let db_path2 = tmp_dir.join("db2");
        cargo_bin_cmd!()
            .arg("import")
            .arg("--db")
            .arg(&db_path2)
            .args(["--input-file-name"])
            .arg(&dump_file2)
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "Successfully imported 100000 keys",
            ));

        // Export db2 to dump_file3
        let dump_file3 = tmp_dir.join("bulk3.csv");
        cargo_bin_cmd!()
            .arg("dump")
            .arg("--db")
            .arg(&db_path2)
            .args(["--output-format", "csv"])
            .args(["--output-file-name"])
            .arg(&dump_file3)
            .assert()
            .success();

        //Compare the two exported databases to ensure exact match!
        let contents1 = fs::read_to_string(&dump_file2).expect("Should read dump file 2");
        let contents2 = fs::read_to_string(&dump_file3).expect("Should read dump file 3");
        assert_eq!(contents1, contents2, "The exported databases do not match!");
    });
}
