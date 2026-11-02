//! Give each shell the environment it needs. PowerShell builds its own module search
//! path: a launcher running under PowerShell 7 otherwise passes incompatible Core modules
//! to the Desktop host, also through a shell started in between.
use std::{io, slice};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
}

struct Environment(*mut u16);
impl Drop for Environment {
    fn drop(&mut self) {
        // SAFETY: this is the unique snapshot returned by GetEnvironmentStringsW.
        unsafe { FreeEnvironmentStringsW(self.0) };
    }
}

pub(super) fn for_shell() -> io::Result<Vec<u16>> {
    // SAFETY: no arguments; returns an owned, double-NUL-terminated UTF-16 block.
    let block = unsafe { GetEnvironmentStringsW() };
    if block.is_null() {
        return Err(io::Error::last_os_error());
    }
    let block = Environment(block);
    let mut length = 0;
    // SAFETY: the OS snapshot contains NUL-terminated entries and an empty final
    // entry. It stays alive and is not modified during this walk or the copy.
    unsafe {
        while *block.0.add(length) != 0 {
            while *block.0.add(length) != 0 {
                length += 1;
            }
            length += 1;
        }
        Ok(without_module_path(slice::from_raw_parts(
            block.0,
            length + 1,
        )))
    }
}

fn without_module_path(block: &[u16]) -> Vec<u16> {
    let prefix = b"PSModulePath=";
    let mut result = Vec::with_capacity(block.len());
    for entry in block.split(|c| *c == 0).take_while(|s| !s.is_empty()) {
        let module_path = entry.len() >= prefix.len()
            && entry.iter().zip(prefix).all(|(&actual, &expected)| {
                u8::try_from(actual).is_ok_and(|c| c.eq_ignore_ascii_case(&expected))
            });
        if !module_path {
            result.extend_from_slice(entry);
            result.push(0);
        }
    }
    // Also retain a valid double terminator when every entry was removed.
    if result.is_empty() {
        result.push(0);
    }
    result.push(0);
    result
}

/// Bash writes its history only when it exits by itself, while End session, closing a
/// tab and quitting all stop the session. Saving after each command keeps what was typed.
pub(super) fn for_git_bash() -> io::Result<Vec<u16>> {
    Ok(saving_history(&for_shell()?))
}

fn saving_history(block: &[u16]) -> Vec<u16> {
    let prefix = "PROMPT_COMMAND=";
    let mut result = Vec::with_capacity(block.len() + 32);
    let mut found = false;
    for entry in block.split(|c| *c == 0).take_while(|s| !s.is_empty()) {
        let text = String::from_utf16_lossy(entry);
        if !found && text.to_ascii_uppercase().starts_with(prefix) {
            found = true;
            result.extend(format!("{prefix}history -a; {}", &text[prefix.len()..]).encode_utf16());
        } else {
            result.extend_from_slice(entry);
        }
        result.push(0);
    }
    if !found {
        result.extend(format!("{prefix}history -a").encode_utf16());
        result.push(0);
    }
    result.push(0);
    result
}

pub(super) fn for_openssh() -> io::Result<Vec<u16>> {
    let original = for_shell()?;
    let mut result = Vec::new();
    for entry in original.split(|c| *c == 0).take_while(|s| !s.is_empty()) {
        let text = String::from_utf16_lossy(entry).to_ascii_uppercase();
        if [
            "SSH_ASKPASS=",
            "SSH_ASKPASS_REQUIRE=",
            "SSH_AUTH_SOCK=",
            "SSH_AGENT_PID=",
        ]
        .iter()
        .any(|p| text.starts_with(p))
        {
            continue;
        }
        result.extend_from_slice(entry);
        result.push(0);
    }
    result.extend("SSH_ASKPASS_REQUIRE=never".encode_utf16());
    result.extend([0, 0]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn removes_only_module_path_preserving_unicode_policy_and_drive_entries() {
        let keep = "=C:=C:\\中文\0APPDATA=C:\\fixture\0PATH=C:\\tools\0PSExecutionPolicyPreference=AllSigned\0PSModulePathExtra=keep\0";
        let original = wide(&format!("{keep}pSmOdUlEpAtH=C:\\CoreModules\0\0"));
        assert_eq!(without_module_path(&original), wide(&format!("{keep}\0")));
        assert!(
            String::from_utf16(&original)
                .unwrap()
                .contains("CoreModules")
        );
        assert_eq!(without_module_path(&[0, 0]), [0, 0]);
        assert_eq!(without_module_path(&wide("PSMODULEPATH=x\0\0")), [0, 0]);
        let unusual = [b'X' as u16, b'=' as u16, 0xd800, 0, 0];
        assert_eq!(without_module_path(&unusual), unusual);
    }
    #[test]
    fn bash_saves_history_after_each_command_and_keeps_its_own_prompt_command() {
        assert_eq!(
            saving_history(&wide("HOME=C:\\ada\0PATH=C:\\tools\0\0")),
            wide("HOME=C:\\ada\0PATH=C:\\tools\0PROMPT_COMMAND=history -a\0\0")
        );
        assert_eq!(
            saving_history(&wide("HOME=C:\\ada\0PROMPT_COMMAND=date\0\0")),
            wide("HOME=C:\\ada\0PROMPT_COMMAND=history -a; date\0\0")
        );
    }
}
