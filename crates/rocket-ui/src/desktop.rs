//! Bounded desktop handoffs; targets are arguments, never shell programs.
use crate::app::Rocket;
use rocket_git::{CancellationToken, Error, ReadTask, Result};
use std::{path::Path, process::{Command, Stdio}, time::{Duration, Instant}};

pub(crate) const FOLDER_HINT: &str = if cfg!(target_os = "macos") { "Show in Finder" }
    else if cfg!(windows) { "Show in File Explorer" } else { "Show in file manager" };

fn opener() -> Result<Command> {
    #[cfg(windows)]
    { std::env::var_os("SystemRoot").map(|p| Command::new(Path::new(&p).join("explorer.exe")))
        .ok_or(Error::InvalidInput("SystemRoot is not set".into())) }
    #[cfg(target_os = "macos")]
    { Ok(Command::new("/usr/bin/open")) }
    #[cfg(target_os = "linux")]
    { Ok(Command::new("/usr/bin/xdg-open")) }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    { Err(Error::InvalidInput("Desktop opening is unavailable on this platform".into())) }
}

pub(crate) fn browser_command(url: &str) -> Result<Command> {
    if !rocket_git::https_url(url) { return Err(Error::InvalidInput("Rocket opens only plain HTTPS links".into())); }
    let mut command = opener()?;
    command.arg(url);
    Ok(command)
}

pub(crate) fn folder_command(path: &Path) -> Result<Command> {
    if !path.is_absolute() || !path.is_dir() { return Err(Error::InvalidInput("Choose an existing absolute folder".into())); }
    let mut command = opener()?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer also parses commas. Quotes cannot occur in a Windows path.
        command.raw_arg(format!("\"{}\"", crate::views::display_path(path)));
    }
    #[cfg(not(windows))]
    {
        // Reveal in Finder: plain `open folder.app` would launch the bundle.
        #[cfg(target_os = "macos")]
        command.arg("-R");
        command.arg(path);
    } // absolute path cannot become an option, URL or app name
    Ok(command)
}

fn run(mut command: Command, cancel: CancellationToken) -> Result<()> {
    cancel.check()?;
    rocket_git::ensure_unprivileged_process()?;
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let mut child = command.spawn().map_err(|source| Error::Io { action: "start desktop opener", source })?;
    let started = Instant::now();
    loop {
        let stopped = cancel.check().err().or_else(|| (started.elapsed() >= Duration::from_secs(10))
            .then_some(Error::InvalidInput("Desktop opener did not finish within ten seconds".into())));
        if let Some(error) = stopped {
            // Only stop/reap our launcher, never an already opened browser/Finder.
            let _ = child.kill();
            child.wait().map_err(|source| Error::Io { action: "reap desktop opener", source })?;
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // Explorer's handoff exit code is not a reliable success signal.
                return if cfg!(windows) || status.success() { Ok(()) }
                    else { Err(Error::InvalidInput("Desktop opener refused the request; check the default application".into())) };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(source) => {
                let _ = child.kill(); let _ = child.wait();
                return Err(Error::Io { action: "wait for desktop opener", source });
            }
        }
    }
}

impl Rocket {
    pub(crate) fn launch_desktop(&mut self, command: impl FnOnce() -> Result<Command> + Send + 'static, operation: &'static str) {
        let result = (|| {
            if self.quitting { return Err(Error::Cancelled); }
            if self.desktop_tasks.len() >= 4 { return Err(Error::InvalidInput("Wait for the pending desktop requests".into())); }
            let log = self.backend.as_ref().map(|b| b.log.clone());
            let task = ReadTask::spawn(move |cancel| match log {
                Some(log) => log.request(operation, |_| command().and_then(|command| run(command, cancel))),
                None => command().and_then(|command| run(command, cancel)),
            })?;
            self.desktop_tasks.push(task);
            Ok(())
        })();
        if let Err(error) = result { self.notice = Some(crate::backend::user_error(&error)); }
    }
    pub(crate) fn poll_desktop(&mut self, ctx: &eframe::egui::Context) {
        if self.quitting { for task in &self.desktop_tasks { task.cancel(); } }
        self.desktop_tasks.retain_mut(|task| match task.try_finish() {
            Some(result) => { if !self.quitting && let Err(error) = result { self.notice = Some(crate::backend::user_error(&error)); } false }
            None => true,
        });
        if !self.desktop_tasks.is_empty() { ctx.request_repaint_after(Duration::from_millis(30)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_urls_are_https_arguments_not_shell_commands() {
        for value in ["file:///tmp/a", "-aCalculator", "https://host/a b", "https://user:pass@host/a"] {
            assert!(browser_command(value).is_err(), "{value}");
        }
        let url = "https://example.invalid/repo/commit/a?x=1&y=2";
        let command = browser_command(url).unwrap();
        assert_eq!(command.get_args().collect::<Vec<_>>(), [url]);
        #[cfg(target_os = "macos")]
        assert_eq!(command.get_program(), "/usr/bin/open");
        #[cfg(target_os = "linux")]
        assert_eq!(command.get_program(), "/usr/bin/xdg-open");
        assert!(folder_command(Path::new("-a")).is_err());
    }
    #[test]
    fn desktop_precancel_does_not_spawn_and_missing_opener_reports_failure() {
        let token = CancellationToken::new(); token.cancel();
        assert!(matches!(run(Command::new("/rocket-test-missing-opener"), token), Err(Error::Cancelled)));
        assert!(matches!(run(Command::new("/rocket-test-missing-opener"), CancellationToken::new()), Err(Error::Io { .. })));
    }
    #[test]
    fn desktop_quit_waits_for_cancelled_worker_cleanup() {
        let fixture = crate::test_common::Fixture::new();
        let ctx = eframe::egui::Context::default();
        let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
        let (started, ready) = std::sync::mpsc::channel();
        let (cleaned, cleanup) = std::sync::mpsc::channel();
        let task = ReadTask::spawn(move |cancel| {
            started.send(()).unwrap();
            while cancel.check().is_ok() { std::thread::sleep(Duration::from_millis(5)); }
            cleanup.recv_timeout(Duration::from_secs(2)).unwrap();
            Err(Error::Cancelled)
        }).unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        app.desktop_tasks.push(task);
        app.begin_quit();
        app.poll_desktop(&ctx);
        assert!(app.working(), "native close waits while launcher cleanup is pending");
        assert_eq!(app.desktop_tasks.len(), 1);
        cleaned.send(()).unwrap();
        let at = Instant::now();
        while !app.desktop_tasks.is_empty() {
            assert!(at.elapsed() < Duration::from_secs(2));
            app.poll_desktop(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(unix)]
    #[test]
    fn desktop_unicode_folder_is_one_literal_argument_and_mac_reveals_bundles() {
        let root = std::env::temp_dir().join(format!("rocket-desktop-{}-資料 $(nothing).app", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let command = folder_command(&root).unwrap();
        let args: Vec<_> = command.get_args().collect();
        #[cfg(target_os = "macos")]
        assert_eq!(args, [std::ffi::OsStr::new("-R"), root.as_os_str()]);
        #[cfg(target_os = "linux")]
        assert_eq!(args, [root.as_os_str()]);
        std::fs::remove_dir(&root).unwrap();
        assert!(folder_command(&root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn desktop_worker_reports_exit_status_and_cancels_owned_launcher() {
        assert!(run(Command::new("/usr/bin/true"), CancellationToken::new()).is_ok());
        assert!(run(Command::new("/usr/bin/false"), CancellationToken::new()).is_err());
        let mut command = Command::new("/bin/sleep"); command.arg("10");
        let task = ReadTask::spawn(|cancel| run(command, cancel)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let at = Instant::now(); task.cancel();
        assert!(matches!(task.wait(), Err(Error::Cancelled)));
        assert!(at.elapsed() < Duration::from_secs(2));
    }
}
