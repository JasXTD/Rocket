//! Advisory provider lifecycle events. No prompt, output, tool arguments or
//! permission decisions leave the callback. A capability binds each run to its
//! own bounded loopback receiver; events never execute a Git or agent action.
use super::model::{Provider, Record};
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const ENV: &str = "ROCKET_AGENT_EVENTS";
const LIMIT: usize = 2048;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Unknown,
    Ready,
    Working,
    Waiting,
    Complete,
    Interrupted,
    Unavailable,
}
impl Activity {
    pub const ALL: [Self; 7] = [
        Self::Unknown,
        Self::Ready,
        Self::Working,
        Self::Waiting,
        Self::Complete,
        Self::Interrupted,
        Self::Unavailable,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Activity not connected",
            Self::Ready => "Ready for input",
            Self::Working => "Working",
            Self::Waiting => "Needs your attention",
            Self::Complete => "Turn finished",
            Self::Interrupted => "Turn interrupted",
            Self::Unavailable => "Activity connection unavailable",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Event {
    pub id: String,
    pub kind: Activity,
    pub at: u64,
    pub turn: Option<String>,
    pub new_turn: bool,
}
pub fn valid_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
/// A resume must not replay a saved positional prompt or accidentally run an
/// exec/print subcommand. Unknown CLI options stay available through reviewed
/// restart; only recognized interactive launch options enable one-click resume.
pub fn can_resume(record: &Record) -> bool {
    let (values, switches): (&[&str], &[&str]) = match record.provider {
        Provider::Codex => (
            &[
                "--model",
                "-m",
                "--sandbox",
                "-s",
                "--ask-for-approval",
                "-a",
                "--profile",
                "-p",
                "--config",
                "-c",
                "--add-dir",
                "--enable",
                "--disable",
            ],
            &["--no-alt-screen", "--search"],
        ),
        Provider::Claude => (
            &[
                "--model",
                "--permission-mode",
                "--effort",
                "--settings",
                "--setting-sources",
                "--agent",
                "--allowedTools",
                "--allowed-tools",
                "--disallowedTools",
                "--disallowed-tools",
                "--tools",
                "--add-dir",
                "--mcp-config",
                "--append-system-prompt",
                "--system-prompt",
            ],
            &["--verbose", "--strict-mcp-config"],
        ),
        Provider::Copilot => (
            &[
                "--model",
                "--config-dir",
                "--log-dir",
                "--log-level",
                "--plugin-dir",
            ],
            &["--no-auto-update", "--no-banner", "--banner"],
        ),
        Provider::Custom => return false,
    };
    let args = record.command.args();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if switches.contains(&arg.as_str()) {
            continue;
        }
        if let Some((key, value)) = arg.split_once('=') {
            if !values.contains(&key) || value.is_empty() {
                return false;
            }
        } else if !values.contains(&arg.as_str()) || args.next().is_none_or(|v| v.is_empty()) {
            return false;
        }
    }
    true
}
fn text<'a>(v: &'a Value, a: &str, b: &str) -> Option<&'a str> {
    v[a].as_str().or_else(|| v[b].as_str())
}
fn event(kind: &str, v: &Value) -> Option<Event> {
    let new_turn = kind == "UserPromptSubmit";
    // Subagent tools must never change the main conversation's attention state.
    if v["agent_id"].as_str().is_some_and(|s| !s.is_empty())
        || v["agentId"].as_str().is_some_and(|s| !s.is_empty())
    {
        return None;
    }
    let id = text(v, "session_id", "sessionId").filter(|s| valid_id(s))?;
    let kind = match kind {
        "SessionStart" => Activity::Ready,
        "UserPromptSubmit" | "PostToolUse" => Activity::Working,
        "PreToolUse" => {
            if text(v, "tool_name", "toolName")
                .is_some_and(|s| matches!(s, "AskUserQuestion" | "ask_user" | "request_user_input"))
            {
                Activity::Waiting
            } else {
                Activity::Working
            }
        }
        "PermissionRequest" => Activity::Waiting,
        "Notification"
            if text(v, "notification_type", "notificationType")
                .is_some_and(|s| matches!(s, "permission_prompt" | "elicitation_dialog")) =>
        {
            Activity::Waiting
        }
        "Stop" if v["stop_hook_active"].as_bool() != Some(true) => Activity::Complete,
        "Interrupt" => Activity::Interrupted,
        _ => return None,
    };
    let turn = v["turn_id"]
        .as_str()
        .filter(|s| s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(str::to_owned);
    Some(Event {
        id: id.into(),
        kind,
        at: now(),
        turn,
        new_turn,
    })
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u64::MAX as u128) as u64
}
fn encode(e: &Event, token: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"token":token,"id":e.id,"kind":format!("{:?}",e.kind),"at":e.at,"turn":e.turn,"new_turn":e.new_turn})).unwrap()
}
fn decode(bytes: &[u8], token: &str) -> Option<Event> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v["token"].as_str() != Some(token) {
        return None;
    }
    let id = v["id"].as_str().filter(|s| valid_id(s))?.to_owned();
    let kind = match v["kind"].as_str()? {
        "Ready" => Activity::Ready,
        "Working" => Activity::Working,
        "Waiting" => Activity::Waiting,
        "Complete" => Activity::Complete,
        "Interrupted" => Activity::Interrupted,
        _ => return None,
    };
    let at = v["at"].as_u64()?;
    let turn = v["turn"]
        .as_str()
        .filter(|s| s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(str::to_owned);
    Some(Event {
        id,
        kind,
        at,
        turn,
        new_turn: v["new_turn"].as_bool() == Some(true),
    })
}

/// Called before desktop/MCP initialization. Always neutral to the provider:
/// neither failures nor events allow/deny tools, append instructions or block a turn.
pub fn callback(args: &[std::ffi::OsString]) -> bool {
    if args.first().is_none_or(|a| a != "--rocket-agent-event") {
        return false;
    }
    if args.len() == 2
        && let Some(kind) = args[1].to_str()
    {
        let _ = (|| -> io::Result<()> {
            let mut bytes = Vec::new();
            io::stdin().take(256 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 256 * 1024 {
                return Ok(());
            }
            if let Ok(v) = serde_json::from_slice(&bytes) {
                emit(kind, &v)?;
            }
            Ok(())
        })();
    }
    println!("{{}}");
    true
}
/// Also used by the fixed local acceptance process; input is sanitized identically.
pub fn emit(kind: &str, value: &Value) -> io::Result<()> {
    let Some(e) = event(kind, value) else {
        return Ok(());
    };
    let config = std::env::var(ENV).map_err(io::Error::other)?;
    let (address, token) = config
        .split_once('/')
        .ok_or_else(|| io::Error::other("Missing activity endpoint"))?;
    let address: SocketAddr = address.parse().map_err(io::Error::other)?;
    if address.ip() != std::net::Ipv4Addr::LOCALHOST
        || token.len() != 64
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(io::Error::other("Invalid activity endpoint"));
    }
    let bytes = encode(&e, token);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_millis(300)))?;
    stream.set_write_timeout(Some(Duration::from_millis(300)))?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    let mut ack = [0];
    stream.read_exact(&mut ack)?;
    if ack != [1] {
        return Err(io::Error::other("Activity queue unavailable"));
    }
    Ok(())
}

pub struct Bridge {
    rx: mpsc::Receiver<Event>,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    pub environment: Vec<(String, String)>,
    plugin: Option<PathBuf>,
}
impl Bridge {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let token = rocket_mcp_control::ephemeral_token()?;
        let environment = vec![(ENV.into(), format!("{}/{token}", listener.local_addr()?))];
        let (tx, rx) = mpsc::sync_channel(128);
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let ending = stop.clone();
        let overflow = lost.clone();
        std::thread::Builder::new()
            .name("rocket-agent-events".into())
            .spawn(move || {
                while !ending.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let received = (|| -> io::Result<Option<Event>> {
                                stream.set_read_timeout(Some(Duration::from_millis(250)))?;
                                stream.set_write_timeout(Some(Duration::from_millis(250)))?;
                                let mut size = [0; 4];
                                stream.read_exact(&mut size)?;
                                let size = u32::from_be_bytes(size) as usize;
                                if size > LIMIT {
                                    return Ok(None);
                                }
                                let mut bytes = vec![0; size];
                                stream.read_exact(&mut bytes)?;
                                Ok(decode(&bytes, &token))
                            })();
                            if let Ok(Some(e)) = received {
                                if tx.try_send(e).is_ok() {
                                    let _ = stream.write_all(&[1]);
                                } else {
                                    overflow.store(true, Ordering::Release);
                                    let _ = stream.write_all(&[0]);
                                }
                                wake();
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => {
                            overflow.store(true, Ordering::Release);
                            wake();
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            rx,
            stop,
            lost,
            environment,
            plugin: None,
        })
    }
    pub fn drain(&self) -> Vec<Event> {
        self.rx.try_iter().take(128).collect()
    }
    pub fn lost(&self) -> bool {
        self.lost.swap(false, Ordering::AcqRel)
    }
    pub fn arguments(&mut self, record: &Record, resume: bool) -> io::Result<Vec<String>> {
        let mut args = record.command.args();
        if resume {
            if !can_resume(record) {
                return Err(io::Error::other(
                    "These arguments need a reviewed restart; resume never replays an initial prompt or noninteractive command",
                ));
            }
            let id = record
                .conversation
                .as_deref()
                .filter(|s| valid_id(s))
                .ok_or_else(|| io::Error::other("No exact conversation is saved"))?;
            match record.provider {
                Provider::Codex => {
                    args.insert(0, id.into());
                    args.insert(0, "resume".into());
                }
                Provider::Claude | Provider::Copilot => args.extend(["--resume".into(), id.into()]),
                Provider::Custom => {
                    return Err(io::Error::other(
                        "Custom commands need their own resume arguments",
                    ));
                }
            }
        }
        let exe = std::env::current_exe()?;
        let events = [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "PermissionRequest",
            "Stop",
            "Interrupt",
            "Notification",
        ];
        let mut hooks = serde_json::Map::new();
        for kind in events {
            let bash = format!(
                "{} --rocket-agent-event {kind}",
                quote(&exe.to_string_lossy())
            );
            let ps = format!(
                "& {} --rocket-agent-event {kind}",
                quote_ps(&exe.to_string_lossy())
            );
            match record.provider {
                Provider::Codex => {
                    if kind == "Notification" {
                        continue;
                    }
                    // CLI overrides are an additional config layer; never edit user files
                    // or bypass Codex's own hook trust review.
                    args.extend(["-c".into(),format!("hooks.{kind}=[{{hooks=[{{type=\"command\",command={},command_windows={},timeout=2}}]}}]",json!(bash),json!(ps))]);
                }
                Provider::Claude => {
                    if kind == "Interrupt" {
                        continue;
                    }
                    hooks.insert(
                        kind.into(),
                        json!([{"hooks":[{"type":"command","command":bash,"timeout":2}]}]),
                    );
                }
                Provider::Copilot => {
                    if kind == "Interrupt" {
                        continue;
                    }
                    hooks.insert(kind.into(),json!([{"type":"command","exec":exe,"args":["--rocket-agent-event",kind],"timeoutSec":2}]));
                }
                Provider::Custom => {}
            }
        }
        match record.provider {
            Provider::Claude => {
                args.extend(["--settings".into(), json!({"hooks":hooks}).to_string()])
            }
            Provider::Copilot => {
                #[cfg(windows)]
                let root = std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .ok_or_else(|| {
                        io::Error::other("Private local application storage is unavailable")
                    })?
                    .join("Rocket/agent-events");
                #[cfg(not(windows))]
                let root = rocket_git::local_storage::directory()?.join("agent-events");
                #[cfg(windows)]
                std::fs::create_dir_all(&root)?;
                #[cfg(not(windows))]
                rocket_git::local_storage::create_private(&root)?;
                let path = root.join(rocket_mcp_control::ephemeral_token()?);
                #[cfg(windows)]
                std::fs::create_dir(&path)?;
                #[cfg(not(windows))]
                rocket_git::local_storage::create_private_new(&path)?;
                self.plugin = Some(path.clone());
                write_new(
                    &path.join("plugin.json"),
                    json!({"name":"rocket-activity","version":"1.0.0","hooks":"./hooks.json"})
                        .to_string()
                        .as_bytes(),
                )?;
                write_new(
                    &path.join("hooks.json"),
                    json!({"version":1,"hooks":hooks}).to_string().as_bytes(),
                )?;
                args.extend([
                    "--plugin-dir".into(),
                    path.to_str()
                        .ok_or_else(|| io::Error::other("Activity folder needs a Unicode path"))?
                        .into(),
                ]);
            }
            _ => {}
        }
        Ok(args)
    }
}
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(bytes)
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "/").replace('\'', "'\"'\"'"))
}
fn quote_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Only our two files; never recursively remove any directory contents.
        if let Some(p) = &self.plugin
            && std::fs::symlink_metadata(p).is_ok_and(|m| {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return false;
                    }
                }
                m.is_dir() && !m.file_type().is_symlink()
            })
        {
            let _ = std::fs::remove_file(p.join("hooks.json"));
            let _ = std::fs::remove_file(p.join("plugin.json"));
            let _ = std::fs::remove_dir(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "11111111-2222-3333-4444-555555555555";
    #[test]
    fn provider_events_strip_payloads_and_ignore_subagents_and_idle_nags() {
        for (name, state) in [
            ("SessionStart", Activity::Ready),
            ("UserPromptSubmit", Activity::Working),
            ("PermissionRequest", Activity::Waiting),
            ("Stop", Activity::Complete),
            ("Interrupt", Activity::Interrupted),
        ] {
            for id_key in ["session_id", "sessionId"] {
                let mut v = json!({id_key:ID,"prompt":"PRIVATE_PROMPT","last_assistant_message":"PRIVATE_REPLY","transcript_path":"PRIVATE_PATH"});
                let e = event(name, &v).unwrap();
                assert_eq!(e.kind, state);
                let bytes = encode(&e, "fixture");
                let data = String::from_utf8(bytes).unwrap();
                assert!(!data.contains("PRIVATE"));
                v["agent_id"] = json!("subagent");
                assert!(event(name, &v).is_none());
            }
        }
        assert!(event("Stop", &json!({"session_id":ID,"stop_hook_active":true})).is_none());
        assert!(
            event(
                "Notification",
                &json!({"session_id":ID,"notification_type":"idle_prompt"})
            )
            .is_none()
        );
        assert!(event("Stop", &json!({"session_id":"--resume-other"})).is_none());
        assert_eq!(
            event(
                "PreToolUse",
                &json!({"session_id":ID,"tool_name":"AskUserQuestion"})
            )
            .unwrap()
            .kind,
            Activity::Waiting
        );
        assert_eq!(
            event("PreToolUse", &json!({"sessionId":ID,"toolName":"ask_user"}))
                .unwrap()
                .kind,
            Activity::Waiting
        );
    }
    #[test]
    fn loopback_events_are_isolated_acknowledged_bounded_and_closed() {
        let a = Bridge::new(Arc::new(|| {})).unwrap();
        let b = Bridge::new(Arc::new(|| {})).unwrap();
        let (address, token) = a.environment[0].1.split_once('/').unwrap();
        let send = |token: &str| {
            let mut c = TcpStream::connect(address).unwrap();
            c.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let bytes = encode(
                &event("SessionStart", &json!({"session_id":ID})).unwrap(),
                token,
            );
            c.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
            c.write_all(&bytes).unwrap();
            let mut ack = [0];
            c.read_exact(&mut ack).map(|_| ack)
        };
        assert!(send("wrong run").is_err());
        assert!(a.drain().is_empty());
        assert_eq!(send(token).unwrap(), [1]);
        assert_eq!(a.drain().len(), 1);
        assert!(b.drain().is_empty());
        for _ in 0..128 {
            assert_eq!(send(token).unwrap(), [1]);
        }
        assert_eq!(send(token).unwrap(), [0]);
        assert!(a.lost());
        assert_eq!(a.drain().len(), 128);
        let addr: SocketAddr = address.parse().unwrap();
        drop(a);
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while TcpStream::connect_timeout(&addr, Duration::from_millis(20)).is_ok() {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn resume_uses_exact_id_and_never_adds_permission_bypasses() {
        for provider in [Provider::Codex, Provider::Claude] {
            let r = Record {
                provider,
                conversation: Some(ID.into()),
                ..Default::default()
            };
            let mut b = Bridge::new(Arc::new(|| {})).unwrap();
            let args = b.arguments(&r, true).unwrap();
            assert!(args.iter().any(|a| a == ID));
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("dangerously") || a == "--last" || a == "--continue")
            );
            assert!(!args.join(" ").contains(&b.environment[0].1));
            assert!(args.iter().any(|a| a.contains("SessionStart")));
            assert!(args.iter().any(|a| a.contains("PermissionRequest")));
        }
        assert_eq!(quote("/tmp/a'b/rocket"), "'/tmp/a'\"'\"'b/rocket'");
        assert_eq!(quote_ps("C:/a'b/rocket.exe"), "'C:/a''b/rocket.exe'");
    }
    #[test]
    fn resume_refuses_saved_prompts_and_noninteractive_commands() {
        for provider in [Provider::Claude, Provider::Codex, Provider::Copilot] {
            let mut record = Record {
                provider,
                ..Default::default()
            };
            assert!(can_resume(&record));
            record.command.arguments = "--model\nreviewed-model".into();
            assert!(can_resume(&record));
            for args in [
                "Repeat the original task",
                "exec\noriginal prompt",
                "--print\noriginal prompt",
                "--prompt=original prompt",
                "--unknown-switch",
                "--model",
            ] {
                record.command.arguments = args.into();
                assert!(!can_resume(&record), "{provider:?}: {args}");
            }
        }
    }
}
