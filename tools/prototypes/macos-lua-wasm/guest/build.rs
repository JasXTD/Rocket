fn main() {
    println!("cargo:rerun-if-changed=deny-wasi.c");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        cc::Build::new()
            .file("deny-wasi.c")
            .warnings(true)
            .compile("rocket-deny-wasi");
    }
    println!("cargo:rerun-if-changed=build.rs");
    lua_src::Build::new()
        .build(lua_src::Lua55)
        .print_cargo_metadata();
}
