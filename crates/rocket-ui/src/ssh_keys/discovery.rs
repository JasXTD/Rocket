use super::*;

pub(super) fn default_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_default()
        .join(".ssh")
}

impl State {
    /// Keep a scan's keys. A chosen found key stays chosen while it is still found.
    pub(super) fn receive_discovery(&mut self, result: Result<SshDiscovery>) {
        match result {
            Ok(d) => {
                let chosen = self.found_key().map(|k| k.key.clone());
                self.found = chosen.and_then(|key| d.keys.iter().position(|k| k.key == key));
                self.discovery = Some(d);
                self.discovery_error = None;
            }
            Err(e) => self.discovery_error = Some(user_error(&e)),
        }
    }
}
