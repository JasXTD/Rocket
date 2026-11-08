//! Explicit user-configured processes. Repository config cannot choose a program.
//! Merge tools only receive disposable copies; publication uses save_conflict.
use crate::{process_tree::ProcessTree, *};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalToolKind {
    Editor,
    Diff,
    Merge,
}
impl ExternalToolKind {
    pub fn index(self) -> usize {
        match self {
            Self::Editor => 0,
            Self::Diff => 1,
            Self::Merge => 2,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Editor => "Editor",
            Self::Diff => "Diff",
            Self::Merge => "Merge",
        }
    }
    pub fn operation(self) -> &'static str {
        match self {
            Self::Editor => "external-editor",
            Self::Diff => "external-diff",
            Self::Merge => "external-merge",
        }
    }
    pub fn placeholders(self) -> &'static [&'static str] {
        match self {
            Self::Editor => &["{file}"],
            Self::Diff => &["{left}", "{right}"],
            Self::Merge => &["{base}", "{ours}", "{theirs}", "{result}"],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalToolConfig {
    pub executable: String,
    pub arguments: String,
}
impl ExternalToolConfig {
    /// Empty executable disables a tool. Configuration does not resolve PATH or run it.
    pub fn validate(&self, kind: ExternalToolKind) -> Result<()> {
        if self.executable.len() > 32768
            || self.arguments.len() > 16384
            || self.executable.contains(['\0', '\n', '\r'])
            || self.arguments.contains('\0')
        {
            return Err(Error::ExternalTool(ExternalToolFailure::Configuration));
        }
        if self.executable.is_empty() {
            return Ok(());
        }
        let path = Path::new(&self.executable);
        if !path.is_absolute() {
            return Err(Error::ExternalTool(ExternalToolFailure::Configuration));
        }
        #[cfg(windows)]
        if !path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        {
            return Err(Error::ExternalTool(ExternalToolFailure::Configuration));
        }
        let args = self.argument_lines();
        if args.len() > 64
            || kind.placeholders().iter().any(|p| !args.contains(p))
            || args
                .iter()
                .any(|a| a.contains(['{', '}']) && !kind.placeholders().contains(a))
        {
            return Err(Error::ExternalTool(ExternalToolFailure::Configuration));
        }
        Ok(())
    }
    fn argument_lines(&self) -> Vec<&str> {
        self.arguments.lines().filter(|s| !s.is_empty()).collect()
    }
    fn expand(&self, bindings: &[(&str, PathBuf)]) -> Vec<OsString> {
        // Each line is a single argument. Substitution cannot introduce an option,
        // a second argument or shell syntax; paths never pass through a shell.
        self.argument_lines()
            .into_iter()
            .map(|arg| {
                bindings
                    .iter()
                    .find(|(k, _)| *k == arg)
                    .map_or_else(|| arg.into(), |(_, p)| p.as_os_str().to_owned())
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalToolFailure {
    Configuration,
    NotConfigured,
    Exit(Option<i32>),
    ResultInvalid,
    DraftChanged,
}
impl ExternalToolFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "external_tool_configuration",
            Self::NotConfigured => "external_tool_not_configured",
            Self::Exit(_) => "external_tool_exit",
            Self::ResultInvalid => "external_merge_result_invalid",
            Self::DraftChanged => "external_merge_draft_changed",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Configuration => {
                "Choose an absolute executable path (.exe on Windows). Enter one argument per line, without shell quoting; include each required placeholder on its own line. Maximum 64 arguments. Clear the executable to disable this tool."
            }
            Self::NotConfigured => {
                "Configure this tool in Edit → External tools first. Use an installed executable that waits for its own window to close."
            }
            Self::Exit(_) => {
                "The external tool did not exit successfully. No merge result was imported. Check its executable and arguments; any temporary merge result is retained."
            }
            Self::ResultInvalid => {
                "The merge result must be a normal UTF-8 file up to 512 KiB and 20,000 lines, with consistent LF or CRLF endings. No result was imported; the temporary workspace is retained."
            }
            Self::DraftChanged => {
                "The conflict selection or Rocket draft changed while the tool was open. The external result is retained. Review it separately; Rocket will not replace your newer draft."
            }
        }
    }
}

pub enum ExternalToolRequest {
    Editor(RepoPath),
    Diff {
        options: DiffOptions,
        path: Option<RepoPath>,
        label: Option<String>,
        untracked: bool,
        comparison: Option<Arc<Comparison>>,
    },
    Merge {
        preview: Arc<ConflictPreview>,
        draft: String,
    },
}
impl ExternalToolRequest {
    pub fn kind(&self) -> ExternalToolKind {
        match self {
            Self::Editor(_) => ExternalToolKind::Editor,
            Self::Diff { .. } => ExternalToolKind::Diff,
            Self::Merge { .. } => ExternalToolKind::Merge,
        }
    }
}
pub struct ExternalToolReport {
    pub workspace: Option<PathBuf>,
    pub preview: Option<Arc<ConflictPreview>>,
    pub text: Option<String>,
    pub result: Result<()>,
}
static NEXT: AtomicU64 = AtomicU64::new(1);

impl Repository {
    pub fn run_external_tool(
        &self,
        config: &ExternalToolConfig,
        request: ExternalToolRequest,
        storage: &Path,
        log: &DiagnosticLog,
        cancel: &CancellationToken,
    ) -> ExternalToolReport {
        let kind = request.kind();
        let mut report = ExternalToolReport {
            workspace: None,
            preview: None,
            text: None,
            result: Ok(()),
        };
        let result=log.request(kind.operation(), |log| {
            cancel.check()?;
            crate::ensure_unprivileged_process()?;
            config.validate(kind)?;
            if config.executable.is_empty() { return Err(Error::ExternalTool(ExternalToolFailure::NotConfigured)); }
            cancel.check()?;
            let executable=Path::new(&config.executable);
            let meta=fs::symlink_metadata(executable).map_err(|e|Error::io("inspect external executable",e))?;
            if !meta.is_file() || crate::local_files::redirected(&meta) { return Err(Error::ExternalTool(ExternalToolFailure::Configuration)); }
            let repo=self.with_log(log.clone());
            let (bindings, working_directory, input_guards)=log.request("external-prepare", |_| {
                let mut guards=Vec::<File>::new();
                match request {
                    ExternalToolRequest::Editor(path)=>{
                        // Same containment/reparse checks as the bounded version reader.
                        crate::image_compare::read_working(&repo,&path,cancel)?;
                        let root=repo.info().worktree.as_ref().ok_or(Error::WorktreeRequired)?;
                        let full=root.join(path.argument()?);
                        Ok((vec![("{file}",full)],root.clone(),guards))
                    }
                    ExternalToolRequest::Diff {options,path,label,untracked,comparison}=>{
                        if let Some(c) = &comparison { repo.validate_comparison(c,cancel)?; }
                        let selection=if untracked {
                            repo.untracked_image(path.as_ref().ok_or_else(||Error::InvalidInput("Select a file first".into()))?,cancel)?
                        } else {
                            let mut candidates=repo.file_comparisons(&options,cancel)?.into_iter().filter(|p| path.as_ref().map_or_else(||label.as_ref().is_some_and(|s|p.path().display_lossy()==*s), |path|p.path()==path));
                            let selected=candidates.next().ok_or_else(||Error::InvalidInput("This comparison changed or is unavailable. Reload and select the file again.".into()))?;
                            if candidates.next().is_some() { return Err(Error::InvalidInput("Ambiguous file identity. Select the path in Changes.".into())); }
                            selected
                        };
                        let versions=repo.image_versions(&selection,cancel)?;
                        if let Some(c) = &comparison { repo.validate_comparison(c,cancel)?; }
                        let directory=workspace(storage)?;
                        report.workspace=Some(directory.clone());
                        let left=write_copy(&directory,"left",selection.path(),versions.before.as_deref().unwrap_or_default(),true,&mut guards)?;
                        let right=write_copy(&directory,"right",selection.path(),versions.after.as_deref().unwrap_or_default(),true,&mut guards)?;
                        Ok((vec![("{left}",left),("{right}",right)],directory,guards))
                    }
                    ExternalToolRequest::Merge {preview,draft}=>{
                        repo.validate_conflict_preview(&preview,cancel)?;
                        crate::conflict::decode(draft.as_bytes())?;
                        let directory=workspace(storage)?;
                        report.workspace=Some(directory.clone());
                        report.preview=Some(preview.clone());
                        let mut bindings=Vec::new();
                        for (stage,key,name) in [(1,"{base}","base"),(2,"{ours}","ours"),(3,"{theirs}","theirs")] {
                            bindings.push((key,write_copy(&directory,name,preview.path(),preview.side(stage).unwrap_or_default().as_bytes(),true,&mut guards)?));
                        }
                        bindings.push(("{result}",write_copy(&directory,"result",preview.path(),draft.as_bytes(),false,&mut guards)?));
                        Ok((bindings,directory,guards))
                    }
                }
            })?;
            cancel.check()?;
            log.request("external-process", |_| {
                let mut command=Command::new(executable);
                command.args(config.expand(&bindings)).current_dir(working_directory).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                let starting=ProcessTree::prepare(&mut command).map_err(|e|Error::io("launch external tool",e))?;
                let mut child=command.spawn().map_err(|e|Error::io("launch external tool",e))?;
                let tree=match ProcessTree::attach(starting,&child) {Ok(tree)=>tree,Err(e)=>{let _=child.kill();let _=child.wait();return Err(Error::io("contain external process",e));}};
                let result=loop {
                    if let Err(e)=cancel.check() { break Err(e); }
                    match crate::process_tree::poll_exit(&mut child) {
                        Ok(Some(status))=>break if status.success(){Ok(())}else{Err(Error::ExternalTool(ExternalToolFailure::Exit(status.code())))},
                        Ok(None)=>std::thread::sleep(Duration::from_millis(20)),
                        Err(e)=>break Err(Error::io("wait for external tool",e)),
                    }
                };
                // Never terminate by name: only this launch and its job's descendants.
                let termination=tree.terminate().map_err(|e|Error::io("stop external tool",e));
                if termination.is_err() { let _=child.kill(); }
                let waited=child.wait().map_err(|e|Error::io("reap external tool",e));
                drop(tree);
                result.and(termination).and(waited.map(|_|()))
            })?;
            drop(input_guards);
            cancel.check()?;
            if kind==ExternalToolKind::Merge {
                let path=&bindings.iter().find(|(k,_)| *k=="{result}").expect("merge binding").1;
                report.text=Some(log.request("external-result", |_|read_result(path,report.workspace.as_ref().unwrap(),cancel))?);
            }
            Ok(())
        });
        report.result = result;
        // Retain all workspaces explicitly. No recursive cleanup of tool-created
        // content, and cancelled/failed merge work remains recoverable after exit.
        report
    }
}
fn workspace(storage: &Path) -> Result<PathBuf> {
    fs::create_dir_all(storage).map_err(|e| Error::io("create external workspace root", e))?;
    crate::local_files::directory(storage)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidInput("System clock precedes epoch".into()))?
        .as_nanos();
    let directory = storage.join(format!(
        "tool-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    #[allow(unused_mut)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&directory)
        .map_err(|e| Error::io("create external workspace", e))?;
    directory
        .canonicalize()
        .map_err(|e| Error::io("resolve external workspace", e))
}
fn write_copy(
    directory: &Path,
    name: &str,
    original: &RepoPath,
    bytes: &[u8],
    readonly: bool,
    guards: &mut Vec<File>,
) -> Result<PathBuf> {
    let extension = original
        .as_bytes()
        .rsplit(|b| *b == b'.')
        .next()
        .filter(|s| s.len() <= 16 && s.iter().all(u8::is_ascii_alphanumeric))
        .and_then(|s| std::str::from_utf8(s).ok());
    let path =
        directory.join(extension.map_or_else(|| name.to_owned(), |ext| format!("{name}.{ext}")));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| Error::io("create external copy", e))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("write external copy", e))?;
    drop(file);
    if readonly {
        let mut permissions = fs::metadata(&path)
            .map_err(|e| Error::io("inspect external input", e))?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)
            .map_err(|e| Error::io("protect external input", e))?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        guards.push(
            options
                .open(&path)
                .map_err(|e| Error::io("pin external input", e))?,
        );
    }
    Ok(path)
}
fn read_result(path: &Path, root: &Path, cancel: &CancellationToken) -> Result<String> {
    let invalid = || Error::ExternalTool(ExternalToolFailure::ResultInvalid);
    cancel.check()?;
    let _parents = crate::local_files::pin_directories(root, path.parent().ok_or_else(invalid)?)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000).share_mode(1);
    }
    let file = options
        .open(path)
        .map_err(|e| Error::io("read external merge result", e))?;
    crate::review::verify_preview_handle(&file, root, path)?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect external merge result", e))?;
    if !meta.is_file()
        || crate::local_files::redirected(&meta)
        || meta.len() > CONFLICT_TEXT_LIMIT as u64
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(CONFLICT_TEXT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read external merge result", e))?;
    cancel.check()?;
    crate::conflict::decode(&bytes)
        .map(|v| v.0)
        .map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_preserve_spaces_unicode_and_metacharacters_without_shell_parsing() {
        let config = ExternalToolConfig {
            executable: "C:/Program Files/tool.exe".into(),
            arguments: "--wait\n{left}\n{right}".into(),
        };
        config.validate(ExternalToolKind::Diff).unwrap();
        let p = PathBuf::from("C:/文 path/$() & name.txt");
        assert_eq!(
            config.expand(&[("{left}", p.clone()), ("{right}", p.clone())]),
            vec![
                OsString::from("--wait"),
                p.as_os_str().to_owned(),
                p.into_os_string()
            ]
        );
        for args in [
            "{left}",
            "{left}\n{right}\n{result}",
            "--left={left}\n{right}",
            "{left}\n{right}\n\0",
        ] {
            assert!(
                ExternalToolConfig {
                    arguments: args.into(),
                    ..config.clone()
                }
                .validate(ExternalToolKind::Diff)
                .is_err()
            );
        }
    }
}
