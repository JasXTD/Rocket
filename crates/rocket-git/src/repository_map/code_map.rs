//! The code map (Map → Code → Folders, canvas section 47): which files use which across
//! the whole snapshot, from the readers Modules has, and what Folders can colour by:
//! the kind of each file, who changed it and when, CODEOWNERS, and what a revision
//! changes against a base. Reads committed objects only; nothing is compiled or run.
use super::owners::{CODEOWNERS_BYTES, CODEOWNERS_FILES, CodeOwners, Ownership, MatchWork};
use super::*;
use std::{collections::HashSet, sync::Arc};

/// Code files one read takes; past this, the packages left are read when their folder
/// is opened.
pub const CODE_MAP_FILES: usize = 10_000;
/// Charge source planning even when a package has no readable files.
pub const CODE_MAP_PACKAGES: usize = 512;
const CODE_MAP_BYTES: usize = 64 * 1024 * 1024;
/// As Modules: a larger file is not read.
const FILE_BYTES: usize = 1024 * 1024;
/// Files go to Git in parts of this size, well inside one process's output limit.
const BATCH_BYTES: usize = 16 * 1024 * 1024;
/// Manifests: each file, and all of them together.
const MANIFEST_FILE: usize = 128 * 1024;
const MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const LINKS: usize = 200_000;
/// Names kept on a link, for Details and hover.
const LINK_NAMES: usize = 8;
const EXTRA_ROUNDS: usize = 8;
/// Commits the history read takes, newest first.
pub const CODE_HISTORY_COMMITS: usize = 10_000;
const HISTORY_FALLBACK: usize = 2_000;
const HISTORY_BYTES: usize = 64 * 1024 * 1024;

/// What a committed file is, for the code map's Kind colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Code,
    Tests,
    Docs,
    /// Configuration, manifests, lock files and data.
    Config,
    /// Pictures, fonts, binaries and anything unknown.
    Other,
}
impl FileKind {
    pub const ALL: [FileKind; 5] = [Self::Code, Self::Tests, Self::Docs, Self::Config, Self::Other];
}

const MEDIA: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "tif", "tiff", "svg", "psd", "ai", "mp4", "mov", "webm", "avi", "mp3",
    "wav", "ogg", "flac", "ttf", "ttc", "otf", "woff", "woff2", "eot", "exe", "dll", "so", "dylib", "a", "lib", "o", "obj", "pdb", "zip",
    "gz", "tgz", "xz", "bz2", "7z", "rar", "jar", "war", "class", "pdf", "bin", "dat", "db", "sqlite", "wasm", "pyc", "node",
];
const DOCS: &[&str] = &["md", "markdown", "mdx", "rst", "adoc", "asciidoc", "txt", "org", "tex", "man"];
const CONFIG: &[&str] = &[
    "json", "jsonc", "json5", "toml", "yaml", "yml", "ini", "cfg", "conf", "xml", "plist", "lock", "properties", "env", "csv", "tsv",
    "editorconfig", "gitignore", "gitattributes", "gitmodules", "dockerignore", "npmrc", "nvmrc", "babelrc", "eslintrc", "prettierrc",
    "mailmap", "gradle", "cmake", "mk",
];
const OTHER_CODE: &[&str] = &[
    "rs", "js", "cjs", "mjs", "jsx", "ts", "cts", "mts", "tsx", "py", "pyi", "go", "java", "kt", "kts", "scala", "swift", "m", "mm",
    "cs", "fs", "vb", "rb", "php", "pl", "pm", "lua", "r", "dart", "ex", "exs",
    "erl", "hrl", "hs", "clj", "cljs", "elm", "ml", "mli", "zig", "nim", "v", "sv", "vhd", "sh", "bash", "zsh", "fish", "ps1", "psm1",
    "psd1", "bat", "cmd", "html", "htm", "css", "scss", "sass", "less", "vue", "svelte", "astro", "sql", "graphql", "gql", "proto",
    "groovy", "jl", "f", "f90", "asm", "s",
];
const DOC_NAMES: &[&str] = &["license", "licence", "copying", "notice", "authors", "contributors", "changelog", "changes", "readme"];
const CONFIG_NAMES: &[&str] = &["dockerfile", "makefile", "cmakelists.txt", "procfile", "gemfile", "rakefile", "justfile", "brewfile", "vagrantfile", "jenkinsfile"];
const TEST_FOLDERS: &[&str] = &["test", "tests", "__tests__", "spec", "specs", "fixtures", "fixture", "testdata", "testing"];

/// The kind of the file at `path`, from its name and folders alone.
pub fn file_kind(path: &[u8]) -> FileKind {
    let name = String::from_utf8_lossy(path.rsplit(|b| *b == b'/').next().unwrap_or_default()).to_lowercase();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        // `.gitignore` and the like: the whole name is the kind.
        Some(("", ext)) => (String::new(), ext.to_string()),
        _ => (name.clone(), String::new()),
    };
    if CONFIG_NAMES.contains(&name.as_str()) || stem.starts_with("requirements") && ext == "txt" || name.starts_with("dockerfile") {
        return FileKind::Config;
    }
    if MEDIA.contains(&ext.as_str()) {
        return FileKind::Other;
    }
    // Names like CHANGES or LICENSE-MIT are documents unless they are code: changes.rs
    // is code.
    let is_code = OTHER_CODE.contains(&ext.as_str()) || packages::c_extension(ext.as_bytes()) || packages::script_language(path).is_some();
    let doc_name = !is_code && DOC_NAMES.iter().any(|d| stem == *d || name == *d || name.starts_with(&format!("{d}-")) || name.starts_with(&format!("{d}_")));
    if DOCS.contains(&ext.as_str()) || doc_name {
        return FileKind::Docs;
    }
    if CONFIG.contains(&ext.as_str()) {
        return FileKind::Config;
    }
    if !is_code {
        return FileKind::Other;
    }
    let folders = path.split(|b| *b == b'/').collect::<Vec<_>>();
    let in_tests = folders[..folders.len().saturating_sub(1)]
        .iter()
        .any(|f| TEST_FOLDERS.contains(&String::from_utf8_lossy(f).to_lowercase().as_str()));
    let test_name = stem.starts_with("test_")
        || stem.starts_with("test-")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
        || stem.ends_with("-test")
        || stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with("_spec")
        || stem == "tests"
        || stem == "test"
        || String::from_utf8_lossy(path.rsplit(|b| *b == b'/').next().unwrap_or_default()).split('.').next().is_some_and(|s| s.ends_with("Test") || s.ends_with("Tests"));
    if in_tests || test_name { FileKind::Tests } else { FileKind::Code }
}

/// An end of a link: a file, or a folder for a package used from outside it and for a
/// Go package, which stands for all its files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeEnd {
    pub path: Vec<u8>,
    pub folder: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeLink {
    pub from: usize,
    pub to: usize,
    /// A few of the names used, for Details and hover.
    pub names: Vec<String>,
}
/// CODEOWNERS applied to every file of the snapshot.
#[derive(Clone, Debug, Default)]
pub struct OwnerRules {
    /// Where the rules were read from.
    pub file: Vec<u8>,
    pub gitlab: bool,
    pub rules: usize,
    pub skipped: usize,
    /// Owner lists, each once, most files first.
    pub sets: Vec<Vec<String>>,
    /// Each file a rule decides: the index of its owner list, or None when the deciding
    /// line names no owner. Files no rule matches are left out.
    pub files: BTreeMap<Vec<u8>, Option<u32>>,
}
/// What the code map read found.
#[derive(Clone, Debug, Default)]
pub struct CodeLinks {
    pub references: AssetReferences,
    pub tip: Option<ObjectId>,
    pub ends: Vec<CodeEnd>,
    pub links: Vec<CodeLink>,
    /// Lines of each code file read.
    pub lines: BTreeMap<Vec<u8>, usize>,
    /// Packages read.
    pub packages: Vec<PackageRef>,
    /// Packages past one read's limit; reading a folder that holds them reads them.
    pub unread: Vec<PackageRef>,
    /// Code files read.
    pub files: usize,
    pub limited: bool,
    /// Reading sources or following imports reached a limit inside a selected package.
    pub sources_limited: bool,
    /// Absent without a CODEOWNERS file, or when this read was for one folder.
    pub owners: Option<OwnerRules>,
    /// CODEOWNERS was found but is too large to read.
    pub owners_too_large: bool,
    /// Matching stopped at its global work budget; no incomplete ownership is shown.
    pub owners_limited: bool,
}
impl CodeLinks {
    /// Adds a later read of more packages (a folder read past the limit).
    pub fn merge(&mut self, other: CodeLinks) {
        let mut at: HashMap<CodeEnd, usize> = self.ends.iter().cloned().enumerate().map(|(i, e)| (e, i)).collect();
        let mut map = Vec::with_capacity(other.ends.len());
        for end in other.ends {
            let i = *at.entry(end.clone()).or_insert_with(|| {
                self.ends.push(end);
                self.ends.len() - 1
            });
            map.push(i);
        }
        let mut known: HashMap<(usize, usize), usize> = self.links.iter().enumerate().map(|(i, l)| ((l.from, l.to), i)).collect();
        for l in other.links {
            let key = (map[l.from], map[l.to]);
            match known.get(&key) {
                Some(&i) => {
                    for n in l.names {
                        if self.links[i].names.len() < LINK_NAMES && !self.links[i].names.contains(&n) {
                            self.links[i].names.push(n);
                        }
                    }
                }
                None => {
                    known.insert(key, self.links.len());
                    self.links.push(CodeLink { from: key.0, to: key.1, names: l.names });
                }
            }
        }
        self.references.merge(other.references);
        self.lines.extend(other.lines);
        let completed: HashSet<_> = other.packages.iter().collect();
        self.unread.retain(|p| !completed.contains(p));
        let mut known_packages: HashSet<_> = self.packages.iter().cloned().collect();
        self.packages.extend(other.packages.into_iter().filter(|p| known_packages.insert(p.clone())));
        self.files += other.files;
        self.sources_limited |= other.sources_limited;
        self.limited = !self.unread.is_empty() || other.limited || self.references.limited || self.sources_limited || self.owners_limited;
    }
}

#[derive(Default)]
struct Builder {
    references: AssetReferences,
    ends: Vec<CodeEnd>,
    at: HashMap<CodeEnd, usize>,
    links: BTreeMap<(usize, usize), BTreeSet<String>>,
    limited: bool,
}
impl Builder {
    fn end(&mut self, path: &[u8], folder: bool) -> usize {
        let key = CodeEnd { path: path.to_vec(), folder };
        if let Some(&i) = self.at.get(&key) {
            return i;
        }
        self.ends.push(key.clone());
        self.at.insert(key, self.ends.len() - 1);
        self.ends.len() - 1
    }
    fn link(&mut self, from: &[u8], from_folder: bool, to: &[u8], to_folder: bool, names: impl IntoIterator<Item = String>) -> bool {
        if from == to && from_folder == to_folder {
            return false;
        }
        let (a, b) = (self.end(from, from_folder), self.end(to, to_folder));
        if !self.links.contains_key(&(a, b)) && self.links.len() >= LINKS {
            self.limited = true;
            return false;
        }
        let set = self.links.entry((a, b)).or_default();
        for n in names {
            if set.len() >= LINK_NAMES {
                break;
            }
            set.insert(n);
        }
        true
    }
}

/// Files to read: path, blob and size.
type Batch = Vec<(Vec<u8>, ObjectId, usize)>;

/// Code files read so far, against one read's limits.
#[derive(Default)]
struct Budget {
    files: usize,
    bytes: usize,
    rust: rust::ReadBudget,
    limited: bool,
}
impl Budget {
    /// Reserve before queuing any source read, including #[path] follow-ups.
    fn reserve(&mut self, files: usize, bytes: usize) -> bool {
        let fits = files <= CODE_MAP_FILES.saturating_sub(self.files)
            && bytes <= CODE_MAP_BYTES.saturating_sub(self.bytes);
        if fits {
            self.files += files;
            self.bytes += bytes;
        } else {
            self.limited = true;
        }
        fits
    }
}

/// A Rust crate root of a package other than its library: `main.rs` beside a library,
/// `src/bin`, `tests`, `examples`, `benches` and `build.rs`.
fn rust_roots(folder: &[u8], files: &BTreeSet<Vec<u8>>) -> (Option<Vec<u8>>, Vec<Vec<u8>>) {
    let src = packages::join(folder, b"src");
    let lib = packages::join(&src, b"lib.rs");
    let main = packages::join(&src, b"main.rs");
    let library = if files.contains(&lib) { Some(lib.clone()) } else if files.contains(&main) { Some(main.clone()) } else { None };
    let mut others = Vec::new();
    let top = |f: &Vec<u8>, dir: &[u8]| -> bool {
        let d = packages::join(folder, dir);
        packages::parent(f) == d.as_slice() || packages::parent(packages::parent(f)) == d.as_slice() && f.ends_with(b"/main.rs")
    };
    for f in files {
        if Some(f) == library.as_ref() {
            continue;
        }
        if *f == main
            || packages::parent(f) == packages::join(&src, b"bin").as_slice()
            || packages::parent(packages::parent(f)) == packages::join(&src, b"bin").as_slice() && f.ends_with(b"/main.rs")
            || top(f, b"tests")
            || top(f, b"examples")
            || top(f, b"benches")
            || *f == packages::join(folder, b"build.rs")
        {
            others.push(f.clone());
        }
    }
    (library, others)
}

impl Repository {
    /// Which files use which across the snapshot at `tip`, read the way Modules reads one
    /// package, for every package; `only` reads just the packages in those folders (ones
    /// an earlier read left unread).
    pub fn code_links(&self, tip: &ObjectId, only: Option<&[Vec<u8>]>, cancel: &CancellationToken) -> Result<CodeLinks> {
        let op = "map-code-links";
        cancel.check()?;
        self.prepare_tree(tip.as_str(), cancel)?;
        let listing = self.runner.step(op, "list-tree", || {
            self.map_read(&args(&["ls-tree", "-r", "-l", "-z", "--full-tree", tip.as_str()]), Vec::new(), cancel)
        })?;
        let mut blobs: BTreeMap<&[u8], (ObjectId, usize)> = BTreeMap::new();
        for raw in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            cancel.check()?;
            let (header, path) = raw.split_once_byte(b'\t').ok_or_else(malformed)?;
            let fields = header.split(|b| b.is_ascii_whitespace()).filter(|f| !f.is_empty()).collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(malformed());
            }
            if fields[0] == b"120000" || fields[1] != b"blob" {
                continue;
            }
            let size = std::str::from_utf8(fields[3]).ok().and_then(|v| v.parse::<usize>().ok()).ok_or_else(malformed)?;
            blobs.insert(path, (parse::oid(fields[2], op)?, size));
        }
        let mut out = CodeLinks {
            tip: Some(tip.clone()),
            ..Default::default()
        };
        if only.is_none() {
            self.code_owners(&blobs, &mut out, cancel)?;
        }
        // Manifests name the packages: every Cargo.toml, package.json, go.mod and
        // tsconfig or jsconfig, each up to 128 KiB, MANIFEST_BYTES together.
        let paths: Vec<&[u8]> = blobs.keys().copied().collect();
        // Keep the full catalog so later reads can select a deferred folder.
        let (mut others, _) = packages::find_up_to(&paths, usize::MAX);
        let lua_manifests = packages::lua_manifests(&blobs);
        let mut manifest_limited = false;
        let mut manifests = |wanted: &mut dyn Iterator<Item = Vec<u8>>| -> Vec<(Vec<u8>, ObjectId, usize)> {
            let mut total = 0;
            let mut batch = Vec::new();
            for path in wanted {
                let Some((id, size)) = blobs.get(path.as_slice()) else { continue };
                if *size > MANIFEST_FILE {
                    continue;
                }
                if total + size > MANIFEST_BYTES {
                    manifest_limited = true;
                    break;
                }
                total += size;
                batch.push((path, id.clone(), *size));
            }
            batch
        };
        let cargo_batch = manifests(&mut paths.iter().filter(|p| packages::file_name(p) == b"Cargo.toml").map(|p| p.to_vec()));
        // Only package.json and go.mod say how a package is named from outside it.
        let small_batch = manifests(
            &mut others
                .iter()
                .filter_map(|f| f.manifest.clone())
                .filter(|m| matches!(packages::file_name(m), b"package.json" | b"go.mod"))
                .chain(others.iter().filter(|f| f.language == ModuleLanguage::JavaScript).flat_map(|f| {
                    [packages::join(&f.folder, b"tsconfig.json"), packages::join(&f.folder, b"jsconfig.json")]
                })).chain(lua_manifests.iter().cloned()),
        );
        let mut crates = BTreeMap::new();
        let mut builds = BTreeMap::new();
        for (path, text) in self.code_texts("read-manifests", &cargo_batch, cancel)? {
            if let (Some(name), _, build) = cargo::manifest_with_build(&text) {
                let folder = packages::parent(&path);
                if let Some(script) = build.and_then(|p| cargo::relative(folder, p.as_bytes()))
                    && blobs.contains_key(script.as_slice())
                    && within_package(&script, folder, &blobs)
                {
                    builds.insert(folder.to_vec(), script);
                }
                crates.insert(folder.to_vec(), name.replace('-', "_"));
            }
        }
        let small = self.code_texts("read-manifests", &small_batch, cancel)?;
        others.extend(packages::lua(&paths, &lua_manifests, &small, usize::MAX).0);
        // A crate as `use` lines name it.
        let crate_names: HashMap<String, Vec<u8>> = crates.iter().map(|(folder, name)| (name.clone(), folder.clone())).collect();
        // How other packages of this snapshot are named from outside them.
        let mut javascript_names: HashMap<String, Vec<u8>> = HashMap::new();
        let mut python_names: HashMap<String, Vec<u8>> = HashMap::new();
        let mut go_modules: Vec<(String, Vec<u8>)> = Vec::new();
        for f in &others {
            let text = f.manifest.as_ref().and_then(|m| small.get(m));
            match f.language {
                ModuleLanguage::JavaScript => {
                    if let Some(name) = text.and_then(|t| packages::json(t)).and_then(|j| j.get("name").and_then(packages::Json::str).map(str::to_string)).filter(|n| !n.is_empty()) {
                        javascript_names.insert(name, f.folder.clone());
                    }
                }
                ModuleLanguage::Python => {
                    python_names.entry(String::from_utf8_lossy(packages::file_name(&f.folder)).into_owned()).or_insert_with(|| f.folder.clone());
                }
                ModuleLanguage::Go | ModuleLanguage::Rust => {
                    if let Some(module) = text.and_then(|t| packages::go_module(t)) {
                        go_modules.push((module, f.folder.clone()));
                    }
                }
                // Java's packages and C#'s namespaces are named by their files, once read.
                ModuleLanguage::Java | ModuleLanguage::C | ModuleLanguage::CSharp | ModuleLanguage::PowerShell | ModuleLanguage::Bash | ModuleLanguage::Lua | ModuleLanguage::GitHubActions | ModuleLanguage::HtmlCss => {}
            }
        }
        // Longest module paths first, so a nested module wins over its parent.
        go_modules.sort_by_key(|m| std::cmp::Reverse(m.0.len()));
        let mut order: Vec<(Vec<u8>, Option<usize>)> = crates.keys().map(|folder| (folder.clone(), None)).collect();
        order.extend(others.iter().enumerate().map(|(i, f)| (f.folder.clone(), Some(i))));
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        // The packages this read takes, in path order, while they fit its limits.
        let mut budget = Budget::default();
        let mut planned: Vec<(PackageRef, Option<usize>, BTreeSet<Vec<u8>>)> = Vec::new();
        let mut batch: Vec<(Vec<u8>, ObjectId, usize)> = Vec::new();
        let selected = only.map(|list| list.iter().map(Vec::as_slice).collect::<HashSet<_>>());
        let mut packages_planned = 0;
        for (folder, other) in order {
            cancel.check()?;
            let reference = PackageRef {
                folder: folder.clone(),
                language: other.map_or(ModuleLanguage::Rust, |i| others[i].language),
            };
            if selected.as_ref().is_some_and(|list| !list.contains(folder.as_slice())) {
                continue;
            }
            if packages_planned >= CODE_MAP_PACKAGES {
                out.unread.push(reference);
                continue;
            }
            packages_planned += 1;
            let (files, own): (BTreeSet<Vec<u8>>, Batch) = match other {
                None => {
                    let mut files = rust_files(&folder, &blobs);
                    if let Some(script) = builds.get(&folder) { files.insert(script.clone()); }
                    let own = files.iter().filter_map(|p| readable(&blobs, p)).collect();
                    (files, own)
                }
                Some(i) => (BTreeSet::new(), others[i].files.iter().filter_map(|p| readable(&blobs, p)).collect()),
            };
            let size: usize = own.iter().map(|(_, _, s)| *s).sum();
            if !budget.reserve(own.len(), size) {
                out.unread.push(reference);
                continue;
            }
            batch.extend(own);
            planned.push((reference, other, files));
        }
        let mut texts = self.code_texts("read-sources", &batch, cancel)?;
        drop(batch);
        for (path, text) in &texts {
            out.lines.insert(path.clone(), text.lines().count().max(1));
        }
        let known: HashSet<&[u8]> = blobs.keys().copied().collect();
        let go_folders: HashSet<&[u8]> = blobs.keys().filter(|p| p.ends_with(b".go")).map(|p| packages::parent(p)).collect();
        // A Go package at the top of the snapshot has no folder card: its files stand for it.
        let top_go: Vec<Vec<u8>> = blobs.keys().filter(|p| packages::parent(p).is_empty() && p.ends_with(b".go") && !p.ends_with(b"_test.go")).map(|p| p.to_vec()).collect();
        // Every Java package and C# namespace the read files name, by the folder most of
        // their files are in: an import from another project finds its folder there.
        let mut java_counts: HashMap<String, BTreeMap<Vec<u8>, usize>> = HashMap::new();
        let mut csharp_counts: HashMap<String, BTreeMap<Vec<u8>, usize>> = HashMap::new();
        for (reference, other, _) in &planned {
            let (Some(i), language) = (other, reference.language) else { continue };
            let counts = match language {
                ModuleLanguage::Java => &mut java_counts,
                ModuleLanguage::CSharp => &mut csharp_counts,
                _ => continue,
            };
            for path in &others[*i].files {
                let Some(text) = texts.get(path) else { continue };
                let name = if language == ModuleLanguage::Java { jvm::package_clause(text) } else { csharp::namespace_clause(text) };
                *counts.entry(name.join(".")).or_default().entry(packages::parent(path).to_vec()).or_default() += 1;
            }
        }
        // Packages no file of this read names (a folder read, or past the read limits) are
        // found by their folders: `b/src/main/java/com/acme/lib` is com.acme.lib, and
        // `src/Shop.Core/Models` Shop.Core.Models, as both languages lay them out.
        let mut java_paths: HashMap<String, BTreeMap<Vec<u8>, usize>> = HashMap::new();
        let mut csharp_paths: HashMap<String, BTreeMap<Vec<u8>, usize>> = HashMap::new();
        for path in blobs.keys() {
            let counts = if packages::java_file(path) {
                &mut java_paths
            } else if packages::csharp_file(path) {
                &mut csharp_paths
            } else {
                continue;
            };
            let folder = packages::parent(path);
            let parts: Vec<&[u8]> = folder.split(|b| *b == b'/').filter(|d| !d.is_empty()).collect();
            for n in 1..=parts.len().min(16) {
                let name = String::from_utf8_lossy(&parts[parts.len() - n..].join(&b'.')).into_owned();
                *counts.entry(name).or_default().entry(folder.to_vec()).or_default() += 1;
            }
        }
        // The folder most of a package's files are in; the shallowest, then the first by
        // name, of those that hold as many (as each project's own units pick it).
        let main = |counts: HashMap<String, BTreeMap<Vec<u8>, usize>>| -> HashMap<String, Vec<u8>> {
            counts
                .into_iter()
                .filter(|(name, _)| !name.is_empty())
                .filter_map(|(name, folders)| folders.into_iter().max_by_key(|(dir, n)| (*n, std::cmp::Reverse(dir.len()), std::cmp::Reverse(dir.clone()))).map(|(dir, _)| (name, dir)))
                .collect()
        };
        let named = |read: HashMap<String, BTreeMap<Vec<u8>, usize>>, laid_out: HashMap<String, BTreeMap<Vec<u8>, usize>>| {
            let mut index = main(read);
            for (name, folder) in main(laid_out) {
                index.entry(name).or_insert(folder);
            }
            index
        };
        let (java_packages, csharp_namespaces) = (named(java_counts, java_paths), named(csharp_counts, csharp_paths));
        // A Java package or C# namespace at the top has no folder card: its files stand for it.
        let top_files = |keep: fn(&[u8]) -> bool| -> Vec<Vec<u8>> { blobs.keys().filter(|p| packages::parent(p).is_empty() && keep(p)).map(|p| p.to_vec()).collect() };
        let (top_java, top_csharp) = (top_files(packages::java_file), top_files(packages::csharp_file));
        let headers = units::Headers::new(blobs.keys().copied());
        let mut b = Builder::default();
        for (reference, other, files) in planned {
            cancel.check()?;
            match other {
                None => {
                    let name = &crates[&reference.folder];
                    let mut read = Read {
                        texts: &mut texts,
                        budget: &mut budget,
                        lines: &mut out.lines,
                        limited: &mut out.sources_limited,
                    };
                    self.code_rust(&reference.folder, name, builds.get(&reference.folder).map(Vec::as_slice), &files, &crate_names, &blobs, &mut read, &mut b, cancel)?;
                }
                Some(i) => {
                    let names = Names {
                        javascript: &javascript_names,
                        python: &python_names,
                        go: &go_modules,
                        java: &java_packages,
                        csharp: &csharp_namespaces,
                        top_java: &top_java,
                        top_csharp: &top_csharp,
                        known: &known,
                        headers: &headers,
                        go_folders: &go_folders,
                        top_go: &top_go,
                    };
                    out.sources_limited |= code_other(&others[i], &small, &names, &blobs, &texts, &mut b, cancel)?;
                }
            }
            out.packages.push(reference);
        }
        out.files = budget.files;
        out.limited = !out.unread.is_empty() || budget.limited || b.limited || manifest_limited || out.sources_limited || out.owners_limited;
        out.limited |= b.references.limited;
        out.references = b.references;
        out.ends = b.ends;
        out.links = b.links.into_iter().map(|((from, to), names)| CodeLink { from, to, names: names.into_iter().collect() }).collect();
        Ok(out)
    }

    /// The UTF-8 texts of `batch`, one Git process for each BATCH_BYTES of it.
    fn code_texts(&self, step: &'static str, batch: &[(Vec<u8>, ObjectId, usize)], cancel: &CancellationToken) -> Result<BTreeMap<Vec<u8>, String>> {
        let mut out = BTreeMap::new();
        let mut start = 0;
        while start < batch.len() {
            cancel.check()?;
            let mut end = start + 1;
            let mut bytes = batch[start].2;
            while end < batch.len() && bytes + batch[end].2 <= BATCH_BYTES {
                bytes += batch[end].2;
                end += 1;
            }
            let part = &batch[start..end];
            let input = part.iter().flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n'))).collect();
            let read = self.runner.step("map-code-links", step, || self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel))?;
            for ((path, _, _), blob) in part.iter().zip(parse_blobs(&read, part, cancel)?) {
                if let Some(text) = source_text(path, blob) {
                    out.insert(path.clone(), text);
                }
            }
            start = end;
        }
        Ok(out)
    }

    /// The first CODEOWNERS file found, applied to every file.
    fn code_owners(&self, blobs: &BTreeMap<&[u8], (ObjectId, usize)>, out: &mut CodeLinks, cancel: &CancellationToken) -> Result<()> {
        let Some((file, (id, size))) = CODEOWNERS_FILES.iter().find_map(|f| blobs.get_key_value(f.as_bytes())) else {
            return Ok(());
        };
        if *size > CODEOWNERS_BYTES {
            out.owners_too_large = true;
            return Ok(());
        }
        let bytes = self.map_read(&args(&["cat-file", "blob", id.as_str()]), Vec::new(), cancel)?;
        let text = String::from_utf8_lossy(&bytes);
        let rules = CodeOwners::parse(&text, file);
        let mut sets: Vec<(Vec<String>, usize)> = Vec::new();
        let mut set_ids = HashMap::new();
        let mut decided: Vec<(Vec<u8>, Option<usize>)> = Vec::new();
        let mut work = MatchWork::new(cancel);
        for path in blobs.keys() {
            cancel.check()?;
            match rules.owners_bounded(path, &mut work) {
                Ownership::Limited => {
                    cancel.check()?;
                    out.owners_limited = true;
                    return Ok(());
                }
                Ownership::Nobody => {}
                Ownership::Unowned => decided.push((path.to_vec(), None)),
                Ownership::Owners(list) => {
                    let at = *set_ids.entry(list.clone()).or_insert_with(|| {
                        sets.push((list, 0));
                        sets.len() - 1
                    });
                    sets[at].1 += 1;
                    decided.push((path.to_vec(), Some(at)));
                }
            }
        }
        // Most files first, so the colours go to the biggest owners.
        let mut order: Vec<usize> = (0..sets.len()).collect();
        order.sort_by(|a, b| sets[*b].1.cmp(&sets[*a].1).then_with(|| sets[*a].0.cmp(&sets[*b].0)));
        let mut renumber = vec![0u32; sets.len()];
        for (new, old) in order.iter().enumerate() {
            renumber[*old] = new as u32;
        }
        out.owners = Some(OwnerRules {
            file: file.to_vec(),
            gitlab: rules.gitlab,
            rules: rules.rules(),
            skipped: rules.skipped,
            sets: order.iter().map(|i| sets[*i].0.clone()).collect(),
            files: decided.into_iter().map(|(p, s)| (p, s.map(|s| renumber[s]))).collect(),
        });
        Ok(())
    }

    /// One Cargo package: its library and every other crate root, with `#[path]` files
    /// outside its usual folders read as their declarations ask.
    #[allow(clippy::too_many_arguments)]
    fn code_rust(
        &self,
        folder: &[u8],
        name: &str,
        build: Option<&[u8]>,
        files: &BTreeSet<Vec<u8>>,
        crate_names: &HashMap<String, Vec<u8>>,
        blobs: &BTreeMap<&[u8], (ObjectId, usize)>,
        read: &mut Read,
        b: &mut Builder,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut facts: HashMap<Vec<u8>, rust::Facts> = HashMap::new();
        for path in files {
            cancel.check()?;
            if let Some(text) = read.texts.get(path) {
                facts.insert(path.clone(), rust::read_with_budget(text, &mut read.budget.rust));
            }
        }
        let known = |p: &[u8]| blobs.contains_key(p);
        let (library, others) = rust_roots(folder, files);
        let mut roots: Vec<Vec<u8>> = library.iter().cloned().collect();
        roots.extend(others);
        let native = match build {
            Some(script) => self.native_definitions(folder, script, blobs, read, cancel)?,
            None => BTreeMap::new(),
        };
        for root in &roots {
            cancel.check()?;
            if !facts.contains_key(root) {
                continue;
            }
            let (mut crate_modules, mut wanted) = modules::tree(root, &facts, &known);
            for _ in 0..EXTRA_ROUNDS {
                let mut missing = Vec::new();
                let mut found = false;
                for path in std::mem::take(&mut wanted) {
                    cancel.check()?;
                    if facts.contains_key(&path) {
                        continue;
                    }
                    if let Some(text) = read.texts.get(&path) {
                        facts.insert(path.clone(), rust::read_with_budget(text, &mut read.budget.rust));
                        found = true;
                    } else if let Some(entry) = readable(blobs, &path) {
                        if read.budget.reserve(1, entry.2) {
                            missing.push(entry);
                        } else {
                            *read.limited = true;
                        }
                    }
                }
                if !missing.is_empty() {
                    for (path, text) in self.code_texts("read-sources", &missing, cancel)? {
                        read.lines.insert(path.clone(), text.lines().count().max(1));
                        facts.insert(path.clone(), rust::read_with_budget(&text, &mut read.budget.rust));
                        read.texts.insert(path, text);
                        found = true;
                    }
                }
                if !found {
                    break;
                }
                (crate_modules, wanted) = modules::tree(root, &facts, &known);
            }
            let found = modules::links(&crate_modules, &facts, cancel)?;
            if !native.is_empty() && Some(root.as_slice()) != build && root != &packages::join(folder, b"build.rs") {
                for (m, module) in crate_modules.iter().enumerate() {
                    cancel.check()?;
                    if let Some(f) = facts.get(&module.file) {
                        let declared = f.native.iter().map(|(_, _, symbol)| symbol);
                        let used = found.native.range((m, String::new())..).take_while(|(at, _)| *at == m).map(|(_, symbol)| symbol);
                        for symbol in declared.chain(used) {
                            if let Some(Some(file)) = native.get(symbol) {
                                b.link(&module.file, false, file, false, [symbol.clone()]);
                            }
                        }
                    }
                }
            }
            for ((from, to), names) in found.uses {
                b.link(&crate_modules[from].file, false, &crate_modules[to].file, false, names);
            }
            for (m, names) in found.outside {
                for used in names {
                    let Some(target) = crate_names.get(&used) else { continue };
                    if used == name {
                        // A test, example or program using its own package's library.
                        if let Some(lib) = &library
                            && Some(root) != library.as_ref()
                        {
                            b.link(&crate_modules[m].file, false, lib, false, [used.clone()]);
                        }
                    } else if target.is_empty() {
                        // A package at the top has no folder card: its library stands for it.
                        if let Some(lib) = [&b"src/lib.rs"[..], b"src/main.rs"].into_iter().find(|p| blobs.contains_key(*p)) {
                            b.link(&crate_modules[m].file, false, lib, false, [used.clone()]);
                        }
                    } else {
                        b.link(&crate_modules[m].file, false, target, true, [used.clone()]);
                    }
                }
            }
        }
        *read.limited |= facts.values().any(|f| f.limited);
        // The Folders view is file-based, including build scripts and test programs.
        let mut sources: Vec<_> = facts.iter().filter(|(_, f)| !f.embeds.is_empty()).collect();
        sources.sort_unstable_by_key(|(path, _)| *path);
        for (path, source) in sources {
            for embedded in source.embedded_files(path, blobs, cancel) {
                let (kind, target, line) = embedded?;
                if b.link(path, false, &target, false, [kind.to_owned()]) {
                    b.references.add(path, &target, line, kind);
                }
            }
        }
        Ok(())
    }

    /// Match symbols only among literal inputs of this crate's build. An unread
    /// input can hide a duplicate definition, so an incomplete set yields no links.
    fn native_definitions(
        &self, folder: &[u8], script: &[u8], blobs: &BTreeMap<&[u8], (ObjectId, usize)>,
        read: &mut Read, cancel: &CancellationToken,
    ) -> Result<BTreeMap<String, Option<Arc<[u8]>>>> {
        let mut symbols = BTreeMap::new();
        let Some(text) = read.texts.get(script) else { return Ok(symbols) };
        let inputs = rust::native_inputs(text, |p| {
            cargo::relative(packages::parent(script), p.as_bytes()).is_some_and(|path| blobs.contains_key(path.as_slice()))
        });
        let mut sources = BTreeMap::new();
        let mut batch = Vec::new();
        for (input, cpp) in inputs {
            cancel.check()?;
            let Some(path) = cargo::relative(folder, input.as_bytes()) else { return Ok(symbols) };
            if !packages::c_file(&path) || cfamily::header(&path) || !within_package(&path, folder, blobs) { return Ok(symbols); }
            let cpp = cpp || packages::cpp_file(&path) || path.ends_with(b".mm");
            if let Some(previous) = sources.insert(path.clone(), cpp) {
                if previous != cpp { return Ok(symbols); }
                continue;
            }
            if !read.texts.contains_key(&path) {
                let Some(entry) = readable(blobs, &path) else {
                    *read.limited |= blobs.contains_key(path.as_slice());
                    return Ok(symbols);
                };
                batch.push(entry);
            }
        }
        if !read.budget.reserve(batch.len(), batch.iter().map(|e| e.2).sum()) {
            *read.limited = true;
            return Ok(symbols);
        }
        for (path, text) in self.code_texts("read-sources", &batch, cancel)? {
            read.lines.insert(path.clone(), text.lines().count().max(1));
            read.texts.insert(path, text);
        }
        for (path, cpp) in sources {
            cancel.check()?;
            let Some(text) = read.texts.get(&path) else { return Ok(BTreeMap::new()) };
            let Some(definitions) = cfamily::definitions(text, cpp) else { *read.limited = true; return Ok(BTreeMap::new()) };
            let path = Arc::<[u8]>::from(path);
            for symbol in definitions {
                symbols.entry(symbol).and_modify(|p| *p = None).or_insert_with(|| Some(path.clone()));
            }
        }
        Ok(symbols)
    }
}

/// The file belongs to this package, including when the repo root is a workspace.
fn within_package(path: &[u8], folder: &[u8], blobs: &BTreeMap<&[u8], (ObjectId, usize)>) -> bool {
    let mut parent = packages::parent(path);
    while parent != folder {
        if parent.is_empty() || blobs.contains_key(packages::join(parent, b"Cargo.toml").as_slice()) { return false; }
        parent = packages::parent(parent);
    }
    true
}

/// The sources read so far, for a Rust package's `#[path]` rounds.
struct Read<'a> {
    texts: &'a mut BTreeMap<Vec<u8>, String>,
    budget: &'a mut Budget,
    lines: &'a mut BTreeMap<Vec<u8>, usize>,
    limited: &'a mut bool,
}

/// A committed file small enough to read, as `code_texts` takes it.
fn readable(blobs: &BTreeMap<&[u8], (ObjectId, usize)>, path: &[u8]) -> Option<(Vec<u8>, ObjectId, usize)> {
    blobs.get(path).filter(|(_, size)| *size <= FILE_BYTES).map(|(id, size)| (path.to_vec(), id.clone(), *size))
}

/// A Cargo package's `.rs` files in `src`, `tests`, `examples` and `benches`, and its
/// `build.rs`.
fn rust_files(folder: &[u8], blobs: &BTreeMap<&[u8], (ObjectId, usize)>) -> BTreeSet<Vec<u8>> {
    let mut out = BTreeSet::new();
    for dir in [&b"src"[..], b"tests", b"examples", b"benches"] {
        let start = [packages::join(folder, dir).as_slice(), b"/"].concat();
        for (path, _) in blobs.range(start.as_slice()..) {
            if !path.starts_with(&start) {
                break;
            }
            if path.ends_with(b".rs") {
                out.insert(path.to_vec());
            }
        }
    }
    let build = packages::join(folder, b"build.rs");
    if blobs.contains_key(build.as_slice()) {
        out.insert(build);
    }
    out
}

/// Connect a non-Cargo group as Modules reads it. Return whether its source analysis was bounded.
fn code_other(
    f: &packages::Found,
    small: &BTreeMap<Vec<u8>, String>,
    names: &Names,
    blobs: &BTreeMap<&[u8], (ObjectId, usize)>,
    texts: &BTreeMap<Vec<u8>, String>,
    b: &mut Builder,
    cancel: &CancellationToken,
) -> Result<bool> {
    let own: BTreeMap<Vec<u8>, String> = f.files.iter().filter_map(|p| texts.get(p).map(|t| (p.clone(), t.clone()))).collect();
    let module = names.go.iter().find(|(_, folder)| *folder == f.folder).map(|(m, _)| m.clone()).unwrap_or_default();
    let built = match f.language {
        ModuleLanguage::JavaScript => {
            let configs = [packages::join(&f.folder, b"tsconfig.json"), packages::join(&f.folder, b"jsconfig.json")];
            let aliases = configs.iter().find_map(|c| small.get(c)).map(|text| packages::aliases(text, &f.folder)).unwrap_or_default();
            units::javascript(&f.folder, &own, names.known, &aliases, cancel)?
        }
        ModuleLanguage::Python => units::python(&f.folder, f.manifest.is_some(), &own, cancel)?,
        ModuleLanguage::Java => units::java(&own, &f.files, cancel)?,
        ModuleLanguage::C => units::c(&f.folder, &own, &f.files, names.known, names.headers, cancel)?,
        ModuleLanguage::CSharp => units::csharp(&own, cancel)?,
        ModuleLanguage::Lua => units::lua(f, &own, cancel)?,
        ModuleLanguage::GitHubActions | ModuleLanguage::HtmlCss => units::file_dependencies(f, &own, names.known, cancel)?,
        ModuleLanguage::PowerShell | ModuleLanguage::Bash => units::scripts(f.language, &f.folder, &own, names.known, cancel)?,
        ModuleLanguage::Go | ModuleLanguage::Rust => units::go(&f.folder, &module, &own, &f.files, cancel)?,
    };
    // A unit is a file, or a folder for a Go or Java package and a C# namespace. The
    // top folder has no card, so a package there stands as its files.
    let ends = |path: &[u8]| -> Vec<(Vec<u8>, bool)> {
        if blobs.contains_key(path) {
            vec![(path.to_vec(), false)]
        } else if path.is_empty() {
            match f.language {
                ModuleLanguage::Go => names.top_go.iter().map(|p| (p.clone(), false)).collect(),
                ModuleLanguage::Java | ModuleLanguage::CSharp => f.files.iter().filter(|p| packages::parent(p).is_empty()).map(|p| (p.clone(), false)).collect(),
                _ => Vec::new(),
            }
        } else {
            vec![(path.to_vec(), true)]
        }
    };
    for ((from, to), used) in &built.links {
        for (a, a_folder) in ends(&built.units[*from].path) {
            for (z, z_folder) in ends(&built.units[*to].path) {
                if b.link(&a, a_folder, &z, z_folder, used.iter().cloned())
                    && let Some(sites) = built.sites.get(&(*from, *to))
                {
                    for &(line, kind) in sites { b.references.add(&a, &z, line, kind); }
                }
            }
        }
    }
    for (u, outside) in &built.outside {
        let from = ends(&built.units[*u].path);
        for name in outside {
            // A relative import or an include into another project names the file itself.
            if matches!(f.language, ModuleLanguage::JavaScript | ModuleLanguage::C | ModuleLanguage::PowerShell | ModuleLanguage::Bash) && names.known.contains(name.as_bytes()) {
                for (a, a_folder) in &from {
                    b.link(a, *a_folder, name.as_bytes(), false, [name.clone()]);
                }
                continue;
            }
            // A package or namespace by its whole name: a shorter one is a parent, and
            // `com.acme.payments` from a library is no part of `com.acme`.
            let target: Option<Vec<u8>> = match f.language {
                ModuleLanguage::JavaScript => names.javascript.get(name).cloned(),
                ModuleLanguage::Python => names.python.get(name.split('.').next().unwrap_or_default()).cloned(),
                ModuleLanguage::Java => names.java.get(name).cloned(),
                ModuleLanguage::CSharp => names.csharp.get(name).cloned(),
                ModuleLanguage::C | ModuleLanguage::PowerShell | ModuleLanguage::Bash | ModuleLanguage::Lua | ModuleLanguage::GitHubActions | ModuleLanguage::HtmlCss => None,
                ModuleLanguage::Go | ModuleLanguage::Rust => names.go.iter().find(|(m, _)| name == m || name.starts_with(&format!("{m}/"))).map(|(m, folder)| {
                    let rest = name[m.len()..].trim_start_matches('/');
                    let package = if rest.is_empty() { folder.clone() } else { packages::join(folder, rest.as_bytes()) };
                    // The package's own folder when it holds Go files, else the module's.
                    if names.go_folders.contains(package.as_slice()) { package } else { folder.clone() }
                }),
            };
            let Some(target) = target.filter(|t| *t != f.folder) else { continue };
            // A package at the top has no card: its package.json, or its files.
            let targets = match (target.is_empty(), f.language) {
                (false, _) => vec![(target, true)],
                (true, ModuleLanguage::JavaScript) => vec![(b"package.json".to_vec(), false)],
                (true, ModuleLanguage::Go) => names.top_go.iter().map(|p| (p.clone(), false)).collect(),
                (true, ModuleLanguage::Java) => names.top_java.iter().map(|p| (p.clone(), false)).collect(),
                (true, ModuleLanguage::CSharp) => names.top_csharp.iter().map(|p| (p.clone(), false)).collect(),
                (true, _) => Vec::new(),
            };
            for (a, a_folder) in &from {
                for (z, z_folder) in &targets {
                    b.link(a, *a_folder, z, *z_folder, [name.clone()]);
                }
            }
        }
    }
    Ok(built.limited)
}

struct Names<'a> {
    javascript: &'a HashMap<String, Vec<u8>>,
    python: &'a HashMap<String, Vec<u8>>,
    go: &'a [(String, Vec<u8>)],
    /// Java's packages and C#'s namespaces, by the folder most of their files are in.
    java: &'a HashMap<String, Vec<u8>>,
    csharp: &'a HashMap<String, Vec<u8>>,
    /// The Java and Kotlin files, and the C# files, at the top, which stand for a package
    /// or namespace there.
    top_java: &'a [Vec<u8>],
    top_csharp: &'a [Vec<u8>],
    /// Every committed path, for JavaScript imports and C includes.
    known: &'a HashSet<&'a [u8]>,
    /// The C and C++ files by name, for includes.
    headers: &'a units::Headers,
    /// Folders holding Go files.
    go_folders: &'a HashSet<&'a [u8]>,
    /// The Go files of a package at the top, which stand for it.
    top_go: &'a [Vec<u8>],
}

/// One commit of the history read.
#[derive(Clone, Debug)]
pub struct HistoryCommit {
    pub id: ObjectId,
    /// Author time, seconds since 1970.
    pub time: i64,
    /// Index into `CodeHistory::authors`.
    pub author: u32,
}
/// Who changed which committed paths, and when: Activity and Owner (most commits).
#[derive(Clone, Debug, Default)]
pub struct CodeHistory {
    pub tip: Option<ObjectId>,
    /// Newest first.
    pub commits: Vec<HistoryCommit>,
    /// People as the snapshot's .mailmap names them, with the name of their newest commit.
    pub authors: Vec<Vec<u8>>,
    /// Each path's commits, as indices into `commits`, newest first.
    pub paths: BTreeMap<Vec<u8>, Vec<u32>>,
    /// More commits exist than were read.
    pub limited: bool,
}
impl CodeHistory {
    /// Whether commit `c` was authored at or after `since` (always for None). Each commit
    /// is checked: the log runs by commit date, and a rebased or cherry-picked commit keeps
    /// an older author date.
    fn within(&self, c: u32, since: Option<i64>) -> bool {
        since.is_none_or(|s| self.commits[c as usize].time >= s)
    }
    /// How many of the commits read were authored at or after `since` (all for None).
    pub fn newer_than(&self, since: Option<i64>) -> usize {
        match since {
            None => self.commits.len(),
            Some(s) => self.commits.iter().filter(|c| c.time >= s).count(),
        }
    }
    /// Commits since `since` that changed the file at `path`.
    pub fn file_commits(&self, path: &[u8], since: Option<i64>) -> usize {
        self.paths.get(path).map_or(0, |list| list.iter().filter(|c| self.within(**c, since)).count())
    }
    /// Commits since `since` that changed anything under `folder` (empty: the whole
    /// snapshot), each commit once; with `own`, only the folder's own files.
    pub fn folder_commits(&self, folder: &[u8], own: bool, since: Option<i64>) -> usize {
        // One bit a commit read: a few KiB for 10,000 commits, cheap to make per card.
        let mut seen = vec![0u64; self.commits.len().div_ceil(64)];
        let mut count = 0;
        let start = if folder.is_empty() { Vec::new() } else { [folder, b"/"].concat() };
        for (path, list) in self.paths.range(start.clone()..) {
            if !path.starts_with(&start) {
                break;
            }
            if own && path[start.len()..].contains(&b'/') {
                continue;
            }
            for &c in list {
                let (word, bit) = (c as usize / 64, 1u64 << (c % 64));
                if seen[word] & bit == 0 && self.within(c, since) {
                    seen[word] |= bit;
                    count += 1;
                }
            }
        }
        count
    }
    /// The person with the most commits to the file, all commits read; on a tie, the
    /// one whose commit is newest. With their commit count.
    pub fn owner(&self, path: &[u8]) -> Option<(u32, usize)> {
        let list = self.paths.get(path)?;
        let mut counts: HashMap<u32, (usize, u32)> = HashMap::new();
        for &c in list {
            let e = counts.entry(self.commits[c as usize].author).or_insert((0, c));
            e.0 += 1;
            e.1 = e.1.min(c);
        }
        counts.into_iter().max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.1.1.cmp(&a.1.1))).map(|(author, (n, _))| (author, n))
    }
    /// The newest commit that changed anything under `folder`, or the file itself.
    pub fn last(&self, path: &[u8], folder: bool) -> Option<&HistoryCommit> {
        if !folder {
            return self.paths.get(path).and_then(|l| l.first()).map(|c| &self.commits[*c as usize]);
        }
        let start = if path.is_empty() { Vec::new() } else { [path, b"/"].concat() };
        self.paths
            .range(start.clone()..)
            .take_while(|(p, _)| p.starts_with(&start))
            .filter_map(|(_, l)| l.first())
            .min()
            .map(|c| &self.commits[*c as usize])
    }
}

/// How a Changes read ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangesOutcome {
    Ready,
    /// The base names no branch, tag or commit here.
    BaseNotFound,
    /// The two have no history in common.
    NoCommonHistory,
    /// They split at more than one commit; choose one of those commits as the base.
    SeveralSplits,
}
/// What the snapshot changes since it split from a base, as a pull request compares.
#[derive(Clone, Debug)]
pub struct CodeChanges {
    pub outcome: ChangesOutcome,
    /// The base as it was typed.
    pub base: String,
    pub base_id: Option<ObjectId>,
    /// Where the two split.
    pub split: Option<ObjectId>,
    /// SeveralSplits: the commits they split at, up to four, any of which can be typed.
    pub splits: Vec<ObjectId>,
    pub commits: usize,
    pub files: Vec<ComparedFile>,
    /// Lines added and removed by path; absent for binary files, or all of them when the
    /// counts were too large to read.
    pub lines: HashMap<Vec<u8>, (usize, usize)>,
}

impl Repository {
    /// Who changed which paths of the snapshot and when: the newest commits, merges left
    /// out, people as the snapshot's .mailmap names them.
    pub fn code_history(&self, tip: &ObjectId, cancel: &CancellationToken) -> Result<CodeHistory> {
        let op = "map-code-history";
        cancel.check()?;
        let aliases = self.runner.step(op, "read-mailmap", || self.map_aliases(tip, cancel))?;
        let run = |count: usize| -> Result<Vec<u8>> {
            let mut runner = self.runner.clone();
            runner.limits.stdout_bytes = HISTORY_BYTES;
            // Bound to this repository as the other reads are, so a vanished .git marker
            // cannot send Git to a folder above.
            let info = self.info();
            let mut arguments = args(&["--git-dir"]);
            arguments.push(crate::runner::git_path(&info.git_dir));
            if let Some(worktree) = &info.worktree {
                arguments.push("--work-tree".into());
                arguments.push(crate::runner::git_path(worktree));
            }
            // log.showSignature would put signature checks between the commits.
            arguments.extend(args(&["--no-replace-objects", "log", "-z", "--no-merges", "--no-renames", "--no-show-signature", "--name-only", "--format=%x00%H%x00%at%x00%an%x00%ae"]));
            arguments.push(format!("--max-count={}", count + 1).into());
            arguments.extend(args(&[tip.as_str(), "--"]));
            runner.run(self.cwd(), &arguments, Vec::new(), cancel)?.success(op)
        };
        let (bytes, wanted) = match self.runner.step(op, "read-history", || run(CODE_HISTORY_COMMITS)) {
            // Too much for one read: the newest 2,000 commits instead.
            Err(Error::OutputLimit { .. } | Error::TimedOut) => (self.runner.step(op, "read-history", || run(HISTORY_FALLBACK))?, HISTORY_FALLBACK),
            other => (other?, CODE_HISTORY_COMMITS),
        };
        cancel.check()?;
        parse_code_history(&bytes, tip, &aliases, wanted, cancel)
    }
}

// An empty field introduces four metadata fields. A Git pathname is always
// nonempty and cannot contain NUL, including paths starting with control bytes.
fn parse_code_history(bytes: &[u8], tip: &ObjectId, aliases: &[Alias], wanted: usize, cancel: &CancellationToken) -> Result<CodeHistory> {
    let op = "map-code-history";
    let mut history = CodeHistory {
        tip: Some(tip.clone()),
        ..Default::default()
    };
    let mut people: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut first = false;
    let mut current: Option<u32> = None;
    let mut tokens = bytes.split_inclusive(|b| *b == 0).map(|field| field.strip_suffix(&[0]));
    while let Some(field) = tokens.next() {
        cancel.check()?;
        let token = field.ok_or_else(|| Error::parse(op, "unterminated field"))?;
        if token.is_empty() {
            if history.commits.len() >= wanted {
                history.limited = true;
                break;
            }
            let mut next = || tokens.next().flatten().ok_or_else(|| Error::parse(op, "incomplete commit header"));
            let (id, time, name, email) = (next()?, next()?, next()?, next()?);
            let id = parse::oid(id, op)?;
            let time = std::str::from_utf8(time).ok().and_then(|t| t.parse::<i64>().ok()).ok_or_else(|| Error::parse(op, "invalid author time"))?;
            let (name, email) = canonical(name, email, aliases);
            let key = if email.is_empty() { name.clone() } else { email.to_ascii_lowercase() };
            let author = *people.entry(key).or_insert_with(|| {
                history.authors.push(name.clone());
                (history.authors.len() - 1) as u32
            });
            history.commits.push(HistoryCommit { id, time, author });
            current = Some((history.commits.len() - 1) as u32);
            first = true;
            continue;
        }
        // Git puts a line break between the format line and the first name.
        let path = if std::mem::take(&mut first) { token.strip_prefix(b"\n").ok_or_else(|| Error::parse(op, "missing path separator"))? } else { token };
        let c = current.filter(|_| !path.is_empty()).ok_or_else(|| Error::parse(op, "unexpected pathname"))?;
        let list = history.paths.entry(path.to_vec()).or_default();
        if list.last() != Some(&c) {
            list.push(c);
        }
    }
    Ok(history)
}

impl Repository {
    /// What `tip` changes since it split from `base`: the files, lines added and removed,
    /// and how many commits.
    pub fn code_changes(&self, base: &str, tip: &ObjectId, cancel: &CancellationToken) -> Result<CodeChanges> {
        let op = "map-code-changes";
        cancel.check()?;
        let typed = base.trim().to_owned();
        let mut out = CodeChanges {
            outcome: ChangesOutcome::Ready,
            base: typed.clone(),
            base_id: None,
            split: None,
            splits: Vec::new(),
            commits: 0,
            files: Vec::new(),
            lines: HashMap::new(),
        };
        let resolved = self.runner.step(op, "resolve-commit", || {
            let bytes = self.read_diff("resolve commit", &commit_revision_args(&typed)?, cancel)?;
            parse::oid(parse::line(&bytes), op)
        });
        let base_id = match resolved {
            Ok(id) => id,
            Err(Error::Git { .. } | Error::InvalidInput(_)) => {
                out.outcome = ChangesOutcome::BaseNotFound;
                return Ok(out);
            }
            Err(e) => return Err(e),
        };
        out.base_id = Some(base_id.clone());
        let output = self.runner.step(op, "merge-base", || {
            self.runner.run(self.cwd(), &args(&["merge-base", "--all", base_id.as_str(), tip.as_str()]), Vec::new(), cancel)
        })?;
        if output.status.code() == Some(1) {
            out.outcome = ChangesOutcome::NoCommonHistory;
            return Ok(out);
        }
        let bases = output.success(op)?;
        let bases: Vec<&[u8]> = bases.split(|b| *b == b'\n').filter(|b| !b.is_empty()).collect();
        if bases.len() != 1 {
            out.outcome = if bases.is_empty() { ChangesOutcome::NoCommonHistory } else { ChangesOutcome::SeveralSplits };
            out.splits = bases.iter().take(4).map(|b| parse::oid(parse::line(b), op)).collect::<Result<_>>()?;
            return Ok(out);
        }
        let split = parse::oid(parse::line(bases[0]), op)?;
        out.split = Some(split.clone());
        let comparison = self.compare_revisions(split.as_str(), tip.as_str(), cancel)?;
        out.files = comparison.files;
        let range = format!("{split}..{tip}");
        let count = self.read_diff("change commit count", &args(&["rev-list", "--count", "--no-merges", range.as_str(), "--"]), cancel)?;
        out.commits = std::str::from_utf8(parse::line(&count)).ok().and_then(|s| s.parse().ok()).ok_or_else(|| Error::parse(op, "invalid commit count"))?;
        let numstat = self.runner.step(op, "count-lines", || {
            self.read_diff(
                "change line counts",
                &args(&["diff", "--numstat", "-z", "--no-color", "--no-ext-diff", "--no-textconv", "--no-relative", "--find-renames", split.as_str(), tip.as_str(), "--"]),
                cancel,
            )
        });
        match numstat {
            Ok(bytes) => out.lines = parse_numstat(&bytes),
            // Line counts are a courtesy: a diff too large to count keeps the files.
            Err(Error::OutputLimit { .. }) => {}
            Err(e) => return Err(e),
        }
        Ok(out)
    }
}

/// `git diff --numstat -z`: `added\tremoved\tpath\0`, or for a rename
/// `added\tremoved\t\0old\0new\0`; `-` counts for binary files.
fn parse_numstat(bytes: &[u8]) -> HashMap<Vec<u8>, (usize, usize)> {
    let mut out = HashMap::new();
    let mut tokens = bytes.split(|b| *b == 0).peekable();
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            continue;
        }
        let mut fields = token.splitn(3, |b| *b == b'\t');
        let (Some(added), Some(removed), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let path = if path.is_empty() {
            // A rename: the old path, then the new one.
            let _old = tokens.next();
            tokens.next().unwrap_or_default()
        } else {
            path
        };
        let number = |v: &[u8]| std::str::from_utf8(v).ok().and_then(|s| s.parse::<usize>().ok());
        if let (Some(a), Some(r)) = (number(added), number(removed)) {
            out.insert(path.to_vec(), (a, r));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_control_bytes_in_paths_and_metadata() {
        let tip = ObjectId::parse(&"a".repeat(40)).unwrap();
        let oid = tip.as_str().as_bytes();
        let fields: &[&[u8]] = &[b"", oid, b"0", b"Lin\x02A", b"", b"\n\nfirst", b"\x01notes",
            b"", oid, b"1", b"Empty", b"", b"", oid, b"2", b"Lin\x02A", b"", b"\n\x01notes"];
        let bytes: Vec<_> = fields.iter().flat_map(|field| field.iter().copied().chain(Some(0))).collect();
        let cancel = CancellationToken::default();
        let history = parse_code_history(&bytes, &tip, &[], 3, &cancel).unwrap();
        assert_eq!(history.commits.len(), 3);
        assert_eq!(history.authors, [b"Lin\x02A".to_vec(), b"Empty".to_vec()]);
        assert_eq!(history.paths[b"\nfirst".as_slice()], [0]);
        assert_eq!(history.paths[b"\x01notes".as_slice()], [0, 2]);
        assert!(!history.limited);
        assert!(parse_code_history(&bytes[..bytes.len()-1], &tip, &[], 3, &cancel).is_err());
        assert!(parse_code_history(b"\0header", &tip, &[], 3, &cancel).is_err());
    }

    #[test]
    fn source_reservations_refuse_count_bytes_and_overflow_without_consuming_space() {
        let mut b = Budget::default();
        assert!(b.reserve(CODE_MAP_FILES, 1));
        assert!(!b.reserve(1, 0));
        assert!(!b.reserve(usize::MAX, usize::MAX));
        assert_eq!((b.files, b.bytes), (CODE_MAP_FILES, 1));
        let mut b = Budget::default();
        assert!(b.reserve(1, CODE_MAP_BYTES));
        assert!(!b.reserve(1, 1));
        assert_eq!((b.files, b.bytes), (1, CODE_MAP_BYTES));
        let mut links = CodeLinks { sources_limited: true, limited: true, ..Default::default() };
        links.merge(CodeLinks::default());
        assert!(links.limited, "another folder read cannot hide missing source data");
    }

    #[test]
    fn kinds_follow_names_and_test_folders() {
        let k = |p: &str| file_kind(p.as_bytes());
        assert_eq!(k("crates/rocket-ui/src/app.rs"), FileKind::Code);
        assert_eq!(k("crates/rocket-ui/src/app_tests.rs"), FileKind::Tests);
        assert_eq!(k("crates/rocket-git/src/conflict/tests.rs"), FileKind::Tests);
        assert_eq!(k("crates/rocket-git/tests/common/mod.rs"), FileKind::Tests);
        assert_eq!(k("tools/macos/test-retained-package.cjs"), FileKind::Tests);
        assert_eq!(k("web/src/button.spec.tsx"), FileKind::Tests);
        assert_eq!(k("pkg/server_test.go"), FileKind::Tests);
        assert_eq!(k("lib/test_parser.py"), FileKind::Tests);
        assert_eq!(k("src/main/java/AppTest.java"), FileKind::Tests);
        assert_eq!(k("README.md"), FileKind::Docs);
        assert_eq!(k("LICENSE-MIT"), FileKind::Docs);
        assert_eq!(k("THIRD-PARTY-NOTICES.txt"), FileKind::Docs);
        assert_eq!(k("requirements-dev.txt"), FileKind::Config);
        assert_eq!(k("Cargo.toml"), FileKind::Config);
        assert_eq!(k("Cargo.lock"), FileKind::Config);
        assert_eq!(k(".gitignore"), FileKind::Config);
        assert_eq!(k("Dockerfile"), FileKind::Config);
        assert_eq!(k(".github/workflows/ci.yml"), FileKind::Config);
        assert_eq!(k("docs/media/map-dark.webp"), FileKind::Other);
        assert_eq!(k("assets/logo.svg"), FileKind::Other);
        assert_eq!(k("docs/site.css"), FileKind::Code);
        assert_eq!(k("tests/fixtures/data.json"), FileKind::Config, "data stays data inside tests");
        assert_eq!(k("notes"), FileKind::Other);
        // Document names are documents only when they are not code.
        assert_eq!(k("CHANGES"), FileKind::Docs);
        assert_eq!(k("crates/rocket-ui/src/agents/changes.rs"), FileKind::Code);
        assert_eq!(k("crates/rocket-ui/src/changes_tests.rs"), FileKind::Tests);
        assert_eq!(k("src/license.py"), FileKind::Code);
        assert_eq!(k("modules/math.cppm"), FileKind::Code);
        assert_eq!(k("src/changes.ixx"), FileKind::Code);
        assert_eq!(k("tests/partition.cxxm"), FileKind::Tests);
    }

    #[test]
    fn numstat_reads_renames_and_skips_binary_counts() {
        let bytes = b"3\t1\tsrc/a.rs\0-\t-\tlogo.png\x000\t0\t\0old/b.rs\0new/b.rs\0";
        let lines = parse_numstat(bytes);
        assert_eq!(lines.get(b"src/a.rs".as_slice()), Some(&(3, 1)));
        assert_eq!(lines.get(b"new/b.rs".as_slice()), Some(&(0, 0)));
        assert!(!lines.contains_key(b"logo.png".as_slice()));
        assert!(!lines.contains_key(b"old/b.rs".as_slice()));
    }

    #[test]
    fn merging_a_folder_read_reuses_ends_and_links() {
        let end = |p: &str, folder| CodeEnd { path: p.as_bytes().to_vec(), folder };
        let mut first = CodeLinks {
            ends: vec![end("a/x.rs", false), end("b", true)],
            links: vec![CodeLink { from: 0, to: 1, names: vec!["b".into()] }],
            unread: vec![PackageRef { folder: b"c".to_vec(), language: ModuleLanguage::Rust }],
            limited: true,
            files: 3,
            ..Default::default()
        };
        let second = CodeLinks {
            ends: vec![end("c/y.rs", false), end("b", true), end("a/x.rs", false)],
            links: vec![CodeLink { from: 0, to: 1, names: vec!["b".into()] }, CodeLink { from: 2, to: 1, names: vec!["more".into()] }],
            packages: vec![PackageRef { folder: b"c".to_vec(), language: ModuleLanguage::Rust }],
            files: 2,
            ..Default::default()
        };
        first.merge(second);
        assert_eq!(first.ends.len(), 3);
        assert_eq!(first.links.len(), 2);
        assert_eq!(first.links[0].names, vec!["b".to_string(), "more".to_string()]);
        assert_eq!(first.files, 5);
        assert!(first.unread.is_empty());
        assert!(!first.limited);
    }

    #[test]
    fn history_counts_each_commit_once_per_folder_and_picks_owners() {
        let id = |n: u8| ObjectId::parse(&format!("{:040x}", n)).unwrap();
        let history = CodeHistory {
            tip: None,
            commits: vec![
                HistoryCommit { id: id(3), time: 300, author: 0 },
                HistoryCommit { id: id(2), time: 200, author: 1 },
                HistoryCommit { id: id(1), time: 100, author: 1 },
            ],
            authors: vec![b"Ann".to_vec(), b"Bo".to_vec()],
            paths: [
                (b"src/a.rs".to_vec(), vec![0, 1]),
                (b"src/b.rs".to_vec(), vec![1, 2]),
                (b"src/deep/c.rs".to_vec(), vec![0]),
                (b"top.md".to_vec(), vec![2]),
            ]
            .into_iter()
            .collect(),
            limited: false,
        };
        assert_eq!(history.folder_commits(b"src", false, None), 3);
        assert_eq!(history.folder_commits(b"src", true, None), 3);
        assert_eq!(history.folder_commits(b"src", false, Some(150)), 2);
        assert_eq!(history.folder_commits(b"", true, None), 1);
        assert_eq!(history.file_commits(b"src/a.rs", Some(250)), 1);
        // Bo has two commits to b.rs; a.rs is one each, and Ann's is newer.
        assert_eq!(history.owner(b"src/b.rs"), Some((1, 2)));
        assert_eq!(history.owner(b"src/a.rs"), Some((0, 1)));
        assert_eq!(history.last(b"src", true).map(|c| c.time), Some(300));
        assert_eq!(history.last(b"top.md", false).map(|c| c.time), Some(100));
        // The log runs by commit date: a commit rebased today keeps its old author date,
        // so each commit's own date decides, not its place in the log.
        let rebased = CodeHistory {
            commits: vec![
                HistoryCommit { id: id(3), time: 100, author: 0 },
                HistoryCommit { id: id(2), time: 300, author: 0 },
                HistoryCommit { id: id(1), time: 250, author: 0 },
            ],
            authors: vec![b"Ann".to_vec()],
            paths: [(b"src/a.rs".to_vec(), vec![0, 1, 2])].into_iter().collect(),
            ..history
        };
        assert_eq!(rebased.newer_than(Some(200)), 2);
        assert_eq!(rebased.file_commits(b"src/a.rs", Some(200)), 2);
        assert_eq!(rebased.folder_commits(b"src", false, Some(200)), 2);
        assert_eq!(rebased.folder_commits(b"", false, None), 3);
    }
}
