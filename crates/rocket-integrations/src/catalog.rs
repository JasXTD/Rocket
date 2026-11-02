use crate::{Command, Setting, array, id, invalid, object, text};
use rocket_git::{CancellationToken, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Package {
    pub directory: PathBuf,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub network: bool,
    pub writes: bool,
    pub repository: bool,
    pub settings: Vec<Setting>,
    pub commands: Vec<Command>,
    pub manifest: Vec<u8>,
    pub source: String,
}
/// Most commands one package can declare.
const MAX_COMMANDS: usize = 8;
/// What an approved snapshot asked for, read back for the update review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedCopy {
    pub name: String,
    pub version: String,
    pub network: bool,
    pub writes: bool,
    pub repository: bool,
    /// The commands the user has seen in a review; menus show these, not the files on disk.
    pub commands: Vec<Command>,
    pub source: String,
}
impl ApprovedCopy {
    pub fn read(snapshot: &Value) -> Option<Self> {
        let manifest: Value = serde_json::from_str(snapshot["manifest"].as_str()?).ok()?;
        let flag = |key: &str| manifest["permissions"][key].as_bool().unwrap_or(false);
        Some(Self {
            name: manifest["name"].as_str()?.to_owned(),
            version: manifest["version"].as_str()?.to_owned(),
            network: flag("network"),
            writes: flag("write"),
            repository: flag("repository"),
            commands: manifest["commands"]
                .as_array()
                .map(|list| list.iter().filter_map(|c| Command::parse(c).ok()).collect())
                .unwrap_or_default(),
            source: snapshot["source"].as_str()?.to_owned(),
        })
    }
}
fn ordinary(path: &Path, dir: bool) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)
        .map_err(|_| invalid("integration_files: Cannot read the extension's folder or files."))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(invalid(
                "integration_files: Extension folders and files cannot be links or junctions.",
            ));
        }
    }
    if m.file_type().is_symlink() || m.is_dir() != dir || !dir && !m.is_file() {
        return Err(invalid(
            "integration_files: Expected an ordinary folder or file.",
        ));
    }
    Ok(m)
}
fn read(path: &Path, max: usize) -> Result<Vec<u8>> {
    ordinary(path, false)?;
    let mut v = vec![];
    fs::File::open(path)
        .and_then(|f| f.take((max + 1) as u64).read_to_end(&mut v))
        .map_err(|_| invalid("integration_files: Could not read the extension's files."))?;
    if v.len() > max {
        return Err(invalid(
            "integration_files_limit: The manifest or Lua source exceeds its size limit.",
        ));
    }
    Ok(v)
}
impl Package {
    pub fn load(directory: &Path) -> Result<Self> {
        ordinary(directory, true)?;
        let manifest = read(&directory.join("integration.json"), 32 * 1024)?;
        let source = String::from_utf8(read(&directory.join("main.lua"), 64 * 1024)?)
            .map_err(|_| invalid("integration_source: Lua source must be UTF-8 text."))?;
        let package = Self::parse(directory, manifest, source)?;
        package.verify()?;
        Ok(package)
    }
    /// Checks a manifest and source pair; nothing here touches the disk.
    fn parse(directory: &Path, manifest: Vec<u8>, source: String) -> Result<Self> {
        if source.starts_with('\u{1b}') || source.contains('\0') {
            return Err(invalid(
                "integration_source: Only Lua source text is accepted; bytecode is disabled.",
            ));
        }
        let v: Value = serde_json::from_slice(&manifest)
            .map_err(|_| invalid("integration_manifest: The manifest is not valid JSON."))?;
        // A newer API may add fields, so this comes before the check for unknown ones.
        if let Some(api) = v["api"].as_u64().filter(|api| *api > 1) {
            let name = v["name"]
                .as_str()
                .filter(|s| !s.is_empty() && text(s, 100))
                .unwrap_or("This package");
            return Err(invalid(&format!(
                "integration_api: {name} needs Lua API {api}. This Rocket has API 1. Update Rocket to install it."
            )));
        }
        if object(&v)?.keys().any(|k| {
            ![
                "api",
                "id",
                "name",
                "version",
                "description",
                "permissions",
                "settings",
                "commands",
            ]
            .contains(&k.as_str())
        }) || v["api"] != 1
        {
            return Err(invalid(
                "integration_manifest: Unsupported API version or manifest fields. This build supports API 1.",
            ));
        }
        let get = |key: &str, max: usize| -> Result<String> {
            v[key]
                .as_str()
                .filter(|s| !s.is_empty() && text(s, max))
                .map(str::to_owned)
                .ok_or_else(|| {
                    invalid("integration_manifest: The manifest needs a name, an ID and a version.")
                })
        };
        let package_id = get("id", 64)?;
        if !id(&package_id) {
            return Err(invalid(
                "integration_manifest: Use an ASCII extension ID.",
            ));
        }
        let permissions = object(&v["permissions"])?;
        if permissions
            .keys()
            .any(|k| !["network", "write", "repository"].contains(&k.as_str()))
            || permissions.values().any(|v| !v.is_boolean())
        {
            return Err(invalid(
                "integration_manifest: Only network, reviewed-write and read-only repository permissions are supported.",
            ));
        }
        let mut settings = vec![];
        let mut ids = std::collections::HashSet::new();
        if let Some(values) = v.get("settings") {
            for value in array(values)? {
                let setting = Setting::parse(value)?;
                if !ids.insert(setting.id.clone()) || settings.len() >= 16 {
                    return Err(invalid(
                        "integration_manifest: Settings must have unique IDs, maximum 16.",
                    ));
                }
                settings.push(setting);
            }
        }
        let repository = v["permissions"]["repository"].as_bool().unwrap_or(false);
        let mut commands: Vec<Command> = vec![];
        if let Some(values) = v.get("commands") {
            for value in array(values)? {
                let command = Command::parse(value)?;
                if commands.iter().any(|c| c.id == command.id) || commands.len() >= MAX_COMMANDS {
                    return Err(invalid(
                        "integration_manifest: Commands must have unique IDs, maximum 8.",
                    ));
                }
                if command.target.is_some() && !repository {
                    return Err(invalid(
                        "integration_manifest: Commit and branch commands need the repository permission.",
                    ));
                }
                commands.push(command);
            }
        }
        Ok(Self {
            directory: directory.into(),
            id: package_id,
            name: get("name", 100)?,
            version: get("version", 40)?,
            description: v["description"]
                .as_str()
                .filter(|s| text(s, 512))
                .unwrap_or("")
                .into(),
            network: v["permissions"]["network"].as_bool().unwrap_or(false),
            writes: v["permissions"]["write"].as_bool().unwrap_or(false),
            repository,
            settings,
            commands,
            manifest,
            source,
        })
    }
    pub fn snapshot(&self) -> Value {
        json!({"manifest":String::from_utf8_lossy(&self.manifest),"source":self.source})
    }
    pub fn approved(&self, snapshot: &Value) -> bool {
        snapshot["manifest"].as_str() == std::str::from_utf8(&self.manifest).ok()
            && snapshot["source"].as_str() == Some(&self.source)
    }
    /// Whether two copies are the same files, wherever they are.
    pub fn same_files(&self, other: &Self) -> bool {
        self.manifest == other.manifest && self.source == other.source
    }
    pub fn verify(&self) -> Result<()> {
        ordinary(&self.directory, true)?;
        if read(&self.directory.join("integration.json"), 32 * 1024)? != self.manifest
            || read(&self.directory.join("main.lua"), 64 * 1024)? != self.source.as_bytes()
        {
            return Err(invalid(
                "integration_changed: The extension's files changed. Review them again before it can run.",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct PackageIssue {
    pub folder: String,
    pub error: String,
    pub directory: PathBuf,
    /// The ID and version of a copy refused because another folder has that ID.
    pub duplicate: Option<(String, String)>,
}
#[derive(Default)]
pub struct Discovery {
    pub packages: Vec<Package>,
    pub issues: Vec<PackageIssue>,
}
fn folder_label(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}
pub fn discover(root: &Path, cancel: &CancellationToken) -> Result<Discovery> {
    cancel.check()?;
    if !root.exists() {
        return Ok(Discovery::default());
    }
    ordinary(root, true)?;
    let mut packages = vec![];
    let mut issues = vec![];
    for (i, entry) in fs::read_dir(root)
        .map_err(|_| invalid("integration_folder: Cannot list the extensions folder."))?
        .enumerate()
    {
        cancel.check()?;
        if i >= 64 {
            return Err(invalid(
                "integration_folder_limit: Keep at most 64 entries in the extensions folder.",
            ));
        }
        let entry =
            entry.map_err(|_| invalid("integration_folder: Could not inspect a folder entry."))?;
        if !entry
            .file_type()
            .map_err(|_| invalid("integration_folder: Could not inspect a folder entry."))?
            .is_dir()
        {
            continue;
        }
        match Package::load(&entry.path()) {
            Ok(package) => packages.push(package),
            Err(rocket_git::Error::InvalidInput(error)) => issues.push(PackageIssue {
                folder: folder_label(&entry.path()),
                error,
                directory: entry.path(),
                duplicate: None,
            }),
            Err(e) => return Err(e),
        }
    }
    cancel.check()?;
    // Refuse every instance of a duplicate, rather than trusting whichever the
    // filesystem enumerates first. Unrelated packages still remain available.
    let mut counts = std::collections::HashMap::new();
    for p in &packages {
        *counts.entry(p.id.clone()).or_insert(0) += 1;
    }
    packages.retain(|p| {
        if counts[&p.id] == 1 { return true; }
        issues.push(PackageIssue {
            folder: folder_label(&p.directory),
            error: format!("integration_duplicate: {} folders use the ID {}, so all copies stay off.", counts[&p.id], p.id),
            directory: p.directory.clone(),
            duplicate: Some((p.id.clone(), p.version.clone())),
        });
        false
    });
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    issues.sort_by(|a, b| a.folder.cmp(&b.folder));
    Ok(Discovery { packages, issues })
}
pub fn example_names() -> [&'static str; 3] {
    ["jira", "playground", "repository"]
}
fn example_files(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "jira" => (
            include_str!("../examples/jira/integration.json"),
            include_str!("../examples/jira/main.lua"),
        ),
        "playground" => (
            include_str!("../examples/playground/integration.json"),
            include_str!("../examples/playground/main.lua"),
        ),
        "repository" => (
            include_str!("../examples/repository/integration.json"),
            include_str!("../examples/repository/main.lua"),
        ),
        _ => return None,
    })
}
/// The extensions that ship inside Rocket, as they would land in `root`. They list before
/// anything is on disk; the first Enable copies them there after its review.
pub fn bundled(root: &Path) -> Vec<Package> {
    ["jira", "playground"]
        .iter()
        .filter_map(|name| {
            let (manifest, source) = example_files(name)?;
            Package::parse(&root.join(name), manifest.as_bytes().to_vec(), source.to_owned()).ok()
        })
        .collect()
}
pub fn install_example(root: &Path, name: &str) -> Result<()> {
    let (manifest, source) =
        example_files(name).ok_or_else(|| invalid("integration_example: Unknown example."))?;
    fs::create_dir_all(root)
        .map_err(|_| invalid("integration_folder: Cannot create the extensions folder."))?;
    ordinary(root, true)?;
    let directory = root.join(name);
    fs::create_dir(&directory).map_err(|_|invalid("integration_exists: The sample is already in the extensions folder. Existing files were preserved."))?;
    for (file, bytes) in [("integration.json", manifest), ("main.lua", source)] {
        let mut f=fs::OpenOptions::new().create_new(true).write(true).open(directory.join(file)).map_err(|_|invalid("integration_example: Could not create example files. Existing files were preserved."))?;
        f.write_all(bytes.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|_| invalid("integration_example: Could not save the example file."))?;
    }
    Ok(())
}
/// The package a chosen `integration.json` belongs to, with `main.lua` beside it.
pub fn read_package_file(file: &Path) -> Result<Package> {
    if !file
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("integration.json"))
    {
        return Err(invalid(
            "integration_file: Choose the integration.json of a package.",
        ));
    }
    let directory = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| invalid("integration_file: Choose the integration.json of a package."))?;
    if !directory.join("main.lua").is_file() {
        return Err(invalid(
            "integration_files: main.lua must be beside integration.json.",
        ));
    }
    Package::load(directory)
}
/// A folder beside `root`: on the same volume, so it renames in one step, and never listed.
fn beside(root: &Path, label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    root.with_file_name(format!("{name}-{label}-{}-{stamp}", std::process::id()))
}
/// Puts a reviewed package in the extensions folder and returns what landed. The bytes are
/// the reviewed ones, whatever the chosen file says by now. A replaced copy must still match
/// what the review showed, and moves aside only once the new files are complete, so a failure
/// leaves it in place.
pub fn install_package(root: &Path, package: &Package, replaces: Option<&Package>) -> Result<Package> {
    let failed = || {
        invalid(&match replaces {
            Some(old) => format!(
                "integration_folder: Could not write to the extensions folder. {} {} is unchanged.",
                old.name, old.version
            ),
            None => "integration_folder: Could not write to the extensions folder. Nothing was installed.".to_owned(),
        })
    };
    fs::create_dir_all(root).map_err(|_| failed())?;
    ordinary(root, true)?;
    let target = match replaces {
        Some(old) => {
            old.verify()?;
            old.directory.clone()
        }
        None => root.join(&package.id),
    };
    if replaces.is_none() && fs::symlink_metadata(&target).is_ok() {
        return Err(invalid(&format!(
            "integration_exists: The extensions folder already has a {} folder. Existing files were preserved.",
            package.id
        )));
    }
    let stage = beside(root, "incoming");
    let staged = (|| {
        fs::create_dir(&stage)?;
        for (file, bytes) in [("integration.json", package.manifest.as_slice()), ("main.lua", package.source.as_bytes())] {
            let mut f = fs::OpenOptions::new().create_new(true).write(true).open(stage.join(file))?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        Ok::<(), std::io::Error>(())
    })();
    if staged.is_err() {
        let _ = fs::remove_dir_all(&stage);
        return Err(failed());
    }
    let aside = beside(root, "previous");
    if replaces.is_some() && fs::rename(&target, &aside).is_err() {
        let _ = fs::remove_dir_all(&stage);
        return Err(failed());
    }
    if fs::rename(&stage, &target).is_err() {
        if replaces.is_some() {
            let _ = fs::rename(&aside, &target);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(failed());
    }
    if replaces.is_some() {
        let _ = fs::remove_dir_all(&aside);
    }
    Package::load(&target)
}
/// Takes a package folder out of the extensions folder in one step, then deletes it.
pub fn remove_package(root: &Path, directory: &Path) -> Result<()> {
    if directory.parent() != Some(root) {
        return Err(invalid(
            "integration_folder: Only a folder directly in the extensions folder can be removed.",
        ));
    }
    ordinary(directory, true)?;
    let aside = beside(root, "removed");
    fs::rename(directory, &aside).map_err(|_| {
        invalid("integration_folder: Could not remove the folder. Its files are unchanged.")
    })?;
    let _ = fs::remove_dir_all(&aside);
    Ok(())
}
