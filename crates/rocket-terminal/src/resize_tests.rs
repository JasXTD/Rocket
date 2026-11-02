use super::*;
use std::{
    fs, thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn wait(session: &Session, condition: impl Fn(&Screen) -> bool) -> Screen {
    let start = Instant::now();
    loop {
        assert!(session.status().error.is_none(), "{:?}", session.status());
        // Snapshot is deliberately nonblocking and may be unavailable while the
        // terminal reader owns the screen lock. Keep polling within the deadline.
        let Some(screen) = session.snapshot(0) else {
            assert!(
                start.elapsed() < Duration::from_secs(12),
                "No terminal snapshot available"
            );
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        if condition(&screen) {
            return screen;
        }
        assert!(
            start.elapsed() < Duration::from_secs(12),
            "cursor {:?}: {}",
            screen.cursor,
            screen.text()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn long_prompt_survives_resize() {
    let root = std::env::current_dir()
        .unwrap()
        .join("../../work")
        .join(format!(
            "resize-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let root = root.canonicalize().unwrap_or(root);
    let repo = root.join("long-repository-directory-to-wrap-the-shell-prompt");
    fs::create_dir_all(&repo).unwrap();
    let session =
        Session::start_with_history(&repo, 180, 20, Arc::new(|| {}), &root.join("history.log"))
            .unwrap();
    wait(&session, |s| s.text().trim_end().ends_with('>'));
    session.resize(94, 8);
    wait(&session, |s| s.columns == 94 && s.rows == 8);
    thread::sleep(Duration::from_millis(300));
    session.input(b"Write-Outp\t").unwrap();
    let s = wait(&session, |s| s.text().contains("Write-Output"));
    let visible: String = s
        .cells
        .iter()
        .flat_map(|row| row.iter().filter(|c| c.width != 0).map(Cell::text))
        .collect();
    assert!(
        visible.contains("long-repository-directory-to-wrap-the-shell-prompt"),
        "{visible}"
    );
    assert!(s.text().lines().any(|l| l.trim_end() == "> Write-Output"));
    session.resize(180, 20);
    wait(&session, |s| s.columns == 180);
    thread::sleep(Duration::from_millis(300));
    session.input(b" 'ROCKET_RESIZE_OK'").unwrap();
    let s = wait(&session, |s| {
        s.text().trim_end().ends_with("ROCKET_RESIZE_OK'")
    });
    assert_eq!(
        s.text()
            .lines()
            .filter(|l| l.starts_with("> Write-Output"))
            .count(),
        1,
        "{}",
        s.text()
    );
    assert!(
        s.text()
            .contains("long-repository-directory-to-wrap-the-shell-prompt")
    );
    assert!(
        s.text()
            .lines()
            .any(|l| l.trim_end() == "> Write-Output 'ROCKET_RESIZE_OK'")
    );
    session.input(b"\r").unwrap();
    wait(&session, |s| {
        s.text().lines().any(|l| l == "ROCKET_RESIZE_OK") && s.text().trim_end().ends_with('>')
    });
    session.stop();
    let deadline = Instant::now();
    while !session.status().closed {
        assert!(deadline.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(25));
    }
}
