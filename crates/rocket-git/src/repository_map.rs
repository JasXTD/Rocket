//! Bounded, read-only maps of immutable Git objects. No worktree file reads,
//! package execution, external diff/filter programs, or lazy network fetching.
use crate::{parse, repository::commit_revision_args, runner::args, *};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod cargo;
mod mermaid;
pub use mermaid::map_mermaid;

pub const MAP_COMMITS: usize = 500;
pub const MAP_HISTORY_MAX: usize = 2000;
pub const MAP_NODES: usize = 4096;
// Bound history snapshots before their links can be cloned or laid out by the UI.
// Mermaid export uses the same connection budget.
const MAP_LINKS: usize = 12_000;
const MAP_PATHS: usize = 100_000;
const EVIDENCE: usize = 24;
const LABEL_BYTES: usize = 4096;

#[derive(Default)]
struct AreaChanges {
    commits: usize,
    paths: BTreeSet<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapKind {
    #[default]
    Code,
    History,
    People,
}
impl MapKind {
    pub fn operation(self) -> &'static str {
        match self {
            Self::Code => "map-code",
            Self::History => "map-history",
            Self::People => "map-people",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapOptions {
    pub kind: MapKind,
    pub revision: String,
    pub all_branches: bool,
    pub since: Option<i64>,
    /// Exclude this commit and its ancestors (base..revision).
    pub base: String,
    /// Literal path filter, applied by Git before the commit limit.
    pub path: Option<RepoPath>,
    pub page_size: usize,
}
impl Default for MapOptions {
    fn default() -> Self {
        Self {
            kind: MapKind::Code,
            revision: "HEAD".into(),
            all_branches: false,
            since: None,
            base: String::new(),
            path: None,
            page_size: MAP_COMMITS,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapNodeKind {
    Directory,
    File,
    Crate,
    Commit,
    Person,
    Area,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapLinkKind {
    Contains,
    DependsOn,
    Parent,
    Changes,
    SharedFiles,
}
#[derive(Clone, Debug)]
pub struct MapNode {
    /// Stable key within the snapshot. Labels and paths retain original Git bytes.
    pub key: Vec<u8>,
    pub label: Vec<u8>,
    pub detail: Vec<u8>,
    pub kind: MapNodeKind,
    pub path: Option<Vec<u8>>,
    pub commit: Option<ObjectId>,
    pub count: usize,
    pub files: usize,
    pub evidence: Vec<Vec<u8>>,
    pub refs: Vec<Vec<u8>>,
    /// Committer time of a commit node, in seconds since 1970; zero for other kinds.
    pub time: i64,
}
#[derive(Clone, Debug)]
pub struct MapLink {
    pub from: usize,
    pub to: usize,
    pub kind: MapLinkKind,
    pub weight: usize,
    pub evidence: Vec<Vec<u8>>,
}
#[derive(Clone, Debug, Default)]
pub struct RepositoryMap {
    pub kind: MapKind,
    pub tip: Option<ObjectId>,
    pub tips: Vec<ObjectId>,
    pub excluded_base: Option<ObjectId>,
    pub nodes: Vec<MapNode>,
    pub links: Vec<MapLink>,
    pub scanned: usize,
    pub limited: bool,
    pub notes: Vec<String>,
    pub scope: MapOptions,
    pub history_limit: usize,
    pub has_more: bool,
    pub boundary_parents: usize,
    pub continuation: Option<MapContinuation>,
}
/// Opaque continuation bound to this repository and immutable Git object IDs.
#[derive(Clone, Debug)]
pub struct MapContinuation {
    repository: std::path::PathBuf,
    tips: Vec<ObjectId>,
    tip: Option<ObjectId>,
    base: Option<ObjectId>,
    branches: Vec<Branch>,
    /// Tag names and the commits they name, for history decorations.
    tags: Vec<(Vec<u8>, ObjectId)>,
    options: MapOptions,
    limit: usize,
}
fn node(kind: MapNodeKind, key: Vec<u8>, label: Vec<u8>, detail: Vec<u8>) -> MapNode {
    MapNode {
        key,
        label,
        detail,
        kind,
        path: None,
        commit: None,
        count: 0,
        files: 0,
        evidence: Vec::new(),
        refs: Vec::new(),
        time: 0,
    }
}
fn link(from: usize, to: usize, kind: MapLinkKind) -> MapLink {
    MapLink {
        from,
        to,
        kind,
        weight: 1,
        evidence: Vec::new(),
    }
}
fn malformed() -> Error {
    Error::parse("repository-map", "malformed object or change record")
}

impl Repository {
    pub fn repository_map(
        &self,
        options: &MapOptions,
        cancel: &CancellationToken,
    ) -> Result<RepositoryMap> {
        cancel.check()?;
        if !(1..=MAP_COMMITS).contains(&options.page_size) {
            return Err(Error::InvalidInput(
                "Map page size must be between 1 and 500.".into(),
            ));
        }
        let op = options.kind.operation();
        let head = self.head(cancel)?;
        let branches = if options.kind != MapKind::Code {
            self.branches(cancel)?
        } else {
            Vec::new()
        };
        // Tags only decorate history; a failed tag read leaves them out.
        let tags = if options.kind == MapKind::History {
            match self.tags(2000, cancel) {
                Ok(page) => page
                    .entries
                    .into_iter()
                    .zip(page.summaries)
                    .filter_map(|(entry, summary)| Some((entry.name, summary.target?)))
                    .collect(),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let tip = if options.revision == "HEAD" && matches!(head, Head::Unborn { .. }) {
            None
        } else {
            let bytes = self.runner.step(op, "resolve-snapshot", || {
                self.map_read(
                    &commit_revision_args(&options.revision)?,
                    Vec::new(),
                    cancel,
                )
            })?;
            Some(parse::oid(parse::line(&bytes), op)?)
        };
        let mut tips = if options.all_branches && options.kind != MapKind::Code {
            branches
                .iter()
                .map(|b| b.target.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        tips.extend(tip.iter().cloned());
        tips.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        tips.dedup();
        let base = if options.kind != MapKind::Code && !options.base.is_empty() {
            let bytes = self.map_read(&commit_revision_args(&options.base)?, Vec::new(), cancel)?;
            Some(parse::oid(parse::line(&bytes), op)?)
        } else {
            None
        };
        let cursor = MapContinuation {
            repository: self.info().git_dir.clone(),
            tips,
            tip,
            base,
            branches,
            tags,
            options: options.clone(),
            limit: options.page_size,
        };
        self.map_snapshot(cursor, cancel)
    }

    pub fn repository_map_more(
        &self,
        previous: &RepositoryMap,
        cancel: &CancellationToken,
    ) -> Result<RepositoryMap> {
        cancel.check()?;
        let mut cursor = previous.continuation.clone().ok_or_else(|| {
            Error::InvalidInput("No more history is available for this snapshot.".into())
        })?;
        if cursor.repository != self.info().git_dir {
            return Err(Error::InvalidInput(
                "Map continuation belongs to another repository.".into(),
            ));
        }
        cursor.limit = (cursor.limit + cursor.options.page_size).min(MAP_HISTORY_MAX);
        self.map_snapshot(cursor, cancel)
    }

    fn map_snapshot(
        &self,
        cursor: MapContinuation,
        cancel: &CancellationToken,
    ) -> Result<RepositoryMap> {
        let options = &cursor.options;
        let op = options.kind.operation();
        let mut map = RepositoryMap {
            kind: options.kind,
            tip: cursor.tip.clone(),
            tips: cursor.tips.clone(),
            excluded_base: cursor.base.clone(),
            scope: options.clone(),
            history_limit: cursor.limit,
            ..Default::default()
        };
        if map.tips.is_empty() {
            return Ok(map);
        }
        if map.tips.len() > 4096 {
            return Err(Error::Unsupported(
                "Map has over 4,096 branch tips. Choose one revision.".into(),
            ));
        }
        if options.kind == MapKind::Code {
            self.map_code(&mut map, cancel)?;
        } else {
            let commits = self.runner.step(op, "read-commits", || {
                self.map_commits(&mut map, &cursor, cancel)
            })?;
            let paths = self.runner.step(op, "read-changed-paths", || {
                self.map_changes(&commits, cancel)
            })?;
            self.runner.step(op, "build-connections", || {
                if options.kind == MapKind::History {
                    let aliases = if let Some(tip) = &map.tip {
                        self.map_aliases(tip, cancel)?
                    } else {
                        Vec::new()
                    };
                    map_history(
                        &mut map,
                        &commits,
                        &paths,
                        &cursor.branches,
                        &cursor.tags,
                        &aliases,
                        cancel,
                    )
                } else {
                    self.map_people(&mut map, &commits, &paths, options.since, cancel)
                }
            })?;
        }
        if map.has_more && cursor.limit < MAP_HISTORY_MAX {
            map.continuation = Some(cursor);
        } else if map.has_more {
            map.notes.push("The 2,000-commit map budget is reached. Narrow the branch, base or literal path scope.".into());
        }
        // Subjects and author names are arbitrary Git bytes. Bound presentation
        // text on the worker before it reaches native text layout.
        let mut shortened = false;
        for n in &mut map.nodes {
            cancel.check()?;
            shortened |= shorten_label(&mut n.label);
            shortened |= shorten_label(&mut n.detail);
        }
        if shortened {
            map.notes.push("Labels longer than 4 KiB were shortened. Object IDs and repository paths are unchanged.".into());
        }
        cancel.check()?;
        Ok(map)
    }
    fn map_read(
        &self,
        arguments: &[std::ffi::OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut plain = args(&["--no-replace-objects"]);
        plain.extend_from_slice(arguments);
        self.read_diff_input("repository-map", &plain, input, cancel)
    }
    fn map_commits(
        &self,
        map: &mut RepositoryMap,
        cursor: &MapContinuation,
        cancel: &CancellationToken,
    ) -> Result<Vec<Commit>> {
        let mut input: Vec<u8> = map
            .tips
            .iter()
            .flat_map(|id| id.as_str().bytes().chain(Some(b'\n')))
            .collect();
        if let Some(base) = &cursor.base {
            input.extend_from_slice(format!("^{base}\n").as_bytes());
        }
        // Runner supplies --literal-pathspecs to every Git command already.
        let mut arguments = args(&[
            "rev-list",
            "--topo-order",
            "--full-history",
            &format!("--max-count={}", cursor.limit + 1),
            "--stdin",
            "--",
        ]);
        if let Some(path) = &cursor.options.path {
            arguments.push(path.argument()?);
        }
        let output = self.map_read(&arguments, input, cancel)?;
        let mut ids = Vec::new();
        for raw in output.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            cancel.check()?;
            ids.push(parse::oid(raw, "repository-map")?);
        }
        map.has_more = ids.len() > cursor.limit;
        map.limited = map.has_more;
        ids.truncate(cursor.limit);
        map.scanned = ids.len();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let input = ids
            .iter()
            .flat_map(|id| id.as_str().bytes().chain(Some(b'\n')))
            .collect();
        let bytes = self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)?;
        cancel.check()?;
        parse::commits(&bytes, &ids)
    }
    fn map_changes(
        &self,
        commits: &[Commit],
        cancel: &CancellationToken,
    ) -> Result<Vec<Vec<Vec<u8>>>> {
        if commits.is_empty() {
            return Ok(Vec::new());
        }
        let input = commits
            .iter()
            .flat_map(|c| c.id.as_str().bytes().chain(Some(b'\n')))
            .collect();
        let bytes = self.map_read(
            &args(&[
                "diff-tree",
                "--stdin",
                "-r",
                "-z",
                "--raw",
                "--no-abbrev",
                "--no-renames",
                "--root",
                "--always",
                "--no-ext-diff",
                "--no-textconv",
            ]),
            input,
            cancel,
        )?;
        parse_changes(&bytes, commits, cancel)
    }
    fn map_code(&self, map: &mut RepositoryMap, cancel: &CancellationToken) -> Result<()> {
        let tip = map.tip.as_ref().ok_or_else(malformed)?.clone();
        self.prepare_tree(tip.as_str(), cancel)?;
        let output = self.runner.step("map-code", "list-tree", || {
            self.map_read(
                &args(&["ls-tree", "-r", "-l", "-z", "--full-tree", tip.as_str()]),
                Vec::new(),
                cancel,
            )
        })?;
        let mut index: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let mut manifests = Vec::new();
        let mut budget = 0usize;
        for raw in output.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            cancel.check()?;
            map.scanned += 1;
            let (header, path) = raw.split_once_byte(b'\t').ok_or_else(malformed)?;
            let fields = header
                .split(|b| b.is_ascii_whitespace())
                .filter(|f| !f.is_empty())
                .collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(malformed());
            }
            if map.nodes.len() >= MAP_NODES {
                map.limited = true;
                continue;
            }
            let parts = path.split(|b| *b == b'/').collect::<Vec<_>>();
            if parts.len() > 64 {
                map.limited = true;
                continue;
            }
            let mut prefix = Vec::new();
            let mut parent = None;
            for (i, part) in parts.iter().enumerate() {
                if !prefix.is_empty() {
                    prefix.push(b'/');
                }
                prefix.extend_from_slice(part);
                let folder = i + 1 < parts.len();
                let at = if let Some(at) = index.get(&prefix) {
                    *at
                } else {
                    if map.nodes.len() >= MAP_NODES {
                        map.limited = true;
                        break;
                    }
                    let mut n = node(
                        if folder {
                            MapNodeKind::Directory
                        } else {
                            MapNodeKind::File
                        },
                        prefix.clone(),
                        part.to_vec(),
                        if folder {
                            b"Committed folder".to_vec()
                        } else if fields[0] == b"120000" {
                            b"Symbolic link (not followed)".to_vec()
                        } else if fields[1] == b"commit" {
                            b"Submodule reference (not followed)".to_vec()
                        } else {
                            b"Committed file".to_vec()
                        },
                    );
                    n.path = Some(prefix.clone());
                    let at = map.nodes.len();
                    map.nodes.push(n);
                    index.insert(prefix.clone(), at);
                    if let Some(parent) = parent {
                        map.links.push(link(parent, at, MapLinkKind::Contains));
                    }
                    at
                };
                map.nodes[at].files += 1;
                parent = Some(at);
            }
            if path.rsplit(|b| *b == b'/').next() == Some(b"Cargo.toml")
                && fields[0] != b"120000"
                && fields[1] == b"blob"
            {
                let size = std::str::from_utf8(fields[3])
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .ok_or_else(malformed)?;
                if size > 128 * 1024 || manifests.len() >= 64 || budget + size > 4 * 1024 * 1024 {
                    map.limited = true;
                    continue;
                }
                budget += size;
                manifests.push((path.to_vec(), parse::oid(fields[2], "map-code")?, size));
            }
        }
        if !manifests.is_empty() {
            self.runner.step("map-code", "read-manifests", || {
                let input = manifests
                    .iter()
                    .flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n')))
                    .collect();
                let output =
                    self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)?;
                let blobs = parse_blobs(&output, &manifests, cancel)?;
                cargo::connect(map, &index, &manifests, &blobs, cancel)
            })?;
        }
        map.notes.push("Committed snapshot only. Folder/file containment and local Cargo path declarations are separate connections. Other language imports, generated code and external packages are not analyzed.".into());
        Ok(())
    }
    fn map_people(
        &self,
        map: &mut RepositoryMap,
        commits: &[Commit],
        paths: &[Vec<Vec<u8>>],
        since: Option<i64>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let aliases = if let Some(tip) = &map.tip {
            self.map_aliases(tip, cancel)?
        } else {
            Vec::new()
        };
        let split = crowded_areas(commits, paths, since);
        let mut people: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let mut areas: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let mut edges: BTreeMap<(usize, usize), AreaChanges> = BTreeMap::new();
        let mut person_files: BTreeMap<usize, BTreeSet<Vec<u8>>> = BTreeMap::new();
        for (commit, changed) in commits.iter().zip(paths) {
            cancel.check()?;
            if since.is_some_and(|s| commit.author.timestamp < s) {
                continue;
            }
            let (name, email) = canonical(&commit.author.name, &commit.author.email, &aliases);
            let key = if email.is_empty() {
                name.clone()
            } else {
                email.to_ascii_lowercase()
            };
            let person = if let Some(i) = people.get(&key) {
                *i
            } else {
                if map.nodes.len() >= MAP_NODES {
                    map.limited = true;
                    continue;
                }
                let i = map.nodes.len();
                map.nodes.push(node(
                    MapNodeKind::Person,
                    key.clone(),
                    name,
                    "Recorded author · pinned .mailmap".as_bytes().to_vec(),
                ));
                people.insert(key, i);
                i
            };
            map.nodes[person].count += 1;
            if map.nodes[person].commit.is_none() {
                map.nodes[person].commit = Some(commit.id.clone());
            }
            let files = person_files.entry(person).or_default();
            let mut seen = BTreeSet::new();
            for path in changed {
                cancel.check()?;
                files.insert(path.clone());
                let area = area_of(path, &split);
                let target = if let Some(i) = areas.get(&area) {
                    *i
                } else {
                    if map.nodes.len() >= MAP_NODES {
                        map.limited = true;
                        continue;
                    }
                    let i = map.nodes.len();
                    let mut n = node(
                        MapNodeKind::Area,
                        area.clone(),
                        area.clone(),
                        b"Changed paths in this folder".to_vec(),
                    );
                    n.path = Some(area.clone());
                    map.nodes.push(n);
                    areas.insert(area, i);
                    i
                };
                let entry = edges.entry((person, target)).or_default();
                entry.paths.insert(path.clone());
                if seen.insert(target) {
                    entry.commits += 1;
                }
            }
        }
        for (i, files) in &person_files {
            map.nodes[*i].files = files.len();
            map.nodes[*i].evidence = files.iter().cloned().collect();
        }
        for ((from, to), changes) in &edges {
            let mut e = link(*from, *to, MapLinkKind::Changes);
            e.weight = changes.commits;
            e.evidence = changes.paths.iter().take(EVIDENCE).cloned().collect();
            map.links.push(e);
            map.nodes[*to].count += changes.commits;
        }
        for target in areas.values() {
            let files = edges
                .iter()
                .filter(|((_, to), _)| to == target)
                .flat_map(|(_, changes)| changes.paths.iter())
                .collect::<BTreeSet<_>>();
            map.nodes[*target].files = files.len();
            map.nodes[*target].evidence = files.into_iter().cloned().collect();
        }
        // Bounded pair comparisons; counts describe overlap, never collaboration.
        let ids = person_files.keys().copied().take(128).collect::<Vec<_>>();
        if person_files.len() > 128 {
            map.limited = true;
            map.notes
                .push("Shared-file connections are limited to the first 128 authors.".into());
        }
        for (at, from) in ids.iter().enumerate() {
            for to in &ids[at + 1..] {
                cancel.check()?;
                let shared = person_files[from]
                    .intersection(&person_files[to])
                    .collect::<Vec<_>>();
                if !shared.is_empty() {
                    let mut e = link(*from, *to, MapLinkKind::SharedFiles);
                    e.weight = shared.len();
                    e.evidence = shared.into_iter().take(EVIDENCE).cloned().collect();
                    map.links.push(e);
                }
            }
        }
        map.notes.push("Counts use author time within the loaded commit scope and the selected commit's .mailmap. Load more can add older activity. Renames count as old/new paths; merge-only resolutions are not attributed. Shared files indicate overlapping changes, not direct collaboration or productivity.".into());
        Ok(())
    }
    fn map_aliases(&self, tip: &ObjectId, cancel: &CancellationToken) -> Result<Vec<Alias>> {
        if self.is_preview() {
            // ls-tree -l needs the blob for its size: fetch .mailmap alone first.
            let entry = self.map_read(&args(&["ls-tree", "-z", tip.as_str(), "--", ".mailmap"]), Vec::new(), cancel)?;
            if let Some(id) = entry.split(|b| *b == b' ' || *b == b'\t').nth(2) {
                self.prepare_objects(&[parse::oid(id, "map-people")?], cancel)?;
            }
        }
        let out = self.map_read(
            &args(&["ls-tree", "-l", "-z", tip.as_str(), "--", ".mailmap"]),
            Vec::new(),
            cancel,
        )?;
        if out.is_empty() {
            return Ok(Vec::new());
        }
        let (header, _) = out.split_once_byte(b'\t').ok_or_else(malformed)?;
        let fields = header
            .split(|b| b.is_ascii_whitespace())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        if fields.len() != 4 || fields[1] != b"blob" || fields[0] == b"120000" {
            return Ok(Vec::new());
        }
        let size = std::str::from_utf8(fields[3])
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or_else(malformed)?;
        if size > 128 * 1024 {
            return Err(Error::Unsupported(
                "The committed .mailmap exceeds the Map's 128 KiB limit.".into(),
            ));
        }
        let id = parse::oid(fields[2], "map-people")?;
        let bytes = self.map_read(
            &args(&["cat-file", "blob", id.as_str()]),
            Vec::new(),
            cancel,
        )?;
        Ok(parse_aliases(&bytes))
    }
}

/// Top-level folders holding at least half the changed paths, with two or more
/// subfolders: People counts their subfolders separately, so one folder such as
/// crates/ doesn't take every commit.
fn crowded_areas(commits: &[Commit], paths: &[Vec<Vec<u8>>], since: Option<i64>) -> BTreeSet<Vec<u8>> {
    let mut total = 0usize;
    let mut counts: BTreeMap<&[u8], (usize, BTreeSet<&[u8]>)> = BTreeMap::new();
    for (commit, changed) in commits.iter().zip(paths) {
        if since.is_some_and(|s| commit.author.timestamp < s) {
            continue;
        }
        for path in changed {
            total += 1;
            let mut parts = path.split(|b| *b == b'/');
            let (Some(top), Some(second)) = (parts.next(), parts.next()) else {
                continue;
            };
            let entry = counts.entry(top).or_default();
            entry.0 += 1;
            if parts.next().is_some() {
                entry.1.insert(second);
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, (count, subfolders))| *count * 2 >= total && subfolders.len() >= 2)
        .map(|(top, _)| top.to_vec())
        .collect()
}
/// The People area of a changed path: its top-level folder, a subfolder of a crowded
/// one, or "/" for files at the top.
fn area_of(path: &[u8], split: &BTreeSet<Vec<u8>>) -> Vec<u8> {
    let mut parts = path.split(|b| *b == b'/');
    let top = parts.next().unwrap_or(path);
    if !path.contains(&b'/') {
        return b"/".to_vec();
    }
    if split.contains(top) {
        let rest: Vec<&[u8]> = parts.collect();
        if rest.len() >= 2 {
            return [top, b"/", rest[0]].concat();
        }
    }
    top.to_vec()
}

fn shorten_label(bytes: &mut Vec<u8>) -> bool {
    if bytes.len() <= LABEL_BYTES {
        return false;
    }
    let mut end = LABEL_BYTES - 3; // Include the marker within the display cap.
    while end > 0 && bytes[end] & 0xc0 == 0x80 {
        end -= 1;
    }
    bytes.truncate(end);
    bytes.extend_from_slice(b"...");
    true
}

trait ByteSplit {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])>;
}
impl ByteSplit for [u8] {
    fn split_once_byte(&self, b: u8) -> Option<(&[u8], &[u8])> {
        let i = self.iter().position(|c| *c == b)?;
        Some((&self[..i], &self[i + 1..]))
    }
}

fn parse_changes(
    bytes: &[u8],
    commits: &[Commit],
    cancel: &CancellationToken,
) -> Result<Vec<Vec<Vec<u8>>>> {
    let lookup = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str().as_bytes(), i))
        .collect::<HashMap<_, _>>();
    let mut result = vec![Vec::new(); commits.len()];
    let mut current: Option<usize> = None;
    let mut fields = bytes.split(|b| *b == 0);
    let mut count = 0;
    while let Some(raw) = fields.next() {
        cancel.check()?;
        let raw = raw.trim_ascii();
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with(b":") {
            let i = current.ok_or_else(malformed)?;
            let path = fields.next().ok_or_else(malformed)?;
            if path.is_empty() {
                return Err(malformed());
            }
            count += 1;
            if count > MAP_PATHS {
                return Err(Error::Unsupported(
                    "Map exceeds 100,000 changed paths. Choose a smaller history scope.".into(),
                ));
            }
            result[i].push(path.to_vec());
        } else {
            current = Some(*lookup.get(raw).ok_or_else(malformed)?);
        }
    }
    for paths in &mut result {
        paths.sort();
        paths.dedup();
    }
    Ok(result)
}
fn map_history(
    map: &mut RepositoryMap,
    commits: &[Commit],
    paths: &[Vec<Vec<u8>>],
    branches: &[Branch],
    tags: &[(Vec<u8>, ObjectId)],
    aliases: &[Alias],
    cancel: &CancellationToken,
) -> Result<()> {
    let lookup = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (&c.id, i))
        .collect::<HashMap<_, _>>();
    for (c, paths) in commits.iter().zip(paths) {
        cancel.check()?;
        let mut n = node(
            MapNodeKind::Commit,
            c.id.as_str().as_bytes().to_vec(),
            c.message
                .split(|b| *b == b'\n')
                .next()
                .unwrap_or_default()
                .to_vec(),
            canonical(&c.author.name, &c.author.email, aliases).0,
        );
        n.commit = Some(c.id.clone());
        n.count = c.parents.len();
        n.files = paths.len();
        n.evidence = paths.clone();
        n.refs = branches
            .iter()
            .filter(|b| b.target == c.id)
            .map(|b| b.reference.clone())
            .collect();
        n.refs.extend(
            tags.iter()
                .filter(|(_, target)| *target == c.id)
                .map(|(name, _)| [&b"refs/tags/"[..], name].concat()),
        );
        n.time = c.committer.timestamp;
        map.nodes.push(n);
    }
    for (i, c) in commits.iter().enumerate() {
        cancel.check()?;
        for p in &c.parents {
            if let Some(from) = lookup.get(p) {
                if map.links.len() >= MAP_LINKS {
                    return Err(Error::Unsupported(
                        "History map exceeds 12,000 parent links. Narrow the branch, base or literal path scope.".into(),
                    ));
                }
                map.links.push(link(*from, i, MapLinkKind::Parent));
            } else {
                map.limited = true;
                map.boundary_parents += 1;
            }
        }
    }
    map.notes.push("Arrows follow original parent to child. Parents outside the loaded or filtered scope are counted at the boundary, never rewritten into false connections. Load more uses the same pinned tips and base; Refresh reads new tips. Authors use the pinned .mailmap.".into());
    Ok(())
}
fn parse_blobs<'a>(
    bytes: &'a [u8],
    expected: &[(Vec<u8>, ObjectId, usize)],
    cancel: &CancellationToken,
) -> Result<Vec<&'a [u8]>> {
    let mut rest = bytes;
    let mut result = Vec::new();
    for (_, id, size) in expected {
        cancel.check()?;
        let (header, body) = rest.split_once_byte(b'\n').ok_or_else(malformed)?;
        if header != format!("{} blob {}", id.as_str(), size).as_bytes()
            || body.get(*size) != Some(&b'\n')
        {
            return Err(malformed());
        }
        result.push(&body[..*size]);
        rest = &body[size + 1..];
    }
    if !rest.is_empty() {
        return Err(malformed());
    }
    Ok(result)
}
struct Alias {
    name: Vec<u8>,
    email: Vec<u8>,
    old_name: Vec<u8>,
    old_email: Vec<u8>,
}
fn parse_aliases(bytes: &[u8]) -> Vec<Alias> {
    let mut result = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        let line = line.trim_ascii();
        if line.starts_with(b"#") {
            continue;
        }
        let Some((name, rest)) = line.split_once_byte(b'<') else {
            continue;
        };
        let Some((email, rest)) = rest.split_once_byte(b'>') else {
            continue;
        };
        let (old_name, old_email) = if let Some((old, rest)) = rest.split_once_byte(b'<') {
            let Some((email, _)) = rest.split_once_byte(b'>') else {
                continue;
            };
            (old.trim_ascii().to_vec(), email.to_vec())
        } else {
            (Vec::new(), email.to_vec())
        };
        result.push(Alias {
            name: name.trim_ascii().to_vec(),
            email: email.to_vec(),
            old_name,
            old_email,
        });
    }
    result
}
fn canonical(name: &[u8], email: &[u8], aliases: &[Alias]) -> (Vec<u8>, Vec<u8>) {
    let matched = aliases
        .iter()
        .rev()
        .find(|a| {
            a.old_email.eq_ignore_ascii_case(email)
                && !a.old_name.is_empty()
                && a.old_name.eq_ignore_ascii_case(name)
        })
        .or_else(|| {
            aliases
                .iter()
                .rev()
                .find(|a| a.old_email.eq_ignore_ascii_case(email) && a.old_name.is_empty())
        });
    matched.map_or_else(
        || (name.to_vec(), email.to_vec()),
        |a| {
            (
                if a.name.is_empty() {
                    name.to_vec()
                } else {
                    a.name.clone()
                },
                a.email.clone(),
            )
        },
    )
}

#[cfg(test)]
mod text_tests {
    use super::*;
    #[test]
    fn overlong_unicode_labels_are_bounded_at_a_character_boundary() {
        let mut text = "界🚀".repeat(2000).into_bytes();
        assert!(shorten_label(&mut text));
        assert!(text.len() <= LABEL_BYTES);
        assert!(std::str::from_utf8(&text).unwrap().ends_with("..."));
        let mut normal = "結果 🚀 é".as_bytes().to_vec();
        assert!(!shorten_label(&mut normal));
        assert_eq!(normal, "結果 🚀 é".as_bytes());
    }
}
