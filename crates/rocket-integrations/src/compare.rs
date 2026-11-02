//! What changed between two copies of a package, for the update review.
use crate::{Command, Target};

/// Lines of unchanged code kept around each change.
const CONTEXT: usize = 3;
/// The largest comparison table. Beyond it the changed middle counts as replaced whole.
const CELLS: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Same,
    Added,
    Removed,
    /// Unchanged lines left out between two changes.
    Gap,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// In the new source; a removed line has its number in the old source.
    pub number: usize,
    pub text: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceChange {
    pub added: usize,
    pub removed: usize,
    /// How many separate places changed.
    pub changes: usize,
    pub lines: Vec<Line>,
}

/// The middle of the two sources as edits, by the longest common run of lines.
fn edits(old: &[&str], new: &[&str]) -> Vec<LineKind> {
    let (n, m) = (old.len(), new.len());
    if (n + 1).saturating_mul(m + 1) > CELLS {
        let mut script = vec![LineKind::Removed; n];
        script.resize(n + m, LineKind::Added);
        return script;
    }
    // Cell (i, j): the common lines of old[i..] and new[j..]. At most CELLS cells means the
    // shorter side has at most 2,000 lines, so the counts fit.
    let mut table = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if old[i] == new[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut script = Vec::with_capacity(n + m);
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            script.push(LineKind::Same);
            i += 1;
            j += 1;
        } else if i < n && (j == m || table[at(i + 1, j)] >= table[at(i, j + 1)]) {
            script.push(LineKind::Removed);
            i += 1;
        } else {
            script.push(LineKind::Added);
            j += 1;
        }
    }
    script
}

/// What happened to a command between two manifests, matched by its ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandChange {
    Same,
    New,
    /// What it was, for each part that changed.
    Changed { label: Option<String>, target: Option<Option<Target>> },
    Removed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    pub command: Command,
    pub change: CommandChange,
}

/// `new` against `old`, in the order a review reads them: new, changed, removed, then what
/// stayed.
pub fn compare_commands(old: &[Command], new: &[Command]) -> Vec<CommandLine> {
    let mut lines: Vec<CommandLine> = new
        .iter()
        .map(|command| {
            let change = match old.iter().find(|c| c.id == command.id) {
                None => CommandChange::New,
                Some(before) if before == command => CommandChange::Same,
                Some(before) => CommandChange::Changed {
                    label: (before.label != command.label).then(|| before.label.clone()),
                    target: (before.target != command.target).then_some(before.target),
                },
            };
            CommandLine { command: command.clone(), change }
        })
        .collect();
    lines.extend(old.iter().filter(|c| !new.iter().any(|n| n.id == c.id)).map(|command| CommandLine {
        command: command.clone(),
        change: CommandChange::Removed,
    }));
    let rank = |line: &CommandLine| match line.change {
        CommandChange::New => 0,
        CommandChange::Changed { .. } => 1,
        CommandChange::Removed => 2,
        CommandChange::Same => 3,
    };
    lines.sort_by_key(rank);
    lines
}

/// The changed lines between `old` and `new` with a little context, in file order.
pub fn compare_source(old: &str, new: &str) -> SourceChange {
    let (old, new): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut script = vec![LineKind::Same; prefix];
    script.extend(edits(&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]));
    script.resize(script.len() + suffix, LineKind::Same);

    // Which rows of the script are shown: every change and the lines around it.
    let mut shown = vec![false; script.len()];
    for (row, kind) in script.iter().enumerate() {
        if *kind != LineKind::Same {
            let last = (row + CONTEXT + 1).min(script.len());
            shown[row.saturating_sub(CONTEXT)..last].fill(true);
        }
    }
    let mut change = SourceChange::default();
    let (mut before, mut after) = (0, 0);
    let mut previous = LineKind::Same;
    for (row, kind) in script.iter().enumerate() {
        let (number, text) = match kind {
            LineKind::Removed => (before + 1, old[before]),
            _ => (after + 1, new.get(after).copied().unwrap_or("")),
        };
        match kind {
            LineKind::Added => {
                change.added += 1;
                after += 1;
            }
            LineKind::Removed => {
                change.removed += 1;
                before += 1;
            }
            _ => {
                before += 1;
                after += 1;
            }
        }
        if *kind != LineKind::Same && previous == LineKind::Same {
            change.changes += 1;
        }
        previous = *kind;
        if shown[row] {
            if !change.lines.is_empty() && !shown[row - 1] {
                change.lines.push(Line { kind: LineKind::Gap, number: 0, text: String::new() });
            }
            change.lines.push(Line { kind: *kind, number, text: text.to_owned() });
        }
    }
    change
}
