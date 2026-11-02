//! Reviewed saves which retain working files. Preparation uses private indexes;
//! the live index is locked, backed up and replaced only for explicit untracking.
use crate::{local_files, parse, runner::args, *};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_FILES: usize = 256;
const MAX_BYTES: usize = 64 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);
fn invalid(code: &str, message: &str) -> Error {
    Error::InvalidInput(format!("{code}: {message}"))
}
fn stale() -> Error {
    invalid(
        "keep_local_stale",
        "Files, staging, checkout or stash tip changed. Review again; inspect the reported publication state before retrying.",
    )
}
fn unique() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn write_new(path: &Path, bytes: &[u8], c: &CancellationToken) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("reserve keep-local file", e))?;
    for chunk in bytes.chunks(65536) {
        c.check()?;
        f.write_all(chunk)
            .map_err(|e| Error::io("write keep-local file", e))?;
    }
    f.sync_all()
        .map_err(|e| Error::io("flush keep-local file", e))
}
fn open_read(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let f = options
        .open(path)
        .map_err(|e| Error::io("pin keep-local file", e))?;
    let m = f
        .metadata()
        .map_err(|e| Error::io("inspect keep-local file", e))?;
    if !m.is_file() || local_files::redirected(&m) {
        return Err(invalid(
            "keep_local_unsupported",
            "Use normal files without links or redirection.",
        ));
    }
    Ok(f)
}
fn read_handle(f: &mut File, c: &CancellationToken) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        c.check()?;
        let n = f
            .read(&mut chunk)
            .map_err(|e| Error::io("read keep-local file", e))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
        if out.len() > MAX_BYTES {
            return Err(invalid(
                "keep_local_limit",
                "Review at most 256 files and 64 MiB at a time (16 MiB per working file).",
            ));
        }
    }
    Ok(out)
}
fn index_bytes(repo: &Repository, c: &CancellationToken) -> Result<Vec<u8>> {
    let root = &repo.info().git_dir;
    let _pin = local_files::pin_directories(root, root)?;
    let mut f = open_read(&root.join("index"))?;
    crate::review::verify_preview_handle(&f, root, &root.join("index"))?;
    read_handle(&mut f, c)
}
fn normal_index(repo: &Repository, c: &CancellationToken) -> Result<()> {
    let split = repo
        .runner
        .run(
            repo.cwd(),
            &args(&["rev-parse", "--shared-index-path"]),
            vec![],
            c,
        )?
        .success("keep-local-read")?;
    if !parse::line(&split).is_empty() {
        return Err(invalid(
            "keep_local_index",
            "Split indexes are not supported here; use a normal index and review again.",
        ));
    }
    Ok(())
}
struct IndexLock {
    path: PathBuf,
    file: Option<File>,
    owned: bool,
    _pins: Vec<File>,
}
impl IndexLock {
    fn acquire(repo: &Repository) -> Result<Self> {
        let root = &repo.info().git_dir;
        let pins = local_files::pin_directories(root, root)?;
        let path = root.join("index.lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                Error::io(
                    "acquire keep-local index lock (another Git operation may be running)",
                    e,
                )
            })?;
        Ok(Self {
            path,
            file: Some(file),
            owned: true,
            _pins: pins,
        })
    }
    fn publish(&mut self, bytes: &[u8], c: &CancellationToken) -> Result<()> {
        let f = self.file.as_mut().unwrap();
        f.write_all(bytes)
            .and_then(|()| f.sync_all())
            .map_err(|e| Error::io("flush reviewed index", e))?;
        c.check()?;
        self.file.take();
        local_files::move_path(&self.path, &self.path.with_file_name("index"), true)?;
        self.owned = false;
        Ok(())
    }
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        self.file.take();
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// Deny replacement/writes on Windows during final capture/publication. Missing
/// files are checked again: a new file is never removed by either workflow.
fn pin_files(
    repo: &Repository,
    files: &[FileEditPreview],
    c: &CancellationToken,
) -> Result<Vec<File>> {
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    let mut pins = Vec::new();
    for p in files {
        repo.recheck_file_edit(p, c)?;
        let target = root.join(p.path().argument()?);
        pins.extend(local_files::pin_directories(
            root,
            target.parent().unwrap(),
        )?);
        if let Some(before) = p.before() {
            let mut f = open_read(&target)?;
            crate::review::verify_preview_handle(&f, root, &target)?;
            if read_handle(&mut f, c)? != before {
                return Err(stale());
            }
            pins.push(f);
        }
    }
    Ok(pins)
}
fn capture(
    repo: &Repository,
    paths: &[RepoPath],
    c: &CancellationToken,
) -> Result<Vec<FileEditPreview>> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(invalid(
            "keep_local_selection",
            "Choose between 1 and 256 files.",
        ));
    }
    let mut files = Vec::new();
    let mut bytes = 0;
    for path in paths {
        c.check()?;
        let p = repo.patch_file_base(path, FileEditSource::Patch, c)?;
        bytes += p.before().map_or(0, <[u8]>::len);
        if bytes > MAX_BYTES {
            return Err(invalid(
                "keep_local_limit",
                "Review at most 64 MiB of working files at a time.",
            ));
        }
        files.push(p);
    }
    Ok(files)
}
fn check(
    repo: &Repository,
    repository: &Path,
    head: &Head,
    index: &[u8],
    files: &[FileEditPreview],
    c: &CancellationToken,
) -> Result<()> {
    c.check()?;
    repo.require_idle(c)?;
    if repo.info().git_dir != repository || repo.head(c)? != *head || index_bytes(repo, c)? != index
    {
        return Err(stale());
    }
    for p in files {
        repo.recheck_file_edit(p, c)?;
    }
    Ok(())
}
fn paths_input(paths: impl Iterator<Item = RepoPath>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
    }
    bytes
}

#[derive(Clone, Debug)]
pub struct IgnoreTrackedPreview {
    repository: PathBuf,
    head: Head,
    index: Vec<u8>,
    files: Vec<FileEditPreview>,
    nested_rules: Vec<FileEditPreview>,
    rules: IgnorePreview,
}
impl IgnoreTrackedPreview {
    pub fn files(&self) -> &[FileEditPreview] {
        &self.files
    }
    pub fn rules(&self) -> &IgnorePreview {
        &self.rules
    }
}
pub struct IgnoreTrackedReport {
    pub result: Result<()>,
    pub recovery: Option<PathBuf>,
    pub rules_saved: bool,
    pub index_published: bool,
    pub status: Result<Status>,
}
impl Repository {
    fn check_ignore_tracked(&self, p: &IgnoreTrackedPreview, c: &CancellationToken) -> Result<()> {
        check(self, &p.repository, &p.head, &p.index, &p.files, c)?;
        for rules in &p.nested_rules {
            self.recheck_file_edit(rules, c)?;
        }
        Ok(())
    }
    pub fn preview_ignore_tracked(
        &self,
        selected: &[RepoPath],
        c: &CancellationToken,
    ) -> Result<IgnoreTrackedPreview> {
        self.runner.step("ignore-tracked-preview","review_keep_local_paths",||{
            self.require_worktree(c)?;self.require_idle(c)?;normal_index(self,c)?;
            if selected.is_empty() || selected.len()>MAX_FILES{return Err(invalid("keep_local_selection","Select files or folders to stop tracking."));}
            let index=index_bytes(self,c)?;
            let status=self.status(c)?;
            let source=self.ignore_snapshot(c)?;
            let tracked=self.runner.run(self.cwd(),&args(&["ls-files","--cached","-z"]),vec![],c)?.success("ignore-tracked-preview")?;
            let mut paths=BTreeSet::new();let mut rules=Vec::new();
            for selected in selected {
                literal_ignore_rule(selected,false)?;
                let prefix=[selected.as_bytes(),b"/"].concat();
                let found:Vec<_>=tracked.split(|b|*b==0).filter(|p|!p.is_empty()&&(*p==selected.as_bytes()||p.starts_with(&prefix))).collect();
                if found.is_empty(){return Err(invalid("keep_local_selection","A selected path has no tracked files. Refresh the selection."));}
                rules.push(literal_ignore_rule(selected,!found.contains(&selected.as_bytes()))?);
                for path in found {if path.eq_ignore_ascii_case(b".gitignore"){return Err(invalid("keep_local_selection","Keep the root .gitignore tracked; select other files or folders."));}paths.insert(path.to_vec());}
            }
            let paths:Vec<_>=paths.into_iter().map(RepoPath::new).collect::<Result<_>>()?;
            if status.entries.iter().any(|e|e.staged()&&(paths.contains(&e.path)||e.original_path.as_ref().is_some_and(|p|paths.contains(p)))){return Err(invalid("ignore_tracked_staged","Selected files have staged changes. Commit or unstage those files first; other staged files may stay."));}
            let files=capture(self,&paths,c)?;
            if files.iter().any(|p|p.before().is_none()){return Err(invalid("ignore_tracked_missing","A selected local file is missing. Restore it or handle its deletion separately."));}
            let text=append_ignore_rules(source.text(),&rules)?;
            let rules=self.preview_ignore(&source,&text,c)?;
            let mut nested=BTreeSet::new();
            for file in &files {
                let mut path=file.path().as_bytes();
                while let Some(i)=path.iter().rposition(|b|*b==b'/') {path=&path[..i];nested.insert([path,b"/.gitignore"].concat());}
            }
            let nested_rules=if nested.is_empty(){Vec::new()}else{capture(self,&nested.into_iter().map(RepoPath::new).collect::<Result<Vec<_>>>()?,c)?};
            let p=IgnoreTrackedPreview{repository:self.info().git_dir.clone(),head:status.head,index,files,nested_rules,rules};
            self.check_ignore_tracked(&p,c)?;Ok(p)
        })
    }
    pub fn ignore_tracked(
        &self,
        p: &IgnoreTrackedPreview,
        c: &CancellationToken,
    ) -> Result<IgnoreTrackedReport> {
        let _permit = self.acquire_write(c)?;
        self.check_ignore_tracked(p, c)?;
        let mut recovery = None;
        let mut rules_saved = false;
        let mut index_published = false;
        let result=self.runner.step("ignore-tracked","save_rules_and_untrack",||{
            let mut lock=IndexLock::acquire(self)?;
            let _files=pin_files(self,&p.files,c)?;
            let _rules=pin_files(self,&p.nested_rules,c)?;
            self.check_ignore_tracked(p,c)?;
            let dir=p.repository.join(format!("rocket-untrack-{}",unique()));
            fs::create_dir(&dir).map_err(|e|Error::io("reserve untrack recovery",e))?;recovery=Some(dir.clone());
            let _dir=local_files::pin_directories(&p.repository,&dir)?;
            write_new(&dir.join("index-before"),&p.index,c)?;
            write_new(&dir.join("ignore-before"),p.rules.before(),c)?;
            write_new(&dir.join("RECOVERY.md"),format!("Original index and root ignore rules before Keep local and stop tracking. Original .gitignore existed: {}. Local files were never deleted. Compare current staging before restoring index-before; do not overwrite newer staging. Ignore replacement also keeps its own recovery folder.\n",p.rules.existed()).as_bytes(),c)?;
            let prepared=dir.join("index-after");write_new(&prepared,&p.index,c)?;
            let zero="0".repeat(if self.info().object_format==ObjectFormat::Sha256{64}else{40});
            let mut input=Vec::new();for f in &p.files{input.extend_from_slice(format!("0 {zero}\t").as_bytes());input.extend_from_slice(f.path().as_bytes());input.push(0);}
            self.runner.private_index(self.cwd(),&prepared,&args(&["update-index","-z","--index-info"]),input,c)?.success("ignore-tracked")?;
            let after=read_handle(&mut open_read(&prepared)?,c)?;
            self.check_ignore_tracked(p,c)?;
            let r=self.apply_ignore_locked(&p.rules,c)?;rules_saved=r.published;
            if let Some(path)=r.recovery{write_new(&dir.join("ignore-recovery-path"),path.to_string_lossy().as_bytes(),&CancellationToken::new())?;}
            r.result?;
            // Nested rules can override root rules. Do not remove entries unless
            // Git confirms every selected path is effectively ignored.
            // check-ignore consumes filenames, not a pathspec; Git rejects the
            // runner's literal pathspec magic for names containing '[' or '*'.
            // Its NUL-delimited output must match every selected filename exactly.
            let output=self.runner.run(self.cwd(),&args(&["--no-literal-pathspecs","check-ignore","--no-index","-z","--stdin"]),paths_input(p.files.iter().map(|f|f.path().clone())),c)?;
            let ignored=if output.status.code()==Some(1){Vec::new()}else{output.success("ignore-tracked")?};
            if p.files.iter().any(|f|!ignored.split(|b|*b==0).any(|path|path==f.path().as_bytes())){return Err(invalid("ignore_tracked_rules","Rules were saved, but nested ignore rules override this selection. Files remain tracked and local; review .gitignore and the recovery folder."));}
            let root=self.info().worktree.as_ref().unwrap();let mut rules=open_read(&root.join(".gitignore"))?;
            crate::review::verify_preview_handle(&rules,root,&root.join(".gitignore"))?;
            if read_handle(&mut rules,c)?!=p.rules.after(){return Err(stale());}
            self.check_ignore_tracked(p,c)?;
            lock.publish(&after,c)?;index_published=true;
            if index_bytes(self,&CancellationToken::new())?!=after{return Err(stale());}Ok(())
        });
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        if let Some(log)=&self.runner.log{log.keep_local_result("ignore-tracked",Some(index_published),rules_saved,p.files.len(),result.is_ok());}
        Ok(IgnoreTrackedReport {
            result,
            recovery,
            rules_saved,
            index_published,
            status,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StashSnapshotPreview {
    repository: PathBuf,
    head: Head,
    index: Vec<u8>,
    files: Vec<FileEditPreview>,
    entries: Vec<StatusEntry>,
    previous: Option<ObjectId>,
    message: String,
    untracked: bool,
}
impl StashSnapshotPreview {
    pub fn files(&self) -> &[FileEditPreview] {
        &self.files
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn include_untracked(&self) -> bool {
        self.untracked
    }
}
pub struct StashSnapshotReport {
    pub result: Result<()>,
    pub object: Option<ObjectId>,
    pub recovery: String,
    /// None means observation failed; cancellation is not proof of non-publication.
    pub published: Option<bool>,
    pub stashes: Result<Vec<StashEntry>>,
}
impl Repository {
    fn snapshot_stash_tip(&self, c: &CancellationToken) -> Result<Option<ObjectId>> {
        let sym = self.runner.run(
            self.cwd(),
            &args(&["symbolic-ref", "-q", "refs/stash"]),
            vec![],
            c,
        )?;
        if sym.status.success() {
            return Err(invalid(
                "stash_snapshot_ref",
                "The stash ref is symbolic; restore a normal stash ref before saving.",
            ));
        }
        if sym.status.code() != Some(1) {
            sym.success("stash-snapshot")?;
        }
        let tip = self.runner.run(
            self.cwd(),
            &args(&["rev-parse", "--verify", "--quiet", "refs/stash"]),
            vec![],
            c,
        )?;
        if tip.status.code() == Some(1) {
            Ok(None)
        } else {
            Ok(Some(parse::oid(
                parse::line(&tip.success("stash-snapshot")?),
                "stash-snapshot",
            )?))
        }
    }
    fn check_stash_snapshot(&self, p: &StashSnapshotPreview, c: &CancellationToken) -> Result<()> {
        check(self, &p.repository, &p.head, &p.index, &p.files, c)?;
        if self.status(c)?.entries != p.entries || self.snapshot_stash_tip(c)? != p.previous {
            return Err(stale());
        }
        Ok(())
    }
    pub fn preview_stash_snapshot(
        &self,
        message: &str,
        include_untracked: bool,
        c: &CancellationToken,
    ) -> Result<StashSnapshotPreview> {
        self.runner.step("stash-snapshot-preview","review_snapshot_files",||{
            self.require_worktree(c)?;self.require_idle(c)?;normal_index(self,c)?;
            if message.trim().is_empty()||message.len()>4096||message.chars().any(char::is_control){return Err(invalid("stash_snapshot_message","Enter a nonblank snapshot name up to 4096 bytes, without control characters."));}
            let index=index_bytes(self,c)?;let status=self.status(c)?;
            if matches!(status.head,Head::Unborn{..}){return Err(invalid("stash_snapshot_head","Create an initial commit before saving a snapshot."));}
            if status.entries.iter().any(|e|e.kind==EntryKind::Unmerged||e.submodule.is_some()){return Err(invalid("stash_snapshot_unsupported","Resolve conflicts and save dirty submodules separately before taking a snapshot."));}
            let flags=self.runner.run(self.cwd(),&args(&["ls-files","-v","-z"]),vec![],c)?.success("stash-snapshot-preview")?;
            if flags.split(|b|*b==0).filter(|r|!r.is_empty()).any(|r|!r.starts_with(b"H ")){return Err(invalid("stash_snapshot_index","Hidden index flags or sparse checkouts require a normal checkout before snapshotting."));}
            let mut paths=BTreeSet::new();
            for e in &status.entries {if e.kind!=EntryKind::Untracked||include_untracked{paths.insert(e.path.as_bytes().to_vec());if let Some(p)=&e.original_path{paths.insert(p.as_bytes().to_vec());}}}
            if paths.is_empty(){return Err(invalid("stash_snapshot_empty","No changes match this snapshot. Include untracked files if those are the only changes."));}
            let files=capture(self,&paths.into_iter().map(RepoPath::new).collect::<Result<Vec<_>>>()?,c)?;
            let p=StashSnapshotPreview{repository:self.info().git_dir.clone(),head:status.head,index,files,entries:status.entries,previous:self.snapshot_stash_tip(c)?,message:message.into(),untracked:include_untracked};
            self.check_stash_snapshot(&p,c)?;Ok(p)
        })
    }
    pub fn stash_snapshot(
        &self,
        p: &StashSnapshotPreview,
        c: &CancellationToken,
    ) -> Result<StashSnapshotReport> {
        let _permit = self.acquire_write(c)?;
        self.check_stash_snapshot(p, c)?;
        let recovery = format!("refs/rocket/stash-snapshot/{}", unique());
        let mut object = None;
        let result = self
            .runner
            .step("stash-snapshot", "capture_and_publish_snapshot", || {
                let _lock = IndexLock::acquire(self)?;
                let _files = pin_files(self, &p.files, c)?;
                self.check_stash_snapshot(p, c)?;
                let scratch = crate::patches::PreviewScratch::new()?;
                let prepared = scratch.path.join("snapshot-index");
                write_new(&prepared, &p.index, c)?;
                let head = match &p.head {
                    Head::Branch { target, .. } | Head::Detached { target } => target,
                    Head::Unborn { .. } => return Err(stale()),
                };
                let tree = |index: &Path| -> Result<ObjectId> {
                    let v = self
                        .runner
                        .private_index(
                            self.cwd(),
                            index,
                            &args(&["--no-replace-objects", "write-tree"]),
                            vec![],
                            c,
                        )?
                        .success("stash-snapshot")?;
                    parse::oid(parse::line(&v), "stash-snapshot")
                };
                let commit =
                    |tree: &ObjectId, parents: &[&ObjectId], message: &str| -> Result<ObjectId> {
                        let mut a = args(&[
                            "--no-replace-objects",
                            "commit-tree",
                            "--no-gpg-sign",
                            tree.as_str(),
                        ]);
                        for parent in parents {
                            a.extend(args(&["-p", parent.as_str()]));
                        }
                        let v = self
                            .runner
                            .write(self.cwd(), &a, message.as_bytes().to_vec(), c)?
                            .success("stash-snapshot")?;
                        parse::oid(parse::line(&v), "stash-snapshot")
                    };
                let index_commit = commit(&tree(&prepared)?, &[head], "Rocket snapshot index")?;
                let tracked: Vec<_> = p
                    .entries
                    .iter()
                    .filter(|e| e.kind != EntryKind::Untracked && e.index != Change::Deleted)
                    .map(|e| e.path.clone())
                    .collect();
                if !tracked.is_empty() {
                    self.runner
                        .private_index(
                            self.cwd(),
                            &prepared,
                            &args(&["add", "-u", "--pathspec-from-file=-", "--pathspec-file-nul"]),
                            paths_input(tracked.into_iter()),
                            c,
                        )?
                        .success("stash-snapshot")?;
                }
                let working_tree = tree(&prepared)?;
                let untracked: Vec<_> = p
                    .entries
                    .iter()
                    .filter(|e| e.kind == EntryKind::Untracked && p.untracked)
                    .map(|e| e.path.clone())
                    .collect();
                let extra = if untracked.is_empty() {
                    None
                } else {
                    let index = scratch.path.join("untracked-index");
                    self.runner
                        .private_index(
                            self.cwd(),
                            &index,
                            &args(&["read-tree", "--empty"]),
                            vec![],
                            c,
                        )?
                        .success("stash-snapshot")?;
                    self.runner
                        .private_index(
                            self.cwd(),
                            &index,
                            &args(&["add", "--pathspec-from-file=-", "--pathspec-file-nul"]),
                            paths_input(untracked.into_iter()),
                            c,
                        )?
                        .success("stash-snapshot")?;
                    Some(commit(
                        &tree(&index)?,
                        &[],
                        "Rocket snapshot untracked files",
                    )?)
                };
                let mut parents = vec![head, &index_commit];
                if let Some(extra) = &extra {
                    parents.push(extra);
                }
                let id = commit(&working_tree, &parents, &p.message)?;
                scratch.cleanup()?;
                self.check_stash_snapshot(p, c)?;
                // Publish the stash and recovery ref in one compare-and-swap transaction.
                // A late cancellation can follow commit; observe the recovery ref below.
                let old = p
                    .previous
                    .as_ref()
                    .map(|v| v.as_str().to_string())
                    .unwrap_or_else(|| "0".repeat(id.as_str().len()));
                let mut input =
                    format!("start\ncreate {recovery} {id}\nupdate refs/stash {id} {old}\n");
                let reference = match &p.head {
                    Head::Branch { name, .. } => format!(
                        "refs/heads/{}",
                        std::str::from_utf8(name).map_err(|_| invalid(
                            "stash_snapshot_head",
                            "Use a UTF-8 branch name for this operation."
                        ))?
                    ),
                    _ => "HEAD".into(),
                };
                input.push_str(&format!("verify {reference} {head}\nprepare\ncommit\n"));
                object = Some(id);
                self.runner
                    .write(
                        self.cwd(),
                        &args(&[
                            "update-ref",
                            "--no-deref",
                            "--create-reflog",
                            "-m",
                            &p.message,
                            "--stdin",
                        ]),
                        input.into_bytes(),
                        c,
                    )?
                    .success("stash-snapshot")?;
                Ok(())
            });
        let observe = CancellationToken::new();
        let published = object.as_ref().map_or(Some(false), |id| {
            self.runner
                .observe("stash-snapshot", "observe_snapshot_ref", || {
                    let out = self.runner.run(
                        self.cwd(),
                        &args(&["rev-parse", "--verify", "--quiet", &recovery]),
                        vec![],
                        &observe,
                    )?;
                    if out.status.code() == Some(1) {
                        return Ok(false);
                    }
                    let v = out.success("stash-snapshot")?;
                    Ok(parse::line(&v) == id.as_str().as_bytes())
                })
                .ok()
        });
        let result=result.and_then(|()|if published==Some(true){Ok(())}else{Err(invalid("stash_snapshot_unknown","Snapshot publication could not be confirmed. Inspect Stashes and the recovery ref before retrying."))});
        let stashes = self
            .runner
            .observe("refresh", "stashes", || self.stashes(&observe));
        if let Some(log)=&self.runner.log{log.keep_local_result("stash-snapshot",published,false,p.files.len(),result.is_ok());}
        Ok(StashSnapshotReport {
            result,
            object,
            recovery,
            published,
            stashes,
        })
    }
}
