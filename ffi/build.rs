use std::env;
use std::error::Error;
use std::fs;

extern crate cbindgen;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set");

    let config = cbindgen::Config::from_file("cbindgen.toml").expect("cbindgen.toml is present");

    fixup_go().expect("couldn't fix up go");

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        // Add any additional configuration options here
        .with_config(config)
        .generate()
        .map_or_else(
            |error| match error {
                cbindgen::Error::ParseSyntaxError { .. } => {}
                e => panic!("{e:?}"),
            },
            |bindings| {
                bindings.write_to_file("firewood.h");
            },
        );
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed=PROFILE");
}
fn fixup_go() -> Result<(), Box<dyn Error>> {
    for direntry in fs::read_dir(".")?.filter(|direntry| {
        direntry.as_ref().ok().is_some_and(|entry| {
            let path = entry.path();
            path.extension() == Some(std::ffi::OsStr::new("go"))
                && !path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .ends_with("_test")
        })
    }) {
        let build_profile = get_build_profile_name();

        let path = direntry?.path();
        let contents = fs::read_to_string(&path)?;
        let mut new_contents = String::new();
        let mut changed = false;
        for line in contents.lines() {
            if line.starts_with("// #cgo LDFLAGS: -L")
                || line.starts_with("// // ignore LDFLAGS: -L")
            {
                let lib = line.split(' ').next_back();

                let change = match lib {
                    Some("-L${SRCDIR}/../target/debug") if build_profile == "debug" => {
                        Some("// #cgo LDFLAGS: -L${SRCDIR}/../target/debug")
                    }
                    Some("-L${SRCDIR}/../target/debug") => {
                        Some("// // ignore LDFLAGS: -L${SRCDIR}/../target/debug")
                    }
                    Some("-L${SRCDIR}/../target/release") if build_profile == "release" => {
                        Some("// #cgo LDFLAGS: -L${SRCDIR}/../target/release")
                    }
                    Some("-L${SRCDIR}/../target/release") => {
                        Some("// // ignore LDFLAGS: -L${SRCDIR}/../target/release")
                    }
                    Some("-L${SRCDIR}/../target/maxperf") if build_profile == "maxperf" => {
                        Some("// #cgo LDFLAGS: -L${SRCDIR}/../target/maxperf")
                    }
                    Some("-L${SRCDIR}/../target/maxperf") => {
                        Some("// // ignore LDFLAGS: -L${SRCDIR}/../target/maxperf")
                    }
                    _ => None,
                };
                if let Some(change) = change {
                    new_contents.push_str(change);
                    changed = true;
                } else {
                    new_contents.push_str(line);
                }
            } else {
                new_contents.push_str(line);
            }
            new_contents.push('\n');
        }
        println!("cargo::rerun-if-changed={}", path.display());
        if changed {
            fs::write(path, new_contents)?;
        }
    }
    Ok(())
}

fn get_build_profile_name() -> String {
    // The profile name is always the 3rd last part of the path (with 1 based indexing).
    // e.g. /code/core/target/cli/build/my-build-info-9f91ba6f99d7a061/out
    std::env::var("OUT_DIR")
        .expect("OUT_DIR is not set")
        .split(std::path::MAIN_SEPARATOR)
        .nth_back(3)
        .unwrap_or("unknown")
        .to_string()
}
