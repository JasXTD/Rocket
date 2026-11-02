//! Deterministic test application. No network, shell, packages or user configuration.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = args.first().and_then(|s| s.to_str()).unwrap_or("");
    match mode {
        "merge" | "empty" | "binary" | "large" | "fail" | "missing" => {
            assert!(args.len() == 5, "mode base ours theirs result");
            for p in &args[1..4] {
                assert!(fs::metadata(p).unwrap().permissions().readonly());
            }
            let bytes: Vec<u8> = match mode {
                "empty" => Vec::new(),
                "binary" => vec![0, 255, 0],
                "large" => vec![b'x'; 524289],
                _ => "resolved 世界 🚀 é\n".as_bytes().to_vec(),
            };
            fs::write(&args[4], bytes).unwrap();
            if mode == "missing" {
                fs::remove_file(&args[4]).unwrap();
            }
            if mode == "fail" {
                std::process::exit(7);
            }
        }
        "diff" => {
            assert_eq!(args.len(), 3);
            let left = PathBuf::from(&args[1]);
            let dir = left.parent().unwrap();
            fs::write(dir.join("observed-left"), fs::read(&args[1]).unwrap()).unwrap();
            fs::write(dir.join("observed-right"), fs::read(&args[2]).unwrap()).unwrap();
            assert!(fs::OpenOptions::new().write(true).open(&args[1]).is_err());
            assert!(fs::OpenOptions::new().write(true).open(&args[2]).is_err());
        }
        "editor" => {
            assert_eq!(args.len(), 2);
            fs::write(&args[1], "edited locally 世界\n").unwrap();
        }
        "wait" => {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .arg("child")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(
                "owned-processes",
                format!("{} {}", std::process::id(), child.id()),
            )
            .unwrap();
            if let Some(marker) = std::env::var_os("ROCKET_EXTERNAL_MARKER") {
                fs::write(marker, format!("{} {}", std::process::id(), child.id())).unwrap();
            }
            for _ in 0..3000 {
                thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        "child" => loop {
            thread::sleep(Duration::from_secs(1));
        },
        _ => std::process::exit(9),
    }
}
