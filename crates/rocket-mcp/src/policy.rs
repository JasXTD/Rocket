use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// Explicitly selected disposable repository; no wildcard host or owner match.
pub const GITHUB_TEST_SOURCES: [&str; 3] = [
    "https://github.com/JasXTD/rocket-auth-test.git",
    "git@github.com:JasXTD/rocket-auth-test.git",
    "ssh://git@github.com/JasXTD/rocket-auth-test.git",
];

fn redirected(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Explicit development scope, independent of the MCP client's working directory.
#[derive(Clone)]
pub struct Policy {
    pub root: PathBuf,
}
impl Policy {
    /// A local operator/test runner may opt in to inspecting synthetic contents.
    /// MCP has no API for granting this consent. No marker means metadata only.
    pub fn private_state_allowed(&self) -> bool {
        read_regular(&self.root.join("rocket-private-state"), 64)
            .is_ok_and(|text| text == "synthetic-content-only-v1\n")
    }
    /// Only the local acceptance runner creates this exact, random lock endpoint.
    pub fn lfs_lock_source(&self) -> Result<String, String> {
        let source = read_regular(&self.root.join("rocket-lfs-lock-server"), 256)?;
        let source = source.trim_end_matches(['\r', '\n']);
        if !rocket_git::is_lfs_lock_fixture_url(source) {
            return Err("LFS lock tests require a marked loopback lock server".into());
        }
        Ok(source.to_owned())
    }
    /// GitHub requests stay off unless the acceptance runner names its loopback
    /// test API, or exactly `https://api.github.com` for a read-only live check.
    pub fn github_endpoint(&self) -> Option<rocket_git::GitHubEndpoint> {
        let text = read_regular(&self.root.join("rocket-github-api"), 256).ok()?;
        let text = text.trim_end_matches(['\r', '\n']);
        if text == "https://api.github.com" {
            return Some(rocket_git::GitHubEndpoint::Public);
        }
        rocket_git::GitHubEndpoint::loopback(text)
    }
    /// A runner may lower the 1 GB preview limit (bytes) to test where previews stop.
    pub fn preview_limit(&self) -> Option<u64> {
        let text = read_regular(&self.root.join("rocket-preview-limit"), 32).ok()?;
        let limit = text.trim().parse::<u64>().ok()?;
        (1..=rocket_git::PREVIEW_LIMIT).contains(&limit).then_some(limit)
    }
    /// A preview tab's repository: `<root>/rocket-previews/<slot>/repository.git`
    /// with no redirected folder on the way. It may not exist yet while the preview
    /// opens. Previews refuse every write, so they carry no fixture marker.
    pub fn preview_repository(&self, path: &Path) -> Result<(), String> {
        let error = "A preview must be in the test root's rocket-previews folder";
        let cache = self.root.join("rocket-previews");
        let plain = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !redirected(&m));
        let rest = path.strip_prefix(&cache).map_err(|_| error)?;
        let parts: Vec<_> = rest.components().collect();
        let [std::path::Component::Normal(slot), std::path::Component::Normal(name)] = parts.as_slice() else {
            return Err(error.into());
        };
        if *name != "repository.git" || !slot.to_string_lossy().starts_with('p') {
            return Err(error.into());
        }
        if !plain(&cache) || !plain(&cache.join(slot)) || (path.exists() && !plain(path)) {
            return Err(error.into());
        }
        Ok(())
    }
    /// App updater fixtures never contact GitHub or replace the running MCP bundle.
    pub fn update_fixture(&self)->Option<u16>{
        let text=read_regular(&self.root.join("rocket-update-api"),128).ok()?;
        let port=text.trim().strip_prefix("http://127.0.0.1:")?.parse::<u16>().ok()?;
        (port!=0).then_some(port)
    }
    /// The hosting adapters can perform writes. They are strictly loopback-only
    /// under MCP, independently of the older GitHub read-only live opt-in.
    /// Only explicit, disposable SSH roots may be managed by native automation.
    pub fn ssh_fixture(&self) -> Option<PathBuf> {
        let marker = read_regular(&self.root.join("rocket-ssh-fixture"), 64).ok()?;
        if marker.trim() != "disposable-only-v1" {
            return None;
        }
        let path = self.root.join("ssh-home");
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_dir() || redirected(&meta) {
            return None;
        }
        Some(path.join("rocket-keys"))
    }
    /// Automatic fetch contacts remotes in the background, so under MCP it stays off
    /// unless the runner marks the root; the marker holds the test interval (1..=60 s).
    pub fn auto_fetch_fixture(&self) -> Option<std::time::Duration> {
        let text = read_regular(&self.root.join("rocket-auto-fetch"), 16).ok()?;
        let seconds = text
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|s| (1..=60).contains(s))?;
        Some(std::time::Duration::from_secs(seconds))
    }
    pub fn hosting_fixture(&self) -> Option<u16> {
        let text = read_regular(&self.root.join("rocket-hosting-api"), 128).ok()?;
        let port = text
            .trim()
            .strip_prefix("http://127.0.0.1:")?
            .parse::<u16>()
            .ok()?;
        (port != 0).then_some(port)
    }
    /// Local operator consent for exactly one private repository and SSH branch.
    /// MCP cannot create or edit this marker. No public publishing or API deletion.
    pub fn github_publish_check(&self) -> Option<(String, String)> {
        if self.hosting_fixture().is_some() || self.hosting_account_check() { return None; }
        let text = read_regular(&self.root.join("rocket-github-publish-check"), 256).ok()?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len()!=5 || lines[0]!="ROCKET-GITHUB-PUBLISH-CHECK-1" || lines[3]!="private" || lines[4]!="ssh" { return None; }
        let config = rocket_git::ForgeConfig::new(rocket_git::ForgeProvider::GitHub,"https://github.com").ok()?.with_github_publish_check(lines[1]).ok()?;
        let repository = config.publish_check_repository()?;
        let suffix = repository.split_once('/')?.1.strip_prefix("rocket-publish-test-")?;
        let branch = format!("rocket-ui-test/publish-{}", &suffix[..12]);
        if lines[2]!=branch || text!=format!("ROCKET-GITHUB-PUBLISH-CHECK-1\n{repository}\n{branch}\nprivate\nssh\n") { return None; }
        Some((repository.into(),branch))
    }
    /// Separate operator consent for one real browser account check. It does
    /// not authorize repository API reads/writes or persistent credentials.
    pub fn hosting_account_check(&self) -> bool {
        self.hosting_fixture().is_none()
            && read_regular(&self.root.join("rocket-github-browser-check"), 160).is_ok_and(|text|
                text == "ROCKET-GITHUB-BROWSER-CHECK-1\nhttps://github.com\nJasXTD/rocket-auth-test\n")
    }
    pub fn lfs_prune_source(&self, repository: &Path, source: &[u8]) -> Result<(), String> {
        if self.remote_url(source).is_ok() { return Ok(()); }
        self.lfs_prune_marker(repository, source).map(|_| ())
    }
    fn lfs_prune_marker(&self, repository: &Path, source: &[u8]) -> Result<rocket_git::LfsPointer, String> {
        let message = "Network LFS pruning requires an exact fixture marker for rocket-auth-test and one synthetic cache object under 1 KiB";
        let (_, branch) = self.repository(repository)?;
        let source = std::str::from_utf8(source).map_err(|_| message)?;
        let text = read_regular(&repository.join(".git/rocket-lfs-prune-fixture"), 768).map_err(|_| message)?;
        prune_marker_object(&text, &branch, source).ok_or_else(|| message.into())
    }
    pub fn lfs_prune_preview(&self, repository: &Path, preview: &rocket_git::LfsPrunePreview) -> Result<(), String> {
        if self.remote_url(preview.source().as_bytes()).is_ok() { return Ok(()); }
        let object = self.lfs_prune_marker(repository, preview.source().as_bytes())?;
        if preview.objects().len() > 1 || preview.objects().iter().any(|p| p != &object) {
            return Err("LFS prune preview contains an object outside the exact approved synthetic fixture".into());
        }
        Ok(())
    }
    /// Network uploads require a test-runner-owned exact commit/object marker,
    /// in addition to the normal GitHub, branch, remote and fixture-root checks.
    pub fn lfs_upload(
        &self,
        repository: &Path,
        preview: &rocket_git::LfsPreview,
    ) -> Result<(), String> {
        let message = "Network LFS uploads require an approved upload marker for one synthetic object under 1 KiB; otherwise use a local destination";
        self.repository(repository)?;
        let marker = repository.join(".git/rocket-lfs-upload-fixture");
        let text = read_regular(&marker, 512).map_err(|_| message)?;
        let head = match preview.head() {
            rocket_git::Head::Branch { target, .. } => target.as_str(),
            _ => return Err(message.into()),
        };
        let first = preview.entries().first().ok_or(message)?;
        if !upload_marker_matches(&text, head, &first.object.oid, first.object.size)
            || preview.object_count() != 1
            || preview.total_bytes() != first.object.size
            || !preview
                .source()
                .is_some_and(|s| GITHUB_TEST_SOURCES.contains(&s))
        {
            return Err(message.into());
        }
        Ok(())
    }
    /// Development control must not become a launcher for unrelated installed programs.
    pub fn external_executable(&self, path: &Path) -> Result<(), String> {
        self.patch_file(path).map_err(|_| {
            "External tool executables must stay inside the configured fixture root".to_owned()
        })?;
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            return Err("External tool tests require a fixture .exe".into());
        }
        Ok(())
    }
    pub fn patch_file(&self, path: &Path) -> Result<(), String> {
        let error =
            "Patch input/output must be an absolute local path inside the configured fixture root";
        #[cfg(windows)]
        if !matches!(path.components().next(),Some(std::path::Component::Prefix(p)) if matches!(p.kind(),std::path::Prefix::Disk(_)|std::path::Prefix::VerbatimDisk(_)))
        {
            return Err(error.into());
        }
        if !path.is_absolute() {
            return Err(error.into());
        }
        let parent = path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .ok_or(error)?;
        if !parent.starts_with(&self.root) || path.file_name().is_none() {
            return Err(error.into());
        }
        if path.exists() && !path.canonicalize().is_ok_and(|p| p.starts_with(&self.root)) {
            return Err(error.into());
        }
        Ok(())
    }
    /// Creation is restricted to explicitly marked destinations, including folders
    /// without .git yet. The runner owns the adjacent marker, never the application.
    pub fn init_destination(&self, path: &Path, branch: &str) -> Result<(), String> {
        let error =
            "Repository creation requires a marked local destination inside the fixture root";
        #[cfg(windows)]
        if !matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        {
            return Err(error.into());
        }
        if !path.is_absolute() {
            return Err(error.into());
        }
        let parent = path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .ok_or(error)?;
        if !parent.starts_with(&self.root) {
            return Err(error.into());
        }
        if path.exists()
            && !path
                .canonicalize()
                .is_ok_and(|p| p != self.root && p.starts_with(&self.root))
        {
            return Err(error.into());
        }
        let mut name = path.file_name().ok_or(error)?.to_os_string();
        name.push(".rocket-init-fixture");
        let marker = parent.join(name);
        let resolved = marker.canonicalize().map_err(|_| error)?;
        if resolved != marker || !resolved.starts_with(&self.root) {
            return Err(error.into());
        }
        let mut value = String::new();
        std::fs::File::open(&marker)
            .and_then(|f| f.take(129).read_to_string(&mut value))
            .map_err(|_| error)?;
        if value != branch
            || branch.len() > 128
            || branch.len() <= 15
            || !branch.starts_with("rocket-ui-test/")
            || !branch
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
        {
            return Err(error.into());
        }
        Ok(())
    }
    /// New remote-edit probes are local-only; reject UNC/device paths before any
    /// filesystem lookup, then resolve links and check the configured boundary.
    pub fn remote_url(&self, value: &[u8]) -> Result<(), String> {
        let error = "Remote management through MCP accepts only existing local paths inside the fixture root";
        let text = std::str::from_utf8(value).map_err(|_| error)?;
        if self.github_publish_check().is_some_and(|(repository,_)| text == format!("git@github.com:{repository}.git")) { return Ok(()); }
        let path = Path::new(text);
        #[cfg(windows)]
        if !matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        {
            return Err(error.into());
        }
        if !path.is_absolute() || !path.canonicalize().is_ok_and(|p| p.starts_with(&self.root)) {
            return Err(error.into());
        }
        Ok(())
    }
    /// Live history replacement needs a separate runner-owned opt-in, tied to
    /// this repository's existing branch marker and one exact approved URL.
    pub fn force_push_url(&self, repository: &Path, value: &[u8]) -> Result<(), String> {
        let source = std::str::from_utf8(value).map_err(|_| "Invalid force-push URL")?;
        if !GITHUB_TEST_SOURCES.contains(&source) {
            return self.remote_url(value);
        }
        let (root, branch) = self.repository(repository)?;
        let message = "Live force-with-lease requires an approved marker for this exact fixture branch and URL";
        let marker =
            read_regular(&root.join(".git/rocket-force-push-fixture"), 512).map_err(|_| message)?;
        if marker != format!("ROCKET-FORCE-PUSH-1\n{branch}\n{source}\n") {
            return Err(message.into());
        }
        Ok(())
    }
    /// Atomic live tests require their own opt-in. The UI separately confines
    /// every source/destination to the marked branch or its hyphen suffixes.
    pub fn atomic_push_url(&self, repository: &Path, value: &[u8]) -> Result<(), String> {
        let source = std::str::from_utf8(value).map_err(|_| "Invalid atomic-push URL")?;
        if !GITHUB_TEST_SOURCES.contains(&source) {
            return self.remote_url(value);
        }
        let (root, branch) = self.repository(repository)?;
        let message =
            "Live atomic push requires an approved marker for this exact fixture namespace and URL";
        let marker = read_regular(&root.join(".git/rocket-atomic-push-fixture"), 512)
            .map_err(|_| message)?;
        if marker != format!("ROCKET-ATOMIC-PUSH-1\n{branch}\n{source}\n") {
            return Err(message.into());
        }
        Ok(())
    }
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|_| "Test root does not exist")?;
        if !root.is_dir() {
            return Err("Test root must be a directory".into());
        }
        Ok(Self { root })
    }
    /// Permit recovery controls only for a detached rebase of the marked fixture branch.
    pub fn rebase_branch(&self, path: &Path, expected: &str) -> bool {
        let Ok((root, branch)) = self.repository(path) else {
            return false;
        };
        if branch != expected {
            return false;
        }
        let directory = root.join(".git/rebase-merge");
        let redirected = |m: &std::fs::Metadata| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                m.file_attributes() & 0x400 != 0
            }
            #[cfg(not(windows))]
            {
                m.file_type().is_symlink()
            }
        };
        if !std::fs::symlink_metadata(&directory).is_ok_and(|m| m.is_dir() && !redirected(&m)) {
            return false;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let Ok(file) = options.open(directory.join("head-name")) else {
            return false;
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && !redirected(&m))
        {
            return false;
        }
        let mut bytes = Vec::new();
        file.take(256).read_to_end(&mut bytes).is_ok()
            && bytes == format!("refs/heads/{expected}\n").as_bytes()
    }
    /// Git Flow switches between branches. A runner that writes a branch prefix to
    /// `.git/rocket-gitflow-fixture` owns every branch starting with it; the prefix must
    /// start with `rocket-ui-test/` and cover the fixture's own branch. A missing,
    /// redirected or malformed marker owns nothing beyond the fixture branch.
    pub fn gitflow_namespace(&self, path: &Path) -> Option<String> {
        let (root, branch) = self.repository(path).ok()?;
        let text = read_regular(&root.join(".git/rocket-gitflow-fixture"), 128).ok()?;
        let prefix = text.trim_end_matches(['\r', '\n']);
        (prefix.starts_with("rocket-ui-test/")
            && prefix.len() > 15
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
            && branch.starts_with(prefix))
        .then(|| prefix.to_owned())
    }
    /// Detached bisect controls remain confined to the marked fixture branch.
    /// A missing/redirected/malformed journal never grants a general write bypass.
    pub fn bisect_branch(&self, path: &Path, expected: &str) -> bool {
        let Ok((root, branch)) = self.repository(path) else {
            return false;
        };
        if branch != expected {
            return false;
        }
        let path = root.join(".git/rocket-bisect");
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !redirected(&m)) {
            return false;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let Ok(file) = options.open(path) else {
            return false;
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && !redirected(&m))
        {
            return false;
        }
        let mut bytes = Vec::new();
        if file.take(16385).read_to_end(&mut bytes).is_err() || bytes.len() > 16384 {
            return false;
        }
        let p: Vec<_> = bytes.split(|b| *b == 0).collect();
        p.len() == 6
            && p[0] == b"ROCKET-BISECT-1"
            && p[1] == expected.as_bytes()
            && p[2..5]
                .iter()
                .all(|id| matches!(id.len(), 40 | 64) && id.iter().all(|b| b.is_ascii_hexdigit()))
            && p[5].strip_prefix(b"refs/rocket/bisect/").is_some_and(|s| {
                !s.is_empty() && s.iter().all(|b| b.is_ascii_digit() || *b == b'-')
            })
    }
    pub fn repository(&self, path: &Path) -> Result<(PathBuf, String), String> {
        if let Some(result) = self.marked_submodule(path) {
            return result;
        }
        let root = path
            .canonicalize()
            .map_err(|_| "Repository path does not exist")?;
        if std::fs::symlink_metadata(root.join(".git")).is_ok_and(|m| m.is_file()) {
            let (branch, source) = self.worktree_marker(&root)?;
            self.normal_repository(&source)?;
            let common = fixture_git_directory(&source)?
                .canonicalize()
                .map_err(|_| "Missing source repository metadata")?;
            let gitfile = read_regular(&root.join(".git"), 4096)?;
            let target = gitfile
                .trim_end_matches(['\r', '\n'])
                .strip_prefix("gitdir: ")
                .ok_or("Invalid worktree Git file")?;
            let git = root
                .join(target)
                .canonicalize()
                .map_err(|_| "Missing worktree metadata")?;
            if git.parent() != Some(common.join("worktrees").as_path()) {
                return Err("Worktree metadata is outside its marked source repository".into());
            }
            let common_link = read_regular(&git.join("commondir"), 4096)?;
            if git
                .join(common_link.trim_end_matches(['\r', '\n']))
                .canonicalize()
                .ok()
                .as_ref()
                != Some(&common)
            {
                return Err("Worktree common-directory identity changed".into());
            }
            let backlink = read_regular(&git.join("gitdir"), 4096)?;
            if git
                .join(backlink.trim_end_matches(['\r', '\n']))
                .canonicalize()
                .ok()
                != root.join(".git").canonicalize().ok()
            {
                return Err("Worktree backlink does not identify the marked folder".into());
            }
            return Ok((root, branch));
        }
        self.normal_repository(path)
    }
    pub fn is_submodule_fixture(&self, path: &Path) -> bool {
        self.marked_submodule(path).is_some_and(|r| r.is_ok())
    }
    fn marked_submodule(&self, path: &Path) -> Option<Result<(PathBuf, String), String>> {
        let root = path.canonicalize().ok()?;
        if !root.starts_with(&self.root) {
            return None;
        }
        let git = root.join(".git");
        let admin = if git.is_file() {
            let text = read_regular(&git, 4096).ok()?;
            root.join(
                text.trim_end_matches(['\r', '\n'])
                    .strip_prefix("gitdir: ")?,
            )
            .canonicalize()
            .ok()?
        } else {
            git.canonicalize().ok()?
        };
        if !admin.starts_with(&self.root) {
            return Some(Err("Submodule metadata escapes the fixture root".into()));
        }
        let marker = admin.join("rocket-ui-submodule-fixture");
        if !marker.exists() {
            return None;
        }
        Some((|| {
            let source = read_regular(&marker, 4096)?;
            let source = Path::new(source.trim_end_matches(['\r', '\n']));
            let (owner, branch) = self.normal_repository(source)?;
            if root == owner || !root.starts_with(&owner) {
                return Err("Submodule fixture is not inside its marked parent".into());
            }
            // Root and metadata must be unredirected throughout the fixture tree.
            for p in root.ancestors().take_while(|p| p.starts_with(&self.root)) {
                if std::fs::symlink_metadata(p).is_ok_and(|m| redirected(&m)) {
                    return Err("Redirected submodule fixture path".into());
                }
            }
            for p in admin.ancestors().take_while(|p| p.starts_with(&self.root)) {
                if std::fs::symlink_metadata(p).is_ok_and(|m| redirected(&m)) {
                    return Err("Redirected submodule fixture metadata".into());
                }
            }
            Ok((root, branch))
        })())
    }
    /// The runner, never Rocket, owns this adjacent marker. Both the destination
    /// and its normal source repository must be inside this session's test root.
    pub fn worktree_destination(
        &self,
        path: &Path,
        branch: &str,
        source: &Path,
    ) -> Result<(), String> {
        let (marked, main) = self.worktree_marker(path)?;
        if marked != branch {
            return Err("Worktree branch differs from its fixture marker".into());
        }
        let (source, _) = self.repository(source)?;
        let expected = if source.join(".git").is_file() {
            self.worktree_marker(&source)?.1
        } else {
            source
        };
        if main != expected {
            return Err("Worktree destination belongs to a different fixture repository".into());
        }
        self.normal_repository(&main)?;
        Ok(())
    }
    fn worktree_marker(&self, path: &Path) -> Result<(String, PathBuf), String> {
        if !path.is_absolute() {
            return Err("Worktree fixture requires an absolute path".into());
        }
        for component in path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("Parent traversal in worktree fixture path".into());
            }
        }
        let parent = path
            .parent()
            .ok_or("Worktree fixture needs a parent")?
            .canonicalize()
            .map_err(|_| "Missing worktree parent")?;
        if !parent.starts_with(&self.root) {
            return Err("Worktree is outside the configured test root".into());
        }
        if path.exists()
            && !path
                .canonicalize()
                .is_ok_and(|p| p != self.root && p.starts_with(&self.root))
        {
            return Err("Worktree path escapes the fixture root".into());
        }
        let mut name = path
            .file_name()
            .ok_or("Worktree fixture needs a folder name")?
            .to_os_string();
        name.push(".rocket-worktree-fixture");
        let marker = parent.join(name);
        if marker.canonicalize().ok().as_ref() != Some(&marker) {
            return Err("Redirected worktree marker refused".into());
        }
        let value = read_regular(&marker, 8192)?;
        let (branch, source) = value
            .split_once('\n')
            .ok_or("Worktree fixture marker needs branch and source")?;
        if !branch.starts_with("rocket-ui-test/")
            || branch.len() <= 15
            || branch.len() > 128
            || !branch
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
        {
            return Err("Invalid worktree fixture branch".into());
        }
        let source = Path::new(source)
            .canonicalize()
            .map_err(|_| "Missing worktree fixture source")?;
        if !source.starts_with(&self.root) || source == self.root {
            return Err("Worktree fixture source escapes the test root".into());
        }
        Ok((branch.into(), source))
    }
    fn normal_repository(&self, path: &Path) -> Result<(PathBuf, String), String> {
        let path = path
            .canonicalize()
            .map_err(|_| "Repository path does not exist")?;
        if path == self.root || !path.starts_with(&self.root) {
            return Err("Repository is outside the configured test root".into());
        }
        let git = fixture_git_directory(&path)?
            .canonicalize()
            .map_err(|_| "A marked fixture repository is required")?;
        if !git.is_dir() || !git.starts_with(&path) {
            return Err(
                "Linked or redirected Git directories are not supported by this test interface"
                    .into(),
            );
        }
        let marker = git
            .join("rocket-ui-fixture")
            .canonicalize()
            .map_err(|_| "Missing rocket-ui-fixture ownership marker in the Git directory")?;
        if !marker.starts_with(&git) {
            return Err("Redirected ownership marker refused".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&marker)
            .and_then(|file| file.take(129).read_to_end(&mut bytes))
            .map_err(|_| "Cannot read fixture marker")?;
        if bytes.len() > 128 {
            return Err("Invalid fixture branch marker".into());
        }
        let branch = String::from_utf8(bytes).map_err(|_| "Invalid fixture branch marker")?;
        if !branch.starts_with("rocket-ui-test/")
            || branch.len() <= 15
            || !branch
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
        {
            return Err("Fixture branch marker must name one rocket-ui-test/ branch".into());
        }
        Ok((path, branch))
    }
}

fn fixture_git_directory(path: &Path) -> Result<PathBuf, String> {
    if path.join(".git").is_dir() { return Ok(path.join(".git")); }
    // Bare fixtures have their explicit marker beside HEAD, never in an arbitrary
    // parent folder. Repository::open independently verifies Git's repository type.
    read_regular(&path.join("HEAD"), 4096)?;
    read_regular(&path.join("config"), 65536)?;
    for name in ["objects", "refs"] {
        let directory=path.join(name);
        if !directory.is_dir() || directory.canonicalize().ok().as_deref() != Some(directory.as_path()) {
            return Err("Invalid or redirected bare repository metadata".into());
        }
    }
    Ok(path.to_owned())
}

fn read_regular(path: &Path, limit: u64) -> Result<String, String> {
    let redirected = |m: &std::fs::Metadata| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            m.file_attributes() & 0x400 != 0
        }
        #[cfg(not(windows))]
        {
            m.file_type().is_symlink()
        }
    };
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| "Missing worktree fixture metadata")?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && !redirected(&m))
    {
        return Err("Worktree metadata must be a regular file".into());
    }
    let mut value = String::new();
    file.take(limit + 1)
        .read_to_string(&mut value)
        .map_err(|_| "Cannot read worktree fixture metadata")?;
    if value.len() as u64 > limit {
        return Err("Worktree fixture metadata exceeds its limit".into());
    }
    Ok(value)
}

fn upload_marker_matches(text: &str, head: &str, oid: &str, bytes: u64) -> bool {
    bytes > 0
        && bytes < 1024
        && oid.len() == 64
        && oid.bytes().all(|b| b.is_ascii_hexdigit())
        && matches!(head.len(), 40 | 64)
        && head.bytes().all(|b| b.is_ascii_hexdigit())
        && text == format!("ROCKET-LFS-UPLOAD-1\n{head}\n{oid}\n{bytes}\n")
}

fn prune_marker_object(text: &str, branch: &str, source: &str) -> Option<rocket_git::LfsPointer> {
    if !GITHUB_TEST_SOURCES.contains(&source) { return None; }
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() != 5 { return None; }
    let oid = lines[3];
    let size = lines[4].parse::<u64>().ok()?;
    if size == 0 || size >= 1024 || oid.len() != 64
        || !oid.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || text != format!("ROCKET-LFS-PRUNE-1\n{branch}\n{source}\n{oid}\n{size}\n") { return None; }
    Some(rocket_git::LfsPointer { oid: oid.into(), size })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_must_sit_in_the_preview_folder_and_limits_stay_bounded() {
        let root=std::env::temp_dir().join(format!("rocket-preview-policy-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();let p=Policy::new(&root).unwrap();
        let cache=p.root.join("rocket-previews");let slot=cache.join("p12-ab-0");
        assert!(p.preview_repository(&slot.join("repository.git")).is_err(),"no preview folder yet");
        std::fs::create_dir_all(&slot).unwrap();
        // Opening: the repository folder doesn't exist yet.
        assert!(p.preview_repository(&slot.join("repository.git")).is_ok());
        std::fs::create_dir(slot.join("repository.git")).unwrap();assert!(p.preview_repository(&slot.join("repository.git")).is_ok());
        for wrong in [slot.join("other.git"),cache.join("repository.git"),slot.join("repository.git/inner"),cache.join("..").join("p12-ab-0").join("repository.git"),p.root.join("q1").join("repository.git"),p.root.join("rocket-previews2/p1/repository.git")] {
            assert!(p.preview_repository(&wrong).is_err(),"{}",wrong.display());
        }
        let other=cache.join("x1");std::fs::create_dir(&other).unwrap();assert!(p.preview_repository(&other.join("repository.git")).is_err(),"slot names start with p");
        assert_eq!(p.preview_limit(),None);
        for (text,limit) in [("2000000\n",Some(2_000_000)),("0",None),("abc",None),(&*format!("{}",rocket_git::PREVIEW_LIMIT+1),None),("1",Some(1))] {
            std::fs::write(p.root.join("rocket-preview-limit"),text).unwrap();assert_eq!(p.preview_limit(),limit,"{text}");
        }
        std::fs::remove_dir_all(&p.root).unwrap();
    }
    #[test]
    fn bare_repository_needs_scoped_metadata_and_ownership_marker() {
        let root=std::env::temp_dir().join(format!("rocket-bare-policy-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let repo=root.join("bare.git");std::fs::create_dir_all(repo.join("objects")).unwrap();std::fs::create_dir(repo.join("refs")).unwrap();
        let p=Policy::new(&root).unwrap();let branch="rocket-ui-test/bare";
        std::fs::write(repo.join("HEAD"),format!("ref: refs/heads/{branch}\n")).unwrap();std::fs::write(repo.join("config"),"[core]\n bare = true\n").unwrap();
        assert!(p.repository(&repo).is_err());std::fs::write(repo.join("rocket-ui-fixture"),branch).unwrap();assert_eq!(p.repository(&repo).unwrap().1,branch);
        std::fs::write(repo.join("rocket-ui-fixture"),"main").unwrap();assert!(p.repository(&repo).is_err());std::fs::write(repo.join("rocket-ui-fixture"),branch).unwrap();
        std::fs::remove_file(repo.join("HEAD")).unwrap();assert!(p.repository(&repo).is_err());
        let resolved=root.canonicalize().unwrap();assert_eq!(resolved.parent(),Some(std::env::temp_dir().canonicalize().unwrap().as_path()));std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn live_publish_marker_pins_private_repository_branch_and_transport() {
        let root=std::env::temp_dir().join(format!("rocket-publish-policy-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();let p=Policy::new(&root).unwrap();
        let repository="Tester/rocket-publish-test-0123456789abcdef0123456789abcdef";
        let branch="rocket-ui-test/publish-0123456789ab";
        let valid=format!("ROCKET-GITHUB-PUBLISH-CHECK-1\n{repository}\n{branch}\nprivate\nssh\n");
        let marker=root.join("rocket-github-publish-check");assert!(p.github_publish_check().is_none());
        let url=format!("git@github.com:{repository}.git");assert!(p.remote_url(url.as_bytes()).is_err());
        std::fs::write(&marker,&valid).unwrap();assert_eq!(p.github_publish_check(),Some((repository.into(),branch.into())));assert!(p.remote_url(url.as_bytes()).is_ok());
        for other in [url.replace("github.com","github.com.invalid"),url.replace("Tester/","Other/"),format!("https://github.com/{repository}.git"),GITHUB_TEST_SOURCES[1].into()] {assert!(p.remote_url(other.as_bytes()).is_err());}
        for invalid in [valid.replace("private","public"),valid.replace("\nssh\n","\nhttps\n"),valid.replace(branch,"rocket-ui-test/other"),format!("{valid}extra\n"),valid.replace("rocket-publish-test-","existing-")] {std::fs::write(&marker,invalid).unwrap();assert!(p.github_publish_check().is_none());}
        std::fs::write(&marker,&valid).unwrap();std::fs::write(root.join("rocket-hosting-api"),"http://127.0.0.1:12345").unwrap();assert!(p.github_publish_check().is_none());
        let actual=root.canonicalize().unwrap();assert_eq!(actual.parent(),Some(std::env::temp_dir().canonicalize().unwrap().as_path()));std::fs::remove_dir_all(actual).unwrap();
    }
    #[test]
    fn live_browser_check_requires_separate_exact_marker() {
        let root = std::env::temp_dir().join(format!("rocket-browser-policy-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        let p = Policy::new(&root).unwrap();
        let marker = root.join("rocket-github-browser-check");
        assert!(!p.hosting_account_check());
        std::fs::write(root.join("rocket-github-api"), "https://api.github.com").unwrap();
        assert!(!p.hosting_account_check());
        let valid = "ROCKET-GITHUB-BROWSER-CHECK-1\nhttps://github.com\nJasXTD/rocket-auth-test\n";
        std::fs::write(&marker, valid).unwrap();
        assert!(p.hosting_account_check());
        for invalid in [valid.replace("github.com", "github.com.invalid"), valid.replace("rocket-auth-test", "another"), format!("{valid}extra\n")] {
            std::fs::write(&marker, invalid).unwrap(); assert!(!p.hosting_account_check());
        }
        std::fs::write(&marker, valid).unwrap();
        std::fs::write(root.join("rocket-hosting-api"), "http://127.0.0.1:12345").unwrap();
        assert!(!p.hosting_account_check());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn live_lfs_prune_pins_branch_source_and_one_small_object() {
        let root = std::env::temp_dir().join(format!("rocket-prune-policy-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let repo = root.join("repo"); let git = repo.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let p = Policy::new(&root).unwrap(); let branch = "rocket-ui-test/prune";
        std::fs::write(git.join("rocket-ui-fixture"), branch).unwrap();
        let oid = "a".repeat(64); let marker = git.join("rocket-lfs-prune-fixture");
        for source in GITHUB_TEST_SOURCES {
            assert!(p.lfs_prune_source(&repo, source.as_bytes()).is_err());
            let valid = format!("ROCKET-LFS-PRUNE-1\n{branch}\n{source}\n{oid}\n72\n");
            std::fs::write(&marker, &valid).unwrap();
            assert_eq!(p.lfs_prune_marker(&repo, source.as_bytes()).unwrap().oid, oid);
            for invalid in [valid.replace(branch, "rocket-ui-test/other"), valid.replace("rocket-auth-test", "another"), valid.replace("72\n", "1024\n"), valid.replace(&oid, "bad"), format!("{valid}extra\n")] {
                std::fs::write(&marker, invalid).unwrap(); assert!(p.lfs_prune_source(&repo, source.as_bytes()).is_err());
            }
            std::fs::remove_file(&marker).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn live_force_push_requires_exact_owned_branch_and_endpoint() {
        let root = std::env::temp_dir().join(format!(
            "rocket-force-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = root.join("repo");
        let git = repo.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let policy = Policy::new(&root).unwrap();
        let branch = "rocket-ui-test/live-force";
        let ownership = git.join("rocket-ui-fixture");
        std::fs::write(&ownership, branch).unwrap();
        let marker = git.join("rocket-force-push-fixture");
        for source in GITHUB_TEST_SOURCES {
            assert!(policy.force_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(
                &marker,
                format!("ROCKET-FORCE-PUSH-1\n{branch}\n{source}\n"),
            )
            .unwrap();
            assert!(policy.force_push_url(&repo, source.as_bytes()).is_ok());
            for other in GITHUB_TEST_SOURCES.into_iter().filter(|s| *s != source) {
                assert!(policy.force_push_url(&repo, other.as_bytes()).is_err());
            }
            std::fs::write(&ownership, "rocket-ui-test/other").unwrap();
            assert!(policy.force_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(&ownership, "main").unwrap();
            assert!(policy.force_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(&ownership, branch).unwrap();
            std::fs::remove_file(&marker).unwrap();
        }
        for source in [
            "https://github.com/JasXTD/another.git",
            "https://github.com/JasXTD/rocket-auth-test.git?other",
            "git@github.com:JasXTD/another.git",
            "../outside",
        ] {
            std::fs::write(
                &marker,
                format!("ROCKET-FORCE-PUSH-1\n{branch}\n{source}\n"),
            )
            .unwrap();
            assert!(policy.force_push_url(&repo, source.as_bytes()).is_err());
        }
        assert!(
            policy
                .force_push_url(&repo, repo.to_str().unwrap().as_bytes())
                .is_ok()
        );
        assert!(
            policy
                .remote_url(GITHUB_TEST_SOURCES[0].as_bytes())
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn live_atomic_push_requires_separate_owned_namespace_and_endpoint() {
        let root = std::env::temp_dir().join(format!(
            "rocket-atomic-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = root.join("repo");
        let git = repo.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let policy = Policy::new(&root).unwrap();
        let branch = "rocket-ui-test/live-atomic";
        let ownership = git.join("rocket-ui-fixture");
        let marker = git.join("rocket-atomic-push-fixture");
        std::fs::write(&ownership, branch).unwrap();
        for source in GITHUB_TEST_SOURCES {
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(
                git.join("rocket-force-push-fixture"),
                format!("ROCKET-FORCE-PUSH-1\n{branch}\n{source}\n"),
            )
            .unwrap();
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(
                &marker,
                format!("ROCKET-ATOMIC-PUSH-1\n{branch}\n{source}\n"),
            )
            .unwrap();
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_ok());
            for other in GITHUB_TEST_SOURCES.into_iter().filter(|s| *s != source) {
                assert!(policy.atomic_push_url(&repo, other.as_bytes()).is_err());
            }
            std::fs::write(&ownership, "rocket-ui-test/other").unwrap();
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(&ownership, "main").unwrap();
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_err());
            std::fs::write(&ownership, branch).unwrap();
            std::fs::remove_file(&marker).unwrap();
        }
        for source in [
            "https://github.com/JasXTD/another.git",
            "https://github.com/JasXTD/rocket-auth-test.git?other",
            "git@github.com:JasXTD/another.git",
            "../outside",
        ] {
            std::fs::write(
                &marker,
                format!("ROCKET-ATOMIC-PUSH-1\n{branch}\n{source}\n"),
            )
            .unwrap();
            assert!(policy.atomic_push_url(&repo, source.as_bytes()).is_err());
        }
        assert!(
            policy
                .atomic_push_url(&repo, repo.to_str().unwrap().as_bytes())
                .is_ok()
        );
        assert!(
            policy
                .remote_url(GITHUB_TEST_SOURCES[0].as_bytes())
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn network_upload_marker_pins_one_small_object_and_exact_commit() {
        let head = "a".repeat(40);
        let oid = "b".repeat(64);
        let marker = format!("ROCKET-LFS-UPLOAD-1\n{head}\n{oid}\n72\n");
        assert!(upload_marker_matches(&marker, &head, &oid, 72));
        assert!(!upload_marker_matches(&marker, &"c".repeat(40), &oid, 72));
        assert!(!upload_marker_matches(&marker, &head, &"c".repeat(64), 72));
        assert!(!upload_marker_matches(&marker, &head, &oid, 73));
        assert!(!upload_marker_matches(
            &format!("{marker}extra"),
            &head,
            &oid,
            72
        ));
        assert!(!upload_marker_matches(
            &format!("ROCKET-LFS-UPLOAD-1\n{head}\n{oid}\n1024\n"),
            &head,
            &oid,
            1024
        ));
    }
    #[test]
    fn linked_worktrees_require_adjacent_ownership_and_matching_backlinks() {
        let root = std::env::temp_dir().join(format!(
            "rocket-worktree-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("source/.git/worktrees/topic")).unwrap();
        let policy = Policy::new(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = root.join("source");
        let dest = root.join("linked");
        std::fs::write(
            source.join(".git/rocket-ui-fixture"),
            "rocket-ui-test/source",
        )
        .unwrap();
        assert!(
            policy
                .worktree_destination(&dest, "rocket-ui-test/topic", &source)
                .is_err()
        );
        let marker = root.join("linked.rocket-worktree-fixture");
        std::fs::write(
            &marker,
            format!("rocket-ui-test/topic\n{}", source.display()),
        )
        .unwrap();
        assert!(
            policy
                .worktree_destination(&dest, "rocket-ui-test/topic", &source)
                .is_ok()
        );
        assert!(policy.worktree_destination(&dest, "main", &source).is_err());
        std::fs::create_dir(&dest).unwrap();
        let admin = source.join(".git/worktrees/topic");
        std::fs::write(dest.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", dest.join(".git").display()),
        )
        .unwrap();
        assert_eq!(policy.repository(&dest).unwrap().1, "rocket-ui-test/topic");
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", source.join(".git/rocket-ui-fixture").display()),
        )
        .unwrap();
        assert!(policy.repository(&dest).is_err());
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", dest.join(".git").display()),
        )
        .unwrap();
        std::fs::write(admin.join("commondir"), root.to_string_lossy().as_bytes()).unwrap();
        assert!(policy.repository(&dest).is_err());
        std::fs::remove_file(marker).unwrap();
        assert!(policy.repository(&dest).is_err());
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn scope_requires_a_normal_marked_repository_and_rejects_escape() {
        let root = std::env::temp_dir().join(format!(
            "rocket-mcp-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        let policy = Policy::new(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let destination = root.join("new-project");
        assert!(
            policy
                .init_destination(&destination, "rocket-ui-test/new")
                .is_err()
        );
        let creation_marker = root.join("new-project.rocket-init-fixture");
        std::fs::write(&creation_marker, "rocket-ui-test/new").unwrap();
        assert!(
            policy
                .init_destination(&destination, "rocket-ui-test/new")
                .is_ok()
        );
        assert!(policy.init_destination(&destination, "main").is_err());
        assert!(
            policy
                .init_destination(&root.parent().unwrap().join("escape"), "rocket-ui-test/new")
                .is_err()
        );
        std::fs::remove_file(creation_marker).unwrap();
        assert!(
            policy
                .remote_url(root.join("repo").to_str().unwrap().as_bytes())
                .is_ok()
        );
        for invalid in [
            "https://example.invalid/repo",
            "git@example.invalid:repo",
            r"\\example.invalid\repo",
            "../escape",
        ] {
            assert!(policy.remote_url(invalid.as_bytes()).is_err());
        }
        assert!(
            policy
                .remote_url(root.parent().unwrap().to_str().unwrap().as_bytes())
                .is_err()
        );
        assert!(policy.repository(&root.join("repo")).is_err());
        let marker = root.join("repo/.git/rocket-ui-fixture");
        std::fs::write(&marker, "rocket-ui-test/local-owned").unwrap();
        assert_eq!(
            policy.repository(&root.join("repo")).unwrap().1,
            "rocket-ui-test/local-owned"
        );
        let child = root.join("repo/vendor/子");
        std::fs::create_dir_all(child.join(".git")).unwrap();
        assert!(!policy.is_submodule_fixture(&child));
        let child_marker = child.join(".git/rocket-ui-submodule-fixture");
        std::fs::write(&child_marker, root.join("repo").to_str().unwrap()).unwrap();
        assert!(policy.is_submodule_fixture(&child));
        assert_eq!(
            policy.repository(&child).unwrap().1,
            "rocket-ui-test/local-owned"
        );
        std::fs::write(&child_marker, root.parent().unwrap().to_str().unwrap()).unwrap();
        assert!(policy.repository(&child).is_err());
        assert!(!policy.is_submodule_fixture(&child));
        std::fs::write(&child_marker, root.join("repo").to_str().unwrap()).unwrap();
        assert!(policy.repository(&root.join("repo/../..")).is_err());
        let rebase = root.join("repo/.git/rebase-merge");
        std::fs::create_dir(&rebase).unwrap();
        let name = rebase.join("head-name");
        std::fs::write(&name, "refs/heads/main\n").unwrap();
        assert!(!policy.rebase_branch(&root.join("repo"), "rocket-ui-test/local-owned"));
        std::fs::write(&name, "refs/heads/rocket-ui-test/local-owned\n").unwrap();
        assert!(policy.rebase_branch(&root.join("repo"), "rocket-ui-test/local-owned"));
        assert!(!policy.rebase_branch(&root.join("repo"), "rocket-ui-test/other"));
        std::fs::write(&name, vec![b'x'; 512]).unwrap();
        assert!(!policy.rebase_branch(&root.join("repo"), "rocket-ui-test/local-owned"));
        std::fs::remove_file(name).unwrap();
        std::fs::remove_dir(rebase).unwrap();
        let journal = root.join("repo/.git/rocket-bisect");
        let id = "a".repeat(40);
        std::fs::write(&journal,format!("ROCKET-BISECT-1\0rocket-ui-test/local-owned\0{id}\0{id}\0{id}\0refs/rocket/bisect/1-2")).unwrap();
        assert!(policy.bisect_branch(&root.join("repo"), "rocket-ui-test/local-owned"));
        assert!(!policy.bisect_branch(&root.join("repo"), "rocket-ui-test/other"));
        std::fs::write(&journal, b"ROCKET-BISECT-1\0main").unwrap();
        assert!(!policy.bisect_branch(&root.join("repo"), "rocket-ui-test/local-owned"));
        std::fs::remove_file(journal).unwrap();
        std::fs::write(&marker, "main").unwrap();
        assert!(policy.repository(&root.join("repo")).is_err());
        std::fs::remove_file(child_marker).unwrap();
        std::fs::remove_dir(child.join(".git")).unwrap();
        std::fs::remove_dir(child).unwrap();
        std::fs::remove_dir(root.join("repo/vendor")).unwrap();
        std::fs::remove_file(marker).unwrap();
        std::fs::remove_dir(root.join("repo/.git")).unwrap();
        std::fs::remove_dir(root.join("repo")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
