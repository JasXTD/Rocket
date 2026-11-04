//! Public Rocket releases only. No credentials, repository config or automatic install.
use crate::{
    hosting::{
        find_curl, header,
        json::{self, Json},
        split_response,
    },
    runner::Runner,
    *,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub const UPDATE_REPOSITORY: &str = "JasXTD/Rocket";
pub const UPDATE_MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
fn curl_has_stream_limit(output: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(output) else {
        return false;
    };
    let mut words = text.lines().next().unwrap_or("").split_whitespace();
    words.next() == Some("curl")
        && words
            .next()
            .and_then(UpdateVersion::parse)
            .is_some_and(|v| v >= UpdateVersion(8, 4, 0))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UpdateVersion(pub u32, pub u32, pub u32);
impl UpdateVersion {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let parts: Vec<_> = text.split('.').collect();
        if parts.len() != 3
            || parts.iter().any(|p| {
                p.is_empty()
                    || p.len() > 9
                    || !p.bytes().all(|b| b.is_ascii_digit())
                    || (p.len() > 1 && p.starts_with('0'))
            })
        {
            return None;
        }
        Some(Self(
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    }
    pub fn text(self) -> String {
        format!("{}.{}.{}", self.0, self.1, self.2)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateEndpoint {
    Public,
    Loopback(u16),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRelease {
    pub version: UpdateVersion,
    pub notes: String,
    pub page: String,
    pub size: u64,
    digest: String,
    asset_id: u64,
}
impl AppRelease {
    pub fn checksum(&self) -> &str {
        &self.digest
    }
}

/// Wait on the actual parent process handle, not repeated PID lookups.
pub fn wait_for_update_parent(parent: u32, ready: &Path, cancel: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        if parent == std::process::id() {
            return Err(invalid("Update helper cannot wait for itself."));
        }
        // SYNCHRONIZE is sufficient; no termination or write access is requested.
        let handle = unsafe { OpenProcess(0x00100000, 0, parent) };
        if handle.is_null() {
            return Err(invalid(
                "The original Rocket process is no longer available. Retry from Rocket.",
            ));
        }
        struct Guard(*mut c_void);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }
        let _guard = Guard(handle);
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
            match unsafe { WaitForSingleObject(handle, 50) } {
                0 => return Ok(()),
                258 => {}
                _ => return Err(invalid("Could not wait for Rocket to exit.")),
            }
            if std::time::Instant::now() > until {
                return Err(invalid(
                    "Rocket did not exit; the update left existing files unchanged.",
                ));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (parent, ready, cancel);
        Err(invalid(
            "App update installation currently requires Windows.",
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    Current,
    Unavailable,
    Available(AppRelease),
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
fn parse_release(body: &[u8], current: UpdateVersion) -> Result<UpdateCheck> {
    let root =
        json::parse(body).ok_or_else(|| invalid("GitHub returned invalid release metadata."))?;
    if root.bool("draft") != Some(false) || root.bool("prerelease") != Some(false) {
        return Err(invalid("GitHub did not return a published stable release."));
    }
    let tag = root
        .str("tag_name")
        .ok_or_else(|| invalid("Release version is missing."))?;
    let version = UpdateVersion::parse(tag)
        .ok_or_else(|| invalid("Release version must be a stable major.minor.patch number."))?;
    if version <= current {
        return Ok(UpdateCheck::Current);
    }
    let page = format!("https://github.com/{UPDATE_REPOSITORY}/releases/tag/{tag}");
    if root.str("html_url") != Some(page.as_str()) {
        return Err(invalid("Release page does not belong to Rocket."));
    }
    let name = format!("Rocket-{}-windows-x64.zip", version.text());
    let assets = root
        .get("assets")
        .and_then(Json::array)
        .ok_or_else(|| invalid("Release has no downloadable bundle."))?;
    let mut found = assets
        .iter()
        .filter(|a| a.str("name") == Some(name.as_str()));
    let asset = found
        .next()
        .ok_or_else(|| invalid("This release has no Windows x64 update bundle yet."))?;
    if found.next().is_some() || asset.str("state") != Some("uploaded") {
        return Err(invalid("Release bundle is ambiguous or incomplete."));
    }
    let digest = asset
        .str("digest")
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            invalid("GitHub did not supply a SHA-256 digest. This update cannot be installed.")
        })?
        .to_ascii_lowercase();
    let asset_id = asset
        .u64("id")
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("Release asset ID is invalid."))?;
    let size = asset
        .u64("size")
        .filter(|n| *n > 0 && *n <= UPDATE_MAX_ARCHIVE)
        .ok_or_else(|| invalid("Release archive exceeds the 512 MiB update limit."))?;
    let notes = root.str("body").unwrap_or("");
    if notes.len() > 128 * 1024 {
        return Err(invalid("Release notes exceed the supported size."));
    }
    Ok(UpdateCheck::Available(AppRelease {
        version,
        notes: notes
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect(),
        page,
        size,
        digest,
        asset_id,
    }))
}

#[derive(Clone)]
pub struct UpdateClient {
    runner: Runner,
    curl: PathBuf,
    endpoint: UpdateEndpoint,
}
impl UpdateClient {
    pub fn new(endpoint: UpdateEndpoint, log: Option<DiagnosticLog>) -> Result<Self> {
        if matches!(endpoint, UpdateEndpoint::Loopback(0)) {
            return Err(invalid("Invalid update test endpoint."));
        }
        let curl = find_curl(Path::new("")).ok_or_else(|| {
            invalid("Windows curl is unavailable; Rocket could not check for updates.")
        })?;
        Ok(Self {
            runner: Runner {
                executable: curl.clone(),
                limits: CommandLimits {
                    timeout: Duration::from_secs(35),
                    stdout_bytes: 2 * 1024 * 1024,
                    stderr_bytes: 8192,
                },
                log,
                transport_scope: None,
                secret_protection: Arc::new(AtomicBool::new(true)),
                guard_ai_api_keys: Arc::new(AtomicBool::new(true)),
                push_check: Default::default(),
                git_http_allowed: true,
                managed_git: None,
            },
            curl,
            endpoint,
        })
    }
    fn base(&self) -> String {
        match self.endpoint {
            UpdateEndpoint::Public => "https://api.github.com".into(),
            UpdateEndpoint::Loopback(port) => format!("http://127.0.0.1:{port}"),
        }
    }
    fn allowed(&self, url: &str) -> bool {
        if url.len() > 16384
            || url
                .bytes()
                .any(|b| !(0x21..=0x7e).contains(&b) || matches!(b, b'"' | b'\\'))
        {
            return false;
        }
        match self.endpoint {
            UpdateEndpoint::Loopback(_) => url.starts_with(&(self.base() + "/")),
            UpdateEndpoint::Public => {
                url.starts_with(&format!(
                    "https://api.github.com/repos/{UPDATE_REPOSITORY}/releases/"
                )) || url.starts_with(&format!(
                    "https://github.com/{UPDATE_REPOSITORY}/releases/download/"
                )) || [
                    "https://release-assets.githubusercontent.com/",
                    "https://objects.githubusercontent.com/",
                ]
                .iter()
                .any(|prefix| url.starts_with(prefix))
            }
        }
    }
    fn http(
        &self,
        cwd: &Path,
        url: &str,
        destination: Option<(&Path, u64)>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        if !self.allowed(url) {
            return Err(invalid(
                "Update redirect left the approved GitHub release hosts.",
            ));
        }
        cancel.check()?;
        let mut runner = self.runner.clone();
        runner.limits.timeout = Duration::from_secs(if destination.is_some() { 600 } else { 35 });
        let protocol = if matches!(self.endpoint, UpdateEndpoint::Public) {
            "=https"
        } else {
            "=http"
        };
        let accept = if destination.is_some() {
            "application/octet-stream"
        } else {
            "application/vnd.github+json"
        };
        let config = format!(
            "url = \"{url}\"\nheader = \"Accept: {accept}\"\nheader = \"X-GitHub-Api-Version: 2022-11-28\"\nuser-agent = \"Rocket/{}\"\n",
            env!("CARGO_PKG_VERSION")
        );
        let size_limit = destination
            .map_or(1024 * 1024, |(_, size)| size)
            .to_string();
        let mut args: Vec<OsString> = [
            "-q",
            "--silent",
            "--show-error",
            "--suppress-connect-headers",
            "--globoff",
            "--proto",
            protocol,
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            if destination.is_some() { "590" } else { "30" },
            "--max-filesize",
            &size_limit,
            "--config",
            "-",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if let Some((path, _)) = destination {
            args.extend([
                OsString::from("--dump-header"),
                OsString::from("-"),
                OsString::from("--output"),
                path.as_os_str().to_owned(),
            ]);
        } else {
            args.push("--include".into());
        }
        if matches!(self.endpoint, UpdateEndpoint::Loopback(_)) {
            args.extend(["--noproxy".into(), "*".into()]);
        }
        let output = runner.tool(&self.curl, "curl", cwd, &args, config.into_bytes(), cancel)?;
        if !output.status.success() {
            return Err(invalid(match output.status.code() {
                Some(6) => {
                    "Could not resolve the update server. Check the connection and try again."
                }
                Some(7) => "Could not connect to the update server. Try again later.",
                Some(28) => "The update request timed out. Your installation is unchanged.",
                Some(60) => "The update connection failed TLS verification. Nothing was installed.",
                Some(63) => "Release data exceeded its download size limit. Nothing was installed.",
                _ => {
                    "The update request could not finish. Check the network connection and try again."
                }
            }));
        }
        Ok(output.stdout)
    }
    pub fn check(
        &self,
        current: &str,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<UpdateCheck> {
        self.runner.step("app-update-check", "read_release", || {
            let current = UpdateVersion::parse(current)
                .ok_or_else(|| invalid("Installed version is not a stable version number."))?;
            let output = self.http(
                directory,
                &format!("{}/repos/{UPDATE_REPOSITORY}/releases/latest", self.base()),
                None,
                cancel,
            )?;
            let response = split_response(&output)
                .ok_or_else(|| invalid("GitHub returned an unreadable update response."))?;
            match response.status {
                200 => parse_release(response.body, current),
                404 => Ok(UpdateCheck::Unavailable),
                403 | 429 => Err(invalid(
                    "GitHub refused or rate-limited the update check. Try again later.",
                )),
                _ => Err(invalid(&format!(
                    "GitHub could not provide release information (HTTP {}). Try again later.",
                    response.status
                ))),
            }
        })
    }
    pub fn download(
        &self,
        release: &AppRelease,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        // Older curl versions do not enforce --max-filesize on chunked bodies.
        // Probe the installed helper with the same cancellable process runner.
        let version = self
            .runner
            .step("app-update-download", "verify_transport", || {
                self.runner.tool(
                    &self.curl,
                    "curl-version",
                    directory,
                    &["-q".into(), "--version".into()],
                    Vec::new(),
                    cancel,
                )
            })?;
        if !version.status.success() || !curl_has_stream_limit(&version.stdout) {
            return Err(invalid(
                "Update downloads require Windows curl 8.4.0 or newer to enforce size limits. Your installation is unchanged.",
            ));
        }
        let archive = directory.join("release.zip");
        // The caller owns a fresh directory; never truncate an earlier download or a link.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&archive)
            .map_err(|e| Error::io("reserve update archive", e))?;
        let result=self.runner.step("app-update-download","download_bundle",||{
            let mut url=format!("{}/repos/{UPDATE_REPOSITORY}/releases/assets/{}",self.base(),release.asset_id);
            for _ in 0..4 {
                let output=self.http(directory,&url,Some((&archive,release.size)),cancel)?;
                let response=split_response(&output).ok_or_else(||invalid("Update download returned invalid response headers."))?;
                match response.status {
                    301|302|303|307|308=>{url=header(&response.headers,"location").ok_or_else(||invalid("Update download redirect is missing its destination."))?.to_owned();},
                    200=>{
                        let size=std::fs::metadata(&archive).map_err(|e|Error::io("read update size",e))?.len();
                        if size!=release.size || osv_file_digest(&archive,UPDATE_MAX_ARCHIVE,cancel)?!=release.digest {return Err(invalid("Update verification failed: size or SHA-256 differs from GitHub. Nothing was installed."));}
                        return Ok(archive.clone());
                    },
                    _=>return Err(invalid("GitHub could not download this release. Nothing was installed.")),
                }
            }
            Err(invalid("Too many update download redirects."))
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&archive);
        }
        result
    }
    /// Extracts data only, with the platform's installed ZIP reader. Downloaded scripts never run.
    pub fn extract(
        &self,
        archive: &Path,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.runner.step("app-update-prepare","extract_bundle",||{
            let shell=self.curl.parent().unwrap().join("WindowsPowerShell/v1.0/powershell.exe");
            let mut runner=self.runner.clone();runner.limits.timeout=Duration::from_secs(180);
            let input=format!("{}\n{}\n",archive.display(),directory.display());
            if input.lines().count()!=2{return Err(invalid("Unsupported control character in update path."));}
            let args=["-NoLogo","-NoProfile","-NonInteractive","-Command",include_str!("app_update/extract.ps1")].map(OsString::from);
            let output=runner.tool(&shell,"update-extract",archive.parent().unwrap(),&args,input.into_bytes(),cancel)?;
            if !output.status.success(){return Err(invalid("The update bundle is invalid or could not be extracted. Nothing was installed."));}
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downloads_require_chunked_body_size_enforcement() {
        assert!(curl_has_stream_limit(
            b"curl 8.13.0 (Windows)\nFeatures: SSL"
        ));
        assert!(curl_has_stream_limit(b"curl 8.4.0 (Windows)"));
        for old in [
            "curl 8.3.0 (Windows)",
            "curl 7.88.1",
            "other 9.1.0",
            "curl 8.4.0-DEV",
        ] {
            assert!(!curl_has_stream_limit(old.as_bytes()));
        }
    }
    #[test]
    fn stable_versions_compare_numerically() {
        assert!(UpdateVersion::parse("0.100.0") > UpdateVersion::parse("0.99.9"));
        for bad in [
            "1.2",
            "1.2.3-beta",
            "01.2.3",
            "1.2.3/evil",
            "1.2.3\n",
            "1.2.3+build",
        ] {
            assert!(UpdateVersion::parse(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn redirects_remain_on_exact_release_hosts() {
        let c = UpdateClient::new(UpdateEndpoint::Public, None).unwrap();
        for bad in [
            "https://api.github.com.evil/repos/JasXTD/Rocket/releases/1",
            "https://release-assets.githubusercontent.com@evil/a",
            "http://github.com/JasXTD/Rocket/releases/download/v1/a",
            "https://github.com/other/Rocket/releases/download/v1/a",
            "https://release-assets.githubusercontent.com/a\nheader",
        ] {
            assert!(!c.allowed(bad));
        }
        assert!(c.allowed("https://release-assets.githubusercontent.com/a?token=synthetic"));
    }
    #[test]
    fn releases_need_a_matching_uploaded_hashed_bundle() {
        let digest = "a".repeat(64);
        let value = format!(
            r#"{{"draft":false,"prerelease":false,"tag_name":"v1.0.0","html_url":"https://github.com/JasXTD/Rocket/releases/tag/v1.0.0","assets":[{{"name":"Rocket-1.0.0-windows-x64.zip","state":"uploaded","id":1,"size":15,"digest":"sha256:{digest}"}}]}}"#
        );
        assert!(matches!(
            parse_release(value.as_bytes(), UpdateVersion(0, 87, 3)).unwrap(),
            UpdateCheck::Available(_)
        ));
        assert_eq!(
            parse_release(value.as_bytes(), UpdateVersion(1, 0, 0)).unwrap(),
            UpdateCheck::Current
        );
        for bad in [
            value.replace("uploaded", "starter"),
            value.replace("sha256:", "sha512:"),
            value.replace("windows-x64", "windows-arm64"),
            value.replace("\"prerelease\":false", "\"prerelease\":true"),
            value.replace("github.com/JasXTD", "github.com/other"),
        ] {
            assert!(parse_release(bad.as_bytes(), UpdateVersion(0, 87, 3)).is_err());
        }
    }
}
