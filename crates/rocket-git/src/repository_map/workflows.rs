//! Static links from the block-form jobs and steps of a committed Actions workflow.
//! Keep only the small part of YAML that determines an operand and its working directory.
use super::{CancellationToken, Result, cargo, packages, scripts};
use std::collections::HashSet;

pub(super) const FOLDER: &[u8] = b".github/workflows";

pub(super) fn is_workflow(path: &[u8]) -> bool {
    packages::parent(path) == FOLDER && (path.ends_with(b".yml") || path.ends_with(b".yaml"))
}

#[derive(Default)]
struct Settings {
    directory: Option<String>,
    shell: Option<String>,
}
#[derive(Default)]
struct Step {
    settings: Settings,
    run: Option<String>,
    uses: Option<String>,
    checkout_override: bool,
    checkout_path: Option<String>,
    conditional: bool,
}
#[derive(Default)]
struct Job {
    defaults: Settings,
    runner: Option<String>,
    uses: Option<String>,
    steps: Vec<Step>,
}
#[derive(Default)]
struct Workflow {
    defaults: Settings,
    jobs: Vec<Job>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Root,
    Jobs,
    Job(usize),
    Steps(usize),
    Step(usize, usize),
    Defaults(Option<usize>),
    RunDefaults(Option<usize>),
    Inputs(usize, usize),
    Other,
}

/// A mapping colon must end the key. Colons in URLs or in shell operands are data.
fn field(s: &str) -> Option<(&str, &str)> {
    let at = s
        .match_indices(':')
        .find(|(at, _)| s.as_bytes().get(at + 1).is_none_or(u8::is_ascii_whitespace))?
        .0;
    let key = s[..at].trim();
    let key = key
        .strip_prefix('"')
        .and_then(|k| k.strip_suffix('"'))
        .or_else(|| key.strip_prefix('\'').and_then(|k| k.strip_suffix('\'')))
        .unwrap_or(key);
    (!key.is_empty()).then_some((key, s[at + 1..].trim_start()))
}

fn scalar(s: &str) -> Option<String> {
    if s.starts_with(['[', '{', '*', '&', '!']) || s.is_empty() {
        return None;
    }
    if let Some(tail) = s.strip_prefix(['\'', '"']) {
        let quote = s.as_bytes()[0] as char;
        let mut out = String::new();
        let mut chars = tail.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            if c == quote {
                if quote == '\'' && chars.next_if(|(_, c)| *c == '\'').is_some() {
                    out.push(c);
                    continue;
                }
                let rest = &tail[at + 1..];
                return (rest.trim().is_empty()
                    || rest.starts_with(char::is_whitespace) && rest.trim_start().starts_with('#'))
                .then_some(out);
            }
            if c == '\\' && quote == '"' {
                out.push(match chars.next()?.1 {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    _ => return None,
                });
            } else {
                out.push(c);
            }
        }
        return None;
    }
    let comment = s
        .match_indices('#')
        .find(|(i, _)| *i == 0 || s.as_bytes()[i - 1].is_ascii_whitespace());
    Some(s[..comment.map_or(s.len(), |(i, _)| i)].trim_end().to_owned())
}

fn indentation(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b' ').count()
}

/// The end of a scalar precedes the next sibling key, including one following "- run:".
fn scalar_end(lines: &[&str], start: usize, column: usize) -> usize {
    let mut end = start;
    while end < lines.len() && (lines[end].trim().is_empty() || indentation(lines[end]) > column) {
        end += 1;
    }
    end
}

fn block(lines: &[&str], marker: &str, column: usize) -> Option<String> {
    let marker = marker.split_whitespace().next()?;
    let folded = marker.starts_with('>');
    let mut indent = None;
    for c in marker[1..].chars() {
        match c {
            '1'..='9' if indent.is_none() => indent = Some(column + c.to_digit(10)? as usize),
            '-' | '+' => {}
            _ => return None,
        }
    }
    let indent = indent
        .or_else(|| lines.iter().find(|s| !s.trim().is_empty()).map(|s| indentation(s)))
        .unwrap_or(0);
    let mut out = String::new();
    let mut prev = "";
    for (n, raw) in lines.iter().enumerate() {
        let line = if raw.trim().is_empty() {
            ""
        } else {
            raw.get(indent..).filter(|_| indentation(raw) >= indent)?
        };
        if n != 0 {
            if !folded
                || line.is_empty()
                || prev.starts_with(char::is_whitespace)
                || line.starts_with(char::is_whitespace)
            {
                out.push('\n');
            } else if !prev.is_empty() {
                out.push(' ');
            }
        }
        out.push_str(line);
        prev = line;
    }
    Some(out)
}

fn parse(source: &str, cancel: &CancellationToken) -> Result<Option<Workflow>> {
    const MAX_LINES: usize = 32_768;
    let mut workflow = Workflow::default();
    let lines: Vec<_> = source
        .trim_start_matches('\u{feff}')
        .lines()
        .take(MAX_LINES + 1)
        .collect();
    if lines.len() > MAX_LINES || source.as_bytes().contains(&0) {
        return Ok(None);
    }
    let mut stack = Vec::<(usize, Scope)>::new();
    let mut at = 0;
    while at < lines.len() {
        cancel.check()?;
        let raw = lines[at];
        at += 1;
        let mut column = indentation(raw);
        let mut line = raw[column..].trim_end();
        if line.is_empty() || line.starts_with('#') || line == "---" && stack.is_empty() {
            continue;
        }
        if line.starts_with('\t') || matches!(line, "---" | "...") {
            return Ok(None);
        }
        let item = line == "-"
            || line
                .strip_prefix('-')
                .is_some_and(|s| s.starts_with(char::is_whitespace));
        while stack
            .last()
            .is_some_and(|(n, scope)| *n >= column && !(item && *n == column && matches!(scope, Scope::Steps(_))))
        {
            stack.pop();
        }
        let mut parent = stack.last().map_or(Scope::Root, |&(_, scope)| scope);
        if item {
            parent = if let Scope::Steps(job) = parent {
                let steps = &mut workflow.jobs[job].steps;
                steps.push(Step::default());
                Scope::Step(job, steps.len() - 1)
            } else {
                Scope::Other
            };
            stack.push((column, parent));
            let rest = line[1..].trim_start();
            column += line.len() - rest.len();
            line = rest;
        }
        if stack.len() > 64 {
            return Ok(None);
        }
        let Some((key, raw_value)) = field(line) else { continue };
        let container = raw_value.is_empty() || raw_value.starts_with('#');
        let value = if container {
            None
        } else {
            let end = scalar_end(&lines, at, column);
            let body = &lines[at..end];
            at = end;
            if raw_value.starts_with(['|', '>']) {
                block(body, raw_value, column)
            } else {
                // Multiline plain/quoted values may fold more shell text into the operand.
                // Skip unfamiliar forms instead of interpreting just their first line.
                let continued = body
                    .iter()
                    .any(|s| !s.trim().is_empty() && !s.trim_start().starts_with('#'));
                if continued { None } else { scalar(raw_value) }
            }
        };
        let child = match (parent, key) {
            (Scope::Root, "jobs") => Scope::Jobs,
            (Scope::Jobs, "<<") => return Ok(None),
            (Scope::Jobs, _) => {
                workflow.jobs.push(Job::default());
                Scope::Job(workflow.jobs.len() - 1)
            }
            (Scope::Job(j), "steps") => Scope::Steps(j),
            (Scope::Root, "defaults") => Scope::Defaults(None),
            (Scope::Job(j), "defaults") => Scope::Defaults(Some(j)),
            (Scope::Defaults(j), "run") => Scope::RunDefaults(j),
            (Scope::RunDefaults(j), "working-directory" | "shell") => {
                let settings = match j {
                    Some(i) => &mut workflow.jobs[i].defaults,
                    None => &mut workflow.defaults,
                };
                if key == "shell" {
                    settings.shell = Some(value.unwrap_or_default());
                } else {
                    settings.directory = Some(value.unwrap_or_default());
                }
                Scope::Other
            }
            (Scope::Job(j), "runs-on") => {
                workflow.jobs[j].runner = value;
                Scope::Other
            }
            (Scope::Job(j), "uses") => {
                workflow.jobs[j].uses = value;
                Scope::Other
            }
            (Scope::Step(j, s), "with") if container => Scope::Inputs(j, s),
            (Scope::Inputs(j, s), key) => {
                let step = &mut workflow.jobs[j].steps[s];
                match key {
                    "path" => step.checkout_path = Some(value.unwrap_or_default()),
                    "repository" | "ref" | "<<" => step.checkout_override = true,
                    _ => {}
                }
                Scope::Other
            }
            (Scope::Step(j, s), key) => {
                let step = &mut workflow.jobs[j].steps[s];
                match key {
                    "uses" => step.uses = value,
                    "run" => step.run = value,
                    "shell" => step.settings.shell = Some(value.unwrap_or_default()),
                    "working-directory" => step.settings.directory = Some(value.unwrap_or_default()),
                    "if" => step.conditional = value.as_deref() != Some("true"),
                    "with" => step.checkout_override = true,
                    "<<" => return Ok(None),
                    _ => {}
                }
                Scope::Other
            }
            (scope, "<<") if scope != Scope::Other => return Ok(None),
            _ => Scope::Other,
        };
        if !container && child != Scope::Other {
            // Flow collections and inherited mappings can change a step's owner or cwd.
            // Empty collections contain no steps or defaults.
            if !matches!(raw_value, "{}" | "[]") {
                return Ok(None);
            }
        }
        stack.push((column, if container { child } else { Scope::Other }));
    }
    Ok(Some(workflow))
}

fn literal_path(base: &[u8], value: &str) -> Option<Vec<u8>> {
    if value.starts_with('~') || value.chars().any(|c| c.is_control() || "$%*?{}[]".contains(c)) {
        return None;
    }
    cargo::relative(base, value.as_bytes())
}

fn local_reference(uses: &str) -> Option<Vec<u8>> {
    let path = if let Some(path) = uses.strip_prefix("$/") {
        if path.contains('@') {
            return None;
        }
        path
    } else {
        uses.strip_prefix("./")?
    };
    literal_path(b"", if path.is_empty() { "." } else { path })
}

fn local_action(folder: &[u8], known: &HashSet<&[u8]>) -> Option<Vec<u8>> {
    let mut manifests = [b"action.yml".as_slice(), b"action.yaml"]
        .into_iter()
        .map(|name| packages::join(folder, name))
        .filter(|p| known.contains(p.as_slice()));
    let first = manifests.next()?;
    manifests.next().is_none().then_some(first)
}

/// A checkout of another revision or into a subfolder changes the meaning of workspace paths.
fn shadowed(mut path: &[u8], checkouts: &HashSet<Vec<u8>>) -> bool {
    while !checkouts.contains(path) {
        if path.is_empty() {
            return false;
        }
        path = packages::parent(path);
    }
    true
}

/// False means the document exceeded a bound or contained unsupported inherited settings.
pub(super) fn read(
    source: &str,
    known: &HashSet<&[u8]>,
    cancel: &CancellationToken,
    mut found: impl FnMut(&'static str, Vec<u8>),
) -> Result<bool> {
    cancel.check()?;
    let Some(workflow) = parse(source, cancel)? else {
        return Ok(false);
    };
    for job in &workflow.jobs {
        cancel.check()?;
        if let Some(path) = job
            .uses
            .as_deref()
            .and_then(local_reference)
            .filter(|p| is_workflow(p) && known.contains(p.as_slice()))
        {
            found("uses", path);
        }
        let mut checkouts = HashSet::new();
        for step in &job.steps {
            cancel.check()?;
            if let Some(uses) = &step.uses {
                if uses
                    .split_once('@')
                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case("actions/checkout"))
                {
                    let dir = step.checkout_path.as_deref().map_or_else(|| Some(Vec::new()), |p| literal_path(b"", p));
                    if !step.checkout_override && dir.as_ref().is_some_and(Vec::is_empty) {
                        if !step.conditional {
                            // A default checkout restores the event revision at the workspace root.
                            checkouts.remove(b"".as_slice());
                        }
                    } else {
                        checkouts.insert(dir.unwrap_or_default());
                    }
                } else if let Some(dir) = local_reference(uses) {
                    if (uses.starts_with("$/") || !shadowed(&dir, &checkouts))
                        && let Some(path) = local_action(&dir, known)
                    {
                        found("uses", path);
                    }
                }
                continue;
            }
            let Some(run) = &step.run else { continue };
            let shell = step
                .settings
                .shell
                .as_ref()
                .or(job.defaults.shell.as_ref())
                .or(workflow.defaults.shell.as_ref());
            let ps = match shell.map(String::as_str) {
                Some("bash" | "sh") => Some(false),
                Some("pwsh" | "powershell") => Some(true),
                None => match job.runner.as_deref() {
                    Some(s) if s.starts_with("windows-") => Some(true),
                    Some(s) if s.starts_with("ubuntu-") || s.starts_with("macos-") => Some(false),
                    _ => None,
                },
                _ => continue,
            };
            let dir = step
                .settings
                .directory
                .as_ref()
                .or(job.defaults.directory.as_ref())
                .or(workflow.defaults.directory.as_ref())
                .map_or(".", String::as_str);
            let dir = if ps == Some(true) {
                dir.replace('\\', "/")
            } else {
                dir.to_owned()
            };
            let Some(base) = literal_path(b"", &dir) else { continue };
            // A matrix or self-hosted runner can choose either default shell. Keep only
            // operands both readers identify, so quoting and cwd changes cannot invent an edge.
            let mut both = HashSet::new();
            if ps.is_none() {
                scripts::workflow_commands(false, run, cancel, |p| {
                    both.insert(p);
                })?;
            }
            scripts::workflow_commands(ps.unwrap_or(true), run, cancel, |operand| {
                if (ps.is_some() || both.contains(&operand))
                    && let Some(path) = literal_path(&base, &operand)
                        .filter(|p| known.contains(p.as_slice()) && !shadowed(p, &checkouts))
                {
                    found("run", path);
                }
            })?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(source: &str, files: &[&str]) -> (bool, Vec<String>) {
        let known = files.iter().map(|p| p.as_bytes()).collect();
        let mut links = Vec::new();
        let complete = read(source, &known, &CancellationToken::new(), |kind, path| {
            links.push(format!("{kind} {}", String::from_utf8(path).unwrap()))
        })
        .unwrap();
        (complete, links)
    }

    #[test]
    fn folded_commands_shell_positions_and_unknown_directories() {
        let source = r#"
jobs:
  ci:
    runs-on: windows-2025
    steps:
    - run: >-
        bash
        'scripts/二 words.sh'
    - run: >
        echo example
        bash ignored.sh
    - run: |
        & '.\scripts\file.ps1'
        Set-Location other
        bash ignored.sh
    - run: |
        cat <<'DOC'
        bash ignored.sh
        DOC
        source ./scripts/env
        node --test \
          './scripts/test.cjs' "$ARG"
        bash -c "bash ignored.sh"
        python -m ignored.sh
        cd "$RUNTIME" && bash ignored.sh
      shell: bash
    - run: bash ignored.sh
      shell: python
    - run: bash ignored.sh
      working-directory: ${{ matrix.dir }}
    - name: |
        jobs:
          - run: bash ignored.sh
      uses: owner/action@v1
      with:
        run: bash ignored.sh
"#;
        let (complete, paths) = links(
            source,
            &[
                "scripts/二 words.sh",
                "scripts/file.ps1",
                "scripts/env",
                "scripts/test.cjs",
                "ignored.sh",
            ],
        );
        assert!(complete);
        assert_eq!(
            paths,
            [
                "run scripts/二 words.sh",
                "run scripts/file.ps1",
                "run scripts/env",
                "run scripts/test.cjs"
            ]
        );
    }

    #[test]
    fn default_checkout_restores_script_and_local_action_links() {
        let source = r#"
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: previous
      - run: bash old.sh
      - uses: ./old
      - uses: actions/checkout@v4
        if: github.event_name == 'release'
      - run: bash old.sh
      - uses: actions/checkout@v4
      - run: bash scripts/build.sh
      - uses: ./local
"#;
        let (complete, paths) = links(source, &["old.sh", "old/action.yml", "scripts/build.sh", "local/action.yml"]);
        assert!(complete);
        assert_eq!(paths, ["run scripts/build.sh", "uses local/action.yml"]);
    }

    #[test]
    fn local_uses_have_distinct_step_and_job_targets() {
        let source = r#"
jobs:
  ci:
    steps:
      - uses: './' # root action
      - uses: ./.github/actions/both
      - uses: ./.github/workflows/ci.yaml
      - uses: owner/repo/.github/workflows/ci.yaml@main
      - uses: ./../external
      - uses: ./${{ inputs.action }}
  shared:
    uses: "./.github/workflows/ci.yaml"
  nested:
    uses: ./.github/workflows/sub/ci.yaml
  invalid:
    uses: ./.github/actions/both
"#;
        let (_, paths) = links(
            source,
            &[
                "action.yml",
                ".github/actions/both/action.yaml",
                ".github/actions/both/action.yml",
                ".github/workflows/ci.yaml",
                ".github/workflows/sub/ci.yaml",
            ],
        );
        assert_eq!(paths, ["uses action.yml", "uses .github/workflows/ci.yaml"]);
    }

    #[test]
    fn self_references_survive_a_different_checkout_and_matrix_shells_agree_on_literal_paths() {
        let source = r#"
jobs:
  ci:
    runs-on: ${{ matrix.os }}
    steps:
      - run: node scripts/test.cjs
      - uses: actions/checkout@v4
        with:
          repository: example/other
          path: vendor
      - run: bash vendor/unrelated.sh
      - run: bash clean.sh
      - uses: actions/checkout@v4
        with:
          ref: old-branch
      - run: bash stale.sh
      - uses: ./local/action
      - uses: $/local/action
      - uses: $/local/action@old
  reusable:
    uses: $/.github/workflows/shared.yml
"#;
        let (complete, paths) = links(
            source,
            &[
                "scripts/test.cjs",
                "vendor/unrelated.sh",
                "clean.sh",
                "stale.sh",
                "local/action/action.yml",
                "local/action@old/action.yml",
                ".github/workflows/shared.yml",
            ],
        );
        assert!(complete);
        assert_eq!(
            paths,
            [
                "run scripts/test.cjs",
                "run clean.sh",
                "uses local/action/action.yml",
                "uses .github/workflows/shared.yml"
            ]
        );
    }

    #[test]
    fn bounds_and_unknown_defaults_cannot_become_root_paths() {
        let flow = "defaults: {run: {working-directory: sub}}\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - run: bash root.sh";
        assert_eq!(links(flow, &["root.sh"]), (false, vec![]));
        assert_eq!(links(&"# x\n".repeat(32_769), &[]), (false, vec![]));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            read(flow, &HashSet::new(), &cancel, |_, _| unreachable!()),
            Err(crate::Error::Cancelled)
        ));
    }

    #[test]
    fn rockets_macos_build_workflow_calls_committed_helpers() {
        let source = include_str!("../../../../.github/workflows/macos-arm64.yml");
        let paths = [
            "tools/build-macos.sh",
            "tools/macos/architecture.sh",
            "tools/tests/test-macos-preflight.cjs",
        ];
        let (complete, found) = links(source, &paths);
        assert!(complete);
        for path in paths {
            assert!(found.contains(&format!("run {path}")), "{path} missing from {found:?}");
        }
    }
}
