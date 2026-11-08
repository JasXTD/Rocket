//! Exact, synthetic Unicode probes. These only write to the fixture terminal,
//! then pause so MCP can inspect the native frame. Interrupt ends the pause.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const BASIC: &str = "[Console]::Clear(); [Console]::Write(\"ASCII |abcd    |END`r`nCJK   |中文    |END`r`nJPN   |日本語  |END`r`nKOR   |한국어  |END`r`nEMOJI |🚀🎉    |END`r`nMARKS |éö      |END\"); Start-Sleep -Seconds 30";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const EMOJI: &str = "[Console]::Clear(); [Console]::Write(\"ZWJ   |👩🏽‍💻      |END`r`nFAMILY|👨‍👩‍👧‍👦      |END`r`nFLAG  |🇲🇾      |END`r`nKEYCAP|1️⃣      |END`r`nHEART |❤️      |END`r`nTEXT  |☺︎       |END\"); Start-Sleep -Seconds 30";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const MARGIN: &str = "[Console]::Clear(); [Console]::Write((' ' * ([Console]::WindowWidth - 1)) + '中|END'); Start-Sleep -Seconds 30";
// Linux: the same rows through bash's printf, as an MCP window runs bash there.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const BASIC: &str = "printf '\\033[H\\033[2J\\033[3JASCII |abcd    |END\\r\\nCJK   |中文    |END\\r\\nJPN   |日本語  |END\\r\\nKOR   |한국어  |END\\r\\nEMOJI |🚀🎉    |END\\r\\nMARKS |éö      |END'; sleep 30";
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const EMOJI: &str = "printf '\\033[H\\033[2J\\033[3JZWJ   |👩🏽‍💻      |END\\r\\nFAMILY|👨‍👩‍👧‍👦      |END\\r\\nFLAG  |🇲🇾      |END\\r\\nKEYCAP|1️⃣      |END\\r\\nHEART |❤️      |END\\r\\nTEXT  |☺︎       |END'; sleep 30";
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const MARGIN: &str = "printf '\\033[H\\033[2J\\033[3J%*s中|END' \"$(($(stty size | cut -d' ' -f2) - 1))\" ''; sleep 30";

pub fn allowed(text: &str) -> bool {
    matches!(text, BASIC | EMOJI | MARGIN)
}

pub const KEYBOARD_NAMES: &[&str] = &[
    "status",
    "typing",
    "history-seed",
    "history-previous",
    "history-next",
    "completion-command",
    "completion-parameter",
    "completion-path",
    "completion-backward",
    "completion-argument",
    "unicode",
    "submit",
    "cancel-line",
    "copy-shortcut",
    "redraw",
    "resize-draft",
    "resize-edit",
    "interrupt-start",
    "interrupt-status",
    "interrupt-native",
];

/// What a named probe types first, or None for one that only presses keys; None overall
/// for a name that is not a probe. This is Windows PowerShell's text.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn typed(name: &str) -> Option<Option<&'static str>> {
    Some(match name {
        "status" => Some(
            "Write-Output ('ROCKET_PSREADLINE_' + [bool](Get-Module PSReadLine)); Write-Output ('ROCKET_HISTORY_PATH_' + (Get-PSReadLineOption).HistorySavePath); Write-Output ('ROCKET_CORE_' + [bool](Get-Command Get-Item,Get-ExecutionPolicy -ErrorAction Stop))",
        ),
        "typing" => Some("Write-Output 'ROCKET_TYPING_OX'"),
        "resize-draft" => Some("Write-Output 'ROCKET_RESIZE_中文_🚀_é_OX'"),
        "history-seed" => {
            Some("Write-Output 'ROCKET_HISTORY_FIRST'; Write-Output 'ROCKET_HISTORY_SECOND'")
        }
        "completion-command" => Some("Write-Outp"),
        "completion-parameter" => Some("Write-Output -Inp"),
        "completion-path" => Some("Get-Item .\\rocket-comple"),
        "unicode" => Some("Write-Output ('ROCKET_UNICODE_' + '中文 🚀 é')"),
        "interrupt-start" => Some(
            "$global:RocketInterruptState=42; $global:RocketInterruptPid=$PID; [Console]::WriteLine('ROCKET_SHELL_PID_'+$PID); [Console]::WriteLine('ROCKET_INTERRUPT_'+'READY'); Start-Sleep -Seconds 60; [Console]::WriteLine('ROCKET_INTERRUPT_'+'UNEXPECTED')",
        ),
        "interrupt-status" => Some(
            "[Console]::WriteLine('ROCKET_SHELL_'+($PID -eq $global:RocketInterruptPid)); [Console]::WriteLine('ROCKET_STATE_'+$global:RocketInterruptState)",
        ),
        "interrupt-native" => Some(
            r#"& ($PSHOME + '\powershell.exe') -NoLogo -NoProfile -Command "[Console]::WriteLine('ROCKET_CHILD_PID_'+`$PID); [Console]::WriteLine('ROCKET_NATIVE_'+'READY'); Start-Sleep -Seconds 60; [Console]::WriteLine('ROCKET_NATIVE_'+'UNEXPECTED')""#,
        ),
        "history-previous"
        | "history-next"
        | "completion-backward"
        | "completion-argument"
        | "submit"
        | "cancel-line"
        | "copy-shortcut"
        | "redraw"
        | "resize-edit" => None,
        _ => return None,
    })
}

/// Linux: the same probes for Rocket's bash, whose readline edits, recalls and completes
/// lines. They print the same markers as the PowerShell text.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn typed(name: &str) -> Option<Option<&'static str>> {
    Some(match name {
        "status" => Some(
            r#"printf 'ROCKET_READLINE_%s\n' "$(set -o | grep -c '^emacs *on')"; printf 'ROCKET_HISTORY_PATH_%s\n' "$HISTFILE""#,
        ),
        "typing" => Some(r"printf '%s\n' 'ROCKET_TYPING_OX'"),
        "resize-draft" => Some(r"printf '%s\n' 'ROCKET_RESIZE_中文_🚀_é_OX'"),
        "history-seed" => {
            Some(r"printf '%s\n' ROCKET_HISTORY_FIRST; printf '%s\n' ROCKET_HISTORY_SECOND")
        }
        "completion-command" => Some("ech"),
        "completion-parameter" => Some("printf -"),
        "completion-path" => Some("ls ./rocket-comple"),
        "unicode" => Some(r"printf 'ROCKET_UNICODE_%s\n' '中文 🚀 é'"),
        "interrupt-start" => Some(
            r"RocketInterruptState=42; RocketInterruptPid=$$; printf 'ROCKET_SHELL_PID_%s\n' $$; printf 'ROCKET_INTERRUPT_%s\n' READY; sleep 60; printf 'ROCKET_INTERRUPT_%s\n' UNEXPECTED",
        ),
        "interrupt-status" => Some(
            r#"printf 'ROCKET_SHELL_%s\n' "$([ "$$" = "$RocketInterruptPid" ] && echo True || echo False)"; printf 'ROCKET_STATE_%s\n' "$RocketInterruptState""#,
        ),
        "interrupt-native" => Some(
            r#"/bin/bash -c 'printf "ROCKET_CHILD_PID_%s\n" $$; printf "ROCKET_NATIVE_%s\n" READY; sleep 60; printf "ROCKET_NATIVE_%s\n" UNEXPECTED'"#,
        ),
        "history-previous"
        | "history-next"
        | "completion-backward"
        | "completion-argument"
        | "submit"
        | "cancel-line"
        | "copy-shortcut"
        | "redraw"
        | "resize-edit" => None,
        _ => return None,
    })
}

/// Fixed sequences drive the real focused widget; no model or PTY shortcut.
pub fn keyboard(name: &str) -> Option<Vec<eframe::egui::Event>> {
    use crate::input::key_event;
    use eframe::egui::{Event, Key, Modifiers};
    fn key(events: &mut Vec<Event>, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            let mut event = key_event(key, pressed);
            if let Event::Key {
                modifiers: current, ..
            } = &mut event
            {
                *current = modifiers;
            }
            events.push(event);
        }
    }
    let mut events = Vec::new();
    let text = typed(name)?;
    if let Some(text) = text {
        // Start at an idle prompt. Cancellation is a separate probe: ConPTY may
        // flush queued input when it handles Ctrl+C, including text sent after it.
        events.push(Event::Text(text.into()));
    }
    match name {
        "resize-draft" => {
            key(&mut events, Key::ArrowLeft, Modifiers::NONE);
        }
        "resize-edit" => {
            key(&mut events, Key::Backspace, Modifiers::NONE);
            events.push(Event::Text("K".into()));
            key(&mut events, Key::Enter, Modifiers::NONE);
        }
        "typing" => {
            key(&mut events, Key::ArrowLeft, Modifiers::NONE);
            key(&mut events, Key::Backspace, Modifiers::NONE);
            events.push(Event::Text("K".into()));
            key(&mut events, Key::Enter, Modifiers::NONE);
        }
        "status" | "history-seed" | "unicode" | "submit" | "interrupt-start"
        | "interrupt-status" | "interrupt-native" => key(&mut events, Key::Enter, Modifiers::NONE),
        "completion-command" | "completion-parameter" | "completion-path" => {
            key(&mut events, Key::Tab, Modifiers::NONE)
        }
        "completion-backward" => key(&mut events, Key::Tab, Modifiers::SHIFT),
        "completion-argument" => {
            events.push(Event::Text(" 'ROCKET_COMPLETION_OK'".into()));
            key(&mut events, Key::Enter, Modifiers::NONE);
        }
        "history-previous" => key(&mut events, Key::ArrowUp, Modifiers::NONE),
        "history-next" => key(&mut events, Key::ArrowDown, Modifiers::NONE),
        // Match the native egui-winit Ctrl+C event, rather than bypassing its
        // clipboard translation with a synthetic Key::C press.
        "cancel-line" => {
            #[cfg(target_os = "macos")]
            key(&mut events, Key::C, Modifiers::CTRL);
            #[cfg(not(target_os = "macos"))]
            events.push(Event::Copy);
        },
        "copy-shortcut" => events.push(Event::Copy),
        "redraw" => key(&mut events, Key::L, Modifiers::CTRL),
        _ => unreachable!(),
    }
    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_probes_are_exact_without_a_general_command_escape() {
        for probe in [BASIC, EMOJI, MARGIN] {
            assert!(allowed(probe));
            assert!(!allowed(&format!("{probe}; Get-ChildItem")));
            assert!(!allowed(&probe.replace("30", "31")));
        }
        assert!(!allowed("[Console]::Write('arbitrary')"));
    }
    #[test]
    fn keyboard_probes_are_named_and_reject_freeform_input() {
        for name in KEYBOARD_NAMES {
            assert!(keyboard(name).is_some());
        }
        assert!(keyboard("Write-Output 'arbitrary'").is_none());
        assert!(keyboard("typing; exit").is_none());
        assert!(keyboard("tab").is_none());
    }
}
