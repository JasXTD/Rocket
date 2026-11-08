//! Shared by Rocket's settings page and its local MCP service. No third-party dependencies.
#[path = "../../rocket-git/src/process_privilege.rs"]
mod process_privilege;
#[cfg(target_os = "macos")]
#[path = "../../rocket-git/src/darwin_acl.rs"]
mod darwin_acl;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const DEFAULT_PORT: u16 = 47631;
pub const MAX_CONFIG: u64 = 16 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub root: PathBuf,
}
impl Config {
    pub fn address(&self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port))
    }
    pub fn endpoint(&self) -> String {
        format!("http://{}/mcp", self.address())
    }
    pub fn validate(&self) -> io::Result<()> {
        self.validate_format()?;
        if !self.root.is_dir() {
            return Err(invalid("Choose an existing fixture folder"));
        }
        Ok(())
    }
    fn validate_format(&self) -> io::Result<()> {
        if self.port < 1024
            || !self.root.is_absolute()
            || self.root.to_string_lossy().contains(['\r', '\n', '\0'])
        {
            return Err(invalid(
                "Use a port from 1024 to 65535 and an existing absolute fixture folder",
            ));
        }
        #[cfg(windows)]
        if self.root.to_string_lossy().starts_with("\\\\") {
            return Err(invalid("Use a local drive for the fixture folder"));
        }
        Ok(())
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
/// `what` names the file and `repair` says how to fix its permissions on Unix.
fn read_bounded(path: &Path, max: u64, what: &str, repair: &str) -> io::Result<String> {
    #[cfg(unix)]
    unix::profile_folder(path.parent().ok_or_else(|| invalid("Invalid MCP profile"))?)?;
    let file = private_options().read(true).open(path)?;
    #[cfg(unix)]
    unix::private_file(&file, what, repair)?;
    #[cfg(not(unix))]
    let _ = (what, repair);
    let mut text = String::new();
    file.take(max + 1).read_to_string(&mut text)?;
    if text.len() as u64 > max {
        return Err(invalid("MCP settings exceed their size limit"));
    }
    Ok(text)
}
pub fn load(profile: &Path) -> io::Result<Config> {
    let text = read_bounded(
        &profile.join("settings"),
        MAX_CONFIG,
        "The MCP settings",
        "Run chmod 600 on settings in the MCP profile folder, then try again.",
    )?;
    let fields: Vec<_> = text.lines().collect();
    if fields.len() != 3 || fields[0] != "ROCKET-MCP-1" {
        return Err(invalid("Invalid MCP settings; existing file preserved"));
    }
    let config = Config {
        port: fields[1].parse().map_err(|_| invalid("Invalid MCP port"))?,
        root: fields[2].into(),
    };
    config.validate_format()?;
    Ok(config)
}
pub fn token(profile: &Path) -> io::Result<String> {
    let token = read_bounded(
        &profile.join("access-token"),
        64,
        "The MCP access token",
        "Rotate it while the service is stopped: clients must then use the new token.",
    )?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(
            "Invalid MCP access token; rotate it while the service is stopped",
        ));
    }
    Ok(token)
}
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.sync_all();
    }
}
fn lock(profile: &Path) -> io::Result<Lock> {
    Ok(Lock(exclusive_file(profile, "settings.lock")?))
}
/// Held for the service lifetime; OS ownership is released on exit or crash.
pub fn service_lock(profile: &Path) -> io::Result<File> {
    exclusive_file(profile, "service.lock").map_err(|e| {
        io::Error::new(
            e.kind(),
            "MCP profile is unavailable or in use; stop its service before changing settings",
        )
    })
}
/// Create the profile folder if it is missing: on Unix 0700, and then checked to be this
/// user's alone. Create it this way before putting anything else in it.
pub fn create_profile(profile: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::create_profile(profile)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(profile)
    }
}
fn exclusive_file(profile: &Path, name: &str) -> io::Result<File> {
    create_profile(profile)?;
    let mut opts = private_options();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0);
        opts.open(profile.join(name))
    }
    #[cfg(unix)]
    {
        // An exclusive advisory lock, as Windows' sharing mode 0: it belongs to this
        // open file, so a second open in this or another process is refused, and the
        // system releases it when the process ends or crashes.
        let file = opts.open(profile.join(name))?;
        unix::private_file(&file, &format!("The MCP lock file {name}"), "Run chmod 600 on it, then try again.")?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::ResourceBusy,
                "MCP settings are in use by another Rocket window or service",
            ),
            fs::TryLockError::Error(error) => error,
        })?;
        Ok(file)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (name, opts);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MCP settings ownership currently requires Windows or Linux",
        ))
    }
}
/// Files this user alone can open: Unix creates them 0600. Windows profile files take
/// the user-only permissions of their folder.
fn private_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000); // O_NOFOLLOW; std also sets close-on-exec.
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100); // Darwin O_NOFOLLOW, not Linux's value.
    }
    options
}
fn replace(profile: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let unique = secret()?;
    let temporary = profile.join(format!("{name}-{}.tmp", &unique[..16]));
    let result = (|| {
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, profile.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn initialize(profile: &Path, root: &Path) -> io::Result<Config> {
    let _lock = lock(profile)?;
    let config = if profile.join("settings").exists() {
        load(profile)?
    } else {
        let config = Config {
            port: DEFAULT_PORT,
            root: root.to_path_buf(),
        };
        config.validate()?;
        write_config(profile, &config)?;
        config
    };
    if !profile.join("access-token").exists() {
        let _service = service_lock(profile)?;
        let value = secret()?;
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(profile.join("access-token"))?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
    }
    Ok(config)
}
fn write_config(profile: &Path, config: &Config) -> io::Result<()> {
    config.validate()?;
    let root = config
        .root
        .to_str()
        .ok_or_else(|| invalid("Fixture folder must be valid Unicode"))?;
    replace(
        profile,
        "settings",
        format!("ROCKET-MCP-1\n{}\n{root}\n", config.port).as_bytes(),
    )
}
pub fn save(profile: &Path, config: &Config) -> io::Result<()> {
    let _lock = lock(profile)?;
    let _service = service_lock(profile)?;
    write_config(profile, config)
}
pub fn rotate_token(profile: &Path) -> io::Result<()> {
    let _lock = lock(profile)?;
    let _service = service_lock(profile)?;
    replace(profile, "access-token", secret()?.as_bytes())
}
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    pub version: String,
    pub pid: u32,
    pub window: String,
}
pub fn status(profile: &Path) -> io::Result<Status> {
    let config = load(profile)?;
    let access_token = token(profile)?;
    // An unused Windows loopback port can time out before reporting refusal.
    // The lifetime lock plus a successful bind proves this profile is stopped;
    // a timeout by itself never does. Do not send credentials to another service.
    if let Ok(_ownership) = service_lock(profile) {
        return match TcpListener::bind(config.address()) {
            Ok(_available_port) => Ok(Status::default()),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => Err(invalid(
                "MCP port is already in use by another service; choose a different port",
            )),
            Err(error) => Err(error),
        };
    }
    match request(&config, &access_token, "GET", "/health") {
        Err(e) => Err(e),
        Ok(text) => {
            let fields: Vec<_> = text.lines().collect();
            if fields.len() != 4 || fields[0] != "rocket-mcp" {
                return Err(invalid(
                    "The port did not return a Rocket MCP health response",
                ));
            }
            Ok(Status {
                running: true,
                version: fields[1].into(),
                pid: fields[2]
                    .parse()
                    .map_err(|_| invalid("Invalid MCP health response"))?,
                window: fields[3].into(),
            })
        }
    }
}
pub fn stop(profile: &Path) -> io::Result<Status> {
    let config = load(profile)?;
    if !status(profile)?.running {
        return Ok(Status::default());
    }
    request(&config, &token(profile)?, "POST", "/shutdown")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(result) = status(profile)
            && !result.running
        {
            return Ok(result);
        }
        if Instant::now() >= deadline {
            return Err(invalid(
                "Stop is still pending; check status before retrying",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn start(profile: &Path, executable: &Path) -> io::Result<Status> {
    process_privilege::check()?;
    let _lock = lock(profile)?;
    load(profile)?.validate()?;
    let current = status(profile)?;
    if current.running {
        return Ok(current);
    }
    let mut command = Command::new(executable);
    command
        .args(["--http-profile"])
        .arg(profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    // Only startup/lifecycle messages; never headers, tokens, request bodies or paths.
    command.stderr(
        private_options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(profile.join("service.log"))?,
    );
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    // Its own process group, as the hidden console detaches it on Windows: Ctrl+C in
    // the terminal that started Rocket must not reach the service.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = child.try_wait()? {
            return Err(invalid(&format!(
                "MCP service exited ({exit}); inspect service.log"
            )));
        }
        if let Ok(state) = status(profile)
            && state.running
        {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(invalid(
                "MCP startup is unconfirmed; check status before retrying",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(all(test, windows))]
mod privilege_tests {
    use super::*;
    #[test]
    fn service_start_refuses_before_profile_writes_or_process_creation() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let profile = std::env::temp_dir().join(format!("rocket-mcp-refusal-{}-{nonce}", std::process::id()));
        assert!(!profile.exists());
        for state in [Err(()), Ok((true, 0x2000)), Ok((false, 0x3000)), Ok((true, 0x4000))] {
            process_privilege::with_test_token(state, || {
                let error = start(&profile, &profile.join("must-not-run.exe")).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                assert!(!profile.exists());
            });
        }
    }
}
fn request(config: &Config, token: &str, method: &str, route: &str) -> io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&config.address(), Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "{method} {route} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        config.address()
    )?;
    let mut response = String::new();
    stream.take(16 * 1024 + 1).read_to_string(&mut response)?;
    if response.len() > 16 * 1024 {
        return Err(invalid("MCP control response exceeded its limit"));
    }
    let (header, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| invalid("Invalid MCP control response"))?;
    let status = header.split_whitespace().nth(1).unwrap_or("");
    if !matches!(status, "200" | "202") {
        return Err(invalid(match status {
            "401" => "MCP access token does not match the running service",
            "409" => "Close the MCP test window before stopping the service",
            _ => "The MCP service rejected the control request; verify the port and service status",
        }));
    }
    Ok(body.into())
}
fn secret() -> io::Result<String> {
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        let mut bytes = [0u8; 32];
        // SAFETY: a valid, writable 32-byte buffer; null provider is required for
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG (2). No handle is acquired or retained.
        let result = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                2,
            )
        };
        if result < 0 {
            return Err(io::Error::other(
                "Windows could not generate an MCP access token",
            ));
        }
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn getrandom(buffer: *mut u8, length: usize, flags: u32) -> isize;
        }
        let mut bytes = [0u8; 32];
        let mut filled = 0;
        while filled < bytes.len() {
            // SAFETY: the rest of a writable 32-byte buffer. Flags 0: the kernel's
            // random source, waiting only until it is first initialized at boot.
            let read = unsafe { getrandom(bytes[filled..].as_mut_ptr(), bytes.len() - filled, 0) };
            if read < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(io::Error::new(
                    error.kind(),
                    format!("Linux could not generate an MCP access token: {error}"),
                ));
            }
            filled += read as usize;
        }
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(target_os = "macos")]
    {
        #[link(name = "Security", kind = "framework")]
        unsafe extern "C" {
            fn SecRandomCopyBytes(random: *const std::ffi::c_void, count: usize, bytes: *mut u8) -> i32;
        }
        let mut bytes = [0u8; 32];
        // SAFETY: kSecRandomDefault is NULL; the writable buffer has count bytes.
        // No keychain access, credentials, or persistent system configuration.
        let status = unsafe { SecRandomCopyBytes(std::ptr::null(), bytes.len(), bytes.as_mut_ptr()) };
        if status != 0 {
            return Err(io::Error::other(format!("macOS could not generate an MCP access token (status {status})")));
        }
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MCP token generation currently requires Windows, Linux or macOS",
        ))
    }
}

/// Unix: the profile folder and its files belong to this user, and other users can't
/// change the folder or open the files, as ssh requires of keys. Windows gets the same
/// from the user-only permissions of its local application data folder.
#[cfg(unix)]
mod unix {
    use super::invalid;
    use std::{
        fs::{self, DirBuilder, File},
        io,
        os::unix::fs::{DirBuilderExt, MetadataExt},
        path::Path,
    };
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    fn user() -> u32 {
        // SAFETY: no arguments; always succeeds.
        unsafe { geteuid() }
    }
    fn refused(message: String) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, message)
    }
    /// Create the profile folder 0700 if it is missing (its parents as the user's
    /// umask makes them), then check it.
    pub(super) fn create_profile(profile: &Path) -> io::Result<()> {
        if let Some(parent) = profile.parent() {
            fs::create_dir_all(parent)?;
        }
        match DirBuilder::new().mode(0o700).create(profile) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        profile_folder(profile)
    }
    /// A real folder of this user that no other user can write to.
    pub(super) fn profile_folder(profile: &Path) -> io::Result<()> {
        let found = fs::symlink_metadata(profile)?;
        if !found.is_dir() {
            return Err(invalid("The MCP profile must be a folder, not a link or a file"));
        }
        if found.uid() != user() {
            return Err(refused(format!("The MCP profile folder {} belongs to another user", profile.display())));
        }
        if found.mode() & 0o022 != 0 {
            return Err(refused(format!(
                "Other users can change the MCP profile folder {}. Run chmod 700 on it, then try again",
                profile.display()
            )));
        }
        #[cfg(target_os = "macos")]
        {
            let file = super::private_options().read(true).open(profile)?;
            super::darwin_acl::check(&file, profile, &found)?;
        }
        Ok(())
    }
    /// An opened profile file: a regular file of this user that only this user can open.
    /// `what` names it and `repair` says how to fix its permissions.
    pub(super) fn private_file(file: &File, what: &str, repair: &str) -> io::Result<()> {
        let found = file.metadata()?;
        if !found.is_file() {
            return Err(invalid(&format!("{what} must be a regular file")));
        }
        if found.uid() != user() {
            return Err(refused(format!("{what} belongs to another user")));
        }
        if found.mode() & 0o077 != 0 {
            return Err(refused(format!(
                "{what} can be opened by other users (mode {:03o}). {repair}",
                found.mode() & 0o777
            )));
        }
        #[cfg(target_os = "macos")]
        super::darwin_acl::check(file, Path::new(what), &found)?;
        Ok(())
    }
}

#[cfg(all(test, any(windows, target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("rocket-mcp-control-{}", secret().unwrap()));
        fs::create_dir(&directory).unwrap();
        directory
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn profile_files_refuse_leaf_links_and_preserve_the_target() {
        use std::os::unix::fs::symlink;
        let root = fixture();
        let profile = root.join("profile");
        initialize(&profile, &root).unwrap();
        let target = root.join("unchanged");
        fs::write(&target, "private fixture").unwrap();
        for name in ["access-token", "settings.lock", "service.lock"] {
            fs::remove_file(profile.join(name)).unwrap();
            symlink(&target, profile.join(name)).unwrap();
            let refused = match name {
                "access-token" => token(&profile).is_err(),
                "settings.lock" => lock(&profile).is_err(),
                _ => service_lock(&profile).is_err(),
            };
            assert!(refused, "{name}");
            assert_eq!(fs::read_to_string(&target).unwrap(), "private fixture");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_token_and_profile_acl_grants_are_refused_without_changing_permissions() {
        let root = fixture();
        let profile = root.join("profile");
        initialize(&profile, &root).unwrap();
        let file = profile.join("access-token");
        let before = fs::read(&file).unwrap();
        for path in [&file, &profile] {
            assert!(Command::new("/bin/chmod").args(["+a", "everyone allow read"]).arg(path).status().unwrap().success());
            let result = token(&profile);
            let permissions = Command::new("/bin/ls").arg("-lde").arg(path).output().unwrap().stdout;
            assert!(result.is_err(), "ACL access grants must not be authorized by mode bits");
            assert_eq!(fs::read(&file).unwrap(), before);
            assert_eq!(Command::new("/bin/ls").arg("-lde").arg(path).output().unwrap().stdout, permissions);
            assert!(Command::new("/bin/chmod").arg("-N").arg(path).status().unwrap().success());
            assert!(token(&profile).is_ok());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unix_profiles_are_private_to_their_user_and_locks_are_exclusive() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777;
        let set = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        let root = fixture();
        let profile = root.join("profile");
        initialize(&profile, &root).unwrap();
        assert_eq!(mode(&profile), 0o700);
        for file in ["settings", "access-token", "settings.lock", "service.lock"] {
            assert_eq!(mode(&profile.join(file)), 0o600, "{file}");
        }
        let first = token(&profile).unwrap();
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()) && first.len() == 64);
        rotate_token(&profile).unwrap();
        assert_ne!(token(&profile).unwrap(), first, "the OS gives a new token");
        assert_eq!(mode(&profile.join("access-token")), 0o600, "rotation keeps it private");

        // A lock belongs to its open file: refused in this process too, released on drop.
        let held = service_lock(&profile).unwrap();
        assert_eq!(service_lock(&profile).unwrap_err().kind(), io::ErrorKind::ResourceBusy);
        drop(held);
        drop(service_lock(&profile).unwrap());

        // A token other users can read is refused until it is rotated.
        set(&profile.join("access-token"), 0o644);
        let error = token(&profile).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("Rotate it"), "{error}");
        assert!(status(&profile).is_err());
        rotate_token(&profile).unwrap();
        assert!(token(&profile).is_ok());
        set(&profile.join("settings"), 0o640);
        assert!(load(&profile).unwrap_err().to_string().contains("chmod 600"));
        set(&profile.join("settings"), 0o600);
        assert!(load(&profile).is_ok());

        // A folder other users can write to, or a link to one, is refused.
        set(&profile, 0o770);
        assert_eq!(load(&profile).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(service_lock(&profile).is_err());
        set(&profile, 0o755);
        assert!(load(&profile).is_ok(), "readable but not writable by others is enough");
        let link = root.join("link");
        std::os::unix::fs::symlink(&profile, &link).unwrap();
        assert!(load(&link).is_err() && service_lock(&link).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unix_tokens_come_from_the_os_and_differ_each_time() {
        let tokens: std::collections::HashSet<_> = (0..64).map(|_| secret().unwrap()).collect();
        assert_eq!(tokens.len(), 64);
        assert!(tokens.iter().all(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())));
    }
    #[test]
    fn stopped_profile_distinguishes_available_and_occupied_ports() {
        let root = fixture();
        let profile = root.join("profile");
        let mut config = initialize(&profile, &root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        config.port = listener.local_addr().unwrap().port();
        save(&profile, &config).unwrap();
        let error = status(&profile).unwrap_err();
        assert!(error.to_string().contains("already in use"));
        drop(listener);
        assert!(!status(&profile).unwrap().running);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn settings_token_and_service_ownership_preserve_existing_files() {
        let root = fixture();
        let profile = root.join("profile");
        let config = initialize(&profile, &root).unwrap();
        assert_eq!(config.address().ip(), Ipv4Addr::LOCALHOST);
        let first = token(&profile).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(initialize(&profile, &root).unwrap(), config);
        assert_eq!(token(&profile).unwrap(), first);
        let guard = service_lock(&profile).unwrap();
        let mut edited = config.clone();
        edited.port = 47632;
        assert!(save(&profile, &edited).is_err());
        assert!(rotate_token(&profile).is_err());
        assert_eq!(load(&profile).unwrap(), config);
        assert_eq!(token(&profile).unwrap(), first);
        drop(guard);
        save(&profile, &edited).unwrap();
        assert_eq!(load(&profile).unwrap(), edited);
        rotate_token(&profile).unwrap();
        assert_ne!(token(&profile).unwrap(), first);
        let saved = fs::read(profile.join("settings")).unwrap();
        edited.port = 80;
        assert!(save(&profile, &edited).is_err());
        assert_eq!(fs::read(profile.join("settings")).unwrap(), saved);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_folder_and_invalid_token_can_be_repaired_without_overwriting_other_settings() {
        let root = fixture();
        let profile = root.join("profile");
        let folder = root.join("fixtures");
        fs::create_dir(&folder).unwrap();
        initialize(&profile, &folder).unwrap();
        fs::remove_dir(&folder).unwrap();
        let mut config = load(&profile).unwrap();
        assert!(config.validate().is_err());
        config.root = root.clone();
        save(&profile, &config).unwrap();
        fs::write(profile.join("access-token"), "damaged").unwrap();
        assert!(token(&profile).is_err());
        assert!(initialize(&profile, &root).is_ok());
        rotate_token(&profile).unwrap();
        assert!(token(&profile).is_ok());
        fs::write(profile.join("settings"), "unsupported future data").unwrap();
        assert!(initialize(&profile, &root).is_err());
        assert_eq!(
            fs::read_to_string(profile.join("settings")).unwrap(),
            "unsupported future data"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
