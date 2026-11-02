//! Executable bits are Git index metadata on Windows. Preserve existing staged blobs.
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
struct Lock {
    path: PathBuf,
    file: Option<File>,
    owned: bool,
}
impl Drop for Lock {
    fn drop(&mut self) {
        self.file.take();
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub(super) fn snapshot(repo: &Repository, c: &CancellationToken) -> Result<Vec<u8>> {
    let split = repo
        .runner
        .run(
            repo.cwd(),
            &args(&["rev-parse", "--shared-index-path"]),
            vec![],
            c,
        )?
        .success("patch-index")?;
    if !crate::parse::line(&split).is_empty() {
        return Err(Error::Unsupported("Patch mode changes require a normal index; disable split-index explicitly and review again.".into()));
    }
    match files::read(&repo.info().git_dir.join("index"), c) {
        Ok(v) => Ok(v),
        Err(e) if !repo.info().git_dir.join("index").exists() => {
            let _ = e;
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}
pub(super) fn publish(
    repo: &Repository,
    p: &PatchPreview,
    c: &CancellationToken,
    recovery: &mut Vec<PathBuf>,
) -> Result<()> {
    let root = &repo.info().git_dir;
    let _pin = crate::local_files::pin_directories(root, root)?;
    let path = root.join("index.lock");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io("acquire patch index lock", e))?;
    let mut lock = Lock {
        path,
        file: Some(file),
        owned: true,
    };
    let before = p.index.as_ref().unwrap();
    if snapshot(repo, c)? != *before {
        return failure(PatchFailure::SourceChanged);
    }
    for file in &p.files {
        repo.verify_patch_file(file, file.before() != file.after(), c)?;
    }
    let dir = root.join(format!(
        "rocket-patch-index-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&dir).map_err(|e| Error::io("reserve patch index recovery", e))?;
    recovery.push(dir.clone());
    let _dir = crate::local_files::pin_directories(root, &dir)?;
    files::write_new(&dir.join("index-before"), before, c)?;
    files::write_new(&dir.join("RECOVERY.md"),b"Original index before patch executable-mode changes. Existing staged blobs are preserved; new executable files are staged. Compare current status before restoring this copy over newer staging.\n",c)?;
    let prepared = dir.join("index-after");
    if before.is_empty() {
        repo.runner
            .private_index(
                repo.cwd(),
                &prepared,
                &args(&["read-tree", "--empty"]),
                vec![],
                c,
            )?
            .success("patch-index")?;
    } else {
        files::write_new(&prepared, before, c)?;
    }
    let mut input = Vec::new();
    for (path, mode) in &p.modes {
        let mut a = args(&["ls-files", "--stage", "-z", "--"]);
        a.push(path.argument()?);
        let entry = repo
            .runner
            .run(repo.cwd(), &a, vec![], c)?
            .success("patch-index")?;
        let id = if entry.is_empty() {
            let bytes = p
                .files
                .iter()
                .find(|f| f.path() == path)
                .and_then(|f| f.after())
                .ok_or(Error::Patch(PatchFailure::Format))?;
            let output = repo
                .runner
                .write(
                    repo.cwd(),
                    &args(&["hash-object", "-w", "--stdin"]),
                    bytes.to_vec(),
                    c,
                )?
                .success("patch-index")?;
            crate::parse::oid(crate::parse::line(&output), "patch-index")?
        } else {
            if entry.iter().filter(|b| **b == 0).count() != 1 {
                return failure(PatchFailure::Unsupported);
            }
            let fields: Vec<_> = entry.split(|b| *b == b' ' || *b == b'\t').collect();
            if fields.len() < 4 || fields[2] != b"0" {
                return failure(PatchFailure::Unsupported);
            }
            crate::parse::oid(fields[1], "patch-index")?
        };
        input.extend(format!("{mode:o} {id}\t").as_bytes());
        input.extend(path.as_bytes());
        input.push(0);
    }
    repo.runner
        .private_index(
            repo.cwd(),
            &prepared,
            &args(&["update-index", "-z", "--index-info"]),
            input,
            c,
        )?
        .success("patch-index")?;
    let after = files::read(&prepared, c)?;
    for file in &p.files {
        repo.verify_patch_file(file, file.before() != file.after(), c)?;
    }
    if snapshot(repo, c)? != *before {
        return failure(PatchFailure::SourceChanged);
    }
    let file = lock.file.as_mut().unwrap();
    file.write_all(&after)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io("flush patch index", e))?;
    c.check()?;
    lock.file.take();
    crate::local_files::move_path(&lock.path, &root.join("index"), true)?;
    lock.owned = false;
    if files::read(&root.join("index"), &CancellationToken::new())? != after {
        return failure(PatchFailure::SourceChanged);
    }
    Ok(())
}
