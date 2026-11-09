include!("../../tools/build-objc.rs");
fn main() {
    build_objc("src/macos.m", "rocket_spell_cocoa");
}
