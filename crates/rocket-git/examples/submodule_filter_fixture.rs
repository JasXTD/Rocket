//! Isolated acceptance-test filter. Never configured by the application.
use std::{
    fs,
    io::{self, Read, Write},
    thread,
    time::Duration,
};
fn main() {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes).unwrap();
    if let (Ok(started), Ok(gate)) = (
        std::env::var("ROCKET_SUBMODULE_STARTED"),
        std::env::var("ROCKET_SUBMODULE_GATE"),
    ) {
        fs::write(started, std::process::id().to_string()).unwrap();
        while !std::path::Path::new(&gate).exists() {
            thread::sleep(Duration::from_millis(20));
        }
    }
    io::stdout().write_all(&bytes).unwrap();
}
