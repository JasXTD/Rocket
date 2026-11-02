//! Display-only patch parsing. Parsed labels are never used as Git arguments.
use std::{collections::BTreeSet, sync::Arc};
pub(crate) mod cache;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Context,
    Added,
    Removed,
    Hunk,
    Header,
}
#[derive(Clone, Debug)]
pub struct Line {
    pub raw_index: usize,
    pub syntax: Vec<crate::diff_tools::Span>,
    pub text: String,
    pub kind: Kind,
    pub old: Option<u64>,
    pub new: Option<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct FilePatch {
    pub label: String,
    /// Exact NUL-delimited Git identity, never reconstructed from a display label.
    pub command_path: Option<rocket_git::RepoPath>,
    pub syntax_language: Option<&'static str>,
    pub syntax_limited: bool,
    pub syntax_spans: usize,
    pub image: Option<rocket_git::ImageComparison>,
    pub submodule: Option<rocket_git::SubmoduleDiff>,
    pub lines: Arc<Vec<Line>>,
    pub id: u64,
    pub added: usize,
    pub removed: usize,
    pub visible: Vec<usize>,
    pub split: Vec<(Option<usize>, Option<usize>)>,
    /// Largest measured row in unified/split layouts, extended as rows are visited.
    pub widths: [f32; 2],
    /// Row-space summaries for unified and split views, prepared on the read worker.
    pub overview: [crate::overview::Summary; 2],
}
#[derive(Debug, Default)]
pub struct Patch {
    pub files: Vec<FilePatch>,
    pub command_commit: Option<rocket_git::ObjectId>,
    pub limited: bool,
    pub review: Option<Arc<rocket_git::ChangeReview>>,
    pub selected: BTreeSet<usize>,
    pub notice: Option<String>,
}

impl Patch {
    /// Bind only when the parsed display label is unique in both lists. The
    /// argument identity always comes from Git's raw metadata, never the label.
    pub fn bind_command_paths(&mut self, commit:rocket_git::ObjectId, files:&[rocket_git::ImageComparison]) {
        let mut paths=std::collections::HashMap::new();
        for file in files {
            paths.entry(crate::text::raw_path_label(file.path().as_bytes()))
                .and_modify(|p|*p=None).or_insert(Some(file.path().clone()));
        }
        let mut counts=std::collections::HashMap::new();
        for file in &self.files { *counts.entry(&file.label).or_insert(0usize)+=1; }
        let unique:std::collections::HashSet<_>=counts.into_iter().filter(|(_,n)|*n==1).map(|(s,_)|s.clone()).collect();
        for file in &mut self.files { file.command_path=unique.contains(&file.label).then(||paths.get(&file.label).cloned().flatten()).flatten(); }
        self.command_commit=Some(commit);
    }
    pub fn load_submodules(&mut self, repo: &rocket_git::Repository, options: &rocket_git::DiffOptions, cancel: &rocket_git::CancellationToken) -> rocket_git::Result<()> {
        if !self.needs_submodules() { return Ok(()); }
        self.bind_submodules(repo.submodule_diffs(options, cancel)?);
        Ok(())
    }
    pub(crate) fn needs_submodules(&self) -> bool {
        self.files.iter().any(|f| f.lines.iter().any(|l| l.text.contains("Subproject commit ")))
    }
    pub(crate) fn load_submodules_for_files(&mut self, repo: &rocket_git::Repository, files: &[rocket_git::ImageComparison], cancel: &rocket_git::CancellationToken) -> rocket_git::Result<()> {
        if self.needs_submodules() { self.bind_submodules(repo.submodule_diffs_for_files(files, cancel)?); }
        Ok(())
    }
    fn bind_submodules(&mut self, summaries: Vec<rocket_git::SubmoduleDiff>) {
        let mut seen = std::collections::HashSet::new();
        for summary in summaries {
            let label = crate::text::raw_path_label(summary.path.as_bytes());
            let matches: Vec<_> = self.files.iter_mut().filter(|f| f.label == label).collect();
            // Multiple raw paths can have the same display label. As for image
            // metadata, attach no summary unless both lists are unambiguous.
            if matches.len() == 1 { matches.into_iter().next().unwrap().submodule = seen.insert(label).then_some(summary); }
        }
    }
    pub fn bind_images(&mut self, images: Vec<rocket_git::ImageComparison>) {
        let mut labels = std::collections::HashMap::new();
        for (i, file) in self.files.iter_mut().enumerate() {
            file.image = None;
            labels
                .entry(file.label.clone())
                .and_modify(|index| *index = None)
                .or_insert(Some(i));
        }
        let mut seen = std::collections::HashSet::new();
        for image in images {
            // Match display labels only to attach metadata; arguments always use
            // the original raw path/object identities issued by the backend.
            let label = crate::text::raw_path_label(image.path().as_bytes());
            if let Some(Some(index)) = labels.get(&label) {
                // Both lists must be unambiguous. A limited patch may omit a
                // second raw path that has the same lossy display label.
                self.files[*index].image = seen.insert(label).then_some(image);
            }
        }
    }
    pub fn image_preview(image: rocket_git::ImageComparison) -> Self {
        let mut patch = Self::default();
        patch.files.push(FilePatch {
            label: crate::text::raw_path_label(image.path().as_bytes()),
            image: Some(image),
            ..Default::default()
        });
        patch.layout();
        patch
    }
    pub fn parse(bytes: &[u8]) -> Self {
        let mut result = Self::default();
        let (mut old, mut new) = (0_u64, 0_u64);
        let mut in_hunk = false;
        for (index, raw) in bytes.split(|b| *b == b'\n').enumerate() {
            if index >= 50_000 {
                result.limited = true;
                break;
            }
            let raw = if raw.len() > 8192 {
                result.limited = true;
                crate::text::prefix(raw, 8192)
            } else {
                raw
            };
            let text = String::from_utf8_lossy(raw)
                .trim_end_matches('\r')
                .to_owned();
            if let Some(header) = text.strip_prefix("diff --git ") {
                let label = header_label(header);
                result.files.push(FilePatch {
                    label,
                    ..Default::default()
                });
                in_hunk = false;
            }
            if result.files.is_empty() {
                if text.is_empty() {
                    continue;
                }
                result.files.push(FilePatch {
                    label: "Patch".into(),
                    ..Default::default()
                });
            }
            let file = result.files.last_mut().expect("file inserted");
            if !in_hunk
                && let Some(path) = text
                    .strip_prefix("--- ")
                    .or_else(|| text.strip_prefix("+++ "))
                && path != "/dev/null"
            {
                file.label = crate::text::diff_file_label(path);
            }
            let mut line = Line {
                raw_index: index,
                syntax: Vec::new(),
                text,
                kind: Kind::Header,
                old: None,
                new: None,
            };
            if let Some(rest) = line.text.strip_prefix("@@ -") {
                let mut fields = rest.split_whitespace();
                let before = fields
                    .next()
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.parse().ok());
                let after = fields
                    .next()
                    .and_then(|s| s.strip_prefix('+'))
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.parse().ok());
                if let (Some(before), Some(after)) = (before, after) {
                    old = before;
                    new = after;
                    in_hunk = true;
                }
                line.kind = Kind::Hunk;
            } else if in_hunk {
                match line.text.as_bytes().first() {
                    Some(b'+') => {
                        line.kind = Kind::Added;
                        line.new = Some(new);
                        new = new.saturating_add(1);
                        file.added += 1;
                    }
                    Some(b'-') => {
                        line.kind = Kind::Removed;
                        line.old = Some(old);
                        old = old.saturating_add(1);
                        file.removed += 1;
                    }
                    Some(b' ') => {
                        line.kind = Kind::Context;
                        line.old = Some(old);
                        line.new = Some(new);
                        old = old.saturating_add(1);
                        new = new.saturating_add(1);
                    }
                    _ => {}
                }
            }
            Arc::make_mut(&mut file.lines).push(line);
        }
        result.layout();
        result
    }

    pub fn from_review(review: rocket_git::ChangeReview) -> Self {
        Self::parse(review.patch()).with_review(review)
    }

    pub(crate) fn with_review(mut self, review: rocket_git::ChangeReview) -> Self {
        self.notice = review.selection_unavailable().or_else(|| review.selection_note()).map(str::to_owned);
        self.review = Some(Arc::new(review));
        self
    }

    pub fn preview(path: &rocket_git::RepoPath, preview: rocket_git::FilePreview) -> Self {
        let mut patch = Self {
            limited: preview.truncated,
            notice: Some(
                if preview.binary {
                    "Binary file. Use Stage file to include it."
                } else {
                    "Untracked file preview. Stage the whole file to include it."
                }
                .into(),
            ),
            ..Default::default()
        };
        let mut file = FilePatch {
            label: path.display_lossy().into_owned(),
            ..Default::default()
        };
        if !preview.binary {
            for (i, bytes) in preview
                .bytes
                .split_inclusive(|b| *b == b'\n')
                .take(50_001)
                .enumerate()
            {
                if i == 50_000 {
                    patch.limited = true;
                    break;
                }
                if bytes.len() > 8192 {
                    patch.limited = true;
                }
                Arc::make_mut(&mut file.lines).push(Line {
                    raw_index: i,
                    syntax: Vec::new(),
                    text: format!(
                        "+{}",
                        String::from_utf8_lossy(crate::text::prefix(bytes, 8192))
                            .trim_end_matches(['\r', '\n'])
                    ),
                    kind: Kind::Added,
                    old: None,
                    new: Some(i as u64 + 1),
                });
            }
            file.added = file.lines.len();
        }
        patch.files.push(file);
        patch.layout();
        patch
    }

    fn assign_ids(&mut self) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        for file in &mut self.files {
            file.id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn layout(&mut self) {
        // Compute once on the read worker, rather than scan the whole file every frame.
        self.assign_ids();
        for file in &mut self.files {
            crate::diff_tools::prepare(file);
            file.visible = file
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    line.kind != Kind::Header
                        || !(line.text.starts_with("diff --git ")
                            || line.text.starts_with("index ")
                            || line.text.starts_with("--- ")
                            || line.text.starts_with("+++ "))
                })
                .map(|(i, _)| i)
                .collect();
            let mut cursor = 0;
            while cursor < file.visible.len() {
                let i = file.visible[cursor];
                if matches!(file.lines[i].kind, Kind::Removed | Kind::Added) {
                    let mut removed = Vec::new();
                    let mut added = Vec::new();
                    while cursor < file.visible.len() {
                        let i = file.visible[cursor];
                        match file.lines[i].kind {
                            Kind::Removed => removed.push(i),
                            Kind::Added => added.push(i),
                            _ => break,
                        }
                        cursor += 1;
                    }
                    for n in 0..removed.len().max(added.len()) {
                        file.split
                            .push((removed.get(n).copied(), added.get(n).copied()));
                    }
                } else {
                    file.split.push((Some(i), Some(i)));
                    cursor += 1;
                }
            }
            let flags = |i: usize| match file.lines[i].kind {
                Kind::Added => crate::overview::ADDED,
                Kind::Removed => crate::overview::REMOVED,
                _ => 0,
            };
            file.overview = [
                crate::overview::Summary::from_rows(file.visible.iter().map(|i| flags(*i))),
                crate::overview::Summary::from_rows(file.split.iter().map(|(a,b)| a.map_or(0, flags) | b.map_or(0, flags))),
            ];
        }
    }
}

fn header_label(header: &str) -> String {
    if let Some((_, used)) = crate::text::unquote_path(header.as_bytes())
        && let Some(destination) = header[used..].strip_prefix(' ')
    {
        return crate::text::path_label(destination);
    }
    if let Some((_, destination)) = header.rsplit_once(" \"b/") {
        return crate::text::path_label(&format!("\"b/{destination}"));
    }
    // For unquoted same-path headers, choose the separator whose paths agree.
    // Text patches also supply unambiguous ---/+++ records, including renames.
    for (i, _) in header.match_indices(" b/") {
        if header[..i].strip_prefix("a/") == Some(&header[i + 3..]) {
            return crate::text::path_label(&header[i + 1..]);
        }
    }
    header.rsplit_once(" b/").map_or_else(
        || crate::text::path_label(header),
        |(_, path)| crate::text::path_label(&format!("b/{path}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_metadata_never_guesses_ambiguous_or_missing_paths() {
        crate::test_common::isolated(
            "ROCKET_COMMAND_BINDING_CASE",
            "diff::tests::command_metadata_never_guesses_ambiguous_or_missing_paths",
            || {
                let fixture = crate::test_common::Fixture::new();
                let repo = fixture.repo();
                let cancel = rocket_git::CancellationToken::new();
                let descriptors: Vec<_> = [0xff, 0xfe]
                    .into_iter()
                    .map(|byte| {
                        repo.untracked_image(
                            &rocket_git::RepoPath::new(vec![byte, b'.', b'p', b'n', b'g']).unwrap(),
                            &cancel,
                        ).unwrap()
                    })
                    .collect();
                let commit = rocket_git::ObjectId::parse(&"1".repeat(40)).unwrap();
                let mut patch = Patch::parse(
                    b"diff --git \"a/\\377.png\" \"b/\\377.png\"\nBinary files differ\n",
                );
                patch.bind_command_paths(commit.clone(), &descriptors);
                assert!(patch.files[0].command_path.is_none());
                patch.bind_command_paths(commit.clone(), &descriptors[..1]);
                assert_eq!(patch.command_commit.as_ref(), Some(&commit));
                assert_eq!(patch.files[0].command_path.as_ref(), Some(descriptors[0].path()));
                patch.files.push(FilePatch {
                    label: patch.files[0].label.clone(),
                    ..Default::default()
                });
                patch.bind_command_paths(commit.clone(), &descriptors[..1]);
                assert!(patch.files.iter().all(|file| file.command_path.is_none()));
                patch.files.truncate(1);
                patch.files[0].label = "unmatched.txt".into();
                patch.bind_command_paths(commit, &descriptors);
                assert!(patch.files[0].command_path.is_none());
            },
        );
    }

    #[test]
    fn image_metadata_does_not_bind_ambiguous_lossy_labels() {
        crate::test_common::isolated(
            "ROCKET_IMAGE_BINDING_CASE",
            "diff::tests::image_metadata_does_not_bind_ambiguous_lossy_labels",
            || {
                let fixture = crate::test_common::Fixture::new();
                let repo = fixture.repo();
                let cancel = rocket_git::CancellationToken::new();
                let descriptors = || {
                    [0xff, 0xfe, 0xfd]
                        .into_iter()
                        .map(|byte| {
                            repo.untracked_image(
                                &rocket_git::RepoPath::new(vec![byte, b'.', b'p', b'n', b'g'])
                                    .unwrap(),
                                &cancel,
                            )
                            .unwrap()
                        })
                        .collect::<Vec<_>>()
                };
                let mut patch = Patch::parse(
                    b"diff --git \"a/\\377.png\" \"b/\\377.png\"\nBinary files differ\n",
                );
                patch.bind_images(descriptors());
                assert!(patch.files[0].image.is_none());
                patch.bind_images(descriptors().into_iter().take(1).collect());
                assert!(patch.files[0].image.is_some());
                patch.files.push(FilePatch {
                    label: patch.files[0].label.clone(),
                    ..Default::default()
                });
                patch.bind_images(descriptors().into_iter().take(1).collect());
                assert!(patch.files.iter().all(|file| file.image.is_none()));
            },
        );
    }
    #[test]
    fn git_space_delimiter_does_not_hide_unicode_or_literal_tab_names() {
        let patch = Patch::parse(b"diff --git a/old b/new\n--- /dev/null\n+++ \"b/\\346\\226\\260 \\346\\226\\207\\t.txt\"\t\n@@ -0,0 +1 @@\n+content\n");
        assert_eq!(patch.files[0].label, "新 文\\t.txt");
        assert_eq!(patch.files[0].added, 1);
        let patch = Patch::parse(b"diff --git a/a b/a\n--- /dev/null\n+++ b/plain space.txt\t\n@@ -0,0 +1 @@\n+content\n");
        assert_eq!(patch.files[0].label, "plain space.txt");
    }
    #[test]
    fn counts_content_not_headers_and_keeps_binary_and_quoted_paths() {
        let patch = Patch::parse(b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -4,2 +4,2 @@\n keep\n-old\n+new\ndiff --git \"a/odd\\tfile\" \"b/odd\\tfile\"\nBinary files differ\n");
        assert_eq!(patch.files.len(), 2);
        assert_eq!((patch.files[0].added, patch.files[0].removed), (1, 1));
        assert_eq!(patch.files[0].lines[5].old, Some(5));
        assert_eq!(patch.files[0].lines[6].new, Some(5));
        assert!(patch.files[1].label.contains("\\t"));
        assert!(
            patch.files[1]
                .lines
                .iter()
                .any(|l| l.text.contains("Binary"))
        );
    }
    #[test]
    fn overview_tracks_unified_and_split_rows_without_changing_patch_identity() {
        let patch = Patch::parse("diff --git a/資料.rs b/資料.rs\n--- a/資料.rs\n+++ b/資料.rs\n@@ -1,4 +1,3 @@\n context\n-old 🚀\n-old two\n+new 文\n end".as_bytes());
        let f = &patch.files[0];
        assert_eq!(f.visible.len(), 6);
        assert_eq!(f.split.len(), 5);
        assert_eq!(f.overview[0].rows, 6);
        assert_eq!(f.overview[1].rows, 5);
        assert_eq!(f.overview[0].marked_rows, 3);
        assert_eq!(f.overview[1].marked_rows, 2);
        assert_eq!(f.lines[f.visible[2]].raw_index, 5);
        assert_eq!(f.lines[f.split[2].1.unwrap()].text, "+new 文");
    }
    #[test]
    fn hostile_long_lines_are_bounded_and_explicitly_flagged() {
        let patch = Patch::parse(&vec![b'a'; 20_000]);
        assert!(patch.limited);
        assert_eq!(patch.files[0].lines[0].text.len(), 8192);
    }

    #[test]
    fn unicode_lines_remain_valid_at_preview_limits() {
        let line = format!("{}文🚀", "x".repeat(8191));
        let patch = Patch::parse(line.as_bytes());
        assert!(patch.limited);
        assert_eq!(patch.files[0].lines[0].text, "x".repeat(8191));
        let preview = Patch::preview(
            &rocket_git::RepoPath::new("文.txt".as_bytes().to_vec()).unwrap(),
            rocket_git::FilePreview {
                bytes: line.into_bytes(),
                truncated: false,
                binary: false,
            },
        );
        assert!(preview.limited);
        assert_eq!(
            preview.files[0].lines[0].text,
            format!("+{}", "x".repeat(8191))
        );
    }

    #[test]
    fn git_quoted_unicode_paths_are_readable_labels() {
        let patch = Patch::parse(b"diff --git \"a/\\346\\226\\207.txt\" \"b/\\346\\226\\207.txt\"\n--- \"a/\\346\\226\\207.txt\"\n+++ \"b/\\346\\226\\207.txt\"\n@@ -1 +1 @@\n-old\n+new\n");
        assert_eq!(patch.files[0].label, "文.txt");
    }

    #[test]
    fn unicode_labels_cover_binary_rename_deletion_and_spaces() {
        for (patch, expected) in [
            (
                "diff --git \"a/\\346\\226\\207.bin\" \"b/\\346\\226\\207.bin\"\nBinary files differ\n",
                "文.bin",
            ),
            (
                "diff --git a/old.txt \"b/\\346\\226\\207.txt\"\nsimilarity index 100%\n",
                "文.txt",
            ),
            (
                "diff --git a/old b/name.txt b/new b/name.txt\n--- a/old b/name.txt\n+++ b/new b/name.txt\n",
                "new b/name.txt",
            ),
            (
                "diff --git a/文 space.txt b/文 space.txt\n--- a/文 space.txt\n+++ /dev/null\n",
                "文 space.txt",
            ),
            (
                "diff --git a/a/文.bin b/a/文.bin\nBinary files differ\n",
                "a/文.bin",
            ),
        ] {
            assert_eq!(Patch::parse(patch.as_bytes()).files[0].label, expected);
        }
    }
}
