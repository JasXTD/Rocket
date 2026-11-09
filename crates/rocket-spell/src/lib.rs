//! Spell checking for Rocket's commit message editor: the operating system's spell
//! checker on Windows/macOS, and on Linux the system's Hunspell dictionaries read by spellbook.
//! Elsewhere [`SpellChecker::start`] reports [`Error::Unavailable`] and callers hide the
//! feature.
//!
//! The Windows checker is a COM object. One worker thread owns it and initializes
//! COM for itself, so the UI thread (which Windows may have set up for drag and
//! drop) never touches COM. Linux has its worker too; macOS marshals its Cocoa
//! calls to the main queue from the worker (the app must service its run loop).
//! Requests are answered
//! asynchronously through [`Pending`].
use std::{
    ops::Range,
    sync::{Arc, Weak, mpsc},
    thread,
};

mod queue;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

/// Text the checker flags in a checked string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Byte range of the flagged text in the checked string.
    pub range: Range<usize>,
    pub action: Action,
}

/// What the checker recommends for flagged text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Offer the choices from [`SpellChecker::suggest`].
    Suggest,
    /// Replace the text with this one, for example a common typo's correction.
    Replace(String),
    /// Delete the text, for example the second word of "the the".
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This platform or language has no spell checker.
    Unavailable,
    /// The checker failed or its worker stopped; the text says where.
    Failed(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable if cfg!(target_os = "linux") => f.write_str(
                "no Hunspell dictionary is installed for this language (on Ubuntu, hunspell-en-us for English)",
            ),
            Self::Unavailable => f.write_str("no spell checker is available for this language"),
            Self::Failed(detail) => write!(f, "the spell checker failed: {detail}"),
        }
    }
}
impl std::error::Error for Error {}

fn stopped() -> Error {
    Error::Failed("its worker stopped".into())
}

/// The answer to one request. Dropping it cancels queued checks/suggestions;
/// accepted dictionary changes still run even when their answer is discarded.
pub struct Pending<T> {
    answer: mpsc::Receiver<Result<T, Error>>,
    _interest: Arc<()>,
}
impl<T> Pending<T> {
    /// The answer, if it has arrived. A stopped worker answers with an error.
    pub fn poll(&self) -> Option<Result<T, Error>> {
        match self.answer.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(stopped())),
        }
    }
    /// Wait for the answer.
    pub fn wait(self) -> Result<T, Error> {
        self.answer.recv().unwrap_or_else(|_| Err(stopped()))
    }
}

struct Reply<T> {
    answer: mpsc::Sender<Result<T, Error>>,
    interest: Weak<()>,
}
impl<T> Reply<T> {
    fn send(self, result: Result<T, Error>) {
        let _ = self.answer.send(result);
    }
    fn needed(&self) -> bool {
        self.interest.strong_count() != 0
    }
}
fn response<T>() -> (Reply<T>, Pending<T>) {
    let (answer, received) = mpsc::channel();
    let interest = Arc::new(());
    (
        Reply {
            answer,
            interest: Arc::downgrade(&interest),
        },
        Pending {
            answer: received,
            _interest: interest,
        },
    )
}

// Matches the persisted commit-message byte limit and bounds every queued payload.
const MAX_TEXT_BYTES: usize = 128 * 1024;

#[cfg_attr(
    not(any(windows, target_os = "linux", target_os = "macos", test)),
    expect(dead_code, reason = "only the Windows and Linux workers read the text")
)]
enum Request {
    Check(String, Reply<Vec<Misspelling>>),
    Suggest(String, Reply<Vec<String>>),
    Ignore(String, Reply<()>),
    Add(String, Reply<()>),
}
impl Request {
    fn needed(&self) -> bool {
        match self {
            Self::Check(_, reply) => reply.needed(),
            Self::Suggest(_, reply) => reply.needed(),
            Self::Ignore(..) | Self::Add(..) => true,
        }
    }
    fn fail(self, error: Error) {
        match self {
            Self::Check(_, reply) => reply.send(Err(error)),
            Self::Suggest(_, reply) => reply.send(Err(error)),
            Self::Ignore(_, reply) | Self::Add(_, reply) => reply.send(Err(error)),
        }
    }
}

/// One language's checker, owned by its worker thread. Dropping it never waits on
/// the caller: the worker finishes the in-flight request and bounded live work,
/// then releases its COM objects. Discarded queued reads are skipped.
pub struct SpellChecker {
    requests: Option<queue::Sender>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: Option<Pending<()>>,
    language: String,
}

impl SpellChecker {
    /// Start a checker for `language`, a BCP 47 tag such as `en-US`, or for the
    /// user's Windows language when None (falling back to `en-US`). This waits
    /// until the checker is ready, so call it off the UI thread.
    pub fn start(language: Option<&str>) -> Result<Self, Error> {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            Self::spawn(language.map(str::to_owned))
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            let _ = language;
            Err(Error::Unavailable)
        }
    }

    /// [`Self::start`] on a separate thread, for callers that must not wait.
    pub fn start_in_background(language: Option<&str>) -> Pending<Self> {
        let (reply, answer) = response();
        let language = language.map(str::to_owned);
        // If the thread cannot start, the dropped reply reports the stop.
        let _ = thread::Builder::new()
            .name("rocket-spell-start".into())
            .spawn(move || {
                reply.send(Self::start(language.as_deref()));
            });
        answer
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn spawn(language: Option<String>) -> Result<Self, Error> {
        let (requests, received) = queue::channel();
        let (ready, started) = mpsc::channel();
        let (finished, completion) = response();
        let worker = thread::Builder::new()
            .name("rocket-spell".into())
            .spawn(move || {
                let checker = match platform::Checker::new(language.as_deref()) {
                    Ok(checker) => checker,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                let _ = ready.send(Ok(checker.language().to_owned()));
                // Receiver teardown also fails pending requests if this worker panics.
                while let Some(request) = received.next() {
                    match request {
                        Request::Check(text, reply) => {
                            reply.send(checker.check(&text));
                        }
                        Request::Suggest(word, reply) => {
                            reply.send(checker.suggest(&word));
                        }
                        Request::Ignore(word, reply) => {
                            reply.send(checker.ignore(&word));
                        }
                        Request::Add(word, reply) => {
                            reply.send(checker.add(&word));
                        }
                    }
                }
                drop(checker);
                finished.send(Ok(()));
            })
            .map_err(|e| Error::Failed(format!("start its worker: {e}")))?;
        match started.recv() {
            Ok(Ok(language)) => Ok(Self {
                requests: Some(requests),
                worker: Some(worker),
                stopped: Some(completion),
                language,
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(stopped())
            }
        }
    }

    /// The BCP 47 tag of the language being checked.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Stop accepting requests and observe worker cleanup without blocking.
    /// The completion arrives after COM cleanup, or reports a stopped worker.
    pub fn stop(mut self) -> Pending<()> {
        self.requests = None;
        self.stopped
            .take()
            .expect("spell worker has a completion receiver")
    }

    fn ask<T>(&self, text: &str, request: impl FnOnce(String, Reply<T>) -> Request) -> Pending<T> {
        let (reply, answer) = response();
        if text.len() > MAX_TEXT_BYTES {
            reply.send(Err(Error::Failed("text exceeds the 128 KiB limit".into())));
        } else if let Some(requests) = &self.requests {
            requests.send(request(text.to_owned(), reply));
        }
        answer
    }

    /// Flag misspelled text, with byte ranges into `text`.
    pub fn check(&self, text: &str) -> Pending<Vec<Misspelling>> {
        self.ask(text, Request::Check)
    }

    /// Replacement words for `word`, best first; possibly none.
    pub fn suggest(&self, word: &str) -> Pending<Vec<String>> {
        self.ask(word, Request::Suggest)
    }

    /// Stop flagging `word` until Rocket restarts.
    pub fn ignore(&self, word: &str) -> Pending<()> {
        self.ask(word, Request::Ignore)
    }

    /// Add `word` to the user's Windows dictionary, which other applications share, or
    /// on Linux to Rocket's own word list for this language.
    pub fn add(&self, word: &str) -> Pending<()> {
        self.ask(word, Request::Add)
    }
}

impl Drop for SpellChecker {
    fn drop(&mut self) {
        self.requests = None;
        // An OS check cannot be interrupted safely. Its owning thread releases
        // the checker on completion; never join that work from the UI thread.
        drop(self.worker.take());
    }
}

/// Han, kana and Hangul. Windows' checkers do not cover these scripts and flag every
/// word in them, so a flag on a word written only in them is dropped.
#[cfg_attr(not(windows), allow(dead_code))]
fn unchecked_script(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{3134F}'
    )
}

/// The byte range of `text` that covers the UTF-16 code units `start..start + len`,
/// or None when either end falls inside a character.
#[cfg_attr(not(windows), allow(dead_code))]
fn byte_range(text: &str, start: usize, len: usize) -> Option<Range<usize>> {
    let end = start.checked_add(len)?;
    let (mut units, mut first) = (0, None);
    for (byte, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if units == start {
            first = Some(byte);
        }
        if units == end {
            return first.map(|first| first..byte);
        }
        if units > end {
            return None;
        }
        units += c.len_utf16();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_does_not_wait_for_inflight_work_and_skips_discarded_checks() {
        use std::time::Duration;
        let (requests, received) = queue::channel();
        let (started, ready) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let (finished, completed) = mpsc::channel();
        let worker = thread::spawn(move || {
            let first = received.next().unwrap();
            started.send(()).unwrap();
            held.recv().unwrap();
            let mut seen = Vec::new();
            let mut next = Some(first);
            while let Some(Request::Check(text, reply)) = next {
                seen.push(text);
                reply.send(Ok(Vec::new()));
                next = received.next();
            }
            finished.send(seen).unwrap();
        });
        let checker = SpellChecker {
            requests: Some(requests),
            worker: Some(worker),
            stopped: None,
            language: "fixture".into(),
        };
        let first = checker.check("in flight");
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        for i in 0..128 {
            drop(checker.check(&format!("discarded {i}")));
        }
        let latest = checker.check("latest");
        let (dropped, done) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(checker);
            dropped.send(()).unwrap();
        });
        let shutdown = done.recv_timeout(Duration::from_secs(2));
        // Release the fixture worker even if the assertion fails, so a regression
        // does not leave the test's worker or teardown thread blocked.
        release.send(()).unwrap();
        dropper.join().unwrap();
        assert!(
            shutdown.is_ok(),
            "dropping the checker waited for the in-flight check"
        );
        assert_eq!(
            completed.recv_timeout(Duration::from_secs(2)).unwrap(),
            ["in flight", "latest"]
        );
        assert!(first.wait().unwrap().is_empty());
        assert!(latest.wait().unwrap().is_empty());
    }

    #[test]
    fn oversized_payloads_are_refused_without_queueing_them() {
        let (requests, received) = queue::channel();
        let checker = SpellChecker {
            requests: Some(requests),
            worker: None,
            stopped: None,
            language: "fixture".into(),
        };
        let text = "x".repeat(MAX_TEXT_BYTES + 1);
        let expected = Error::Failed("text exceeds the 128 KiB limit".into());
        assert_eq!(checker.check(&text).wait(), Err(expected.clone()));
        assert_eq!(checker.suggest(&text).wait(), Err(expected.clone()));
        assert_eq!(checker.ignore(&text).wait(), Err(expected.clone()));
        assert_eq!(checker.add(&text).wait(), Err(expected));
        drop(checker);
        assert!(received.next().is_none());
    }

    #[test]
    fn utf16_ranges_map_to_byte_ranges() {
        let text = "中文 smple 🚀 tst";
        assert_eq!(byte_range(text, 3, 5), Some(7..12));
        assert_eq!(&text[7..12], "smple");
        // The rocket is two UTF-16 units and four bytes.
        assert_eq!(byte_range(text, 12, 3).map(|r| &text[r]), Some("tst"));
        assert_eq!(byte_range(text, 9, 2).map(|r| &text[r]), Some("🚀"));
        assert_eq!(byte_range(text, 10, 1), None);
        assert_eq!(byte_range(text, 0, 0), Some(0..0));
        assert_eq!(byte_range(text, 15, 1), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_checker_flags_suggests_and_ignores() {
        // en-US ships with Windows; a machine without it cannot run this check.
        let checker = SpellChecker::start(Some("en-US")).expect("the en-US checker");
        assert_eq!(checker.language(), "en-US");
        let text = "中文 This is a smple sentence 🚀 with a tpyo.";
        let found = checker.check(text).wait().unwrap();
        let words: Vec<&str> = found.iter().map(|m| &text[m.range.clone()]).collect();
        assert_eq!(words, ["smple", "tpyo"]);
        // Words only in scripts the checker does not cover are not flagged.
        assert!(checker.check("修复 検索 한국어").wait().unwrap().is_empty());
        assert!(!unchecked_script('a') && unchecked_script('検'));
        let suggestions = checker.suggest("smple").wait().unwrap();
        assert!(suggestions.iter().any(|s| s == "simple"), "{suggestions:?}");
        // A repeated word is flagged for deletion.
        let repeated = checker.check("Fix the the bug").wait().unwrap();
        assert!(
            repeated
                .iter()
                .any(|m| m.action == Action::Delete && m.range.start > 4),
            "{repeated:?}"
        );
        // Text after a NUL is still checked.
        let found = checker.check("ok\0smple").wait().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range, 3..8);
        // Ignored words stay ignored for this checker only.
        checker.ignore("smple").wait().unwrap();
        assert!(checker.check("smple").wait().unwrap().is_empty());
        assert!(checker.check("").wait().unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn asynchronous_stop_finishes_live_requests_and_reports_cleanup() {
        let checker = SpellChecker::start(Some("en-US")).expect("the en-US checker");
        let pending = checker.check("smple");
        let stopped = checker.stop();
        assert_eq!(pending.wait().unwrap().len(), 1);
        stopped.wait().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn an_unknown_language_is_unavailable_and_a_stopped_checker_answers_with_an_error() {
        assert!(matches!(
            SpellChecker::start(Some("zz-ZZ")),
            Err(Error::Unavailable)
        ));
        let mut checker = SpellChecker::start(Some("en-US")).expect("the en-US checker");
        let pending = checker.check("smple");
        let worker = checker.worker.take().unwrap();
        drop(checker);
        // Live requests sent before the drop are still answered. Joining here is
        // test-only and verifies that the real COM worker releases its resources.
        assert!(matches!(pending.wait(), Ok(found) if found.len() == 1));
        worker.join().unwrap();
    }
}
