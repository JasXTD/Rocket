//! Local disposable fixture only. Build with the installed rustc; no dependencies.
use std::{
    env, fs,
    io::{self, Write},
    process::{Command, exit},
    thread,
    time::Duration,
};
fn main() {
    let a: Vec<String> = env::args().collect();
    if a.get(1).map(String::as_str) == Some("child") {
        fs::write(&a[2], std::process::id().to_string()).unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    assert!(
        std::path::Path::new(".git").is_dir(),
        "fixture repository required"
    );
    let mode = a.get(1).map(String::as_str).unwrap_or("version");
    println!("PRIVATE_AUTO_OUTPUT 測試 🚀");
    match mode {
        "version" => {
            let n: u32 = fs::read_to_string("版本 文.txt")
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            exit(if n < 4 { 0 } else { 1 });
        }
        "skip" => exit(125),
        "exit" => exit(a[2].parse().unwrap()),
        "dirty" => {
            fs::write("版本 文.txt", "PRIVATE_EXTERNAL_EDIT").unwrap();
        }
        "generated" => {
            fs::write("generated 文.txt", "PRIVATE_GENERATED").unwrap();
            exit(125);
        }
        "output" => {
            print!("{}", "x".repeat(100_000));
            io::stdout().flush().unwrap();
            exit(125);
        }
        "flood" => loop {
            print!("{}", "x".repeat(1024 * 1024));
            io::stdout().flush().unwrap();
        },
        "hold" | "orphan" => {
            if std::path::Path::new(&a[2]).exists() {
                fs::remove_file(&a[2]).unwrap();
            }
            let mut cmd = Command::new(env::current_exe().unwrap());
            cmd.args(["child", &a[2]]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let mut child = cmd.spawn().unwrap();
            while !std::path::Path::new(&a[2]).exists() {
                thread::sleep(Duration::from_millis(5));
            }
            fs::write(format!("{}.parent", &a[2]), std::process::id().to_string()).unwrap();
            if mode == "orphan" {
                exit(125);
            }
            let _ = child.wait();
        }
        "gate" => {
            fs::write(&a[2], std::process::id().to_string()).unwrap();
            while !std::path::Path::new(&a[3]).exists() {
                thread::sleep(Duration::from_millis(10));
            }
        }
        _ => panic!("unknown fixture mode"),
    }
}
