//! User-owned tool profiles. Repository configuration never supplies an executable.
use crate::known_tools::KnownTool;
use rocket_git::{Error, ExternalToolConfig, ExternalToolKind, Result};
use std::path::Path;

pub const MAX_PROFILES: usize = 16;
pub const KINDS: [ExternalToolKind; 3] = [
    ExternalToolKind::Editor,
    ExternalToolKind::Diff,
    ExternalToolKind::Merge,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub config: ExternalToolConfig,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub entries: Vec<Profile>,
    pub default: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles(pub [Group; 3]);

impl Default for Profiles {
    fn default() -> Self {
        Self::from_legacy(&std::array::from_fn(|_| ExternalToolConfig::default()))
    }
}
impl Profiles {
    pub fn from_legacy(configs: &[ExternalToolConfig; 3]) -> Self {
        Self(KINDS.map(|kind| {
            let mut config = configs[kind.index()].clone();
            if config.executable.is_empty() && config.arguments.is_empty() {
                config.arguments = kind.placeholders().join("\n");
            }
            Group {
                entries: vec![Profile {
                    name: "Default".into(),
                    config,
                }],
                default: 0,
            }
        }))
    }
    pub fn defaults(&self) -> [ExternalToolConfig; 3] {
        std::array::from_fn(|i| self.0[i].entries[self.0[i].default].config.clone())
    }
    pub fn selections(&self) -> [usize; 3] {
        std::array::from_fn(|i| self.0[i].default)
    }
    pub fn validate(&self) -> Result<()> {
        for kind in KINDS {
            let group = &self.0[kind.index()];
            if group.entries.is_empty()
                || group.entries.len() > MAX_PROFILES
                || group.default >= group.entries.len()
            {
                return Err(Error::InvalidInput(
                    "Keep 1–16 tool profiles per type and choose an existing default.".into(),
                ));
            }
            let mut names = std::collections::HashSet::new();
            for profile in &group.entries {
                if profile.name.trim().is_empty()
                    || profile.name.chars().count() > 80
                    || profile.name.chars().any(char::is_control)
                    || !names.insert(profile.name.trim().to_lowercase())
                {
                    return Err(Error::InvalidInput(
                        "Use a unique profile name of 1–80 characters within each tool type."
                            .into(),
                    ));
                }
                profile.config.validate(kind)?;
            }
        }
        Ok(())
    }
}
impl Profile {
    /// Nothing chosen yet: no program, and the arguments a new profile starts with.
    fn untouched(&self, kind: ExternalToolKind) -> bool {
        self.config.executable.trim().is_empty() && self.config.arguments == kind.placeholders().join("\n")
    }
}
impl Group {
    /// The profile that already holds a known tool: its program, or its name while the
    /// program is still to be chosen.
    pub fn holding(&self, tool: &KnownTool, program: Option<&Path>) -> Option<usize> {
        self.entries.iter().position(|profile| {
            let executable = profile.config.executable.trim();
            if executable.is_empty() {
                profile.name.trim().eq_ignore_ascii_case(tool.name)
            } else {
                program.is_some_and(|path| path.to_string_lossy().eq_ignore_ascii_case(executable))
            }
        })
    }
    /// Puts a known tool in the form and returns the profile to select. An empty selected
    /// profile takes it, so the first Default stops being a blank; otherwise it is a new
    /// profile, and a tool already here just selects its own. Nothing is saved or run.
    pub fn use_known(
        &mut self,
        selected: usize,
        kind: ExternalToolKind,
        tool: &KnownTool,
        program: Option<&Path>,
    ) -> Option<usize> {
        if let Some(index) = self.holding(tool, program) {
            return Some(index);
        }
        let target = if self.entries.get(selected).is_some_and(|p| p.untouched(kind)) {
            selected
        } else if self.entries.len() < MAX_PROFILES {
            self.entries.len()
        } else {
            return None;
        };
        let name = (1..)
            .map(|i| if i == 1 { tool.name.to_owned() } else { format!("{} {i}", tool.name) })
            .find(|name| {
                !self.entries.iter().enumerate().any(|(i, p)| i != target && p.name.trim().eq_ignore_ascii_case(name))
            })
            .unwrap();
        let profile = Profile {
            name,
            config: ExternalToolConfig {
                executable: program.map_or_else(String::new, |path| path.display().to_string()),
                arguments: tool.arguments(kind).join("\n"),
            },
        };
        if target == self.entries.len() {
            self.entries.push(profile);
        } else {
            self.entries[target] = profile;
        }
        Some(target)
    }
    /// Returns a new unsaved selection. Names remain unique even after deletions.
    pub fn add(&mut self, kind: ExternalToolKind, copy: Option<usize>) -> Option<usize> {
        if self.entries.len() >= MAX_PROFILES {
            return None;
        }
        let config = copy
            .and_then(|i| self.entries.get(i))
            .map(|p| p.config.clone())
            .unwrap_or_else(|| ExternalToolConfig {
                executable: String::new(),
                arguments: kind.placeholders().join("\n"),
            });
        let name = (1..)
            .map(|i| format!("{} profile {i}", kind.label()))
            .find(|name| {
                !self
                    .entries
                    .iter()
                    .any(|p| p.name.trim().eq_ignore_ascii_case(name))
            })
            .unwrap();
        self.entries.push(Profile { name, config });
        Some(self.entries.len() - 1)
    }
    pub fn remove(&mut self, index: usize) -> Option<usize> {
        if self.entries.len() <= 1 || index >= self.entries.len() {
            return None;
        }
        self.entries.remove(index);
        self.default = if self.default == index {
            0
        } else {
            self.default - usize::from(self.default > index)
        };
        Some(index.min(self.entries.len() - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_and_profile_edits_keep_programs_and_defaults() {
        let old = [
            ExternalToolConfig::default(),
            ExternalToolConfig {
                executable: "D:/Tools/比較.exe".into(),
                arguments: "{left}\n{right}".into(),
            },
            ExternalToolConfig::default(),
        ];
        let mut profiles = Profiles::from_legacy(&old);
        assert_eq!(profiles.defaults()[1], old[1]);
        let group = &mut profiles.0[1];
        assert_eq!(group.add(ExternalToolKind::Diff, Some(0)), Some(1));
        group.entries[1].name = "比較 🚀".into();
        group.default = 1;
        assert_eq!(group.entries[1].config, old[1]);
        assert_eq!(group.remove(0), Some(0));
        assert_eq!(group.default, 0);
        assert_eq!(group.entries[0].name, "比較 🚀");
        assert_eq!(group.remove(0), None);
        profiles.validate().unwrap();
    }
    #[test]
    fn a_known_tool_fills_an_empty_profile_or_adds_one_and_is_never_added_twice() {
        use crate::known_tools::named;
        let kind = ExternalToolKind::Merge;
        let code = named("VS Code", kind).unwrap();
        let path = Path::new(r"C:\Tools\Code.exe");
        let mut profiles = Profiles::default();
        let group = &mut profiles.0[kind.index()];
        assert_eq!(group.use_known(0, kind, code, Some(path)), Some(0), "the empty Default takes it");
        assert_eq!(group.entries.len(), 1);
        assert_eq!(group.entries[0].name, "VS Code");
        assert_eq!(group.entries[0].config.executable, r"C:\Tools\Code.exe");
        assert_eq!(group.entries[0].config.arguments, code.arguments(kind).join("\n"));
        assert_eq!(group.default, 0);
        assert_eq!(group.use_known(0, kind, code, Some(path)), Some(0), "already here");
        assert_eq!(group.entries.len(), 1);
        // Not found: the name and arguments still go in, for Browse to finish.
        let meld = named("Meld", kind).unwrap();
        assert_eq!(group.use_known(0, kind, meld, None), Some(1), "a used profile stays as it is");
        assert_eq!(group.entries[1].config.executable, "");
        assert_eq!(group.entries[1].config.arguments, meld.arguments(kind).join("\n"));
        assert_eq!(group.default, 0, "adding doesn't move the default");
        assert_eq!(group.use_known(0, kind, meld, None), Some(1), "found again by its name");
        profiles.validate().unwrap();
        // Another copy at another path gets its own unique name.
        let group = &mut profiles.0[kind.index()];
        let other = Path::new(r"D:\Portable\Code.exe");
        assert_eq!(group.use_known(0, kind, code, Some(other)), Some(2));
        assert_eq!(group.entries[2].name, "VS Code 2");
        profiles.validate().unwrap();
        let group = &mut profiles.0[kind.index()];
        while group.add(kind, Some(0)).is_some() {}
        let kdiff = named("KDiff3", kind).unwrap();
        assert_eq!(group.use_known(0, kind, kdiff, None), None, "the list is full");
    }
    #[test]
    fn invalid_names_limits_and_commands_are_rejected() {
        let mut profiles = Profiles::default();
        profiles.0[1].add(ExternalToolKind::Diff, None);
        profiles.0[1].entries[1].name = " default ".into();
        assert!(profiles.validate().is_err());
        profiles.0[1].entries[1].name = "Comparison".into();
        profiles.0[1].entries[1].config.executable = "relative.exe".into();
        assert!(profiles.validate().is_err());
        profiles.0[1].entries[1].config.executable.clear();
        while profiles.0[1].add(ExternalToolKind::Diff, None).is_some() {}
        assert_eq!(profiles.0[1].entries.len(), MAX_PROFILES);
        profiles.validate().unwrap();
        profiles.0[1].default = MAX_PROFILES;
        assert!(profiles.validate().is_err());
    }
}
