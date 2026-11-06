use std::sync::mpsc::SyncSender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchPhase {
    Enumerating,
    Counting,
    Compressing,
    Receiving,
    Resolving,
    CheckingLfsObjects,
    UploadingLfsObjects,
    CheckingRepository,
}
impl FetchPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Enumerating => "Enumerating",
            Self::Counting => "Counting",
            Self::Compressing => "Compressing",
            Self::Receiving => "Receiving",
            Self::Resolving => "Resolving",
            Self::CheckingLfsObjects => "Checking LFS objects",
            Self::UploadingLfsObjects => "Uploading LFS objects",
            Self::CheckingRepository => "Checking repository",
        }
    }
}

/// Best-effort progress from known Git stderr records. This contains only phase
/// and object counts, never remote-provided prose, URLs, paths, or credentials.
/// Phases can repeat or be absent; these counts are not an overall percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchProgress {
    pub phase: FetchPhase,
    pub current: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClonePhase {
    Preparing,
    Cloning,
    Verifying,
    Publishing,
    Opening,
    /// Previews and Keep: downloading file contents after the clone itself.
    DownloadingFiles,
}

/// Bounded, best-effort events without source/destination names or helper prose.
/// Counts belong to one phase, not the entire clone. The report signals completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneProgress {
    Phase(ClonePhase),
    Transfer(FetchProgress),
    Checkout { current: u64, total: Option<u64> },
    /// Previews and Keep: file contents downloaded so far, of `total`.
    Files { done: u64, total: u64 },
}

#[derive(Clone, Copy)]
pub(crate) enum ProgressSink<'a> {
    Fetch(&'a SyncSender<FetchProgress>),
    Clone(&'a SyncSender<CloneProgress>),
    Lfs(&'a SyncSender<FetchProgress>),
}

pub(crate) struct ProgressParser<'a> {
    sink: ProgressSink<'a>,
    line: Vec<u8>,
    oversized: bool,
}

impl<'a> ProgressParser<'a> {
    pub(crate) fn new(sink: ProgressSink<'a>) -> Self {
        Self {
            sink,
            line: Vec::new(),
            oversized: false,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if matches!(byte, b'\r' | b'\n') {
                self.finish();
            } else if self.line.len() < 4096 {
                self.line.push(byte);
            } else {
                self.oversized = true;
            }
        }
    }

    pub(crate) fn finish(&mut self) {
        if !self.oversized {
            if let ProgressSink::Lfs(sender) = self.sink
                && let Some((current, total)) = std::str::from_utf8(&self.line)
                    .ok()
                    .and_then(|s| s.trim().strip_prefix("Uploading LFS objects: "))
                    .and_then(parse_counts)
                && total.is_some_and(|n| n <= 2000)
            {
                let _ = sender.try_send(FetchProgress {
                    phase: FetchPhase::UploadingLfsObjects,
                    current,
                    total,
                });
            }
            // A slow/disconnected UI must never block the pipe reader or Git.
            // Full queues intentionally lose intermediate updates. The fetch
            // report, not receipt of a final progress event, signals completion.
            match (self.sink, parse(&self.line)) {
                (ProgressSink::Fetch(sender), Some(update)) => {
                    let _ = sender.try_send(update);
                }
                (ProgressSink::Clone(sender), Some(update)) => {
                    let _ = sender.try_send(CloneProgress::Transfer(update));
                }
                (ProgressSink::Clone(sender), None) => {
                    if let Some(counts) = std::str::from_utf8(&self.line)
                        .ok()
                        .and_then(|s| s.trim().strip_prefix("Updating files: "))
                        .and_then(parse_counts)
                    {
                        let _ = sender.try_send(CloneProgress::Checkout {
                            current: counts.0,
                            total: counts.1,
                        });
                    }
                }
                _ => (),
            }
        }
        self.line.clear();
        self.oversized = false;
    }
}

fn parse(bytes: &[u8]) -> Option<FetchProgress> {
    let line = std::str::from_utf8(bytes).ok()?.trim();
    let line = line.strip_prefix("remote: ").unwrap_or(line);
    let phases = [
        ("Enumerating objects: ", FetchPhase::Enumerating),
        ("Counting objects: ", FetchPhase::Counting),
        ("Compressing objects: ", FetchPhase::Compressing),
        ("Receiving objects: ", FetchPhase::Receiving),
        ("Resolving deltas: ", FetchPhase::Resolving),
    ];
    let (rest, phase) = phases
        .iter()
        .find_map(|(prefix, phase)| line.strip_prefix(prefix).map(|rest| (rest.trim(), *phase)))?;
    let (current, total) = parse_counts(rest)?;
    Some(FetchProgress {
        phase,
        current,
        total,
    })
}

fn parse_counts(rest: &str) -> Option<(u64, Option<u64>)> {
    Some(if let Some((_, counts)) = rest.split_once('(') {
        let (counts, _) = counts.split_once(')')?;
        let (current, total) = counts.split_once('/')?;
        let current = number(current)?;
        let total = number(total)?;
        if current > total {
            return None;
        }
        (current, Some(total))
    } else {
        (number(rest.split(',').next()?.trim())?, None)
    })
}

fn number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    #[test]
    fn lfs_progress_is_bounded_numeric_and_does_not_accept_git_or_private_output() {
        let (tx, rx) = sync_channel(2);
        let mut p = ProgressParser::new(ProgressSink::Lfs(&tx));
        p.feed(b"PRIVATE credentials\nReceiving objects: 100% (1/1)\nUploading LFS obj");
        p.feed(b"ects: 50% (1/2), PRIVATE name\rUploading LFS objects: 100% (2/2), 40 B | 0 B/s\n");
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            FetchProgress {
                phase: FetchPhase::UploadingLfsObjects,
                current: 1,
                total: Some(2)
            }
        );
        for line in [
            "Uploading LFS objects: 100% (3/2)",
            "Uploading LFS objects: 0% (0/2001)",
            "Uploading LFS objects: PRIVATE",
            "Uploading LFS objects: 50% (-1/2)",
        ] {
            p.feed(line.as_bytes());
            p.feed(b"\n");
        }
        p.feed(&vec![b'x'; 10000]);
        p.feed(b"\n");
        assert_eq!(rx.try_iter().count(), 0);
        for _ in 0..100 {
            p.feed(b"Uploading LFS objects: 100% (2/2)\n");
        }
        assert_eq!(rx.try_iter().count(), 2);
        drop(rx);
        p.feed(b"Uploading LFS objects: 100% (2/2)\n");
    }

    #[test]
    fn parses_split_carriage_returns_without_forwarding_remote_prose() {
        let (sender, receiver) = sync_channel(8);
        let mut parser = ProgressParser::new(ProgressSink::Fetch(&sender));
        parser.feed(b"remote: PRIVATE secret\nremote: Counting obj");
        parser.feed(b"ects:  50% (2/4)\r\nReceiving objects: 100% (4/4), done.\r");
        parser.feed(b"Enumerating objects: 4, done.");
        parser.finish();
        let updates: Vec<_> = receiver.try_iter().collect();
        assert_eq!(updates.len(), 3);
        assert_eq!(
            updates[0],
            FetchProgress {
                phase: FetchPhase::Counting,
                current: 2,
                total: Some(4)
            }
        );
        assert_eq!(updates[1].phase, FetchPhase::Receiving);
        assert_eq!(updates[2].total, None);
    }

    #[test]
    fn clone_counts_checkout_and_transfer_without_forwarding_payloads() {
        let (sender, receiver) = sync_channel(2);
        let mut parser = ProgressParser::new(ProgressSink::Clone(&sender));
        parser.feed(b"PRIVATE URL token\nUpdating fi");
        parser.feed(b"les: 50% (1/2)\rReceiving objects: 100% (3/3), PRIVATE\n");
        assert_eq!(
            receiver.try_recv().unwrap(),
            CloneProgress::Checkout {
                current: 1,
                total: Some(2)
            }
        );
        assert!(matches!(
            receiver.try_recv().unwrap(),
            CloneProgress::Transfer(FetchProgress {
                phase: FetchPhase::Receiving,
                current: 3,
                total: Some(3)
            })
        ));
        for _ in 0..1000 {
            parser.feed(b"Updating files: 100% (2/2)\r");
        }
        assert_eq!(receiver.try_iter().count(), 2);
        drop(receiver);
        parser.feed(b"Updating files: 100% (2/2)\r");
    }

    #[test]
    fn bounds_lines_and_handles_full_or_disconnected_consumers() {
        let (sender, receiver) = sync_channel(1);
        let mut parser = ProgressParser::new(ProgressSink::Fetch(&sender));
        parser.feed(&vec![b'x'; 100_000]);
        assert_eq!(parser.line.len(), 4096);
        parser.feed(b"\rCounting objects: 100% (1/1)\r");
        for _ in 0..1000 {
            parser.feed(b"Counting objects: 100% (2/2)\r");
        }
        assert_eq!(receiver.try_iter().count(), 1);
        drop(receiver);
        parser.feed(b"Counting objects: 100% (3/3)\r");
        for invalid in [
            "Counting objects: 101% (2/1)",
            "Receiving objects: 0% (x/2)",
            "Enumerating objects: 18446744073709551616",
            "PRIVATE 1/1",
        ] {
            assert!(parse(invalid.as_bytes()).is_none());
        }
    }
}
