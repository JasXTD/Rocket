//! Local acceptance-test relay. Real Git/LFS paths and gates are supplied only by the test.
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, exit},
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let helper = env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("git-lfs");
    let mode = env::var_os("ROCKET_LFS_PUSH_MODE_FILE")
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let pushing = helper && args.first().is_some_and(|s| s == "push");
    let gate = || {
        fs::write(
            env::var_os("ROCKET_LFS_STARTED").unwrap(),
            std::process::id().to_string(),
        )
        .unwrap();
        let release = PathBuf::from(env::var_os("ROCKET_LFS_GATE").unwrap());
        while !release.exists() {
            thread::sleep(Duration::from_millis(10));
        }
    };
    if pushing {
        let failure = match mode.trim() {
            "auth" => Some(
                "batch response: Authorization error: PRIVATE_token https://private.invalid/repo",
            ),
            "denied" => Some("PRIVATE_path: status code: 403"),
            "reject" => Some("PRIVATE_path: upload rejected: status code: 422"),
            "quota" => Some("PRIVATE_path: repository exceeded its data quota"),
            "connection" => Some("PRIVATE_host: connection refused"),
            "unknown" => Some("PRIVATE_unknown server response"),
            _ => None,
        };
        if let Some(message) = failure {
            eprintln!("{message}");
            exit(2);
        }
        if mode.trim() == "pause_before" {
            gate();
        }
    }
    if helper && args.first().is_some_and(|s| s == "pointer") && mode.trim() == "pause_hash" {
        gate();
    }
    // Pause a real Git read only after the attributes recovery copy exists.
    // This makes cancellation/external-edit acceptance deterministic without
    // sleeps or test-only gates in production save code.
    if !helper
        && args.iter().any(|s| s == "ls-files")
        && let Some(root) = env::var_os("ROCKET_LFS_ATTRIBUTES_RECOVERY")
        && let Some(marker) = env::var_os("ROCKET_LFS_STARTED")
        && let Some(gate) = env::var_os("ROCKET_LFS_GATE")
    {
        let marker = PathBuf::from(marker);
        let gate = PathBuf::from(gate);
        let ready = fs::read_dir(root).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.path().join("RECOVERY.md").exists())
        });
        if ready && !marker.exists() && !gate.exists() {
            fs::write(&marker, std::process::id().to_string()).unwrap();
            while !gate.exists() {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
    if helper && args.first().is_some_and(|s| s == "version") {
        if let Some(pause) = env::var_os("ROCKET_LFS_VERSION_PAUSE") {
            let pause = PathBuf::from(pause);
            if pause.exists() {
                fs::write(
                    env::var_os("ROCKET_LFS_STARTED").unwrap(),
                    std::process::id().to_string(),
                )
                .unwrap();
                while pause.exists() {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        if let Some(version) = env::var_os("ROCKET_LFS_VERSION_FILE")
            && let Ok(version) = fs::read_to_string(version)
        {
            println!("git-lfs/{} (fixture)", version.trim());
            return;
        }
    }
    if helper
        && args.first().is_some_and(|s| s == "fetch")
        && let Some(marker) = env::var_os("ROCKET_LFS_STARTED")
    {
        fs::write(marker, std::process::id().to_string()).unwrap();
        let gate = PathBuf::from(env::var_os("ROCKET_LFS_GATE").expect("fixture gate"));
        while !gate.exists() {
            thread::sleep(Duration::from_millis(10));
        }
    }
    let exe = env::var_os(if helper {
        "ROCKET_LFS_REAL_HELPER"
    } else {
        "ROCKET_LFS_REAL_GIT"
    })
    .expect("fixture executable");
    let mut command = Command::new(exe);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let status = command.status().unwrap();
    // The real file adapter already wrote objects and progress. Hold only the
    // relay's exit to exercise late cancellation without production test hooks.
    if pushing && mode.trim() == "pause_after" && status.success() {
        gate();
    }
    exit(status.code().unwrap_or(1));
}
