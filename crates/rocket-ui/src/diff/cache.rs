//! Bounded memoization of display parsing, never Git state or write authority.
use super::{FilePatch, Patch};
use std::{collections::VecDeque, sync::Mutex};

const MAX_ENTRIES: usize = 8;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cache(Mutex<Entries>);

#[derive(Default)]
struct Entries {
    items: VecDeque<Entry>,
    bytes: usize,
}

struct Entry {
    source: Vec<u8>,
    files: Vec<FilePatch>,
    limited: bool,
    bytes: usize,
}

impl Cache {
    pub fn parse(&self, source: &[u8]) -> Patch {
        // Only exact bytes hit. Metadata, images, submodules and sealed reviews
        // are attached by the caller from this request's fresh repository reads.
        if let Some(patch) = self.lookup(source) {
            return patch;
        }
        // Parsing does not hold the shared lock: other tabs can read concurrently.
        let patch = Patch::parse(source);
        let bytes = source.len() + display_bytes(&patch.files);
        if bytes <= MAX_BYTES
            && let Ok(mut entries) = self.0.lock()
        {
            // Two simultaneous misses can compute the same display data.
            if !entries.items.iter().any(|entry| entry.source == source) {
                while entries.items.len() >= MAX_ENTRIES || entries.bytes + bytes > MAX_BYTES {
                    if let Some(entry) = entries.items.pop_front() {
                        entries.bytes -= entry.bytes;
                    }
                }
                entries.bytes += bytes;
                entries.items.push_back(Entry {
                    source: source.to_vec(),
                    files: patch.files.clone(),
                    limited: patch.limited,
                    bytes,
                });
            }
        }
        patch
    }

    fn lookup(&self, source: &[u8]) -> Option<Patch> {
        // Cache failures only disable reuse; the authoritative read still runs.
        let mut entries = self.0.lock().ok()?;
        let index = entries
            .items
            .iter()
            .position(|entry| entry.source == source)?;
        let entry = entries.items.remove(index)?;
        let mut patch = Patch {
            files: entry.files.clone(),
            limited: entry.limited,
            ..Default::default()
        };
        // Search/word workers are scoped to each displayed patch. Reusing text
        // must not reuse a cancelled worker's generation or selection state.
        patch.assign_ids();
        entries.items.push_back(entry);
        Some(patch)
    }
}

fn display_bytes(files: &[FilePatch]) -> usize {
    std::mem::size_of_val(files)
        + files
            .iter()
            .map(|file| {
                file.label.capacity()
                    + file.lines.capacity() * std::mem::size_of::<super::Line>()
                    + file
                        .lines
                        .iter()
                        .map(|line| {
                            line.text.capacity()
                                + line.syntax.capacity()
                                    * std::mem::size_of::<crate::diff_tools::Span>()
                        })
                        .sum::<usize>()
                    + file.visible.capacity() * std::mem::size_of::<usize>()
                    + file.split.capacity() * std::mem::size_of::<(Option<usize>, Option<usize>)>()
            })
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn source(value: &str) -> Vec<u8> {
        format!("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+{value}\n")
            .into_bytes()
    }

    #[test]
    fn exact_bytes_share_display_data_but_never_selection_or_metadata() {
        let cache = Cache::default();
        let bytes = source("你好");
        let mut first = cache.parse(&bytes);
        first.selected.insert(5);
        first.notice = Some("private review notice".into());
        first.files[0].widths = [999.0, 999.0];
        let second = cache.parse(&bytes);
        assert!(Arc::ptr_eq(&first.files[0].lines, &second.files[0].lines));
        assert_ne!(first.files[0].id, second.files[0].id);
        assert!(second.selected.is_empty());
        assert!(second.notice.is_none() && second.review.is_none());
        assert!(second.files[0].command_path.is_none());
        assert_eq!(second.files[0].widths, [0.0, 0.0]);
        let changed = cache.parse(&source("changed"));
        assert!(!Arc::ptr_eq(&first.files[0].lines, &changed.files[0].lines));
        assert!(
            changed.files[0]
                .lines
                .iter()
                .any(|line| line.text == "+changed")
        );
    }

    #[test]
    fn recently_used_entries_survive_and_limits_bound_retention() {
        let cache = Cache::default();
        let first = cache.parse(&source("first"));
        let old = cache.parse(&source("old"));
        for n in 2..MAX_ENTRIES {
            cache.parse(&source(&n.to_string()));
        }
        cache.parse(&source("first"));
        cache.parse(&source("new"));
        assert!(Arc::ptr_eq(
            &first.files[0].lines,
            &cache.parse(&source("first")).files[0].lines
        ));
        assert!(!Arc::ptr_eq(
            &old.files[0].lines,
            &cache.parse(&source("old")).files[0].lines
        ));
        let entries = cache.0.lock().unwrap();
        assert_eq!(entries.items.len(), MAX_ENTRIES);
        assert!(entries.bytes <= MAX_BYTES);
    }

    #[test]
    fn byte_budget_evicts_before_the_entry_limit() {
        let cache = Cache::default();
        for n in 0..5 {
            let mut bytes = vec![b'x'; 9 * 1024 * 1024];
            bytes[0] = b'0' + n;
            assert!(cache.parse(&bytes).limited);
            let entries = cache.0.lock().unwrap();
            assert!(entries.bytes <= MAX_BYTES);
            assert!(entries.items.len() < MAX_ENTRIES);
        }
        assert_eq!(cache.0.lock().unwrap().items.len(), 3);
    }

    #[test]
    fn oversized_input_is_displayed_without_retaining_it() {
        let cache = Cache::default();
        let patch = cache.parse(&vec![b'x'; MAX_BYTES + 1]);
        assert!(patch.limited);
        assert!(!patch.files.is_empty());
        assert!(cache.0.lock().unwrap().items.is_empty());
    }
}
