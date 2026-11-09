//! Focused real Cocoa spelling check. No words are added to the shared dictionary.
#[cfg(target_os = "macos")]
fn main() {
    use std::{
        ffi::c_void,
        sync::mpsc,
        time::{Duration, Instant},
    };
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| {
            let checker =
                rocket_spell::SpellChecker::start(Some("en-US")).expect("native English checker");
            let text = "資料 🚀 cafe\u{301} mispellling";
            let found = checker.check(text).wait().unwrap();
            assert!(
                found
                    .iter()
                    .any(|f| &text[f.range.clone()] == "mispellling"),
                "real misspelling after Unicode prefix"
            );
            for entry in found {
                assert!(
                    text.is_char_boundary(entry.range.start)
                        && text.is_char_boundary(entry.range.end)
                );
            }
            assert!(checker.check("hello world").wait().unwrap().is_empty());
            assert!(!checker.suggest("mispellling").wait().unwrap().is_empty());
            checker.ignore("mispellling").wait().unwrap();
            assert!(checker.check("mispellling").wait().unwrap().is_empty());
            for _ in 0..100 {
                drop(checker.check("discarded checking"));
            }
            checker.stop().wait().unwrap();
            assert!(rocket_spell::SpellChecker::start(Some("zz-INVALID")).is_err());
            let again = rocket_spell::SpellChecker::start(Some("en-US")).unwrap();
            assert!(
                !again.check("mispellling").wait().unwrap().is_empty(),
                "ignored word must not persist"
            );
            again.stop().wait().unwrap();
        });
        tx.send(result.is_ok()).unwrap();
    });
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        if let Ok(passed) = rx.try_recv() {
            assert!(passed, "native spell checks failed");
            break;
        }
        assert!(Instant::now() < until, "Cocoa spelling timed out");
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.02, 1);
        }
    }
    println!(
        "PASS native Cocoa spelling: Unicode, suggestions, session Ignore, discarded work, restart and unavailable language"
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("This probe requires native macOS");
}
