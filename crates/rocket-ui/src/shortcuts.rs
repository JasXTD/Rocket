//! User bindings share the ordinary command dispatch and its write/review guards.
mod editor;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    app::{DialogKind, Intent, Rocket, View},
    backend::Action as GitAction,
    preferences,
};
use eframe::egui::*;
use model::{Action, Binding, Bindings, COMMANDS, Trigger, Wheel, modifiers};
use serde_json::{Value, json};

pub struct Edit {
    pub action: Action,
    pub index: usize,
    pub recording: bool,
    pub candidate: Option<Binding>,
    pub error: Option<String>,
    pub serial: u64,
}
#[derive(Default)]
pub struct State {
    pub bindings: Bindings,
    pub edit: Option<Edit>,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub reset_all: bool,
    /// Waiting for one input to find the command that uses it; never binds it.
    pub finding: Option<u64>,
    /// The input found that way; the table lists only the command using it.
    pub found: Option<Binding>,
    scroll_editor: bool,
    next_serial: u64,
    blocked_buttons: Vec<PointerButton>,
    blocked_keys: Vec<Key>,
    pending: Option<Action>,
    wheel: Option<(Binding, f32)>,
}
impl State {
    pub fn record(&mut self, action: Action, index: usize) {
        self.scroll_editor = true;
        self.next_serial = self.next_serial.wrapping_add(1);
        self.edit = Some(Edit {
            action,
            index,
            recording: true,
            candidate: None,
            error: None,
            serial: self.next_serial,
        });
        self.error = None;
        self.notice = None;
        self.reset_all = false;
        self.finding = None;
    }
    /// Waits for one chord or gesture to search by; Esc or a plain click cancels.
    pub fn find(&mut self) {
        self.next_serial = self.next_serial.wrapping_add(1);
        self.finding = Some(self.next_serial);
        self.found = None;
        self.edit = None;
        self.reset_all = false;
    }
    pub fn recording(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.recording)
    }
    /// Waiting for one input: to record it into a cell or to find a command by it.
    pub fn capturing(&self) -> bool {
        self.recording() || self.finding.is_some()
    }
    /// The serial automation sends with input for the capture that is waiting.
    pub fn capture_serial(&self) -> Option<u64> {
        self.finding
            .or(self.edit.as_ref().filter(|e| e.recording).map(|e| e.serial))
    }
    pub fn snapshot(&self) -> Value {
        json!({"bindings":COMMANDS.iter().map(|c|json!({"id":c.id,"label":c.label,"inputs":self.bindings.get(c.action).iter().map(|b|b.text()).collect::<Vec<_>>(),"custom":self.bindings.0.contains_key(c.id)})).collect::<Vec<_>>(),"editor":self.edit.as_ref().map(|e|json!({"action":e.action.command().id,"index":e.index,"recording":e.recording,"candidate":e.candidate.map(|b|b.text()),"error":e.error,"serial":e.serial})),"error":self.error,"reset_confirmation":self.reset_all,"finding":self.finding,"found":self.found.map(|b|b.text())})
    }
    fn capture(&mut self, binding: Binding) {
        let Some(e) = &mut self.edit else { return };
        e.recording = false;
        e.candidate = Some(binding);
        let mut proposed = self.bindings.get(e.action);
        if e.index < proposed.len() {
            proposed[e.index] = binding
        } else {
            proposed.push(binding)
        }
        e.error = self.bindings.clone().set(e.action, proposed).err();
    }
    fn filter(&mut self, input: &mut RawInput, allowed: &[Action], pointer_allowed: bool) {
        self.pending = None;
        if !input.focused {
            self.blocked_buttons.clear();
            self.blocked_keys.clear();
            self.wheel = None;
            return;
        }
        let capturing = self.capturing();
        let mut chosen = false;
        input.events.retain(|event| {
            match event {
                Event::PointerButton {
                    button,
                    pressed: false,
                    ..
                } if self.blocked_buttons.contains(button) => {
                    self.blocked_buttons.retain(|b| b != button);
                    return false;
                }
                Event::Key {
                    key,
                    pressed: false,
                    ..
                } if self.blocked_keys.contains(key) => {
                    self.blocked_keys.retain(|k| k != key);
                    return false;
                }
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } if self.edit.is_some() || self.finding.is_some() => {
                    self.edit = None;
                    self.finding = None;
                    self.blocked_keys.push(Key::Escape);
                    return false;
                }
                Event::Text(_) | Event::Paste(_) if capturing => return false,
                _ => (),
            }
            if !capturing
                && !pointer_allowed
                && matches!(
                    event,
                    Event::PointerButton { .. } | Event::MouseWheel { .. }
                )
            {
                self.wheel = None;
                return true;
            }
            let binding = match event {
                Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers: m,
                    ..
                } => modifiers(*m).map(|modifiers| Binding {
                    modifiers,
                    trigger: Trigger::Key(*key),
                }),
                Event::PointerButton {
                    button,
                    pressed: true,
                    modifiers: m,
                    ..
                } => modifiers(*m).map(|modifiers| Binding {
                    modifiers,
                    trigger: Trigger::Mouse(*button),
                }),
                Event::MouseWheel {
                    delta,
                    modifiers: m,
                    ..
                } if delta.length_sq() > 0.0 => modifiers(*m).map(|modifiers| Binding {
                    modifiers,
                    trigger: Trigger::Wheel(if delta.y.abs() >= delta.x.abs() {
                        if delta.y > 0.0 {
                            Wheel::Up
                        } else {
                            Wheel::Down
                        }
                    } else if delta.x > 0.0 {
                        Wheel::Left
                    } else {
                        Wheel::Right
                    }),
                }),
                // Bound keys never auto-repeat writes or accumulate delayed actions.
                Event::Key {
                    key,
                    pressed: true,
                    repeat: true,
                    modifiers: m,
                    ..
                } => {
                    return !self.blocked_keys.contains(key)
                        && !modifiers(*m).is_some_and(|m| {
                            self.bindings
                                .find(Binding {
                                    modifiers: m,
                                    trigger: Trigger::Key(*key),
                                })
                                .is_some_and(|a| allowed.contains(&a))
                        });
                }
                _ => None,
            };
            let Some(b) = binding else { return true };
            // Plain wheel scrolling cannot be bound, so it still scrolls while finding.
            if self.finding.is_some() && b.modifiers == 0 && matches!(b.trigger, Trigger::Wheel(_)) {
                return true;
            }
            let action = self.bindings.find(b).filter(|a| allowed.contains(a));
            if !capturing && action.is_none() {
                self.wheel = None;
                return true;
            }
            match b.trigger {
                Trigger::Mouse(button) => self.blocked_buttons.push(button),
                Trigger::Key(key) => self.blocked_keys.push(key),
                _ => (),
            }
            if capturing {
                if !chosen && self.finding.take().is_some() {
                    // A plain click leaves finding, as Esc does; it cannot be bound.
                    let plain_click = b.modifiers == 0
                        && matches!(b.trigger, Trigger::Mouse(PointerButton::Primary | PointerButton::Secondary | PointerButton::Middle));
                    self.found = (!plain_click).then_some(b);
                    chosen = true
                } else if !chosen && self.recording() {
                    self.capture(b);
                    chosen = true
                }
                return false;
            }
            if let Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta,
                phase,
                ..
            } = event
            {
                let amount = delta.x.abs().max(delta.y.abs());
                let sum = self
                    .wheel
                    .filter(|(last, _)| *last == b)
                    .map_or(0.0, |(_, n)| n)
                    + amount;
                self.wheel = Some((b, sum));
                if *phase == TouchPhase::End {
                    self.wheel = None
                }
                if sum < 40.0 {
                    return false;
                }
                self.wheel = None;
            }
            if self.pending.is_none() {
                self.pending = action
            }
            false
        });
    }
}
impl Rocket {
    /// Only the native fixture host uses this predicate. Arbitrary shortcut
    /// probes must never become another route to Git writes or shell commands.
    pub fn shortcut_probe_safe(&self, binding: Binding) -> bool {
        if self.blocking_dialog() || self.quitting || self.guide.is_some() {
            return false;
        }
        if self.terminal_focused(&self.context) && binding.text() != "Ctrl+B" {
            return false;
        }
        match self.keybindings.bindings.find(binding) {
            Some(action) => action.fixture_safe() && self.shortcut_available(action),
            None if matches!(binding.trigger, Trigger::Key(_)) => Bindings::default()
                .find(binding)
                .is_some_and(Action::fixture_safe),
            None => false,
        }
    }
    pub fn shortcut_recording(&self) -> bool {
        self.shortcut_editor_visible() && self.keybindings.capturing()
    }
    fn shortcut_editor_visible(&self) -> bool {
        self.dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Preferences))
            && self.preferences.page == preferences::Page::Shortcuts
    }
    pub fn shortcut_raw_input(&mut self, ctx: &Context, input: &mut RawInput) {
        if input.viewport_id != ViewportId::ROOT {
            return;
        }
        if !self.shortcut_editor_visible() {
            self.keybindings.edit = None;
            self.keybindings.finding = None;
            self.keybindings.reset_all = false
        }
        let over_terminal = input
            .events
            .iter()
            .filter_map(|e| match e {
                Event::PointerMoved(p) | Event::PointerButton { pos: p, .. } => Some(*p),
                _ => None,
            })
            .next_back()
            .or_else(|| ctx.pointer_hover_pos())
            .is_some_and(|pos| {
                ["terminal-screen", "terminal-command"].iter().any(|id| {
                    self.frame_widgets
                        .get(*id)
                        .is_some_and(|c| c.rect.contains(pos))
                })
            });
        let allowed: Vec<_> = COMMANDS
            .iter()
            .filter(|c| {
                self.shortcut_available(c.action)
                    && (!self.terminal_focused(ctx) || c.action == Action::Terminal)
            })
            .map(|c| c.action)
            .collect();
        // Preserve event order and swallow a previously captured button's release
        // even when the pointer has since moved into the terminal.
        self.keybindings.filter(input, &allowed, !over_terminal);
    }
    fn shortcut_available(&self, a: Action) -> bool {
        if self.quitting
            || self.blocking_dialog()
            || self.guide.is_some()
            || (self.palette.open && a != Action::Palette)
        {
            return false;
        }
        match a {
            Action::New | Action::Clone | Action::Manager => {
                self.backend.is_some() && self.write.is_none()
            }
            Action::Open => self.backend.is_some(),
            Action::Close | Action::Terminal | Action::Changes | Action::History => {
                self.tab().is_some()
            }
            Action::Next | Action::Previous => self.tabs.len() > 1,
            Action::Reopen => !self.closed_tabs.is_empty(),
            Action::Branch | Action::Fetch | Action::Pull | Action::Push => self.can_write(),
            Action::Commit => self.page().is_none() && self.can_write(),
            Action::Refresh => {
                self.backend.is_some() && self.tab().is_some() && self.write.is_none()
            }
            _ => true,
        }
    }
    pub(crate) fn run_shortcuts(&mut self, ctx: &Context, intents: &mut Vec<Intent>) {
        // The fallback also serves headless callers using egui's real key events.
        // Native input is normally filtered before egui so a mouse gesture never
        // also activates the widget or scroll/zoom beneath the pointer.
        let pending = self.keybindings.pending.take();
        let terminal_focused = self.terminal_focused(ctx);
        let action = pending.or_else(|| {
            ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers: m,
                        ..
                    } = e
                    {
                        modifiers(*m)
                            .and_then(|modifiers| {
                                self.keybindings.bindings.find(Binding {
                                    modifiers,
                                    trigger: Trigger::Key(*key),
                                })
                            })
                            .filter(|a| {
                                self.shortcut_available(*a)
                                    && (!terminal_focused || *a == Action::Terminal)
                            })
                    } else {
                        None
                    }
                })
            })
        });
        let Some(a) = action.filter(|a| self.shortcut_available(*a)) else {
            return;
        };
        if pending.is_none() {
            for b in self.keybindings.bindings.get(a) {
                if let Trigger::Key(key) = b.trigger {
                    ctx.input_mut(|i| {
                        i.consume_key(b.egui_modifiers(), key);
                    });
                }
            }
        }
        match a {
            Action::New => intents.push(Intent::Dialog(DialogKind::NewRepository)),
            Action::Open => intents.push(Intent::Dialog(DialogKind::Open)),
            Action::Clone => intents.push(Intent::Dialog(DialogKind::Clone)),
            Action::Manager => intents.push(Intent::Dialog(DialogKind::RepositoryManager)),
            Action::Close => {
                if let Some(t) = self.tab() {
                    intents.push(Intent::CloseTab(t.id))
                }
            }
            Action::Reopen => self.reopen_closed_tab(),
            Action::Next => self.active = (self.active + 1) % self.tabs.len(),
            Action::Previous => self.active = (self.active + self.tabs.len() - 1) % self.tabs.len(),
            Action::Palette => self.palette.toggle(),
            Action::Sidebar => self.show_sidebar = !self.show_sidebar,
            Action::Activity => self.toggle_activity(),
            Action::Terminal => intents.push(Intent::Terminal),
            Action::Preferences => intents.push(Intent::Dialog(DialogKind::Preferences)),
            Action::Branch => intents.push(Intent::Dialog(DialogKind::NewBranch)),
            Action::Refresh => intents.push(Intent::Refresh),
            Action::Fetch => intents.push(Intent::Dialog(DialogKind::Fetch)),
            Action::Pull => intents.push(Intent::Dialog(DialogKind::Pull)),
            Action::Push => intents.push(Intent::Dialog(DialogKind::Push)),
            Action::Changes => intents.push(Intent::View(View::Changes)),
            Action::History => intents.push(Intent::View(View::Commits)),
            Action::Commit => {
                if let Some(t) = self.tab() {
                    if let Some(intent) = t.continue_paused().filter(|_| t.view == View::Changes) {
                        intents.push(intent);
                    } else if t.view == View::Changes && t.can_commit() {
                        let merge = t
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.state.as_ref())
                            .is_some_and(|s| !s.merge_heads.is_empty());
                        intents.push(if !merge && self.general.push_after_commit && t.commit_pushes() {
                            Intent::CommitPush
                        } else {
                            Intent::Execute(
                                t.id,
                                if merge {
                                    GitAction::ContinueMerge(t.draft.clone())
                                } else {
                                    GitAction::Commit(t.draft.clone())
                                },
                            )
                        });
                    } else {
                        intents.push(Intent::View(View::Changes));
                    }
                }
            }
        }
    }
}
/// Existing menu/keycap call sites resolve their old hint through this catalog.
/// Installation is per egui Context, never a process-global mutable binding map.
pub fn hint(ctx: &Context, old: &str) -> String {
    let Some(command) = COMMANDS
        .iter()
        .find(|c| !c.default.is_empty() && c.default == old)
    else {
        return old.into();
    };
    ctx.data(|d| d.get_temp::<Bindings>(Id::new("rocket-shortcut-hints")))
        .unwrap_or_default()
        .label(command.action)
}
