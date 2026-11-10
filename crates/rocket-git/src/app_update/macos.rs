//! Mac update trust and parent lifetime. No downloaded executable runs during verification.
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// A concurrent directory must never be overwritten during install or recovery.
pub fn move_directory(from: &Path, to: &Path) -> Result<()> {
    crate::local_files::move_new(from, to)
        .map_err(|e| crate::local_files::no_replace_error(e, "move Mac update bundle"))
}

#[repr(C)]
#[derive(Default)]
struct Event {
    ident: u64,
    filter: i16,
    flags: u16,
    fflags: u32,
    data: i64,
    udata: u64,
    ext: [u64; 2],
}
#[repr(C)]
struct Timespec {
    seconds: i64,
    nanos: i64,
}
unsafe extern "C" {
    fn kqueue() -> i32;
    fn kevent64(
        kq: i32,
        changes: *const Event,
        count: i32,
        events: *mut Event,
        capacity: i32,
        flags: u32,
        timeout: *const Timespec,
    ) -> i32;
    fn getppid() -> i32;
}

pub(super) fn wait_for_parent(parent: u32, ready: &Path, cancel: &Path) -> Result<()> {
    // The helper is a direct child; the parent waits for readiness before closing.
    // Register NOTE_EXIT before readiness, rather than polling a reusable PID.
    if parent == std::process::id() || unsafe { getppid() } as u32 != parent {
        return Err(invalid(
            "The original Rocket parent is unavailable. Retry from Rocket.",
        ));
    }
    let fd = unsafe { kqueue() };
    if fd < 0 {
        return Err(Error::io(
            "create update process notification",
            std::io::Error::last_os_error(),
        ));
    }
    let queue = unsafe { OwnedFd::from_raw_fd(fd) };
    let registration = Event {
        ident: parent.into(),
        filter: -5,
        flags: 0x15,
        fflags: 0x80000000,
        ..Event::default()
    };
    if unsafe {
        kevent64(
            queue.as_raw_fd(),
            &registration,
            1,
            std::ptr::null_mut(),
            0,
            0,
            std::ptr::null(),
        )
    } < 0
    {
        return Err(invalid(
            "Could not observe Rocket's process. Nothing was replaced.",
        ));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(ready)
        .map_err(|e| Error::io("announce updater readiness", e))?;
    let until = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        if cancel.exists() {
            return Err(Error::Cancelled);
        }
        let mut event = Event::default();
        let delay = Timespec {
            seconds: 0,
            nanos: 50_000_000,
        };
        let count = unsafe {
            kevent64(
                queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                0,
                &delay,
            )
        };
        if count > 0 {
            if event.flags & 0x4000 != 0
                || event.ident != u64::from(parent)
                || event.fflags & 0x80000000 == 0
            {
                return Err(invalid(
                    "Invalid update process notification. Nothing was replaced.",
                ));
            }
            return Ok(());
        }
        if count < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(invalid("Could not wait for Rocket to exit."));
        }
        if std::time::Instant::now() > until {
            return Err(invalid(
                "Rocket did not exit; the update left existing files unchanged.",
            ));
        }
    }
}

fn run(name: &str, args: &[OsString], cancel: &CancellationToken) -> Result<crate::runner::Output> {
    let executable = system_program(name)?;
    let mut runner = UpdateClient::new(UpdateEndpoint::Public, None)?.runner;
    runner.executable = executable.clone();
    runner.limits = CommandLimits {
        timeout: Duration::from_secs(45),
        stdout_bytes: 8192,
        stderr_bytes: 16384,
    };
    runner.tool(
        &executable,
        "verify-macos-update",
        Path::new("/"),
        args,
        Vec::new(),
        cancel,
    )
}
fn team_from_display(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut ids = text
        .lines()
        .filter_map(|line| line.strip_prefix("TeamIdentifier="));
    let id = ids.next()?;
    (ids.next().is_none()
        && id.len() == 10
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
    .then_some(id)
}
fn requirement(team: &str, notarized: bool) -> String {
    format!(
        "=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{team}\" and identifier \"io.github.jasxtd.rocket\"{}",
        if notarized { " and notarized" } else { "" }
    )
}
fn verify(path: &Path, requirement: &str, cancel: &CancellationToken) -> Result<()> {
    let output = run(
        "codesign",
        &[
            "--verify".into(),
            "--deep".into(),
            "--strict".into(),
            "--all-architectures".into(),
            "--test-requirement".into(),
            requirement.into(),
            path.as_os_str().into(),
        ],
        cancel,
    )?;
    if !output.status.success() {
        return Err(invalid(
            "Mac update signature, publisher identity or notarization is invalid. The existing app is unchanged.",
        ));
    }
    Ok(())
}
/// Trust is anchored in the running Rocket signature, never in a download or plan.
/// Ad-hoc development builds deliberately cannot install public updates.
pub fn verify_app(
    app: &Path,
    version: Option<UpdateVersion>,
    cancel: &CancellationToken,
) -> Result<()> {
    cancel.check()?;
    let host = std::env::current_exe().map_err(|e| Error::io("locate signed Rocket", e))?;
    let display = run(
        "codesign",
        &[
            "--display".into(),
            "--verbose=4".into(),
            host.as_os_str().into(),
        ],
        cancel,
    )?;
    let team = team_from_display(&display.stderr).filter(|_| display.status.success())
        .ok_or_else(|| invalid("Mac in-app installation requires a Developer ID signed Rocket. This development build cannot install public updates."))?;
    let host_rule = requirement(team, false).replace("identifier \"io.github.jasxtd.rocket\"", "(identifier \"io.github.jasxtd.rocket\" or identifier \"io.github.jasxtd.rocket.rocket-mcp\")");
    verify(&host, &host_rule, cancel)?;
    verify(app, &requirement(team, true), cancel)?;
    for name in [
        "rocket",
        "rocket-cli",
        "rocket-mcp",
        "rocket-rebase-editor",
        "rocket-ssh",
    ] {
        use std::io::Read;
        let mut header = [0u8; 8];
        std::fs::File::open(app.join("Contents/MacOS").join(name))
            .and_then(|mut file| file.read_exact(&mut header))
            .map_err(|e| Error::io("read Mac update architecture", e))?;
        if !super::macos_executable_matches(&header, std::env::consts::ARCH) {
            return Err(invalid(
                "Mac update executables must match this Mac architecture. Nothing was replaced.",
            ));
        }
    }
    let property = |key: &str| -> Result<String> {
        let result = run(
            "plutil",
            &[
                "-extract".into(),
                key.into(),
                "raw".into(),
                "-o".into(),
                "-".into(),
                app.join("Contents/Info.plist").into_os_string(),
            ],
            cancel,
        )?;
        if !result.status.success() {
            return Err(invalid("Mac update bundle metadata is invalid."));
        }
        String::from_utf8(result.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|_| invalid("Mac update metadata is not Unicode."))
    };
    let bundle_version = property("CFBundleShortVersionString")?;
    if property("CFBundleIdentifier")? != "io.github.jasxtd.rocket"
        || property("CFBundleExecutable")? != "rocket"
        || version.is_some_and(|v| bundle_version != v.text())
    {
        return Err(invalid(
            "Mac update bundle identity or version does not match the reviewed release.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publisher_fields_are_strict_and_notarization_is_mandatory() {
        assert_eq!(
            team_from_display(b"TeamIdentifier=AB12CD34EF\n"),
            Some("AB12CD34EF")
        );
        for s in [
            "TeamIdentifier=not set\n",
            "TeamIdentifier=abc1234567\n",
            "TeamIdentifier=AB12CD34EF\nTeamIdentifier=AB12CD34EF\n",
            "TeamIdentifier=\" or true",
        ] {
            assert_eq!(team_from_display(s.as_bytes()), None);
        }
        assert!(requirement("AB12CD34EF", true).ends_with(" and notarized"));
    }
    #[test]
    fn development_host_and_cancel_cannot_authorize_update() {
        let cancel = CancellationToken::new();
        assert!(verify_app(Path::new("/does-not-exist/Rocket.app"), None, &cancel).is_err());
        cancel.cancel();
        assert!(matches!(
            verify_app(Path::new("/does-not-exist/Rocket.app"), None, &cancel),
            Err(Error::Cancelled)
        ));
    }
}
