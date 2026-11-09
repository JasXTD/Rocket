// Shared build glue for first-party Cocoa adapters. No downloaded tools or crates.
pub fn build_objc(source: &str, name: &str) {
    println!("cargo:rerun-if-changed=../../tools/build-objc.rs");
    println!("cargo:rerun-if-changed={source}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if !std::env::var("HOST").unwrap().contains("apple-darwin") {
        // Rust-only `cargo check` can run on the development Windows host. No
        // substitute implementation is emitted: linking still requires this
        // archive and therefore a real Mac SDK build.
        println!(
            "cargo:warning=Cocoa adapter requires native Mac validation; cross-host cargo check checks Rust only"
        );
        println!("cargo:rustc-link-lib=static={name}");
        println!("cargo:rustc-link-lib=framework=AppKit");
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let object = out.join(format!("{name}.o"));
    let archive = out.join(format!("lib{name}.a"));
    let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        _ => panic!("Cocoa adapters currently require Apple Silicon"),
    };
    let status = std::process::Command::new("/usr/bin/xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "-arch",
            architecture,
            "-mmacosx-version-min=15.0",
            "-fobjc-arc",
            "-fblocks",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-c",
            source,
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("start Apple clang");
    assert!(status.success(), "Cocoa adapter compilation failed");
    assert!(
        std::process::Command::new("/usr/bin/ar")
            .arg("crs")
            .arg(&archive)
            .arg(object)
            .status()
            .expect("archive Cocoa adapter")
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static={name}");
    println!("cargo:rustc-link-lib=framework=AppKit");
}
