include!("../../tools/build-objc.rs");
fn main() {
    build_objc("src/macos_menu.m", "rocket_menu_cocoa");
}
