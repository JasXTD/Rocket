//! Versioned, data-only command definitions. Import never executes a command.
use rocket_git::{
    CancellationToken, CommandAction, CommandInput, CommandInputKind, CommandScope, CustomCommand, Error, MAX_CUSTOM_COMMANDS,
    Result,
};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const LIMIT: usize = 128 * 1024;
fn invalid() -> Error {
    Error::InvalidInput("command_definitions_invalid: expected version 1, 2 or 3, at most 20 commands and 8 valid inputs per command; no unknown fields.".into())
}
pub fn encode(commands: &[CustomCommand]) -> Result<Vec<u8>> {
    if commands.len() > MAX_CUSTOM_COMMANDS {
        return Err(invalid());
    }
    for command in commands {
        command.validate()?;
    }
    let urls = commands.iter().any(|c| c.action == CommandAction::Url);
    let scoped = urls || commands.iter().any(|c| c.scope != CommandScope::Repository);
    let value = json!({"version":if urls {3} else if scoped {2} else {1},"commands":commands.iter().map(|c| {
        let mut row = json!({
        "name":c.name,"program":c.program,"arguments":c.arguments,"group":c.group,
        "inputs":c.inputs.iter().map(|i|json!({"key":i.key,"label":i.label,"kind":i.kind.name(),
            "required":i.required,"default":i.default_value,"checked":i.checked_value})).collect::<Vec<_>>()
        });
        if scoped { row["scope"] = json!(c.scope.key()); }
        if urls { row["action"] = json!(c.action.key()); }
        row
    }).collect::<Vec<_>>()});
    let bytes = serde_json::to_vec_pretty(&value).map_err(|_| invalid())?;
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    Ok(bytes)
}
fn keys(v: &Value, allowed: &[&str]) -> Result<()> {
    if !v.as_object().is_some_and(|o| {
        o.len() == allowed.len() && o.keys().all(|k| allowed.contains(&k.as_str()))
    }) {
        return Err(invalid());
    }
    Ok(())
}
fn text(v: &Value, key: &str) -> Result<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
pub fn decode(bytes: &[u8]) -> Result<Vec<CustomCommand>> {
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    keys(&root, &["version", "commands"])?;
    let version = root["version"].as_u64();
    if !matches!(version, Some(1..=3)) {
        return Err(invalid());
    }
    let list = root["commands"].as_array().ok_or_else(invalid)?;
    if list.len() > MAX_CUSTOM_COMMANDS {
        return Err(invalid());
    }
    let mut commands = Vec::new();
    for c in list {
        if version == Some(1) { keys(c, &["name", "program", "arguments", "group", "inputs"])?; }
        else if version == Some(2) { keys(c, &["name", "program", "arguments", "group", "inputs", "scope"])?; }
        else { keys(c, &["name", "program", "arguments", "group", "inputs", "scope", "action"])?; }
        let mut command = CustomCommand {
            action: if version != Some(3) { CommandAction::Program } else { match text(c,"action")?.as_str() {"program"=>CommandAction::Program,"url"=>CommandAction::Url,_=>return Err(invalid())} },
            scope: if version == Some(1) { CommandScope::Repository } else { CommandScope::from_key(&text(c, "scope")?).ok_or_else(invalid)? },
            name: text(c, "name")?,
            program: text(c, "program")?,
            arguments: text(c, "arguments")?,
            group: text(c, "group")?,
            inputs: Vec::new(),
        };
        let inputs = c["inputs"].as_array().ok_or_else(invalid)?;
        if inputs.len() > 8 {
            return Err(invalid());
        }
        for i in inputs {
            keys(
                i,
                &["key", "label", "kind", "required", "default", "checked"],
            )?;
            command.inputs.push(CommandInput {
                key: text(i, "key")?,
                label: text(i, "label")?,
                kind: match text(i, "kind")?.as_str() {
                    "text" => CommandInputKind::Text,
                    "branch" => CommandInputKind::Branch,
                    "file" => CommandInputKind::File,
                    "checkbox" => CommandInputKind::Checkbox,
                    _ => return Err(invalid()),
                },
                required: i["required"].as_bool().ok_or_else(invalid)?,
                default_value: text(i, "default")?,
                checked_value: text(i, "checked")?,
            });
        }
        command.validate()?;
        commands.push(command);
    }
    Ok(commands)
}
fn parent(path: &Path, scope: Option<&Path>) -> Result<Vec<std::fs::File>> {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path
            .file_name()
            .is_none_or(|n| n.to_string_lossy().contains(':'))
    {
        return Err(invalid());
    }
    let parent = path.parent().ok_or_else(invalid)?;
    scope
        .map(|root| crate::folder_browser::scope::pin_directory(parent, root))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn read(
    path: &Path,
    scope: Option<&Path>,
    cancel: &CancellationToken,
) -> Result<Vec<CustomCommand>> {
    cancel.check()?;
    let _guards = parent(path, scope)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|e| Error::Io {
        action: "open command definitions",
        source: e,
    })?;
    let meta = file.metadata().map_err(|e| Error::Io {
        action: "inspect command definitions",
        source: e,
    })?;
    if !meta.is_file() || meta.len() > LIMIT as u64 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Io {
            action: "read command definitions",
            source: e,
        })?;
    cancel.check()?;
    decode(&bytes)
}
pub fn export(
    path: &Path,
    commands: &[CustomCommand],
    scope: Option<&Path>,
    cancel: &CancellationToken,
) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    cancel.check()?;
    let bytes = encode(commands)?;
    let _guards = parent(path, scope)?;
    let temporary: PathBuf = path.parent().unwrap().join(format!(
        ".rocket-command-share-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(&temporary).map_err(|e| Error::Io {
        action: "prepare command export",
        source: e,
    })?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| Error::Io {
                action: "write command export",
                source: e,
            })?;
        cancel.check()?;
        // Publish complete bytes without replacing an existing file, including a racing creator.
        fs::hard_link(&temporary, path).map_err(|e| Error::Io {
            action: "publish command export to a new filename",
            source: e,
        })?;
        Ok(())
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_types_round_trip_and_legacy_imports_stay_repository_scoped() {
        let mut c=command();
        let legacy=encode(std::slice::from_ref(&c)).unwrap();
        assert_eq!(decode(&legacy).unwrap()[0].scope,CommandScope::Repository);
        for scope in CommandScope::ALL {
            c.scope=scope;let bytes=encode(std::slice::from_ref(&c)).unwrap();
            assert_eq!(decode(&bytes).unwrap(),vec![c.clone()]);
        }
        c.scope=CommandScope::Branch;
        let bytes=encode(&[c]).unwrap();let bad=String::from_utf8(bytes).unwrap().replace("\"branch\"","\"unknown\"");
        assert!(decode(bad.as_bytes()).is_err());
    }
    fn command() -> CustomCommand {
        CustomCommand {
            action: CommandAction::Program,
            name: "Fixture 文".into(),
            scope: CommandScope::Repository,
            program: "C:/Tools/git.exe".into(),
            arguments: "log\n{input:branch}\n{input:flag}".into(),
            group: "Reviews".into(),
            inputs: vec![
                CommandInput {
                    key: "branch".into(),
                    label: "Branch 文".into(),
                    kind: CommandInputKind::Branch,
                    required: true,
                    default_value: "main".into(),
                    ..Default::default()
                },
                CommandInput {
                    key: "flag".into(),
                    label: "Compact".into(),
                    kind: CommandInputKind::Checkbox,
                    default_value: "false".into(),
                    checked_value: "--oneline".into(),
                    ..Default::default()
                },
            ],
        }
    }
    #[test]
    fn url_actions_use_version_three_and_keep_old_program_definitions() {
        let old=command();let url=CustomCommand{action:CommandAction::Url,name:"Review".into(),scope:CommandScope::File,program:"https://example.invalid/?file={path}".into(),group:"Tools / Review".into(),..Default::default()};
        let bytes=encode(&[old.clone(),url.clone()]).unwrap();
        assert_eq!(decode(&bytes).unwrap(),[old,url]);
        let mut v:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(v["version"],3);
        v["commands"][1]["action"]=json!("shell");assert!(decode(&serde_json::to_vec(&v).unwrap()).is_err());
        v["commands"][1]["action"]=json!("url");v["commands"][1]["program"]=json!("file:///unsafe");assert!(decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    fn directory() -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../work/productivity-0.70")
            .join(format!(
                "definitions-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }
    #[test]
    fn definition_data_round_trips_and_rejects_unknown_schema_and_invalid_fields() {
        let c = command();
        let bytes = encode(std::slice::from_ref(&c)).unwrap();
        assert_eq!(decode(&bytes).unwrap(), vec![c]);
        let mut v: Value = serde_json::from_slice(&bytes).unwrap();
        v["commands"][0]["inputs"][1]["key"] = json!("branch");
        assert!(decode(&serde_json::to_vec(&v).unwrap()).is_err());
        v["version"] = json!(2);
        assert!(decode(&serde_json::to_vec(&v).unwrap()).is_err());
        assert!(decode(&vec![b' '; LIMIT + 1]).is_err());
        assert!(decode(br#"{"version":1,"commands":[],"execute":true}"#).is_err());
    }
    #[test]
    fn export_is_complete_no_replace_cancellable_and_import_is_scoped() {
        let d = directory();
        let p = d.join("共有 文.json");
        let c = CancellationToken::new();
        export(&p, &[command()], Some(&d), &c).unwrap();
        let before = fs::read(&p).unwrap();
        assert_eq!(read(&p, Some(&d), &c).unwrap(), vec![command()]);
        assert!(export(&p, &[], Some(&d), &c).is_err());
        assert_eq!(fs::read(&p).unwrap(), before);
        let stop = CancellationToken::new();
        stop.cancel();
        let cancelled = d.join("cancel.json");
        assert!(matches!(
            export(&cancelled, &[], Some(&d), &stop),
            Err(Error::Cancelled)
        ));
        assert!(!cancelled.exists());
        let other = directory();
        assert!(read(&p, Some(&other), &c).is_err());
        assert!(export(&other.join("outside.json"), &[], Some(&d), &c).is_err());
        assert!(
            fs::read_dir(&d).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }
}
