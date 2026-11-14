//! The packages Modules reads besides Cargo's, found from the snapshot's paths alone:
//! JavaScript and TypeScript projects (a folder with package.json, else a top-level
//! folder), Python packages (a folder with `__init__.py` no other package holds, else a
//! top-level folder of scripts) and Go modules (a folder with go.mod). A small JSON
//! reader takes package.json's name and tsconfig's paths; go.mod's module line is read
//! as text. Nothing is run.
use super::ModuleLanguage;
use std::collections::{BTreeMap, HashSet};

/// Packages one language may list in one snapshot.
pub(super) const PACKAGES: usize = 128;

/// A package found by its paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub folder: Vec<u8>,
    pub language: ModuleLanguage,
    /// Its package.json, `__init__.py` or go.mod; None for a folder that has none.
    pub manifest: Option<Vec<u8>>,
    /// A JavaScript project with TypeScript files or a tsconfig.json.
    pub typescript: bool,
    /// Its source files, sorted.
    pub files: Vec<Vec<u8>>,
}

pub(super) fn file_name(path: &[u8]) -> &[u8] {
    path.rsplit(|b| *b == b'/').next().unwrap_or_default()
}
/// The folder a path is in; empty at the top.
pub(super) fn parent(path: &[u8]) -> &[u8] {
    path.rsplitn(2, |b| *b == b'/').nth(1).unwrap_or_default()
}
pub(super) fn join(base: &[u8], part: &[u8]) -> Vec<u8> {
    if base.is_empty() {
        part.to_vec()
    } else {
        [base, b"/", part].concat()
    }
}
/// Whether `path` is `folder` or inside it; everything is inside the top folder.
pub(super) fn inside(path: &[u8], folder: &[u8]) -> bool {
    folder.is_empty() || path == folder || path.len() > folder.len() && path.starts_with(folder) && path[folder.len()] == b'/'
}
/// `path` from inside `folder`.
pub(super) fn within<'a>(path: &'a [u8], folder: &[u8]) -> &'a [u8] {
    if folder.is_empty() {
        path
    } else {
        path.get(folder.len() + 1..).unwrap_or_default()
    }
}

const JAVASCRIPT: [&[u8]; 4] = [b".js", b".mjs", b".cjs", b".jsx"];
const TYPESCRIPT: [&[u8]; 4] = [b".ts", b".mts", b".cts", b".tsx"];
pub(super) fn javascript_file(path: &[u8]) -> bool {
    JAVASCRIPT.iter().chain(&TYPESCRIPT).any(|e| path.ends_with(e)) && file_name(path).len() > 3
}
pub(super) fn typescript_file(path: &[u8]) -> bool {
    TYPESCRIPT.iter().any(|e| path.ends_with(e))
}
pub(super) fn python_file(path: &[u8]) -> bool {
    path.ends_with(b".py") && file_name(path).len() > 3
}
/// A Go file a build may take: test files are left out, as `go build` leaves them.
pub(super) fn go_file(path: &[u8]) -> bool {
    path.ends_with(b".go") && !path.ends_with(b"_test.go") && file_name(path).len() > 3
}

/// Folders a language's own tools pass over: hidden ones and installed packages, and
/// for Go `testdata`, `vendor` and names starting with `_`.
fn skipped(path: &[u8], language: ModuleLanguage) -> bool {
    let folders = path.split(|b| *b == b'/').collect::<Vec<_>>();
    folders[..folders.len().saturating_sub(1)].iter().any(|f| {
        f.starts_with(b".")
            || *f == b"node_modules"
            // A committed virtual environment holds installed packages, not the project's.
            || language == ModuleLanguage::Python && matches!(*f, b"__pycache__" | b"site-packages" | b"dist-packages" | b"__pypackages__")
            || language == ModuleLanguage::Go && (*f == b"testdata" || *f == b"vendor" || f.starts_with(b"_"))
    })
}

/// The nearest of `folders` that holds `path`.
fn nearest<'a>(path: &[u8], folders: &HashSet<&'a [u8]>) -> Option<&'a [u8]> {
    let mut folder = parent(path);
    for _ in 0..64 {
        if let Some(found) = folders.get(folder) {
            return Some(*found);
        }
        if folder.is_empty() {
            return None;
        }
        folder = parent(folder);
    }
    None
}
/// A file's top-level folder; empty for a file at the top.
fn top_folder(path: &[u8]) -> &[u8] {
    match path.iter().position(|b| *b == b'/') {
        Some(at) => &path[..at],
        None => b"",
    }
}

/// The packages `paths` hold, by language and folder, and whether some were left out
/// past [`PACKAGES`].
pub(super) fn find(paths: &[&[u8]]) -> (Vec<Found>, bool) {
    let mut out = Vec::new();
    let mut limited = false;
    let mut add = |mut list: Vec<Found>, out: &mut Vec<Found>| {
        list.retain(|f| !f.files.is_empty());
        list.sort_by(|a, b| a.folder.cmp(&b.folder));
        limited |= list.len() > PACKAGES;
        out.extend(list.into_iter().take(PACKAGES));
    };
    add(javascript(paths), &mut out);
    add(python(paths), &mut out);
    add(go(paths), &mut out);
    (out, limited)
}

/// Each package.json's folder is a project; files no project holds make one project
/// per top-level folder.
fn javascript(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::JavaScript;
    let manifests: HashSet<&[u8]> = paths.iter().filter(|p| file_name(p) == b"package.json" && !skipped(p, language)).map(|p| parent(p)).collect();
    let configs: HashSet<&[u8]> = paths
        .iter()
        .filter(|p| matches!(file_name(p), b"tsconfig.json" | b"jsconfig.json") && !skipped(p, language))
        .map(|p| parent(p))
        .collect();
    let mut projects: BTreeMap<(Vec<u8>, bool), Found> = BTreeMap::new();
    for path in paths.iter().filter(|p| javascript_file(p) && !skipped(p, language)) {
        let (folder, manifest) = match nearest(path, &manifests) {
            Some(folder) => (folder.to_vec(), true),
            None => (top_folder(path).to_vec(), false),
        };
        let found = projects.entry((folder.clone(), manifest)).or_insert_with(|| Found {
            manifest: manifest.then(|| join(&folder, b"package.json")),
            typescript: false,
            files: Vec::new(),
            folder,
            language,
        });
        found.typescript |= typescript_file(path);
        found.files.push(path.to_vec());
    }
    let mut list: Vec<Found> = projects.into_values().collect();
    for found in &mut list {
        found.typescript |= configs.contains(found.folder.as_slice());
        found.files.sort();
    }
    // A folder with no package.json that holds one deeper down is still its own project:
    // the deeper files belong to theirs.
    list
}

/// A folder with `__init__.py` whose parent has none is a package; files no package
/// holds make one folder of scripts per top-level folder.
fn python(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::Python;
    let packages: HashSet<&[u8]> = paths.iter().filter(|p| file_name(p) == b"__init__.py" && !skipped(p, language)).map(|p| parent(p)).collect();
    // A stray __init__.py at the top of the repository makes no package of everything.
    let tops: HashSet<&[u8]> = packages.iter().copied().filter(|f| !f.is_empty() && (parent(f).is_empty() || !packages.contains(parent(f)))).collect();
    let mut found: BTreeMap<(Vec<u8>, bool), Found> = BTreeMap::new();
    for path in paths.iter().filter(|p| python_file(p) && !skipped(p, language)) {
        let (folder, manifest) = match nearest(path, &tops) {
            Some(folder) => (folder.to_vec(), true),
            None => (top_folder(path).to_vec(), false),
        };
        found
            .entry((folder.clone(), manifest))
            .or_insert_with(|| Found {
                manifest: manifest.then(|| join(&folder, b"__init__.py")),
                typescript: false,
                files: Vec::new(),
                folder,
                language,
            })
            .files
            .push(path.to_vec());
    }
    let mut list: Vec<Found> = found.into_values().collect();
    for f in &mut list {
        f.files.sort();
    }
    list
}

/// Each go.mod's folder is a module holding the Go files below it that no deeper
/// module holds. Go files outside every module are not read.
fn go(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::Go;
    let modules: HashSet<&[u8]> = paths.iter().filter(|p| file_name(p) == b"go.mod" && !skipped(p, language)).map(|p| parent(p)).collect();
    let mut found: BTreeMap<Vec<u8>, Found> = BTreeMap::new();
    for path in paths.iter().filter(|p| go_file(p) && !skipped(p, language)) {
        let Some(folder) = nearest(path, &modules) else { continue };
        found
            .entry(folder.to_vec())
            .or_insert_with(|| Found {
                folder: folder.to_vec(),
                language,
                manifest: Some(join(folder, b"go.mod")),
                typescript: false,
                files: Vec::new(),
            })
            .files
            .push(path.to_vec());
    }
    let mut list: Vec<Found> = found.into_values().collect();
    for f in &mut list {
        f.files.sort();
    }
    list
}

// ---- go.mod and JSON ---------------------------------------------------------------

/// go.mod's `module` path, quoted or not.
pub(super) fn go_module(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let Some(rest) = line.strip_prefix("module") else { continue };
        if !rest.starts_with(char::is_whitespace) && !rest.starts_with('"') {
            continue;
        }
        let path = rest.trim().trim_matches('"').trim_matches('`');
        return (!path.is_empty() && !path.contains(char::is_whitespace)).then(|| path.to_owned());
    }
    None
}

/// A JSON value as far as package.json and tsconfig.json need one.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Json {
    Object(Vec<(String, Json)>),
    Array(Vec<Json>),
    Str(String),
    Other,
}
impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}
/// Reads JSON, with tsconfig's comments and trailing commas; None when it is not.
pub(super) fn json(text: &str) -> Option<Json> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let value = json_value(&chars, &mut at, 0)?;
    skip_space(&chars, &mut at);
    (at >= chars.len()).then_some(value)
}
fn skip_space(c: &[char], at: &mut usize) {
    loop {
        while c.get(*at).is_some_and(|x| x.is_whitespace() || *x == '\u{feff}') {
            *at += 1;
        }
        if c.get(*at) == Some(&'/') && c.get(*at + 1) == Some(&'/') {
            while *at < c.len() && c[*at] != '\n' {
                *at += 1;
            }
        } else if c.get(*at) == Some(&'/') && c.get(*at + 1) == Some(&'*') {
            *at += 2;
            while *at < c.len() && !(c[*at] == '*' && c.get(*at + 1) == Some(&'/')) {
                *at += 1;
            }
            *at = (*at + 2).min(c.len());
        } else {
            return;
        }
    }
}
fn json_string(c: &[char], at: &mut usize) -> Option<String> {
    let mut out = String::new();
    *at += 1;
    while *at < c.len() {
        match c[*at] {
            '"' => {
                *at += 1;
                return Some(out);
            }
            '\\' => {
                let escaped = *c.get(*at + 1)?;
                *at += 2;
                match escaped {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' | 'f' => {}
                    'u' => {
                        let hex: String = c.get(*at..*at + 4)?.iter().collect();
                        out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'));
                        *at += 4;
                    }
                    other => out.push(other),
                }
            }
            x => {
                out.push(x);
                *at += 1;
            }
        }
    }
    None
}
fn json_value(c: &[char], at: &mut usize, depth: usize) -> Option<Json> {
    if depth > 64 {
        return None;
    }
    skip_space(c, at);
    match *c.get(*at)? {
        '{' => {
            *at += 1;
            let mut fields = Vec::new();
            loop {
                skip_space(c, at);
                match *c.get(*at)? {
                    '}' => {
                        *at += 1;
                        return Some(Json::Object(fields));
                    }
                    '"' => {
                        let key = json_string(c, at)?;
                        skip_space(c, at);
                        if c.get(*at) != Some(&':') {
                            return None;
                        }
                        *at += 1;
                        fields.push((key, json_value(c, at, depth + 1)?));
                        skip_space(c, at);
                        match *c.get(*at)? {
                            ',' => *at += 1,
                            '}' => {}
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
        }
        '[' => {
            *at += 1;
            let mut items = Vec::new();
            loop {
                skip_space(c, at);
                if c.get(*at) == Some(&']') {
                    *at += 1;
                    return Some(Json::Array(items));
                }
                items.push(json_value(c, at, depth + 1)?);
                skip_space(c, at);
                match *c.get(*at)? {
                    ',' => *at += 1,
                    ']' => {}
                    _ => return None,
                }
            }
        }
        '"' => json_string(c, at).map(Json::Str),
        _ => {
            // Numbers, true, false, null.
            let start = *at;
            while c.get(*at).is_some_and(|x| x.is_alphanumeric() || matches!(x, '-' | '+' | '.')) {
                *at += 1;
            }
            (*at > start).then_some(Json::Other)
        }
    }
}

/// tsconfig.json's (or jsconfig.json's) `compilerOptions.baseUrl` and `paths`: where a
/// bare import such as `@/view` looks, relative to the repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Aliases {
    /// Each pattern (at most one `*`) and the places it stands for, in order.
    pub paths: Vec<(String, Vec<Vec<u8>>)>,
    /// `baseUrl`: a bare import that no pattern takes is looked for here first
    /// (`components/Button` in `src`), as TypeScript does.
    pub base: Option<Vec<u8>>,
}
pub(super) fn aliases(text: &str, folder: &[u8]) -> Aliases {
    let Some(config) = json(text) else { return Aliases::default() };
    let Some(options) = config.get("compilerOptions") else { return Aliases::default() };
    let base_url = options.get("baseUrl").and_then(Json::str).map(|url| super::cargo::relative(folder, url.trim_end_matches('/').as_bytes()).unwrap_or_else(|| folder.to_vec()));
    let base = base_url.clone().unwrap_or_else(|| folder.to_vec());
    let mut paths = Vec::new();
    if let Some(Json::Object(fields)) = options.get("paths") {
        for (pattern, targets) in fields.iter().take(256) {
            if pattern.matches('*').count() > 1 {
                continue;
            }
            let Json::Array(list) = targets else { continue };
            let places = list
                .iter()
                .filter_map(Json::str)
                .filter(|t| t.matches('*').count() <= 1)
                .filter_map(|t| super::cargo::relative(&base, t.as_bytes()))
                .collect::<Vec<_>>();
            if !places.is_empty() {
                paths.push((pattern.clone(), places));
            }
        }
    }
    // TypeScript tries the pattern with the longest prefix first.
    paths.sort_by_key(|(p, _)| std::cmp::Reverse(p.split('*').next().unwrap_or_default().len()));
    Aliases { paths, base: base_url }
}
impl Aliases {
    /// The places an import may be, by the first pattern it matches.
    pub fn places(&self, spec: &str) -> Option<Vec<Vec<u8>>> {
        for (pattern, places) in &self.paths {
            let star = match pattern.split_once('*') {
                Some((before, after)) => {
                    if spec.len() < before.len() + after.len() || !spec.starts_with(before) || !spec.ends_with(after) {
                        continue;
                    }
                    Some(&spec[before.len()..spec.len() - after.len()])
                }
                None if spec == pattern => None,
                None => continue,
            };
            return Some(
                places
                    .iter()
                    .map(|place| match star {
                        Some(s) => {
                            let text = String::from_utf8_lossy(place);
                            text.replacen('*', s, 1).into_bytes()
                        }
                        None => place.clone(),
                    })
                    .collect(),
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(paths: &[&str]) -> Vec<(String, ModuleLanguage, bool, bool, Vec<String>)> {
        let bytes: Vec<&[u8]> = paths.iter().map(|p| p.as_bytes()).collect();
        let (list, limited) = find(&bytes);
        assert!(!limited);
        list.into_iter()
            .map(|f| {
                (
                    String::from_utf8(f.folder).unwrap(),
                    f.language,
                    f.manifest.is_some(),
                    f.typescript,
                    f.files.into_iter().map(|p| String::from_utf8(p).unwrap()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn javascript_projects_are_package_json_folders_else_top_folders() {
        let list = found(&[
            "web/package.json",
            "web/src/app.tsx",
            "web/src/util.js",
            "web/node_modules/lib/index.js",
            "web/packages/core/package.json",
            "web/packages/core/index.ts",
            "tools/a.cjs",
            "tools/deep/b.mjs",
            "build.js",
            ".github/x.js",
            "docs/site.js",
            "docs/notes.md",
            "README.md",
        ]);
        let js: Vec<_> = list.iter().map(|(f, _, m, t, files)| (f.as_str(), *m, *t, files.len())).collect();
        assert_eq!(js, [("", false, false, 1), ("docs", false, false, 1), ("tools", false, false, 2), ("web", true, true, 2), ("web/packages/core", true, true, 1)]);
    }

    #[test]
    fn python_packages_and_folders_of_scripts() {
        let list = found(&[
            "src/email/__init__.py",
            "src/email/utils.py",
            "src/email/mime/__init__.py",
            "src/email/mime/text.py",
            "src/email/tests/test_a.py",
            "src/email/__pycache__/utils.cpython-314.py",
            "tools/release.py",
            "setup.py",
            "venv/Lib/site-packages/requests/__init__.py",
        ]);
        let py: Vec<_> = list.iter().map(|(f, l, m, _, files)| (f.as_str(), *l, *m, files.clone())).collect();
        assert_eq!(
            py,
            [
                ("", ModuleLanguage::Python, false, vec!["setup.py".to_owned()]),
                ("src/email", ModuleLanguage::Python, true, vec!["src/email/__init__.py".into(), "src/email/mime/__init__.py".into(), "src/email/mime/text.py".into(), "src/email/tests/test_a.py".into(), "src/email/utils.py".into()]),
                ("tools", ModuleLanguage::Python, false, vec!["tools/release.py".to_owned()]),
            ]
        );
    }

    #[test]
    fn a_stray_init_at_the_top_leaves_the_packages_below_it() {
        let list = found(&["__init__.py", "email/__init__.py", "email/a.py"]);
        let py: Vec<_> = list.iter().map(|(f, _, m, _, files)| (f.as_str(), *m, files.len())).collect();
        assert_eq!(py, [("", false, 1), ("email", true, 2)]);
    }

    #[test]
    fn go_modules_hold_their_files_without_tests_vendor_or_testdata() {
        let list = found(&["go.mod", "main.go", "net/http/client.go", "net/http/client_test.go", "net/http/testdata/x.go", "vendor/golang.org/x/y.go", "_tools/gen.go", "tools/go.mod", "tools/gen.go", "loose/a.go"]);
        let go: Vec<_> = list.iter().map(|(f, _, _, _, files)| (f.as_str(), files.clone())).collect();
        assert_eq!(
            go,
            [("", vec!["loose/a.go".to_owned(), "main.go".into(), "net/http/client.go".into()]), ("tools", vec!["tools/gen.go".to_owned()])]
        );
    }

    #[test]
    fn go_mod_and_tsconfig_are_read_as_text() {
        assert_eq!(go_module("// comment\nmodule github.com/acme/app // trailing\n\ngo 1.22\n").as_deref(), Some("github.com/acme/app"));
        assert_eq!(go_module("module \"std\"\n").as_deref(), Some("std"));
        assert_eq!(go_module("modules x\n"), None);
        let config = r#"{
            // comments and trailing commas, as tsconfig allows
            "compilerOptions": {
                "baseUrl": "./src",
                "paths": { "@/*": ["*", "generated/*",], "~lib": ["../lib/index.ts"], "x*y*": ["no"] },
            },
        }"#;
        let a = aliases(config, b"web");
        assert_eq!(a.places("@/view/card"), Some(vec![b"web/src/view/card".to_vec(), b"web/src/generated/view/card".to_vec()]));
        assert_eq!(a.places("~lib"), Some(vec![b"web/lib/index.ts".to_vec()]));
        assert_eq!(a.places("react"), None);
        assert_eq!(a.base.as_deref(), Some(b"web/src".as_slice()));
        // baseUrl alone: no patterns, a place for bare imports.
        let only = aliases(r#"{"compilerOptions":{"baseUrl":"src"}}"#, b"app");
        assert!(only.paths.is_empty());
        assert_eq!(only.base.as_deref(), Some(b"app/src".as_slice()));
        assert_eq!(aliases(r#"{"compilerOptions":{}}"#, b"app").base, None);
        assert_eq!(json(r#"{"name": "@acme/web", "version": "1.0.0", "x": [1, true, null, "é"]}"#).and_then(|j| j.get("name").and_then(Json::str).map(str::to_owned)).as_deref(), Some("@acme/web"));
        assert_eq!(json("{\"name\": "), None);
    }
}
