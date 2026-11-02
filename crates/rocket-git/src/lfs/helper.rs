use super::*;

pub(super) fn locate(repo: &Repository, c: &CancellationToken) -> Result<(PathBuf, String)> {
    let parent = repo
        .runner
        .executable
        .parent()
        .ok_or(Error::Lfs(LfsFailure::HelperMissing))?;
    let name = if cfg!(windows) {
        "git-lfs.exe"
    } else {
        "git-lfs"
    };
    let candidates = [
        parent.join(name),
        parent.join("../ucrt64/bin").join(name),
        parent.join("../mingw64/bin").join(name),
        parent.join("../libexec/git-core").join(name),
    ];
    let exe = candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or(Error::Lfs(LfsFailure::HelperMissing))?
        .canonicalize()
        .map_err(|e| Error::io("resolve LFS helper", e))?;
    let raw = repo
        .runner
        // Probe an unapproved helper outside the repository. Even its startup
        // must not read repository-controlled configuration before this gate.
        .lfs(&exe, exe.parent().unwrap(), &args(&["version"]), &config(None), c)?
        .success("lfs-version")?;
    let number = supported_version(&raw)?;
    Ok((exe, format!("Git LFS {number}")))
}

fn supported_version(raw: &[u8]) -> Result<&str> {
    let version = std::str::from_utf8(parse::line(raw))
        .map_err(|_| Error::Lfs(LfsFailure::HelperVersion))?;
    let number = version
        .strip_prefix("git-lfs/")
        .and_then(|s| s.split_whitespace().next())
        .ok_or(Error::Lfs(LfsFailure::HelperVersion))?;
    let parts = number
        .split('.')
        .map(|s| s.parse::<u32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::Lfs(LfsFailure::HelperVersion))?;
    if parts.len() != 3 || parts.as_slice() < [3, 8, 1].as_slice() {
        return fail(LfsFailure::HelperVersion);
    }
    Ok(number)
}
pub(super) fn validate_config(repo: &Repository, config: &[u8]) -> Result<()> {
    if fs::symlink_metadata(repo.cwd().join(".lfsconfig")).is_ok() {
        return fail(LfsFailure::Configuration);
    }
    for record in config.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let key = record.split(|b| *b == b'\n').next().unwrap();
        let key = String::from_utf8_lossy(key).to_ascii_lowercase();
        if (key.starts_with("lfs.")
            && auth_record_key(record).is_none()
            && !matches!(
                key.as_str(),
                "lfs.repositoryformatversion"
                    | "lfs.fetchinclude"
                    | "lfs.fetchexclude"
                    | "lfs.fetchrecentalways"
                    | "lfs.fetchrecentrefsdays"
                    | "lfs.fetchrecentcommitsdays"
                    | "lfs.fetchrecentremoterefs"
                    | "lfs.concurrenttransfers"
                    | "lfs.activitytimeout"
                    | "lfs.dialtimeout"
                    | "lfs.tlstimeout"
                    | "lfs.locksverify"
                    | "lfs.transfer.maxretries"
            ))
            || (key.starts_with("remote.")
                && (key.ends_with(".lfsurl") || key.ends_with(".lfspushurl")))
            // LFS applies aliases even to explicit lfs.url overrides. Refuse
            // them rather than resolving a reviewed URL a second time.
            || (key.starts_with("url.")
                && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof")))
        {
            return fail(LfsFailure::Configuration);
        }
    }
    Ok(())
}

// Git LFS caches the authentication mode, never the credential, under this key.
// Treat only recognized modes at a credential-free HTTPS endpoint as metadata.
pub(super) fn auth_record_key(record: &[u8]) -> Option<&str> {
    let (key, value) = std::str::from_utf8(record).ok()?.split_once('\n')?;
    if !["none", "basic", "private", "negotiate"]
        .iter()
        .any(|mode| value.eq_ignore_ascii_case(mode))
    {
        return None;
    }
    let endpoint = key.strip_prefix("lfs.")?.strip_suffix(".access")?;
    (plain_https(endpoint)
        || endpoint
            .strip_suffix("/info/lfs")
            .is_some_and(is_lfs_lock_fixture_url))
    .then_some(key)
}
fn plain_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    !host.is_empty()
        && !host.contains('@')
        && !path.is_empty()
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        && !url.contains(['?', '#', '\\'])
}
/// The standard endpoint derivation used by Git LFS. Unusual URLs remain
/// conservative: their config changes will require review after the fetch.
pub(super) fn access_key(source: &str) -> Option<String> {
    if is_lfs_lock_fixture_url(source) {
        return Some(format!("lfs.{source}/info/lfs.access"));
    }
    let base = if source.starts_with("https://") {
        source.to_owned()
    } else {
        let (authority, path) = if let Some(rest) = source.strip_prefix("ssh://") {
            rest.split_once('/')?
        } else {
            source.split_once(':')?
        };
        let host = authority.rsplit('@').next()?;
        let host = if let Some((host, port)) = host.split_once(':') {
            if port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            host
        } else {
            host
        };
        if host.contains(['/', '[', ']']) {
            return None;
        }
        format!("https://{host}/{path}")
    };
    let base = base.trim_end_matches('/');
    if !base.is_ascii() || !plain_https(base) {
        return None;
    }
    let suffix = if base.ends_with(".git") {
        "/info/lfs"
    } else {
        ".git/info/lfs"
    };
    Some(format!("lfs.{base}{suffix}.access"))
}
pub(super) fn config(remote: Option<(&str, &str)>) -> Vec<(String, OsString)> {
    // Custom storage is refused before invocation. Do not inject lfs.storage:
    // the built-in file adapter also inspects the remote repository with this
    // environment, and a client-absolute override would redirect its cache.
    let mut out = Vec::new();
    for (k, v) in [
        ("core.fsmonitor", "false"),
        ("gc.auto", "0"),
        ("maintenance.auto", "false"),
        ("lfs.fetchrecentalways", "false"),
        ("lfs.fetchrecentrefsdays", "0"),
        ("lfs.fetchrecentcommitsdays", "0"),
        ("lfs.fetchinclude", ""),
        ("lfs.fetchexclude", ""),
        ("lfs.skipdownloaderrors", "false"),
        ("lfs.concurrenttransfers", "3"),
        ("lfs.transfer.maxretries", "1"),
        ("lfs.basictransfersonly", "true"),
        ("lfs.locksverify", "false"),
    ] {
        out.push((k.into(), v.into()))
    }
    if let Some((name, url)) = remote {
        out.push((format!("remote.{name}.url"), url.into()));
        // The file adapter receives an explicit reviewed URL, including in MCP
        // fixtures; no fallback endpoint or default remote can escape that root.
        if Path::new(url).is_absolute() {
            let normalized = url.replace('\\', "/");
            let normalized = normalized
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>();
            let url = if cfg!(windows) {
                format!("file:///{normalized}")
            } else {
                format!("file://{normalized}")
            };
            out.push(("lfs.url".into(), url.clone().into()));
            out.push((format!("remote.{name}.lfsurl"), url.into()));
        }
    }
    out
}
pub(super) fn source(
    repo: &Repository,
    name: &str,
    scope: Option<&LfsSourceScope>,
    c: &CancellationToken,
) -> Result<(String, Option<PathBuf>)> {
    endpoint(repo, name, scope, false, false, c)
}
pub(super) fn destination(
    repo: &Repository,
    name: &str,
    scope: Option<&LfsSourceScope>,
    c: &CancellationToken,
) -> Result<(String, Option<PathBuf>)> {
    endpoint(repo, name, scope, true, false, c)
}
pub(super) fn lock_source(
    repo: &Repository,
    name: &str,
    scope: Option<&LfsSourceScope>,
    c: &CancellationToken,
) -> Result<String> {
    let (url, local) = endpoint(repo, name, scope, true, true, c)?;
    if local.is_some() {
        return fail(LfsFailure::LockUnsupported);
    }
    Ok(url)
}
fn endpoint(
    repo: &Repository,
    name: &str,
    scope: Option<&LfsSourceScope>,
    push: bool,
    locks: bool,
    c: &CancellationToken,
) -> Result<(String, Option<PathBuf>)> {
    let fixture_root = scope.map(|s| s.root.as_path());
    if name.is_empty()
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return Err(Error::InvalidInput("Select one named remote".into()));
    }
    let remote = repo
        .remotes(c)?
        .into_iter()
        .find(|r| r.name == name.as_bytes())
        .ok_or_else(|| Error::InvalidInput("The selected remote no longer exists".into()))?;
    if remote.urls.len() != 1 {
        return fail(LfsFailure::Configuration);
    }
    let mut command = args(&["remote", "get-url", "--all"]);
    if push {
        command.push("--push".into());
    }
    command.push(name.into());
    let urls = repo.lfs_read(&command, Vec::new(), c)?;
    let url = std::str::from_utf8(parse::line(&urls))
        .map_err(|_| Error::Lfs(LfsFailure::Configuration))?;
    if url.is_empty() || url.chars().any(char::is_control) || url.contains(['\0', '\n', '\r']) {
        return fail(LfsFailure::Configuration);
    }
    if Path::new(url).is_absolute() {
        let _fixture_guards = if let Some(root) = fixture_root {
            let root = PathBuf::from(git_path(root));
            let candidate = Path::new(url);
            if !candidate.starts_with(&root) {
                return Err(Error::InvalidInput(
                    "LFS test sources must stay inside the fixture root".into(),
                ));
            }
            let guards = local_files::pin_directories(&root, candidate)?;
            let admin = candidate.join(".git");
            let admin = match fs::symlink_metadata(&admin) {
                Ok(m) if m.is_dir() && !local_files::redirected(&m) => admin,
                Ok(_) => return fail(LfsFailure::Layout),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => candidate.to_path_buf(),
                Err(e) => return Err(Error::io("inspect LFS source", e)),
            };
            if fs::symlink_metadata(admin.join("commondir")).is_ok() {
                return fail(LfsFailure::Layout);
            }
            Some(guards)
        } else {
            None
        };
        let canonical = Path::new(url)
            .canonicalize()
            .map_err(|e| Error::io("resolve LFS source", e))?;
        if let Some(root) = fixture_root {
            let root = root
                .canonicalize()
                .map_err(|e| Error::io("resolve LFS fixture root", e))?;
            if !canonical.starts_with(root) {
                return Err(Error::InvalidInput(
                    "LFS test sources must stay inside the fixture root".into(),
                ));
            }
        }
        local_files::directory(&canonical)?;
        let source = GitClient::from_runner(repo.runner.clone()).open(&canonical, c)?;
        if let Some(root) = fixture_root {
            let root = root
                .canonicalize()
                .map_err(|e| Error::io("resolve LFS fixture root", e))?;
            if !source.info().common_dir.starts_with(&root)
                || !source.info().git_dir.starts_with(&root)
            {
                return fail(LfsFailure::Layout);
            }
        }
        let cfg = source.lfs_read(
            &args(&["config", "--null", "--list", "--includes"]),
            Vec::new(),
            c,
        )?;
        validate_config(&source, &cfg)?;
        disk::check_existing_store(&source)?;
        let normalized = git_path(&canonical)
            .to_str()
            .ok_or(Error::Lfs(LfsFailure::Layout))?
            .to_owned();
        return Ok((normalized, Some(canonical)));
    }
    if scope.is_some_and(|s| !s.network_sources.iter().any(|allowed| allowed == url)) {
        return Err(Error::InvalidInput(
            "LFS test transfers require a local repository inside the fixture root or an explicitly approved network source".into(),
        ));
    }
    let https = url.strip_prefix("https://").is_some_and(|s| {
        let host = s.split('/').next().unwrap_or("");
        !host.is_empty() && !host.contains('@')
    });
    let ssh = url
        .strip_prefix("ssh://")
        .is_some_and(|s| !s.split('/').next().unwrap_or("").is_empty())
        || (!url.contains("://")
            && url.contains('@')
            && url
                .split_once(':')
                .is_some_and(|(host, p)| !host.contains('/') && !p.is_empty()));
    if !https && !ssh {
        if locks
            && scope.is_some_and(|s| {
                repo.info().git_dir.starts_with(&s.root)
                    && s.network_sources.contains(&url.to_owned())
            })
            && is_lfs_lock_fixture_url(url)
        {
            return Ok((url.to_owned(), None));
        }
        return Err(Error::InvalidInput("LFS transfers support absolute local paths, HTTPS without embedded credentials, or SSH remotes".into()));
    }
    Ok((url.to_owned(), None))
}
pub(super) fn push_config(name: &str, destination: &str) -> Vec<(String, OsString)> {
    let mut cfg = config(Some((name, destination)));
    // Pin both Git URLs. A configured pushurl must never redirect a reviewed upload.
    cfg.push((format!("remote.{name}.pushurl"), destination.into()));
    cfg.push(("lfs.allowincompletepush".into(), "false".into()));
    if Path::new(destination).is_absolute() {
        let url = cfg
            .iter()
            .find(|(key, _)| key == "lfs.url")
            .unwrap()
            .1
            .clone();
        cfg.push(("lfs.pushurl".into(), url.clone()));
        cfg.push((format!("remote.{name}.lfspushurl"), url));
    }
    cfg
}
pub(super) fn hash(
    repo: &Repository,
    helper: &Path,
    path: &Path,
    c: &CancellationToken,
) -> Result<LfsPointer> {
    let mut a = args(&["pointer"]);
    let mut arg = OsString::from("--file=");
    arg.push(git_path(path));
    a.push(arg);
    let output = repo
        .runner
        .lfs(helper, repo.cwd(), &a, &config(None), c)?
        .success("lfs-hash")?;
    if output.is_empty() {
        // Git LFS encodes the empty object's pointer as zero bytes.
        Ok(LfsPointer {
            oid: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            size: 0,
        })
    } else {
        LfsPointer::parse(&output)
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn security_floor_rejects_old_malformed_and_prerelease_helpers() {
        for version in ["3.7.1", "3.8.0", "3.8.1-rc1", "3.8", "3.8.1.0", "garbage"] {
            let output = format!("git-lfs/{version} (fixture)\n");
            assert!(matches!(supported_version(output.as_bytes()), Err(Error::Lfs(LfsFailure::HelperVersion))), "{version}");
        }
        for version in ["3.8.1", "3.8.2", "3.9.0", "4.0.0"] {
            let output = format!("git-lfs/{version} (fixture)\n");
            assert_eq!(supported_version(output.as_bytes()).unwrap(), version);
        }
    }
}
