//! Literal helper loads from committed shell text. This is a statement reader, not
//! a shell evaluator. Paths are checked against Git's tree by the caller.
use super::{CancellationToken, LABEL_BYTES, ModuleLanguage, Result};

pub(super) struct Load {
    pub path: String,
    pub beside_script: bool,
    pub command: &'static str,
}

fn word(input: &str) -> (&str, &str) {
    let input = input.trim_start();
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    (&input[..end], input[end..].trim_start())
}

fn unquote(s: &str) -> (&str, bool) {
    for quote in ['\'', '"'] {
        if let Some(value) = s.strip_prefix(quote).and_then(|s| s.strip_suffix(quote)) {
            return (value, quote == '"');
        }
    }
    (s, true)
}

fn fixed(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= LABEL_BYTES
        && !s.starts_with(['/', '\\', '~', '-'])
        && !s.chars().any(|c| c.is_control() || "$`'\"*?[]{}():;|&<>\\".contains(c))
}

/// Split the first argument without interpreting its contents. A narrow allowlist
/// below decides whether a quoted path or a parenthesized expression is knowable.
fn operand(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let (mut quote, mut depth) = (0, 0usize);
    for (i, &b) in bytes.iter().enumerate() {
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
        } else {
            match b {
                b'\'' | b'"' => quote = b,
                b'(' => depth += 1,
                b')' => depth = depth.checked_sub(1)?,
                _ if b.is_ascii_whitespace() && depth == 0 => return Some((&s[..i], s[i..].trim_start())),
                _ => {}
            }
        }
    }
    (quote == 0 && depth == 0 && !s.is_empty()).then_some((s, ""))
}

fn strip_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len()).filter(|s| s.eq_ignore_ascii_case(prefix))?;
    Some(&text[prefix.len()..])
}

fn powershell(statement: &str) -> Option<Load> {
    let (verb, rest) = word(statement);
    let (command, args) = if verb == "." || verb == "&" {
        (if verb == "." { "." } else { "&" }, rest)
    } else if verb.eq_ignore_ascii_case("Import-Module") {
        let mut args = rest;
        loop {
            let (flag, rest) = word(args);
            if ["-Name", "-Force", "-Global", "-PassThru", "-DisableNameChecking"]
                .iter()
                .any(|f| flag.eq_ignore_ascii_case(f))
            {
                args = rest;
            } else {
                break;
            }
        }
        ("Import-Module", args)
    } else if verb.eq_ignore_ascii_case("using") {
        let (module, rest) = word(rest);
        if !module.eq_ignore_ascii_case("module") {
            return None;
        }
        ("using module", rest)
    } else {
        return None;
    };
    let (arg, _) = operand(args)?;
    if arg.len() > LABEL_BYTES + 128 {
        return None;
    }
    let (value, expands) = unquote(arg);
    let value = value.replace('\\', "/");
    let root = |s: &str| s.eq_ignore_ascii_case("$PSScriptRoot") || s.eq_ignore_ascii_case("${PSScriptRoot}");
    let (path, beside_script) = if let Some(expr) =
        value.strip_prefix('(').and_then(|s| s.strip_suffix(')')).filter(|_| arg.starts_with('('))
    {
        let (join, args) = word(expr);
        if !join.eq_ignore_ascii_case("Join-Path") {
            return None;
        }
        let (mut base, mut child) = word(args);
        if base.eq_ignore_ascii_case("-Path") {
            (base, child) = word(child);
        }
        if !root(base) {
            return None;
        }
        let (flag, rest) = word(child);
        if flag.eq_ignore_ascii_case("-ChildPath") {
            child = rest;
        }
        let (child, rest) = operand(child)?;
        if !rest.is_empty() {
            return None;
        }
        (unquote(child).0, true)
    } else if let Some(child) =
        expands.then(|| strip_ci(&value, "$PSScriptRoot/").or_else(|| strip_ci(&value, "${PSScriptRoot}/"))).flatten()
    {
        (child, true)
    } else {
        // A bare module name is searched for on PSModulePath.
        if !value.contains('/') {
            return None;
        }
        (value.as_str(), command == "using module")
    };
    fixed(path).then(|| Load { path: path.into(), beside_script, command })
}

/// Known spellings of the directory of the entry script or the sourced Bash file.
/// Keep the literal suffix (often /..), never run dirname, cd or pwd.
fn bash_directory(value: &str) -> Option<&str> {
    let remaining = if let Some(rest) = value.strip_prefix("${BASH_SOURCE[0]%/*}") {
        rest
    } else {
        let rest = value.strip_prefix("$(dirname ")?;
        let rest = rest.strip_prefix("-- ").unwrap_or(rest);
        [r#""$0")"#, r#""${BASH_SOURCE[0]}")"#, "$0)", "${BASH_SOURCE[0]})"]
            .iter()
            .find_map(|p| rest.strip_prefix(p))?
    };
    if remaining.is_empty() { Some("") } else { remaining.strip_prefix('/').filter(|s| fixed(s)) }
}

fn directory_binding(statement: &str) -> Option<(&str, &str)> {
    let (name, value) = statement.split_once('=')?;
    if name.is_empty()
        || name.len() > 128
        || !name.bytes().all(|c| c == b'_' || c.is_ascii_alphanumeric())
        || name.as_bytes()[0].is_ascii_digit()
    {
        return None;
    }
    let (value, expands) = unquote(value);
    if !expands {
        return None;
    }
    let dir = if let Some(cd) = value.strip_prefix("$(cd ") {
        let cd = cd.strip_suffix(" && pwd -P)").or_else(|| cd.strip_suffix(" && pwd)"))?;
        let (dir, expands) = unquote(cd.strip_prefix("-- ").unwrap_or(cd));
        if !expands {
            return None;
        }
        dir
    } else {
        value
    };
    Some((name, bash_directory(dir)?))
}

fn bash(statement: &str, binding: Option<(&str, &str)>) -> Option<Load> {
    let (verb, rest) = word(statement);
    let command = match verb {
        "." => ".",
        "source" => "source",
        _ => return None,
    };
    let (arg, _) = operand(rest)?;
    if arg.len() > LABEL_BYTES + 128 {
        return None;
    }
    let (value, expands) = unquote(arg);
    let (path, beside_script) = if let Some(path) = expands.then(|| bash_directory(value)).flatten() {
        (path.to_owned(), true)
    } else if expands && value.starts_with('$') {
        let (name, base) = binding?;
        let (var, child) = value[1..].split_once('/')?;
        let var = var.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(var);
        if var != name || !fixed(child) {
            return None;
        }
        (if base.is_empty() { child.into() } else { format!("{base}/{child}") }, true)
    } else {
        // With no slash, source and dot search PATH.
        if !value.contains('/') {
            return None;
        }
        (value.to_owned(), false)
    };
    fixed(&path).then_some(Load { path, beside_script, command })
}

/// Skip strings (including multiline ones) before looking for statement separators.
/// Returning the end of input for an unfinished construct avoids reading its body as code.
fn quoted(bytes: &[u8], mut at: usize, ps: bool) -> usize {
    let quote = bytes[at];
    at += 1;
    while at < bytes.len() {
        let b = bytes[at];
        at += 1;
        if b == quote {
            if ps && bytes.get(at) == Some(&quote) {
                at += 1;
            } else {
                break;
            }
        } else if quote != b'\'' && b == if ps { b'`' } else { b'\\' } {
            at = (at + 1).min(bytes.len());
        }
    }
    at
}

fn newline(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| at + i)
}

/// Skip heredoc bodies at the next newline in left-to-right order. Quoted delimiters
/// have their quotes removed by the shell, and <<- permits leading tabs on the end line.
fn here_doc(s: &str) -> Option<(String, bool, usize)> {
    let strip_tabs = s.starts_with('-');
    let rest = s[usize::from(strip_tabs)..].trim_start_matches([' ', '\t']);
    let mut end = 0;
    let mut quote = 0;
    let mut delimiter = Vec::new();
    let bytes = rest.as_bytes();
    while let Some(&b) = bytes.get(end) {
        if quote == 0 && (b.is_ascii_whitespace() || b";|&()<>".contains(&b)) {
            break;
        }
        end += 1;
        match b {
            b'\'' | b'"' if quote == 0 => quote = b,
            _ if b == quote => quote = 0,
            b'\\' if quote != b'\'' => {
                delimiter.push(*bytes.get(end)?);
                end += 1;
            }
            _ => delimiter.push(b),
        }
        if delimiter.len() > LABEL_BYTES {
            return None;
        }
    }
    if quote != 0 || end == 0 {
        return None;
    }
    Some((String::from_utf8(delimiter).ok()?, strip_tabs, s.len() - rest.len() + end))
}

pub(super) fn read(
    language: ModuleLanguage,
    source: &str,
    cancel: &CancellationToken,
    mut on_load: impl FnMut(Load),
) -> Result<()> {
    cancel.check()?;
    if source.as_bytes().contains(&0) {
        return Ok(());
    }
    let ps = language == ModuleLanguage::PowerShell;
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let bytes = source.as_bytes();
    let mut binding = None;
    let mut cwd_changed = false;
    let mut statement = |s: &'_ str| {
        let mut s = s.trim();
        if !ps {
            while let ("then" | "else" | "do" | "if" | "!" | "{", rest) = word(s) {
                s = rest;
            }
        }
        if s.is_empty() {
            return;
        }
        let (verb, _) = word(s);
        if (ps
            && ["cd", "chdir", "sl", "Set-Location", "pushd", "popd", "Push-Location", "Pop-Location"]
                .iter()
                .any(|v| verb.eq_ignore_ascii_case(v)))
            || (!ps && matches!(verb, "cd" | "pushd" | "popd"))
        {
            cwd_changed = true;
        }
        if ps {
            if let Some(load) = powershell(s).filter(|l| l.beside_script || !cwd_changed) {
                on_load(load);
            }
        } else if let Some(load) = bash(s, binding.as_ref().map(|(n, d): &(String, String)| (n.as_str(), d.as_str()))) {
            if load.beside_script || !cwd_changed {
                on_load(load);
            }
        } else {
            // Do not propagate a variable through arbitrary commands, branches or reassignment.
            binding = directory_binding(s).map(|(n, d)| (n.to_owned(), d.to_owned()));
        }
    };
    let (mut start, mut at, mut depth, mut next_check) = (0, 0, 0usize, 0);
    let mut documents = Vec::new();
    while at < bytes.len() {
        if at >= next_check {
            cancel.check()?;
            next_check = at + 4096;
        }
        let b = bytes[at];
        let rest = &bytes[at..];
        if ps && rest.starts_with(b"<#") {
            let mut nesting = 1usize;
            at += 2;
            while nesting > 0 && at < bytes.len() {
                if bytes[at..].starts_with(b"#>") {
                    nesting -= 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"<#") {
                    nesting += 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            continue;
        }
        if ps
            && (rest.starts_with(b"@'") || rest.starts_with(b"@\""))
            && rest[2..].iter().find(|&&c| !matches!(c, b' ' | b'\t' | b'\r')) == Some(&b'\n')
        {
            let quote = bytes[at + 1];
            at = newline(bytes, at + 2);
            while at < bytes.len() {
                at += 1;
                if bytes[at..].starts_with(&[quote, b'@']) {
                    at += 2;
                    break;
                }
                at = newline(bytes, at);
            }
            continue;
        }
        if matches!(b, b'\'' | b'"') || (!ps && b == b'`') {
            at = quoted(bytes, at, ps);
            continue;
        }
        if b == if ps { b'`' } else { b'\\' } {
            at = (at + 2).min(bytes.len());
            continue;
        }
        if b == b'#'
            && (at == start || at == 0 || bytes[at - 1].is_ascii_whitespace() || b";|&(".contains(&bytes[at - 1]))
        {
            let end = newline(bytes, at);
            if depth == 0 {
                statement(&source[start..at]);
                start = end;
            }
            at = end;
            continue;
        }
        if !ps && rest.starts_with(b"<<<") {
            at += 3;
            continue;
        }
        if !ps && depth == 0 && rest.starts_with(b"<<") {
            let Some((delimiter, tabs, consumed)) = here_doc(&source[at + 2..]) else { break };
            if documents.len() >= 16 {
                break;
            }
            documents.push((delimiter, tabs));
            at += 2 + consumed;
            continue;
        }
        if b == b'(' {
            depth += 1;
        }
        if b == b')' {
            depth = depth.saturating_sub(1);
        }
        let separator = match b {
            b'\n' | b';' | b'|' if depth == 0 => {
                if rest.starts_with(b"||") {
                    2
                } else {
                    1
                }
            }
            b'&' if depth == 0 && rest.starts_with(b"&&") => 2,
            b'{' if ps && depth == 0 && at > 0 && (bytes[at - 1].is_ascii_whitespace() || bytes[at - 1] == b')') => 1,
            _ => 0,
        };
        if separator > 0 {
            statement(&source[start..at]);
            at += separator;
            if b == b'\n' {
                for (delimiter, tabs) in documents.drain(..) {
                    while at < bytes.len() {
                        let end = newline(bytes, at);
                        let line = source[at..end].trim_end_matches('\r');
                        at = (end + 1).min(bytes.len());
                        if (if tabs { line.trim_start_matches('\t') } else { line }) == delimiter {
                            break;
                        }
                    }
                }
            }
            start = at;
        } else {
            at += 1;
        }
    }
    if at == bytes.len() && depth == 0 {
        statement(&source[start..]);
    }
    cancel.check()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loads(language: ModuleLanguage, source: &str) -> Vec<(&'static str, String, bool)> {
        let mut found = Vec::new();
        read(language, source, &CancellationToken::new(), |l| found.push((l.command, l.path, l.beside_script)))
            .unwrap();
        found
    }

    #[test]
    fn powershell_paths_and_module_imports_are_static() {
        let source = r##"
# . './fake.ps1'
$example = @'
Import-Module './fake.psm1'
'@
<# outer <# nested #>
. './fake.ps1'
#>
Write-Output "a multiline string
. './fake.ps1'
"
& (join-PATH -Path $psscriptroot -ChildPath 'tools\補 助.ps1') -Quiet
IMPORT-module -Force -Name "${PSScriptRoot}\module.psm1"
using module ./lib/entry.psd1
. '.\child.ps1'; . "$PSScriptRoot/helper.ps1"
. '$PSScriptRoot/fake.ps1'
Import-Module PSReadLine
. "$env:HOME/fake.ps1"
& "$PSScriptRoot/helper.ps1".Replace('a','b')
. "$(Get-Location)/fake.ps1"
. 'C:\outside.ps1'
"##;
        assert_eq!(
            loads(ModuleLanguage::PowerShell, source),
            [
                ("&", "tools/補 助.ps1".into(), true),
                ("Import-Module", "module.psm1".into(), true),
                ("using module", "./lib/entry.psd1".into(), true),
                (".", "./child.ps1".into(), false),
                (".", "helper.ps1".into(), true),
            ]
        );
    }

    #[test]
    fn bash_source_resolves_known_anchors_and_skips_literal_bodies() {
        let source = r##"
cat <<'FIRST' <<-SECOND
source ./fake.sh
FIRST
. ./fake.sh
SECOND
mask=$((1 << 2))
echo "a multiline
source ./fake.sh"
echo 'sample; source ./fake.sh'
echo "$(printf '%s' 'source ./fake.sh')"
name=$(cd "$(dirname "$0")/.." && pwd -P)
source "$name/tools/補 助.sh"
. "${BASH_SOURCE[0]%/*}/one.bash"
true && source "$(dirname -- "${BASH_SOURCE[0]}")/two.sh"
if test -f ./folder/three.sh; then . './folder/three.sh'; fi # source ./fake.sh
source /etc/profile
source global.sh
source "$HOME/fake.sh"
name=$(choose-directory)
. "$name/fake.sh"
source ./lib/*.sh
"##;
        assert_eq!(
            loads(ModuleLanguage::Bash, source),
            [
                ("source", "../tools/補 助.sh".into(), true),
                (".", "one.bash".into(), true),
                ("source", "two.sh".into(), true),
                (".", "./folder/three.sh".into(), false),
            ]
        );
    }

    #[test]
    fn location_changes_and_reassigned_directory_variables_are_not_guessed() {
        let ps = "Set-Location $other\n. './local.ps1'\n. \"$PSScriptRoot/real.ps1\"\n";
        assert_eq!(loads(ModuleLanguage::PowerShell, ps), [(".", "real.ps1".into(), true)]);
        let sh = "here=$(dirname \"$0\")\nhere=/elsewhere\nsource \"$here/ghost.sh\"\ncd unknown\n. ./ghost.sh\n. \"${BASH_SOURCE[0]%/*}/real.sh\"\n";
        assert_eq!(loads(ModuleLanguage::Bash, sh), [(".", "real.sh".into(), true)]);
    }

    #[test]
    fn cancellation_during_a_file_interrupts_the_read() {
        let cancel = CancellationToken::new();
        let text = "source ./helper.sh\n".repeat(2000);
        let mut visits = 0;
        let result = read(ModuleLanguage::Bash, &text, &cancel, |_| {
            visits += 1;
            cancel.cancel();
        });
        assert!(matches!(result, Err(crate::Error::Cancelled)));
        assert!((1..2000).contains(&visits));
    }

    #[test]
    fn catalog_and_decoding_follow_the_script_file_type() {
        use super::super::{FileKind, file_kind, packages, source_text};
        let paths: &[&[u8]] = &[
            b"build.PS1",
            b".github/ci/Build.PSM1",
            b".github/module.psd1",
            b".bashrc",
            b"bin/test.bash",
            b"bin/test.sh",
            b"node_modules/pkg/test.sh",
            b"manual.md",
        ];
        let (groups, limited) = packages::find(paths);
        assert!(!limited);
        assert_eq!(
            groups.iter().map(|g| (g.language, g.folder.as_slice(), g.files.len())).collect::<Vec<_>>(),
            [
                (ModuleLanguage::PowerShell, b"".as_slice(), 1),
                (ModuleLanguage::PowerShell, b".github".as_slice(), 2),
                (ModuleLanguage::Bash, b"".as_slice(), 1),
                (ModuleLanguage::Bash, b"bin".as_slice(), 2),
            ]
        );
        assert_eq!(file_kind(b".bashrc"), FileKind::Code);
        assert_eq!(source_text(b"build.ps1", &[0xfe, 0xff, 0, b'#']).as_deref(), Some("#"));
        assert!(source_text(b"build.ps1", &[0xff, 0xfe, 0x01]).is_none());
        assert!(source_text(b"build.ps1", &[0xfe, 0xff, 0xd8, 0]).is_none());
        assert!(source_text(b"build.sh", &[0xff, 0xfe, b'#', 0]).is_none());
    }
}
