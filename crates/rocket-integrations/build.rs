fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "runtime")]
    lua_src::Build::new()
        .build(lua_src::Lua55)
        .print_cargo_metadata();
}
