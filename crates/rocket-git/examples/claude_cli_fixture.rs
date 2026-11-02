//! A stand-in for Claude Code's `claude.exe` and, when built as `codex.exe`, for Codex's
//! `codex.exe` in tests: no network, account or model. It checks the options Rocket must
//! pass, records each request in its own folder (a disposable test folder), and answers
//! as the `mode` file there says (default `ok`), in the CLI's own output format.
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const REPLY: &str = "Add PRIVATE_DRAFT_SUBJECT\n\nPRIVATE_DRAFT_BODY explains the staged change.";
/// Options that keep Claude Code from using tools, customizations or a saved session.
const REQUIRED: [&str; 7] = [
    "--print",
    "--strict-mcp-config",
    "--safe-mode",
    "--disable-slash-commands",
    "--no-session-persistence",
    "--max-turns",
    "--system-prompt",
];
/// Options and `-c` settings that keep Codex read-only, without tools, the user's
/// config.toml, rules or session files.
const CODEX_REQUIRED: [&str; 7] = [
    "--json",
    "--ephemeral",
    "--ignore-user-config",
    "--ignore-rules",
    "--strict-config",
    "--skip-git-repo-check",
    "--color",
];
const CODEX_SETTINGS: [&str; 5] = [
    "approval_policy=\"never\"",
    "web_search=\"disabled\"",
    "history.persistence=\"none\"",
    "agents.enabled=false",
    "tools.experimental_request_user_input.enabled=false",
];

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("heartbeat") {
        // The grandchild of `slow`: proves the whole tree is stopped.
        let path = PathBuf::from(&args[1]);
        loop {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
                .unwrap();
            file.write_all(b".").unwrap();
            drop(file);
            thread::sleep(Duration::from_millis(50));
        }
    }
    let mut stdin = Vec::new();
    std::io::stdin().read_to_end(&mut stdin).unwrap();
    let exe = env::current_exe().unwrap();
    let folder = exe.parent().unwrap();
    let mode = fs::read_to_string(folder.join("mode"))
        .map(|m| m.trim().to_owned())
        .unwrap_or_else(|_| "ok".into());
    record(folder, &args, &stdin);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let codex = exe
        .file_stem()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("codex"));
    if codex {
        return codex_main(folder, &args, &stdin, &mode, &exe);
    }
    let missing = REQUIRED.iter().find(|r| !args.iter().any(|a| a == *r));
    if let Some(option) = missing {
        fail(&format!("fixture: required option {option} is missing"));
    }
    if value("--tools") != Some("")
        || value("--permission-mode") != Some("dontAsk")
        || value("--output-format") != Some("json")
        || value("--max-turns") != Some("1")
        || stdin.is_empty()
    {
        fail("fixture: no-tools print mode with a prompt on standard input is required");
    }
    let reply = fs::read_to_string(folder.join("reply")).unwrap_or_else(|_| REPLY.into());
    match mode.as_str() {
        "ok" => result(false, &reply),
        "fence" => result(false, &format!("```text\n{reply}\n```")),
        "signin" => {
            result(true, "Invalid API key · Please run /login");
            std::process::exit(1);
        }
        "fail" => {
            result(true, "Credit balance is too low");
            std::process::exit(1);
        }
        "outdated" => fail("error: unknown option '--safe-mode'"),
        "garbage" => print!("PRIVATE_DRAFT_SUBJECT is not JSON"),
        "flood" => print!("{}", "x".repeat(300 * 1024)),
        "slow" => slow(folder, &exe),
        other => fail(&format!("fixture: unknown mode {other}")),
    }
}

/// Codex: `codex exec --json` events in the shapes Codex 0.163 prints.
fn codex_main(folder: &Path, args: &[String], stdin: &[u8], mode: &str, exe: &Path) {
    let settings: Vec<&str> = args
        .windows(2)
        .filter(|w| w[0] == "-c")
        .map(|w| w[1].as_str())
        .collect();
    if args.first().map(String::as_str) != Some("exec")
        || args.last().map(String::as_str) != Some("-")
        || stdin.is_empty()
    {
        fail("fixture: codex exec with the prompt on standard input (-) is required");
    }
    if let Some(option) = CODEX_REQUIRED
        .iter()
        .find(|r| !args.iter().any(|a| a == *r))
    {
        fail(&format!("fixture: required option {option} is missing"));
    }
    if !args.windows(2).any(|w| w == ["--sandbox", "read-only"]) {
        fail("fixture: the read-only sandbox is required");
    }
    if let Some(setting) = CODEX_SETTINGS.iter().find(|s| !settings.contains(s)) {
        fail(&format!("fixture: required setting {setting} is missing"));
    }
    for feature in ["shell_tool", "code_mode_host", "multi_agent", "apps", "plugins", "hooks", "memories"] {
        if !args.windows(2).any(|w| w == ["--disable", feature]) {
            fail(&format!("fixture: required disabled feature {feature} is missing"));
        }
    }
    if !settings.iter().any(|s| s.starts_with("instructions=\"")) {
        fail("fixture: Rocket's instructions are required");
    }
    let reply = fs::read_to_string(folder.join("reply")).unwrap_or_else(|_| REPLY.into());
    let thread =
        "{\"type\":\"thread.started\",\"thread_id\":\"00000000-0000-4000-8000-000000000000\"}";
    let events = |text: &str| {
        println!("{thread}");
        // An expected disabled-tool notice remains harmless.
        println!(
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item_0\",\"type\":\"error\",\"message\":\"Code Mode is unavailable because code-mode host is disabled.\"}}}}"
        );
        println!("{{\"type\":\"turn.started\"}}");
        println!(
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":{}}}}}",
            json_string(text)
        );
        println!(
            "{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":10,\"cached_input_tokens\":0,\"output_tokens\":5}}}}"
        );
    };
    let failed = |message: &str| -> ! {
        println!("{thread}");
        println!("{{\"type\":\"turn.started\"}}");
        let retry = format!("Reconnecting... 1/5 ({message})");
        println!("{{\"type\":\"error\",\"message\":{}}}", json_string(&retry));
        println!(
            "{{\"type\":\"error\",\"message\":{}}}",
            json_string(message)
        );
        println!(
            "{{\"type\":\"turn.failed\",\"error\":{{\"message\":{}}}}}",
            json_string(message)
        );
        std::process::exit(1);
    };
    match mode {
        "ok" => events(&reply),
        "config-warning" => {
            eprintln!("WARNING: unknown configuration field: tools.shell");
            events(&reply);
        }
        "feature-warning" => {
            println!("{{\"type\":\"error\",\"message\":\"Unrecognized feature key: shell_tool\"}}");
            events(&reply);
        }
        "fence" => events(&format!("```text\n{reply}\n```")),
        "signin" => failed(
            "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header",
        ),
        "fail" => failed("You've hit your usage limit. Try again later."),
        "outdated" => {
            eprintln!(
                "error: unexpected argument '--ignore-user-config' found\n\n  tip: to pass '--ignore-user-config' as a value, use '-- --ignore-user-config'\n\nUsage: codex exec [OPTIONS] [PROMPT]"
            );
            std::process::exit(2);
        }
        "garbage" => print!("PRIVATE_DRAFT_SUBJECT is not JSON"),
        "flood" => print!("{}", "x".repeat(300 * 1024)),
        "slow" => slow(folder, exe),
        other => fail(&format!("fixture: unknown mode {other}")),
    }
}

/// Starts a heartbeat grandchild and waits: Rocket must stop the whole tree.
fn slow(folder: &Path, exe: &Path) {
    let mut child = Command::new(exe)
        .arg("heartbeat")
        .arg(folder.join("heartbeat"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    fs::write(
        folder.join("owned-processes"),
        format!("{} {}", std::process::id(), child.id()),
    )
    .unwrap();
    // Rocket stops the whole tree long before this; the cleanup is a fallback.
    thread::sleep(Duration::from_secs(300));
    let _ = child.kill();
    let _ = child.wait();
}

fn result(is_error: bool, text: &str) {
    println!(
        "{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":{is_error},\"duration_ms\":12,\"num_turns\":1,\"result\":{},\"session_id\":\"00000000-0000-4000-8000-000000000000\",\"total_cost_usd\":0}}",
        json_string(text)
    );
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(64);
}

/// The request in numbered files: arguments (one per line), standard input, the working
/// folder and the Claude Code variables Rocket sets.
fn record(folder: &Path, args: &[String], stdin: &[u8]) {
    let n = (1..)
        .find(|n| !folder.join(format!("request-{n}.txt")).exists())
        .unwrap();
    fs::write(folder.join(format!("arguments-{n}.txt")), args.join("\n")).unwrap();
    let mut environment = format!("cwd={}\n", env::current_dir().unwrap().display());
    for key in [
        "CLAUDE_CODE_SKIP_PROMPT_HISTORY",
        "CLAUDE_CODE_DISABLE_CLAUDE_MDS",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        "DISABLE_AUTOUPDATER",
    ] {
        environment.push_str(&format!("{key}={}\n", env::var(key).unwrap_or_default()));
    }
    fs::write(folder.join(format!("environment-{n}.txt")), environment).unwrap();
    // Written last: its presence means the request is complete.
    fs::write(folder.join(format!("request-{n}.txt")), stdin).unwrap();
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
