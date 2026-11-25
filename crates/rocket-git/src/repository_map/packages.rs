//! The packages Modules reads besides Cargo's, found from the snapshot's paths alone:
//! JavaScript and TypeScript projects (a folder with package.json, else a top-level
//! folder), Python packages (a folder with `__init__.py` no other package holds, else a
//! top-level folder of scripts), Go modules (a folder with go.mod), Java and Kotlin
//! projects (a folder with a Maven or Gradle build, else a top-level folder), C and C++
//! projects (the top-most folder with a CMake, Meson, Make or similar build, else one for
//! the files no build holds) and C# projects (a folder with a .csproj, else a top-level
//! folder). A small JSON reader takes package.json's name and tsconfig's paths; go.mod's
//! module line and the build files' project names are read as text. Nothing is run.
use super::{Dialect, ModuleLanguage};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Packages one language may list in one snapshot.
pub(super) const PACKAGES: usize = 128;

/// A package found by its paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub folder: Vec<u8>,
    pub language: ModuleLanguage,
    /// Its package.json, `__init__.py` or go.mod; a Maven or Gradle build (the one that
    /// names the project when there is one), a C or C++ build (CMakeLists.txt or
    /// meson.build first) or a .csproj; None for a folder that has none.
    pub manifest: Option<Vec<u8>>,
    /// TypeScript, Kotlin or C++ beside the language's own files.
    pub dialect: Option<Dialect>,
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
/// Returns the file's original extension, or an empty slice when it has none.
fn extension(path: &[u8]) -> &[u8] {
    let name = file_name(path);
    match name.iter().rposition(|b| *b == b'.') {
        Some(at) if at > 0 => &name[at + 1..],
        _ => b"",
    }
}
pub(super) fn kotlin_file(path: &[u8]) -> bool {
    path.ends_with(b".kt") && file_name(path).len() > 3
}

/// Match whole words in source-set names such as `testFixtures`, `androidTestDebug` and
/// `integration-test`. The last letters of a production name such as `latest` are part of that word.
fn test_source_set(name: &[u8]) -> bool {
    name.split(|b| b.is_ascii() && !b.is_ascii_alphabetic()).any(|part| {
        let mut start = 0;
        for end in 1..=part.len() {
            // A capital can also start a word at the end of an acronym, as in JVMTest.
            if end == part.len() || part[end].is_ascii_uppercase()
                && (part[end - 1].is_ascii_lowercase() || part.get(end + 1).is_some_and(u8::is_ascii_lowercase))
            {
                let word = &part[start..end];
                if word.eq_ignore_ascii_case(b"test") || word.eq_ignore_ascii_case(b"tests") {
                    return true;
                }
                start = end;
            }
        }
        false
    })
}

/// A Java or Kotlin file of a build's main code. Test source sets (`src/test`,
/// `src/androidTest`, `src/jvmTest` and the like) are left out, as Go's tests are: their
/// classes share the packages of the code they test. So are module-info.java, which
/// names a module and no package, and Kotlin scripts.
pub(super) fn java_file(path: &[u8]) -> bool {
    let name = file_name(path);
    if !(path.ends_with(b".java") && name.len() > 5 || kotlin_file(path)) || name == b"module-info.java" {
        return false;
    }
    let mut after_src = false;
    for folder in parent(path).split(|b| *b == b'/') {
        if after_src && test_source_set(folder) {
            return false;
        }
        after_src = folder == b"src";
    }
    true
}
/// A C or C++ source file or header; `.inl` holds inline code in both.
pub(super) fn c_file(path: &[u8]) -> bool {
    c_extension(extension(path))
}
/// Source and header extensions shared by the Code and Modules views.
pub(super) fn c_extension(ext: &[u8]) -> bool {
    ext.eq_ignore_ascii_case(b"c") || ext.eq_ignore_ascii_case(b"h") || ext.eq_ignore_ascii_case(b"inl") || cpp_extension(ext)
}
/// A file only C++ has.
pub(super) fn cpp_file(path: &[u8]) -> bool {
    cpp_extension(extension(path))
}
fn cpp_extension(ext: &[u8]) -> bool {
    const EXTENSIONS: &[&[u8]] = &[
        b"cc", b"cpp", b"cxx", b"c++", b"hh", b"hpp", b"hxx", b"h++", b"ipp", b"tpp",
        b"cppm", b"ixx", b"mpp", b"ccm", b"cxxm", b"c++m",
    ];
    // Uppercase .C is C++ even when it is the only source in a project.
    ext == b"C" || EXTENSIONS.iter().any(|other| ext.eq_ignore_ascii_case(other))
}
pub(super) fn csharp_file(path: &[u8]) -> bool {
    path.ends_with(b".cs") && file_name(path).len() > 3
}

pub(super) fn script_language(path: &[u8]) -> Option<ModuleLanguage> {
    let ext = extension(path);
    if [b"ps1".as_slice(), b"psm1", b"psd1"].iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        Some(ModuleLanguage::PowerShell)
    } else if matches!(ext, b"sh" | b"bash") || matches!(file_name(path), b".bashrc" | b".bash_profile" | b".bash_login" | b".bash_logout" | b".bash_aliases") {
        Some(ModuleLanguage::Bash)
    } else {
        None
    }
}

/// Folders a language's own tools pass over: hidden ones and installed packages, for Go
/// `testdata`, `vendor` and names starting with `_`, and for C# the build's `bin` and `obj`.
fn skipped(path: &[u8], language: ModuleLanguage) -> bool {
    let folders = path.split(|b| *b == b'/').collect::<Vec<_>>();
    folders[..folders.len().saturating_sub(1)].iter().any(|f| {
        f.starts_with(b".")
            || *f == b"node_modules"
            // A committed virtual environment holds installed packages, not the project's.
            || language == ModuleLanguage::Python && matches!(*f, b"__pycache__" | b"site-packages" | b"dist-packages" | b"__pypackages__")
            || language == ModuleLanguage::Go && (*f == b"testdata" || *f == b"vendor" || f.starts_with(b"_"))
            || language == ModuleLanguage::CSharp && matches!(*f, b"bin" | b"obj")
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
pub(super) fn top_folder(path: &[u8]) -> &[u8] {
    match path.iter().position(|b| *b == b'/') {
        Some(at) => &path[..at],
        None => b"",
    }
}

/// The packages `paths` hold, by language and folder, and whether some were left out
/// past [`PACKAGES`].
pub(super) fn find(paths: &[&[u8]]) -> (Vec<Found>, bool) {
    find_up_to(paths, PACKAGES)
}

/// As [`find`], with up to `limit` packages a language.
pub(super) fn find_up_to(paths: &[&[u8]], limit: usize) -> (Vec<Found>, bool) {
    let mut out = Vec::new();
    let mut limited = false;
    let mut add = |mut list: Vec<Found>, out: &mut Vec<Found>| {
        list.retain(|f| !f.files.is_empty());
        list.sort_by(|a, b| a.folder.cmp(&b.folder));
        limited |= list.len() > limit;
        out.extend(list.into_iter().take(limit));
    };
    add(javascript(paths), &mut out);
    add(python(paths), &mut out);
    add(go(paths), &mut out);
    add(java(paths), &mut out);
    add(c(paths), &mut out);
    add(csharp(paths), &mut out);
    for (language, folders) in script_folders(paths) {
        add(folders.into_iter().map(|(folder, mut files)| {
            files.sort();
            Found { folder, language, files, manifest: None, dialect: None }
        }).collect(), &mut out);
    }
    (out, limited)
}

/// Scripts need no manifest. Keep CI helpers in committed dot-folders such as .github.
fn script_folders(paths: &[&[u8]]) -> BTreeMap<ModuleLanguage, BTreeMap<Vec<u8>, Vec<Vec<u8>>>> {
    let mut groups: BTreeMap<_, BTreeMap<_, Vec<_>>> = BTreeMap::new();
    for &path in paths {
        if let Some(language) = script_language(path) {
            if !path.split(|&b| b == b'/').any(|part| part == b"node_modules") {
                groups.entry(language).or_default().entry(top_folder(path).to_vec()).or_default().push(path.to_vec());
            }
        }
    }
    groups
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
            dialect: None,
            files: Vec::new(),
            folder,
            language,
        });
        if typescript_file(path) {
            found.dialect = Some(Dialect::TypeScript);
        }
        found.files.push(path.to_vec());
    }
    let mut list: Vec<Found> = projects.into_values().collect();
    for found in &mut list {
        if configs.contains(found.folder.as_slice()) {
            found.dialect = Some(Dialect::TypeScript);
        }
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
                dialect: None,
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
                dialect: None,
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

/// Maven's and Gradle's builds, in the order that names a project: settings.gradle's
/// rootProject.name, then pom.xml's artifactId.
const JAVA_BUILDS: [&[u8]; 5] = [b"settings.gradle.kts", b"settings.gradle", b"pom.xml", b"build.gradle.kts", b"build.gradle"];

/// Each Maven or Gradle build's folder is a project holding the Java and Kotlin files
/// below it that no deeper build holds, as Maven's modules and Gradle's subprojects are;
/// files no build holds make one project per top-level folder. Kotlin names a project
/// with more Kotlin files than Java ones.
fn java(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::Java;
    let mut builds: HashMap<&[u8], Vec<u8>> = HashMap::new();
    for path in paths.iter().filter(|p| !skipped(p, language)) {
        let name = file_name(path);
        if let Some(rank) = JAVA_BUILDS.iter().position(|b| *b == name) {
            let best = builds.entry(parent(path)).or_insert_with(|| path.to_vec());
            if JAVA_BUILDS.iter().position(|b| *b == file_name(best)).is_none_or(|r| rank < r) {
                *best = path.to_vec();
            }
        }
    }
    let folders: HashSet<&[u8]> = builds.keys().copied().collect();
    let mut found: BTreeMap<(Vec<u8>, bool), (Found, usize)> = BTreeMap::new();
    for path in paths.iter().filter(|p| java_file(p) && !skipped(p, language)) {
        let (folder, manifest) = match nearest(path, &folders) {
            Some(folder) => (folder.to_vec(), true),
            None => (top_folder(path).to_vec(), false),
        };
        let (f, kotlin) = found.entry((folder.clone(), manifest)).or_insert_with(|| {
            (
                Found {
                    manifest: manifest.then(|| builds[folder.as_slice()].clone()),
                    dialect: None,
                    files: Vec::new(),
                    folder,
                    language,
                },
                0,
            )
        });
        *kotlin += usize::from(kotlin_file(path));
        f.files.push(path.to_vec());
    }
    found
        .into_values()
        .map(|(mut f, kotlin)| {
            f.files.sort();
            f.dialect = (kotlin * 2 > f.files.len()).then_some(Dialect::Kotlin);
            f
        })
        .collect()
}

/// The builds C and C++ projects have, by name; Visual Studio's by its `.vcxproj`.
const C_BUILDS: [&[u8]; 11] = [
    b"CMakeLists.txt", b"meson.build", b"Makefile", b"GNUmakefile", b"makefile", b"configure.ac", b"configure.in", b"SConstruct", b"MODULE.bazel",
    b"WORKSPACE", b"WORKSPACE.bazel",
];

/// The top-most folder with a C or C++ build is a project holding every C and C++ file
/// below it: CMake, Meson and Make read the folders below from one build, so the build
/// files deeper down are parts of it. Files no build holds make one project, in the
/// deepest folder that holds them all: a library kept beside another language's code
/// spreads over `src` and `include`. C++ names a project with any C++ file.
fn c(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::C;
    let mut builds: HashMap<&[u8], Vec<u8>> = HashMap::new();
    for path in paths.iter().filter(|p| !skipped(p, language)) {
        let name = file_name(path);
        let rank = C_BUILDS.iter().position(|b| *b == name).or_else(|| extension(path).eq_ignore_ascii_case(b"vcxproj").then_some(C_BUILDS.len()));
        if let Some(rank) = rank {
            let best = builds.entry(parent(path)).or_insert_with(|| path.to_vec());
            let current = C_BUILDS.iter().position(|b| *b == file_name(best)).unwrap_or(C_BUILDS.len());
            if rank < current {
                *best = path.to_vec();
            }
        }
    }
    let all: HashSet<&[u8]> = builds.keys().copied().collect();
    // Only the top-most builds; one in the top folder holds everything.
    let tops: HashSet<&[u8]> = all
        .iter()
        .copied()
        .filter(|f| {
            let mut up = *f;
            while !up.is_empty() {
                up = parent(up);
                if all.contains(up) {
                    return false;
                }
            }
            true
        })
        .collect();
    let mut found: BTreeMap<Vec<u8>, Found> = BTreeMap::new();
    let mut loose: Vec<Vec<u8>> = Vec::new();
    for path in paths.iter().filter(|p| c_file(p) && !skipped(p, language)) {
        let Some(folder) = nearest(path, &tops) else {
            loose.push(path.to_vec());
            continue;
        };
        found
            .entry(folder.to_vec())
            .or_insert_with(|| Found {
                manifest: Some(builds[folder].clone()),
                dialect: None,
                files: Vec::new(),
                folder: folder.to_vec(),
                language,
            })
            .files
            .push(path.to_vec());
    }
    let mut list: Vec<Found> = found.into_values().collect();
    if !loose.is_empty() {
        // The deepest folder that holds every loose file.
        let mut base: Option<Vec<&[u8]>> = None;
        for file in &loose {
            let parts: Vec<&[u8]> = parent(file).split(|b| *b == b'/').filter(|p| !p.is_empty()).collect();
            base = Some(match base {
                None => parts,
                Some(b) => b.iter().zip(&parts).take_while(|(x, y)| x == y).map(|(x, _)| *x).collect(),
            });
        }
        list.push(Found {
            folder: base.unwrap_or_default().join(&b'/'),
            language,
            manifest: None,
            dialect: None,
            files: loose,
        });
    }
    for f in &mut list {
        f.files.sort();
        if f.files.iter().any(|p| cpp_file(p)) {
            f.dialect = Some(Dialect::Cpp);
        }
    }
    list
}

/// Each .csproj's folder is a project holding the C# files below it that no deeper
/// project holds, as an SDK project takes them; files no project holds make one
/// project per top-level folder.
fn csharp(paths: &[&[u8]]) -> Vec<Found> {
    let language = ModuleLanguage::CSharp;
    let mut projects: BTreeMap<&[u8], &[u8]> = BTreeMap::new();
    for &path in paths.iter().filter(|p| extension(p).eq_ignore_ascii_case(b"csproj") && !skipped(p, language)) {
        // Two in one folder: the first by name.
        let entry = projects.entry(parent(path)).or_insert(path);
        if path < *entry {
            *entry = path;
        }
    }
    let folders: HashSet<&[u8]> = projects.keys().copied().collect();
    let mut found: BTreeMap<(Vec<u8>, bool), Found> = BTreeMap::new();
    for path in paths.iter().filter(|p| csharp_file(p) && !skipped(p, language)) {
        let (folder, manifest) = match nearest(path, &folders) {
            Some(folder) => (folder.to_vec(), true),
            None => (top_folder(path).to_vec(), false),
        };
        found
            .entry((folder.clone(), manifest))
            .or_insert_with(|| Found {
                manifest: manifest.then(|| projects[folder.as_slice()].to_vec()),
                dialect: None,
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

// ---- Project names -------------------------------------------------------------------

/// The name a build gives its project, from the manifest's text: Gradle's
/// `rootProject.name`, Maven's own artifactId, CMake's or Meson's `project(…)`, a
/// .csproj's own file name. None when it names none plainly (a `${…}` name too).
pub(super) fn project_name(manifest: &[u8], text: Option<&str>) -> Option<String> {
    let name = file_name(manifest);
    if extension(manifest).eq_ignore_ascii_case(b"csproj") {
        let stem = &name[..name.len() - b".csproj".len()];
        return (!stem.is_empty()).then(|| String::from_utf8_lossy(stem).into_owned());
    }
    let text = text?;
    let found = match name {
        b"settings.gradle" | b"settings.gradle.kts" => gradle_name(text),
        b"pom.xml" => maven_name(text),
        b"CMakeLists.txt" => cmake_name(text),
        b"meson.build" => meson_name(text),
        _ => None,
    }?;
    let found = found.trim().to_owned();
    (!found.is_empty() && !found.contains('$') && !found.contains(char::is_whitespace) && found.len() <= 200).then_some(found)
}
/// Whether a manifest's text names its project, so a read wants it.
pub(super) fn names_project(manifest: &[u8]) -> bool {
    matches!(file_name(manifest), b"settings.gradle" | b"settings.gradle.kts" | b"pom.xml" | b"CMakeLists.txt" | b"meson.build")
}
fn gradle_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let Some(rest) = line.strip_prefix("rootProject.name") else { continue };
        let rest = rest.trim_start().strip_prefix('=')?.trim();
        let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'')?;
        return rest[1..].split(quote).next().map(str::to_owned);
    }
    None
}
fn maven_name(text: &str) -> Option<String> {
    // Comments and the parent's own artifactId are no part of the project's.
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        plain.push_str(&rest[..at]);
        rest = rest[at..].find("-->").map_or("", |end| &rest[at + end + 3..]);
    }
    plain.push_str(rest);
    if let (Some(open), Some(close)) = (plain.find("<parent>"), plain.find("</parent>"))
        && open < close
    {
        plain.replace_range(open..close + "</parent>".len(), "");
    }
    // The project's own artifactId comes before its dependencies and build.
    let end = ["<dependencies>", "<dependencyManagement>", "<build>", "<profiles>", "<modules>"].iter().filter_map(|t| plain.find(t)).min().unwrap_or(plain.len());
    let head = &plain[..end];
    let start = head.find("<artifactId>")? + "<artifactId>".len();
    head[start..].split("</artifactId>").next().map(str::to_owned)
}
fn cmake_name(text: &str) -> Option<String> {
    // Comments are no part of the call (`# project(fake)`); the call may span lines.
    let plain = text.lines().map(|l| l.split('#').next().unwrap_or_default()).collect::<Vec<_>>().join("\n");
    let lower = plain.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("project") {
        let at = from + found;
        from = at + "project".len();
        // A command of its own, at a line's start, then `(`.
        let starts = lower[..at].chars().rev().take_while(|c| *c != '\n').all(char::is_whitespace);
        let rest = lower[from..].trim_start();
        if !starts || !rest.starts_with('(') {
            continue;
        }
        let open = plain.len() - rest.len() + 1;
        let name: String = plain[open..].trim_start().chars().take_while(|c| !c.is_whitespace() && *c != ')').collect();
        return Some(name.trim_matches('"').to_owned());
    }
    None
}
fn meson_name(text: &str) -> Option<String> {
    let at = text.find("project(")? + "project(".len();
    let rest = text[at..].trim_start();
    let quote = rest.chars().next().filter(|q| *q == '\'' || *q == '"')?;
    rest[1..].split(quote).next().map(str::to_owned)
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
                    f.dialect.is_some(),
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
    fn java_c_and_csharp_projects_by_their_builds() {
        let list = found(&[
            // Gradle: the root build and a subproject; tests and scripts left out.
            "settings.gradle.kts",
            "build.gradle.kts",
            "app/build.gradle.kts",
            "app/src/main/kotlin/com/acme/App.kt",
            "app/src/main/kotlin/com/acme/Util.kt",
            "app/src/main/java/com/acme/Legacy.java",
            "app/src/test/kotlin/com/acme/AppTest.kt",
            "app/src/androidTest/java/com/acme/UiTest.java",
            "app/src/main/java/module-info.java",
            "buildSrc/src/main/kotlin/Deps.kt",
            "samples/Hello.java",
            // C: one top-most CMake project; nested builds are its parts. Loose files make one.
            "native/CMakeLists.txt",
            "native/src/CMakeLists.txt",
            "native/src/core.c",
            "native/include/core.h",
            "third/x/a.c",
            "third/y/b.h",
            // C#: each .csproj; bin and obj are the build's.
            "dotnet/Web/Web.csproj",
            "dotnet/Web/Program.cs",
            "dotnet/Web/obj/Debug/Gen.cs",
            "dotnet/Core/Core.csproj",
            "dotnet/Core/Models/User.cs",
            "dotnet/Core/Models/Order.cs",
            "scripts/tool.cs",
        ]);
        let rows: Vec<_> = list.iter().map(|(f, l, m, d, files)| (f.as_str(), *l, *m, *d, files.len())).collect();
        assert_eq!(
            rows,
            [
                // The root build holds what no deeper build does: buildSrc and samples.
                ("", ModuleLanguage::Java, true, false, 2),
                // Two Kotlin files of three.
                ("app", ModuleLanguage::Java, true, true, 3),
                ("native", ModuleLanguage::C, true, false, 2),
                ("third", ModuleLanguage::C, false, false, 2),
                ("dotnet/Core", ModuleLanguage::CSharp, true, false, 2),
                ("dotnet/Web", ModuleLanguage::CSharp, true, false, 1),
                ("scripts", ModuleLanguage::CSharp, false, false, 1),
            ]
        );
        // app/ has its own build: the root build holds what no deeper build does.
        let bytes: Vec<&[u8]> = ["settings.gradle", "core/pom.xml", "core/src/main/java/a/A.java", "B.java"].iter().map(|p| p.as_bytes()).collect();
        let (java, _) = find(&bytes);
        let folders: Vec<(&[u8], Option<&[u8]>)> = java.iter().map(|f| (f.folder.as_slice(), f.manifest.as_deref())).collect();
        assert_eq!(folders, [(b"".as_slice(), Some(b"settings.gradle".as_slice())), (b"core", Some(b"core/pom.xml"))]);
        // C++ by any C++ file; a .h alone is C.
        let bytes: Vec<&[u8]> = ["Makefile", "main.cpp", "util.h"].iter().map(|p| p.as_bytes()).collect();
        assert_eq!(find(&bytes).0[0].dialect, Some(Dialect::Cpp));
    }

    #[test]
    fn jvm_test_source_sets_use_word_boundaries() {
        for set in ["latestDebug", "CONTEST", "testimonials", "Testimony", "détesté"] {
            assert!(java_file(format!("src/{set}/App.kt").as_bytes()), "{set}");
        }
        for set in ["tests", "unit_tests", "JVMTest", "testRelease", "iosSimulatorArm64Test"] {
            assert!(!java_file(format!("src/{set}/App.kt").as_bytes()), "{set}");
        }
    }

    #[test]
    fn cpp_module_interface_extensions_are_source_files() {
        for file in ["a.cppm", "a.ixx", "a.mpp", "a.ccm", "a.cxxm", "a.c++m", "a.CPPM"] {
            assert!(c_file(file.as_bytes()) && cpp_file(file.as_bytes()), "{file}");
        }
        for file in ["a.ifc", "a.pcm", ".C"] {
            assert!(!c_file(file.as_bytes()), "{file}");
        }
    }

    #[test]
    fn project_names_from_their_builds() {
        let name = |manifest: &str, text: &str| project_name(manifest.as_bytes(), Some(text));
        assert_eq!(name("settings.gradle.kts", "pluginManagement { }\nrootProject.name = \"petclinic\"\ninclude(\":app\")\n").as_deref(), Some("petclinic"));
        assert_eq!(name("settings.gradle", "// rootProject.name = 'old'\nrootProject.name = 'shop'\n").as_deref(), Some("shop"));
        assert_eq!(
            name("pom.xml", "<project><!-- <artifactId>comment</artifactId> --><parent><artifactId>parent</artifactId></parent><groupId>g</groupId><artifactId>store</artifactId><dependencies><dependency><artifactId>dep</artifactId></dependency></dependencies></project>").as_deref(),
            Some("store")
        );
        assert_eq!(name("pom.xml", "<project><artifactId>${name}</artifactId></project>"), None);
        assert_eq!(name("CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\n# project(fake)\nPROJECT( libgit2 VERSION 1.9.0 LANGUAGES C)\n").as_deref(), Some("libgit2"));
        assert_eq!(name("CMakeLists.txt", "project(\"quoted\")\n").as_deref(), Some("quoted"));
        assert_eq!(name("CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\nproject(\n  fmt # the library\n  VERSION 11.0.0\n  LANGUAGES CXX)\n").as_deref(), Some("fmt"));
        assert_eq!(name("CMakeLists.txt", "set(project_version 1)\nproject_version(x)\n"), None);
        assert_eq!(name("meson.build", "project('glib', 'c', version: '2.80')\n").as_deref(), Some("glib"));
        assert_eq!(project_name(b"src/Acme.Web/Acme.Web.csproj", None).as_deref(), Some("Acme.Web"));
        assert_eq!(name("build.gradle", "plugins {}"), None);
        assert!(names_project(b"x/pom.xml") && !names_project(b"x/build.gradle") && !names_project(b"x/App.csproj"));
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
