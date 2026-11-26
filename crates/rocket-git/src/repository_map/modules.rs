//! Map → Code → Modules: one Rust package's modules and which uses which, from the
//! committed files of the snapshot. Files are read as text (see `rust.rs`): nothing is
//! compiled, no build script or macro runs, and other packages are not followed.
use super::rust::{self, Facts};
use super::*;
use std::collections::HashSet;

/// Rust files read from one package's `src` folder, and their size limits.
const FILES: usize = 2000;
const FILE_BYTES: usize = 1024 * 1024;
const TOTAL_BYTES: usize = 32 * 1024 * 1024;
/// Reads for files a `#[path]` attribute names outside `src`.
const EXTRA_ROUNDS: usize = 8;
/// Outline items kept per file, and names kept per connection.
const OUTLINE: usize = 400;
const NAMES: usize = 64;
const DEPTH: usize = 64;

/// One module with a file of its own.
#[derive(Clone, Debug)]
pub(super) struct Module {
    /// Its path in the crate; empty for the crate root.
    pub path: Vec<String>,
    pub file: Vec<u8>,
    own_folder: bool,
    pub parent: Option<usize>,
    pub test: bool,
}

fn directory(file: &[u8]) -> &[u8] {
    file.rsplitn(2, |b| *b == b'/').nth(1).unwrap_or_default()
}
fn join(base: &[u8], part: &[u8]) -> Vec<u8> {
    if base.is_empty() {
        part.to_vec()
    } else {
        [base, b"/", part].concat()
    }
}
/// Where `mod name;` lines in a module's file look for their files: beside a root,
/// `mod.rs` or `#[path]` file, and in a folder named after any other file.
fn child_directory(file: &[u8], own_folder: bool) -> Vec<u8> {
    let folder = directory(file).to_vec();
    if own_folder {
        return folder;
    }
    let name = file.rsplit(|b| *b == b'/').next().unwrap_or_default();
    join(&folder, name.strip_suffix(b".rs").unwrap_or(name))
}

impl Module {
    fn declaration_files(&self, d: &rust::Declaration) -> Vec<(Vec<u8>, bool)> {
        let inline = d.chain.iter().fold(child_directory(&self.file, self.own_folder), |dir, part| join(&dir, part.as_bytes()));
        match &d.path {
            Some(custom) => {
                let base = if d.chain.is_empty() { directory(&self.file) } else { &inline };
                cargo::relative(base, custom.as_bytes()).into_iter().map(|p| (p, true)).collect()
            }
            None => vec![
                (join(&inline, format!("{}.rs", d.name).as_bytes()), false),
                (join(&join(&inline, d.name.as_bytes()), b"mod.rs"), true),
            ],
        }
    }
}

/// The module tree that `root` declares. Files not read yet that a declaration names
/// are returned in `wanted` when `known` says the snapshot has them.
pub(super) fn tree(
    root: &[u8],
    facts: &HashMap<Vec<u8>, Facts>,
    known: &dyn Fn(&[u8]) -> bool,
) -> (Vec<Module>, BTreeSet<Vec<u8>>) {
    let mut modules = vec![Module {
        path: Vec::new(),
        file: root.to_vec(),
        own_folder: true,
        parent: None,
        test: false,
    }];
    let mut used: HashSet<Vec<u8>> = HashSet::from([root.to_vec()]);
    let mut wanted = BTreeSet::new();
    let mut at = 0;
    while at < modules.len() {
        let (file, path, test) = (modules[at].file.clone(), modules[at].path.clone(), modules[at].test);
        let Some(read) = facts.get(&file) else {
            at += 1;
            continue;
        };
        for d in &read.declarations {
            if path.len() + d.chain.len() >= DEPTH || modules.len() >= MAP_NODES {
                break;
            }
            let candidates = modules[at].declaration_files(d);
            let Some((target, folder)) = candidates.iter().find(|(p, _)| facts.contains_key(p)).cloned() else {
                if let Some((p, _)) = candidates.iter().find(|(p, _)| known(p)) {
                    wanted.insert(p.clone());
                }
                continue;
            };
            if !used.insert(target.clone()) {
                continue;
            }
            let mut child = path.clone();
            child.extend(d.chain.iter().cloned());
            child.push(d.name.clone());
            modules.push(Module {
                path: child,
                file: target,
                own_folder: folder,
                parent: Some(at),
                test: test || d.test,
            });
        }
        at += 1;
    }
    (modules, wanted)
}

/// A name found in this crate: the module whose file holds it, and whether the item
/// itself was seen (a module, or an item its file declares or re-exports).
#[derive(Clone)]
enum Found {
    At(usize, String, bool),
    Native(usize, String, String),
    Outside,
}
struct Index<'a> {
    modules: HashMap<Vec<String>, usize>,
    inline: HashMap<Vec<String>, usize>,
    items: HashMap<Vec<String>, HashSet<&'a str>>,
    native: HashMap<Vec<String>, HashMap<&'a str, Option<&'a str>>>,
    uses: HashMap<Vec<String>, Vec<&'a rust::Use>>,
    /// Finished searches by module and name: many files ask for the same names.
    found: std::cell::RefCell<HashMap<Lookup, Found>>,
    /// Finished lookups of a name a use line brings in as a module, by module and name.
    imports: std::cell::RefCell<HashMap<Lookup, Option<Vec<String>>>>,
}
/// A name looked up from a module (its path in the crate).
type Lookup = (Vec<String>, String);
impl<'a> Index<'a> {
    fn new(modules: &[Module], facts: &'a HashMap<Vec<u8>, Facts>) -> Self {
        let mut index = Self {
            modules: modules.iter().enumerate().map(|(i, m)| (m.path.clone(), i)).collect(),
            inline: HashMap::new(),
            items: HashMap::new(),
            native: HashMap::new(),
            uses: HashMap::new(),
            found: Default::default(),
            imports: Default::default(),
        };
        let within = |m: &Module, chain: &[String]| [m.path.as_slice(), chain].concat();
        for (i, m) in modules.iter().enumerate() {
            let Some(f) = facts.get(&m.file) else { continue };
            for chain in &f.inline {
                index.inline.entry(within(m, chain)).or_insert(i);
            }
            for (chain, name) in &f.items {
                index.items.entry(within(m, chain)).or_default().insert(name.as_str());
            }
            for (chain, binding, symbol) in &f.native {
                let names = index.native.entry(within(m, chain)).or_default();
                names.entry(binding).and_modify(|entry| {
                    if *entry != Some(symbol.as_str()) { *entry = None; }
                }).or_insert(Some(symbol));
            }
            for u in &f.uses {
                index.uses.entry(within(m, &u.chain)).or_default().push(u);
            }
        }
        index
    }
    fn owner(&self, path: &[String]) -> Option<usize> {
        self.modules.get(path).or_else(|| self.inline.get(path)).copied()
    }
    fn exists(&self, path: &[String]) -> bool {
        self.owner(path).is_some()
    }
    /// Where a path starts: `crate`, `self`, `super`, a module inside the current one, or
    /// a module a `use` line brings in by name or with `*` (`use super::*; details::card()`).
    fn start(&self, segments: &[String], current: &[String]) -> Option<(Vec<String>, usize)> {
        match segments.first()?.as_str() {
            "crate" => Some((Vec::new(), 1)),
            "self" => Some((current.to_vec(), 1)),
            "super" => {
                let mut base = current.to_vec();
                let mut i = 0;
                while segments.get(i).is_some_and(|s| s == "super") {
                    base.pop()?;
                    i += 1;
                }
                Some((base, i))
            }
            first => {
                let mut child = current.to_vec();
                child.push(first.to_owned());
                if self.exists(&child) {
                    return Some((current.to_vec(), 0));
                }
                // Every path whose first name is not a module here asks this (`Vec::new`
                // too), at any depth: each name is looked up once per module. A lookup
                // still running reads as "not a module", which ends loops between globs.
                let key = (current.to_vec(), first.to_owned());
                if let Some(found) = self.imports.borrow().get(&key) {
                    return found.clone().map(|module| (module, 1));
                }
                self.imports.borrow_mut().insert(key.clone(), None);
                let found = self.imported(first, current, 0, &mut HashSet::new());
                self.imports.borrow_mut().insert(key, found.clone());
                found.map(|module| (module, 1))
            }
        }
    }
    /// The module `name` stands for in `module` through its `use` lines, if it is one.
    fn imported(&self, name: &str, module: &[String], depth: usize, seen: &mut HashSet<Vec<String>>) -> Option<Vec<String>> {
        if depth > 6 || !seen.insert(module.to_vec()) {
            return None;
        }
        let uses = self.uses.get(module).map(Vec::as_slice).unwrap_or_default();
        for u in uses.iter().filter(|u| !u.glob) {
            if u.alias.as_deref().or(u.segments.last().map(String::as_str)) != Some(name) {
                continue;
            }
            let (found, used) = self.module(&u.segments, module)?;
            let whole = used == u.segments.len() || used + 1 == u.segments.len() && u.segments[used] == "self";
            return whole.then_some(found);
        }
        for u in uses.iter().filter(|u| u.glob) {
            let Some((glob, used)) = self.module(&u.segments, module) else { continue };
            if used < u.segments.len() || glob == module {
                continue;
            }
            let mut child = glob.clone();
            child.push(name.to_owned());
            if self.exists(&child) {
                return Some(child);
            }
            if let Some(found) = self.imported(name, &glob, depth + 1, seen) {
                return Some(found);
            }
        }
        None
    }
    /// The module a path names, following as many module segments as it can.
    fn module(&self, segments: &[String], current: &[String]) -> Option<(Vec<String>, usize)> {
        let (mut base, mut i) = self.start(segments, current)?;
        while let Some(next) = segments.get(i) {
            let mut child = base.clone();
            child.push(next.clone());
            if !self.exists(&child) {
                break;
            }
            base = child;
            i += 1;
        }
        Some((base, i))
    }
    fn resolve(&self, segments: &[String], current: &[String], depth: usize) -> Option<Found> {
        let (base, i) = self.module(segments, current)?;
        match segments.get(i).map(String::as_str) {
            None | Some("self") => {
                let name = base.last().cloned().unwrap_or_else(|| "crate".into());
                Some(Found::At(self.owner(&base)?, name, true))
            }
            Some(name) => Some(self.find(&base, name, depth)),
        }
    }
    fn find(&self, module: &[String], name: &str, depth: usize) -> Found {
        // Only a search from the start is kept: one deeper in a chain of re-exports
        // can stop at the depth limit short of what a fresh search would find.
        if depth > 0 {
            return self.search(module, name, depth, &mut HashSet::new());
        }
        let key = (module.to_vec(), name.to_owned());
        if let Some(found) = self.found.borrow().get(&key) {
            return found.clone();
        }
        let found = self.search(module, name, 0, &mut HashSet::new());
        self.found.borrow_mut().insert(key, found.clone());
        found
    }
    /// One search for `name` from `module`. Each module is looked in once: `use …::*`
    /// lines can lead back to a module already searched.
    fn search(&self, module: &[String], name: &str, depth: usize, seen: &mut HashSet<Vec<String>>) -> Found {
        let owner = self.owner(module).unwrap_or(0);
        if self.items.get(module).is_some_and(|names| names.contains(name)) {
            return Found::At(owner, name.into(), true);
        }
        if let Some(symbol) = self.native.get(module).and_then(|names| names.get(name)) {
            return match symbol {
                Some(symbol) => Found::Native(owner, name.into(), (*symbol).into()),
                None => Found::At(owner, name.into(), true),
            };
        }
        if depth > 6 || !seen.insert(module.to_vec()) {
            return Found::At(owner, name.into(), false);
        }
        let uses = self.uses.get(module).map(Vec::as_slice).unwrap_or_default();
        for u in uses.iter().filter(|u| !u.glob) {
            let shown = u.alias.as_deref().or(u.segments.last().map(String::as_str));
            if shown == Some(name) {
                return match self.resolve(&u.segments, module, depth + 1) {
                    Some(found) => found,
                    None => Found::Outside,
                };
            }
        }
        for u in uses.iter().filter(|u| u.glob) {
            let Some((glob, used)) = self.module(&u.segments, module) else { continue };
            if used < u.segments.len() || glob == module {
                continue;
            }
            match self.search(&glob, name, depth + 1, seen) {
                found @ (Found::At(_, _, true) | Found::Native(..) | Found::Outside) => return found,
                Found::At(..) => {}
            }
        }
        Found::At(owner, name.into(), false)
    }
}

/// Which module's file uses which, with the names it uses, and how many paths into
/// the crate were followed and how many of those ended at a module but no known item;
/// also the other crates each module's `use` lines name.
pub(super) struct Links {
    pub uses: BTreeMap<(usize, usize), BTreeSet<String>>,
    pub native: BTreeSet<(usize, String)>,
    pub followed: usize,
    pub unknown: usize,
    pub outside: BTreeMap<usize, BTreeSet<String>>,
}
pub(super) fn links(
    modules: &[Module], facts: &HashMap<Vec<u8>, Facts>, known: &dyn Fn(&[u8]) -> bool, cancel: &CancellationToken,
    mut reference: impl FnMut(usize, usize, u32, &'static str),
) -> Result<Links> {
    let index = Index::new(modules, facts);
    let by_file: HashMap<&[u8], usize> = modules.iter().enumerate().map(|(i, m)| (m.file.as_slice(), i)).collect();
    let mut out = Links {
        uses: BTreeMap::new(),
        native: BTreeSet::new(),
        followed: 0,
        unknown: 0,
        outside: BTreeMap::new(),
    };
    let add = |from: usize, found: Found, out: &mut Links| {
        let (to, name, known) = match found {
            Found::At(to, name, known) => (to, name, known),
            Found::Native(to, binding, symbol) => {
                out.native.insert((from, symbol));
                (to, binding, true)
            }
            Found::Outside => return None,
        };
        out.followed += 1;
        if !known {
            out.unknown += 1;
        }
        if from != to {
            out.uses.entry((from, to)).or_default().insert(name);
        }
        (known && from != to).then_some(to)
    };
    for (m, module) in modules.iter().enumerate() {
        cancel.check()?;
        let Some(f) = facts.get(&module.file) else { continue };
        let within = |chain: &[String]| [module.path.as_slice(), chain].concat();
        for d in &f.declarations {
            cancel.check()?;
            let candidates = module.declaration_files(d);
            // Check the snapshot, including candidates omitted by the source read limits.
            let mut present = candidates.iter().filter(|(path, _)| known(path));
            if let Some((path, _)) = present.next() && present.next().is_none()
                && let Some(&to) = by_file.get(path.as_slice()) && m != to
            {
                reference(m, to, d.line, "mod");
            }
        }
        for u in f.uses.iter().filter(|u| !u.glob) {
            if let Some(found) = index.resolve(&u.segments, &within(&u.chain), 0)
                && let Some(to) = add(m, found, &mut out)
            {
                reference(m, to, u.line, u.kind);
            }
        }
        // `use std::…`, `use egui::…`: a first name that is no module or item here is a crate.
        for u in &f.uses {
            let current = within(&u.chain);
            let Some(first) = u.segments.first() else { continue };
            let krate = if first.is_empty() {
                // `use ::serde::X` names the crate after the leading `::`.
                match u.segments.get(1).filter(|k| !k.is_empty()) {
                    Some(k) => k,
                    None => continue,
                }
            } else {
                // Not found here at all: a module, an item, or a name another use line
                // brings in (`use std::fmt; use fmt::Write;`) is no crate.
                if ["crate", "self", "super"].contains(&first.as_str()) || index.start(&u.segments, &current).is_some() || !matches!(index.find(&current, first, 0), Found::At(_, _, false)) {
                    continue;
                }
                first
            };
            let names = out.outside.entry(m).or_default();
            if names.len() < 256 {
                names.insert(krate.clone());
            }
        }
        for p in &f.paths {
            if let Some(found) = index.resolve(&p.segments, &within(&p.chain), 0) {
                add(m, found, &mut out);
            }
        }
        // Names a `use …::*` brings in, matched by name. A name the module declares
        // itself is its own.
        for u in f.uses.iter().filter(|u| u.glob) {
            let current = within(&u.chain);
            let Some((glob, used)) = index.module(&u.segments, &current) else { continue };
            if used < u.segments.len() || glob == current {
                continue;
            }
            if let Some(to) = index.owner(&glob) && m != to {
                reference(m, to, u.line, u.kind);
            }
            let mut seen = HashSet::new();
            for (chain, name) in f.names.iter().filter(|(chain, _)| chain.starts_with(&u.chain)) {
                if !seen.insert(name.as_str()) || index.items.get(&within(chain)).is_some_and(|own| own.contains(name.as_str())) {
                    continue;
                }
                if let found @ (Found::At(_, _, true) | Found::Native(..)) = index.find(&glob, name, 0) {
                    add(m, found, &mut out);
                }
            }
        }
    }
    Ok(out)
}

/// What the module read has taken in so far.
#[derive(Default)]
struct Sources {
    facts: HashMap<Vec<u8>, Facts>,
    budget: rust::ReadBudget,
    lines: HashMap<Vec<u8>, usize>,
    unlinked: BTreeMap<Vec<u8>, UnlinkedReason>,
    total: usize,
}

/// Without a choice: the largest package of the language with the most files, Rust
/// first when two languages have as many.
fn default_package(packages: &[ModulePackage]) -> Option<usize> {
    let mut totals: BTreeMap<ModuleLanguage, usize> = BTreeMap::new();
    for p in packages {
        *totals.entry(p.language).or_default() += p.files;
    }
    // A repository with a Rust package opens on Rust, as it did before the other languages
    // were read: its JavaScript may be fixtures or a website.
    let language = if totals.get(&ModuleLanguage::Rust).is_some_and(|n| *n > 0) {
        ModuleLanguage::Rust
    } else {
        totals.iter().max_by_key(|(l, n)| (**n, std::cmp::Reverse(**l))).map(|(l, _)| *l)?
    };
    (0..packages.len()).filter(|&i| packages[i].language == language).max_by_key(|&i| (packages[i].files, std::cmp::Reverse(i)))
}

/// Read limits shared by every language's files: the same as Rust's.
fn readable(size: usize, read: usize, total: usize) -> std::result::Result<(), UnlinkedReason> {
    if size > FILE_BYTES {
        Err(UnlinkedReason::TooLarge)
    } else if read >= FILES || total + size > TOTAL_BYTES {
        Err(UnlinkedReason::Limit)
    } else {
        Ok(())
    }
}

/// The package's programs other than the library: `main.rs` beside `lib.rs` and the
/// files in `bin`.
fn programs(src: &[u8], files: &BTreeSet<Vec<u8>>, root: &[u8]) -> Vec<Vec<u8>> {
    let bin = join(src, b"bin");
    files
        .iter()
        .filter(|f| f.as_slice() != root)
        .filter(|f| {
            **f == join(src, b"main.rs")
                || directory(f) == bin.as_slice()
                || directory(directory(f)) == bin.as_slice() && f.ends_with(b"/main.rs")
        })
        .cloned()
        .collect()
}

impl Repository {
    pub(super) fn map_modules(&self, map: &mut RepositoryMap, cancel: &CancellationToken) -> Result<()> {
        let op = "map-modules";
        let tip = map.tip.as_ref().ok_or_else(malformed)?.clone();
        self.prepare_tree(tip.as_str(), cancel)?;
        let output = self.runner.step(op, "list-tree", || {
            self.map_read(&args(&["ls-tree", "-r", "-l", "-z", "--full-tree", tip.as_str()]), Vec::new(), cancel)
        })?;
        let mut entries: Vec<(&[u8], &[u8], ObjectId, usize)> = Vec::new();
        for raw in output.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            cancel.check()?;
            map.scanned += 1;
            let (header, path) = raw.split_once_byte(b'\t').ok_or_else(malformed)?;
            let fields = header.split(|b| b.is_ascii_whitespace()).filter(|f| !f.is_empty()).collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(malformed());
            }
            // Symbolic links and submodules are references, not files to read.
            if fields[0] == b"120000" || fields[1] != b"blob" {
                continue;
            }
            let size = std::str::from_utf8(fields[3]).ok().and_then(|v| v.parse::<usize>().ok()).ok_or_else(malformed)?;
            entries.push((path, fields[0], parse::oid(fields[2], op)?, size));
        }
        // Cargo packages, within the same limits as Packages.
        let mut manifests = Vec::new();
        let mut budget = 0usize;
        for (path, _, id, size) in &entries {
            if path.rsplit(|b| *b == b'/').next() == Some(b"Cargo.toml") {
                if *size > 128 * 1024 || manifests.len() >= 64 || budget + size > 4 * 1024 * 1024 {
                    map.limited = true;
                    continue;
                }
                budget += size;
                manifests.push((path.to_vec(), id.clone(), *size));
            }
        }
        let mut packages: Vec<(Vec<u8>, String)> = Vec::new();
        if !manifests.is_empty() {
            let bytes = self.runner.step(op, "read-manifests", || {
                let input = manifests.iter().flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n'))).collect();
                self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)
            })?;
            for ((path, _, _), blob) in manifests.iter().zip(parse_blobs(&bytes, &manifests, cancel)?) {
                if let Ok(text) = std::str::from_utf8(blob)
                    && let (Some(name), _) = cargo::manifest(text)
                {
                    packages.push((directory(path).to_vec(), name));
                }
            }
        }
        let rust: BTreeMap<&[u8], (ObjectId, usize)> = entries
            .iter()
            .filter(|(path, ..)| path.ends_with(b".rs"))
            .map(|(path, _, id, size)| (*path, (id.clone(), *size)))
            .collect();
        let src_of = |folder: &[u8]| join(folder, b"src");
        let inside = |path: &[u8], folder: &[u8]| folder.is_empty() || path.len() > folder.len() && path.starts_with(folder) && path[folder.len()] == b'/';
        packages.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        for (folder, name) in &packages {
            let mut n = node(MapNodeKind::Crate, [b"package:".as_slice(), folder].concat(), name.clone().into_bytes(), b"Cargo package".to_vec());
            n.path = Some(folder.clone());
            n.files = entries.iter().filter(|(path, ..)| inside(path, folder)).count();
            let src = src_of(folder);
            n.count = rust.keys().filter(|path| inside(path, &src)).count();
            let at = map.nodes.len();
            map.nodes.push(n);
            map.module_packages.push(ModulePackage {
                folder: folder.clone(),
                name: name.clone().into_bytes(),
                language: ModuleLanguage::Rust,
                dialect: None,
                manifest: true,
                files: map.nodes[at].count,
                node: Some(at),
            });
        }
        // The other languages' packages, found by their paths.
        let blobs: BTreeMap<&[u8], (ObjectId, usize)> = entries.iter().map(|(path, _, id, size)| (*path, (id.clone(), *size))).collect();
        let paths: Vec<_> = blobs.keys().copied().collect();
        let (mut others, limited) = packages::find(&paths);
        let lua_manifests = packages::lua_manifests(&blobs);
        // The manifests that name a package or say how it is imported.
        let wanted: Vec<Vec<u8>> = others
            .iter()
            .filter_map(|f| f.manifest.clone())
            .filter(|m| matches!(packages::file_name(m), b"package.json" | b"go.mod") || packages::names_project(m))
            .chain(lua_manifests.iter().cloned())
            .collect();
        let small = if wanted.is_empty() { BTreeMap::new() } else { self.runner.step(op, "read-manifests", || self.map_small(&wanted, &blobs, cancel))? };
        let (lua, lua_limited) = packages::lua(&paths, &lua_manifests, &small, packages::PACKAGES);
        others.extend(lua);
        // A cut list of packages leaves the package read whole: a note, not a partial map.
        if limited || lua_limited {
            map.notes.push(format!("More than {} packages of one language: the first {} by folder are listed.", packages::PACKAGES, packages::PACKAGES));
        }
        for f in &others {
            let text = f.manifest.as_ref().and_then(|m| small.get(m));
            let own = f.group_name();
            let built = f.manifest.is_some();
            let named = || f.manifest.as_ref().and_then(|m| packages::project_name(m, text.map(String::as_str))).map(String::into_bytes).unwrap_or_else(|| own.clone());
            let (prefix, name, detail): (&[u8], Vec<u8>, &[u8]) = match f.language {
                ModuleLanguage::JavaScript => (
                    b"package-javascript:",
                    text.and_then(|t| packages::json(t)).and_then(|j| j.get("name").and_then(packages::Json::str).map(|n| n.as_bytes().to_vec())).filter(|n| !n.is_empty()).unwrap_or(own),
                    match (f.dialect.is_some(), built) {
                        (true, true) => b"TypeScript project",
                        (true, false) => b"TypeScript folder",
                        (false, true) => b"JavaScript project",
                        (false, false) => b"JavaScript folder",
                    },
                ),
                ModuleLanguage::Python => (b"package-python:", own, if built { b"Python package" } else { b"Python folder" }),
                ModuleLanguage::Java => (
                    b"package-java:",
                    named(),
                    match (f.dialect.is_some(), built) {
                        (true, true) => b"Kotlin project",
                        (true, false) => b"Kotlin folder",
                        (false, true) => b"Java project",
                        (false, false) => b"Java folder",
                    },
                ),
                ModuleLanguage::C => (
                    b"package-c:",
                    named(),
                    match (f.dialect.is_some(), built) {
                        (true, true) => b"C++ project",
                        (true, false) => b"C++ folder",
                        (false, true) => b"C project",
                        (false, false) => b"C folder",
                    },
                ),
                ModuleLanguage::CSharp => (b"package-csharp:", named(), if built { b"C# project" } else { b"C# folder" }),
                ModuleLanguage::PowerShell => (b"package-powershell:", own, b"PowerShell scripts"),
                ModuleLanguage::Bash => (b"package-bash:", own, b"Bash scripts"),
                ModuleLanguage::Lua => (b"package-lua:", own, b"Lua scripts"),
                ModuleLanguage::GitHubActions => (b"package-github-actions:", own, b"GitHub Actions workflows"),
                ModuleLanguage::HtmlCss => (b"package-html-css:", own, b"HTML / CSS"),
                ModuleLanguage::Go | ModuleLanguage::Rust => (b"package-go:", text.and_then(|t| packages::go_module(t)).map(String::into_bytes).unwrap_or(own), b"Go module"),
            };
            let mut n = node(MapNodeKind::Crate, [prefix, f.folder.as_slice()].concat(), name.clone(), detail.to_vec());
            n.path = Some(f.folder.clone());
            let source_count = f.source_count();
            n.files = if f.language == ModuleLanguage::HtmlCss { source_count } else { entries.iter().filter(|(path, ..)| inside(path, &f.folder)).count() };
            n.count = source_count;
            let at = map.nodes.len();
            map.nodes.push(n);
            map.module_packages.push(ModulePackage {
                folder: f.folder.clone(),
                name,
                language: f.language,
                dialect: f.dialect,
                manifest: f.manifest.is_some(),
                files: source_count,
                node: Some(at),
            });
        }
        let chosen = match &map.scope.package {
            Some(wanted) => Some(map.module_packages.iter().position(|p| p.folder == wanted.folder && p.language == wanted.language).ok_or_else(|| {
                Error::InvalidInput("The chosen package is not in this snapshot. Choose another package.".into())
            })?),
            None => default_package(&map.module_packages),
        };
        let Some(chosen) = chosen else { return Ok(()) };
        map.scope.package = Some(map.module_packages[chosen].reference());
        if map.module_packages[chosen].language != ModuleLanguage::Rust {
            let found = &others[chosen - packages.len()];
            let package = map.module_packages[chosen].node.unwrap_or_default();
            // A Go module's path, as its imports start; empty when go.mod names none.
            let module = (found.language == ModuleLanguage::Go).then(|| found.manifest.as_ref().and_then(|m| small.get(m)).and_then(|t| packages::go_module(t)).unwrap_or_default());
            return self.map_other(map, package, found, module, &blobs, cancel);
        }
        map.notes.push("Read from the committed Rust files of one package. Connections follow use lines, paths into the package and literal files embedded by include_str! or include_bytes!. Names from use …::* are matched by name. Macro expansion and cfg evaluation are outside this analysis.".into());
        let package = chosen;
        let src = src_of(&packages[package].0);
        let mut details = ModuleMap {
            package,
            ..Default::default()
        };
        let sources: BTreeSet<Vec<u8>> = rust.keys().filter(|p| inside(p, &src)).map(|p| p.to_vec()).collect();
        let root = [join(&src, b"lib.rs"), join(&src, b"main.rs")].into_iter().find(|p| sources.contains(p));
        // What to read first: the package's own files, shallow ones first.
        let mut first: Vec<Vec<u8>> = sources.iter().cloned().collect();
        first.sort_by_key(|p| (p.iter().filter(|b| **b == b'/').count(), p.clone()));
        let mut s = Sources::default();
        let read = |paths: Vec<Vec<u8>>, s: &mut Sources, map: &mut RepositoryMap| -> Result<()> {
            let mut batch = Vec::new();
            for path in paths {
                let Some((id, size)) = rust.get(path.as_slice()) else { continue };
                if *size > FILE_BYTES {
                    s.unlinked.insert(path, UnlinkedReason::TooLarge);
                    continue;
                }
                if s.facts.len() + batch.len() >= FILES || s.total + size > TOTAL_BYTES {
                    map.limited = true;
                    s.unlinked.insert(path, UnlinkedReason::Limit);
                    continue;
                }
                s.total += size;
                batch.push((path, id.clone(), *size));
            }
            if batch.is_empty() {
                return Ok(());
            }
            let input = batch.iter().flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n'))).collect();
            let bytes = self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)?;
            for ((path, _, _), blob) in batch.iter().zip(parse_blobs(&bytes, &batch, cancel)?) {
                cancel.check()?;
                match std::str::from_utf8(blob) {
                    Ok(text) => {
                        s.lines.insert(path.clone(), text.lines().count().max(1));
                        let facts = rust::read_with_budget(text, &mut s.budget);
                        map.limited |= facts.limited;
                        s.facts.insert(path.clone(), facts);
                    }
                    Err(_) => {
                        s.unlinked.insert(path.clone(), UnlinkedReason::NotUtf8);
                    }
                }
            }
            Ok(())
        };
        self.runner.step(op, "read-sources", || read(first, &mut s, map))?;
        let Some(root) = root else {
            map.notes.push("This package has no src/lib.rs or src/main.rs, so its modules are not mapped.".into());
            details.unlinked = sources.iter().map(|p| (p.clone(), UnlinkedReason::NotDeclared)).collect();
            map.modules = Some(details);
            return Ok(());
        };
        let known = |p: &[u8]| rust.contains_key(p);
        let programs = programs(&src, &sources, &root);
        // `#[path]` can name a file outside src; read those as their declarations ask.
        let (mut modules, mut wanted) = tree(&root, &s.facts, &known);
        for _ in 0..EXTRA_ROUNDS {
            for program in &programs {
                wanted.extend(tree(program, &s.facts, &known).1);
            }
            wanted.retain(|p| !s.facts.contains_key(p) && !s.unlinked.contains_key(p));
            if wanted.is_empty() {
                break;
            }
            let next: Vec<Vec<u8>> = std::mem::take(&mut wanted).into_iter().collect();
            self.runner.step(op, "read-sources", || read(next, &mut s, map))?;
            (modules, wanted) = tree(&root, &s.facts, &known);
        }
        cancel.check()?;
        // Files no declaration of the library reaches: other programs, and the rest.
        let reached: HashSet<&[u8]> = modules.iter().map(|m| m.file.as_slice()).collect();
        let mut by_program: HashSet<Vec<u8>> = HashSet::new();
        for program in &programs {
            by_program.extend(tree(program, &s.facts, &known).0.into_iter().map(|m| m.file));
        }
        for path in &sources {
            if reached.contains(path.as_slice()) || s.unlinked.contains_key(path) {
                continue;
            }
            let reason = if by_program.contains(path) { UnlinkedReason::Program } else { UnlinkedReason::NotDeclared };
            s.unlinked.insert(path.clone(), reason);
        }
        details.unlinked = std::mem::take(&mut s.unlinked).into_iter().collect();
        let (facts, lines) = (s.facts, s.lines);
        let base = map.nodes.len();
        for m in &modules {
            if map.nodes.len() >= MAP_NODES {
                map.limited = true;
                break;
            }
            let key = [b"module:".as_slice(), m.path.join("::").as_bytes()].concat();
            let label = match m.path.last() {
                Some(name) => name.clone().into_bytes(),
                None => m.file.rsplit(|b| *b == b'/').next().unwrap_or_default().to_vec(),
            };
            let detail: &[u8] = if m.path.is_empty() {
                b"Crate root"
            } else if m.test {
                b"Test module"
            } else {
                b"Rust module"
            };
            let mut n = node(MapNodeKind::Module, key, label, detail.to_vec());
            n.path = Some(m.file.clone());
            n.count = lines.get(&m.file).copied().unwrap_or_default();
            n.files = 1;
            let at = map.nodes.len();
            map.nodes.push(n);
            if let Some(outline) = facts.get(&m.file).map(|f| f.outline.iter().take(OUTLINE).cloned().collect::<Vec<_>>()) {
                details.outline.insert(at, outline);
            }
            if let Some(parent) = m.parent {
                map.links.push(link(base + parent, at, MapLinkKind::Contains));
            }
        }
        let shown = map.nodes.len() - base;
        let found = self.runner.step(op, "connect-modules", || links(&modules, &facts, &known, cancel, |from, to, line, kind| {
            if from < shown && to < shown {
                details.references.add(&modules[from].file, &modules[to].file, line, kind);
            }
        }))?;
        let mut uses = found.uses;
        let mut sites = BTreeMap::<_, Vec<_>>::new();
        uses.retain(|&(from, to), _| from < shown && to < shown);
        let mut by_file: HashMap<Vec<u8>, usize> = modules.iter().take(shown).enumerate().map(|(i, m)| (m.file.clone(), i)).collect();
        for (from, m) in modules.iter().take(shown).enumerate() {
            let Some(source) = facts.get(&m.file) else { continue };
            for embedded in source.embedded_files(&m.file, &blobs, cancel) {
                let (kind, path, line) = embedded?;
                let existing = by_file.get(&path).copied();
                if existing == Some(from) { continue; }
                if map.links.len() + uses.len() >= MAP_LINKS && !existing.is_some_and(|to| uses.contains_key(&(from, to))) {
                    map.limited = true;
                    continue;
                }
                let to = if let Some(to) = existing { to } else {
                    if map.nodes.len() >= MAP_NODES { map.limited = true; continue; }
                    let mut asset = node(MapNodeKind::Module, [b"asset:".as_slice(), &path].concat(), path.clone(), b"Referenced file".to_vec());
                    asset.path = Some(path.clone());
                    asset.files = 1;
                    let to = map.nodes.len() - base;
                    map.nodes.push(asset);
                    by_file.insert(path, to);
                    to
                };
                sites.entry((from, to)).or_default().push((line, kind));
                uses.entry((from, to)).or_default().insert(kind.to_owned());
                details.followed += 1;
            }
        }
        for (m, names) in found.outside {
            if m < shown {
                details.outside.insert(base + m, names.into_iter().map(String::into_bytes).collect());
            }
        }
        for ((from, to), names) in uses {
            if map.links.len() >= MAP_LINKS { map.limited = true; break; }
            if let Some(sites) = sites.remove(&(from, to)) {
                let target = map.nodes[base + to].path.as_deref().unwrap();
                for (line, kind) in sites { details.references.add(&modules[from].file, target, line, kind); }
            }
            let mut e = link(base + from, base + to, MapLinkKind::DependsOn);
            e.weight = names.len();
            e.evidence = names.into_iter().take(NAMES).map(String::into_bytes).collect();
            map.links.push(e);
        }
        map.limited |= details.references.limited;
        details.root = (shown > 0).then_some(base);
        details.followed += found.followed;
        details.unknown = found.unknown;
        map.modules = Some(details);
        Ok(())
    }

    /// Small text files by path (package.json, go.mod, tsconfig.json): at most 512 of
    /// 128 KiB, 4 MiB in all; one that is larger, missing or not UTF-8 is left out.
    fn map_small(&self, paths: &[Vec<u8>], blobs: &BTreeMap<&[u8], (ObjectId, usize)>, cancel: &CancellationToken) -> Result<BTreeMap<Vec<u8>, String>> {
        let mut batch = Vec::new();
        let mut total = 0;
        for path in paths.iter().take(512) {
            let Some((id, size)) = blobs.get(path.as_slice()) else { continue };
            if *size > 128 * 1024 || total + size > 4 * 1024 * 1024 {
                continue;
            }
            total += size;
            batch.push((path.clone(), id.clone(), *size));
        }
        let mut out = BTreeMap::new();
        if batch.is_empty() {
            return Ok(out);
        }
        let input = batch.iter().flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n'))).collect();
        let bytes = self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)?;
        for ((path, _, _), blob) in batch.iter().zip(parse_blobs(&bytes, &batch, cancel)?) {
            if let Ok(text) = std::str::from_utf8(blob) {
                out.insert(path.clone(), text.to_owned());
            }
        }
        Ok(out)
    }

    /// One JavaScript, Python or Go package: its files read as text within Rust's limits,
    /// then its units, connections, outline and outside imports.
    fn map_other(&self, map: &mut RepositoryMap, package: usize, found: &packages::Found, module: Option<String>, blobs: &BTreeMap<&[u8], (ObjectId, usize)>, cancel: &CancellationToken) -> Result<()> {
        let op = "map-modules";
        let mut details = ModuleMap {
            package,
            language: found.language,
            ..Default::default()
        };
        // What to read: shallow files first, so a limit leaves out the deepest ones.
        let mut order = found.files.clone();
        order.sort_by_key(|p| (p.iter().filter(|b| **b == b'/').count(), p.clone()));
        let mut batch = Vec::new();
        let mut total = 0;
        for path in order {
            let Some((id, size)) = blobs.get(path.as_slice()) else { continue };
            match readable(*size, batch.len(), total) {
                Ok(()) => {
                    total += size;
                    batch.push((path, id.clone(), *size));
                }
                Err(reason) => {
                    map.limited |= reason == UnlinkedReason::Limit;
                    details.unlinked.push((path, reason));
                }
            }
        }
        let mut texts: BTreeMap<Vec<u8>, String> = BTreeMap::new();
        if !batch.is_empty() {
            let bytes = self.runner.step(op, "read-sources", || {
                let input = batch.iter().flat_map(|(_, id, _)| id.as_str().bytes().chain(Some(b'\n'))).collect();
                self.map_read(&args(&["cat-file", "--batch", "--buffer"]), input, cancel)
            })?;
            for ((path, _, _), blob) in batch.iter().zip(parse_blobs(&bytes, &batch, cancel)?) {
                cancel.check()?;
                match source_text(path, blob) {
                    Some(text) => {
                        texts.insert(path.clone(), text);
                    }
                    None if is_binary_source(path, blob) => details.unlinked.push((path.clone(), UnlinkedReason::Binary)),
                    None => details.unlinked.push((path.clone(), UnlinkedReason::NotUtf8)),
                }
            }
        }
        let built = self.runner.step(op, "connect-modules", || match found.language {
            ModuleLanguage::JavaScript => {
                // tsconfig.json's paths, else jsconfig.json's: where `@/view` looks.
                let configs: Vec<Vec<u8>> = [b"tsconfig.json".as_slice(), b"jsconfig.json"].iter().map(|n| packages::join(&found.folder, n)).collect();
                let small = self.map_small(&configs, blobs, cancel)?;
                let aliases = configs.iter().find_map(|c| small.get(c)).map(|text| packages::aliases(text, &found.folder)).unwrap_or_default();
                let known: HashSet<&[u8]> = blobs.keys().copied().collect();
                units::javascript(&found.folder, &texts, &known, &aliases, cancel)
            }
            ModuleLanguage::Python => units::python(&found.folder, found.manifest.is_some(), &texts, cancel),
            ModuleLanguage::Java => units::java(&texts, &found.files, cancel),
            ModuleLanguage::C => {
                let known: HashSet<&[u8]> = blobs.keys().copied().collect();
                let headers = units::Headers::new(blobs.keys().copied());
                units::c(&found.folder, &texts, &found.files, &known, &headers, cancel)
            }
            ModuleLanguage::CSharp => units::csharp(&texts, cancel),
            ModuleLanguage::Lua => units::lua(found, &texts, cancel),
            ModuleLanguage::GitHubActions | ModuleLanguage::HtmlCss => units::file_dependencies(found, &texts, &blobs.keys().copied().collect(), cancel),
            ModuleLanguage::PowerShell | ModuleLanguage::Bash => {
                let known = blobs.keys().copied().collect();
                units::scripts(found.language, &found.folder, &texts, &known, cancel)
            }
            ModuleLanguage::Go | ModuleLanguage::Rust => units::go(&found.folder, module.as_deref().unwrap_or_default(), &texts, &found.files, cancel),
        })?;
        map.notes.push(
            match found.language {
                ModuleLanguage::JavaScript => "Read from the committed JavaScript and TypeScript files of one project. Connections come from import, export … from, require() and import() with a fixed path, and tsconfig paths; other imports are listed as outside. Nothing is run, so paths built while a program runs are not followed.",
                ModuleLanguage::Python => "Read from the committed Python files of one package. Connections come from import and from … import lines, also inside functions; other imports are listed as outside. Nothing is run, so importlib and sys.path changes are not followed.",
                ModuleLanguage::Java => "Read from the committed Java and Kotlin files of one project. Each package stands alone; connections come from imports, the names a file uses through a * import, and names written out with their package. Test source sets (src/test and the like) are left out. Nothing is compiled, so names reached through inheritance are not followed.",
                ModuleLanguage::C => "Read from the committed C and C++ files of one project. Connections come from #include lines, found beside the including file, else by the end of a path in the project or the snapshot; the names on a connection are those the included file declares and the other uses. The preprocessor is not run: macros are not expanded, and only the first branch of an #if is read for declarations.",
                ModuleLanguage::CSharp => "Read from the committed C# files of one project. Each namespace stands alone; connections come from using lines, the type names a file uses from its namespace's parents and the namespaces it uses, and names written out in full. Nothing is compiled, so extension methods are not told apart from other methods.",
                ModuleLanguage::PowerShell => "Arrows show which committed scripts dot-source a helper, invoke it with &, or import a module by path. Runtime expressions and ambiguous working-directory paths are skipped.",
                ModuleLanguage::Bash => "Arrows show which committed scripts load a helper with source or dot. Runtime expressions, PATH lookups and ambiguous working-directory paths are skipped.",
                ModuleLanguage::Lua => "Rocket integration manifests point to main.lua. Ordinary Lua scripts link to unique committed modules named in literal require calls.",
                ModuleLanguage::GitHubActions => "Workflow arrows point to committed scripts, local action manifests and reusable workflows. Script paths use the working directory for that step.",
                ModuleLanguage::HtmlCss => "Arrows connect pages and stylesheets to committed assets. Select an asset to see which pages and stylesheets use it.",
                ModuleLanguage::Go | ModuleLanguage::Rust => "Read from the committed Go files of one module. Each package stands alone; connections come from imports and the names used through them. Test files and files marked //go:build ignore are left out; other build constraints are not evaluated.",
            }
            .into(),
        );
        units::push(map, &mut details, built);
        details.unlinked.sort_by(|a, b| a.0.cmp(&b.0));
        map.modules = Some(details);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(files: &[(&str, &str)]) -> (Vec<Module>, HashMap<Vec<u8>, Facts>) {
        let facts: HashMap<Vec<u8>, Facts> = files.iter().map(|(p, s)| (p.as_bytes().to_vec(), rust::read(s))).collect();
        let (modules, wanted) = tree(b"src/lib.rs", &facts, &|_| false);
        assert!(wanted.is_empty());
        (modules, facts)
    }
    fn named(modules: &[Module], links: &Links) -> Vec<(String, String, Vec<String>)> {
        let name = |i: usize| if modules[i].path.is_empty() { "crate".to_owned() } else { modules[i].path.join("::") };
        links
            .uses
            .iter()
            .map(|((a, b), names)| (name(*a), name(*b), names.iter().cloned().collect()))
            .collect()
    }

    fn connections(modules: &[Module], facts: &HashMap<Vec<u8>, Facts>) -> Links {
        links(modules, facts, &|p| facts.contains_key(p), &CancellationToken::new(), |_, _, _, _| {}).unwrap()
    }

    #[test]
    fn files_follow_rusts_module_rules() {
        let (modules, _) = build(&[
            ("src/lib.rs", "mod app;\nmod style;\n#[cfg(test)]\nmod tests;\nmod missing;\nmod inner { mod deep; }\n"),
            ("src/app.rs", "mod details;\n#[path = \"shared/odd.rs\"] mod odd;\n"),
            ("src/app/details.rs", ""),
            ("src/shared/odd.rs", "mod beside;\n"),
            ("src/shared/beside.rs", ""),
            ("src/style/mod.rs", "mod tokens;\n"),
            ("src/style/tokens.rs", ""),
            ("src/tests.rs", ""),
            ("src/inner/deep.rs", ""),
            ("src/stray.rs", ""),
        ]);
        let found: Vec<(String, String, bool)> = modules
            .iter()
            .map(|m| (m.path.join("::"), String::from_utf8_lossy(&m.file).into_owned(), m.test))
            .collect();
        assert_eq!(
            found,
            [
                ("".into(), "src/lib.rs".into(), false),
                ("app".into(), "src/app.rs".into(), false),
                ("style".into(), "src/style/mod.rs".into(), false),
                ("tests".into(), "src/tests.rs".into(), true),
                ("inner::deep".into(), "src/inner/deep.rs".into(), false),
                ("app::details".into(), "src/app/details.rs".into(), false),
                ("app::odd".into(), "src/shared/odd.rs".into(), false),
                ("style::tokens".into(), "src/style/tokens.rs".into(), false),
                ("app::odd::beside".into(), "src/shared/beside.rs".into(), false),
            ]
        );
    }

    #[test]
    fn a_path_outside_the_package_is_asked_for_and_an_escape_is_refused() {
        let facts: HashMap<Vec<u8>, Facts> = [("crates/a/src/lib.rs", "#[path = \"../../b/src/shared.rs\"] mod shared;\n#[path = \"../../../../outside.rs\"] mod escape;\n")]
            .iter()
            .map(|(p, s)| (p.as_bytes().to_vec(), rust::read(s)))
            .collect();
        let (modules, wanted) = tree(b"crates/a/src/lib.rs", &facts, &|p| p == b"crates/b/src/shared.rs");
        assert_eq!(modules.len(), 1);
        assert_eq!(wanted.into_iter().collect::<Vec<_>>(), [b"crates/b/src/shared.rs".to_vec()]);
    }

    #[test]
    fn uses_paths_reexports_and_globs_connect_the_defining_module() {
        let (modules, facts) = build(&[
            (
                "src/lib.rs",
                "mod app;\nmod style;\nmod map;\npub use app::Rocket;\nfn top() {}\n",
            ),
            ("src/app.rs", "pub struct Rocket;\npub enum Intent { A }\n"),
            ("src/style.rs", "pub fn icon() {}\npub struct Icon;\npub fn label() {}\n"),
            (
                "src/map.rs",
                "mod details;\nuse crate::style::*;\nuse crate::Rocket;\nfn view() { details::card(); icon(); let _ = Icon; let local = 1; std::mem::drop(local); }\nfn label() {}\nfn other() { label(); }\n",
            ),
            (
                "src/map/details.rs",
                "use super::*;\npub fn card() { crate::app::Intent::A; super::super::top(); missing::thing(); let _ = Icon; view(); }\n",
            ),
        ]);
        let found = connections(&modules, &facts);
        // The crate root's re-export, a re-export followed through `crate::Rocket`, a
        // glob of style, a child module's function, `super::super`, an enum's module,
        // and names that `use super::*` brings in from the parent and from its glob.
        let found = named(&modules, &found);
        let row = |a: &str, b: &str, names: &[&str]| (a.to_owned(), b.to_owned(), names.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(
            found,
            [
                row("crate", "app", &["Rocket"]),
                row("map", "app", &["Rocket"]),
                row("map", "style", &["Icon", "icon"]),
                row("map", "map::details", &["card"]),
                row("map::details", "crate", &["top"]),
                row("map::details", "app", &["Intent"]),
                row("map::details", "style", &["Icon"]),
                row("map::details", "map", &["view"]),
            ],
            "{found:#?}"
        );
    }

    #[test]
    fn modules_a_use_line_brings_in_are_followed_by_their_paths() {
        let (modules, facts) = build(&[
            ("src/lib.rs", "mod map;\nmod style;\nmod helper;\n"),
            ("src/style.rs", "pub fn icon() {}\n"),
            ("src/helper.rs", "pub fn x() {}\n"),
            ("src/map.rs", "mod details;\nmod drawing;\nuse crate::style;\nfn view() {}\n#[cfg(test)]\nmod tests { use super::*; fn t() { details::card(); } }\n"),
            ("src/map/details.rs", "pub fn card() {}\n"),
            // A sibling through `use super::*`, a module the parent imports by name, and an alias.
            ("src/map/drawing.rs", "use super::*;\nuse crate::helper as h;\nfn draw() { details::card(); style::icon(); h::x(); outside::thing(); }\n"),
        ]);
        let found = connections(&modules, &facts);
        let found = named(&modules, &found);
        let row = |a: &str, b: &str, names: &[&str]| (a.to_owned(), b.to_owned(), names.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(
            found,
            [
                // map: its `use crate::style` line, and its inline tests calling a child.
                row("map", "style", &["style"]),
                row("map", "map::details", &["card"]),
                row("map::drawing", "style", &["icon", "style"]),
                row("map::drawing", "helper", &["helper", "x"]),
                row("map::drawing", "map::details", &["card"]),
            ],
            "{found:#?}"
        );
    }

    #[test]
    fn a_name_a_module_declares_itself_is_not_matched_through_a_glob() {
        let (modules, facts) = build(&[
            ("src/lib.rs", "mod a;\nmod b;\n"),
            ("src/a.rs", "pub fn label() {}\n"),
            ("src/b.rs", "use crate::a::*;\nfn label() {}\nfn run() { label(); }\n"),
        ]);
        assert!(connections(&modules, &facts).uses.is_empty());
    }

    #[test]
    fn globs_that_lead_back_to_each_other_are_searched_once() {
        // Every module globs the crate root, which globs every module back: unknown
        // names once took minutes as each search walked the same loop again.
        let count = 40;
        let mut files = vec![(
            "src/lib.rs".to_owned(),
            (0..count).map(|i| format!("mod m{i};\npub use m{i}::*;\n")).collect::<String>(),
        )];
        for i in 0..count {
            let calls: String = (0..30).map(|k| format!("Missing{k}(); ")).collect();
            files.push((format!("src/m{i}.rs"), format!("use crate::*;\npub fn own{i}() {{}}\nfn run() {{ own{}(); {calls} }}\n", (i + 1) % count)));
        }
        let borrowed: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
        let (modules, facts) = build(&borrowed);
        let start = std::time::Instant::now();
        let found = connections(&modules, &facts);
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
        // Each module uses the next one's function, found through the root's globs.
        assert_eq!(found.uses.iter().filter(|((a, _), _)| *a != 0).count(), count);
    }

    #[test]
    fn rust_opens_first_and_other_crates_are_named_once() {
        let package = |language, files| ModulePackage { folder: Vec::new(), name: Vec::new(), language, dialect: None, manifest: true, files, node: None };
        // A crate with a large JavaScript fixtures tree still opens on the crate.
        assert_eq!(default_package(&[package(ModuleLanguage::JavaScript, 900), package(ModuleLanguage::Rust, 3)]), Some(1));
        assert_eq!(default_package(&[package(ModuleLanguage::Rust, 0), package(ModuleLanguage::Go, 2), package(ModuleLanguage::Python, 5)]), Some(2));
        let (modules, facts) = build(&[
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "use ::serde::Serialize;\nuse std::fmt;\nuse fmt::Write;\nuse std::cmp::Ordering;\nuse Ordering::*;\nenum Mode { A }\nuse Mode::*;\n"),
        ]);
        let found = connections(&modules, &facts);
        let outside: Vec<&str> = found.outside.get(&1).map(|n| n.iter().map(String::as_str).collect()).unwrap_or_default();
        assert_eq!(outside, ["serde", "std"]);
    }

    #[test]
    fn other_programs_are_told_apart() {
        let files: BTreeSet<Vec<u8>> = ["p/src/lib.rs", "p/src/main.rs", "p/src/bin/tool.rs", "p/src/bin/big/main.rs", "p/src/bin/big/part.rs", "p/src/util.rs"]
            .iter()
            .map(|p| p.as_bytes().to_vec())
            .collect();
        let found = programs(b"p/src", &files, b"p/src/lib.rs");
        assert_eq!(found, [b"p/src/bin/big/main.rs".to_vec(), b"p/src/bin/tool.rs".to_vec(), b"p/src/main.rs".to_vec()]);
    }
}
