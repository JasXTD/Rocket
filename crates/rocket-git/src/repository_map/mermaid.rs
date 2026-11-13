use super::*;
use std::fmt::Write;

/// Serialize only the reviewed, displayed graph. Labels are data, never syntax.
/// No renderer, URL, callbacks, HTML or Mermaid configuration is emitted.
pub fn map_mermaid(
    map: &RepositoryMap,
    visible: &[usize],
    links: &[MapLink],
    cancel: &CancellationToken,
) -> Result<String> {
    cancel.check()?;
    if visible.is_empty() || visible.len() > 240 || links.len() > MAP_LINKS {
        return Err(Error::InvalidInput("Export requires 1–240 visible nodes and at most 12,000 connections. Narrow the map first.".into()));
    }
    let mut selected = BTreeSet::new();
    for &i in visible {
        if i >= map.nodes.len() || !selected.insert(i) {
            return Err(malformed());
        }
    }
    let mut out = format!(
        "%% Rocket {:?} map · visible view, not a complete repository\n",
        map.kind
    );
    for id in &map.tips {
        writeln!(out, "%% Pinned tip: {id}").unwrap();
    }
    if let Some(id) = &map.excluded_base {
        writeln!(out, "%% Pinned excluded base: {id}").unwrap();
    }
    writeln!(
        out,
        "%% Scope revision: {} | excluded base: {} | literal path: {}",
        escaped(map.scope.revision.as_bytes()),
        escaped(map.scope.base.as_bytes()),
        escaped(map.scope.path.as_ref().map_or(b"", RepoPath::as_bytes))
    )
    .unwrap();
    writeln!(
        out,
        "%% Loaded commits/records: {} | partial: {} | parents outside view: {}",
        map.scanned, map.limited, map.boundary_parents
    )
    .unwrap();
    out.push_str("flowchart LR\n");
    for &i in visible {
        cancel.check()?;
        let n = &map.nodes[i];
        // A module by its path in the crate, so `a::util` and `b::util` stay apart.
        let mut title = match n.key.strip_prefix(b"module:") {
            Some(path) if n.kind == MapNodeKind::Module && !path.is_empty() => path.to_vec(),
            _ => n.label.clone(),
        };
        if let Some(id) = &n.commit {
            title.extend_from_slice(format!(" · {}", &id.as_str()[..12]).as_bytes());
        }
        writeln!(out, "  n{i}[\"{}\"]", escaped(&title)).unwrap();
    }
    for e in links {
        cancel.check()?;
        if !selected.contains(&e.from) || !selected.contains(&e.to) {
            continue;
        }
        let label = match e.kind {
            MapLinkKind::Contains => "contains",
            MapLinkKind::DependsOn if map.kind == MapKind::Modules => "uses",
            MapLinkKind::DependsOn => "depends on",
            MapLinkKind::Parent => "parent",
            MapLinkKind::Changes => "changed",
            MapLinkKind::SharedFiles => "shared files",
        };
        // Shared-file overlap is undirected; other edges preserve their direction.
        let arrow = if e.kind == MapLinkKind::SharedFiles {
            "---"
        } else {
            "-->"
        };
        writeln!(
            out,
            "  n{} {arrow}|{label}{}| n{}",
            e.from,
            if e.weight > 1 {
                format!(" ({})", e.weight)
            } else {
                String::new()
            },
            e.to
        )
        .unwrap();
    }
    cancel.check()?;
    Ok(out)
}
fn escaped(bytes: &[u8]) -> String {
    let mut out = String::new();
    for ch in String::from_utf8_lossy(bytes).chars() {
        if ch.is_control() {
            out.push(' ');
        } else if ch.is_ascii_punctuation()
            && !matches!(ch, '_' | '-' | '.' | '/' | ':' | '@' | '+')
        {
            write!(out, "#{};", ch as u32).unwrap();
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_only_selected_edges_and_escapes_repository_text() {
        let map = RepositoryMap {
            nodes: vec![
                node(
                    MapNodeKind::File,
                    b"a".to_vec(),
                    "結果 🚀 é\"]\nclick n0 \"https://example.invalid\"\n%%{init: <script>}"
                        .as_bytes()
                        .to_vec(),
                    vec![],
                ),
                node(MapNodeKind::File, b"b".to_vec(), b"end".to_vec(), vec![]),
            ],
            ..Default::default()
        };
        let edges = vec![link(0, 1, MapLinkKind::Contains)];
        let text = map_mermaid(&map, &[0], &edges, &CancellationToken::new()).unwrap();
        assert!(text.contains("結果 🚀 é#34;#93;"));
        assert!(!text.contains("\nclick ") && !text.contains("<script>") && !text.contains("%%{"));
        assert!(!text.contains("n1[\"") && !text.contains(" -->"));
        let text = map_mermaid(&map, &[0, 1], &edges, &CancellationToken::new()).unwrap();
        assert!(text.contains("n0 -->|contains| n1"));
        // Modules are named by their path in the crate.
        let modules = RepositoryMap {
            kind: MapKind::Modules,
            nodes: vec![
                node(MapNodeKind::Module, b"module:a::util".to_vec(), b"util".to_vec(), vec![]),
                node(MapNodeKind::Module, b"module:b::util".to_vec(), b"util".to_vec(), vec![]),
            ],
            ..Default::default()
        };
        let edges = vec![link(0, 1, MapLinkKind::DependsOn)];
        let text = map_mermaid(&modules, &[0, 1], &edges, &CancellationToken::new()).unwrap();
        assert!(text.contains("n0[\"a::util\"]") && text.contains("n1[\"b::util\"]") && text.contains("-->|uses|"), "{text}");
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            map_mermaid(&map, &[0], &edges, &cancel),
            Err(Error::Cancelled)
        ));
        assert!(map_mermaid(&map, &[2], &edges, &CancellationToken::new()).is_err());
    }
}
