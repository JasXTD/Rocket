//! SSH keys as the desktop's SSH keys page finds them, read only, and this
//! repository's core.sshCommand binding to one Rocket profile. Nothing is unlocked,
//! created or added to ssh-agent, and no passphrase passes through the CLI.
use crate::{
    advanced::review_only,
    args::Command,
    output::{self as out, Outcome, Problem},
};
use rocket_git::*;
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let manager = SshManager::new(home()?.join(".ssh").join("rocket-keys"), None)?;
    match command {
        Command::SshKeys => {
            let inventory = manager.inventory(cancel)?;
            let found = manager.discover_keys(&scan_folder()?, cancel)?;
            Ok(Outcome::success(json!({"folder":out::os_path(&found.directory),"agent_running":inventory.agent_running,
                "keys":found.keys.iter().map(|k|json!({"path":out::os_path(&k.key),"kind":k.kind,"fingerprint":k.fingerprint,
                    "public_key":k.public_key(),"unlocked":inventory.loaded.contains(&k.fingerprint)})).collect::<Vec<_>>(),
                "skipped":found.skipped,"truncated":found.truncated,
                "profiles":inventory.profiles.iter().map(|p|profile(p,&inventory.loaded)).collect::<Vec<_>>(),
                "invalid_profiles":inventory.invalid})))
        }
        Command::SshCheck(alias) => {
            let p = manager.load(alias, cancel)?;
            let config = manager.check_config(&p, cancel)?;
            Ok(Outcome::success(json!({"profile":profile(&p,&[]),"effective_config":config})))
        }
        Command::SshBinding { alias, expected, preview } => {
            let selected = alias.as_ref().map(|a| manager.load(a, cancel)).transpose()?;
            let review = repo.preview_ssh_binding(&manager, selected.as_ref(), cancel)?;
            // The repository configuration as reviewed; the save requires it unchanged.
            let config = std::fs::read(repo.info().common_dir.join("config"))
                .map_err(|source| Error::Io { action: "read the repository configuration", source })?;
            let digest = sha256_bytes(&config)?;
            let plan = json!({"restoring":review.restoring,"profile":review.profile.as_ref().map(|p|profile(p,&[])),
                "backup":review.backup.as_deref().map(out::os_path),"config_sha256":digest});
            if *preview {
                return Ok(review_only(plan));
            }
            if expected.as_deref() != Some(digest.as_str()) {
                return Ok(Outcome::failure(Problem::new("ssh_config_changed", "The repository configuration changed after review.",
                    "Run it again with --preview and pass the SHA-256 it reports. Nothing was written.", 1)));
            }
            let backup = repo.apply_ssh_binding(&manager, &review, cancel)?;
            Ok(Outcome::success(json!({"preview":plan,"recovery_directory":out::os_path(&backup)})))
        }
        _ => unreachable!("only SSH commands are dispatched here"),
    }
}

/// The desktop keeps Rocket's own profiles under %USERPROFILE%\.ssh\rocket-keys.
fn home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::InvalidInput("USERPROFILE must name an absolute folder for SSH keys.".into()))
}
/// The folder the desktop scans for key pairs: HOME's .ssh when HOME is a folder.
fn scan_folder() -> Result<PathBuf> {
    match std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()) {
        Some(home) => Ok(home.join(".ssh")),
        None => Ok(home()?.join(".ssh")),
    }
}
fn profile(p: &SshProfile, loaded: &[String]) -> Value {
    json!({"alias":p.alias,"host":p.host,"user":p.user,"port":p.port,"key":out::os_path(&p.key),"config":out::os_path(&p.config),
        "fingerprint":p.fingerprint,"encrypted":p.encrypted,"public_key":p.public_key,"unlocked":loaded.contains(&p.fingerprint)})
}
