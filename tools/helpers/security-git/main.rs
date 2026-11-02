//! Owned offline fixture: records protocol policy; never runs Git or opens a socket.
use std::{env, fs, process};
fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!(
            "git version {}",
            env::var("ROCKET_SECURITY_TEST_GIT_VERSION").expect("fixture version")
        );
        return;
    }
    if args.iter().any(|arg| arg == "clone") {
        let marker = env::var_os("ROCKET_SECURITY_TEST_MARKER").expect("fixture marker");
        fs::write(marker, env::var("GIT_ALLOW_PROTOCOL").unwrap_or_default()).unwrap();
        eprintln!("Owned fixture stopped before any Git or network operation.");
        process::exit(42);
    }
    eprintln!("Unsupported owned-fixture invocation.");
    process::exit(43);
}
