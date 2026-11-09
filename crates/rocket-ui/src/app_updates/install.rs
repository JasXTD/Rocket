//! Replace a verified packaged directory after the old process exits. Keep its backup.
use super::package::*;
use rocket_git::{CancellationToken, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub struct Handoff {
    pub job: PathBuf,
    approved: bool,
}
impl Handoff {
    pub fn approve(&mut self) -> Result<()> {
        if !self.approved {
            #[cfg(unix)]
            rocket_git::local_storage::check_private(&self.job).map_err(io)?;
            write_json(&self.job.join("approved.json"), &json!({"install":true}))?;
            self.approved = true;
        }
        Ok(())
    }
}
impl Drop for Handoff {
    fn drop(&mut self) {
        if !self.approved {
            cancel(self);
        }
    }
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| error("Could not encode update receipt."))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    std::io::Write::write_all(&mut file, &bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    // Its own process group, as Windows starts it without a console: Ctrl+C in the terminal
    // Rocket was started from, or closing that terminal, must not stop the helper in the
    // middle of a swap. It isn't one of the shell's jobs, so the shell never hangs it up.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}
fn move_directory(from: &Path, to: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    { rocket_git::app_update::macos::move_directory(from, to) }
    #[cfg(not(target_os = "macos"))]
    { fs::rename(from, to).map_err(io) }
}
fn same_parent(path: &Path, target: &Path, prefix: &str) -> Result<()> {
    if path.parent() != target.parent()
        || !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        || path == target
    {
        return Err(error(
            "Update transaction directory is outside the installation parent.",
        ));
    }
    plain(path)
}
fn file_map(value: &Value) -> Result<BTreeMap<String, String>> {
    let object = value
        .as_object()
        .filter(|o| o.len() <= 20000)
        .ok_or_else(|| error("Invalid update file map."))?;
    let mut map = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (name, v) in object {
        let hash = v
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| error("Invalid update file hash."))?;
        if !relative(name) || !names.insert(name.to_lowercase()) {
            return Err(error("Unsafe update file path."));
        }
        map.insert(name.clone(), hash.to_ascii_lowercase());
    }
    for name in BINARIES {
        if !map.contains_key(name) {
            return Err(error("Incomplete update file map."));
        }
    }
    Ok(map)
}
pub fn handoff(
    prepared: &Prepared,
    target: &Path,
    helper: &Path,
    repositories: &[PathBuf],
    cancel: &CancellationToken,
) -> Result<Handoff> {
    cancel.check()?;
    rocket_git::ensure_unprivileged_process()?;
    if !is_packaged(target) {
        return Err(error(
            "This is a development build. In-place updates require a packaged Rocket installation; the verified download is kept.",
        ));
    }
    plain(target)?;
    plain(helper)?;
    plain(&prepared.job)?;
    #[cfg(unix)]
    rocket_git::local_storage::check_private(&prepared.job).map_err(io)?;
    let old = installed(target, cancel)?;
    if manifest(&prepared.job.join("bundle"), prepared.version, cancel)? != prepared.files {
        return Err(error("Prepared update changed. Download it again."));
    }
    let parent = target
        .parent()
        .ok_or_else(|| error("Installation has no parent directory."))?;
    let suffix = prepared
        .job
        .file_name()
        .ok_or_else(|| error("Invalid update job directory."))?
        .to_string_lossy();
    let extension = if cfg!(target_os = "macos") { ".app" } else { "" };
    let stage = parent.join(format!(".rocket-next-{suffix}{extension}"));
    let backup = parent.join(format!(".rocket-previous-{suffix}{extension}"));
    let lock = parent.join(format!(
        ".{}.rocket-update-lock",
        target
            .file_name()
            .ok_or_else(|| error("Install Rocket in its own folder, not a drive root."))?
            .to_string_lossy()
    ));
    for (path, prefix) in [(&stage, ".rocket-next-"), (&backup, ".rocket-previous-")] {
        same_parent(path, target, prefix)?;
        if path.exists() {
            return Err(error(
                "An update transaction directory already exists. Existing files are preserved.",
            ));
        }
    }
    plain(&lock)?;
    let mut lock_file=fs::OpenOptions::new().write(true).create_new(true).open(&lock).map_err(|_|error("Another update is active or needs recovery. Check the update transaction beside this installation."))?;
    std::io::Write::write_all(&mut lock_file, prepared.job.to_string_lossy().as_bytes())
        .map_err(io)?;
    drop(lock_file);
    let result = (|| {
        copy_tree(
            &prepared_root(&prepared.job),
            &stage,
            &prepared.files,
            cancel,
        )?;
        #[cfg(target_os = "macos")]
        rocket_git::app_update::macos::verify_app(&stage, Some(prepared.version), cancel)?;
        let worker = prepared.job.join(if cfg!(windows) {
            "update-worker.exe"
        } else {
            "update-worker"
        });
        let helper_hash = digest(helper, cancel)?;
        #[cfg(target_os = "macos")]
        if old.get(ENTRYPOINT) != Some(&helper_hash) {
            return Err(error("The installed update helper changed. Nothing was replaced."));
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&worker)
            .map_err(io)?;
        fs::copy(helper, &worker).map_err(io)?;
        if digest(&worker, cancel)? != helper_hash {
            return Err(error("Update helper changed while copying."));
        }
        let plan = prepared.job.join("install.json");
        write_json(
            &plan,
            &json!({"schema":1,"target":target,"stage":stage,"backup":backup,"lock":lock,"old":old,"new":prepared.files,"parent":std::process::id(),"version":prepared.version.text(),"repositories":repositories}),
        )?;
        cancel.check()?;
        command(&worker)
            .arg("--rocket-apply-update")
            .arg(&plan)
            .spawn()
            .map_err(io)?;
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Err(e) = cancel.check() {
                let _ = fs::write(prepared.job.join("cancel"), b"cancel");
                return Err(e);
            }
            if prepared.job.join("ready").is_file() {
                return Ok(Handoff {
                    job: prepared.job.clone(),
                    approved: false,
                });
            }
            if prepared.job.join("result.json").exists() || Instant::now() > until {
                let _ = fs::write(prepared.job.join("cancel"), b"cancel");
                return Err(error(
                    "The update helper did not become ready. Rocket stayed open.",
                ));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&lock);
    }
    result
}
pub fn cancel(handoff: &Handoff) {
    #[cfg(unix)]
    if rocket_git::local_storage::check_private(&handoff.job).is_err() {
        return;
    }
    let _ = fs::write(handoff.job.join("cancel"), b"cancel");
}

/// Invoked only by Rocket's explicit installer mode. Receipts are local data.
pub fn apply(path: &Path) -> Result<()> {
    apply_verified(path, |app, version, cancel| {
        #[cfg(target_os = "macos")]
        {
            rocket_git::app_update::macos::verify_app(app, version, cancel)?;
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, version, cancel);
        Ok(())
    })
}
fn apply_verified(path: &Path, verify: impl Fn(&Path, Option<rocket_git::app_update::UpdateVersion>, &CancellationToken) -> Result<()>) -> Result<()> {
    rocket_git::ensure_unprivileged_process()?;
    plain(path)?;
    if path.file_name().is_none_or(|p| p != "install.json") {
        return Err(error("Invalid update plan filename."));
    }
    let job = path
        .parent()
        .ok_or_else(|| error("Update plan directory is missing."))?;
    #[cfg(unix)]
    rocket_git::local_storage::check_private(job).map_err(io)?;
    let plan = json_file(path)?;
    if plan["schema"] != 1 {
        return Err(error("Unknown update plan schema."));
    }
    let target = PathBuf::from(
        plan["target"]
            .as_str()
            .ok_or_else(|| error("Missing installation path."))?,
    );
    let stage = PathBuf::from(
        plan["stage"]
            .as_str()
            .ok_or_else(|| error("Missing prepared path."))?,
    );
    let backup = PathBuf::from(
        plan["backup"]
            .as_str()
            .ok_or_else(|| error("Missing backup path."))?,
    );
    let lock = PathBuf::from(
        plan["lock"]
            .as_str()
            .ok_or_else(|| error("Missing update lock."))?,
    );
    plain(&target)?;
    same_parent(&stage, &target, ".rocket-next-")?;
    same_parent(&backup, &target, ".rocket-previous-")?;
    if target.parent() != lock.parent()
        || lock
            .file_name()
            .is_none_or(|n| !n.to_string_lossy().ends_with(".rocket-update-lock"))
    {
        return Err(error("Invalid update lock location."));
    }
    if read(&lock, 32768)? != job.to_string_lossy().as_bytes() {
        return Err(error("Update lock belongs to a different transaction."));
    }
    if target.starts_with(job)
        || stage.starts_with(job)
        || backup.starts_with(job)
        || job.starts_with(&target)
    {
        return Err(error("Update helper must stay outside the installation."));
    }
    let old = file_map(&plan["old"])?;
    let new = file_map(&plan["new"])?;
    let parent = plan["parent"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| error("Missing update parent process."))?;
    let repos = plan["repositories"]
        .as_array()
        .filter(|a| a.len() <= 128)
        .ok_or_else(|| error("Invalid restart repositories."))?;
    let repos: Vec<PathBuf> = repos
        .iter()
        .map(|v| {
            v.as_str()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .ok_or_else(|| error("Invalid restart path."))
        })
        .collect::<Result<_>>()?;
    let token = CancellationToken::new();
    let mut moved_old = false;
    let mut installed_new = false;
    let mut rollback_ok = true;
    let mut parent_exited = false;
    let success_receipt = json!({"schema":1,"code":"installed","version":plan["version"],"backup":backup,"message":null});
    let receipt_path = job.join("result.json");
    let mut receipt_written = false;
    let result = (|| {
        // Open the live parent handle before announcing readiness. A reused PID
        // cannot authorize installation, and a cancelled close never touches files.
        rocket_git::app_update::wait_for_update_parent(
            parent,
            &job.join("ready"),
            &job.join("cancel"),
        )?;
        parent_exited = true;
        if job.join("cancel").exists()
            || !job.join("approved.json").is_file()
            || json_file(&job.join("approved.json"))?["install"] != true
        {
            return Err(rocket_git::Error::Cancelled);
        }
        plain(&target)?;
        plain(&stage)?;
        plain(&backup)?;
        if backup.exists()
            || inventory(&target, &token)? != old
            || inventory(&stage, &token)? != new
        {
            return Err(error(
                "Installation or prepared files changed. Nothing was replaced.",
            ));
        }
        let version = plan["version"].as_str().and_then(rocket_git::app_update::UpdateVersion::parse)
            .ok_or_else(|| error("Invalid update version."))?;
        verify(&target, None, &token)?;
        verify(&stage, Some(version), &token)?;
        // Cancellation is accepted until the first rename. Once replacement starts,
        // finish replacement or rollback so the app is never left half-installed.
        if job.join("cancel").exists() { return Err(rocket_git::Error::Cancelled); }
        write_json(
            &job.join("transaction.json"),
            &json!({"phase":"ready_to_swap","target":target,"stage":stage,"backup":backup}),
        )?;
        move_directory(&target, &backup)?;
        moved_old = true;
        move_directory(&stage, &target)?;
        installed_new = true;
        // Keep the complete old directory; never delete user files to recover.
        let executable = target.join(ENTRYPOINT);
        if digest(&executable, &token)? != new[ENTRYPOINT] {
            return Err(error("Installed executable changed before restart."));
        }
        verify(&target, Some(version), &token)?;
        // Publish the receipt before the new process starts reading it.
        write_json(&receipt_path, &success_receipt)?;
        receipt_written = true;
        command(&executable)
            .args(&repos)
            .arg("--rocket-update-result")
            .arg(job)
            .spawn()
            .map_err(io)?;
        Ok(())
    })();
    if result.is_err() && moved_old {
        // If restart failed, move the failed new directory aside and put the old
        // one back. Both sets of files survive; no recursive deletion is used.
        if installed_new && move_directory(&target, &stage).is_err() {
            rollback_ok = false;
        }
        if rollback_ok && move_directory(&backup, &target).is_err() {
            rollback_ok = false;
        }
    }
    let code = if result.is_ok() {
        "installed"
    } else if matches!(result, Err(rocket_git::Error::Cancelled)) {
        "cancelled"
    } else if !rollback_ok {
        "recovery_required"
    } else if moved_old {
        "rolled_back"
    } else {
        "unchanged"
    };
    if result.is_err() {
        if receipt_written {
            if json_file(&receipt_path)? != success_receipt {
                return Err(error(
                    "Update receipt changed externally. Existing files were retained.",
                ));
            }
            fs::remove_file(&receipt_path).map_err(io)?;
        }
        write_json(
            &receipt_path,
            &json!({"schema":1,"code":code,"version":plan["version"],"backup":backup,"message":result.as_ref().err().map(ToString::to_string)}),
        )?;
    }
    if rollback_ok {
        let _ = fs::remove_file(&lock);
    }
    // A failed replacement reopens the old program only after full rollback.
    if result.is_err()
        && parent_exited
        && rollback_ok
        && digest(&target.join(ENTRYPOINT), &token).is_ok_and(|sha| sha == old[ENTRYPOINT])
        && (!cfg!(target_os = "macos") || (inventory(&target, &token).is_ok_and(|files| files == old) && verify(&target, None, &token).is_ok()))
    {
        let _ = command(&target.join(ENTRYPOINT))
            .args(&repos)
            .arg("--rocket-update-result")
            .arg(job)
            .spawn();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only linked into the Rust test executable. Production apply() always uses
    // Developer ID + notarization; no environment/CLI switch can disable that.
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_transaction_child() {
        let Some(plan) = std::env::var_os("ROCKET_MAC_UPDATE_TEST_PLAN") else { return };
        let result = apply_verified(Path::new(&plan), |app, _, cancel| {
                cancel.check()?;
                let status = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict", "--test-requirement", "=identifier \"io.github.jasxtd.rocket\""]).arg(app).status().map_err(io)?;
                if !status.success() { return Err(error("Disposable app signature is invalid")); }
            Ok(())
        });
        // Outcome itself is asserted independently by the external driver from
        // receipts, file inventories and actual restarted probe executables.
        println!("test helper outcome: {}", if result.is_ok() { "installed" } else { "refused or recovered" });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unsafe_jobs_and_ancestors_are_refused_before_handoff_or_plan_reads() {
        use std::os::unix::fs::PermissionsExt;
        let root = unique(&std::env::temp_dir()).unwrap();
        let target = root.join("installation");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("SHA256SUMS.txt"), b"untouched installation").unwrap();
        let helper = root.join("helper");
        fs::write(&helper, b"never executed").unwrap();
        let parent = root.join("updates");
        rocket_git::local_storage::create_private(&parent).unwrap();
        let job = unique(&parent).unwrap();
        let prepared = Prepared { job: job.clone(), version: rocket_git::app_update::UpdateVersion(9, 0, 0), files: BTreeMap::new() };
        let plan = job.join("install.json");
        fs::write(&plan, b"not read before the permission check").unwrap();
        let chmod = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        for unsafe_folder in [&job, &parent] {
            chmod(unsafe_folder, 0o777);
            let refused = handoff(&prepared, &target, &helper, &[], &CancellationToken::new())
                .err().expect("unsafe job must not launch a helper");
            assert!(matches!(refused, rocket_git::Error::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied));
            let refused = apply(&plan).unwrap_err();
            assert!(matches!(refused, rocket_git::Error::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied));
            {
                let mut handoff = Handoff { job: job.clone(), approved: false };
                assert!(handoff.approve().is_err());
            }
            assert!(!job.join("update-worker").exists());
            assert!(!job.join("approved.json").exists());
            assert!(!job.join("cancel").exists());
            assert_eq!(fs::read(target.join("SHA256SUMS.txt")).unwrap(), b"untouched installation");
            chmod(unsafe_folder, 0o700);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discarded_handoff_cancels_without_approving_installation() {
        let root = unique(&std::env::temp_dir()).unwrap();
        {
            let _handoff = Handoff {
                job: root.clone(),
                approved: false,
            };
        }
        assert!(root.join("cancel").is_file());
        assert!(!root.join("approved.json").exists());
        fs::remove_file(root.join("cancel")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
