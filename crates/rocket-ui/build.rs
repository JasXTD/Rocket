// Embeds `src/rocket.ico` in the package's executables as their file icon, so File
// Explorer, shortcuts and pinned taskbar entries show the rocket. The script writes the
// 32-bit resource (.res) file itself, laid out as rc.exe compiles `1 ICON "rocket.ico"`:
// one RT_ICON per image and an RT_GROUP_ICON listing them. The MSVC linker takes the
// .res as an input, so the build needs no Windows SDK tool and no build dependency.
// `crates/rocket-mcp/build.rs` includes this file, so it has no `//!` comments or inner
// attributes, and `include_bytes!` finds the icon relative to this file.
use std::{env, fs, path::PathBuf};

const ICON: &[u8] = include_bytes!("src/rocket.ico");
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // A two-stage build avoids a worker -> host -> worker digest cycle. An
    // ordinary unbound cargo build compiles, but refuses Lua at runtime.
    println!("cargo:rerun-if-env-changed=ROCKET_LUA_WORKER_SHA256");
    let worker_hash = env::var("ROCKET_LUA_WORKER_SHA256").unwrap_or_default();
    assert!(worker_hash.is_empty() || worker_hash.len() == 64 && worker_hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "ROCKET_LUA_WORKER_SHA256 must be a SHA-256 digest");
    println!("cargo:rustc-env=ROCKET_LUA_WORKER_SHA256={}", worker_hash.to_ascii_lowercase());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        println!("cargo:warning=No file icon: the icon resource is linked only for MSVC targets");
        return;
    }
    let word = |at: usize| u16::from_le_bytes([ICON[at], ICON[at + 1]]);
    let dword = |at: usize| u32::from_le_bytes(ICON[at..at + 4].try_into().unwrap()) as usize;
    let count = word(4);
    assert!(
        word(0) == 0 && word(2) == 1 && count > 0 && ICON.len() >= 6 + 16 * count as usize,
        "src/rocket.ico is not an icon file"
    );
    // An empty first entry marks the 32-bit resource format.
    let mut res = Vec::new();
    resource(&mut res, 0, 0, 0, &[]);
    let mut group = Vec::new();
    for field in [0, 1, count] {
        group.extend(field.to_le_bytes());
    }
    for id in 1..=count {
        let entry = 6 + 16 * (id as usize - 1);
        let (len, offset) = (dword(entry + 8), dword(entry + 12));
        let image = ICON
            .get(offset..offset + len)
            .expect("src/rocket.ico names an image past its end");
        // Memory flags as rc.exe writes them (ignored by Windows).
        resource(&mut res, RT_ICON, id, 0x1010, image);
        // A group entry is the file's directory entry with the image's resource ID
        // in place of its offset.
        group.extend(&ICON[entry..entry + 12]);
        group.extend(id.to_le_bytes());
    }
    resource(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);
    let path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("rocket.res");
    fs::write(&path, res).expect("write the icon resource");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

/// Appends a resource with a numeric type and name, in the neutral language, padded to
/// a 4-byte boundary.
fn resource(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes());
    // Header size, then type and name as 0xFFFF and the ordinal.
    res.extend(32u32.to_le_bytes());
    for ordinal in [kind, name] {
        res.extend([0xFF, 0xFF]);
        res.extend(ordinal.to_le_bytes());
    }
    // Data version, memory flags, language, version and characteristics.
    res.extend(0u32.to_le_bytes());
    res.extend(flags.to_le_bytes());
    res.extend([0; 10]);
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}
