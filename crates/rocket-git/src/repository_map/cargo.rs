//! Deliberately conservative manifest reader. Recognizes literal local path
//! dependencies without executing Cargo or treating comments/strings as code.
use super::*;

pub(super) fn connect(
    map: &mut RepositoryMap,
    index: &BTreeMap<Vec<u8>, usize>,
    manifests: &[(Vec<u8>, ObjectId, usize)],
    blobs: &[&[u8]],
    cancel: &CancellationToken,
) -> Result<()> {
    let mut packages = BTreeMap::new();
    let mut pending = Vec::new();
    for ((path, _, _), blob) in manifests.iter().zip(blobs) {
        cancel.check()?;
        let Ok(text) = std::str::from_utf8(blob) else {
            map.notes
                .push("A non-UTF-8 Cargo manifest was skipped.".into());
            continue;
        };
        let (name, deps) = manifest(text);
        let folder = path
            .rsplitn(2, |b| *b == b'/')
            .nth(1)
            .unwrap_or_default()
            .to_vec();
        let Some(name) = name else { continue };
        let Some(&at) = index.get(if folder.is_empty() { path } else { &folder }) else {
            continue;
        };
        map.nodes[at].kind = MapNodeKind::Crate;
        map.nodes[at].label = name.into_bytes();
        map.nodes[at].detail = "Cargo package · committed manifest".as_bytes().to_vec();
        packages.insert(folder.clone(), at);
        pending.push((folder, at, deps));
    }
    for (folder, from, paths) in pending {
        for path in paths {
            cancel.check()?;
            let Some(resolved) = relative(&folder, path.as_bytes()) else {
                continue;
            };
            if let Some(&to) = packages.get(&resolved)
                && from != to
                && !map
                    .links
                    .iter()
                    .any(|e| e.from == from && e.to == to && e.kind == MapLinkKind::DependsOn)
            {
                map.links.push(link(from, to, MapLinkKind::DependsOn));
            }
        }
    }
    Ok(())
}
fn literal(value: &str) -> Option<String> {
    let value = value.trim();
    let quote = value.as_bytes().first().copied()?;
    if !b"'\"".contains(&quote) || value.len() < 2 || value.as_bytes().last() != Some(&quote) {
        return None;
    }
    let body = &value[1..value.len() - 1];
    if body.contains(['\n', '\r'])
        || (quote == b'"' && body.contains('\\'))
        || body.as_bytes().contains(&quote)
    {
        return None;
    }
    Some(body.into())
}
fn outside(text: &str, delimiter: u8) -> Vec<&str> {
    let mut quote = 0;
    let mut escape = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in text.bytes().enumerate() {
        if quote != 0 {
            if escape {
                escape = false;
            } else if c == b'\\' && quote == b'"' {
                escape = true;
            } else if c == quote {
                quote = 0;
            }
        } else if b"'\"".contains(&c) {
            quote = c;
        } else if c == delimiter {
            parts.push(&text[start..i]);
            start = i + 1;
        }
    }
    parts.push(&text[start..]);
    parts
}
fn assignment(line: &str) -> Option<(&str, &str)> {
    let (a, b) = line.split_once('=')?;
    Some((a.trim(), b.trim()))
}
fn dependency_table(table: &str) -> bool {
    table == "dependencies"
        || table == "dev-dependencies"
        || table == "build-dependencies"
        || table.starts_with("target.")
            && [".dependencies", ".dev-dependencies", ".build-dependencies"]
                .iter()
                .any(|s| table.ends_with(s))
}
fn dependency_entry(table: &str) -> bool {
    ["dependencies.", "dev-dependencies.", "build-dependencies."]
        .iter()
        .any(|p| table.starts_with(p))
}
fn manifest(text: &str) -> (Option<String>, Vec<String>) {
    let mut section = String::new();
    let mut name = None;
    let mut paths = Vec::new();
    // Unrecognized/multiline declarations stay unlinked instead of guessed.
    let mut multiline = None;
    for raw in text.lines() {
        let line = outside(raw, b'#')[0].trim();
        if let Some(delimiter) = multiline {
            if line.contains(delimiter) {
                multiline = None;
            }
            continue;
        }
        if line.contains("\"\"\"") {
            if line.matches("\"\"\"").count() % 2 == 1 {
                multiline = Some("\"\"\"");
            }
            continue;
        }
        if line.contains("'''") {
            if line.matches("'''").count() % 2 == 1 {
                multiline = Some("'''");
            }
            continue;
        }
        if line.starts_with('[') {
            section = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or_default()
                .trim()
                .into();
            continue;
        }
        let Some((key, value)) = assignment(line) else {
            continue;
        };
        if section == "package" && key == "name" {
            name = literal(value);
        }
        if dependency_entry(&section)
            && key == "path"
            && let Some(path) = literal(value)
        {
            paths.push(path);
        }
        if dependency_table(&section)
            && let Some(inner) = value.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
        {
            for field in outside(inner, b',') {
                if let Some(("path", value)) = assignment(field)
                    && let Some(path) = literal(value)
                {
                    paths.push(path);
                }
            }
        }
    }
    (name, paths)
}
fn relative(base: &[u8], path: &[u8]) -> Option<Vec<u8>> {
    if path.is_empty()
        || path.starts_with(b"/")
        || path.contains(&b'\\')
        || path.contains(&b':')
        || path.contains(&0)
    {
        return None;
    }
    let mut parts = base
        .split(|b| *b == b'/')
        .filter(|p| !p.is_empty())
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    for part in path.split(|b| *b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => {
                parts.pop()?;
            }
            p => parts.push(p.to_vec()),
        }
    }
    Some(parts.join(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_literals_ignore_comments_strings_and_unsupported_syntax() {
        let (name, paths) = manifest(
            "[package]\nname = '検索'\n[dependencies]\na={path='../core',version='1'} # path='fake'\nb='path=\"../wrong\"'\n# c={path='../wrong'}\n[dependencies.helper]\npath='../helper'\n[package.metadata]\ntext='''\n[dependencies]\nx={path='../wrong'}\n'''\n",
        );
        assert_eq!(name.as_deref(), Some("検索"));
        assert_eq!(paths, ["../core", "../helper"]);
        assert_eq!(
            relative(b"crates/ui", b"../core"),
            Some(b"crates/core".to_vec())
        );
        assert_eq!(relative(b"crates/ui", b"../../../outside"), None);
    }
}
