//! Protected PR drafts and write-ahead recovery. No credentials or executable
//! previews enter this file; disk I/O and credential protection run off the UI thread.
use super::*;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};
const FILE: &str = "hosting-drafts.dpapi";
const LIMIT: usize = 2 * 1024 * 1024;
const RECORDS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub provider: ForgeProvider,
    pub server: String,
    pub api: String,
    pub account: String,
    pub repository: String,
    pub number: u64,
}
impl Scope {
    pub fn new(
        session: &ForgeSession,
        repo: &ForgeRepository,
        number: u64,
    ) -> rocket_git::Result<Self> {
        Ok(Self { provider: repo.config().provider(), server: repo.config().server().into(),
            api: repo.config().azure_api_version().into(), account: session.account_identity().ok_or_else(||invalid_draft("The provider did not return a stable account identity. Reconnect before saving or submitting PR drafts."))?.into(),
            repository: repo.path().into(), number })
    }
    fn config(&self, fixture: Option<u16>) -> rocket_git::Result<ForgeConfig> {
        let mut config = ForgeConfig::new(self.provider, &self.server)?;
        if self.provider == ForgeProvider::AzureServer {
            config = config.with_azure_api_version(&self.api)?;
        }
        if let Some(port) = fixture {
            config = config.with_fixture(port)?;
        }
        Ok(config)
    }
    pub fn matches(&self, session: &ForgeSession) -> bool {
        self.provider == session.config().provider()
            && self.server == session.config().server()
            && self.api == session.config().azure_api_version()
            && Some(self.account.as_str()) == session.account_identity()
    }
    fn json(&self) -> Value {
        json!({"provider":self.provider.key(),"server":self.server,"api":self.api,
        "account":self.account,"repository":self.repository,"number":self.number})
    }
}
#[derive(Clone, PartialEq)]
pub struct Record {
    pub scope: Scope,
    pub data: Value,
}
pub struct Loaded {
    records: Vec<Record>,
    snapshot: Option<Vec<u8>>,
}
#[derive(Default)]
pub struct State {
    pub loaded: bool,
    pub error: Option<String>,
    pub records: Vec<Record>,
    pub active: Option<Scope>,
    pub restored: Vec<ForgeInlineDraft>,
    pub restored_selection: Option<Value>,
    pub pending: Option<String>,
    pub write_ahead: Option<String>,
    pub ready_action: bool,
    pub delete_scope: Option<Scope>,
    written: Vec<Record>,
    snapshot: Option<Vec<u8>>,
    task: Option<WriteTask<Loaded>>,
    scanned: Option<Instant>,
    // The active editor can contain text rejected by the bounded record store.
    uncaptured: bool,
}
fn refused() -> Error {
    Error::InvalidInput(if cfg!(windows) {
        "hosting_drafts_preserved: saved PR drafts changed externally, are invalid, or could not be protected for this Windows user. Existing data was preserved; no new remote write was started."
    } else if cfg!(target_os = "macos") {
        "hosting_drafts_preserved: saved PR drafts changed externally, are invalid, or Keychain access was denied. Unlock the default macOS Keychain and review Rocket's access, then retry. Existing data was preserved; no new remote write was started."
    } else {
        "hosting_drafts_preserved: saved PR drafts changed externally, are invalid, or could not be protected with the key in your desktop keyring. Existing data was preserved; no new remote write was started."
    }.into())
}
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}
fn read(directory: &Path) -> rocket_git::Result<Option<Vec<u8>>> {
    let path = directory.join(FILE);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("read protected PR drafts", e)),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(refused());
        }
    }
    if !meta.is_file() || meta.len() > LIMIT as u64 {
        return Err(refused());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(LIMIT as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| io("read protected PR drafts", e))?;
    if bytes.len() > LIMIT {
        return Err(refused());
    }
    Ok(Some(bytes))
}
fn field(v: &Value, key: &str, max: usize) -> rocket_git::Result<String> {
    v[key]
        .as_str()
        .filter(|s| s.len() <= max && !s.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(refused)
}
fn decode(v: Value) -> rocket_git::Result<Vec<Record>> {
    if v["version"] != 1 {
        return Err(refused());
    }
    let rows = v["records"]
        .as_array()
        .filter(|r| r.len() <= RECORDS)
        .ok_or_else(refused)?;
    let mut records = Vec::new();
    for row in rows {
        let s = &row["scope"];
        let scope = Scope {
            provider: ForgeProvider::from_key(&field(s, "provider", 32)?).ok_or_else(refused)?,
            server: field(s, "server", 512)?,
            api: field(s, "api", 8)?,
            account: field(s, "account", 512)?,
            repository: field(s, "repository", 4096)?,
            number: s["number"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(refused)?,
        };
        if scope.account.is_empty() || scope.account.chars().any(char::is_control) {
            return Err(refused());
        }
        ForgeRepository::new(scope.config(None)?, &scope.repository).map_err(|_| refused())?;
        let data = &row["data"];
        if !data.is_object() || serde_json::to_vec(data).map_err(|_| refused())?.len() > 256 * 1024
        {
            return Err(refused());
        }
        field(data, "body", 65536)?;
        field(data, "inline_body", 8192)?;
        if let Some(thread) = data.get("thread") {
            field(thread, "body", 8192)?;
            field(thread, "selected", 256)?;
            if thread["unknown"].as_bool().is_none()
                || !(thread["action"].is_null()
                    || matches!(
                        thread["action"].as_str(),
                        Some("reply" | "resolve" | "unresolve")
                    ))
            {
                return Err(refused());
            }
        }
        if data["review_unknown"].as_bool().is_none() || data["merge_unknown"].as_bool().is_none() {
            return Err(refused());
        }
        if !matches!(
            data["action"].as_str(),
            Some("COMMENT" | "APPROVE" | "REQUEST_CHANGES")
        ) {
            return Err(refused());
        }
        let comments = data["comments"]
            .as_array()
            .filter(|a| a.len() <= 32)
            .ok_or_else(refused)?;
        for c in comments {
            parse_comment(c)?;
        }
        if records.iter().any(|r: &Record| r.scope == scope) {
            return Err(refused());
        }
        records.push(Record {
            scope,
            data: data.clone(),
        });
    }
    Ok(records)
}
fn encode(records: &[Record]) -> Value {
    json!({"version":1,"records":records.iter().map(|r|json!({"scope":r.scope.json(),"data":r.data})).collect::<Vec<_>>()})
}
fn load(directory: &Path) -> rocket_git::Result<Loaded> {
    let snapshot = read(directory)?;
    let records = if let Some(bytes) = &snapshot {
        let mut plain = rocket_git::protect_hosting_secret(bytes, false)?;
        let value = serde_json::from_slice(&plain);
        plain.fill(0);
        decode(value.map_err(|_| refused())?)?
    } else {
        Vec::new()
    };
    Ok(Loaded { records, snapshot })
}
fn save(
    directory: &Path,
    records: Vec<Record>,
    expected: Option<Vec<u8>>,
) -> rocket_git::Result<Loaded> {
    let value = encode(&records);
    decode(value.clone())?;
    let mut plain = serde_json::to_vec(&value).map_err(|_| refused())?;
    if plain.len() > LIMIT - 16384 {
        plain.fill(0);
        return Err(refused());
    }
    let protected = rocket_git::protect_hosting_secret(&plain, true);
    plain.fill(0);
    let encrypted = protected?;
    if read(directory)? != expected {
        return Err(refused());
    }
    let temporary = directory.join(format!(
        "pr-drafts-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| refused())?
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| io("prepare protected PR drafts", e))?;
    let result = (|| {
        file.write_all(&encrypted)
            .map_err(|e| io("write protected PR drafts", e))?;
        file.sync_all()
            .map_err(|e| io("flush protected PR drafts", e))?;
        drop(file);
        if read(directory)? != expected {
            return Err(refused());
        }
        fs::rename(&temporary, directory.join(FILE))
            .map_err(|e| io("replace protected PR drafts", e))?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temporary);
        return Err(e);
    }
    Ok(Loaded {
        records,
        snapshot: Some(encrypted),
    })
}
fn comment_json(c: &ForgeInlineDraft) -> Value {
    json!({"path":c.path,"start":c.start,"line":c.line,"side":c.side.key(),"head":c.head,"base":c.base,"body":c.body,"diff_revision":c.diff_revision})
}
fn valid_diff_revision(value: &str) -> bool {
    let sha = |s: &str| s.len()==40 && s.bytes().all(|b|b.is_ascii_hexdigit());
    let number = |s: &str| !s.is_empty() && s.len()<=10 && s.bytes().all(|b|b.is_ascii_digit()) && s.parse::<u32>().is_ok_and(|n| n<=i32::MAX as u32);
    let parts: Vec<_> = value.split(':').collect();
    match parts.as_slice() {
        [base,head,start] => sha(base) && sha(head) && sha(start),
        ["bitbucket-server",version] => number(version),
        ["azure",iteration,base,head,target] => number(iteration) && *iteration!="0" && sha(base) && sha(head) && sha(target),
        _ => false,
    }
}
fn parse_comment(v: &Value) -> rocket_git::Result<ForgeInlineDraft> {
    let side = match v["side"].as_str() {
        Some("LEFT") => ForgeDiffSide::Left,
        Some("RIGHT") => ForgeDiffSide::Right,
        _ => return Err(refused()),
    };
    let c = ForgeInlineDraft {
        path: field(v, "path", 8192)?,
        start: v["start"].as_u64().ok_or_else(refused)?,
        line: v["line"].as_u64().ok_or_else(refused)?,
        side,
        head: field(v, "head", 40)?,
        base: field(v, "base", 40)?,
        body: field(v, "body", 8192)?,
        diff_revision: match v.get("diff_revision") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if valid_diff_revision(s) => Some(s.clone()),
            _ => return Err(refused()),
        },
    };
    if c.start == 0
        || c.start > c.line
        || c.line - c.start >= 2000
        || [&c.head, &c.base]
            .iter()
            .any(|s| s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(refused());
    }
    Ok(c)
}
fn selection_json(s: &pull_review::inline::Selection) -> Value {
    json!({"path":s.path,"head":s.head,"base":s.base,"start":s.start,"line":s.line,"side":s.side.key(),"diff_revision":s.diff_revision})
}
pub fn capture(s: &mut super::State) {
    s.drafts.uncaptured = false;
    let Some(scope) = s.drafts.active.clone() else {
        return;
    };
    let in_scope = |v: &Option<(ForgeRepository, u64)>| {
        v.as_ref().is_some_and(|(repo, n)| {
            *n == scope.number
                && repo.path() == scope.repository
                && repo.config().server() == scope.server
                && repo.config().provider() == scope.provider
        })
    };
    let review_unknown =
        in_scope(&s.pull.unknown) || s.drafts.write_ahead.as_deref() == Some("pull-submit");
    let merge_unknown = in_scope(&s.pull.merge.unknown)
        || s.drafts.write_ahead.as_deref() == Some("pull-merge-submit");
    let comments = s
        .pull
        .comments
        .iter()
        .map(|c| comment_json(&c.draft()))
        .chain(s.drafts.restored.iter().map(comment_json))
        .collect::<Vec<_>>();
    let data = json!({"body":s.pull.body,"action":s.pull.action.event(),"comments":comments,"inline_body":s.pull.inline_body,
        "selection":s.pull.inline_selection.as_ref().map(selection_json).or_else(||s.drafts.restored_selection.clone()),"editing":s.pull.inline_editing,
        "review_unknown":review_unknown,"merge_unknown":merge_unknown,
        "thread":{"body":s.pull.threads.body,"selected":s.pull.threads.selected,"action":s.pull.threads.action.map(|a|a.key()),"unknown":s.pull.threads.unknown||s.drafts.write_ahead.as_deref()==Some("pull-thread-submit")}});
    let keep = s.pull.has_draft()
        || !s.drafts.restored.is_empty()
        || s.drafts.restored_selection.is_some()
        || review_unknown
        || merge_unknown
        || s.pull.threads.unknown
        || s.drafts.write_ahead.is_some();
    let found = s.drafts.records.iter().position(|r| r.scope == scope);
    if keep {
        let record = Record { scope, data };
        if let Some(i) = found {
            s.drafts.records[i] = record;
        } else if s.drafts.records.len() < RECORDS {
            s.drafts.records.push(record);
        } else {
            s.drafts.uncaptured = true;
            s.drafts.error=Some("Saved PR draft limit reached (32). Remove an unneeded draft before another remote write.".into());
        }
    } else if let Some(i) = found {
        s.drafts.records.remove(i);
    }
}
impl State {
    pub fn saving(&self) -> bool {
        self.task.is_some()
            || self.pending.is_some()
            || (self.loaded && self.error.is_none() && self.records != self.written)
    }
    fn unsaved(&self) -> bool {
        self.uncaptured || self.records != self.written
    }
    pub fn has_restored(&self) -> bool {
        !self.restored.is_empty() || self.restored_selection.is_some()
    }
    pub fn cancel_pending(&mut self) {
        if self.pending.take().is_some() {
            self.write_ahead = None;
            self.ready_action = false;
        }
    }
}
impl Rocket {
    pub(crate) fn poll_hosting_drafts(&mut self) {
        if !self.profile_storage_ready() {
            return;
        }
        if let Some(result) = self
            .forge
            .drafts
            .task
            .as_mut()
            .and_then(WriteTask::try_finish)
        {
            let d = &mut self.forge.drafts;
            d.task = None;
            match result {
                Ok(loaded) => {
                    if !d.loaded {
                        d.records = loaded.records.clone();
                    }
                    d.written = loaded.records;
                    d.snapshot = loaded.snapshot;
                    d.loaded = true;
                }
                Err(e) => {
                    d.error = Some(user_error(&e));
                    d.cancel_pending();
                }
            }
        }
        if self.quitting {
            self.forge.drafts.cancel_pending();
        }
        let scan = self.quitting
            || self.forge.drafts.pending.is_some()
            || self
                .forge
                .drafts
                .scanned
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(200));
        if scan {
            capture(&mut self.forge);
            self.forge.drafts.scanned = Some(Instant::now());
        }
        let d = &mut self.forge.drafts;
        if d.error.is_some() {
            d.cancel_pending();
            if self.quitting && d.unsaved() {
                self.quitting = false;
                self.notice = d.error.clone();
            }
            return;
        }
        if d.task.is_none() && (!d.loaded || d.records != d.written) {
            let directory = self.state_directory.clone();
            let records = d.records.clone();
            let expected = d.snapshot.clone();
            let loaded = d.loaded;
            let log = self.backend.as_ref().map(|b| b.log.clone());
            d.task = match WriteTask::spawn(move |_| {
                let run = || {
                    if loaded {
                        save(&directory, records, expected)
                    } else {
                        load(&directory)
                    }
                };
                if let Some(log) = log {
                    log.request(
                        if loaded {
                            "hosting-drafts-save"
                        } else {
                            "hosting-drafts-load"
                        },
                        |_| run(),
                    )
                } else {
                    run()
                }
            }) {
                Ok(t) => Some(t),
                Err(e) => {
                    d.error = Some(user_error(&e));
                    None
                }
            };
        }
        if d.loaded
            && d.task.is_none()
            && d.records == d.written
            && !self.quitting
            && let Some(action) = d.pending.take()
        {
            d.ready_action = true;
            self.start_pull_action(&action);
            self.forge.drafts.ready_action = false;
        }
    }
    pub(super) fn gate_pull_write(&mut self, action: &str) -> bool {
        if !matches!(
            action,
            "pull-submit" | "pull-merge-submit" | "pull-thread-submit"
        ) {
            return true;
        }
        let d = &mut self.forge.drafts;
        if !d.loaded || d.active.is_none() || d.error.is_some() {
            self.forge.error=Some(d.error.clone().unwrap_or_else(||"Wait for protected PR drafts to load and open this PR with the connected account before writing.".into()));
            return false;
        }
        if self
            .forge
            .session
            .as_ref()
            .is_none_or(|session| !d.active.as_ref().unwrap().matches(session))
        {
            self.forge.error =
                Some("Connect the account that owns this draft before submitting.".into());
            return false;
        }
        if d.ready_action {
            return true;
        }
        d.write_ahead = Some(action.into());
        d.pending = Some(action.into());
        capture(&mut self.forge);
        self.forge.notice = Some("Saving recovery record before sending…".into());
        false
    }
}
pub fn activate(
    s: &mut super::State,
    repo: &ForgeRepository,
    number: u64,
) -> rocket_git::Result<()> {
    let session = s.session.as_ref().ok_or_else(refused)?;
    if !s.drafts.loaded {
        return Err(invalid_draft("Wait for saved PR drafts to load."));
    }
    let scope = Scope::new(session, repo, number)?;
    if s.drafts.active.as_ref() == Some(&scope) {
        return Ok(());
    }
    s.drafts.active = Some(scope.clone());
    s.drafts.restored.clear();
    s.drafts.restored_selection = None;
    if let Some(record) = s.drafts.records.iter().find(|r| r.scope == scope) {
        let v = &record.data;
        s.pull.body = field(v, "body", 65536)?;
        s.pull.inline_body = field(v, "inline_body", 8192)?;
        s.pull.action = match v["action"].as_str() {
            Some("APPROVE") => ForgeReviewAction::Approve,
            Some("REQUEST_CHANGES") => ForgeReviewAction::RequestChanges,
            _ => ForgeReviewAction::Comment,
        };
        s.drafts.restored = v["comments"]
            .as_array()
            .ok_or_else(refused)?
            .iter()
            .map(parse_comment)
            .collect::<rocket_git::Result<_>>()?;
        s.drafts.restored_selection = v.get("selection").filter(|v| v.is_object()).cloned();
        s.pull.inline_editing = v["editing"].as_u64().and_then(|n| usize::try_from(n).ok());
        s.pull.composing = true;
        if v["review_unknown"] == true {
            s.pull.unknown = Some((repo.clone(), number));
        }
        if v["merge_unknown"] == true {
            s.pull.merge.unknown = Some((repo.clone(), number));
        }
        if let Some(t) = v.get("thread") {
            s.pull.threads.body = field(t, "body", 8192)?;
            s.pull.threads.selected = field(t, "selected", 256)?;
            s.pull.threads.unknown = t["unknown"] == true;
            s.pull.threads.action = match t["action"].as_str() {
                Some("reply") => Some(ForgeThreadAction::Reply),
                Some("resolve") => Some(ForgeThreadAction::Resolve),
                Some("unresolve") => Some(ForgeThreadAction::Unresolve),
                _ => None,
            };
        }
        s.notice=Some("Saved review restored. Refresh validates its original revision; no previous preview or submit is replayed.".into());
    }
    Ok(())
}
fn invalid_draft(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
pub fn rebind(s: &mut super::State) {
    let Some(details) = &s.pull.details else {
        return;
    };
    if s.drafts.active.as_ref().is_none_or(|scope| {
        scope.number != details.summary().number
            || scope.repository != details.repository().path()
            || s.session
                .as_ref()
                .is_none_or(|session| !scope.matches(session))
    }) {
        return;
    }
    // Keep order intact: partially restoring a list would move the index of an
    // unfinished edit onto a different comment.
    // A reconnect to the same account creates a new backend session. Revalidate
    // existing comments too, so their obsolete session cannot be submitted.
    if s.drafts.restored.is_empty() && !s.pull.comments.is_empty() {
        s.drafts.restored = s.pull.comments.drain(..).map(|c| c.draft()).collect();
    }
    if !s.drafts.restored.is_empty()
        && let Ok(comments) = s
            .drafts
            .restored
            .iter()
            .map(|raw| details.restore_inline_comment(raw))
            .collect::<rocket_git::Result<Vec<_>>>()
    {
        s.pull.comments = comments;
        s.drafts.restored.clear();
        if s.pull
            .inline_editing
            .is_some_and(|i| i >= s.pull.comments.len())
        {
            s.pull.inline_editing = None;
        }
    }
    if !s.drafts.restored.is_empty() {
        return;
    }
    let Some(v) = s.drafts.restored_selection.as_ref() else {
        return;
    };
    if v["head"].as_str() != Some(&details.summary().head)
        || v["base"].as_str() != Some(&details.summary().base)
        || v["diff_revision"].as_str() != details.inline_diff_revision().as_deref()
    {
        return;
    }
    let Some(file) = details
        .files
        .items
        .iter()
        .position(|f| Some(f.path.as_str()) == v["path"].as_str())
    else {
        return;
    };
    let side = match v["side"].as_str() {
        Some("LEFT") => ForgeDiffSide::Left,
        Some("RIGHT") => ForgeDiffSide::Right,
        _ => return,
    };
    let (Some(start), Some(line)) = (v["start"].as_u64(), v["line"].as_u64()) else {
        return;
    };
    if details
        .inline_comment_range(file, start, line, side, "validate saved location")
        .is_err()
    {
        return;
    }
    s.pull.inline_selection = Some(pull_review::inline::Selection {
        file,
        path: details.files.items[file].path.clone(),
        head: details.summary().head.clone(),
        base: details.summary().base.clone(),
        diff_revision: details.inline_diff_revision(),
        start,
        line,
        side,
    });
    s.drafts.restored_selection = None;
}
pub(super) fn panel(
    ui: &mut Ui,
    s: &mut super::State,
    widgets: &mut HashMap<String, UiControl>,
    busy: bool,
    out: &mut PageOut,
) {
    ui.heading("Saved PR drafts and recovery");
    ui.label(if cfg!(windows) {
        "Protected for your Windows user. Connect the same account to restore a draft; saved requests never run automatically."
    } else if cfg!(target_os = "macos") {
        "Encrypted with a key in your macOS Keychain. Connect the same account to restore a draft; saved requests never run automatically."
    } else {
        "Encrypted with a key in your desktop keyring. Connect the same account to restore a draft; saved requests never run automatically."
    });
    if let Some(error) = &s.drafts.error {
        ui.colored_label(RED(), error);
        ui.label("Fix the storage problem or shorten/remove an unneeded draft, then retry. Rocket will check the original file again before saving.");
        if btn(
            ui,
            widgets,
            "hosting-drafts-retry",
            "Retry draft storage",
            !busy && s.drafts.task.is_none(),
        ) {
            s.drafts.error = None;
        }
    }
    let mut restore = None;
    let mut delete = None;
    for (i, r) in s.drafts.records.iter().enumerate() {
        ui.separator();
        ui.strong(format!(
            "{} · {} #{}",
            r.scope.provider.label(),
            r.scope.repository,
            r.scope.number
        ));
        ui.add(Label::new(format!("{} · {}", r.scope.server, r.scope.account)).wrap());
        if r.data["review_unknown"] == true
            || r.data["merge_unknown"] == true
            || r.data["thread"]["unknown"] == true
        {
            ui.colored_label(AMBER(),"Outcome needs checking on the server. This recovery record cannot be deleted here.");
        }
        let matches = s
            .session
            .as_ref()
            .is_some_and(|session| r.scope.matches(session));
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                widgets,
                &format!("hosting-draft-restore-{i}"),
                "Restore and refresh",
                !busy && matches,
            ) {
                restore = Some(i);
            }
            if btn(
                ui,
                widgets,
                &format!("hosting-draft-delete-{i}"),
                "Delete saved draft",
                !busy
                    && r.data["review_unknown"] != true
                    && r.data["merge_unknown"] != true
                    && r.data["thread"]["unknown"] != true,
            ) {
                s.drafts.delete_scope = Some(r.scope.clone());
            }
        });
    }
    if s.drafts.records.is_empty() {
        ui.label("No saved PR drafts.");
    }
    if s.drafts.delete_scope.is_some() {
        ui.colored_label(
            AMBER(),
            "Delete this local review draft? Nothing is deleted on the server.",
        );
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                widgets,
                "hosting-draft-delete-confirm",
                "Delete draft",
                !busy,
            ) {
                delete = s.drafts.delete_scope.clone();
            }
            if btn(
                ui,
                widgets,
                "hosting-draft-delete-cancel",
                "Keep draft",
                !busy,
            ) {
                s.drafts.delete_scope = None;
            }
        });
    }
    if let Some(scope) = delete {
        if let Some(i) = s.drafts.records.iter().position(|r| r.scope == scope) {
            let r = &s.drafts.records[i];
            if r.data["review_unknown"] == true
                || r.data["merge_unknown"] == true
                || r.data["thread"]["unknown"] == true
            {
                s.drafts.delete_scope = None;
                s.error = Some(
                    "Check the uncertain server outcome before deleting its recovery record."
                        .into(),
                );
                return;
            }
            if s.drafts.active.as_ref() == Some(&r.scope) {
                s.pull = Default::default();
                s.drafts.active = None;
                s.drafts.restored.clear();
                s.drafts.restored_selection = None;
            }
            s.drafts.records.remove(i);
        }
        s.drafts.delete_scope = None;
    }
    if let Some(i) = restore {
        if s.pull.has_draft() || s.drafts.has_restored() {
            s.error = Some(
                "Finish or discard the currently open review draft before restoring another."
                    .into(),
            );
            return;
        }
        let r = s.drafts.records[i].clone();
        let Some(session) = s
            .session
            .as_ref()
            .filter(|session| r.scope.matches(session))
        else {
            return;
        };
        match ForgeRepository::new(session.config().clone(), &r.scope.repository) {
            Ok(repo) => {
                s.pull = Default::default();
                s.pull.repository = Some(repo.clone());
                s.pull.number = r.scope.number;
                s.drafts.active = None;
                match activate(s, &repo, r.scope.number) {
                    Ok(()) => {
                        s.repository = r.scope.repository;
                        s.section = Section::PullDetail;
                        out.action = Some("pull-refresh");
                    }
                    Err(e) => s.error = Some(user_error(&e)),
                }
            }
            Err(e) => s.error = Some(user_error(&e)),
        }
    }
}

#[cfg(all(test, windows))]
mod lifecycle_tests;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn protected_drafts_roundtrip_and_external_edit_fail_closed() {
        let dir = std::env::temp_dir().join(format!(
            "rocket-pr-drafts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let scope = Scope {
            provider: ForgeProvider::GitLab,
            server: "https://gitlab.com".into(),
            api: "7.1".into(),
            account: "fixture".into(),
            repository: "team/sample".into(),
            number: 42,
        };
        let comment = ForgeInlineDraft { path: "資料/新 é 🚀.rs".into(), start: 21, line: 21,
            side: ForgeDiffSide::Right, head: "a".repeat(40), base: "b".repeat(40), body: "PRIVATE inline".into(),
            diff_revision: Some(format!("{}:{}:{}", "c".repeat(40), "a".repeat(40), "b".repeat(40))) };
        let data = json!({"body":"PRIVATE draft 資料","action":"COMMENT","comments":[comment_json(&comment)],"inline_body":"unfinished","review_unknown":true,"merge_unknown":false});
        let record = Record { scope, data };
        let saved = save(&dir, vec![record.clone()], None).unwrap();
        assert!(
            !saved
                .snapshot
                .as_ref()
                .unwrap()
                .windows(7)
                .any(|w| w == b"PRIVATE")
        );
        assert_eq!(load(&dir).unwrap().records[0].data, record.data);
        assert_eq!(parse_comment(&load(&dir).unwrap().records[0].data["comments"][0]).unwrap(), comment);
        let mut invalid = comment_json(&comment);
        invalid["diff_revision"] = json!("invalid");
        assert!(parse_comment(&invalid).is_err());
        invalid.as_object_mut().unwrap().remove("diff_revision");
        assert_eq!(parse_comment(&invalid).unwrap().diff_revision, None);
        for revision in ["bitbucket-server:7".to_owned(),format!("azure:3:{}:{}:{}","c".repeat(40),"a".repeat(40),"b".repeat(40))] {
            invalid["diff_revision"] = json!(revision);
            assert_eq!(parse_comment(&invalid).unwrap().diff_revision.as_deref(),Some(revision.as_str()));
        }
        for revision in ["azure:0:a:b:c","azure:3:a:b:c","bitbucket-server:-1","bitbucket-server:999999999999","bitbucket-server:7:extra"] {
            invalid["diff_revision"] = json!(revision); assert!(parse_comment(&invalid).is_err());
        }
        fs::write(dir.join(FILE), b"external edit").unwrap();
        assert!(save(&dir, vec![], saved.snapshot).is_err());
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), b"external edit");
        assert!(load(&dir).is_err());
        fs::remove_file(dir.join(FILE)).unwrap();
        fs::remove_dir(&dir).unwrap();
    }
}
