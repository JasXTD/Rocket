fn main() {
    println!("cargo:rerun-if-changed=src/macos.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/macos.c")
            .flag("-std=c11")
            .warnings_into_errors(true)
            .compile("rocket_prototype_containment");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}
