//! Use Git LFS's retention/remote verification, but delete only the exact
//! reviewed cache objects. Never run an unbounded second destructive prune.
use super::*;
use std::fs::{File, OpenOptions};
#[derive(Clone)]
pub struct LfsPrunePreview {
    repository: PathBuf,
    context: Context,
    refs: Vec<u8>,
    helper: PathBuf,
    remote: String,
    source: String,
    scope: Option<LfsSourceScope>,
    objects: Vec<LfsPointer>,
}
impl std::fmt::Debug for LfsPrunePreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LfsPrunePreview")
            .field("objects", &self.objects.len())
            .finish_non_exhaustive()
    }
}
impl LfsPrunePreview {
    pub fn objects(&self) -> &[LfsPointer] {
        &self.objects
    }
    pub fn bytes(&self) -> u64 {
        self.objects.iter().map(|p| p.size).sum()
    }
    pub fn remote(&self) -> &str {
        &self.remote
    }
    pub fn source(&self) -> &str {
        &self.source
    }
}
#[derive(Debug)]
pub struct LfsPruneReport {
    pub result: Result<()>,
    pub removed: usize,
    pub removed_bytes: u64,
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn path(repo: &Repository, p: &LfsPointer) -> PathBuf {
    repo.info()
        .common_dir
        .join("lfs/objects")
        .join(&p.oid[..2])
        .join(&p.oid[2..4])
        .join(&p.oid)
}
impl Repository {
    /// The local file-transfer adapter can acknowledge upload existence without
    /// checking its disk object. Verify and pin that store ourselves instead of
    /// treating its successful batch response as evidence of a recoverable copy.
    #[cfg(windows)]
    fn pin_prune_source(
        &self,
        remote: &str,
        scope: Option<&LfsSourceScope>,
        objects: &[LfsPointer],
        c: &CancellationToken,
    ) -> Result<Vec<File>> {
        let (_, local) = helper::source(self, remote, scope, c)?;
        let Some(local) = local else {
            return Ok(Vec::new());
        };
        let source = GitClient::from_runner(self.runner.clone()).open(&local, c)?;
        if source.info().common_dir == self.info().common_dir {
            return Err(invalid(
                "lfs_prune_remote: the recovery source must use a different object store.",
            ));
        }
        let mut guards = Vec::new();
        for object in objects {
            c.check()?;
            let file_path = path(&source, object);
            guards.extend(local_files::pin_directories(&source.info().common_dir,file_path.parent().unwrap()).map_err(|_|invalid("lfs_prune_remote_missing: the local remote has no verified recovery object; cache retained."))?);
            let file=disk::open(&file_path,false)?.ok_or_else(||invalid("lfs_prune_remote_missing: the local remote is missing a recovery object; cache retained."))?;
            if file
                .metadata()
                .map_err(|e| Error::io("inspect LFS recovery object", e))?
                .len()
                != object.size
                || owned_sha256(&file, c)? != object.oid
            {
                return Err(invalid(
                    "lfs_prune_remote_corrupt: the local remote recovery object is invalid; cache retained.",
                ));
            }
            guards.push(file);
        }
        Ok(guards)
    }
    #[cfg(not(windows))]
    fn pin_prune_source(
        &self,
        _: &str,
        _: Option<&LfsSourceScope>,
        _: &[LfsPointer],
        _: &CancellationToken,
    ) -> Result<Vec<File>> {
        Err(invalid("LFS cache pruning currently requires Windows."))
    }
    fn prune_refs(&self, c: &CancellationToken) -> Result<Vec<u8>> {
        let worktrees = self.lfs_read(&args(&["worktree", "list", "--porcelain"]), vec![], c)?;
        if worktrees
            .split(|b| *b == b'\n')
            .filter(|r| r.starts_with(b"worktree "))
            .count()
            != 1
        {
            return Err(invalid(
                "lfs_prune_worktrees: review cleanup with a single registered worktree; linked-worktree caches are retained.",
            ));
        }
        if self
            .status(c)?
            .entries
            .iter()
            .any(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        {
            return Err(invalid(
                "lfs_prune_dirty: commit or stash tracked changes before pruning cached objects. Files were not changed.",
            ));
        }
        self.lfs_read(
            &args(&[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(symref)",
            ]),
            vec![],
            c,
        )
    }
    fn prune_scan(&self, c: &CancellationToken) -> Result<(BTreeMap<String, u64>, Vec<File>)> {
        disk::check_existing_store(self)?;
        let root = &self.info().common_dir;
        let store = root.join("lfs/objects");
        if !store.exists() {
            return Ok((BTreeMap::new(), Vec::new()));
        }
        let mut guards = local_files::pin_directories(root, &store)?;
        let mut objects = BTreeMap::new();
        for first in fs::read_dir(&store).map_err(|e| Error::io("read LFS cache", e))? {
            c.check()?;
            let first = first.map_err(|e| Error::io("read LFS cache", e))?;
            let one = first.file_name().to_string_lossy().into_owned();
            if !hex(&one, 2) {
                return Err(invalid(
                    "lfs_prune_layout: unexpected cache directory; inspect it before cleanup.",
                ));
            }
            guards.extend(local_files::pin_directories(root, &first.path())?);
            for second in fs::read_dir(first.path()).map_err(|e| Error::io("read LFS cache", e))? {
                let second = second.map_err(|e| Error::io("read LFS cache", e))?;
                let two = second.file_name().to_string_lossy().into_owned();
                if !hex(&two, 2) {
                    return Err(invalid("lfs_prune_layout: unexpected cache directory."));
                }
                guards.extend(local_files::pin_directories(root, &second.path())?);
                for entry in
                    fs::read_dir(second.path()).map_err(|e| Error::io("read LFS cache", e))?
                {
                    c.check()?;
                    let entry = entry.map_err(|e| Error::io("read LFS cache", e))?;
                    let oid = entry.file_name().to_string_lossy().into_owned();
                    let m = fs::symlink_metadata(entry.path())
                        .map_err(|e| Error::io("inspect LFS cache", e))?;
                    if !hex(&oid, 64)
                        || !oid.starts_with(&format!("{one}{two}"))
                        || !m.is_file()
                        || local_files::redirected(&m)
                        || objects.len() >= 10_000
                    {
                        return Err(invalid(
                            "lfs_prune_layout: unsupported or excessive cache entries; no objects were removed.",
                        ));
                    }
                    objects.insert(oid, m.len());
                }
            }
        }
        Ok((objects, guards))
    }
    pub fn preview_lfs_prune(
        &self,
        remote: &str,
        scope: Option<&LfsSourceScope>,
        c: &CancellationToken,
    ) -> Result<LfsPrunePreview> {
        self.runner.step("lfs-prune-preview","verify_prunable_lfs_objects",||{
            let context=self.lfs_context(c)?;let refs=self.prune_refs(c)?;
            let (helper,_)=helper::locate(self,c)?;let (source,_)=helper::source(self,remote,scope,c)?;
            let (cache,_guards)=self.prune_scan(c)?;
            let mut config=helper::config(Some((remote,&source)));
            // Keep a conservative recent window; never inherit force/recent or
            // exclusion settings that can discard currently checked-out content.
            config.extend([(String::from("lfs.pruneremotetocheck"),remote.into()),("lfs.fetchrecentrefsdays".into(),"7".into()),("lfs.fetchrecentcommitsdays".into(),"3".into()),("lfs.pruneoffsetdays".into(),"3".into())]);
            let output=self.runner.lfs(&helper,self.cwd(),&args(&["prune","--dry-run","--verbose","--verify-remote","--verify-unreachable","--when-unverified=halt"]),&config,c)?;
            if !output.status.success(){return Err(invalid("lfs_prune_verification: Git LFS could not verify the cleanup candidates on the selected remote. No objects were removed; check access and remote availability."));}
            let mut raw=output.stdout;raw.extend_from_slice(&output.stderr);
            let text=std::str::from_utf8(&raw).map_err(|_|invalid("lfs_prune_response: unsupported helper output"))?;
            let mut objects=Vec::new();let mut declared=None;let mut retained_summary=None;
            for line in text.split(['\r','\n']){
                let line=line.trim();let line=line.strip_prefix("prune: ").unwrap_or(line);
                if line.contains("file would be pruned")||line.contains("files would be pruned") {declared=line.split_whitespace().next().and_then(|n|n.parse::<usize>().ok());}
                // Non-interactive Git LFS 3.8.1 prints the retained/total
                // summary instead of the transient "would be pruned" progress.
                let parts=line.split(',').map(str::trim).collect::<Vec<_>>();
                if parts.len()>=2 && (parts[0].ends_with(" local object")||parts[0].ends_with(" local objects")) && parts[1].ends_with(" retained")
                    && let (Some(total),Some(retained))=(parts[0].split_whitespace().next().and_then(|n|n.parse::<usize>().ok()),parts[1].split_whitespace().next().and_then(|n|n.parse::<usize>().ok())) {retained_summary=total.checked_sub(retained);}
                if let Some(oid)=line.strip_prefix("* ").and_then(|s|s.split_whitespace().next()) {
                    if !hex(oid,64)||objects.len()>=1000{return Err(invalid("lfs_prune_response: invalid or excessive prune candidates"));}
                    let size=*cache.get(oid).ok_or_else(||invalid("lfs_prune_stale: a reviewed cache object moved"))?;
                    if size>LFS_FILE_LIMIT{return Err(invalid("lfs_prune_limit: reviewed objects must be at most 512 MiB each."));}
                    objects.push(LfsPointer{oid:oid.into(),size});
                }
            }
            objects.sort_by(|a,b|a.oid.cmp(&b.oid));
            if (declared.or(retained_summary)!=Some(objects.len()))||objects.windows(2).any(|w|w[0].oid==w[1].oid){return Err(invalid("lfs_prune_response: helper candidate count did not match the object list"));}
            let _source_guards=self.pin_prune_source(remote,scope,&objects,c)?;
            let fresh=self.lfs_context(c)?;
            if !context.matches_after_fetch(&fresh,Some(&source))||refs!=self.prune_refs(c)?||cache!=self.prune_scan(c)?.0{return Err(invalid("lfs_prune_stale: repository or cache changed during review."));}
            Ok(LfsPrunePreview{repository:self.info().git_dir.clone(),context:fresh,refs,helper,remote:remote.into(),source,scope:scope.cloned(),objects})
        })
    }
    pub fn apply_lfs_prune(
        &self,
        p: &LfsPrunePreview,
        c: &CancellationToken,
    ) -> Result<LfsPruneReport> {
        let _permit = self.acquire_write(c)?;
        let mut report = LfsPruneReport {
            result: Ok(()),
            removed: 0,
            removed_bytes: 0,
        };
        report.result = self.runner.step("lfs-prune","apply_lfs_cache_prune",|| {
            if p.repository != self.info().git_dir
                || p.context != self.lfs_context(c)?
                || p.refs != self.prune_refs(c)?
            {
                return Err(invalid(
                    "lfs_prune_stale: repository changed; review cleanup again.",
                ));
            }
            let fresh = self.preview_lfs_prune(&p.remote, p.scope.as_ref(), c)?;
            if fresh.objects != p.objects || fresh.source != p.source || fresh.helper != p.helper {
                return Err(invalid(
                    "lfs_prune_stale: candidates or destination changed; review again.",
                ));
            }
            let _source_guards =
                self.pin_prune_source(&p.remote, p.scope.as_ref(), &p.objects, c)?;
            let (_, _guards) = self.prune_scan(c)?;
            for object in &p.objects {
                c.check()?;
                if !fresh
                    .context
                    .matches_after_fetch(&self.lfs_context(c)?, Some(&p.source))
                    || p.refs != self.prune_refs(c)?
                {
                    return Err(invalid(
                        "lfs_prune_changed: repository changed during cleanup. Remaining objects were kept.",
                    ));
                }
                self.runner
                    .step("lfs-prune", "delete_verified_cache_object", || {
                        delete_verified(self, &p.helper, object, c)
                    })?;
                report.removed += 1;
                report.removed_bytes += object.size;
            }
            Ok(())
        });
        Ok(report)
    }
}
#[cfg(windows)]
fn delete_verified(
    repo: &Repository,
    _helper: &Path,
    object: &LfsPointer,
    c: &CancellationToken,
) -> Result<()> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    let path = path(repo, object);
    let file = OpenOptions::new()
        .access_mode(0x80000000 | 0x10000)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(&path)
        .map_err(|e| Error::io("lock reviewed LFS object", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect reviewed LFS object", e))?;
    if !meta.is_file() || local_files::redirected(&meta) || meta.len() != object.size {
        return Err(invalid("lfs_prune_stale: cache object changed"));
    }
    if owned_sha256(&file, c)? != object.oid {
        return Err(invalid(
            "lfs_prune_corrupt: cache content does not match its object ID; object retained.",
        ));
    }
    c.check()?;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: u32,
            data: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let delete: u32 = 1;
    // SAFETY: the owned handle has DELETE access; FileDispositionInfo contains
    // one BOOL. Windows deletes this verified file on close, never a path substitute.
    if unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u32).cast(), 4)
    } == 0
    {
        return Err(Error::io(
            "remove reviewed LFS object",
            std::io::Error::last_os_error(),
        ));
    }
    drop(file);
    Ok(())
}
#[cfg(not(windows))]
fn delete_verified(_: &Repository, _: &Path, _: &LfsPointer, _: &CancellationToken) -> Result<()> {
    Err(invalid(
        "LFS cache pruning currently requires Windows handle-based deletion.",
    ))
}

/// Hash through the same immutable handle we will delete. Opening the path in a
/// helper would either break Windows sharing rules or introduce a substitution gap.
#[cfg(windows)]
fn owned_sha256(mut file: &File, c: &CancellationToken) -> Result<String> {
    use std::{ffi::c_void, io::Read, ptr};
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptOpenAlgorithmProvider(
            out: *mut *mut c_void,
            name: *const u16,
            provider: *const u16,
            flags: u32,
        ) -> i32;
        fn BCryptCreateHash(
            algorithm: *mut c_void,
            out: *mut *mut c_void,
            object: *mut u8,
            length: u32,
            secret: *mut u8,
            secret_length: u32,
            flags: u32,
        ) -> i32;
        fn BCryptHashData(hash: *mut c_void, data: *const u8, length: u32, flags: u32) -> i32;
        fn BCryptFinishHash(hash: *mut c_void, out: *mut u8, length: u32, flags: u32) -> i32;
        fn BCryptDestroyHash(hash: *mut c_void) -> i32;
        fn BCryptCloseAlgorithmProvider(algorithm: *mut c_void, flags: u32) -> i32;
    }
    struct Algorithm(*mut c_void);
    impl Drop for Algorithm {
        fn drop(&mut self) {
            unsafe {
                BCryptCloseAlgorithmProvider(self.0, 0);
            }
        }
    }
    struct Hash(*mut c_void);
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                BCryptDestroyHash(self.0);
            }
        }
    }
    fn checked(status: i32) -> Result<()> {
        if status >= 0 {
            Ok(())
        } else {
            Err(invalid(
                "lfs_prune_hash: Windows could not verify the cached object; it was retained.",
            ))
        }
    }
    let mut algorithm = ptr::null_mut();
    // SAFETY: all buffers have the documented lengths, handles are owned by
    // RAII guards, and Windows allocates the hash object when its buffer is null.
    checked(unsafe {
        BCryptOpenAlgorithmProvider(
            &mut algorithm,
            [83u16, 72, 65, 50, 53, 54, 0].as_ptr(),
            ptr::null(),
            0,
        )
    })?;
    let algorithm = Algorithm(algorithm);
    let mut hash = ptr::null_mut();
    checked(unsafe {
        BCryptCreateHash(
            algorithm.0,
            &mut hash,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            0,
            0,
        )
    })?;
    let hash = Hash(hash);
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let n = file
            .read(&mut buffer)
            .map_err(|e| Error::io("hash reviewed LFS object", e))?;
        if n == 0 {
            break;
        }
        checked(unsafe { BCryptHashData(hash.0, buffer.as_ptr(), n as u32, 0) })?;
    }
    let mut digest = [0u8; 32];
    checked(unsafe { BCryptFinishHash(hash.0, digest.as_mut_ptr(), 32, 0) })?;
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}
