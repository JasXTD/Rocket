#![forbid(unsafe_code)]
use rocket_mcp::{
    policy::Policy,
    protocol::{Server, Shared, read_line},
    ui::Ui,
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn run() -> Result<(), String> {
    rocket_git::ensure_unprivileged_process().map_err(|error| error.to_string())?;
    let mut args = std::env::args_os().skip(1);
    let mode = args
        .next()
        .ok_or("Usage: rocket-mcp --test-root DIRECTORY | --http-profile DIRECTORY")?;
    let directory = PathBuf::from(args.next().ok_or("Missing directory")?);
    if mode == "--init-http" {
        let root = PathBuf::from(args.next().ok_or("Provide the fixture root")?);
        if !directory.is_absolute() || args.next().is_some() {
            return Err("Provide absolute profile and fixture directories".into());
        }
        let config = rocket_mcp_control::initialize(&directory, &root)
            .map_err(|e| format!("MCP setup failed: {e}"))?;
        println!("{}", config.endpoint());
        return Ok(());
    }
    let public_key = match args.next() {
        None => None,
        Some(flag) if flag == "--windows-ssh-agent" => Some(PathBuf::from(args.next().ok_or("Provide an in-scope public key file")?)),
        _ => return Err("Unknown option; use --windows-ssh-agent PUBLIC_KEY_FILE for the restricted GitHub test profile".into()),
    };
    if !directory.is_absolute() || args.next().is_some() {
        return Err("Provide one absolute directory and an optional --windows-ssh-agent PUBLIC_KEY_FILE".into());
    }
    let http = mode == "--http-profile";
    let _service = if http {
        Some(rocket_mcp_control::service_lock(&directory).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let config = if http {
        Some(rocket_mcp_control::load(&directory).map_err(|_| "Cannot read MCP HTTP settings")?)
    } else if mode == "--test-root" {
        None
    } else {
        return Err("Unknown transport mode".into());
    };
    let root = config.as_ref().map_or(&directory, |c| &c.root);
    let policy = Policy::new(root)?;
    let sources = policy.github_publish_check().map_or_else(
        || rocket_mcp::policy::GITHUB_TEST_SOURCES.iter().map(|s| (*s).to_owned()).collect(),
        |(repository,_)| vec![format!("git@github.com:{repository}.git")]);
    let mut scope = rocket_git::TransportScope::new(&policy.root, sources)
        .map_err(|e| e.to_string())?;
    if let Some(public_key) = public_key {
        scope = scope.with_windows_github_agent(&public_key).map_err(|e| e.to_string())?;
    }
    let (output, responses) = mpsc::sync_channel::<serde_json::Value>(32);
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        window: AtomicU8::new(0),
        transition: Mutex::new(()),
        output,
        pending: Mutex::new(Default::default()),
        context: Mutex::new(None),
        diagnostics: Mutex::new(None),
    });
    let (queue_tx, queue) = mpsc::sync_channel(16);
    let queue = Arc::new(Mutex::new(queue));
    let (start_tx, start) = mpsc::sync_channel(1);
    let mut server = Server::new(shared.clone(), queue_tx, start_tx, policy.clone());
    if http {
        server.ai_profile = Some(directory.clone());
        *shared.diagnostics.lock().unwrap()=Some(rocket_git::DiagnosticLog::open(directory.join("logs")).map_err(|_|"Cannot open MCP diagnostic log")?);
    }
    let mut threads = Vec::new();
    if let Some(config) = config {
        let token =
            rocket_mcp_control::token(&directory).map_err(|_| "Cannot read MCP access token")?;
        threads.push(
            rocket_mcp::http::spawn(config, token, server, responses)
                .map_err(|_| "Cannot bind loopback MCP port; check for an existing service")?,
        );
    } else {
        threads.push(std::thread::spawn(move || {
            let mut out = io::BufWriter::new(io::stdout().lock());
            for value in responses {
                if serde_json::to_writer(&mut out, &value).is_err()
                    || out.write_all(b"\n").is_err()
                    || out.flush().is_err()
                {
                    break;
                }
            }
        }));
        threads.push(std::thread::spawn(move || {
            let mut stdin = io::stdin().lock();
            loop {
                match read_line(&mut stdin) {
                    Ok(Some(line)) => server.handle(&line),
                    Ok(None) => break,
                    Err(_) => {
                        eprintln!(
                            "Rocket MCP input framing failed; closing the owned test session"
                        );
                        break;
                    }
                }
                if server.shared.stop.load(Ordering::Acquire) {
                    break;
                }
            }
            server.shared.shutdown();
        }));
    }
    while !shared.stop.load(Ordering::Acquire) {
        let open = match start.recv_timeout(Duration::from_millis(100)) {
            Ok(open) => open,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        if open.ticket.inactive() {
            finish_window(&shared, &queue, 0, "Window startup was cancelled");
            continue;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "System clock unavailable")?
            .as_nanos();
        let logs = policy
            .root
            .join(format!("mcp-logs-{}-{stamp}", std::process::id()));
        if std::fs::create_dir(&logs).is_err() {
            open.ticket
                .fail("Cannot create the session diagnostic directory");
            finish_window(&shared, &queue, 3, "Window startup failed");
            continue;
        }
        let ui_shared = shared.clone();
        let ui_queue = queue.clone();
        let ui_policy = policy.clone();
        let ui_scope = scope.clone();
        let profile = http.then(|| directory.clone());
        let result = eframe::run_native(
            "Rocket MCP test window",
            eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_title("Rocket")
                    .with_icon(rocket_ui::app_icon::icon())
                    // Fixture input needs no OS focus. An active window takes the
                    // foreground once the desktop has been idle past Windows's lock.
                    .with_active(false)
                    .with_inner_size([1480.0, 920.0])
                    .with_min_inner_size([780.0, 620.0]),
                renderer: eframe::Renderer::Glow,
                run_and_return: true,
                ..Default::default()
            },
            Box::new(move |cc| {
                let mut ui = Ui::new(&cc.egui_ctx, ui_policy, ui_shared, ui_queue, open, logs, ui_scope);
                if let Some(profile) = profile {
                    ui.app.mcp.set_profile(profile, true);
                }
                Ok(Box::new(ui))
            }),
        );
        // eframe reuses its native event loop. Drain the previous window's
        // requests before atomically allowing an explicit reopen.
        finish_window(
            &shared,
            &queue,
            3,
            "Window closed before request delivery; inspect state before retrying",
        );
        if result.is_err() {
            eprintln!(
                "Rocket native window failed; use rocket_open to try opening a fixture again"
            );
        }
    }
    while let Ok(request) = start.try_recv() {
        request.ticket.fail("MCP is stopping");
    }
    while let Ok(request) = queue.lock().unwrap().try_recv() {
        request.ticket.fail("MCP is stopping");
    }
    drop(start);
    drop(queue);
    drop(shared);
    for thread in threads {
        thread.join().map_err(|_| "MCP transport thread failed")?;
    }
    Ok(())
}
fn finish_window(
    shared: &Shared,
    queue: &Mutex<mpsc::Receiver<rocket_mcp::protocol::Request>>,
    window: u8,
    message: &str,
) {
    let _transition = shared.transition.lock().unwrap();
    *shared.context.lock().unwrap() = None;
    shared.window.store(window, Ordering::Release);
    while let Ok(request) = queue.lock().unwrap().try_recv() {
        if request.name == "rocket_state" {
            request
                .ticket
                .ok(rocket_mcp::protocol::idle_window_state(window));
        } else {
            request.ticket.fail(message);
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
