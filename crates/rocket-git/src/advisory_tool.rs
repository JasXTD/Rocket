//! The pinned OSV helper. No repository configuration or credentials are inherited.
use crate::{CancellationToken, CommandLimits, Error, Result};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
    process::Command,
    time::Duration,
};

pub const OSV_VERSION: &str = "2.7.0";
const WINDOWS_SHA256: &str = "9964ed64ec9a387391eb4cc006ad5c531154821aef29b793e02c448de4686438";

pub struct OsvOutput {
    pub code: Option<i32>,
    pub json: Vec<u8>,
    /// Only the fact that diagnostics were emitted is exposed; output may contain private paths.
    pub diagnostics: bool,
}

/// Checks the pinned helper without executing it. Launches recheck the same bytes.
pub fn verify_osv_scanner(executable: &Path, cancel: &CancellationToken) -> Result<()> {
    verified_program(executable, cancel).map(drop)
}

fn verified_program(executable: &Path, cancel: &CancellationToken) -> Result<File> {
    cancel.check()?;
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err(Error::InvalidInput(
            "This OSV integration currently requires Windows x64".into(),
        ));
    }
    if !executable.is_absolute() {
        return Err(Error::InvalidInput(
            "Select an absolute OSV-Scanner path".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut program = options.open(executable).map_err(|e| Error::Io {
        action: "open OSV-Scanner",
        source: e,
    })?;
    if digest(&mut program, 80 * 1024 * 1024, cancel)? != WINDOWS_SHA256 {
        return Err(Error::InvalidInput(
            "OSV-Scanner does not match the reviewed 2.7.0 Windows x64 binary".into(),
        ));
    }
    Ok(program)
}

/// Runs only the reviewed Windows x64 OSV binary, with bounded output and owned children.
/// `directory` contains copied lockfiles and a Rocket-owned empty config. Updates must
/// receive a synthetic inventory, never a project's inventory (enforced by the caller).
pub fn run_offline_osv(
    executable: &Path,
    directory: &Path,
    database: &Path,
    lockfiles: &[std::path::PathBuf],
    update_database: bool,
    cancel: &CancellationToken,
) -> Result<OsvOutput> {
    cancel.check()?;
    if !executable.is_absolute()
        || !directory.is_absolute()
        || !database.is_absolute()
        || lockfiles.is_empty()
        || lockfiles.len() > 256
        || lockfiles
            .iter()
            .any(|p| !p.is_absolute() || !p.starts_with(directory))
    {
        return Err(Error::InvalidInput("Invalid offline scanner paths".into()));
    }
    // Keep a non-write/non-delete shared handle open until the child has finished.
    let program = verified_program(executable, cancel)?;
    let command = command(executable, directory, database, lockfiles, update_database);
    let result = crate::runner::run_process(
        command,
        Vec::new(),
        &CommandLimits {
            timeout: Duration::from_secs(if update_database { 180 } else { 90 }),
            stdout_bytes: 16 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
        },
        cancel,
    )?;
    drop(program);
    Ok(OsvOutput {
        code: result.status.code(),
        json: result.stdout,
        diagnostics: !result.stderr.is_empty(),
    })
}

fn command(
    executable: &Path,
    directory: &Path,
    database: &Path,
    files: &[std::path::PathBuf],
    update: bool,
) -> Command {
    let directory = osv_path(directory);
    let database = osv_path(database);
    let mut command = Command::new(executable);
    command.current_dir(&directory).env_clear();
    // Windows' loader and temporary-file APIs need these, but no user PATH, auth,
    // proxy, package-manager config, HOME or Git configuration is passed through.
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("TEMP", &directory)
        .env("TMP", &directory)
        .env("HOME", &directory)
        .env("USERPROFILE", &directory)
        .env("APPDATA", &directory)
        .env("LOCALAPPDATA", &directory)
        .env("OSV_SCANNER_LOCAL_DB_CACHE_DIRECTORY", &database)
        .args([
            "scan",
            "source",
            "--offline",
            "--no-call-analysis=all",
            "--no-resolve",
            "--format=json",
            "--all-packages",
            "--all-vulns",
            "--verbosity=error",
        ])
        .arg("--config")
        .arg(directory.join("osv-scanner.toml"));
    if update {
        command.arg("--download-offline-databases");
    }
    for file in files {
        let mut value = std::ffi::OsString::from(if update { "osv-scanner:" } else { ":" });
        value.push(osv_path(file));
        command.arg("--lockfile").arg(value);
    }
    command
}

fn osv_path(path: &Path) -> std::path::PathBuf {
    // OSV's Go cache code mixes path.Join (slashes) with Windows paths. Verbatim
    // Windows prefixes reject those mixed separators, so use the equivalent DOS/UNC
    // spelling for the reviewed helper. Rust retains canonical paths for validation.
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(drive) => {
                    return std::path::PathBuf::from(format!("{}:\\", drive as char))
                        .join(components.as_path());
                }
                Prefix::VerbatimUNC(server, share) => {
                    let mut root = std::ffi::OsString::from(r"\\");
                    root.push(server);
                    root.push(r"\");
                    root.push(share);
                    return std::path::PathBuf::from(root).join(components.as_path());
                }
                _ => {}
            }
        }
    }
    path.to_path_buf()
}

pub fn osv_file_digest(path: &Path, max: u64, cancel: &CancellationToken) -> Result<String> {
    let mut file = File::open(path).map_err(|e| Error::Io {
        action: "read advisory data",
        source: e,
    })?;
    digest(&mut file, max, cancel)
}

#[cfg(windows)]
pub(crate) fn digest(file: &mut dyn Read, max: u64, cancel: &CancellationToken) -> Result<String> {
    use std::{ffi::c_void, ptr};
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptOpenAlgorithmProvider(
            h: *mut *mut c_void,
            id: *const u16,
            implementation: *const u16,
            flags: u32,
        ) -> i32;
        fn BCryptCloseAlgorithmProvider(h: *mut c_void, flags: u32) -> i32;
        fn BCryptCreateHash(
            a: *mut c_void,
            h: *mut *mut c_void,
            object: *mut u8,
            object_len: u32,
            secret: *mut u8,
            secret_len: u32,
            flags: u32,
        ) -> i32;
        fn BCryptHashData(h: *mut c_void, input: *const u8, length: u32, flags: u32) -> i32;
        fn BCryptFinishHash(h: *mut c_void, output: *mut u8, length: u32, flags: u32) -> i32;
        fn BCryptDestroyHash(h: *mut c_void) -> i32;
    }
    struct Handles {
        algorithm: *mut c_void,
        hash: *mut c_void,
    }
    impl Drop for Handles {
        fn drop(&mut self) {
            // SAFETY: Each successful BCrypt allocation has exactly one matching release.
            unsafe {
                if !self.hash.is_null() {
                    BCryptDestroyHash(self.hash);
                }
                if !self.algorithm.is_null() {
                    BCryptCloseAlgorithmProvider(self.algorithm, 0);
                }
            }
        }
    }
    let failed =
        || Error::InvalidInput("Could not verify the offline scanner or advisory data".into());
    let mut h = Handles {
        algorithm: ptr::null_mut(),
        hash: ptr::null_mut(),
    };
    let name: Vec<u16> = "SHA256\0".encode_utf16().collect();
    // SAFETY: Valid output handles, NUL-terminated algorithm name. Windows 7+ allocates
    // the hash object when its buffer is null; DestroyHash releases that allocation.
    unsafe {
        if BCryptOpenAlgorithmProvider(&mut h.algorithm, name.as_ptr(), ptr::null(), 0) < 0
            || BCryptCreateHash(
                h.algorithm,
                &mut h.hash,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                0,
            ) < 0
        {
            return Err(failed());
        }
    }
    let mut count = 0u64;
    let mut bytes = [0u8; 64 * 1024];
    loop {
        cancel.check()?;
        let n = file.read(&mut bytes).map_err(|e| Error::Io {
            action: "verify offline scanner data",
            source: e,
        })?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > max {
            return Err(failed());
        }
        // SAFETY: The hash is live; input is readable for n bytes and n fits u32.
        if unsafe { BCryptHashData(h.hash, bytes.as_ptr(), n as u32, 0) } < 0 {
            return Err(failed());
        }
    }
    let mut output = [0u8; 32];
    // SAFETY: Live hash and writable SHA-256-sized output.
    if unsafe { BCryptFinishHash(h.hash, output.as_mut_ptr(), 32, 0) } < 0 {
        return Err(failed());
    }
    Ok(output.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(not(windows))]
pub(crate) fn digest(_: &mut dyn Read, _: u64, _: &CancellationToken) -> Result<String> {
    Err(Error::InvalidInput(
        "OSV binary verification requires Windows".into(),
    ))
}

/// Lowercase hexadecimal SHA-256 of in-memory bytes, for review pins such as
/// the CLI's `--expect-patch`. Windows only, like the file digest above.
pub fn sha256_bytes(bytes: &[u8]) -> Result<String> {
    digest(
        &mut std::io::Cursor::new(bytes),
        bytes.len() as u64,
        &CancellationToken::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_verbatim_paths_use_helper_compatible_spelling() {
        assert_eq!(
            osv_path(Path::new(r"\\?\C:\fixture\cache")),
            Path::new(r"C:\fixture\cache")
        );
        assert_eq!(
            osv_path(Path::new(r"\\?\UNC\server\share\cache")),
            Path::new(r"\\server\share\cache")
        );
    }
    #[test]
    fn scan_arguments_cannot_enable_network_resolution_or_project_configuration() {
        let command = command(
            Path::new("C:/verified/osv.exe"),
            Path::new("C:/snapshot"),
            Path::new("C:/cache"),
            &["C:/snapshot/0/Cargo.lock".into()],
            false,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for flag in [
            "--offline",
            "--no-call-analysis=all",
            "--no-resolve",
            "--all-packages",
        ] {
            assert!(args.iter().any(|a| a == flag));
        }
        assert!(
            !args
                .iter()
                .any(|a| a.contains("download") || a == "--serve" || a == "fix")
        );
        assert_eq!(args.last().unwrap(), ":C:/snapshot/0/Cargo.lock");
        assert!(command.get_envs().all(|(k, _)| {
            !["GITHUB_TOKEN", "PATH", "HTTP_PROXY", "GIT_CONFIG"]
                .iter()
                .any(|s| k == *s)
        }));
    }
    #[test]
    fn mismatched_binary_is_never_executed() {
        let exe = std::env::current_exe().unwrap();
        assert!(verify_osv_scanner(&exe, &CancellationToken::new()).is_err());
        assert!(verify_osv_scanner(Path::new("relative.exe"), &CancellationToken::new()).is_err());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            verify_osv_scanner(&exe, &cancel),
            Err(Error::Cancelled)
        ));
        let dir = exe.parent().unwrap();
        assert!(
            run_offline_osv(
                &exe,
                dir,
                dir,
                &[dir.join("Cargo.lock")],
                false,
                &CancellationToken::new()
            )
            .is_err()
        );
    }
    #[test]
    fn sha256_known_vector_and_limits() {
        let path = std::env::temp_dir().join(format!("rocket-osv-hash-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, b"abc").unwrap();
        drop(file);
        assert_eq!(
            osv_file_digest(&path, 3, &CancellationToken::new()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(osv_file_digest(&path, 2, &CancellationToken::new()).is_err());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            osv_file_digest(&path, 3, &cancel),
            Err(Error::Cancelled)
        ));
        std::fs::remove_file(path).unwrap();
    }
}
