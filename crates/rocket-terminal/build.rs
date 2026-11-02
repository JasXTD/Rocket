//! Package the reviewed runtime and compile our small Rust startup helper.
//! Uses Cargo's existing compiler; no downloads or shell execution.
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let architecture = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let architecture = match architecture.as_str() {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        other => panic!("No reviewed ConPTY runtime for {other}"),
    };
    let relative = "conpty/1.25.260930003";
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../runtime")
        .join(relative);
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let profile = output.ancestors().nth(3).expect("Cargo output profile");
    assert_eq!(
        output.ancestors().nth(2).unwrap().file_name().unwrap(),
        "build"
    );
    let helper_source =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src/host.rs");
    println!("cargo:rerun-if-changed={}", helper_source.display());
    println!("cargo:rerun-if-changed={}", helper_source.parent().unwrap().join("../../rocket-git/src/process_privilege.rs").display());
    let helper = output.join("rocket-terminal-host.exe");
    let status = Command::new(env::var_os("RUSTC").expect("Cargo Rust compiler"))
        .args([
            "--crate-name",
            "rocket_terminal_host",
            "--edition=2024",
            "--crate-type=bin",
            "--target",
        ])
        .arg(env::var_os("TARGET").unwrap())
        .args(["-C", "opt-level=1", "-o"])
        .arg(&helper)
        .args([
            "-C",
            if env::var("DEBUG").as_deref() == Ok("true") {
                "debuginfo=2"
            } else {
                "debuginfo=0"
            },
        ])
        .arg(&helper_source)
        .status()
        .expect("Build the owned terminal helper");
    assert!(status.success(), "Terminal helper compilation failed");
    let helper_bytes = fs::read(&helper).expect("Read terminal helper");
    for directory in [
        profile.to_path_buf(),
        profile.join("deps"),
        profile.join("examples"),
    ] {
        fs::create_dir_all(&directory).expect("Create terminal output directory");
        let destination = directory.join("rocket-terminal-host.exe");
        if fs::read(&destination).ok().as_deref() != Some(helper_bytes.as_slice()) {
            fs::copy(&helper, &destination).expect("Package the owned terminal helper");
        }
    }
    for file in ["conpty.dll", "OpenConsole.exe", "LICENSE"] {
        let from = if file == "LICENSE" {
            source.join(file)
        } else {
            source.join(architecture).join(file)
        };
        println!("cargo:rerun-if-changed={}", from.display());
        let bytes = fs::read(&from)
            .unwrap_or_else(|e| panic!("Missing reviewed runtime {}: {e}", from.display()));
        // Cargo executables, integration tests and examples each resolve the
        // runtime beside their own executable, never from a repository or PATH.
        for directory in [
            profile.to_path_buf(),
            profile.join("deps"),
            profile.join("examples"),
        ] {
            let directory = directory.join(relative);
            fs::create_dir_all(&directory).expect("Create runtime output directory");
            let destination = directory.join(file);
            // Do not rewrite DLLs in use when their bytes have not changed.
            if fs::read(&destination).ok().as_deref() != Some(bytes.as_slice()) {
                fs::copy(&from, &destination)
                    .unwrap_or_else(|e| panic!("Package {}: {e}", destination.display()));
            }
        }
    }
    println!("cargo:rustc-env=ROCKET_CONPTY_DIRECTORY={relative}");
}
