//! Commands the user defines: a program and literal arguments, run in the
//! repository folder after the user confirms the exact command line. No shell
//! parses anything; a shell runs only when the user chose one as the program.
use crate::{process_tree::ProcessTree, runner::git_path, *};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Most custom commands Rocket keeps.
pub const MAX_CUSTOM_COMMANDS: usize = 20;
/// Existing execution limit, also enforced while expanding untrusted defaults.
pub const CUSTOM_ARGUMENT_LIMIT: usize = 32000;
/// Output kept per stream; the rest is not read.
pub const CUSTOM_OUTPUT_LIMIT: usize = 64 * 1024;
/// A command writing more than this to either stream is stopped.
const OUTPUT_STOP: u64 = 16 * 1024 * 1024;
/// Values a command line can use, each replaced by the current selection.
pub const COMMAND_PLACEHOLDERS: [&str; 6] = ["{repo}", "{branch}", "{commit}", "{file}", "{submodule}", "{path}"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandAction { #[default] Program, Url }
impl CommandAction {
    pub fn key(self) -> &'static str { match self {Self::Program=>"program",Self::Url=>"url"} }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandScope {
    #[default]
    Repository,
    Commit,
    File,
    Branch,
    Submodule,
}
impl CommandScope {
    pub const ALL: [Self; 5] = [Self::Repository, Self::Commit, Self::File, Self::Branch, Self::Submodule];
    pub fn key(self) -> &'static str {
        match self { Self::Repository => "repository", Self::Commit => "commit", Self::File => "file", Self::Branch => "branch", Self::Submodule => "submodule" }
    }
    pub fn label(self) -> &'static str {
        match self { Self::Repository => "Repository", Self::Commit => "Commit", Self::File => "File", Self::Branch => "Branch", Self::Submodule => "Submodule" }
    }
    pub fn from_key(key: &str) -> Option<Self> { Self::ALL.into_iter().find(|s| s.key() == key) }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomCommand {
    pub action: CommandAction,
    /// Which selection's menu offers this command. Older definitions are repository commands.
    pub scope: CommandScope,
    pub name: String,
    /// Absolute program path (an .exe on Windows), or the fixed-host HTTPS template.
    pub program: String,
    /// One argument per line, without quoting.
    pub arguments: String,
    pub group: String,
    pub inputs: Vec<CommandInput>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandInputKind {
    #[default]
    Text,
    Branch,
    File,
    Checkbox,
}
impl CommandInputKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Branch => "branch",
            Self::File => "file",
            Self::Checkbox => "checkbox",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandInput {
    pub key: String,
    pub label: String,
    pub kind: CommandInputKind,
    pub required: bool,
    pub default_value: String,
    /// A checkbox occupies a whole argument line; unchecked omits that argument.
    pub checked_value: String,
}
fn input_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 32
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn input_text(value: &str) -> bool {
    value.len() <= 4096 && !value.chars().any(crate::display_text::hidden)
}

/// The selection a command line is expanded for.
#[derive(Clone, Debug, Default)]
pub struct CommandContext {
    pub repository: PathBuf,
    /// The selected branch, or the checked-out branch in repository context.
    pub branch: Option<String>,
    pub commit: Option<ObjectId>,
    /// Absolute path of the selected file.
    pub file: Option<PathBuf>,
    /// Absolute folder of the selected initialized submodule.
    pub submodule: Option<PathBuf>,
    /// The file belongs to a committed revision, while {file} still names its working-tree path.
    pub historical_file: bool,
}

fn definition(reason: &str) -> Error {
    Error::InvalidInput(format!("custom command: {reason}"))
}

/// Lexical policy only: no filesystem access, network lookup or executable launch.
/// Windows drive paths (including extended local drive paths) are allowed; UNC,
/// device namespaces and alternate streams are not. Mapped drives and junctions
/// are resolved only after an explicit program check or confirmed execution.
pub fn valid_custom_program_path(path: &Path) -> bool {
    let Some(text) = path.to_str() else { return false };
    if text.len() > 32768 || text.trim() != text || !path.is_absolute()
        || text.chars().any(crate::display_text::hidden) {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(p))
            if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            || !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe"))
            || path.components().any(|c| matches!(c, Component::Normal(s) if s.to_string_lossy().contains(':'))) {
            return false;
        }
    }
    true
}

/// Full visible representation for command review. Never use it as an argument.
/// The tenfold budget covers every Unicode escape without shortening the review.
pub fn custom_command_display(value: &str) -> String {
    crate::display_text::bounded(value, value.len().saturating_mul(10)).text
}

impl CustomCommand {
    /// Checks the definition only; it never resolves or runs the program.
    pub fn validate(&self) -> Result<()> {
        let hidden = crate::display_text::hidden;
        if [&self.name, &self.group, &self.program].into_iter().any(|s| s.chars().any(hidden))
            || self.argument_lines().iter().any(|s| s.chars().any(hidden))
            || self.inputs.iter().any(|i| [&i.label, &i.default_value, &i.checked_value]
                .into_iter().any(|s| s.chars().any(hidden))) {
            return Err(definition("remove invisible control or formatting characters from the command definition"));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(definition("enter a name of 1 to 64 characters"));
        }
        if self.group.chars().count() > 64
            || self.group.chars().any(char::is_control)
            || self.inputs.len() > 8
        {
            return Err(definition(
                "use a group of at most 64 characters and at most 8 inputs",
            ));
        }
        let mut keys = std::collections::HashSet::new();
        for input in &self.inputs {
            if !input_key(&input.key)
                || !keys.insert(&input.key)
                || input.label.trim().is_empty()
                || input.label.chars().count() > 64
                || input.label.chars().any(char::is_control)
                || !input_text(&input.default_value)
                || !input_text(&input.checked_value)
                || (input.kind == CommandInputKind::Checkbox
                    && (!matches!(input.default_value.as_str(), "true" | "false")
                        || input.checked_value.is_empty()))
            {
                return Err(definition(
                    "inputs need unique letter/number/underscore keys, labels, and valid single-line defaults; checkbox defaults are true or false with a checked argument",
                ));
            }
        }
        let program = Path::new(&self.program);
        if self.action == CommandAction::Program && !valid_custom_program_path(program)
        {
            return Err(definition("choose the program by a local absolute path; network and device paths are not allowed"));
        }
        #[cfg(windows)]
        if self.action == CommandAction::Program && !program
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        {
            return Err(definition(
                "choose an .exe program; to run a script, choose its shell",
            ));
        }
        let lines = if self.action == CommandAction::Url {
            if self.program.len()>16384 {return Err(definition("URL template is too long"));}
            if !self.arguments.is_empty() || self.inputs.iter().any(|i|i.kind==CommandInputKind::Checkbox) {
                return Err(definition("URL actions use a URL template and text, branch or file inputs, without program arguments or checkboxes"));
            }
            let rest=self.program.strip_prefix("https://").ok_or_else(||definition("URL actions require an HTTPS destination"))?;
            let authority=rest.split(['/', '?', '#']).next().unwrap_or_default();
            if authority.contains(['{','}']) {return Err(definition("the URL host must be literal; placeholders belong in its path or query"));}
            let parts=split(&self.program).ok_or_else(||definition("invalid URL placeholder"))?;
            let sample=parts.iter().map(|p|match p {Part::Text(t)=>*t,_=>"x"}).collect::<String>();
            if !crate::https_url(&sample) {return Err(definition("use a plain HTTPS URL without credentials, spaces or unsafe characters"));}
            vec![self.program.as_str()]
        } else { self.argument_lines() };
        if self.arguments.len() > 16384 || self.arguments.contains('\0') || lines.len() > 64 {
            return Err(definition("enter at most 64 arguments, one per line"));
        }
        for line in lines {
            let parts = split(line).ok_or_else(|| {
                definition(
                    "braces may only form {repo}, {branch}, {commit}, {file}, {submodule}, {path} or a defined {input:key}",
                )
            })?;
            for part in parts {
                if let Part::Input(key) = part {
                    let input =
                        self.inputs.iter().find(|i| i.key == key).ok_or_else(|| {
                            definition("an argument refers to an undefined input")
                        })?;
                    if input.kind == CommandInputKind::Checkbox
                        && line != format!("{{input:{key}}}")
                    {
                        return Err(definition(
                            "a checkbox placeholder must occupy its whole argument line",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    /// The arguments, one per non-empty line.
    pub fn argument_lines(&self) -> Vec<&str> {
        self.arguments
            .lines()
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .filter(|l| !l.is_empty())
            .collect()
    }
    /// The placeholders the arguments use, in [`COMMAND_PLACEHOLDERS`] order.
    pub fn placeholders(&self) -> Vec<&'static str> {
        let used: Vec<&str> = (if self.action==CommandAction::Url {vec![self.program.as_str()]}else{self.argument_lines()})
            .into_iter()
            .filter_map(split)
            .flatten()
            .filter_map(|part| match part {
                Part::Placeholder(p) => Some(p),
                Part::Text(_) | Part::Input(_) => None,
            })
            .collect();
        COMMAND_PLACEHOLDERS
            .into_iter()
            .filter(|p| used.contains(p))
            .collect()
    }
    /// The exact arguments for `context`, or the selection value that is missing.
    pub fn expand(&self, context: &CommandContext) -> std::result::Result<Vec<OsString>, String> {
        self.expand_with_values(context, &BTreeMap::new())
    }
    pub fn expand_with_values(
        &self,
        context: &CommandContext,
        values: &BTreeMap<String, String>,
    ) -> std::result::Result<Vec<OsString>, String> {
        self.validate().map_err(|e| e.to_string())?;
        if self.action!=CommandAction::Program {return Err("URL actions must be reviewed and opened in the browser.".into());}
        check_selection(self,context,values)?;
        let mut arguments = Vec::new();
        let mut total = 0;
        'lines: for line in self.argument_lines() {
            let mut argument = OsString::new();
            for part in split(line).ok_or("invalid placeholder")? {
                match part {
                    Part::Input(key) => {
                        let input = self
                            .inputs
                            .iter()
                            .find(|i| i.key == key)
                            .ok_or("undefined input")?;
                        let value = values.get(key).unwrap_or(&input.default_value);
                        if !input_text(value) {
                            return Err(format!(
                                "{} must be a single line, at most 4096 bytes.",
                                input.label
                            ));
                        }
                        if input.kind == CommandInputKind::Checkbox {
                            match value.as_str() {
                                "false" => continue 'lines,
                                "true" => argument.push(&input.checked_value),
                                _ => {
                                    return Err(format!(
                                        "{} must be checked or unchecked.",
                                        input.label
                                    ));
                                }
                            }
                        } else {
                            if input.required && value.is_empty() {
                                return Err(format!("Enter {}.", input.label));
                            }
                            argument.push(value);
                        }
                    }
                    Part::Text(text) => argument.push(text),
                    // Plain paths: many programs cannot open `\\?\` verbatim ones.
                    Part::Placeholder("{repo}") => argument.push(git_path(&context.repository)),
                    Part::Placeholder("{branch}") => argument.push(
                        context
                            .branch
                            .as_deref()
                            .ok_or("{branch} needs a checked-out branch")?,
                    ),
                    Part::Placeholder("{commit}") => argument.push(
                        context
                            .commit
                            .as_ref()
                            .ok_or("{commit} needs a selected commit")?
                            .as_str(),
                    ),
                    Part::Placeholder("{submodule}") => argument.push(git_path(
                        context.submodule.as_ref().ok_or("{submodule} needs an initialized submodule")?,
                    )),
                    Part::Placeholder("{path}") => argument.push(relative_file(context)?),
                    Part::Placeholder(_) => argument.push(git_path(
                        context
                            .file
                            .as_ref()
                            .ok_or("{file} needs a selected file")?,
                    )),
                }
                if total + argument.len() + 1 > CUSTOM_ARGUMENT_LIMIT {
                    return Err("The expanded command line exceeds 32000 bytes.".into());
                }
            }
            total += argument.len() + 1;
            arguments.push(argument);
        }
        Ok(arguments)
    }
    /// Expand each substituted value as one URL component. The host is always literal.
    pub fn expand_url(&self, context: &CommandContext, values: &BTreeMap<String,String>) -> std::result::Result<String,String> {
        self.validate().map_err(|e|e.to_string())?;
        if self.action!=CommandAction::Url {return Err("This command runs a program.".into());}
        check_selection(self,context,values)?;
        let mut url=String::new();
        for part in split(&self.program).ok_or("Invalid URL template")? {
            let value=match part {
                Part::Text(text)=>{url.push_str(text);continue;}
                Part::Input(key)=>{
                    let input=self.inputs.iter().find(|i|i.key==key).ok_or("Unknown URL input")?;
                    let value=values.get(key).unwrap_or(&input.default_value);
                    if !input_text(value) || (input.required&&value.is_empty()) {return Err(format!("Enter a valid {}.",input.label));}
                    value.clone()
                }
                Part::Placeholder("{repo}")=>git_path(&context.repository).to_str().ok_or("Repository path is not Unicode")?.into(),
                Part::Placeholder("{branch}")=>context.branch.clone().ok_or("Select a branch")?,
                Part::Placeholder("{commit}")=>context.commit.as_ref().ok_or("Select a commit")?.to_string(),
                Part::Placeholder("{file}")=>git_path(context.file.as_ref().ok_or("Select a file")?).to_str().ok_or("File path is not Unicode")?.into(),
                Part::Placeholder("{submodule}")=>git_path(context.submodule.as_ref().ok_or("Select a submodule")?).to_str().ok_or("Submodule path is not Unicode")?.into(),
                Part::Placeholder("{path}")=>relative_file(context)?,
                Part::Placeholder(_)=>return Err("Unknown URL placeholder".into()),
            };
            for b in value.bytes() {
                if b.is_ascii_alphanumeric()||b"-._~".contains(&b) {url.push(b as char)}else{use std::fmt::Write;write!(url,"%{b:02X}").unwrap();}
            }
        }
        if !crate::https_url(&url) {return Err("Expanded URL is invalid or too long.".into());}
        Ok(url)
    }
}

fn relative_file(context:&CommandContext)->std::result::Result<String,String> {
    let path=context.file.as_ref().ok_or("{path} needs a selected file")?.strip_prefix(&context.repository).map_err(|_|"Selected file is outside the repository")?;
    Ok(path.to_str().ok_or("Selected file path is not Unicode")?.replace('\\',"/"))
}
fn check_selection(command:&CustomCommand,context:&CommandContext,values:&BTreeMap<String,String>)->std::result::Result<(),String> {
    match command.scope {
        CommandScope::Commit if context.commit.is_none()=>return Err("Select a commit for this command.".into()),
        CommandScope::File if context.file.is_none()=>return Err("Select a file for this command.".into()),
        CommandScope::Branch if context.branch.is_none()=>return Err("Select a branch for this command.".into()),
        CommandScope::Submodule if context.submodule.is_none()=>return Err("Select an initialized submodule for this command.".into()),
        _=>{}
    }
    if values.keys().any(|k|!command.inputs.iter().any(|i|&i.key==k)){return Err("Unknown command input; reopen the command.".into());}
    Ok(())
}

enum Part<'a> {
    Text(&'a str),
    Placeholder(&'static str),
    Input(&'a str),
}

/// Split one argument line into text and placeholders; `None` for stray braces.
fn split(line: &str) -> Option<Vec<Part<'_>>> {
    let mut parts = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return None;
        }
        if let Some(input) = rest[open..].strip_prefix("{input:") {
            let end = input.find('}')?;
            let key = &input[..end];
            if !input_key(key) {
                return None;
            }
            if open > 0 {
                parts.push(Part::Text(&rest[..open]));
            }
            parts.push(Part::Input(key));
            rest = &input[end + 1..];
            continue;
        }
        let placeholder = COMMAND_PLACEHOLDERS
            .into_iter()
            .find(|p| rest[open..].starts_with(p))?;
        if open > 0 {
            parts.push(Part::Text(&rest[..open]));
        }
        parts.push(Part::Placeholder(placeholder));
        rest = &rest[open + placeholder.len()..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest));
    }
    Some(parts)
}

/// What running a custom command observed. The run itself never fails silently:
/// `exit` is the program's exit code, or why it did not run to completion.
#[derive(Debug)]
pub struct CustomCommandReport {
    pub exit: Result<Option<i32>>,
    /// The first [`CUSTOM_OUTPUT_LIMIT`] bytes of each stream.
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    /// Status after the command; it may have changed files.
    pub status: Result<Status>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl Repository {
    /// Run a confirmed command line in the repository folder: `program` with exactly
    /// `arguments`, no shell, no console window and standard input closed. Output
    /// goes to private files in `storage`, so a program it starts that keeps running
    /// cannot block completion. Cancellation stops the program and everything it
    /// started; otherwise programs it started keep running. Holds the repository
    /// write gate while it runs, then refreshes status.
    pub fn run_custom_command(
        &self,
        program: &Path,
        arguments: &[OsString],
        storage: &Path,
        cancel: &CancellationToken,
    ) -> Result<CustomCommandReport> {
        let _permit = self.acquire_write(cancel)?;
        self.run_command_locked(program, arguments, storage, None, cancel)
    }

    /// Caller owns the repository write permit. Bounded tests close their entire
    /// process tree on every outcome; ordinary editor commands retain that tree.
    pub(crate) fn run_command_locked(
        &self,
        program: &Path,
        arguments: &[OsString],
        storage: &Path,
        timeout: Option<Duration>,
        cancel: &CancellationToken,
    ) -> Result<CustomCommandReport> {
        cancel.check()?;
        crate::ensure_unprivileged_process()?;
        if !valid_custom_program_path(program) {
            return Err(definition("choose a local absolute executable path without hidden characters, network shares or device namespaces"));
        }
        if arguments.len() > 64
            || arguments.iter().map(|a| a.len() + 1).sum::<usize>() > CUSTOM_ARGUMENT_LIMIT
        {
            return Err(definition("the command line is too long"));
        }
        let meta = fs::symlink_metadata(program)
            .map_err(|e| Error::io("inspect the command program", e))?;
        if !meta.is_file() || crate::local_files::redirected(&meta) {
            return Err(definition("the program must be a regular file"));
        }
        let root = self
            .info()
            .worktree
            .clone()
            .ok_or(Error::WorktreeRequired)?;
        cancel.check()?;
        fs::create_dir_all(storage)
            .map_err(|e| Error::io("create the command output folder", e))?;
        crate::local_files::directory(storage)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let base = format!(
            "command-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let out_path = storage.join(format!("{base}.out"));
        let err_path = storage.join(format!("{base}.err"));
        let create = |path: &Path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| Error::io("create command output", e))
        };
        let (out, err) = (create(&out_path)?, create(&err_path)?);
        let mut command = Command::new(program);
        command
            .args(arguments)
            // cmd.exe cannot start in a `\\?\` verbatim folder.
            .current_dir(git_path(&root))
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        let starting =
            ProcessTree::prepare(&mut command).map_err(|e| Error::io("start the command", e))?;
        let exit = self.runner.step(if timeout.is_some() {"bisect-auto"} else {"custom-command"}, if timeout.is_some() {"run_test_command"} else {"run"}, || {
            let started = std::time::Instant::now();
            let mut child = command
                .spawn()
                .map_err(|e| Error::io("start the command", e))?;
            let attached = if timeout.is_some() { ProcessTree::attach(starting, &child) }
                else { ProcessTree::attach_lasting(starting, &child) };
            let tree = match attached {
                Ok(tree) => tree,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::io("contain the command", e));
                }
            };
            let size = |path: &Path| fs::metadata(path).map_or(0, |m| m.len());
            loop {
                let stop = if cancel.is_cancelled() {
                    Some(Error::Cancelled)
                } else if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                    Some(Error::TimedOut)
                } else if size(&out_path).max(size(&err_path)) > OUTPUT_STOP {
                    Some(Error::OutputLimit {
                        stream: "command",
                        limit: OUTPUT_STOP as usize,
                    })
                } else {
                    None
                };
                if let Some(error) = stop {
                    // Stop the program and everything it started; never by name.
                    if tree.terminate().is_err() {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                    if timeout.is_some() { tree.wait_stopped().map_err(|e| Error::io("wait for test processes to stop", e))?; }
                    return Err(error);
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if timeout.is_some() {
                            tree.terminate().map_err(|e| Error::io("stop test subprocesses", e))?;
                            tree.wait_stopped().map_err(|e| Error::io("wait for test processes to stop", e))?;
                        }
                        return Ok(status.code());
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(e) => {
                        let _ = tree.terminate();
                        let _ = child.wait();
                        return Err(Error::io("wait for the command", e));
                    }
                }
            }
        });
        let (stdout, out_truncated) = read_prefix(&out_path);
        let (stderr, err_truncated) = read_prefix(&err_path);
        // A program the command started may still hold a file; it stays private.
        let _ = fs::remove_file(&out_path);
        let _ = fs::remove_file(&err_path);
        let status = self.runner.observe("refresh", "status", || {
            self.status(&CancellationToken::new())
        });
        Ok(CustomCommandReport {
            exit,
            stdout,
            stderr,
            truncated: out_truncated || err_truncated,
            status,
        })
    }
}

fn read_prefix(path: &Path) -> (Vec<u8>, bool) {
    let Ok(file) = File::open(path) else {
        return (Vec::new(), false);
    };
    let mut bytes = Vec::new();
    let _ = file
        .take(CUSTOM_OUTPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes);
    let truncated = bytes.len() > CUSTOM_OUTPUT_LIMIT;
    bytes.truncate(CUSTOM_OUTPUT_LIMIT);
    (bytes, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(arguments: &str) -> CustomCommand {
        CustomCommand {
            name: "Fixture".into(),
            program: if cfg!(windows) {
                r"C:\Tools\fixture.exe".into()
            } else {
                "/usr/bin/fixture".into()
            },
            arguments: arguments.into(),
            ..Default::default()
        }
    }

    #[test]
    fn definitions_allow_only_known_placeholders_and_absolute_programs() {
        assert!(command("status\n--short").validate().is_ok());
        assert!(command("--path={file}\n{repo}").validate().is_ok());
        for bad in ["{unknown}", "{repo", "repo}", "{{repo}}"] {
            assert!(command(bad).validate().is_err(), "{bad}");
        }
        let mut relative = command("");
        relative.program = "fixture.exe".into();
        assert!(relative.validate().is_err());
        let mut unnamed = command("");
        unnamed.name = "  ".into();
        assert!(unnamed.validate().is_err());
        assert!(command(&"x\n".repeat(65)).validate().is_err());
    }
    #[test]
    fn input_forms_expand_once_and_checkbox_omits_only_its_argument() {
        let mut c =
            command("--title={input:text}\n{input:compact}\n{input:base}\n--\n{input:path}");
        c.inputs = vec![
            CommandInput {
                key: "text".into(),
                label: "Title".into(),
                required: true,
                ..Default::default()
            },
            CommandInput {
                key: "compact".into(),
                label: "Compact".into(),
                kind: CommandInputKind::Checkbox,
                default_value: "false".into(),
                checked_value: "--oneline".into(),
                ..Default::default()
            },
            CommandInput {
                key: "base".into(),
                label: "Base".into(),
                kind: CommandInputKind::Branch,
                required: true,
                ..Default::default()
            },
            CommandInput {
                key: "path".into(),
                label: "File".into(),
                kind: CommandInputKind::File,
                required: true,
                ..Default::default()
            },
        ];
        let values = BTreeMap::from([
            ("text".into(), "literal {repo} 文 & $()".into()),
            ("base".into(), "feature/検索".into()),
            ("path".into(), "C:/repo/日本語 file.txt".into()),
        ]);
        let ctx = CommandContext::default();
        let args = c.expand_with_values(&ctx, &values).unwrap();
        assert_eq!(
            args,
            [
                "--title=literal {repo} 文 & $()",
                "feature/検索",
                "--",
                "C:/repo/日本語 file.txt"
            ]
            .map(OsString::from)
        );
        let mut checked = values.clone();
        checked.insert("compact".into(), "true".into());
        assert_eq!(
            c.expand_with_values(&ctx, &checked).unwrap()[1],
            "--oneline"
        );
        checked.insert("compact".into(), "--unsafe".into());
        assert!(c.expand_with_values(&ctx, &checked).is_err());
        checked = values;
        checked.insert("text".into(), "two\nlines".into());
        assert!(c.expand_with_values(&ctx, &checked).is_err());
        c.arguments = "--option={input:compact}".into();
        assert!(c.validate().is_err());
        c.arguments = "{input:unknown}".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn expansion_keeps_each_line_one_argument_and_names_missing_values() {
        let c = command("log\n--format=%H {branch}\n{commit}\n\n{file}");
        assert_eq!(c.placeholders(), ["{branch}", "{commit}", "{file}"]);
        let mut context = CommandContext {
            repository: PathBuf::from(if cfg!(windows) {
                r"C:\repo with space"
            } else {
                "/repo with space"
            }),
            branch: Some("feature/文".into()),
            commit: None,
            file: None,
            submodule: None,
            historical_file: false,
        };
        assert_eq!(
            c.expand(&context).unwrap_err(),
            "{commit} needs a selected commit"
        );
        context.commit = Some(ObjectId::parse(&"a".repeat(40)).unwrap());
        context.file = Some(context.repository.join("a b.txt"));
        let expanded = c.expand(&context).unwrap();
        assert_eq!(expanded.len(), 4);
        assert_eq!(expanded[1], OsString::from("--format=%H feature/文"));
        assert_eq!(
            expanded[3],
            context.repository.join("a b.txt").into_os_string()
        );
        // Canonical repository paths reach the program without the verbatim prefix.
        if cfg!(windows) {
            let c = command("{repo}\n--file={file}");
            context.repository = PathBuf::from(r"\\?\C:\repo with space");
            context.file = Some(context.repository.join("a b.txt"));
            assert_eq!(
                c.expand(&context).unwrap(),
                [
                    OsString::from(r"C:\repo with space"),
                    OsString::from(r"--file=C:\repo with space\a b.txt")
                ]
            );
        }
    }

    #[test]
    fn scoped_commands_require_selection_and_keep_submodule_paths_literal() {
        let mut command=command("--version");
        let empty=CommandContext::default();
        for scope in [CommandScope::Commit,CommandScope::File,CommandScope::Branch,CommandScope::Submodule] {
            command.scope=scope;assert!(command.expand(&empty).is_err(),"{scope:?}");
        }
        command.scope=CommandScope::Submodule;command.arguments="-C\n{submodule}\nstatus\n--short".into();
        let path=PathBuf::from(if cfg!(windows){"C:/fixture/文 & $()"}else{"/fixture/文 & $()"});
        let context=CommandContext{submodule:Some(path.clone()),..Default::default()};
        let expanded=command.expand(&context).unwrap();assert_eq!(expanded.len(),4);
        assert_eq!(expanded[1],git_path(&path));assert_eq!(expanded[0],"-C");
    }
    #[test]
    fn url_commands_encode_selected_paths_and_inputs_once_without_changing_host() {
        let c=CustomCommand {action:CommandAction::Url,scope:CommandScope::File,name:"Open review".into(),program:"https://example.invalid/view/{commit}?path={path}&q={input:query}".into(),inputs:vec![CommandInput{key:"query".into(),label:"Query".into(),required:true,..Default::default()}],..Default::default()};
        let root=PathBuf::from(if cfg!(windows){"C:/fixture"}else{"/fixture"});
        let context=CommandContext{file:Some(root.join("資料/a & b.txt")),repository:root,commit:Some(ObjectId::parse(&"a".repeat(40)).unwrap()),historical_file:true,..Default::default()};
        assert!(c.expand_url(&context,&BTreeMap::new()).is_err());
        let values=BTreeMap::from([("query".into(),"{repo} &next=https://evil.invalid/%0a".into())]);
        let url=c.expand_url(&context,&values).unwrap();
        assert_eq!(url,format!("https://example.invalid/view/{}?path=%E8%B3%87%E6%96%99%2Fa%20%26%20b.txt&q=%7Brepo%7D%20%26next%3Dhttps%3A%2F%2Fevil.invalid%2F%250a","a".repeat(40)));
        assert!(c.expand_with_values(&context,&values).is_err());
        assert_eq!(command("{path}").expand(&context).unwrap(),[OsString::from("資料/a & b.txt")]);
        let mut outside=context.clone();outside.file=Some(PathBuf::from(if cfg!(windows){"D:/other"}else{"/other"}));
        assert!(c.expand_url(&outside,&values).is_err());
        let mut unknown=values.clone();unknown.insert("extra".into(),"value".into());assert!(c.expand_url(&context,&unknown).is_err());
        let long=BTreeMap::from([("query".into(),"文".repeat(1300))]);assert!(c.expand_url(&context,&long).is_err());
    }
    #[test]
    fn url_commands_refuse_unsafe_destinations_and_program_arguments() {
        let mut c=CustomCommand{action:CommandAction::Url,name:"URL".into(),..Default::default()};
        for bad in ["http://example.com/","file:///C:/test","javascript:alert(1)","https://{branch}/","https://user:secret@example.com/","https://example.com\\@other.com/","https://example.com/\n","https://example.com/%zz","https://example.com/{unknown}"] {
            c.program=bad.into();assert!(c.validate().is_err(),"{bad}");
        }
        c.program="https://example.com/{branch}".into();assert!(c.validate().is_ok());
        assert!(c.expand_url(&CommandContext::default(),&BTreeMap::new()).is_err());
        c.arguments="--unsafe".into();assert!(c.validate().is_err());c.arguments.clear();
        c.inputs.push(CommandInput{key:"flag".into(),label:"Flag".into(),kind:CommandInputKind::Checkbox,default_value:"false".into(),checked_value:"--flag".into(),..Default::default()});assert!(c.validate().is_err());
    }
}
