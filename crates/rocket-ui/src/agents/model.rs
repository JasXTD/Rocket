//! Saved metadata contains no process handles, output, credentials or implicit startup.
use rocket_git::{Error, Result};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const MAX_SESSIONS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provider {
    #[default]
    Claude,
    Codex,
    Custom,
}
impl Provider {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Custom];
    pub fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Custom => "custom",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Custom => "Custom command",
        }
    }
    pub fn installed(self) -> String {
        let path = match self {
            Self::Claude => rocket_git::DraftProvider::Claude.default_program(),
            Self::Codex => rocket_git::DraftProvider::Codex.default_program(),
            Self::Custom => None,
        };
        path.map_or_else(String::new, |p| p.to_string_lossy().into_owned())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    /// Literal argv entries; never a shell command, template or automatic prompt.
    pub arguments: String,
}
impl Command {
    pub fn args(&self) -> Vec<String> {
        self.arguments.lines().map(str::to_owned).collect()
    }
    pub fn validate(&self) -> Result<()> {
        if !rocket_git::valid_custom_program_path(std::path::Path::new(&self.program))
            || self.args().len() > 64
            || self.arguments.len() > 16000
            || self.args().iter().any(|a| !visible(a))
        {
            return Err(invalid(
                "Choose an absolute executable and at most 64 literal argument lines without hidden characters.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    pub name: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub branch: String,
    pub base: String,
    pub provider: Provider,
    pub command: Command,
    pub setup: Option<Command>,
}
impl Record {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > 160
            || !visible(&self.name)
            || !self.source.is_absolute()
            || !self.destination.is_absolute()
            || [&self.branch, &self.base]
                .iter()
                .any(|s| s.is_empty() || s.len() > 1024 || !visible(s))
        {
            return Err(invalid(
                "Enter a task name, branch, base and absolute worktree folder.",
            ));
        }
        self.command.validate()?;
        if let Some(c) = &self.setup {
            c.validate()?;
        }
        Ok(())
    }
}
fn visible(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() || matches!(c,'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}'))
}
pub(super) fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}

pub(crate) fn encode(records: &[Record]) -> std::io::Result<Vec<u8>> {
    if records.len() > MAX_SESSIONS {
        return Err(std::io::Error::other("Too many agent sessions"));
    }
    let mut ids = std::collections::HashSet::new();
    let command = |c: &Command| json!({"program":c.program,"arguments":c.arguments});
    let mut rows = Vec::new();
    for r in records {
        r.validate().map_err(std::io::Error::other)?;
        if r.id == 0
            || r.id >= (1 << 63)
            || !ids.insert(r.id)
            || r.source.to_str().is_none()
            || r.destination.to_str().is_none()
        {
            return Err(std::io::Error::other("Invalid agent session identity"));
        }
        rows.push(json!({"id":r.id,"name":r.name,"source":r.source,"destination":r.destination,"branch":r.branch,"base":r.base,"provider":r.provider.key(),"command":command(&r.command),"setup":r.setup.as_ref().map(command)}));
    }
    serde_json::to_vec(&rows).map_err(std::io::Error::other)
}
pub(crate) fn decode(bytes: &[u8]) -> std::io::Result<Vec<Record>> {
    let bad = || std::io::Error::other("Invalid saved agent sessions");
    let v: Value = serde_json::from_slice(bytes).map_err(std::io::Error::other)?;
    let rows = v.as_array().ok_or_else(bad)?;
    if rows.len() > MAX_SESSIONS {
        return Err(bad());
    }
    let text = |v: &Value, k: &str| v[k].as_str().map(str::to_owned).ok_or_else(bad);
    let command = |v: &Value| -> std::io::Result<Command> {
        Ok(Command {
            program: text(v, "program")?,
            arguments: text(v, "arguments")?,
        })
    };
    let mut records = Vec::new();
    for v in rows {
        records.push(Record {
            id: v["id"].as_u64().ok_or_else(bad)?,
            name: text(v, "name")?,
            source: PathBuf::from(text(v, "source")?),
            destination: PathBuf::from(text(v, "destination")?),
            branch: text(v, "branch")?,
            base: text(v, "base")?,
            provider: Provider::ALL
                .into_iter()
                .find(|p| Some(p.key()) == v["provider"].as_str())
                .ok_or_else(bad)?,
            command: command(&v["command"])?,
            setup: if v["setup"].is_null() {
                None
            } else {
                Some(command(&v["setup"])?)
            },
        });
    }
    encode(&records)?;
    Ok(records)
}
