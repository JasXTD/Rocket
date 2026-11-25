//! The module tree of a JavaScript, Python or Go package, as Modules shows it: units
//! (files, folders, Go packages), which unit uses which and through which names, what
//! each declares and what each imports from outside the package. Rust's tree comes
//! from its own module rules (modules.rs).
use super::packages::{Aliases, file_name, inside, join, parent, within};
use super::*;
use std::collections::{HashMap, HashSet};

/// Names kept per connection.
const NAMES: usize = 64;
const OUTSIDE_NAMES: usize = 256;

#[derive(Clone, Debug)]
pub(super) struct Unit {
    /// Its path in the package as the map names it (`tests/git-relay.cjs`, `mime.text`,
    /// `net/http`); empty for a Python package's own `__init__.py` and a Go module's
    /// top-level package.
    pub key: String,
    pub label: Vec<u8>,
    pub detail: &'static [u8],
    /// Its own file, or the folder it stands for.
    pub path: Vec<u8>,
    pub parent: Option<usize>,
    /// A grouping folder: listed, never drawn as a dependency.
    pub folder: bool,
    pub files: usize,
    pub lines: usize,
}
#[derive(Default)]
pub(super) struct Built {
    pub units: Vec<Unit>,
    pub root: Option<usize>,
    pub links: BTreeMap<(usize, usize), BTreeSet<String>>,
    pub outline: BTreeMap<usize, Vec<OutlineItem>>,
    pub outside: BTreeMap<usize, BTreeSet<String>>,
    pub unlinked: Vec<(Vec<u8>, UnlinkedReason)>,
    pub followed: usize,
    pub unknown: usize,
    pub limited: bool,
}
impl Built {
    fn add(&mut self, unit: Unit) -> usize {
        self.units.push(unit);
        self.units.len() - 1
    }
    fn uses(&mut self, from: usize, to: usize, names: impl IntoIterator<Item = String>) {
        if from != to {
            let set = self.links.entry((from, to)).or_default();
            for name in names {
                if set.len() >= NAMES {
                    break;
                }
                set.insert(name);
            }
        }
    }
    fn outside(&mut self, unit: usize, name: String) {
        let set = self.outside.entry(unit).or_default();
        if set.len() < OUTSIDE_NAMES {
            set.insert(name);
        }
    }
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn lines(source: &str) -> usize {
    source.lines().count().max(1)
}

/// Folder units from the package's folder down to `folder` (not the package's own),
/// made as they are first needed.
struct Folders {
    at: HashMap<Vec<u8>, usize>,
}
impl Folders {
    fn unit(&mut self, built: &mut Built, base: &[u8], folder: &[u8], top: Option<usize>, key: &dyn Fn(&[u8]) -> String) -> Option<usize> {
        if folder == base || !inside(folder, base) {
            return top;
        }
        let mut missing = Vec::new();
        let mut ancestor = folder;
        let mut up = top;
        while ancestor != base {
            if let Some(&at) = self.at.get(ancestor) {
                up = Some(at);
                break;
            }
            missing.push(ancestor);
            ancestor = parent(ancestor);
        }
        for dir in missing.into_iter().rev() {
            let at = built.add(Unit {
                key: key(within(dir, base)),
                label: file_name(dir).to_vec(),
                detail: b"Folder",
                path: dir.to_vec(),
                parent: up,
                folder: false,
                files: 0,
                lines: 0,
            });
            self.at.insert(dir.to_vec(), at);
            up = Some(at);
        }
        up
    }
}

/// The deepest folder that holds every file, at or below `folder`: a project whose code
/// all sits in `src` starts there, as Rust's `src` is no module either.
fn shared_base<'a>(folder: &[u8], files: impl Iterator<Item = &'a Vec<u8>>) -> Vec<u8> {
    let mut base: Option<Vec<&[u8]>> = None;
    for file in files {
        let parts: Vec<&[u8]> = parent(file).split(|b| *b == b'/').filter(|p| !p.is_empty()).collect();
        base = Some(match base {
            None => parts,
            Some(b) => b.iter().zip(&parts).take_while(|(x, y)| x == y).map(|(x, _)| *x).collect(),
        });
    }
    let base = base.map(|parts| parts.join(&b'/')).unwrap_or_default();
    if inside(&base, folder) { base } else { folder.to_vec() }
}

// ---- JavaScript and TypeScript -----------------------------------------------------

const PROBES: [&str; 9] = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"];

enum Target {
    Local(usize),
    Missing,
    Outside(String),
    /// A JSON, CSS or other file that is not a module.
    Data,
}

/// One unit per source file and containing folder. Repeated names show their paths.
fn file_units<'a>(folder: &[u8], texts: &'a BTreeMap<Vec<u8>, String>, detail: impl Fn(&[u8]) -> &'static [u8], cancel: &CancellationToken) -> Result<(Built, HashMap<&'a [u8], usize>)> {
    let mut built = Built::default();
    let mut folders = Folders { at: HashMap::new() };
    let mut file_unit: HashMap<&[u8], usize> = HashMap::new();
    let key = |rel: &[u8]| text(rel);
    let base = shared_base(folder, texts.keys());
    for (path, source) in texts {
        cancel.check()?;
        let up = folders.unit(&mut built, &base, parent(path), None, &key);
        let at = built.add(Unit {
            key: text(within(path, &base)),
            label: file_name(path).to_vec(),
            detail: detail(path),
            path: path.clone(),
            parent: up,
            folder: false,
            files: 1,
            lines: lines(source),
        });
        file_unit.insert(path.as_slice(), at);
    }
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    for u in &built.units {
        *seen.entry(u.label.clone()).or_default() += 1;
    }
    for u in &mut built.units {
        if seen.get(&u.label).copied().unwrap_or(0) > 1 {
            u.label = u.key.clone().into_bytes();
        }
    }
    Ok((built, file_unit))
}

/// A JavaScript project: one unit per file and per folder; imports by relative path,
/// tsconfig paths, else from outside.
pub(super) fn javascript(folder: &[u8], texts: &BTreeMap<Vec<u8>, String>, known: &HashSet<&[u8]>, aliases: &Aliases, cancel: &CancellationToken) -> Result<Built> {
    let (mut built, file_unit) = file_units(folder, texts, |p| if packages::typescript_file(p) { b"TypeScript module" } else { b"JavaScript module" }, cancel)?;
    for (path, source) in texts {
        cancel.check()?;
        let facts = javascript::read(source);
        let from = file_unit[path.as_slice()];
        for import in facts.imports {
            match javascript_target(&import.specifier, path, folder, &file_unit, known, aliases) {
                Target::Local(to) => {
                    built.followed += 1;
                    built.uses(from, to, import.names);
                }
                Target::Missing => {
                    built.followed += 1;
                    built.unknown += 1;
                }
                Target::Outside(name) => built.outside(from, name),
                Target::Data => {}
            }
        }
        if !facts.outline.is_empty() {
            built.outline.insert(from, facts.outline);
        }
    }
    Ok(built)
}

/// The package an outside import names: `@scope/name` or its first part; `node:fs` as
/// written.
fn package_name(spec: &str) -> String {
    let mut parts = spec.split('/');
    let first = parts.next().unwrap_or_default();
    match parts.next() {
        Some(second) if first.starts_with('@') => format!("{first}/{second}"),
        _ => first.to_owned(),
    }
}

fn javascript_target(spec: &str, from: &[u8], project: &[u8], units: &HashMap<&[u8], usize>, known: &HashSet<&[u8]>, aliases: &Aliases) -> Target {
    let probe = |base: &[u8]| -> Option<Target> {
        let mut candidates: Vec<Vec<u8>> = Vec::new();
        // The top folder is the empty path: it has an index, never a name of its own.
        if !base.is_empty() {
            candidates.push(base.to_vec());
            candidates.extend(PROBES.iter().map(|e| [base, e.as_bytes()].concat()));
        }
        // TypeScript's own convention: `./view.js` names `view.ts`.
        for (js, ts) in [(".js", [".ts", ".tsx"]), (".jsx", [".tsx", ".tsx"]), (".mjs", [".mts", ".mts"]), (".cjs", [".cts", ".cts"])] {
            if let Some(stem) = base.strip_suffix(js.as_bytes()) {
                candidates.extend(ts.iter().map(|e| [stem, e.as_bytes()].concat()));
            }
        }
        candidates.extend(PROBES.iter().map(|e| [join(base, b"index").as_slice(), e.as_bytes()].concat()));
        for candidate in &candidates {
            if let Some(&at) = units.get(candidate.as_slice()) {
                return Some(Target::Local(at));
            }
            if known.contains(candidate.as_slice()) {
                if !packages::javascript_file(candidate) {
                    return Some(Target::Data);
                }
                // A project file left unread (too large), or another project's file.
                return Some(if inside(candidate, project) { Target::Missing } else { Target::Outside(text(candidate)) });
            }
        }
        (!base.is_empty() && known.contains([base, b".json"].concat().as_slice())).then_some(Target::Data)
    };
    if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
        let Some(base) = cargo::relative(parent(from), spec.as_bytes()) else { return Target::Missing };
        return probe(&base).unwrap_or(Target::Missing);
    }
    if spec.starts_with('/') || spec.is_empty() {
        return Target::Missing;
    }
    if let Some(places) = aliases.places(spec) {
        for place in places {
            if let Some(found) = probe(&place) {
                return found;
            }
        }
    }
    // With a baseUrl, `components/Button` is looked for there before outside.
    if let Some(found) = aliases.base.as_ref().and_then(|base| cargo::relative(base, spec.as_bytes())).and_then(|place| probe(&place)) {
        return found;
    }
    Target::Outside(package_name(spec))
}

fn script_units<'a>(folder: &[u8], texts: &'a BTreeMap<Vec<u8>, String>, detail: impl Fn(&[u8]) -> &'static [u8], cancel: &CancellationToken) -> Result<(Built, HashMap<&'a [u8], usize>)> {
    let (mut built, files) = file_units(folder, texts, detail, cancel)?;
    for unit in &mut built.units { unit.folder = unit.files == 0; }
    Ok((built, files))
}

pub(super) fn workflows(texts: &BTreeMap<Vec<u8>, String>, known: &HashSet<&[u8]>, cancel: &CancellationToken) -> Result<Built> {
    let (mut built, files) = script_units(workflows::FOLDER, texts, |_| b"GitHub Actions workflow", cancel)?;
    let mut targets = HashMap::new();
    for (&path, &id) in &files {
        built.units[id].key = text(path);
        targets.insert(path.to_vec(), id);
    }
    for (path, source) in texts {
        let from = files[path.as_slice()];
        let complete = workflows::read(source, known, cancel, |kind, path| {
            if built.followed >= MAP_LINKS { built.limited = true; return; }
            built.followed += 1;
            let to = if let Some(&id) = targets.get(&path) { id } else {
                if built.units.len() >= MAP_NODES { built.limited = true; return; }
                let id = built.add(Unit { key: text(&path), label: path.clone(), detail: b"Referenced file",
                    path: path.clone(), parent: None, folder: false, files: 1, lines: 0 });
                targets.insert(path, id);
                id
            };
            built.uses(from, to, [kind.to_owned()]);
        })?;
        built.limited |= !complete;
        if built.limited && built.followed >= MAP_LINKS { break; }
    }
    Ok(built)
}

pub(super) fn lua(group: &packages::Found, texts: &BTreeMap<Vec<u8>, String>, cancel: &CancellationToken) -> Result<Built> {
    let (mut built, files) = script_units(&group.folder, texts, |p| if p.ends_with(b".lua") { b"Lua script" } else { b"JSON file" }, cancel)?;
    if let Some(path) = &group.manifest {
        let Some(manifest) = texts.get(path) else { return Ok(built); };
        if lua::integration_manifest(manifest) {
            let entry = join(&group.folder, b"main.lua");
            if let Some(&to) = files.get(entry.as_slice()).filter(|_| texts[&entry].len() <= 64 * 1024) {
                built.uses(files[path.as_slice()], to, ["main.lua".to_owned()]);
                built.followed = 1;
            }
            // Rocket loads this entry directly; its sandbox has no Lua module loader.
            return Ok(built);
        }
    }
    let known: HashSet<_> = group.files.iter().map(Vec::as_slice).collect();
    for (path, source) in texts.iter().filter(|(p, _)| p.ends_with(b".lua")) {
        cancel.check()?;
        let from = files[path.as_slice()];
        lua::read(source, cancel, |name| {
            if built.followed >= MAP_LINKS { built.limited = true; return false; }
            built.followed += 1;
            let stem = name.replace('.', "/");
            let direct = join(&group.folder, format!("{stem}.lua").as_bytes());
            let init = join(&group.folder, format!("{stem}/init.lua").as_bytes());
            let mut candidates = [&direct, &init].into_iter().filter(|p| known.contains(p.as_slice()));
            match (candidates.next(), candidates.next()) {
                (Some(path), None) => match files.get(path.as_slice()) {
                    Some(&to) => built.uses(from, to, [name.to_owned()]),
                    None => built.unknown += 1,
                },
                (None, _) => {
                    let outside = built.outside.entry(from).or_default();
                    if outside.len() < OUTSIDE_NAMES { outside.insert(name.to_owned()); }
                    else if !outside.contains(name) { built.limited = true; }
                }
                _ => built.unknown += 1,
            }
            true
        })?;
        if built.limited && built.followed >= MAP_LINKS { break; }
    }
    Ok(built)
}

/// Scripts share the immutable file index across groups. For cwd-relative operands,
/// use a unique committed match at the repository root or beside the calling script.
/// Never pick one if both exist, and never search the user's PATH or filesystem.
pub(super) fn scripts(language: ModuleLanguage, folder: &[u8], texts: &BTreeMap<Vec<u8>, String>, known: &HashSet<&[u8]>, cancel: &CancellationToken) -> Result<Built> {
    let ps = language == ModuleLanguage::PowerShell;
    let (mut built, files) = script_units(folder, texts, |_| if ps { b"PowerShell script" } else { b"Bash script" }, cancel)?;
    for (path, source) in texts {
        cancel.check()?;
        if ps && path.rsplit(|&b| b == b'.').next().is_some_and(|e| e.eq_ignore_ascii_case(b"psd1")) {
            continue; // Module manifests contain data.
        }
        let from = files[path.as_slice()];
        scripts::read(language, source, cancel, |load| {
            if built.followed >= MAP_LINKS {
                built.limited = true;
                return;
            }
            built.followed += 1;
            let lookup = |base: &[u8]| cargo::relative(base, load.path.as_bytes()).filter(|p| known.contains(p.as_slice()));
            let beside = lookup(parent(path));
            let root = if load.beside_script || parent(path).is_empty() { None } else { lookup(b"") };
            let target = match (beside, root) {
                (Some(a), Some(b)) if a != b => None,
                (Some(a), _) | (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            let Some(target) = target else { built.unknown += 1; return; };
            let target_language = packages::script_language(&target);
            if ps && target_language != Some(language) { return; }
            if let Some(&to) = files.get(target.as_slice()) {
                built.uses(from, to, [load.command.to_owned()]);
            } else if target_language == Some(language) && packages::top_folder(&target) == folder {
                built.unknown += 1; // Same group, outside this read's source limits.
            } else {
                let Ok(name) = std::str::from_utf8(&target) else { built.unknown += 1; return; };
                let set = built.outside.entry(from).or_default();
                if set.len() < OUTSIDE_NAMES {
                    set.insert(name.to_owned());
                } else if !set.contains(name) {
                    built.limited = true;
                }
            }
        })?;
        if built.limited && built.followed >= MAP_LINKS { break; }
    }
    Ok(built)
}

// ---- Python -------------------------------------------------------------------------

/// A Python package (`package`: the folder has `__init__.py`, its root) or a folder of
/// scripts: one unit per module and per folder; imports by their dotted paths.
pub(super) fn python(folder: &[u8], package: bool, texts: &BTreeMap<Vec<u8>, String>, cancel: &CancellationToken) -> Result<Built> {
    let mut built = Built::default();
    let name = text(file_name(folder));
    let own_init = join(folder, b"__init__.py");
    let root = package.then(|| {
        built.add(Unit {
            key: String::new(),
            label: if name.is_empty() { b"__init__.py".to_vec() } else { name.clone().into_bytes() },
            detail: b"Package root",
            path: own_init.clone(),
            parent: None,
            folder: false,
            files: usize::from(texts.contains_key(&own_init)),
            lines: texts.get(&own_init).map(|s| lines(s)).unwrap_or_default(),
        })
    });
    built.root = root;
    let base = if package { folder.to_vec() } else { shared_base(folder, texts.keys()) };
    // Dotted keys: `mime.text`; a name used twice keeps its file name (`a.py` beside `a/`).
    let mut keys: HashSet<String> = HashSet::new();
    let dotted = |rel: &[u8]| text(rel.strip_suffix(b".py").unwrap_or(rel)).replace('/', ".");
    let mut folders = Folders { at: HashMap::new() };
    let mut modules: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut file_unit: HashMap<&[u8], usize> = HashMap::new();
    if let Some(r) = root {
        file_unit.insert(own_init.as_slice(), r);
    }
    // Every folder first: `a.py` sorts before `a/`, and the folder keeps the dotted name.
    for path in texts.keys() {
        if Some(path) != root.map(|_| &own_init) {
            folders.unit(&mut built, &base, parent(path), root, &dotted);
        }
    }
    for (path, source) in texts {
        if Some(path) == root.map(|_| &own_init) {
            continue;
        }
        let dir = parent(path);
        if file_name(path) == b"__init__.py"
            && dir != base.as_slice()
            && let Some(at) = folders.unit(&mut built, &base, dir, root, &dotted)
        {
            // The folder's unit becomes its package, with this file as its own.
            let unit = &mut built.units[at];
            unit.detail = b"Python package";
            unit.path = path.clone();
            unit.files = 1;
            unit.lines = lines(source);
            file_unit.insert(path.as_slice(), at);
            continue;
        }
        let up = folders.unit(&mut built, &base, dir, root, &dotted);
        let mut key = dotted(within(path, &base));
        if !keys.insert(key.clone()) || folders.at.values().any(|&f| built.units[f].key == key) {
            key = text(within(path, &base));
        }
        let at = built.add(Unit {
            key,
            label: file_name(path).strip_suffix(b".py").unwrap_or(file_name(path)).to_vec(),
            detail: b"Python module",
            path: path.clone(),
            parent: up,
            folder: false,
            files: 1,
            lines: lines(source),
        });
        modules.insert(path.strip_suffix(b".py").unwrap_or(path).to_vec(), at);
        file_unit.insert(path.as_slice(), at);
    }
    let folder_at: HashMap<Vec<u8>, usize> = folders.at.iter().map(|(k, v)| (k.clone(), *v)).chain(root.map(|r| (folder.to_vec(), r))).collect();
    let details: Vec<&'static [u8]> = built.units.iter().map(|u| u.detail).collect();
    let built_detail = |unit: usize| details[unit];
    // The deepest module or package along a dotted path from `base`, and how many of
    // the path's names it took.
    let locate = |base: &[u8], segments: &[String]| -> (Option<usize>, usize) {
        let mut current = base.to_vec();
        let mut found = folder_at.get(base).copied();
        for (i, s) in segments.iter().enumerate() {
            let next = join(&current, s.as_bytes());
            // Python takes a package (with __init__.py) before a module of its name, and
            // the module before a folder without one.
            let package = folder_at.get(&next).filter(|&&f| built_detail(f) == b"Python package");
            if let (Some(&m), None) = (modules.get(&next), package) {
                return (Some(m), i + 1);
            }
            match folder_at.get(&next) {
                Some(&f) => {
                    found = Some(f);
                    current = next;
                }
                None => return (found, i),
            }
        }
        (found, segments.len())
    };
    let is_folder = |unit: usize, built: &Built| built.units[unit].detail != b"Python module";
    for (path, source) in texts {
        cancel.check()?;
        let Some(&from) = file_unit.get(path.as_slice()) else { continue };
        let facts = python::read(source);
        for import in facts.imports {
            // Where the dotted path starts.
            let base: Option<Vec<u8>> = if import.level > 0 {
                let mut base = parent(path).to_vec();
                let mut ok = true;
                for _ in 1..import.level {
                    if base.as_slice() == folder || base.is_empty() {
                        ok = false;
                        break;
                    }
                    base = parent(&base).to_vec();
                }
                ok.then_some(base)
            } else if package {
                (import.module.first().map(String::as_bytes) == Some(file_name(folder))).then(|| parent(folder).to_vec())
            } else {
                // A script's own folder first, then the folder of scripts.
                [parent(path), folder]
                    .into_iter()
                    .find(|b| locate(b, &import.module[..1.min(import.module.len())]).1 >= 1)
                    .map(<[u8]>::to_vec)
            };
            let Some(base) = base else {
                if import.level > 0 {
                    built.followed += 1;
                    built.unknown += 1;
                } else if let Some(first) = import.module.first() {
                    built.outside(from, first.clone());
                }
                continue;
            };
            built.followed += 1;
            let (found, used) = locate(&base, &import.module);
            let Some(target) = found.filter(|_| used == import.module.len()) else {
                built.unknown += 1;
                continue;
            };
            if import.names.is_empty() {
                // import email.utils: the module, named by its last part.
                let shown = import.module.last().cloned().unwrap_or_else(|| text(file_name(&base)));
                built.uses(from, target, [shown]);
                continue;
            }
            // from email import errors: errors when it is a module, else the names in the package.
            let mut own = Vec::new();
            for n in import.names {
                let module = (n != "*" && is_folder(target, &built))
                    .then(|| {
                        let at = if target == root.unwrap_or(usize::MAX) { folder.to_vec() } else { built.units[target].path.clone() };
                        let dir = if file_name(&at) == b"__init__.py" { parent(&at).to_vec() } else { at };
                        locate(&dir, std::slice::from_ref(&n))
                    })
                    .and_then(|(m, used)| m.filter(|_| used == 1));
                match module {
                    Some(m) => built.uses(from, m, [n]),
                    None => own.push(n),
                }
            }
            if !own.is_empty() {
                built.uses(from, target, own);
            }
        }
        if !facts.outline.is_empty() {
            built.outline.insert(from, facts.outline);
        }
    }
    Ok(built)
}

// ---- Go -------------------------------------------------------------------------------

/// A Go module: one unit per package (a folder of Go files), each standing alone; a
/// folder that only holds packages is listed for the tree. Imports name packages by
/// the module's path; the names used through one connect it.
/// A Go file read: its path, what it says and its lines.
type GoFile<'a> = (&'a Vec<u8>, golang::Facts, usize);
/// `files` are all the module's Go files, read or not: a package left unread by the read
/// limits is still the module's own, never outside it.
pub(super) fn go(folder: &[u8], module: &str, texts: &BTreeMap<Vec<u8>, String>, files_of_module: &[Vec<u8>], cancel: &CancellationToken) -> Result<Built> {
    let mut built = Built::default();
    let own_folders: HashSet<&[u8]> = files_of_module.iter().map(|f| parent(f)).collect();
    let mut files: BTreeMap<Vec<u8>, Vec<GoFile>> = BTreeMap::new();
    for (path, source) in texts {
        cancel.check()?;
        let facts = golang::read(source);
        if facts.ignored {
            built.unlinked.push((path.clone(), UnlinkedReason::BuildIgnore));
            continue;
        }
        files.entry(parent(path).to_vec()).or_default().push((path, facts, lines(source)));
    }
    let rel_of = |dir: &[u8]| text(if dir == folder { b"" } else { within(dir, folder) });
    // The package clause most of a folder's files have.
    let declared: HashMap<Vec<u8>, String> = files
        .iter()
        .map(|(dir, list)| {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for (_, f, _) in list {
                if let Some(p) = &f.package {
                    *counts.entry(p.as_str()).or_default() += 1;
                }
            }
            let name = counts.into_iter().max_by_key(|(n, c)| (*c, std::cmp::Reverse(*n))).map(|(n, _)| n.to_owned());
            (dir.clone(), name.unwrap_or_else(|| text(file_name(dir))))
        })
        .collect();
    let last = module.rsplit('/').next().unwrap_or(module);
    // Packages first, by folder; then the folders between them.
    let mut unit_at: HashMap<Vec<u8>, usize> = HashMap::new();
    for (dir, list) in &files {
        let rel = rel_of(dir);
        let label = if rel.is_empty() { declared.get(dir).cloned().unwrap_or_else(|| last.to_owned()).into_bytes() } else { file_name(rel.as_bytes()).to_vec() };
        let at = built.add(Unit {
            key: rel,
            label,
            detail: b"Go package",
            path: dir.clone(),
            parent: None,
            folder: false,
            files: list.len(),
            lines: list.iter().map(|(_, _, n)| n).sum(),
        });
        unit_at.insert(dir.clone(), at);
    }
    let packages: Vec<Vec<u8>> = files.keys().cloned().collect();
    for dir in &packages {
        let mut up = parent(dir).to_vec();
        while dir.as_slice() != folder && inside(&up, folder) && up.as_slice() != folder && !unit_at.contains_key(&up) {
            let at = built.add(Unit {
                key: rel_of(&up),
                label: file_name(&up).to_vec(),
                detail: b"Folder",
                path: up.clone(),
                parent: None,
                folder: true,
                files: 0,
                lines: 0,
            });
            unit_at.insert(up.clone(), at);
            up = parent(&up).to_vec();
        }
    }
    // Each unit sits in the nearest unit above it; the module's own top-level package
    // holds none, as it is listed beside the others.
    let units: Vec<(Vec<u8>, usize)> = unit_at.iter().map(|(d, a)| (d.clone(), *a)).collect();
    for (dir, at) in units {
        if dir.as_slice() == folder {
            continue;
        }
        let mut up = parent(&dir).to_vec();
        while inside(&up, folder) && up.as_slice() != folder {
            if let Some(&p) = unit_at.get(&up) {
                built.units[at].parent = Some(p);
                break;
            }
            up = parent(&up).to_vec();
        }
    }
    // Where an import path is in this module.
    let local = |path: &str| -> Option<Vec<u8>> {
        let rel = if module == "std" {
            path
        } else if path == module {
            ""
        } else {
            path.strip_prefix(module)?.strip_prefix('/')?
        };
        Some(if rel.is_empty() { folder.to_vec() } else { join(folder, rel.as_bytes()) })
    };
    for (dir, list) in &files {
        let from = unit_at[dir];
        let mut outline: Vec<OutlineItem> = Vec::new();
        let mut methods: HashMap<String, usize> = HashMap::new();
        for (_, facts, _) in list {
            cancel.check()?;
            for import in &facts.imports {
                let Some(target) = local(&import.path).filter(|_| module == "std" || !module.is_empty()) else {
                    built.outside(from, import.path.clone());
                    continue;
                };
                let Some(&to) = unit_at.get(&target).filter(|&&to| !built.units[to].folder) else {
                    // A path with none of the module's files is outside it: std's `C` (cgo) and
                    // vendored paths, or a nested module with its own go.mod. A package the read
                    // limits left unread is the module's own all the same.
                    if !own_folders.contains(target.as_slice()) {
                        built.outside(from, import.path.clone());
                    } else {
                        built.followed += 1;
                        built.unknown += 1;
                    }
                    continue;
                };
                built.followed += 1;
                if from == to {
                    continue;
                }
                let name = match import.alias.as_deref() {
                    Some("_" | ".") => None,
                    Some(alias) => Some(alias.to_owned()),
                    None => declared.get(&target).cloned(),
                };
                let names: BTreeSet<String> = name
                    .map(|name| facts.selectors.iter().filter(|(x, y)| *x == name && golang::exported(y)).map(|(_, y)| y.clone()).collect())
                    .unwrap_or_default();
                // An import with no name used through it (`_`, `.`) still connects the two.
                built.links.entry((from, to)).or_default();
                built.uses(from, to, names);
            }
            outline.extend(facts.outline.iter().cloned());
            for m in &facts.methods {
                *methods.entry(m.clone()).or_default() += 1;
            }
        }
        for item in outline.iter_mut().filter(|o| o.kind == OutlineKind::Type) {
            item.functions = methods.get(&text(&item.name)).copied().unwrap_or_default();
        }
        if !outline.is_empty() {
            built.outline.insert(from, outline);
        }
    }
    Ok(built)
}

/// The names two sets share, sorted, up to what a connection keeps: the smaller set is
/// the one walked.
fn shared_names(used: &BTreeSet<String>, declared: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = if declared.len() < used.len() {
        declared.iter().filter(|d| used.contains(*d)).cloned().collect()
    } else {
        used.iter().filter(|u| declared.contains(*u)).cloned().collect()
    };
    out.sort();
    out.truncate(NAMES);
    out
}

/// The folder most of `files` are in; the shallowest, then the first by name, of those
/// that hold as many.
fn main_folder<'a>(files: impl Iterator<Item = &'a Vec<u8>>) -> Vec<u8> {
    let mut counts: BTreeMap<&[u8], usize> = BTreeMap::new();
    for f in files {
        *counts.entry(parent(f)).or_default() += 1;
    }
    counts.into_iter().max_by_key(|(dir, n)| (*n, std::cmp::Reverse(dir.len()), std::cmp::Reverse(*dir))).map(|(d, _)| d.to_vec()).unwrap_or_default()
}

/// Units for dotted names, Java's packages and C#'s namespaces, each standing alone:
/// one per name, from `names` (its folder, files and lines), nested in the list under
/// the name above it. Parts of names between them (`com.acme` above `com.acme.store`)
/// are folders, listed and never drawn. The list starts below the part every name
/// shares, as Go's starts in its module, and each is labelled by its name below that
/// part (`store`, `web.api`); a name that is that part is listed at the top beside the
/// others with its last name, as is the unnamed one (`unnamed` its label). The keys
/// are the whole names.
fn dotted_units(built: &mut Built, names: &BTreeMap<String, (Vec<u8>, usize, usize)>, detail: &'static [u8], unnamed: &[u8]) -> HashMap<String, usize> {
    let parts = |name: &str| -> Vec<String> { if name.is_empty() { Vec::new() } else { name.split('.').map(str::to_owned).collect() } };
    let mut base: Option<Vec<String>> = None;
    for name in names.keys().filter(|n| !n.is_empty()) {
        let p = parts(name);
        base = Some(match base {
            None => p,
            Some(b) => b.iter().zip(&p).take_while(|(x, y)| x == y).map(|(x, _)| x.clone()).collect(),
        });
    }
    let base = base.unwrap_or_default();
    // A name below the shared part, or the last name of the one that is it.
    let below = |p: &[String]| -> Option<Vec<u8>> {
        if p.len() > base.len() { Some(p[base.len()..].join(".").into_bytes()) } else { p.last().map(|l| l.clone().into_bytes()) }
    };
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut named: HashMap<String, usize> = HashMap::new();
    for (name, (folder, files, lines)) in names {
        let p = parts(name);
        let label = below(&p).unwrap_or_else(|| unnamed.to_vec());
        let unit = built.add(Unit {
            key: name.clone(),
            label,
            detail,
            path: folder.clone(),
            parent: None,
            folder: false,
            files: *files,
            lines: *lines,
        });
        at.insert(name.clone(), unit);
        named.insert(name.clone(), unit);
    }
    // The parts between the shared start and each name, as folders.
    for (name, (folder, _, _)) in names {
        let p = parts(name);
        for l in base.len() + 1..p.len() {
            let prefix = p[..l].join(".");
            if at.contains_key(&prefix) {
                continue;
            }
            // Where the folder is: the name's own folder less the names below the part,
            // when the folders follow the names; else the folder above it.
            let dirs: Vec<&[u8]> = folder.split(|b| *b == b'/').filter(|d| !d.is_empty()).collect();
            let rest = &p[l..];
            let follows = dirs.len() >= rest.len() && dirs[dirs.len() - rest.len()..].iter().zip(rest).all(|(d, n)| *d == n.as_bytes());
            let path = if follows { dirs[..dirs.len() - rest.len()].join(&b'/') } else { parent(folder).to_vec() };
            let unit = built.add(Unit {
                key: prefix.clone(),
                label: below(&p[..l]).unwrap_or_default(),
                detail: b"Folder",
                path,
                parent: None,
                folder: true,
                files: 0,
                lines: 0,
            });
            at.insert(prefix, unit);
        }
    }
    for (name, &unit) in &at {
        let p = parts(name);
        if p.len() > base.len() + 1 {
            built.units[unit].parent = at.get(&p[..p.len() - 1].join(".")).copied();
        }
    }
    // The shared part's own last name can be another's name below it (`app` beside
    // `com.acme.app.app`): those show the whole name.
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    for u in &built.units {
        *seen.entry(u.label.clone()).or_default() += 1;
    }
    for (name, &unit) in &at {
        if seen.get(&built.units[unit].label).copied().unwrap_or(0) > 1 && !name.is_empty() {
            built.units[unit].label = name.clone().into_bytes();
        }
    }
    named
}

// ---- Java and Kotlin -----------------------------------------------------------------

/// The package an import from outside the project names: its names before the first
/// one starting with a capital letter (a class), else all but a member's.
fn java_package(import: &jvm::Import) -> Vec<String> {
    let path = &import.path;
    let end = match path.iter().position(|p| p.starts_with(|c: char| c.is_uppercase())) {
        Some(0) => path.len(),
        Some(at) => at,
        None if import.all => path.len(),
        None => path.len() - 1,
    };
    path[..end.max(1)].to_vec()
}

/// A Java or Kotlin project: one unit per package (its files' package clause), each
/// standing alone, the parts of package names between them listed for the tree.
/// Imports name a package and one of its types (or Kotlin's functions); a `*` import
/// takes the names the file uses that the package declares, and a name written out with
/// its package (`com.acme.store.Db`) connects the two as well. `files_of_project` are
/// all the project's files, read or not: an import of a package the read limits left
/// unread is still the project's own, never outside it.
pub(super) fn java(texts: &BTreeMap<Vec<u8>, String>, files_of_project: &[Vec<u8>], cancel: &CancellationToken) -> Result<Built> {
    let mut built = Built::default();
    let mut files: Vec<(&Vec<u8>, jvm::Facts, usize)> = Vec::new();
    for (path, source) in texts {
        cancel.check()?;
        files.push((path, jvm::read(source, packages::kotlin_file(path)), lines(source)));
    }
    let mut by_package: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, (_, f, _)) in files.iter().enumerate() {
        by_package.entry(f.package.join(".")).or_default().push(i);
    }
    let names: BTreeMap<String, (Vec<u8>, usize, usize)> = by_package
        .iter()
        .map(|(name, list)| (name.clone(), (main_folder(list.iter().map(|&i| files[i].0)), list.len(), list.iter().map(|&i| files[i].2).sum())))
        .collect();
    let unit_of = dotted_units(&mut built, &names, b"Java package", b"(default package)");
    // What each package declares, in all its files.
    let mut declared: HashMap<usize, HashSet<String>> = HashMap::new();
    for (_, f, _) in &files {
        declared.entry(unit_of[&f.package.join(".")]).or_default().extend(f.outline.iter().map(|o| text(&o.name)));
    }
    let firsts: HashSet<&str> = names.keys().filter_map(|n| n.split('.').next()).filter(|n| !n.is_empty()).collect();
    // The folders of the project's files that were not read, as their names.
    let read: HashSet<&[u8]> = texts.keys().map(Vec::as_slice).collect();
    let unread: Vec<Vec<&[u8]>> = files_of_project.iter().filter(|p| !read.contains(p.as_slice())).map(|p| parent(p).split(|b| *b == b'/').collect()).collect();
    let own_unread = |package: &[String]| {
        unread
            .iter()
            .any(|dir| dir.len() >= package.len() && dir[dir.len() - package.len()..].iter().zip(package).all(|(d, n)| *d == n.as_bytes()))
    };
    // The longest start of `parts`, up to `max` names, that is one of the project's
    // packages and is followed by a class (named with a capital), the last name (a
    // Kotlin function, a lowercase class) or nothing: in `com.acme.app.missing.Thing`
    // `missing` is a package, not a class of com.acme.app.
    let local = |parts: &[String], max: usize| {
        (1..=max.min(parts.len())).rev().find_map(|n| {
            let class = parts.get(n).is_none_or(|next| n + 1 == parts.len() || next.starts_with(|c: char| c.is_uppercase()));
            unit_of.get(&parts[..n].join(".")).filter(|_| class).map(|&u| (u, n))
        })
    };
    let empty = HashSet::new();
    for (_, f, _) in &files {
        cancel.check()?;
        let from = unit_of[&f.package.join(".")];
        let own = declared.get(&from).unwrap_or(&empty);
        for import in &f.imports {
            if import.module {
                built.outside(from, import.path.join("."));
                continue;
            }
            let max = if import.all { import.path.len() } else { import.path.len().saturating_sub(1) };
            match local(&import.path, max) {
                Some((to, n)) => {
                    built.followed += 1;
                    if to == from {
                        continue;
                    }
                    let theirs = declared.get(&to).unwrap_or(&empty);
                    let used: Vec<String> = if n < import.path.len() {
                        // A type (or a Kotlin function), or with `*` a class's members.
                        let name = import.path[n].clone();
                        if !theirs.contains(&name) {
                            built.unknown += 1;
                        }
                        vec![name]
                    } else {
                        // `*`: what the file uses that the package declares, and its own package does not.
                        let mut used: Vec<String> = shared_names(&f.names, theirs).into_iter().filter(|x| !own.contains(x)).collect();
                        if used.is_empty() {
                            used.push("*".into());
                        }
                        used
                    };
                    built.links.entry((from, to)).or_default();
                    built.uses(from, to, used);
                }
                None => {
                    let package = java_package(import);
                    if own_unread(&package) {
                        built.followed += 1;
                        built.unknown += 1;
                    } else {
                        built.outside(from, package.join("."));
                    }
                }
            }
        }
        for q in f.qualified.iter().filter(|q| firsts.contains(q[0].as_str())) {
            if let Some((to, n)) = local(q, q.len() - 1)
                && to != from
                && declared.get(&to).is_some_and(|d| d.contains(&q[n]))
            {
                built.links.entry((from, to)).or_default();
                built.uses(from, to, [q[n].clone()]);
            }
        }
        if !f.outline.is_empty() {
            built.outline.entry(from).or_default().extend(f.outline.iter().cloned());
        }
    }
    Ok(built)
}

// ---- C and C++ -------------------------------------------------------------------------

/// The snapshot's C and C++ files by their file names, for an include found by the end
/// of its path, as a build's include folders find it.
pub(super) struct Headers {
    by_name: HashMap<Vec<u8>, Vec<Vec<u8>>>,
}
impl Headers {
    pub fn new<'a>(paths: impl Iterator<Item = &'a [u8]>) -> Self {
        let mut by_name: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
        for p in paths.filter(|p| packages::c_file(p)) {
            by_name.entry(file_name(p).to_vec()).or_default().push(p.to_vec());
        }
        Self { by_name }
    }
    /// The file whose path ends with `spec`, one of `within` when given, never `from`
    /// itself: the one sharing the most folders with `from`, then the shortest.
    pub fn find(&self, spec: &[u8], from: &[u8], within: Option<&HashSet<&[u8]>>) -> Option<&[u8]> {
        let list = self.by_name.get(file_name(spec))?;
        let suffix = [b"/".as_slice(), spec].concat();
        list.iter()
            .filter(|p| p.as_slice() != from && (p.as_slice() == spec || p.ends_with(&suffix)) && within.is_none_or(|w| w.contains(p.as_slice())))
            .max_by_key(|p| (shared_folders(p, from), std::cmp::Reverse(p.len()), std::cmp::Reverse(p.as_slice())))
            .map(Vec::as_slice)
    }
}
/// How many folders two paths share from the top.
fn shared_folders(a: &[u8], b: &[u8]) -> usize {
    let (a, b) = (parent(a), parent(b));
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    a.split(|x| *x == b'/').zip(b.split(|x| *x == b'/')).take_while(|(x, y)| x == y).count()
}
/// The library an include from outside names: `<openssl/ssl.h>` is openssl, `<stdio.h>`
/// stdio.h.
fn library(spec: &str) -> String {
    match spec.split_once('/') {
        Some((first, _)) if !first.is_empty() => first.to_owned(),
        _ => spec.to_owned(),
    }
}

/// A C or C++ project: one unit per file and per folder. An `#include "…"` is looked
/// for beside the including file first; then, as the build's include folders would find
/// it, one of the project's files whose path ends with it, nearest first; then such a
/// file elsewhere in the snapshot, for `<…>` only when it names a folder
/// (`<git2/types.h>`, not `<stdio.h>`). A file never includes itself: a header's
/// `#include_next <stdint.h>` names the system's. The names on a connection are those
/// the included file declares and the including one uses. `files_of_project` are all
/// the project's files, read or not: a project of loose files can sit in a folder that
/// holds another project, whose files are not its own.
pub(super) fn c(folder: &[u8], texts: &BTreeMap<Vec<u8>, String>, files_of_project: &[Vec<u8>], known: &HashSet<&[u8]>, headers: &Headers, cancel: &CancellationToken) -> Result<Built> {
    let own: HashSet<&[u8]> = files_of_project.iter().map(Vec::as_slice).chain(texts.keys().map(Vec::as_slice)).collect();
    // A `.h` in a project with C++ files is C++'s.
    let cpp = texts.keys().any(|p| packages::cpp_file(p));
    let (mut built, file_unit) = file_units(folder, texts, |path| {
        if path.ends_with(b".mm") { return b"Objective-C++ source"; }
        if path.ends_with(b".m") { return b"Objective-C source"; }
        let is_cpp = packages::cpp_file(path) || cpp && !path.ends_with(b".c");
        match (is_cpp, cfamily::header(path)) {
            (true, true) => b"C++ header",
            (true, false) => b"C++ source",
            (false, true) => b"C header",
            (false, false) => b"C source",
        }
    }, cancel)?;
    let mut facts: Vec<(&Vec<u8>, cfamily::Facts)> = Vec::new();
    for (path, source) in texts {
        cancel.check()?;
        facts.push((path, cfamily::read(source, cfamily::header(path))));
    }
    // What each file declares, for the names an include brings in.
    let declared: HashMap<usize, HashSet<String>> = facts
        .iter()
        .map(|(path, f)| (file_unit[path.as_slice()], f.outline.iter().map(|o| text(&o.name)).chain(f.constants.iter().cloned()).collect()))
        .collect();
    for (path, f) in &facts {
        cancel.check()?;
        let from = file_unit[path.as_slice()];
        for include in &f.includes {
            let spec = include.path.trim_start_matches("./").as_bytes();
            let beside = if include.angle { None } else { cargo::relative(parent(path), spec).filter(|p| known.contains(p.as_slice()) && p != *path) };
            // `..` only makes sense beside the including file.
            let plain = !spec.split(|b| *b == b'/').any(|p| p == b".." || p == b".");
            let target: Option<Vec<u8>> = beside
                .or_else(|| plain.then(|| headers.find(spec, path, Some(&own))).flatten().map(<[u8]>::to_vec))
                .or_else(|| (plain && (!include.angle || spec.contains(&b'/'))).then(|| headers.find(spec, path, None)).flatten().map(<[u8]>::to_vec));
            match target {
                Some(t) => match file_unit.get(t.as_slice()) {
                    Some(&to) => {
                        built.followed += 1;
                        let used = declared.get(&to).map(|d| shared_names(&f.names, d)).unwrap_or_default();
                        built.links.entry((from, to)).or_default();
                        built.uses(from, to, used);
                    }
                    // Generated tables and the like (`.inc`, `.def`) are no C file.
                    None if !packages::c_file(&t) => {}
                    // A project file the read limits left unread.
                    None if own.contains(t.as_slice()) => {
                        built.followed += 1;
                        built.unknown += 1;
                    }
                    None => built.outside(from, text(&t)),
                },
                None if include.angle => built.outside(from, library(&include.path)),
                // A quoted include no committed file ends with: generated, or from outside.
                None => {
                    built.followed += 1;
                    built.unknown += 1;
                }
            }
        }
        if !f.outline.is_empty() {
            built.outline.insert(from, f.outline.clone());
        }
    }
    Ok(built)
}

// ---- C# ------------------------------------------------------------------------------------

/// A C# project: one unit per namespace (the namespace a file's types are in), each
/// standing alone, the parts of namespace names between them listed for the tree. A
/// `using` line of the project's namespace connects the two; the type names a file uses
/// connect it to the namespace that declares them, looked for as C# does: its own
/// namespace, its parents, then the namespaces its using lines (and the project's
/// `global using` lines) open. A name written out with its namespace connects too.
pub(super) fn csharp(texts: &BTreeMap<Vec<u8>, String>, cancel: &CancellationToken) -> Result<Built> {
    let mut built = Built::default();
    let mut files: Vec<(&Vec<u8>, csharp::Facts, usize)> = Vec::new();
    for (path, source) in texts {
        cancel.check()?;
        files.push((path, csharp::read(source), lines(source)));
    }
    // Each namespace's own files; a namespace only a second block of a file declares has that file's folder.
    let mut members: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut types: BTreeMap<String, Vec<(usize, OutlineItem)>> = BTreeMap::new();
    for (i, (_, f, _)) in files.iter().enumerate() {
        members.entry(f.namespace.join(".")).or_default().push(i);
        for (namespace, item) in &f.outline {
            types.entry(namespace.join(".")).or_default().push((i, item.clone()));
        }
    }
    let mut names: BTreeMap<String, (Vec<u8>, usize, usize)> = BTreeMap::new();
    for name in members.keys().chain(types.keys()) {
        if names.contains_key(name) {
            continue;
        }
        let own: Vec<usize> = members.get(name).cloned().unwrap_or_else(|| types[name].iter().map(|(i, _)| *i).collect());
        let counted = members.get(name).map(Vec::as_slice).unwrap_or_default();
        names.insert(name.clone(), (main_folder(own.iter().map(|&i| files[i].0)), counted.len(), counted.iter().map(|&i| files[i].2).sum()));
    }
    let unit_of = dotted_units(&mut built, &names, b"C# namespace", b"(global namespace)");
    // A partial type declared in several files is one: its methods add up.
    for (name, items) in types {
        let mut merged: Vec<OutlineItem> = Vec::new();
        for (_, item) in items {
            match merged.iter_mut().find(|m| m.kind == item.kind && m.name == item.name) {
                Some(m) => {
                    m.functions += item.functions;
                    m.exported |= item.exported;
                    if m.visibility.is_empty() {
                        m.visibility = item.visibility;
                    }
                }
                None => merged.push(item),
            }
        }
        built.outline.insert(unit_of[&name], merged);
    }
    let declared: HashMap<usize, HashSet<String>> = built.outline.iter().map(|(u, items)| (*u, items.iter().map(|o| text(&o.name)).collect())).collect();
    // The project's `global using` namespaces open for every file: they connect another
    // file only through the names it uses, or every namespace would point at them.
    let global: Vec<usize> = files
        .iter()
        .flat_map(|(_, f, _)| f.usings.iter().filter(|u| u.global && !u.members && u.alias.is_none()))
        .filter_map(|u| unit_of.get(&u.path.join(".")).copied())
        .collect();
    let firsts: HashSet<&str> = names.keys().filter_map(|n| n.split('.').next()).filter(|n| !n.is_empty()).collect();
    let local = |parts: &[String], max: usize| (1..=max.min(parts.len())).rev().find_map(|n| unit_of.get(&parts[..n].join(".")).map(|&u| (u, n)));
    let empty = HashSet::new();
    for (_, f, _) in &files {
        cancel.check()?;
        let from = unit_of[&f.namespace.join(".")];
        let own = declared.get(&from).unwrap_or(&empty);
        // The namespaces the using lines open, in order.
        let mut opened: Vec<usize> = Vec::new();
        for u in &f.usings {
            let whole = unit_of.get(&u.path.join(".")).copied();
            if u.members || u.alias.is_some() && whole.is_none() {
                // A type: `using static A.B.C;`, `using X = A.B.C;`. Namespaces and types are
                // both named with capitals: a shorter namespace counts only for a type it declares.
                let n = u.path.len() - 1;
                let found = unit_of.get(&u.path[..n].join(".")).map(|&to| (to, n)).or_else(|| {
                    local(&u.path, n.saturating_sub(1)).filter(|(to, at)| declared.get(to).is_some_and(|d| d.contains(&u.path[*at])))
                });
                match found {
                    Some((to, n)) => {
                        built.followed += 1;
                        if to != from {
                            let name = u.path[n].clone();
                            if !declared.get(&to).is_some_and(|d| d.contains(&name)) {
                                built.unknown += 1;
                            }
                            built.links.entry((from, to)).or_default();
                            built.uses(from, to, [name]);
                        }
                    }
                    None => built.outside(from, u.path[..u.path.len() - usize::from(u.members && u.path.len() > 1)].join(".")),
                }
                continue;
            }
            match whole {
                Some(to) => {
                    built.followed += 1;
                    if to != from {
                        opened.push(to);
                        built.links.entry((from, to)).or_default();
                    }
                }
                None => built.outside(from, u.path.join(".")),
            }
        }
        // Where each name the file uses is declared: its parents, nearest first (the
        // global namespace last), then the opened namespaces.
        let mut look: Vec<usize> = (1..f.namespace.len()).rev().filter_map(|n| unit_of.get(&f.namespace[..n].join(".")).copied()).collect();
        if !f.namespace.is_empty()
            && let Some(&g) = unit_of.get("")
        {
            look.push(g);
        }
        look.extend(opened);
        look.extend(global.iter().copied());
        let mut by_name: HashMap<&str, usize> = HashMap::new();
        for &ns in look.iter().filter(|&&ns| ns != from) {
            for name in declared.get(&ns).into_iter().flatten() {
                by_name.entry(name.as_str()).or_insert(ns);
            }
        }
        let mut used: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for name in f.names.iter().filter(|n| !own.contains(*n)) {
            if let Some(&ns) = by_name.get(name.as_str()) {
                used.entry(ns).or_default().push(name.clone());
            }
        }
        for q in f.qualified.iter().filter(|q| firsts.contains(q[0].as_str())) {
            if let Some((to, n)) = local(q, q.len() - 1)
                && to != from
                && declared.get(&to).is_some_and(|d| d.contains(&q[n]))
            {
                used.entry(to).or_default().push(q[n].clone());
            }
        }
        for (to, names) in used {
            built.links.entry((from, to)).or_default();
            built.uses(from, to, names);
        }
    }
    Ok(built)
}

/// Puts a package's units on the map as nodes after the package nodes, with their
/// connections, outline and outside imports.
pub(super) fn push(map: &mut RepositoryMap, details: &mut ModuleMap, built: Built) {
    map.limited |= built.limited;
    let base = map.nodes.len();
    let mut shown = 0;
    for u in &built.units {
        if map.nodes.len() >= MAP_NODES {
            map.limited = true;
            break;
        }
        let kind = if u.folder { MapNodeKind::Directory } else { MapNodeKind::Module };
        let mut n = node(kind, [b"module:".as_slice(), u.key.as_bytes()].concat(), u.label.clone(), u.detail.to_vec());
        n.path = Some(u.path.clone());
        n.count = u.lines;
        n.files = u.files;
        map.nodes.push(n);
        shown += 1;
    }
    for (i, u) in built.units.iter().enumerate().take(shown) {
        if let Some(p) = u.parent.filter(|&p| p < shown) {
            map.links.push(link(base + p, base + i, MapLinkKind::Contains));
        }
    }
    for ((from, to), names) in built.links {
        if from >= shown || to >= shown || map.links.len() >= MAP_LINKS {
            map.limited |= from < shown && to < shown;
            continue;
        }
        let mut e = link(base + from, base + to, MapLinkKind::DependsOn);
        e.weight = names.len();
        e.evidence = names.into_iter().map(String::into_bytes).collect();
        map.links.push(e);
    }
    for (i, items) in built.outline {
        if i < shown {
            details.outline.insert(base + i, items.into_iter().take(400).collect());
        }
    }
    for (i, names) in built.outside {
        if i < shown {
            details.outside.insert(base + i, names.into_iter().map(String::into_bytes).collect());
        }
    }
    details.root = built.root.filter(|&r| r < shown).map(|r| base + r);
    details.unlinked.extend(built.unlinked);
    details.followed = built.followed;
    details.unknown = built.unknown;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(files: &[(&str, &str)]) -> BTreeMap<Vec<u8>, String> {
        files.iter().map(|(p, s)| (p.as_bytes().to_vec(), s.to_string())).collect()
    }
    fn links(built: &Built) -> Vec<(String, String, Vec<String>)> {
        let key = |i: usize| if built.units[i].key.is_empty() { "(root)".to_owned() } else { built.units[i].key.clone() };
        built.links.iter().map(|((a, b), names)| (key(*a), key(*b), names.iter().cloned().collect())).collect()
    }
    fn row(a: &str, b: &str, names: &[&str]) -> (String, String, Vec<String>) {
        (a.to_owned(), b.to_owned(), names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn javascript_files_and_folders_connect_by_relative_paths_and_aliases() {
        let files = texts(&[
            ("web/src/app.tsx", "import { card } from './view/card';\nimport * as ui from '@/view';\nimport data from './data.json';\nimport React from 'react';\nimport { x } from './missing';\nimport { y } from '../../other/lib.js';\nui.list();\n"),
            ("web/src/view/card.ts", "export function card() {}\nimport { helper } from './util.js';\n"),
            ("web/src/view/index.ts", "export * from './card';\n"),
            ("web/src/view/util.ts", "export const helper = 1;\n"),
        ]);
        let known: HashSet<&[u8]> = ["web/src/app.tsx", "web/src/view/card.ts", "web/src/view/index.ts", "web/src/view/util.ts", "web/src/data.json", "other/lib.js"].iter().map(|p| p.as_bytes()).collect();
        let aliases = packages::aliases(r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#, b"web");
        let built = javascript(b"web", &files, &known, &aliases, &CancellationToken::new()).unwrap();
        let keys: Vec<(&str, Option<&str>)> = built.units.iter().map(|u| (u.key.as_str(), u.parent.map(|p| built.units[p].key.as_str()))).collect();
        // Every file sits in web/src, so the project starts there.
        assert_eq!(keys, [("app.tsx", None), ("view", None), ("view/card.ts", Some("view")), ("view/index.ts", Some("view")), ("view/util.ts", Some("view"))]);
        assert_eq!(
            links(&built),
            [
                row("app.tsx", "view/card.ts", &["card"]),
                row("app.tsx", "view/index.ts", &["*", "list"]),
                row("view/card.ts", "view/util.ts", &["helper"]),
                row("view/index.ts", "view/card.ts", &["*"]),
            ]
        );
        let outside: Vec<Vec<String>> = built.outside.values().map(|s| s.iter().cloned().collect()).collect();
        assert_eq!(outside, [vec!["other/lib.js".to_owned(), "react".into()]]);
        assert_eq!((built.followed, built.unknown), (5, 1));
    }

    #[test]
    fn python_packages_resolve_absolute_relative_and_module_names() {
        let files = texts(&[
            ("Lib/email/__init__.py", "def message_from_string(s):\n    from email.parser import Parser\n"),
            ("Lib/email/errors.py", "class MessageError(Exception): pass\n"),
            ("Lib/email/parser.py", "from email.feedparser import FeedParser\nimport re\n"),
            ("Lib/email/feedparser.py", "from email import errors\nfrom email._policybase import compat32\nimport email.message as m\n"),
            ("Lib/email/_policybase.py", "compat32 = None\n"),
            ("Lib/email/message.py", "from . import errors\nfrom .missing import x\n"),
            ("Lib/email/mime/__init__.py", ""),
            ("Lib/email/mime/text.py", "from email.mime.nonmultipart import MIMENonMultipart\nfrom .. import errors\n"),
            ("Lib/email/mime/nonmultipart.py", "from email.mime import base\n"),
            ("Lib/email/mime/base.py", "import email.policy\n"),
        ]);
        let built = python(b"Lib/email", true, &files, &CancellationToken::new()).unwrap();
        assert_eq!(built.root, Some(0));
        let keys: Vec<&str> = built.units.iter().map(|u| u.key.as_str()).collect();
        // Folders first, then modules.
        assert_eq!(keys, ["", "mime", "_policybase", "errors", "feedparser", "message", "mime.base", "mime.nonmultipart", "mime.text", "parser"]);
        assert_eq!(built.units[1].detail, b"Python package");
        assert_eq!(
            links(&built),
            [
                row("(root)", "parser", &["Parser"]),
                row("feedparser", "_policybase", &["compat32"]),
                row("feedparser", "errors", &["errors"]),
                row("feedparser", "message", &["message"]),
                row("message", "errors", &["errors"]),
                row("mime.nonmultipart", "mime.base", &["base"]),
                row("mime.text", "errors", &["errors"]),
                row("mime.text", "mime.nonmultipart", &["MIMENonMultipart"]),
                row("parser", "feedparser", &["FeedParser"]),
            ]
        );
        // email.policy is not in the snapshot; .missing is not either.
        assert_eq!(built.unknown, 2);
        let outside: Vec<Vec<String>> = built.outside.values().map(|s| s.iter().cloned().collect()).collect();
        assert_eq!(outside, [vec!["re".to_owned()]]);
    }

    #[test]
    fn javascript_top_level_folders_base_url_and_python_names_shared_by_a_module_and_a_package() {
        // A project at the top of the repository: `require('..')` reaches index.js; baseUrl takes bare imports.
        let files = texts(&[("index.js", "module.exports = {};\n"), ("test/x.js", "require('..');\nimport B from 'components/Button';\n"), ("components/Button.js", "")]);
        let known: HashSet<&[u8]> = ["index.js", "test/x.js", "components/Button.js"].iter().map(|p| p.as_bytes()).collect();
        let aliases = packages::aliases(r#"{"compilerOptions":{"baseUrl":"."}}"#, b"");
        let built = javascript(b"", &files, &known, &aliases, &CancellationToken::new()).unwrap();
        assert_eq!(links(&built), [row("test/x.js", "components/Button.js", &["default"]), row("test/x.js", "index.js", &[])]);
        assert_eq!(built.unknown, 0);
        // a.py beside the package a/: the package keeps the dotted name and wins an import.
        let files = texts(&[("pkg/__init__.py", ""), ("pkg/a.py", ""), ("pkg/a/__init__.py", ""), ("pkg/b.py", "from pkg import a\n")]);
        let built = python(b"pkg", true, &files, &CancellationToken::new()).unwrap();
        let keys: Vec<&str> = built.units.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(keys, ["", "a", "a.py", "b"]);
        assert_eq!(links(&built), [row("b", "a", &["a"])]);
    }

    #[test]
    fn python_scripts_import_beside_themselves() {
        let files = texts(&[("tools/release.py", "import helpers\nimport os\nfrom lib import util\n"), ("tools/helpers.py", ""), ("tools/lib/util.py", "")]);
        let built = python(b"tools", false, &files, &CancellationToken::new()).unwrap();
        assert_eq!(built.root, None);
        assert_eq!(links(&built), [row("release", "helpers", &["helpers"]), row("release", "lib.util", &["util"])]);
    }

    #[test]
    fn java_packages_connect_by_imports_star_imports_and_names_written_out() {
        let files = texts(&[
            (
                "app/src/main/java/com/acme/app/App.java",
                "package com.acme.app;\nimport com.acme.app.store.Db;\nimport com.acme.app.web.*;\nimport java.util.List;\nimport static com.acme.app.util.Strings.trim;\nimport module java.base;\npublic class App { void run() { new Router(); Db.open(); com.acme.app.store.Cache.clear(); } }\n",
            ),
            ("app/src/main/java/com/acme/app/store/Db.java", "package com.acme.app.store;\npublic class Db { public static Db open() { return null; } }\n"),
            ("app/src/main/java/com/acme/app/store/Cache.java", "package com.acme.app.store;\npublic class Cache {}\n"),
            ("app/src/main/java/com/acme/app/web/Router.java", "package com.acme.app.web;\npublic class Router {}\nclass Hidden {}\n"),
            ("app/src/main/java/com/acme/app/web/api/Api.java", "package com.acme.app.web.api;\nimport com.acme.app.missing.Thing;\nimport com.acme.app.store.Nope;\nclass Api {}\n"),
            ("app/src/main/kotlin/com/acme/app/util/Strings.kt", "package com.acme.app.util\nobject Strings { fun trim(s: String) = s }\nfun helper() = 1\n"),
        ]);
        let built = java(&files, &[], &CancellationToken::new()).unwrap();
        let keys: Vec<(&str, &str, Option<&str>)> = built.units.iter().map(|u| (u.key.as_str(), std::str::from_utf8(&u.label).unwrap(), u.parent.map(|p| built.units[p].key.as_str()))).collect();
        // The list starts below com.acme.app, which every name shares; api sits in web.
        assert_eq!(
            keys,
            [
                ("com.acme.app", "app", None),
                ("com.acme.app.store", "store", None),
                ("com.acme.app.util", "util", None),
                ("com.acme.app.web", "web", None),
                ("com.acme.app.web.api", "web.api", Some("com.acme.app.web")),
            ]
        );
        assert_eq!(built.units[1].path, b"app/src/main/java/com/acme/app/store");
        assert_eq!(
            links(&built),
            [
                row("com.acme.app", "com.acme.app.store", &["Cache", "Db"]),
                row("com.acme.app", "com.acme.app.util", &["Strings"]),
                row("com.acme.app", "com.acme.app.web", &["Router"]),
                row("com.acme.app.web.api", "com.acme.app.store", &["Nope"]),
            ]
        );
        // Store has no Nope; com.acme.app.missing is no package here.
        assert_eq!((built.followed, built.unknown), (4, 1));
        let outside: Vec<Vec<String>> = built.outside.values().map(|s| s.iter().cloned().collect()).collect();
        assert_eq!(outside, [vec!["java.base".to_owned(), "java.util".into()], vec!["com.acme.app.missing".to_owned()]]);
        let web: Vec<(String, bool)> = built.outline[&3].iter().map(|o| (text(&o.name), o.exported)).collect();
        assert_eq!(web, [("Router".to_owned(), true), ("Hidden".into(), false)]);
        // A package of the project the read limits left unread is its own, not outside.
        let files = texts(&[("src/com/acme/App.java", "package com.acme;\nimport com.acme.big.Huge;\nclass App {}\n")]);
        let built = java(&files, &[b"src/com/acme/App.java".to_vec(), b"src/com/acme/big/Huge.java".to_vec()], &CancellationToken::new()).unwrap();
        assert!(built.outside.is_empty());
        assert_eq!((built.followed, built.unknown), (1, 1));
    }

    #[test]
    fn c_includes_beside_by_the_end_of_a_path_and_outside() {
        let files = texts(&[
            ("lib/include/acme/acme.h", "#ifndef ACME_H\n#define ACME_H\n#include \"acme/types.h\"\n#include <stdio.h>\nint acme_open(acme_handle *h);\n#endif\n"),
            ("lib/include/acme/types.h", "typedef struct acme_handle acme_handle;\nenum acme_mode { ACME_READ, ACME_WRITE };\n"),
            ("lib/src/acme.c", "#include \"acme/acme.h\"\n#include \"internal.h\"\n#include \"config.h\"\n#include \"../../other/shared.h\"\n#include \"table.inc\"\n#include <zlib.h>\nint acme_open(acme_handle *h) { return ACME_READ + helper(); }\n"),
            ("lib/src/internal.h", "static int helper(void) { return 0; }\n"),
        ]);
        let paths = ["lib/include/acme/acme.h", "lib/include/acme/types.h", "lib/src/acme.c", "lib/src/internal.h", "lib/src/table.inc", "other/shared.h", "vendor/zlib/zlib.h"];
        let known: HashSet<&[u8]> = paths.iter().map(|p| p.as_bytes()).collect();
        let headers = Headers::new(known.iter().copied());
        let own: Vec<Vec<u8>> = files.keys().cloned().collect();
        let built = c(b"lib", &files, &own, &known, &headers, &CancellationToken::new()).unwrap();
        let keys: Vec<(&str, &[u8])> = built.units.iter().map(|u| (u.key.as_str(), u.detail)).collect();
        assert_eq!(
            keys,
            [
                ("include", b"Folder".as_slice()),
                ("include/acme", b"Folder"),
                ("include/acme/acme.h", b"C header"),
                ("include/acme/types.h", b"C header"),
                ("src", b"Folder"),
                ("src/acme.c", b"C source"),
                ("src/internal.h", b"C header"),
            ]
        );
        assert_eq!(
            links(&built),
            [
                row("include/acme/acme.h", "include/acme/types.h", &["acme_handle"]),
                row("src/acme.c", "include/acme/acme.h", &["acme_open"]),
                row("src/acme.c", "src/internal.h", &["helper"]),
            ]
        );
        // config.h is no committed file; table.inc is no C file; <zlib.h> names no folder.
        assert_eq!((built.followed, built.unknown), (4, 1));
        let outside: Vec<Vec<String>> = built.outside.values().map(|s| s.iter().cloned().collect()).collect();
        assert_eq!(outside, [vec!["stdio.h".to_owned()], vec!["other/shared.h".to_owned(), "zlib.h".into()]]);
        let rows: Vec<(String, bool)> = built.outline[&3].iter().map(|o| (text(&o.name), o.exported)).collect();
        assert_eq!(rows, [("acme_handle".to_owned(), true), ("acme_mode".into(), true)]);
    }

    #[test]
    fn c_loose_files_beside_another_project_and_a_header_naming_itself() {
        // Loose files in tools/ and scripts/ make a project at the top; c/ is a CMake project.
        let files = texts(&[
            ("tools/a.c", "#include \"cdemo_api.h\"\nint run(void) { return cdemo_open(); }\n"),
            ("scripts/b.c", "int b(void) { return 0; }\n"),
            ("compat/stdint.h", "#include_next <stdint.h>\ntypedef long intmax_t;\n"),
        ]);
        let paths = ["tools/a.c", "scripts/b.c", "compat/stdint.h", "c/CMakeLists.txt", "c/include/cdemo_api.h"];
        let known: HashSet<&[u8]> = paths.iter().map(|p| p.as_bytes()).collect();
        let headers = Headers::new(known.iter().copied());
        let own: Vec<Vec<u8>> = files.keys().cloned().collect();
        let built = c(b"", &files, &own, &known, &headers, &CancellationToken::new()).unwrap();
        // The other project's header is outside this one, not one of its unread files; a
        // header never includes itself.
        assert!(built.links.is_empty(), "{:?}", links(&built));
        let outside: BTreeMap<String, Vec<String>> = built.outside.iter().map(|(u, s)| (built.units[*u].key.clone(), s.iter().cloned().collect())).collect();
        assert_eq!(outside, BTreeMap::from([("compat/stdint.h".to_owned(), vec!["stdint.h".to_owned()]), ("tools/a.c".into(), vec!["c/include/cdemo_api.h".to_owned()])]));
        assert_eq!((built.followed, built.unknown), (0, 0));
    }

    #[test]
    fn csharp_namespaces_connect_by_using_lines_parents_and_type_names() {
        let files = texts(&[
            (
                "Web/Controllers/HomeController.cs",
                "using Acme.Core.Models;\nusing static Acme.Core.Util.Strings;\nusing Microsoft.AspNetCore.Mvc;\nnamespace Acme.Web.Controllers;\npublic class HomeController : Controller { public string Index() => Trim(new User().Name) + Helper.Format(); }\n",
            ),
            ("Web/Helper.cs", "namespace Acme.Web;\npublic static class Helper { public static string Format() => \"\"; }\n"),
            ("Core/Models/User.cs", "namespace Acme.Core.Models { public class User { public string Name { get; set; } } public partial class Order { void A() {} } }\n"),
            ("Core/Models/Order.cs", "namespace Acme.Core.Models { public partial class Order { void B() {} void C() {} } }\n"),
            ("Core/Util/Strings.cs", "namespace Acme.Core.Util;\npublic static class Strings { public static string Trim(string s) => s; }\n"),
            ("GlobalUsings.cs", "global using Acme.Core.Models;\nglobal using System;\n"),
            ("Web/Pages/Index.cs", "namespace Acme.Web.Pages;\nclass IndexPage { User u; Acme.Core.Util.Strings s; }\n"),
        ]);
        let built = csharp(&files, &CancellationToken::new()).unwrap();
        let keys: Vec<(&str, &str, bool, Option<&str>)> = built.units.iter().map(|u| (u.key.as_str(), std::str::from_utf8(&u.label).unwrap(), u.folder, u.parent.map(|p| built.units[p].key.as_str()))).collect();
        assert_eq!(
            keys,
            [
                ("", "(global namespace)", false, None),
                ("Acme.Core.Models", "Core.Models", false, Some("Acme.Core")),
                ("Acme.Core.Util", "Core.Util", false, Some("Acme.Core")),
                ("Acme.Web", "Web", false, None),
                ("Acme.Web.Controllers", "Web.Controllers", false, Some("Acme.Web")),
                ("Acme.Web.Pages", "Web.Pages", false, Some("Acme.Web")),
                ("Acme.Core", "Core", true, None),
            ]
        );
        assert_eq!(
            links(&built),
            [
                // GlobalUsings.cs's own line; the others only through the names they use.
                row("(root)", "Acme.Core.Models", &[]),
                row("Acme.Web.Controllers", "Acme.Core.Models", &["User"]),
                row("Acme.Web.Controllers", "Acme.Core.Util", &["Strings"]),
                row("Acme.Web.Controllers", "Acme.Web", &["Helper"]),
                row("Acme.Web.Pages", "Acme.Core.Models", &["User"]),
                row("Acme.Web.Pages", "Acme.Core.Util", &["Strings"]),
            ]
        );
        let outside: BTreeMap<String, Vec<String>> = built.outside.iter().map(|(u, s)| (built.units[*u].key.clone(), s.iter().cloned().collect())).collect();
        assert_eq!(outside, BTreeMap::from([("".to_owned(), vec!["System".to_owned()]), ("Acme.Web.Controllers".into(), vec!["Microsoft.AspNetCore.Mvc".to_owned()])]));
        // A partial class's methods add up across its files (listed by file: Order.cs first).
        let models: Vec<(String, usize)> = built.outline[&1].iter().map(|o| (text(&o.name), o.functions)).collect();
        assert_eq!(models, [("Order".to_owned(), 3), ("User".into(), 0)]);
    }

    #[test]
    fn go_packages_stand_alone_with_folders_between() {
        let files = texts(&[
            ("src/net/http/client.go", "package http\nimport (\n\t\"io\"\n\t\"net/url\"\n\t\"golang.org/x/net/idna\"\n)\nfunc Get() { io.Copy(); url.Parse(); url.parse() }\ntype Client struct{}\nfunc (c *Client) Do() {}\n"),
            ("src/net/http/server.go", "package http\nimport \"io\"\nfunc (c *Client) Close() { io.ReadAll() }\n"),
            ("src/net/http/triv.go", "//go:build ignore\n\npackage main\nimport \"net/http\"\n"),
            ("src/net/url/url.go", "package url\nfunc Parse() {}\n"),
            ("src/io/io.go", "package io\nfunc Copy() {}\n"),
            ("src/archive/tar/reader.go", "package tar\nimport \"io\"\nvar _ = io.EOF\n"),
        ]);
        let built = go(b"src", "std", &files, &[], &CancellationToken::new()).unwrap();
        let keys: Vec<(&str, bool, Option<&str>)> = built.units.iter().map(|u| (u.key.as_str(), u.folder, u.parent.map(|p| built.units[p].key.as_str()))).collect();
        assert_eq!(keys, [("archive/tar", false, Some("archive")), ("io", false, None), ("net/http", false, Some("net")), ("net/url", false, Some("net")), ("archive", true, None), ("net", true, None)]);
        assert_eq!(links(&built), [row("archive/tar", "io", &["EOF"]), row("net/http", "io", &["Copy", "ReadAll"]), row("net/http", "net/url", &["Parse"])]);
        assert_eq!(built.unlinked, [(b"src/net/http/triv.go".to_vec(), UnlinkedReason::BuildIgnore)]);
        let http = built.outline.get(&2).unwrap();
        let rows: Vec<(String, usize)> = http.iter().map(|o| (text(&o.name), o.functions)).collect();
        assert_eq!(rows, [("Get".to_owned(), 0), ("Client".to_owned(), 2)]);
        assert_eq!(built.outside.get(&2).map(|s| s.iter().cloned().collect::<Vec<_>>()), Some(vec!["golang.org/x/net/idna".to_owned()]));
        // A module with its own path: imports start with it.
        let files = texts(&[("app/main.go", "package main\nimport \"example.com/app/store\"\nfunc main() { store.Open() }\n"), ("app/store/db.go", "package store\nfunc Open() {}\n")]);
        let built = go(b"app", "example.com/app", &files, &[], &CancellationToken::new()).unwrap();
        assert_eq!(links(&built), [row("(root)", "store", &["Open"])]);
        assert_eq!(built.units[0].label, b"main");
        // A std package the read limits left unread is the module's own: unresolved, not outside.
        let files = texts(&[("src/net/url/url.go", "package url\nimport (\n\t\"unicode/utf8\"\n\t\"C\"\n)\n")]);
        let module_files = [b"src/net/url/url.go".to_vec(), b"src/unicode/utf8/utf8.go".to_vec()];
        let built = go(b"src", "std", &files, &module_files, &CancellationToken::new()).unwrap();
        assert_eq!(built.outside.values().flatten().cloned().collect::<Vec<_>>(), ["C".to_owned()]);
        assert_eq!((built.followed, built.unknown), (1, 1));
        // A nested module (its own go.mod, so none of these files) is outside this one.
        let files = texts(&[("app/main.go", "package main\nimport \"example.com/app/tools/gen\"\n")]);
        let built = go(b"app", "example.com/app", &files, &[b"app/main.go".to_vec()], &CancellationToken::new()).unwrap();
        assert_eq!(built.outside.values().flatten().cloned().collect::<Vec<_>>(), ["example.com/app/tools/gen".to_owned()]);
        assert_eq!(built.unknown, 0);
    }
}
