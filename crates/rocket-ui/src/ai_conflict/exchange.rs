//! Local, user-protected mailbox for explicitly shared conflict snapshots.
//! It grants no repository access; only the native editor can apply a proposal.
use super::model::{CONTEXT_LIMIT, PROPOSAL_LIMIT, invalid, review};
use rocket_git::{CancellationToken, Result, protect_hosting_secret};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = CONTEXT_LIMIT + 8192;
const TTL: u64 = 15 * 60;
static NEXT: AtomicU64 = AtomicU64::new(0);
fn protected(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    protect_hosting_secret(bytes,encrypt).map_err(|_|invalid(if cfg!(windows) {"ai_exchange_data: Cannot protect or decrypt this conflict snapshot for the current Windows user. Nothing was applied."} else if cfg!(target_os="macos") {"ai_exchange_data: Cannot protect or decrypt this conflict snapshot. Unlock the default macOS Keychain and review Rocket's access, then retry. Nothing was applied."} else {"ai_exchange_data: Cannot protect or decrypt this conflict snapshot with the key in your desktop keyring. Nothing was applied."}))
}
struct Shared {
    id: String,
    expires: u64,
    payload: Value,
}
fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| invalid("ai_exchange_clock: Check the system clock."))
}
pub fn request_id() -> Result<String> {
    Ok(format!(
        "{:x}-{:x}-{:x}",
        now()?,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
fn regular(path: &Path, directory: bool) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)
        .map_err(|_| invalid("ai_exchange_files: Cannot read the local proposal mailbox."))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(invalid(
                "ai_exchange_link: Linked mailbox files are refused.",
            ));
        }
    }
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(invalid(
            "ai_exchange_files: Expected an ordinary mailbox file or directory.",
        ));
    }
    Ok(m)
}
fn root(profile: &Path, create: bool) -> Result<PathBuf> {
    if !profile.is_absolute() {
        return Err(invalid(
            "ai_exchange_profile: MCP profile must be an absolute local path.",
        ));
    }
    for p in profile.ancestors() {
        regular(p, true)?;
    }
    let root = profile.join("ai-conflicts");
    if create {
        match fs::create_dir(&root) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => {
                return Err(invalid(
                    "ai_exchange_files: Cannot create the proposal mailbox.",
                ));
            }
        }
    }
    if root
        .try_exists()
        .map_err(|_| invalid("ai_exchange_files: Cannot inspect the mailbox."))?
    {
        regular(&root, true)?;
    }
    Ok(root)
}
fn read(path: &Path, max: usize) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => (),
    }
    if regular(path, false)?.len() > max as u64 {
        return Err(invalid(
            "ai_exchange_limit: Oversized mailbox file refused.",
        ));
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .and_then(|f| f.take((max + 1) as u64).read_to_end(&mut bytes))
        .map_err(|_| invalid("ai_exchange_files: Cannot read the mailbox file."))?;
    if bytes.len() > max {
        return Err(invalid(
            "ai_exchange_limit: Oversized mailbox file refused.",
        ));
    }
    Ok(Some(bytes))
}
fn publish(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    regular(root, true)?;
    let temp = root.join(format!("{}.tmp", request_id()?));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| invalid("ai_exchange_files: Cannot prepare a mailbox file."))?;
        f.write_all(bytes)
            .and_then(|()| f.sync_all())
            .map_err(|_| invalid("ai_exchange_files: Cannot flush a mailbox file."))?;
        drop(f);
        // Atomic create-only publication: never replace an existing response.
        fs::hard_link(&temp,root.join(name)).map_err(|_|invalid("ai_exchange_exists: This request already has a response or the mailbox cannot publish it."))?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    result
}
fn prune(root: &Path) -> Result<()> {
    let time = now()?;
    for entry in fs::read_dir(root)
        .map_err(|_| invalid("ai_exchange_files: Cannot inspect expired requests."))?
        .take(512)
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".closed") else {
            continue;
        };
        let mut parts = id.split('-');
        let stamp = parts.next().and_then(|s| u64::from_str_radix(s, 16).ok());
        if !safe_id(id)
            || parts.count() != 2
            || stamp.is_none_or(|s| s.saturating_add(TTL) >= time)
            || read(&entry.path(), 0)?.is_none()
        {
            continue;
        }
        // Only expired tombstones produced by Lease permit companion cleanup.
        for suffix in ["reply", "request", "closed"] {
            let p = root.join(format!("{id}.{suffix}"));
            if regular(&p, false).is_ok() {
                fs::remove_file(p).map_err(|_| {
                    invalid("ai_exchange_files: Cannot remove expired owned request.")
                })?;
            }
        }
    }
    Ok(())
}
fn request(profile: &Path, id: &str) -> Result<(PathBuf, Shared)> {
    if !safe_id(id) {
        return Err(invalid("ai_exchange_id: Invalid request ID."));
    }
    let root = root(profile, false)?;
    if root.join(format!("{id}.closed")).exists() {
        return Err(invalid(
            "ai_exchange_closed: The user cancelled this request. Prepare a new one in Rocket.",
        ));
    }
    let bytes = read(&root.join(format!("{id}.request")), LIMIT)?.ok_or_else(|| {
        invalid("ai_exchange_missing: No shared request with this ID. Share it from Rocket first.")
    })?;
    let mut plain = protected(&bytes, false)?;
    let result = serde_json::from_slice::<Value>(&plain);
    plain.fill(0);
    let value = result.map_err(|_| invalid("ai_exchange_data: Invalid protected request."))?;
    let data = Shared {
        id: value["id"].as_str().unwrap_or("").into(),
        expires: value["expires"].as_u64().unwrap_or(0),
        payload: value["payload"].clone(),
    };
    let time = now()?;
    if data.id != id
        || data.expires <= time
        || data.expires > time + TTL
        || data.payload.to_string().len() > CONTEXT_LIMIT
    {
        return Err(invalid(
            "ai_exchange_expired: This request expired or is invalid. Share a new snapshot from Rocket.",
        ));
    }
    Ok((root, data))
}
pub fn list(profile: &Path) -> Result<Value> {
    let root = root(profile, false)?;
    if !root.exists() {
        return Ok(json!({"requests":[]}));
    }
    let entries = fs::read_dir(&root)
        .map_err(|_| invalid("ai_exchange_files: Cannot list shared requests."))?;
    let mut result = vec![];
    for entry in entries.take(256).flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".request")
            && let Ok((root, data)) = request(profile, id)
        {
            result.push(json!({"request_id":id,"path":data.payload["path"],"expires_unix":data.expires,"proposal_received":root.join(format!("{id}.reply")).exists()}));
        }
    }
    Ok(json!({"requests":result}))
}
pub fn get(profile: &Path, id: &str) -> Result<Value> {
    let (_, r) = request(profile, id)?;
    Ok(json!({"request_id":id,"expires_unix":r.expires,"context":r.payload}))
}
pub fn propose(profile: &Path, id: &str, proposal: &str) -> Result<Value> {
    let (root, data) = request(profile, id)?;
    review(
        data.payload["draft"]
            .as_str()
            .ok_or_else(|| invalid("ai_exchange_data: Missing draft."))?,
        proposal,
    )?;
    let bytes = protected(proposal.as_bytes(), true)?;
    request(profile, id)?;
    publish(&root, &format!("{id}.reply"), &bytes)?;
    Ok(
        json!({"request_id":id,"status":"proposal_submitted","applied":false,"message":"Rocket must review this result. The editor, files and index have not been changed by this tool."}),
    )
}
/// The guard lives on the background worker, so cleanup never blocks rendering.
struct Lease {
    root: PathBuf,
    id: String,
    closed: bool,
}
impl Lease {
    fn close(&mut self) -> Result<()> {
        regular(&self.root, true)?;
        // Mark closed first: an in-flight responder can never resurrect a request.
        let closed = self.root.join(format!("{}.closed", self.id));
        let marked = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&closed)
            .is_ok()
            || read(&closed, 0).is_ok_and(|b| b.is_some());
        let mut removed = true;
        for suffix in ["request", "reply"] {
            let p = self.root.join(format!("{}.{suffix}", self.id));
            if regular(&p, false).is_ok() {
                removed &= fs::remove_file(p).is_ok();
            } else if p.exists() {
                removed = false;
            }
        }
        self.closed = marked && removed;
        if !self.closed {
            return Err(invalid(
                "ai_exchange_cleanup: Could not finish revoking or removing the local snapshot. Stop Rocket's MCP server if access must end now; the request expires after 15 minutes. No proposal was applied.",
            ));
        }
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close();
        }
        // The non-sensitive closed tombstone expires with the profile. It also
        // prevents a late writer from treating this ID as a new request.
    }
}
pub fn wait(
    profile: &Path,
    id: &str,
    payload: Value,
    cancel: &CancellationToken,
    shared: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    if !safe_id(id) || payload.to_string().len() > CONTEXT_LIMIT {
        return Err(invalid("ai_exchange_limit: Invalid or oversized context."));
    }
    cancel.check()?;
    let root = root(profile, true)?;
    prune(&root)?;
    if fs::read_dir(&root)
        .map_err(|_| invalid("ai_exchange_files: Cannot inspect mailbox."))?
        .count()
        > 256
    {
        return Err(invalid(
            "ai_exchange_limit: Mailbox is full. Remove expired requests while Rocket MCP is stopped.",
        ));
    }
    let mut lease = Lease {
        root: root.clone(),
        id: id.to_owned(),
        closed: false,
    };
    let data = Shared {
        id: id.to_owned(),
        expires: now()? + TTL,
        payload,
    };
    let mut plain =
        serde_json::to_vec(&json!({"id":data.id,"expires":data.expires,"payload":data.payload}))
            .map_err(|_| invalid("ai_exchange_data: Cannot encode shared context."))?;
    let encrypted = protected(&plain, true);
    plain.fill(0);
    publish(&root, &format!("{id}.request"), &encrypted?)?;
    shared.store(true, Ordering::Release);
    let result = (|| {
        loop {
            cancel.check()?;
            if now()? >= data.expires {
                return Err(invalid(
                    "ai_exchange_expired: Waiting for the agent timed out after 15 minutes. Your editor text is retained.",
                ));
            }
            if let Some(bytes) = read(&root.join(format!("{id}.reply")), PROPOSAL_LIMIT + 4096)? {
                let mut plain = protected(&bytes, false)?;
                let text = String::from_utf8(plain.clone())
                    .map_err(|_| invalid("ai_proposal_schema: Proposal must be UTF-8."));
                plain.fill(0);
                let text = text?;
                review(data.payload["draft"].as_str().unwrap_or(""), &text)?;
                cancel.check()?;
                return Ok(text);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    lease.close()?;
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn profile() -> PathBuf {
        let p = std::env::temp_dir().join(format!("rocket-ai-{}", request_id().unwrap()));
        fs::create_dir(&p).unwrap();
        p
    }
    fn context() -> Value {
        json!({"path":"界.rs","draft":"before\n<<<<<<< ours\none\n=======\ntwo\n>>>>>>> theirs\nafter\n"})
    }
    fn proposal() -> String {
        json!({"resolutions":[{"id":0,"replacement":"one\ntwo\n","explanation":"Both","confidence":"medium"}]}).to_string()
    }
    fn ready(shared: &std::sync::atomic::AtomicBool) {
        let start = std::time::Instant::now();
        while !shared.load(Ordering::Acquire) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn cleanup(p: &Path) {
        let root = p.join("ai-conflicts");
        for e in fs::read_dir(&root).unwrap() {
            fs::remove_file(e.unwrap().path()).unwrap();
        }
        fs::remove_dir(root).unwrap();
        fs::remove_dir(p).unwrap();
    }
    #[test]
    fn shared_snapshot_proposal_and_revocation() {
        let p = profile();
        let id = request_id().unwrap();
        let cancel = CancellationToken::new();
        let shared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = {
            let p = p.clone();
            let id = id.clone();
            let shared = shared.clone();
            std::thread::spawn(move || wait(&p, &id, context(), &cancel, &shared))
        };
        ready(&shared);
        assert_eq!(get(&p, &id).unwrap()["context"]["path"], "界.rs");
        assert_eq!(list(&p).unwrap()["requests"].as_array().unwrap().len(), 1);
        assert!(propose(&p, &id, "{}").is_err());
        assert!(get(&p, "../bad").is_err());
        let bytes = fs::read(p.join(format!("ai-conflicts/{id}.request"))).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("before"));
        assert_eq!(propose(&p, &id, &proposal()).unwrap()["applied"], false);
        assert_eq!(task.join().unwrap().unwrap(), proposal());
        assert!(get(&p, &id).is_err());
        assert!(propose(&p, &id, &proposal()).is_err());
        cleanup(&p);
    }
    #[test]
    fn cancellation_removes_shared_private_content() {
        let p = profile();
        let id = request_id().unwrap();
        let cancel = CancellationToken::new();
        let shared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = {
            let p = p.clone();
            let id = id.clone();
            let shared = shared.clone();
            let cancel = cancel.clone();
            std::thread::spawn(move || wait(&p, &id, context(), &cancel, &shared))
        };
        ready(&shared);
        cancel.cancel();
        assert!(matches!(
            task.join().unwrap(),
            Err(rocket_git::Error::Cancelled)
        ));
        assert!(!p.join(format!("ai-conflicts/{id}.request")).exists());
        assert!(get(&p, &id).is_err());
        cleanup(&p);
    }
    #[test]
    fn cancellation_reports_cleanup_failure_and_preserves_foreign_entries() {
        let p = profile();
        let id = request_id().unwrap();
        let cancel = CancellationToken::new();
        let shared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = {
            let p = p.clone();
            let id = id.clone();
            let shared = shared.clone();
            let cancel = cancel.clone();
            std::thread::spawn(move || wait(&p, &id, context(), &cancel, &shared))
        };
        ready(&shared);
        let foreign = p.join(format!("ai-conflicts/{id}.closed"));
        fs::create_dir(&foreign).unwrap();
        cancel.cancel();
        let error = task.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("ai_exchange_cleanup"));
        assert!(foreign.is_dir());
        assert!(get(&p, &id).is_err());
        fs::remove_dir(foreign).unwrap();
        cleanup(&p);
    }
}
