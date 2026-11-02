//! Stable command IDs and bounded, exact (not subset) input chords.
use eframe::egui::{Key, Modifiers, PointerButton};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    New,
    Open,
    Clone,
    Manager,
    Close,
    Reopen,
    Next,
    Previous,
    Palette,
    Sidebar,
    Activity,
    Terminal,
    Preferences,
    Commit,
    Branch,
    Refresh,
    Fetch,
    Pull,
    Push,
    Changes,
    History,
}
pub struct Command {
    pub action: Action,
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub default: &'static str,
}
pub const COMMANDS: &[Command] = &[
    Command {
        action: Action::New,
        id: "new",
        label: "New repository",
        group: "Repositories",
        default: "Ctrl+N",
    },
    Command {
        action: Action::Open,
        id: "open",
        label: "Open repository",
        group: "Repositories",
        default: "Ctrl+O",
    },
    Command {
        action: Action::Clone,
        id: "clone",
        label: "Clone repository",
        group: "Repositories",
        default: "Ctrl+Shift+O",
    },
    Command {
        action: Action::Manager,
        id: "manager",
        label: "Repository manager",
        group: "Repositories",
        default: "Ctrl+Shift+M",
    },
    Command {
        action: Action::Close,
        id: "close",
        label: "Close repository",
        group: "Repositories",
        default: "Ctrl+W",
    },
    Command {
        action: Action::Reopen,
        id: "reopen",
        label: "Reopen closed repository",
        group: "Repositories",
        default: "Ctrl+Shift+T",
    },
    Command {
        action: Action::Next,
        id: "next",
        label: "Next repository",
        group: "Repositories",
        default: "Ctrl+Tab",
    },
    Command {
        action: Action::Previous,
        id: "previous",
        label: "Previous repository",
        group: "Repositories",
        default: "Ctrl+Shift+Tab",
    },
    Command {
        action: Action::Palette,
        id: "palette",
        label: "Command palette",
        group: "Panels",
        default: "Ctrl+K",
    },
    Command {
        action: Action::Sidebar,
        id: "sidebar",
        label: "Sidebar",
        group: "Panels",
        default: "Ctrl+B",
    },
    Command {
        action: Action::Activity,
        id: "activity",
        label: "Activity",
        group: "Panels",
        default: "Ctrl+`",
    },
    Command {
        action: Action::Terminal,
        id: "terminal",
        label: "Terminal",
        group: "Panels",
        default: "Ctrl+Shift+`",
    },
    Command {
        action: Action::Preferences,
        id: "preferences",
        label: "Preferences",
        group: "Panels",
        default: "Ctrl+,",
    },
    Command {
        action: Action::Changes,
        id: "changes",
        label: "Show changes",
        group: "Panels",
        default: "",
    },
    Command {
        action: Action::History,
        id: "history",
        label: "Show commits",
        group: "Panels",
        default: "",
    },
    Command {
        action: Action::Commit,
        id: "commit",
        label: "Commit staged changes",
        group: "Working",
        default: "Ctrl+Enter",
    },
    Command {
        action: Action::Branch,
        id: "branch",
        label: "New branch",
        group: "Working",
        default: "Ctrl+Shift+B",
    },
    Command {
        action: Action::Refresh,
        id: "refresh",
        label: "Refresh",
        group: "Working",
        default: "F5",
    },
    Command {
        action: Action::Fetch,
        id: "fetch",
        label: "Fetch review",
        group: "Working",
        default: "",
    },
    Command {
        action: Action::Pull,
        id: "pull",
        label: "Pull review",
        group: "Working",
        default: "",
    },
    Command {
        action: Action::Push,
        id: "push",
        label: "Push review",
        group: "Working",
        default: "",
    },
];
impl Action {
    pub fn command(self) -> &'static Command {
        COMMANDS
            .iter()
            .find(|c| c.action == self)
            .expect("known action")
    }
    pub fn from_id(id: &str) -> Option<Self> {
        COMMANDS.iter().find(|c| c.id == id).map(|c| c.action)
    }
    pub fn fixture_safe(self) -> bool {
        matches!(
            self,
            Self::Next
                | Self::Previous
                | Self::Palette
                | Self::Sidebar
                | Self::Activity
                | Self::Preferences
                | Self::Changes
                | Self::History
                | Self::Refresh
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(Key),
    Mouse(PointerButton),
    Wheel(Wheel),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub modifiers: u8,
    pub trigger: Trigger,
}
pub fn modifiers(m: Modifiers) -> Option<u8> {
    (!m.mac_cmd).then_some(u8::from(m.ctrl) | (u8::from(m.alt) * 2) | (u8::from(m.shift) * 4))
}
impl Binding {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > 64 {
            return Err("Choose one bounded keyboard or mouse shortcut.".into());
        }
        let mut rest = text;
        let mut mask = 0;
        while let Some((word, tail)) = rest.split_once('+') {
            let bit = match word {
                "Ctrl" => 1,
                "Alt" => 2,
                "Shift" => 4,
                _ => break,
            };
            if mask & bit != 0 {
                return Err("A modifier is repeated.".into());
            }
            mask |= bit;
            rest = tail;
        }
        let trigger = match rest {
            "MouseLeft" => Trigger::Mouse(PointerButton::Primary),
            "MouseRight" => Trigger::Mouse(PointerButton::Secondary),
            "MouseMiddle" => Trigger::Mouse(PointerButton::Middle),
            "MouseBack" => Trigger::Mouse(PointerButton::Extra1),
            "MouseForward" => Trigger::Mouse(PointerButton::Extra2),
            "WheelUp" => Trigger::Wheel(Wheel::Up),
            "WheelDown" => Trigger::Wheel(Wheel::Down),
            "WheelLeft" => Trigger::Wheel(Wheel::Left),
            "WheelRight" => Trigger::Wheel(Wheel::Right),
            _ => Trigger::Key(Key::from_name(rest).ok_or("Unknown key or mouse input.")?),
        };
        Ok(Self {
            modifiers: mask,
            trigger,
        })
    }
    pub fn text(self) -> String {
        let mut pieces = vec![];
        for (bit, name) in [(1, "Ctrl"), (2, "Alt"), (4, "Shift")] {
            if self.modifiers & bit != 0 {
                pieces.push(name)
            }
        }
        pieces.push(match self.trigger {
            Trigger::Key(k) => k.name(),
            Trigger::Mouse(b) => match b {
                PointerButton::Primary => "MouseLeft",
                PointerButton::Secondary => "MouseRight",
                PointerButton::Middle => "MouseMiddle",
                PointerButton::Extra1 => "MouseBack",
                PointerButton::Extra2 => "MouseForward",
            },
            Trigger::Wheel(w) => match w {
                Wheel::Up => "WheelUp",
                Wheel::Down => "WheelDown",
                Wheel::Left => "WheelLeft",
                Wheel::Right => "WheelRight",
            },
        });
        pieces.join("+")
    }
    pub fn validate(self) -> Result<(), String> {
        if self.modifiers > 7 {
            return Err("Unsupported modifier.".into());
        }
        match self.trigger {
            Trigger::Mouse(PointerButton::Extra1|PointerButton::Extra2)=>Ok(()),
            Trigger::Mouse(_)|Trigger::Wheel(_) if self.modifiers!=0=>Ok(()),
            Trigger::Mouse(_)|Trigger::Wheel(_)=>Err("Add Ctrl, Alt or Shift. Plain clicks and wheel scrolling keep their normal behavior.".into()),
            Trigger::Key(k)=>{
                let m=self.modifiers;let function=k.name().starts_with('F')&&k.name()[1..].parse::<u8>().is_ok();
                if matches!(k,Key::Escape|Key::F1) || (m&2!=0&&matches!(k,Key::Tab|Key::F4|Key::Space))
                    || (m&1!=0&&matches!(k,Key::C|Key::X|Key::V|Key::A|Key::Z|Key::Y|Key::Insert|Key::Backspace|Key::Delete|Key::Plus|Key::Minus|Key::Equals|Key::Num0|Key::ArrowLeft|Key::ArrowRight|Key::ArrowUp|Key::ArrowDown|Key::Home|Key::End))
                    || (m&3==3&&!function) {
                    return Err("Reserved for text editing, AltGr, zoom, Help or the operating system.".into())
                }
                if m&3==0&&!function{return Err("Use Ctrl or Alt with this key so ordinary typing and focus navigation stay available.".into())}Ok(())
            }
        }
    }
    pub fn egui_modifiers(self) -> Modifiers {
        Modifiers {
            ctrl: self.modifiers & 1 != 0,
            command: self.modifiers & 1 != 0,
            alt: self.modifiers & 2 != 0,
            shift: self.modifiers & 4 != 0,
            ..Modifiers::NONE
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings(pub BTreeMap<String, Vec<Binding>>);
impl Bindings {
    pub fn get(&self, action: Action) -> Vec<Binding> {
        self.0.get(action.command().id).cloned().unwrap_or_else(|| {
            let d = action.command().default;
            if d.is_empty() {
                vec![]
            } else {
                vec![Binding::parse(d).expect("valid built-in shortcut")]
            }
        })
    }
    pub fn label(&self, action: Action) -> String {
        self.get(action)
            .iter()
            .map(|b| b.text())
            .collect::<Vec<_>>()
            .join(" / ")
    }
    pub fn find(&self, b: Binding) -> Option<Action> {
        COMMANDS
            .iter()
            .find(|c| self.get(c.action).contains(&b))
            .map(|c| c.action)
    }
    pub fn set(&mut self, action: Action, bindings: Vec<Binding>) -> Result<(), String> {
        if bindings.len() > 2 {
            return Err("A command supports up to two shortcuts.".into());
        }
        for (i, b) in bindings.iter().enumerate() {
            b.validate()?;
            if bindings[..i].contains(b) {
                return Err("This shortcut is already on this command.".into());
            }
            if let Some(other) = COMMANDS
                .iter()
                .find(|c| c.action != action && self.get(c.action).contains(b))
            {
                return Err(format!(
                    "Already used by {}. Choose another input or clear that binding first.",
                    other.label
                ));
            }
        }
        let defaults = Self::default().get(action);
        if bindings == defaults {
            self.0.remove(action.command().id);
        } else {
            self.0.insert(action.command().id.into(), bindings);
        }
        Ok(())
    }
    pub fn reset(&mut self, action: Action) -> Result<(), String> {
        self.set(action, Self::default().get(action))
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let entries:Vec<_>=self.0.iter().map(|(id,v)|json!({"command":id,"bindings":v.iter().map(|b|b.text()).collect::<Vec<_>>()})).collect();
        serde_json::to_vec(&entries).map_err(|_| "Cannot encode shortcuts.".into())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 8192 {
            return Err("Shortcut preferences exceed their limit.".into());
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| "Invalid shortcut preferences.")?;
        let entries = value
            .as_array()
            .filter(|v| v.len() <= COMMANDS.len())
            .ok_or("Invalid shortcut entries.")?;
        let mut out = Self::default();
        for e in entries {
            if e.as_object().is_none_or(|v| v.len() != 2) {
                return Err("Invalid shortcut entry.".into());
            }
            let id = e["command"]
                .as_str()
                .filter(|s| Action::from_id(s).is_some())
                .ok_or("Unknown shortcut command.")?;
            let values = e["bindings"]
                .as_array()
                .filter(|v| v.len() <= 2)
                .ok_or("Invalid shortcut bindings.")?;
            let v = values
                .iter()
                .map(|v| Binding::parse(v.as_str().ok_or("Invalid shortcut chord.")?))
                .collect::<Result<Vec<_>, _>>()?;
            if out.0.insert(id.into(), v).is_some() {
                return Err("Duplicate shortcut command.".into());
            }
        }
        out.validate()?;
        Ok(out)
    }
    fn validate(&self) -> Result<(), String> {
        if self.0.len() > COMMANDS.len() || self.0.keys().any(|id| Action::from_id(id).is_none()) {
            return Err("Unknown shortcut command.".into());
        }
        let mut seen = vec![];
        for c in COMMANDS {
            let values = self.get(c.action);
            if values.len() > 2 {
                return Err("Too many shortcuts.".into());
            }
            for b in values {
                b.validate()?;
                if seen.contains(&b) {
                    return Err("Duplicate shortcut bindings.".into());
                }
                seen.push(b)
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_mouse_bindings_roundtrip_without_prefix_matches() {
        let mut b = Bindings::default();
        assert_eq!(Bindings::decode(&b.encode().unwrap()).unwrap(), b);
        assert_eq!(
            b.find(Binding::parse("Ctrl+B").unwrap()),
            Some(Action::Sidebar)
        );
        assert_eq!(
            b.find(Binding::parse("Ctrl+Shift+B").unwrap()),
            Some(Action::Branch)
        );
        b.set(
            Action::Next,
            vec![
                Binding::parse("Ctrl+Tab").unwrap(),
                Binding::parse("MouseForward").unwrap(),
            ],
        )
        .unwrap();
        b.set(
            Action::Previous,
            vec![Binding::parse("Ctrl+Shift+WheelUp").unwrap()],
        )
        .unwrap();
        assert_eq!(Bindings::decode(&b.encode().unwrap()).unwrap(), b);
    }
    #[test]
    fn duplicates_reserved_inputs_and_invalid_saved_data_are_refused() {
        let mut b = Bindings::default();
        let original = b.clone();
        assert!(
            b.set(Action::Next, vec![Binding::parse("Ctrl+O").unwrap()])
                .is_err()
        );
        assert_eq!(b, original);
        for text in [
            "MouseLeft",
            "MouseRight",
            "MouseMiddle",
            "WheelDown",
            "A",
            "Shift+A",
            "Esc",
            "Ctrl+C",
            "Ctrl+ArrowLeft",
            "Alt+F4",
            "Ctrl+Alt+E",
            "Ctrl++",
            "F1",
        ] {
            assert!(Binding::parse(text).unwrap().validate().is_err(), "{text}")
        }
        for text in [
            "MouseBack",
            "Shift+MouseLeft",
            "Ctrl+MouseRight",
            "Alt+MouseMiddle",
            "Ctrl+WheelDown",
            "Ctrl+Alt+F9",
        ] {
            Binding::parse(text).unwrap().validate().unwrap()
        }
        for text in [
            r#"[{"command":"unknown","bindings":[]}]"#,
            r#"[{"command":"open","bindings":[]},{"command":"open","bindings":[]}]"#,
            r#"[{"command":"open","bindings":["Ctrl+B"]}]"#,
        ] {
            assert!(Bindings::decode(text.as_bytes()).is_err())
        }
        b.set(Action::Open, vec![]).unwrap();
        b.set(Action::Next, vec![Binding::parse("Ctrl+O").unwrap()])
            .unwrap();
        assert!(b.reset(Action::Open).is_err());
        assert_eq!(
            b.find(Binding::parse("Ctrl+O").unwrap()),
            Some(Action::Next)
        );
    }
}
