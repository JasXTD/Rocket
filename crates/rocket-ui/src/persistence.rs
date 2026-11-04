//! Bounded, versioned local state. All I/O is performed by the controller's workers.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
mod drafts;

const MAX_BYTES: u64 = 8 * 1024 * 1024;
// v18 atomically points to a streamed draft archive and encloses the existing
// settings format. Older binaries reject it instead of losing external drafts.
const DRAFT_STORE_MAGIC: &[u8] = b"ROCKET-STATE\0\x12";
// v19 extends the enclosed settings with validated keyboard/mouse overrides.
const SHORTCUT_MAGIC: &[u8] = b"ROCKET-STATE\0\x13";
// v20 appends the theme chosen in Preferences → Appearance after the v19 settings.
const THEME_MAGIC: &[u8] = b"ROCKET-STATE\0\x14";
// v21 appends workspace-parent records. Older binaries reject this version,
// rather than silently flattening a saved hierarchy or merging sibling names.
const WORKSPACES_MAGIC: &[u8] = b"ROCKET-STATE\0\x15";
// v22 appends the General page's repository, diff and commit list settings. It is
// written only once one differs from its default, so an unchanged state stays readable
// by older binaries.
const GENERAL_MAGIC: &[u8] = b"ROCKET-STATE\0\x16";
// v23 appends the shell chosen on Preferences → Terminal, under the same rule.
const SHELL_MAGIC: &[u8] = b"ROCKET-STATE\0\x17";
// v24 adds opt-in pre-push dependency review. Default-off profiles retain their
// earlier format; older binaries reject an enabled policy instead of losing it.
const PUSH_CHECK_MAGIC: &[u8] = b"ROCKET-STATE\0\x18";
// v25 persists explicit opt-out of the independent default-on AI-key guard.
// Default-on settings retain their older representation. Older binaries reject
// v25 rather than silently losing a user's explicit security preference.
const AI_KEYS_MAGIC: &[u8] = b"ROCKET-STATE\0\x19";
// v15 uses the v14 layout and version-2 definitions for context-specific commands.
const SCOPED_MAGIC: &[u8] = b"ROCKET-STATE\0\x0f";
const URL_MAGIC: &[u8] = b"ROCKET-STATE\0\x10";
const ORDER_MAGIC: &[u8] = b"ROCKET-STATE\0\x11";
const SEARCH_MAGIC: &[u8] = b"ROCKET-STATE\0\x0e";
// v13 appends the default-on offline dependency-warning preference.
const DEPENDENCIES_MAGIC: &[u8] = b"ROCKET-STATE\0\x0d";
// v12 adds the default-on secret guard; v11 adds per-worktree views and command forms.
const SECRETS_MAGIC: &[u8] = b"ROCKET-STATE\0\x0c";
// v9 appends the drafting CLI choice and Codex's consent and program.
// v8 appends the Claude Code drafting preferences.
// v7 appends saved repositories, folders and workspaces.
// v6 appends the repository-list and spell-check preferences; v5 the automatic-fetch
// interval and custom commands; v4 recent commit messages; v3 the sidebar-labels
// preference; v2 added external tools.
const MAGIC: &[u8] = b"ROCKET-STATE\0\x0b";
const PROFILES_MAGIC: &[u8] = b"ROCKET-STATE\0\x0a";
const CODEX_MAGIC: &[u8] = b"ROCKET-STATE\0\x09";
const DRAFTING_MAGIC: &[u8] = b"ROCKET-STATE\0\x08";
const LIBRARY_MAGIC: &[u8] = b"ROCKET-STATE\0\x07";
const UI_MAGIC: &[u8] = b"ROCKET-STATE\0\x06";
const COMMANDS_MAGIC: &[u8] = b"ROCKET-STATE\0\x05";
const MESSAGES_MAGIC: &[u8] = b"ROCKET-STATE\0\x04";
const LABELS_MAGIC: &[u8] = b"ROCKET-STATE\0\x03";
const TOOLS_MAGIC: &[u8] = b"ROCKET-STATE\0\x02";
const OLD_MAGIC: &[u8] = b"ROCKET-STATE\0\x01";
/// Repositories whose recent commit messages are kept.
pub const MESSAGE_REPOSITORIES: usize = 64;
/// Per-worktree presentation records retained in the state file.
pub const WORKTREE_VIEWS: usize = 256;
/// Automatic fetch intervals in minutes; 0 turns automatic fetch off.
pub const AUTO_FETCH_MINUTES: [u16; 6] = [0, 5, 10, 15, 30, 60];

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub sidebar: bool,
    pub details: bool,
    pub console: bool,
    pub auto_refresh: bool,
    pub secret_protection: bool,
    pub guard_ai_api_keys: bool,
    pub dependency_warnings: bool,
    pub dependency_before_push: bool,
    pub row_height: f32,
    pub zoom: f32,
    /// Off shows the sidebar as an icon rail.
    pub sidebar_labels: bool,
    /// Minutes between automatic fetches of every open repository; 0 is off.
    pub auto_fetch_minutes: u16,
    /// Open repositories are listed in the sidebar instead of tabs above it.
    pub repository_list: bool,
    /// Commit messages are spell-checked (Windows only).
    pub spell_check: bool,
    /// Commit message drafts with Claude Code; off by default.
    pub drafting: crate::commit_draft::Settings,
    pub shortcuts: crate::shortcuts::model::Bindings,
    pub theme: crate::theme::Settings,
    pub general: crate::general::Settings,
    pub shell: crate::terminal_settings::Settings,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            sidebar: true,
            details: true,
            console: false,
            auto_refresh: true,
            secret_protection: true,
            guard_ai_api_keys: true,
            dependency_warnings: true,
            dependency_before_push: false,
            row_height: 33.0,
            zoom: 1.0,
            sidebar_labels: true,
            auto_fetch_minutes: 0,
            repository_list: false,
            spell_check: false,
            drafting: Default::default(),
            shortcuts: Default::default(),
            theme: Default::default(),
            general: Default::default(),
            shell: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedState {
    pub hosting_searches: Vec<crate::forge::search::Recent>,
    pub worktree_views: BTreeMap<PathBuf, crate::worktree_view::Settings>,
    pub library: crate::repository_library::Library,
    pub tools: [rocket_git::ExternalToolConfig; 3],
    pub tool_profiles: Option<crate::tool_profiles::Profiles>,
    pub preferences: Preferences,
    pub recent: Vec<PathBuf>,
    /// Immutable text is shared with in-flight and written snapshots. Unchanged
    /// drafts compare by Arc identity and cloning never copies retained text.
    /// Replace a value when editing; the persisted format remains unchanged.
    pub drafts: BTreeMap<PathBuf, Arc<str>>,
    /// Messages of commits made in Rocket, newest first, per repository.
    pub recent_messages: BTreeMap<PathBuf, Vec<String>>,
    /// User-defined commands, in menu order.
    pub commands: Vec<rocket_git::CustomCommand>,
}

impl SavedState {
    /// Preserve sharing on unchanged text and on edits reverted to the written
    /// value, including when closing a tab before the next autosave poll.
    pub(crate) fn set_draft(&mut self, path: &Path, text: &str, written: &Self) -> bool {
        if text.is_empty() {
            return self.drafts.remove(path).is_some();
        }
        if self.drafts.get(path).map(AsRef::as_ref) == Some(text) {
            return false;
        }
        let text = written.drafts.get(path)
            .filter(|previous| previous.as_ref() == text)
            .cloned()
            .unwrap_or_else(|| text.into());
        self.drafts.insert(path.to_owned(), text);
        true
    }

    /// Prefer active/open/recent views when the presentation cache reaches its
    /// on-disk bound. Drafts and repository-library membership are independent.
    pub(crate) fn retain_worktree_views<'a>(
        &mut self,
        preferred: impl IntoIterator<Item = &'a PathBuf>,
    ) -> bool {
        if self.worktree_views.len() <= WORKTREE_VIEWS {
            return false;
        }
        let mut keep = BTreeSet::new();
        for path in preferred {
            if keep.len() == WORKTREE_VIEWS {
                break;
            }
            if self.worktree_views.contains_key(path) {
                keep.insert(path.clone());
            }
        }
        for path in self.worktree_views.keys() {
            if keep.len() == WORKTREE_VIEWS {
                break;
            }
            keep.insert(path.clone());
        }
        self.worktree_views.retain(|path, _| keep.contains(path));
        true
    }
}

pub struct Store {
    directory: PathBuf,
    _lock: File,
    archive: Mutex<Option<String>>,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or unsupported Rocket state; existing file preserved",
    )
}
impl Store {
    pub fn open(directory: &Path) -> io::Result<(Self, SavedState)> {
        fs::create_dir_all(directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0); // OS releases ownership on exit/crash; no stale lock deletion.
        }
        #[cfg(not(windows))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "State ownership currently requires Windows",
        ));
        #[cfg(windows)]
        {
            let lock = options.open(directory.join("state.lock"))?;
            let mut archive = None;
            let mut state = match File::open(directory.join("state.bin")) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > MAX_BYTES {
                        return Err(invalid());
                    }
                    if let Some(inner) = bytes.strip_prefix(DRAFT_STORE_MAGIC) {
                        let mut r = Reader(inner);
                        let name = std::str::from_utf8(r.bytes()?).map_err(|_| invalid())?;
                        if !drafts::archive_name(name) { return Err(invalid()); }
                        archive = Some(name.to_owned());
                        let settings = decode(r.0)?;
                        if !settings.drafts.is_empty() { return Err(invalid()); }
                        settings
                    } else {
                        decode(&bytes)?
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => SavedState::default(),
                Err(e) => return Err(e),
            };
            if let Some(name) = &archive {
                state.drafts = drafts::read(&directory.join(name))?;
            }
            Ok((
                Self {
                    directory: directory.into(),
                    _lock: lock,
                    archive: Mutex::new(archive),
                },
                state,
            ))
        }
    }
    pub fn save(&self, state: &SavedState) -> io::Result<()> {
        let mut archive = self.archive.lock().map_err(|_| io::Error::other("State writer poisoned"))?;
        drafts::validate(&state.drafts)?;
        let settings = encode_inner(state, false)?;
        // Only this Store holds the OS lock; each temporary file has a unique name.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_nanos();
        let name = format!("drafts-{}-{stamp}.bin", std::process::id());
        let draft_path = self.directory.join(&name);
        let mut bytes = DRAFT_STORE_MAGIC.to_vec();
        put(&mut bytes, name.as_bytes())?;
        bytes.extend(settings);
        if bytes.len() as u64 > MAX_BYTES { return Err(invalid()); }
        drafts::write(&draft_path, &state.drafts)?;
        let temporary = self
            .directory
            .join(format!("state-{}-{stamp}.tmp", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.directory.join("state.bin"))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&draft_path);
        } else if let Some(previous) = archive.replace(name) {
            // Never remove the old archive before publishing the new pointer.
            let _ = fs::remove_file(self.directory.join(previous));
        }
        result
    }
}

fn put(bytes: &mut Vec<u8>, value: &[u8]) -> io::Result<()> {
    if value.len() > 128 * 1024 {
        return Err(invalid());
    }
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value);
    Ok(())
}
fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().as_encoded_bytes().to_vec()
    }
}
fn encode(state: &SavedState) -> io::Result<Vec<u8>> {
    encode_inner(state, true)
}
fn encode_inner(state: &SavedState, inline_drafts: bool) -> io::Result<Vec<u8>> {
    if state.recent.len() > 12 || (inline_drafts && state.drafts.len() > 64) {
        return Err(invalid());
    }
    let p = &state.preferences;
    if ![29.0, 33.0, 38.0].contains(&p.row_height) || !(0.75..=1.5).contains(&p.zoom) {
        return Err(invalid());
    }
    if !p.general.valid() || !p.shell.valid() {
        return Err(invalid());
    }
    let ai_keys = !p.guard_ai_api_keys;
    let push_check = ai_keys || p.dependency_before_push;
    let shell = push_check || p.shell != crate::terminal_settings::Settings::default();
    let general = shell || p.general != crate::general::Settings::default();
    let nested_workspaces = general || state.library.workspaces.iter().any(|w| w.parent.is_some());
    let themes = nested_workspaces || p.theme != crate::theme::Settings::default();
    let shortcuts = themes || !p.shortcuts.0.is_empty();
    let orders = shortcuts || state.worktree_views.values().any(|v| v.history_order != rocket_git::HistoryOrder::default());
    let urls = orders || state.commands.iter().any(|c| c.action == rocket_git::CommandAction::Url);
    let scoped = urls || state.commands.iter().any(|c| c.scope != rocket_git::CommandScope::Repository);
    let mut bytes = if ai_keys { AI_KEYS_MAGIC } else if push_check { PUSH_CHECK_MAGIC } else if shell { SHELL_MAGIC } else if general { GENERAL_MAGIC } else if nested_workspaces { WORKSPACES_MAGIC } else if themes { THEME_MAGIC } else if shortcuts { SHORTCUT_MAGIC } else if orders { ORDER_MAGIC } else if urls { URL_MAGIC } else if scoped { SCOPED_MAGIC } else if state.hosting_searches.is_empty() { DEPENDENCIES_MAGIC } else { SEARCH_MAGIC }.to_vec();
    bytes.extend([
        u8::from(p.sidebar),
        u8::from(p.details),
        u8::from(p.console),
        u8::from(p.auto_refresh),
    ]);
    bytes.extend(p.row_height.to_le_bytes());
    bytes.extend(p.zoom.to_le_bytes());
    bytes.extend((state.recent.len() as u32).to_le_bytes());
    for path in &state.recent {
        put(&mut bytes, &path_bytes(path))?;
    }
    bytes.extend((if inline_drafts { state.drafts.len() as u32 } else { 0 }).to_le_bytes());
    if inline_drafts {
        for (path, draft) in &state.drafts {
            put(&mut bytes, &path_bytes(path))?;
            put(&mut bytes, draft.as_bytes())?;
        }
    }
    for (kind, tool) in [
        rocket_git::ExternalToolKind::Editor,
        rocket_git::ExternalToolKind::Diff,
        rocket_git::ExternalToolKind::Merge,
    ]
    .into_iter()
    .zip(&state.tools)
    {
        tool.validate(kind).map_err(|_| invalid())?;
        put(&mut bytes, tool.executable.as_bytes())?;
        put(&mut bytes, tool.arguments.as_bytes())?;
    }
    bytes.push(u8::from(p.sidebar_labels));
    if state.recent_messages.len() > MESSAGE_REPOSITORIES {
        return Err(invalid());
    }
    bytes.extend((state.recent_messages.len() as u32).to_le_bytes());
    for (path, messages) in &state.recent_messages {
        if !(1..=crate::composer::RECENT_MESSAGES).contains(&messages.len()) {
            return Err(invalid());
        }
        put(&mut bytes, &path_bytes(path))?;
        bytes.extend((messages.len() as u32).to_le_bytes());
        for message in messages {
            if !(1..=crate::composer::RECENT_MESSAGE_BYTES).contains(&message.len()) {
                return Err(invalid());
            }
            put(&mut bytes, message.as_bytes())?;
        }
    }
    if !AUTO_FETCH_MINUTES.contains(&p.auto_fetch_minutes)
        || state.commands.len() > rocket_git::MAX_CUSTOM_COMMANDS
    {
        return Err(invalid());
    }
    bytes.extend(p.auto_fetch_minutes.to_le_bytes());
    bytes.extend((state.commands.len() as u32).to_le_bytes());
    for command in &state.commands {
        command.validate().map_err(|_| invalid())?;
        put(&mut bytes, command.name.as_bytes())?;
        put(&mut bytes, command.program.as_bytes())?;
        put(&mut bytes, command.arguments.as_bytes())?;
    }
    bytes.push(u8::from(p.repository_list));
    bytes.push(u8::from(p.spell_check));
    encode_library(&mut bytes, &state.library)?;
    let d = &p.drafting;
    bytes.extend([
        u8::from(d.enabled),
        u8::from(d.notice_accepted),
        u8::from(d.style == rocket_git::DraftStyle::Conventional),
    ]);
    put(&mut bytes, d.executable.as_bytes())?;
    put(&mut bytes, d.language.as_bytes())?;
    bytes.extend([
        u8::from(d.provider == rocket_git::DraftProvider::Codex),
        u8::from(d.codex_notice_accepted),
    ]);
    put(&mut bytes, d.codex_executable.as_bytes())?;
    encode_profiles(&mut bytes, state.tool_profiles.as_ref())?;
    encode_views(&mut bytes, &state.worktree_views)?;
    put(&mut bytes, &crate::command_definitions::encode(&state.commands).map_err(|_|invalid())?)?;
    bytes.push(u8::from(p.secret_protection));
    bytes.push(u8::from(p.dependency_warnings));
    if scoped || !state.hosting_searches.is_empty() {
        if state.hosting_searches.len() > 12 { return Err(invalid()); }
        bytes.extend((state.hosting_searches.len() as u32).to_le_bytes());
        for recent in &state.hosting_searches {
            if !recent.valid() { return Err(invalid()); }
            for text in [&recent.server, &recent.repository, &recent.search.text, &recent.search.author, &recent.search.assignee] { put(&mut bytes, text.as_bytes())?; }
            bytes.push(u8::from(recent.kind == rocket_git::ForgeKind::Issues));
            bytes.push(match recent.state { rocket_git::ForgeState::Open => 0, rocket_git::ForgeState::Closed => 1, rocket_git::ForgeState::All => 2 });
        }
    }
    if orders {
        bytes.extend((state.worktree_views.len() as u32).to_le_bytes());
        for (path, view) in &state.worktree_views {
            put(&mut bytes, &path_bytes(path))?;
            bytes.push(match view.history_order { rocket_git::HistoryOrder::Topological => 0, rocket_git::HistoryOrder::CommitDate => 1, rocket_git::HistoryOrder::AuthorDate => 2 });
        }
    }
    if shortcuts { put(&mut bytes, &p.shortcuts.encode().map_err(|_| invalid())?)?; }
    if themes { put(&mut bytes, &p.theme.encode())?; }
    if nested_workspaces {
        bytes.extend((state.library.workspaces.len() as u32).to_le_bytes());
        for workspace in &state.library.workspaces {
            bytes.extend(workspace.id.to_le_bytes());
            bytes.extend(workspace.parent.unwrap_or(0).to_le_bytes());
        }
    }
    if general {
        let g = &p.general;
        bytes.push(u8::from(g.source_folder.is_some()));
        if let Some(path) = &g.source_folder {
            put(&mut bytes, &path_bytes(path))?;
        }
        bytes.extend([
            match g.fetch_tags { rocket_git::FetchTags::None => 0, rocket_git::FetchTags::RemoteDefault => 1, rocket_git::FetchTags::All => 2 },
            match g.branch_space { crate::general::BranchSpace::Keep => 0, crate::general::BranchSpace::Hyphen => 1, crate::general::BranchSpace::Underscore => 2 },
            u8::from(g.submodules_on_checkout),
            u8::from(g.push_after_commit),
            u8::from(g.diff_marks),
            g.diff_font,
            u8::from(g.diff_syntax),
            u8::from(g.compact_labels),
        ]);
    }
    if shell {
        use crate::terminal_settings::Choice;
        let s = &p.shell;
        bytes.push(match s.choice {
            Choice::WindowsPowerShell => 0,
            Choice::GitBash => 1,
            Choice::CommandPrompt => 2,
            Choice::PowerShell7 => 3,
            Choice::Custom => 4,
        });
        put(&mut bytes, s.program.as_bytes())?;
        put(&mut bytes, s.arguments.as_bytes())?;
    }
    if push_check { bytes.push(u8::from(p.dependency_before_push)); }
    if ai_keys { bytes.push(u8::from(p.guard_ai_api_keys)); }
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}
fn encode_views(bytes: &mut Vec<u8>, views: &BTreeMap<PathBuf, crate::worktree_view::Settings>) -> io::Result<()> {
    if views.len() > WORKTREE_VIEWS { return Err(invalid()); }
    bytes.extend((views.len() as u32).to_le_bytes());
    for (path, v) in views {
        if !path.is_absolute() || !v.valid() { return Err(invalid()); }
        put(bytes, &path_bytes(path))?;
        put(bytes, v.branch_filter.as_bytes())?;
        put(bytes, v.history_ref.as_deref().unwrap_or("").as_bytes())?;
        bytes.extend([u8::from(v.history_all), u8::from(v.hide_tags), u8::from(v.hide_stashes), u8::from(v.split_diff), u8::from(v.folding.all)]);
        bytes.extend(v.sections.map(u8::from));
        let mut folded: Vec<_> = v.folding.except.iter().collect(); folded.sort_by_key(|v| v.as_str());
        bytes.extend((folded.len() as u32).to_le_bytes());
        for id in folded { put(bytes, id.as_str().as_bytes())?; }
    }
    Ok(())
}
fn decode_views(r: &mut Reader<'_>) -> io::Result<BTreeMap<PathBuf, crate::worktree_view::Settings>> {
    let mut views = BTreeMap::new(); let count = r.u32()?;
    if count as usize > WORKTREE_VIEWS { return Err(invalid()); }
    let text = |r: &mut Reader| -> io::Result<String> { Ok(std::str::from_utf8(r.bytes()?).map_err(|_|invalid())?.into()) };
    for _ in 0..count {
        let path = r.path()?;
        let branch_filter = text(r)?; let reference = text(r)?;
        let mut v = crate::worktree_view::Settings {
            branch_filter, history_ref: (!reference.is_empty()).then_some(reference),
            history_all: r.boolean()?, hide_tags: r.boolean()?, hide_stashes: r.boolean()?,
            split_diff: r.boolean()?, ..Default::default()
        };
        v.folding.all = r.boolean()?;
        for open in &mut v.sections { *open = r.boolean()?; }
        let count = r.u32()?; if count > 2048 { return Err(invalid()); }
        for _ in 0..count {
            let id = rocket_git::ObjectId::parse(&text(r)?).map_err(|_|invalid())?;
            if !v.folding.except.insert(id) { return Err(invalid()); }
        }
        if !v.valid() || views.insert(path, v).is_some() { return Err(invalid()); }
    }
    Ok(views)
}
fn encode_profiles(bytes: &mut Vec<u8>, profiles: Option<&crate::tool_profiles::Profiles>) -> io::Result<()> {
    bytes.push(u8::from(profiles.is_some()));
    if let Some(profiles) = profiles {
        profiles.validate().map_err(|_| invalid())?;
        for group in &profiles.0 {
            bytes.extend((group.entries.len() as u32).to_le_bytes());
            bytes.extend((group.default as u32).to_le_bytes());
            for profile in &group.entries {
                put(bytes, profile.name.as_bytes())?;
                put(bytes, profile.config.executable.as_bytes())?;
                put(bytes, profile.config.arguments.as_bytes())?;
            }
        }
    }
    Ok(())
}
fn decode_profiles(r: &mut Reader<'_>) -> io::Result<Option<crate::tool_profiles::Profiles>> {
    if !r.boolean()? { return Ok(None); }
    let mut profiles = crate::tool_profiles::Profiles::default();
    for group in &mut profiles.0 {
        let count = r.u32()? as usize;
        if !(1..=crate::tool_profiles::MAX_PROFILES).contains(&count) { return Err(invalid()); }
        group.default = r.u32()? as usize;
        group.entries.clear();
        let text = |r: &mut Reader| -> io::Result<String> {
            Ok(std::str::from_utf8(r.bytes()?).map_err(|_| invalid())?.to_owned())
        };
        for _ in 0..count {
            group.entries.push(crate::tool_profiles::Profile {
                name: text(r)?,
                config: rocket_git::ExternalToolConfig {
                    executable: text(r)?, arguments: text(r)?,
                },
            });
        }
    }
    profiles.validate().map_err(|_| invalid())?;
    Ok(Some(profiles))
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if n > self.0.len() {
            return Err(invalid());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > 128 * 1024 {
            return Err(invalid());
        }
        self.take(n)
    }
    fn boolean(&mut self) -> io::Result<bool> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }
    fn path(&mut self) -> io::Result<PathBuf> {
        let bytes = self.bytes()?;
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            if !bytes.len().is_multiple_of(2) {
                return Err(invalid());
            }
            let wide: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            if wide.contains(&0) {
                return Err(invalid());
            }
            PathBuf::from(std::ffi::OsString::from_wide(&wide))
        };
        #[cfg(not(windows))]
        let path = PathBuf::from(std::str::from_utf8(bytes).map_err(|_| invalid())?);
        if !path.is_absolute() {
            return Err(invalid());
        }
        Ok(path)
    }
}
fn decode(bytes: &[u8]) -> io::Result<SavedState> {
    let mut r = Reader(bytes);
    let magic = r.take(MAGIC.len())?;
    let ai_keys = magic == AI_KEYS_MAGIC;
    let push_check = ai_keys || magic == PUSH_CHECK_MAGIC;
    let shell = push_check || magic == SHELL_MAGIC;
    let general = shell || magic == GENERAL_MAGIC;
    let nested_workspaces = general || magic == WORKSPACES_MAGIC;
    let themes = nested_workspaces || magic == THEME_MAGIC;
    let shortcuts = themes || magic == SHORTCUT_MAGIC;
    let orders = shortcuts || magic == ORDER_MAGIC;
    let urls = orders || magic == URL_MAGIC;
    let scoped = urls || magic == SCOPED_MAGIC;
    let search_preferences = magic == SEARCH_MAGIC || scoped;
    let dependency_preferences = magic == DEPENDENCIES_MAGIC || search_preferences;
    let secret_preferences = magic == SECRETS_MAGIC || dependency_preferences;
    // v12 preserves the entire v11 layout and appends one explicit Boolean.
    let magic = if secret_preferences { MAGIC } else { magic };
    if ![
        MAGIC,
        PROFILES_MAGIC,
        CODEX_MAGIC,
        DRAFTING_MAGIC,
        LIBRARY_MAGIC,
        UI_MAGIC,
        COMMANDS_MAGIC,
        MESSAGES_MAGIC,
        LABELS_MAGIC,
        TOOLS_MAGIC,
        OLD_MAGIC,
    ]
    .contains(&magic)
    {
        return Err(invalid());
    }
    let preferences = Preferences {
        sidebar: r.boolean()?,
        details: r.boolean()?,
        console: r.boolean()?,
        auto_refresh: r.boolean()?,
        secret_protection: true,
        guard_ai_api_keys: true,
        dependency_warnings: true,
        dependency_before_push: false,
        row_height: f32::from_bits(r.u32()?),
        zoom: f32::from_bits(r.u32()?),
        sidebar_labels: true,
        auto_fetch_minutes: 0,
        repository_list: false,
        spell_check: false,
        drafting: Default::default(),
        shortcuts: Default::default(),
        theme: Default::default(),
        general: Default::default(),
        shell: Default::default(),
    };
    let mut state = SavedState {
        preferences,
        ..Default::default()
    };
    let count = r.u32()?;
    if count > 12 {
        return Err(invalid());
    }
    for _ in 0..count {
        state.recent.push(r.path()?);
    }
    let count = r.u32()?;
    if count > 64 {
        return Err(invalid());
    }
    for _ in 0..count {
        let path = r.path()?;
        let text = std::str::from_utf8(r.bytes()?)
            .map_err(|_| invalid())?
            .to_owned();
        if state.drafts.insert(path, text.into()).is_some() {
            return Err(invalid());
        }
    }
    if magic != OLD_MAGIC {
        for tool in &mut state.tools {
            tool.executable = std::str::from_utf8(r.bytes()?)
                .map_err(|_| invalid())?
                .into();
            tool.arguments = std::str::from_utf8(r.bytes()?)
                .map_err(|_| invalid())?
                .into();
        }
    }
    if [
        MAGIC,
        PROFILES_MAGIC,
        CODEX_MAGIC,
        DRAFTING_MAGIC,
        LIBRARY_MAGIC,
        UI_MAGIC,
        COMMANDS_MAGIC,
        MESSAGES_MAGIC,
        LABELS_MAGIC,
    ]
    .contains(&magic)
    {
        state.preferences.sidebar_labels = r.boolean()?;
    }
    if [
        MAGIC,
        PROFILES_MAGIC,
        CODEX_MAGIC,
        DRAFTING_MAGIC,
        LIBRARY_MAGIC,
        UI_MAGIC,
        COMMANDS_MAGIC,
        MESSAGES_MAGIC,
    ]
    .contains(&magic)
    {
        let count = r.u32()?;
        if count as usize > MESSAGE_REPOSITORIES {
            return Err(invalid());
        }
        for _ in 0..count {
            let path = r.path()?;
            let n = r.u32()? as usize;
            if !(1..=crate::composer::RECENT_MESSAGES).contains(&n) {
                return Err(invalid());
            }
            let mut messages = Vec::with_capacity(n);
            for _ in 0..n {
                messages.push(
                    std::str::from_utf8(r.bytes()?)
                        .map_err(|_| invalid())?
                        .to_owned(),
                );
            }
            if state.recent_messages.insert(path, messages).is_some() {
                return Err(invalid());
            }
        }
    }
    if [
        MAGIC,
        PROFILES_MAGIC,
        CODEX_MAGIC,
        DRAFTING_MAGIC,
        LIBRARY_MAGIC,
        UI_MAGIC,
        COMMANDS_MAGIC,
    ]
    .contains(&magic)
    {
        state.preferences.auto_fetch_minutes = u16::from_le_bytes(r.take(2)?.try_into().unwrap());
        let count = r.u32()?;
        if count as usize > rocket_git::MAX_CUSTOM_COMMANDS {
            return Err(invalid());
        }
        let text = |r: &mut Reader| -> io::Result<String> {
            Ok(std::str::from_utf8(r.bytes()?)
                .map_err(|_| invalid())?
                .to_owned())
        };
        for _ in 0..count {
            let (name, program, arguments) = (text(&mut r)?, text(&mut r)?, text(&mut r)?);
            state.commands.push(rocket_git::CustomCommand {
                name,
                program,
                arguments,
                ..Default::default()
            });
        }
    }
    if [MAGIC, PROFILES_MAGIC, CODEX_MAGIC, DRAFTING_MAGIC, LIBRARY_MAGIC, UI_MAGIC].contains(&magic) {
        state.preferences.repository_list = r.boolean()?;
        state.preferences.spell_check = r.boolean()?;
    }
    if [MAGIC, PROFILES_MAGIC, CODEX_MAGIC, DRAFTING_MAGIC, LIBRARY_MAGIC].contains(&magic) {
        state.library = decode_library(&mut r, nested_workspaces)?;
    }
    if [MAGIC, PROFILES_MAGIC, CODEX_MAGIC, DRAFTING_MAGIC].contains(&magic) {
        let d = &mut state.preferences.drafting;
        d.enabled = r.boolean()?;
        d.notice_accepted = r.boolean()?;
        d.style = if r.boolean()? {
            rocket_git::DraftStyle::Conventional
        } else {
            rocket_git::DraftStyle::Plain
        };
        let text = |r: &mut Reader| -> io::Result<String> {
            Ok(std::str::from_utf8(r.bytes()?)
                .map_err(|_| invalid())?
                .to_owned())
        };
        d.executable = text(&mut r)?;
        d.language = text(&mut r)?;
        if magic == MAGIC || magic == PROFILES_MAGIC || magic == CODEX_MAGIC {
            d.provider = if r.boolean()? {
                rocket_git::DraftProvider::Codex
            } else {
                rocket_git::DraftProvider::Claude
            };
            d.codex_notice_accepted = r.boolean()?;
            d.codex_executable = text(&mut r)?;
        }
    }
    if magic == MAGIC || magic == PROFILES_MAGIC { state.tool_profiles = decode_profiles(&mut r)?; }
    if magic == MAGIC {
        state.worktree_views = decode_views(&mut r)?;
        let commands = crate::command_definitions::decode(r.bytes()?).map_err(|_|invalid())?;
        if !urls && commands.iter().any(|c| c.action != rocket_git::CommandAction::Program) { return Err(invalid()); }
        if !scoped && commands.iter().any(|c| c.scope != rocket_git::CommandScope::Repository) { return Err(invalid()); }
        if commands.len() != state.commands.len() || commands.iter().zip(&state.commands)
            .any(|(a,b)|a.name!=b.name || a.program!=b.program || a.arguments!=b.arguments) { return Err(invalid()); }
        state.commands = commands;
    }
    if secret_preferences { state.preferences.secret_protection = r.boolean()?; }
    if dependency_preferences { state.preferences.dependency_warnings = r.boolean()?; }
    if search_preferences {
        let count = r.u32()?;
        if count > 12 { return Err(invalid()); }
        let text = |r: &mut Reader| -> io::Result<String> { Ok(std::str::from_utf8(r.bytes()?).map_err(|_|invalid())?.into()) };
        for _ in 0..count {
            let server = text(&mut r)?; let repository = text(&mut r)?;
            let search = rocket_git::ForgeSearch { text: text(&mut r)?, author: text(&mut r)?, assignee: text(&mut r)? };
            let kind = if r.boolean()? { rocket_git::ForgeKind::Issues } else { rocket_git::ForgeKind::Pulls };
            let state_filter = match r.take(1)?[0] { 0 => rocket_git::ForgeState::Open, 1 => rocket_git::ForgeState::Closed, 2 => rocket_git::ForgeState::All, _ => return Err(invalid()) };
            state.hosting_searches.push(crate::forge::search::Recent { server, repository, kind, state: state_filter, search });
        }
    }
    if orders {
        let count = r.u32()? as usize;
        if count != state.worktree_views.len() { return Err(invalid()); }
        let mut seen = std::collections::HashSet::new();
        for _ in 0..count {
            let path = r.path()?;
            if !seen.insert(path.clone()) { return Err(invalid()); }
            let view = state.worktree_views.get_mut(&path).ok_or_else(invalid)?;
            view.history_order = match r.take(1)?[0] { 0 => rocket_git::HistoryOrder::Topological,
                1 => rocket_git::HistoryOrder::CommitDate, 2 => rocket_git::HistoryOrder::AuthorDate, _ => return Err(invalid()) };
        }
    }
    if shortcuts { state.preferences.shortcuts = crate::shortcuts::model::Bindings::decode(r.bytes()?).map_err(|_| invalid())?; }
    if themes { state.preferences.theme = crate::theme::Settings::decode(r.bytes()?).ok_or_else(invalid)?; }
    if nested_workspaces {
        let count = r.u32()? as usize;
        if count != state.library.workspaces.len() { return Err(invalid()); }
        let mut seen = BTreeSet::new();
        for _ in 0..count {
            let id = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
            let parent = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
            if !seen.insert(id) { return Err(invalid()); }
            let workspace = state.library.workspaces.iter_mut().find(|w| w.id == id).ok_or_else(invalid)?;
            workspace.parent = (parent != 0).then_some(parent);
        }
    }
    if general {
        let g = &mut state.preferences.general;
        g.source_folder = if r.boolean()? { Some(r.path()?) } else { None };
        g.fetch_tags = match r.take(1)?[0] { 0 => rocket_git::FetchTags::None, 1 => rocket_git::FetchTags::RemoteDefault, 2 => rocket_git::FetchTags::All, _ => return Err(invalid()) };
        g.branch_space = match r.take(1)?[0] { 0 => crate::general::BranchSpace::Keep, 1 => crate::general::BranchSpace::Hyphen, 2 => crate::general::BranchSpace::Underscore, _ => return Err(invalid()) };
        g.submodules_on_checkout = r.boolean()?;
        g.push_after_commit = r.boolean()?;
        g.diff_marks = r.boolean()?;
        g.diff_font = r.take(1)?[0];
        g.diff_syntax = r.boolean()?;
        g.compact_labels = r.boolean()?;
    }
    if shell {
        use crate::terminal_settings::Choice;
        let text = |r: &mut Reader| -> io::Result<String> { Ok(std::str::from_utf8(r.bytes()?).map_err(|_| invalid())?.into()) };
        let s = &mut state.preferences.shell;
        s.choice = match r.take(1)?[0] {
            0 => Choice::WindowsPowerShell,
            1 => Choice::GitBash,
            2 => Choice::CommandPrompt,
            3 => Choice::PowerShell7,
            4 => Choice::Custom,
            _ => return Err(invalid()),
        };
        s.program = text(&mut r)?;
        s.arguments = text(&mut r)?;
    }
    if push_check { state.preferences.dependency_before_push = r.boolean()?; }
    if ai_keys { state.preferences.guard_ai_api_keys = r.boolean()?; }
    if !r.0.is_empty() {
        return Err(invalid());
    }
    encode(&state)?; // Apply the same value limits on both paths.
    Ok(state)
}

#[cfg(all(test, windows))]
mod lifecycle_tests;

#[cfg(all(test, windows))]
mod nested_workspace_tests;

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn dependency_push_opt_in_roundtrips_and_old_profiles_default_off() {
        let mut state = super::SavedState::default();
        let old = super::encode(&state).unwrap();
        assert!(!super::decode(&old).unwrap().preferences.dependency_before_push);
        state.preferences.dependency_before_push = true;
        state.preferences.dependency_warnings = false;
        let bytes = super::encode(&state).unwrap();
        assert!(bytes.starts_with(super::PUSH_CHECK_MAGIC));
        assert_eq!(super::decode(&bytes).unwrap(), state);
        assert!(super::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() = 2;
        assert!(super::decode(&bad).is_err());
        state.preferences.dependency_before_push = false;
        state.preferences.dependency_warnings = true;
        assert_eq!(super::encode(&state).unwrap(), old);
    }
    #[test]
    fn shortcuts_roundtrip_with_drafts_and_prior_formats_keep_defaults() {
        use crate::shortcuts::model::{Action, Binding, Bindings};
        let mut state = super::SavedState::default();
        let path = std::path::PathBuf::from("C:/Rocket fixtures/資料");
        state.drafts.insert(path.clone(), "private draft 🦀".into());
        state.worktree_views.insert(path, Default::default());
        let old = super::encode(&state).unwrap();
        assert_eq!(super::decode(&old).unwrap().preferences.shortcuts, Bindings::default());
        state.preferences.shortcuts.set(Action::Sidebar, vec![Binding::parse("Ctrl+F8").unwrap(), Binding::parse("MouseBack").unwrap()]).unwrap();
        let bytes = super::encode(&state).unwrap();
        assert!(bytes.starts_with(super::SHORTCUT_MAGIC));
        assert_eq!(super::decode(&bytes).unwrap(), state);
        let mut corrupted = bytes.clone();
        let offset = corrupted.windows(b"MouseBack".len()).position(|w| w == b"MouseBack").unwrap();
        corrupted[offset..offset+9].copy_from_slice(b"MouseLeft");
        assert!(super::decode(&corrupted).is_err(), "invalid unmodified click must not load");
        state.preferences.shortcuts = Bindings::default();
        assert_eq!(super::encode(&state).unwrap(), old);
    }
    #[test]
    fn themes_roundtrip_and_older_formats_keep_the_default_theme() {
        use crate::theme::{Choice, Custom, Picture, Settings};
        let mut state = super::SavedState::default();
        let old = super::encode(&state).unwrap();
        assert_eq!(super::decode(&old).unwrap().preferences.theme, Settings::default());
        state.preferences.theme.choice = Choice::Light;
        let bytes = super::encode(&state).unwrap();
        assert!(bytes.starts_with(super::THEME_MAGIC));
        assert_eq!(super::decode(&bytes).unwrap(), state);
        state.preferences.theme = Settings {
            choice: Choice::Custom,
            custom: Custom { picture: Picture::Own("picture-00000000000000ff.png".into()), ..Default::default() },
        };
        let bytes = super::encode(&state).unwrap();
        assert_eq!(super::decode(&bytes).unwrap(), state);
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() = 99;
        assert!(super::decode(&bad).is_err(), "a visibility out of range must not load");
        state.preferences.theme = Settings::default();
        assert_eq!(super::encode(&state).unwrap(), old, "the Default theme keeps writing the earlier format");
    }
    #[test]
    fn general_settings_roundtrip_and_defaults_keep_the_earlier_format() {
        use crate::general::{BranchSpace, Settings};
        let mut state = super::SavedState::default();
        let old = super::encode(&state).unwrap();
        assert_eq!(super::decode(&old).unwrap().preferences.general, Settings::default());
        state.preferences.general = Settings {
            source_folder: Some(r"C:\Code\仓库".into()),
            fetch_tags: rocket_git::FetchTags::All,
            branch_space: BranchSpace::Underscore,
            submodules_on_checkout: true,
            push_after_commit: true,
            diff_marks: false,
            diff_font: 16,
            diff_syntax: false,
            compact_labels: true,
        };
        let bytes = super::encode(&state).unwrap();
        assert!(bytes.starts_with(super::GENERAL_MAGIC));
        assert_eq!(super::decode(&bytes).unwrap(), state);
        let mut bad = bytes.clone();
        let font = bad.len() - 3;
        bad[font] = 40;
        assert!(super::decode(&bad).is_err(), "a font size out of range must not load");
        state.preferences.general = Settings { compact_labels: true, ..Default::default() };
        assert_eq!(super::decode(&super::encode(&state).unwrap()).unwrap(), state);
        state.preferences.general.source_folder = Some("relative".into());
        assert!(super::encode(&state).is_err(), "a partial path must not be saved");
        state.preferences.general = Settings::default();
        assert_eq!(super::encode(&state).unwrap(), old, "the defaults keep writing the earlier format");
    }
    #[test]
    fn shell_setting_roundtrips_and_the_default_keeps_the_earlier_format() {
        use crate::terminal_settings::{Choice, Settings};
        let mut state = super::SavedState::default();
        let old = super::encode(&state).unwrap();
        assert_eq!(super::decode(&old).unwrap().preferences.shell, Settings::default());
        state.preferences.shell = Settings {
            choice: Choice::Custom,
            program: r"C:\工具\zsh.exe".into(),
            arguments: "--login\n-i".into(),
        };
        let bytes = super::encode(&state).unwrap();
        assert!(bytes.starts_with(super::SHELL_MAGIC));
        assert_eq!(super::decode(&bytes).unwrap(), state);
        let mut bad = bytes.clone();
        let choice = bad.len() - "--login\n-i".len() - r"C:\工具\zsh.exe".len() - 2 * 4 - 1;
        assert_eq!(bad[choice], 4);
        bad[choice] = 9;
        assert!(super::decode(&bad).is_err(), "an unknown shell must not load");
        state.preferences.shell = Settings { choice: Choice::GitBash, ..Default::default() };
        assert_eq!(super::decode(&super::encode(&state).unwrap()).unwrap(), state);
        state.preferences.shell.program = "line\nbreak".into();
        assert!(super::encode(&state).is_err(), "a program with a line break must not be saved");
        state.preferences.shell = Settings::default();
        assert_eq!(super::encode(&state).unwrap(), old, "the default keeps writing the earlier format");
    }
    #[test]
    fn history_order_roundtrips_and_old_profiles_default_to_topological() {
        use rocket_git::HistoryOrder;
        let mut state=super::SavedState::default();
        let path=std::path::PathBuf::from("C:/Rocket fixtures/資料");
        state.worktree_views.insert(path.clone(),Default::default());
        let old=super::encode(&state).unwrap();
        assert_eq!(super::decode(&old).unwrap().worktree_views[&path].history_order,HistoryOrder::Topological);
        for order in [HistoryOrder::CommitDate,HistoryOrder::AuthorDate] {
            state.worktree_views.get_mut(&path).unwrap().history_order=order;
            let bytes=super::encode(&state).unwrap();assert!(bytes.starts_with(super::ORDER_MAGIC));
            assert_eq!(super::decode(&bytes).unwrap().worktree_views[&path].history_order,order);
            assert!(super::decode(&bytes[..bytes.len()-1]).is_err());
            let mut bad=bytes;*bad.last_mut().unwrap()=9;assert!(super::decode(&bad).is_err());
        }
    }
    use super::*;

    #[test]
    fn edits_during_autosave_keep_the_latest_draft_after_close() {
        use crate::{app::Rocket, batch::flow_tests::wait, test_common as common};
        use eframe::egui::Context;
        use rocket_git::MAX_COMMIT_MESSAGE_BYTES;
        use std::{thread, time::{Duration, Instant}};
        common::isolated(
            "ROCKET_DRAFT_SNAPSHOT_CASE",
            "persistence::tests::edits_during_autosave_keep_the_latest_draft_after_close",
            || {
                let f = common::Fixture::new();
                f.write("a", "base");
                f.commit("base");
                let ctx = Context::default();
                let directory = f.root.join("saved");
                let mut app = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs"), directory.clone());
                wait(&ctx, &mut app);
                let path = app.tabs[0].path.clone();
                app.tabs[0].draft = "x".repeat(MAX_COMMIT_MESSAGE_BYTES);
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    app.poll(&ctx);
                    if app.saving() { break; }
                    assert!(Instant::now() < deadline, "autosave did not start");
                    thread::sleep(Duration::from_millis(10));
                }
                // The queued save owns the previous snapshot. An edit and
                // immediate tab close must persist the newer value as well.
                app.tabs[0].draft.pop();
                app.tabs[0].draft.push('y');
                let id = app.tabs[0].id;
                app.remove_tab(id);
                app.begin_quit();
                wait(&ctx, &mut app);
                assert!(app.state_error.is_none(), "{:?}", app.state_error);
                drop(app);
                let (_, state) = Store::open(&directory).unwrap();
                let text = &state.drafts[&path];
                assert_eq!(text.len(), MAX_COMMIT_MESSAGE_BYTES);
                assert!(text.bytes().take(MAX_COMMIT_MESSAGE_BYTES - 1).all(|b| b == b'x'));
                assert!(text.ends_with('y'));
            },
        );
    }

    #[test]
    fn saved_draft_snapshots_share_text_and_preserve_previous_content() {
        let path = PathBuf::from("retained-repository");
        let mut state = SavedState::default();
        state.drafts.insert(path.clone(), "x".repeat(1024 * 1024).into());
        let snapshot = state.clone();
        assert_eq!(state.drafts[&path].as_ptr(), snapshot.drafts[&path].as_ptr(),
            "preparing an autosave must not copy retained draft bytes");
        assert_eq!(state, snapshot);
        assert!(!state.set_draft(&path, &snapshot.drafts[&path], &snapshot));
        assert_eq!(state.drafts[&path].as_ptr(), snapshot.drafts[&path].as_ptr());
        assert!(state.set_draft(&path, "new draft", &snapshot));
        assert_ne!(state, snapshot);
        assert_eq!(snapshot.drafts[&path].len(), 1024 * 1024);
        assert!(snapshot.drafts[&path].bytes().all(|b| b == b'x'));
        state.drafts.insert(path.clone(), "x".repeat(1024 * 1024).into());
        assert_eq!(state, snapshot, "independently allocated equal text still compares by value");
        assert!(state.set_draft(&path, "changed again", &snapshot));
        assert!(state.set_draft(&path, &"x".repeat(1024 * 1024), &snapshot));
        assert_eq!(state.drafts[&path].as_ptr(), snapshot.drafts[&path].as_ptr(),
            "reverting an edit must reuse the written draft allocation");
        assert!(state.set_draft(&path, "", &snapshot));
        assert_eq!(snapshot.drafts[&path].len(), 1024 * 1024);
    }

    #[test]
    fn view_retention_prioritizes_active_open_and_recent_paths_without_touching_drafts() {
        let root = std::env::temp_dir().join("rocket-view-retention");
        let paths: Vec<_> = (0..WORKTREE_VIEWS + 44).map(|i| root.join(format!("repo-{i:03}"))).collect();
        let mut state = SavedState::default();
        for path in &paths {
            state.worktree_views.insert(path.clone(), Default::default());
        }
        state.drafts.insert(paths[1].clone(), "keep this draft".into());
        let missing = root.join("not-loaded");
        let preferred = [&paths[299], &paths[299], &missing, &paths[0]];
        assert!(state.retain_worktree_views(preferred.into_iter().chain(paths.iter().rev())));
        assert_eq!(state.worktree_views.len(), WORKTREE_VIEWS);
        assert!(state.worktree_views.contains_key(&paths[299]));
        assert!(state.worktree_views.contains_key(&paths[0]));
        assert_eq!(state.drafts[&paths[1]].as_ref(), "keep this draft");
        assert!(!state.worktree_views.contains_key(&missing));
        assert!(!state.retain_worktree_views(paths.iter()));
    }
    #[test]
    fn hosting_recent_searches_round_trip_with_limits_and_old_state() {
        let mut state=SavedState::default();
        assert!(decode(&encode(&state).unwrap()).unwrap().hosting_searches.is_empty());
        let recent=crate::forge::search::Recent {server:"https://github.com".into(),repository:"team/資料".into(),kind:rocket_git::ForgeKind::Issues,state:rocket_git::ForgeState::Closed,search:rocket_git::ForgeSearch {text:"文".into(),author:"octocat".into(),assignee:String::new()}};
        state.hosting_searches.push(recent.clone());
        let bytes=encode(&state).unwrap();assert_eq!(decode(&bytes).unwrap(),state);
        for n in 0..bytes.len(){assert!(decode(&bytes[..n]).is_err());}
        state.hosting_searches=vec![recent;13];assert!(encode(&state).is_err());
        state.hosting_searches.truncate(1);state.hosting_searches[0].search.text="repo:other".into();assert!(encode(&state).is_err());
    }
    #[test]
    fn dependency_warnings_default_on_migrate_v12_and_persist_off() {
        let mut state = SavedState::default();
        assert!(state.preferences.dependency_warnings);
        state.preferences.secret_protection = false;
        let mut legacy = encode(&state).unwrap();
        legacy.pop(); legacy[MAGIC.len() - 1] = 12;
        let migrated = decode(&legacy).unwrap();
        assert!(migrated.preferences.dependency_warnings);
        assert!(!migrated.preferences.secret_protection);
        state.preferences.dependency_warnings = false;
        assert_eq!(decode(&encode(&state).unwrap()).unwrap(), state);
        let mut invalid = encode(&state).unwrap(); *invalid.last_mut().unwrap() = 2;
        assert!(decode(&invalid).is_err());
    }
    #[test]
    fn secret_protection_defaults_on_migrates_v11_and_persists_explicit_opt_out() {
        let mut state = SavedState::default();
        assert!(state.preferences.secret_protection);
        let mut legacy = encode(&state).unwrap();
        legacy.pop(); // v13 dependency-warning preference
        legacy.pop();
        legacy[MAGIC.len() - 1] = 11;
        assert!(decode(&legacy).unwrap().preferences.secret_protection);
        state.preferences.secret_protection = false;
        let bytes = encode(&state).unwrap();
        assert_eq!(decode(&bytes).unwrap(), state);
        let mut invalid_boolean = bytes.clone();
        let secret_index = invalid_boolean.len() - 2;
        invalid_boolean[secret_index] = 2;
        assert!(decode(&invalid_boolean).is_err());
        state.preferences.secret_protection = true;
        assert!(decode(&encode(&state).unwrap()).unwrap().preferences.secret_protection);
    }

    #[test]
    fn ai_key_guard_migrates_old_profiles_and_persists_independently() {
        for secrets in [false,true] {
            for push in [false,true] {
                let mut state = SavedState::default();
                state.preferences.secret_protection = secrets;
                state.preferences.dependency_before_push = push;
                let legacy = encode(&state).unwrap();
                assert_ne!(&legacy[..MAGIC.len()], AI_KEYS_MAGIC);
                let restored = decode(&legacy).unwrap();
                assert!(restored.preferences.guard_ai_api_keys);
                assert_eq!(restored.preferences.secret_protection,secrets);
                state.preferences.guard_ai_api_keys = false;
                let bytes = encode(&state).unwrap();
                assert_eq!(&bytes[..MAGIC.len()],AI_KEYS_MAGIC);
                assert_eq!(decode(&bytes).unwrap(),state);
                for length in 0..bytes.len() { assert!(decode(&bytes[..length]).is_err()); }
                let mut invalid = bytes.clone(); *invalid.last_mut().unwrap()=2;
                assert!(decode(&invalid).is_err());
                let mut old_version=bytes; old_version[MAGIC.len()-1]=24;
                assert!(decode(&old_version).is_err(),"the new field cannot be silently ignored");
                state.preferences.guard_ai_api_keys=true;
                assert_eq!(decode(&encode(&state).unwrap()).unwrap(),state);
            }
        }
    }
    #[test]
    fn tool_profiles_round_trip_and_reject_truncation_or_invalid_defaults() {
        let mut state = SavedState::default();
        state.drafts.insert(PathBuf::from("D:/保持"), "Draft stays".into());
        let mut profiles = crate::tool_profiles::Profiles::default();
        profiles.0[2].add(rocket_git::ExternalToolKind::Merge, Some(0));
        profiles.0[2].default = 1;
        profiles.0[2].entries[1].name = "比較 🚀".into();
        profiles.0[2].entries[1].config.executable = "D:/Tools 文/merge.exe".into();
        state.tools = profiles.defaults();
        state.tool_profiles = Some(profiles);
        let bytes = encode(&state).unwrap();
        assert_eq!(decode(&bytes).unwrap(), state);
        for n in 0..bytes.len() { assert!(decode(&bytes[..n]).is_err()); }
        state.tool_profiles.as_mut().unwrap().0[2].default = 99;
        assert!(encode(&state).is_err());
    }
    #[test]
    fn legacy_v9_tool_configuration_migrates_exactly() {
        let mut state = SavedState::default();
        state.tools[1] = rocket_git::ExternalToolConfig {
            executable: "E:/Tools 文/diff.exe".into(), arguments: "--wait\n{left}\n{right}".into(),
        };
        let mut bytes = encode(&state).unwrap();
        bytes.pop(); // v13 dependency-warning preference is absent from legacy fixtures.
        bytes.truncate(bytes.len() - 1 - 1 - 4 - 4 - crate::command_definitions::encode(&state.commands).unwrap().len());
        bytes[MAGIC.len()-1] = 9;
        let loaded = decode(&bytes).unwrap();
        let profiles = crate::tool_profiles::Profiles::from_legacy(&loaded.tools);
        assert_eq!(profiles.defaults()[1], state.tools[1]);
        assert_eq!(profiles.0[1].entries[0].name, "Default");
    }
#[test]
    fn scoped_commands_persist_without_losing_legacy_definitions() {
        let mut state=SavedState::default();
        for scope in rocket_git::CommandScope::ALL {
            state.commands.push(rocket_git::CustomCommand{name:scope.label().into(),program:if cfg!(windows){"C:/Tools/git.exe"}else{"/usr/bin/git"}.into(),arguments:"--version".into(),scope,..Default::default()});
        }
        let bytes=encode(&state).unwrap();assert!(bytes.starts_with(SCOPED_MAGIC));assert_eq!(decode(&bytes).unwrap(),state);
        let legacy=SavedState{commands:vec![rocket_git::CustomCommand{scope:rocket_git::CommandScope::Repository,..state.commands[0].clone()}],..Default::default()};
        assert_eq!(decode(&encode(&legacy).unwrap()).unwrap(),legacy);
    }
    #[test]
    fn url_commands_require_new_state_version_and_preserve_groups() {
        let mut state=SavedState::default();state.commands.push(rocket_git::CustomCommand{action:rocket_git::CommandAction::Url,name:"Review".into(),scope:rocket_git::CommandScope::File,program:"https://example.invalid/?file={path}".into(),group:"Tools / Review".into(),..Default::default()});
        let mut bytes=encode(&state).unwrap();assert!(bytes.starts_with(URL_MAGIC));assert_eq!(decode(&bytes).unwrap(),state);
        bytes[URL_MAGIC.len()-1]=15;assert!(decode(&bytes).is_err());
    }
    #[test]
    fn worktree_views_and_command_forms_round_trip_independently_of_drafts() {
        let a=PathBuf::from("C:/work/日本語");
        let b=PathBuf::from("C:/work/feature 文");
        let mut state=SavedState::default();
        let view=crate::worktree_view::Settings {branch_filter:"feature/文".into(),history_all:true,
            hide_tags:true,sections:[true,false,true,false],..Default::default()};
        state.worktree_views.insert(a.clone(),view.clone());
        let mut inherited=view;inherited.hide_stashes=true;
        state.worktree_views.insert(b.clone(),inherited);
        state.drafts.insert(a.clone(),"draft A".into());state.drafts.insert(b.clone(),"draft B".into());
        state.commands.push(rocket_git::CustomCommand {name:"Form".into(),program:"C:/Tools/git.exe".into(),group:"Review".into(),arguments:"log\n{input:base}".into(),inputs:vec![
            rocket_git::CommandInput {key:"base".into(),label:"Base".into(),kind:rocket_git::CommandInputKind::Branch,required:true,..Default::default()}
        ],..Default::default()});
        let bytes=encode(&state).unwrap();
        let restored=decode(&bytes).unwrap();assert_eq!(restored,state);
        assert!(!restored.worktree_views[&a].hide_stashes);assert!(restored.worktree_views[&b].hide_stashes);
        state.worktree_views.get_mut(&a).unwrap().history_ref=Some("--all".into());
        assert!(encode(&state).is_err());
        assert_eq!(decode(&bytes).unwrap().drafts[&b].as_ref(),"draft B");
    }
    #[test]
    fn old_settings_migrate_without_losing_drafts_and_tools_round_trip() {
        let mut old = SavedState::default();
        old.recent.push(PathBuf::from("C:/repository 文"));
        old.drafts
            .insert(old.recent[0].clone(), "Retained draft 🚀".into());
        let mut bytes = encode(&old).unwrap();
        bytes.pop(); // Remove the new v13 preference before constructing older layouts.
        // v10 has no worktree views.
        bytes.truncate(bytes.len() - 1 - 4 - 4 - crate::command_definitions::encode(&old.commands).unwrap().len());
        bytes[MAGIC.len() - 1] = 10;
        assert_eq!(decode(&bytes).unwrap(), old);
        // v9 has no profile-presence flag. Its exact tools and drafts are preserved.
        bytes.truncate(bytes.len() - 1);
        bytes[MAGIC.len() - 1] = 9;
        assert_eq!(decode(&bytes).unwrap(), old);
        // v8 has no drafting CLI choice or Codex settings (two flags and ""); Claude Code
        // stays the CLI and Codex still asks first.
        bytes.truncate(bytes.len() - 6);
        bytes[MAGIC.len() - 1] = 8;
        assert_eq!(decode(&bytes).unwrap(), old);
        // v7 has no drafting section (three flags, "" and "English"); drafting stays off.
        bytes.truncate(bytes.len() - 18);
        bytes[MAGIC.len() - 1] = 7;
        assert_eq!(decode(&bytes).unwrap(), old);
        // v6 has no saved-library section (8-byte next ID + three counts).
        bytes.truncate(bytes.len() - 20);
        bytes[MAGIC.len() - 1] = 6;
        assert_eq!(decode(&bytes).unwrap(), old);
        // v5: no repository-list or spell-check byte; both stay off.
        let mut v5 = bytes.clone();
        v5[MAGIC.len() - 1] = 5;
        v5.truncate(v5.len() - 2);
        assert_eq!(decode(&v5).unwrap(), old);
        // v4: no automatic-fetch interval (2 bytes) or commands count (4 bytes) either.
        let mut v4 = bytes.clone();
        v4[MAGIC.len() - 1] = 4;
        v4.truncate(v4.len() - 8);
        assert_eq!(decode(&v4).unwrap(), old);
        // v3: no trailing recent-messages count either.
        let mut v3 = bytes.clone();
        v3[MAGIC.len() - 1] = 3;
        v3.truncate(v3.len() - 12);
        assert_eq!(decode(&v3).unwrap(), old);
        // v2: no sidebar-labels byte either; labels stay on.
        let mut v2 = bytes.clone();
        v2[MAGIC.len() - 1] = 2;
        v2.truncate(v2.len() - 13);
        assert_eq!(decode(&v2).unwrap(), old);
        bytes[MAGIC.len() - 1] = 1;
        // Three empty tool pairs, the labels byte, the messages count, v5 and v6.
        bytes.truncate(bytes.len() - 37);
        assert_eq!(decode(&bytes).unwrap(), old);
        old.preferences.sidebar_labels = false;
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        old.tools[2] = rocket_git::ExternalToolConfig {
            executable: "C:/Tools 文/merge.exe".into(),
            arguments: "{base}\n{ours}\n{theirs}\n{result}".into(),
        };
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        old.recent_messages.insert(
            old.recent[0].clone(),
            vec!["second\n\nbody".into(), "first 文".into()],
        );
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        old.preferences.auto_fetch_minutes = 15;
        old.commands.push(rocket_git::CustomCommand {
            name: "Open 文".into(),
            program: "C:/Tools 文/editor.exe".into(),
            arguments: "--path={file}\n{repo}".into(),
            ..Default::default()
        });
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        old.preferences.repository_list = true;
        old.preferences.spell_check = true;
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        old.preferences.drafting = crate::commit_draft::Settings {
            enabled: true,
            provider: rocket_git::DraftProvider::Claude,
            notice_accepted: true,
            codex_notice_accepted: false,
            style: rocket_git::DraftStyle::Conventional,
            executable: "C:/Tools 文/claude.exe".into(),
            codex_executable: String::new(),
            language: "Deutsch".into(),
        };
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
        let drafting = &mut old.preferences.drafting;
        drafting.provider = rocket_git::DraftProvider::Codex;
        drafting.codex_notice_accepted = true;
        drafting.codex_executable = "D:/Codex 文/codex.exe".into();
        assert_eq!(decode(&encode(&old).unwrap()).unwrap(), old);
    }
    #[test]
    fn repository_library_round_trips_and_invalid_library_preserves_existing_state() {
        let mut s = SavedState::default();
        let folder = s.library.add_folder("研究 🚀".into()).unwrap();
        let a = s
            .library
            .save_repository(
                PathBuf::from("D:/Projects/日本語"),
                "日本語".into(),
                Some(folder),
            )
            .unwrap();
        s.library
            .save_workspace(None, "Daily work".into(), vec![a], Some(a))
            .unwrap();
        s.drafts
            .insert(PathBuf::from("D:/Projects/日本語"), "Keep my draft".into());
        let bytes = encode(&s).unwrap();
        assert_eq!(decode(&bytes).unwrap(), s);
        for n in 0..bytes.len() {
            assert!(decode(&bytes[..n]).is_err());
        }
        let dir = std::env::temp_dir().join(format!(
            "rocket-library-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (store, _) = Store::open(&dir).unwrap();
        store.save(&s).unwrap();
        let stored = fs::read(dir.join("state.bin")).unwrap();
        s.library.workspaces[0].repositories.push(999);
        assert!(store.save(&s).is_err());
        assert_eq!(fs::read(dir.join("state.bin")).unwrap(), stored);
        assert!(Store::open(&dir).is_err());
        drop(store);
        assert_eq!(
            Store::open(&dir).unwrap().1.library.repositories[0].name,
            "日本語"
        );
    }
    #[test]
    fn automatic_fetch_and_commands_stay_within_their_limits() {
        let mut state = SavedState::default();
        state.preferences.auto_fetch_minutes = 7;
        assert!(encode(&state).is_err());
        state.preferences.auto_fetch_minutes = 60;
        let command = rocket_git::CustomCommand {
            name: "Status".into(),
            program: "C:/Tools/git.exe".into(),
            arguments: "status".into(),
            ..Default::default()
        };
        state.commands = vec![command.clone(); rocket_git::MAX_CUSTOM_COMMANDS + 1];
        assert!(encode(&state).is_err());
        state.commands = vec![rocket_git::CustomCommand {
            program: "relative.exe".into(),
            ..command.clone()
        }];
        assert!(encode(&state).is_err());
        state.commands = vec![command];
        assert_eq!(decode(&encode(&state).unwrap()).unwrap(), state);
    }
    #[test]
    fn recent_messages_stay_within_their_limits() {
        let path = PathBuf::from("C:/repository 文");
        let mut state = SavedState::default();
        for messages in [
            Vec::new(),
            vec![String::new()],
            vec!["x".repeat(crate::composer::RECENT_MESSAGE_BYTES + 1)],
            vec!["m".to_owned(); crate::composer::RECENT_MESSAGES + 1],
        ] {
            state.recent_messages.insert(path.clone(), messages);
            assert!(encode(&state).is_err());
        }
        state.recent_messages.clear();
        for i in 0..=MESSAGE_REPOSITORIES {
            state
                .recent_messages
                .insert(PathBuf::from(format!("C:/r{i}")), vec!["m".into()]);
        }
        assert!(encode(&state).is_err());
        state.recent_messages.pop_first();
        assert_eq!(decode(&encode(&state).unwrap()).unwrap(), state);
    }
    #[test]
    fn versioned_state_rejects_truncation_invalid_values_and_trailing_data() {
        let mut state = SavedState::default();
        state.recent.push(PathBuf::from("C:/fixture 文"));
        state
            .drafts
            .insert(state.recent[0].clone(), "message\n\n文".into());
        let bytes = encode(&state).unwrap();
        assert_eq!(decode(&bytes).unwrap(), state);
        for end in 0..bytes.len() {
            assert!(
                decode(&bytes[..end]).is_err(),
                "accepted truncated input {end}"
            );
        }
        let mut invalid_version = bytes.clone();
        invalid_version[MAGIC.len() - 1] = 99;
        assert!(decode(&invalid_version).is_err());
        let mut invalid_boolean = bytes.clone();
        invalid_boolean[MAGIC.len()] = 2;
        assert!(decode(&invalid_boolean).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        state.preferences.zoom = f32::NAN;
        assert!(encode(&state).is_err());
    }
}

fn encode_library(bytes: &mut Vec<u8>, l: &crate::repository_library::Library) -> io::Result<()> {
    l.validate().map_err(|_| invalid())?;
    bytes.extend(l.next_id.to_le_bytes());
    bytes.extend((l.folders.len() as u32).to_le_bytes());
    for f in &l.folders {
        bytes.extend(f.id.to_le_bytes());
        put(bytes, f.name.as_bytes())?;
    }
    bytes.extend((l.repositories.len() as u32).to_le_bytes());
    for r in &l.repositories {
        bytes.extend(r.id.to_le_bytes());
        put(bytes, r.name.as_bytes())?;
        put(bytes, &path_bytes(&r.path))?;
        bytes.extend(r.folder.unwrap_or(0).to_le_bytes());
    }
    bytes.extend((l.workspaces.len() as u32).to_le_bytes());
    for w in &l.workspaces {
        bytes.extend(w.id.to_le_bytes());
        put(bytes, w.name.as_bytes())?;
        bytes.extend((w.repositories.len() as u32).to_le_bytes());
        for id in &w.repositories {
            bytes.extend(id.to_le_bytes());
        }
        bytes.extend(w.active.unwrap_or(0).to_le_bytes());
    }
    Ok(())
}
fn decode_library(r: &mut Reader<'_>, nested_workspaces: bool) -> io::Result<crate::repository_library::Library> {
    use crate::repository_library::*;
    fn id(r: &mut Reader<'_>) -> io::Result<u64> {
        Ok(u64::from_le_bytes(r.take(8)?.try_into().unwrap()))
    }
    fn count(r: &mut Reader<'_>, max: usize) -> io::Result<usize> {
        let n = r.u32()? as usize;
        if n > max { Err(invalid()) } else { Ok(n) }
    }
    fn text(r: &mut Reader<'_>) -> io::Result<String> {
        Ok(std::str::from_utf8(r.bytes()?)
            .map_err(|_| invalid())?
            .to_owned())
    }
    let mut l = Library {
        next_id: id(r)?,
        ..Default::default()
    };
    for _ in 0..count(r, MAX_FOLDERS)? {
        l.folders.push(Folder {
            id: id(r)?,
            name: text(r)?,
        });
    }
    for _ in 0..count(r, MAX_REPOSITORIES)? {
        let (id, name, path, folder) = (id(r)?, text(r)?, r.path()?, id(r)?);
        l.repositories.push(SavedRepository {
            id,
            name,
            path,
            folder: (folder != 0).then_some(folder),
        });
    }
    for _ in 0..count(r, MAX_WORKSPACES)? {
        let (wid, name) = (id(r)?, text(r)?);
        let mut repositories = Vec::new();
        for _ in 0..count(r, MAX_WORKSPACE_REPOSITORIES)? {
            repositories.push(id(r)?);
        }
        let active = id(r)?;
        l.workspaces.push(Workspace {
            id: wid,
            name,
            parent: None,
            repositories,
            active: (active != 0).then_some(active),
        });
    }
    // v21 may repeat a name under different parents, supplied later in the file.
    // decode() validates the complete state, including those parent records.
    if !nested_workspaces { l.validate().map_err(|_| invalid())?; }
    Ok(l)
}
