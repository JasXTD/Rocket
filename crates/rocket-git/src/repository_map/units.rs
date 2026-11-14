//! The module tree of a JavaScript, Python or Go package, as Modules shows it: units
//! (files, folders, Go packages), which unit uses which and through which names, what
//! each declares and what each imports from outside the package. Rust's tree comes
//! from its own module rules (modules.rs).
use super::packages::{Aliases, file_name, inside, join, parent, within};
use super::*;
use std::collections::{HashMap, HashSet};

/// Names kept per connection.
const NAMES: usize = 64;

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
    /// A Go folder that holds packages and is none itself: listed, never drawn.
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
        if set.len() < 256 {
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
        if let Some(&at) = self.at.get(folder) {
            return Some(at);
        }
        let up = self.unit(built, base, parent(folder), top, key);
        let at = built.add(Unit {
            key: key(within(folder, base)),
            label: file_name(folder).to_vec(),
            detail: b"Folder",
            path: folder.to_vec(),
            parent: up,
            folder: false,
            files: 0,
            lines: 0,
        });
        self.at.insert(folder.to_vec(), at);
        Some(at)
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

/// A JavaScript project: one unit per file and per folder; imports by relative path,
/// tsconfig paths, else from outside.
pub(super) fn javascript(folder: &[u8], texts: &BTreeMap<Vec<u8>, String>, known: &HashSet<&[u8]>, aliases: &Aliases, cancel: &CancellationToken) -> Result<Built> {
    let mut built = Built::default();
    let mut folders = Folders { at: HashMap::new() };
    let mut file_unit: HashMap<&[u8], usize> = HashMap::new();
    let key = |rel: &[u8]| text(rel);
    let base = shared_base(folder, texts.keys());
    for (path, source) in texts {
        let up = folders.unit(&mut built, &base, parent(path), None, &key);
        let at = built.add(Unit {
            key: text(within(path, &base)),
            label: file_name(path).to_vec(),
            detail: if packages::typescript_file(path) { b"TypeScript module" } else { b"JavaScript module" },
            path: path.clone(),
            parent: up,
            folder: false,
            files: 1,
            lines: lines(source),
        });
        file_unit.insert(path.as_slice(), at);
    }
    // A name two of the project's files or folders share (index.js) shows its path.
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    for u in &built.units {
        *seen.entry(u.label.clone()).or_default() += 1;
    }
    for u in &mut built.units {
        if seen.get(&u.label).copied().unwrap_or(0) > 1 {
            u.label = u.key.clone().into_bytes();
        }
    }
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

/// Puts a package's units on the map as nodes after the package nodes, with their
/// connections, outline and outside imports.
pub(super) fn push(map: &mut RepositoryMap, details: &mut ModuleMap, built: Built) {
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
