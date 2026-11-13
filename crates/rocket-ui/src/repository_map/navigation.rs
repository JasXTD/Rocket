use super::*;
use std::collections::BTreeMap;

pub(super) fn is_group(data: &RepositoryMap, i: usize) -> bool {
    data.nodes[i].kind == MapNodeKind::Directory
        || data.nodes[i].kind == MapNodeKind::Crate
            && data
                .links
                .iter()
                .any(|e| e.kind == MapLinkKind::Contains && e.from == i)
}

impl MapPage {
    pub(super) fn toggle_group(&mut self, i: usize) {
        let Some(n) = self.data.as_ref().and_then(|d| d.nodes.get(i)) else {
            return;
        };
        if !is_group(self.data.as_ref().unwrap(), i) {
            return;
        }
        let key = n.key.clone();
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        self.filter_key = None;
        self.display_offset = 0;
    }
    pub(super) fn neighbors(&mut self) {
        self.neighbor_anchor = if self.neighbor_anchor.is_some() {
            None
        } else {
            self.selected
                .and_then(|i| self.data.as_ref()?.nodes.get(i))
                .map(|n| n.key.clone())
        };
        self.filter_key = None;
        self.display_offset = 0;
    }
    /// The folder tree shows folders only: a file counts towards its folder.
    fn folders_view(&self) -> bool {
        self.data
            .as_ref()
            .is_some_and(|d| d.kind == MapKind::Code && !self.alternate)
    }
    pub(super) fn rebuild(&mut self) {
        if self.data.as_ref().is_some_and(|d| d.kind == MapKind::Modules) {
            modules::rebuild(self);
            return;
        }
        let key = (self.query.clone(), self.alternate, None, false);
        if self.filter_key.as_ref() == Some(&key) {
            return;
        }
        if self
            .filter_key
            .as_ref()
            .is_some_and(|previous| previous != &key)
        {
            self.display_offset = 0;
        }
        self.filter_key = Some(key);
        let folders = self.folders_view();
        let Some(data) = &self.data else { return };
        let mut parent = vec![None; data.nodes.len()];
        for e in &data.links {
            if e.kind == MapLinkKind::Contains {
                parent[e.to] = Some(e.from);
            }
        }
        let representative = (0..data.nodes.len())
            .map(|i| {
                let mut result = i;
                let mut p = parent[i];
                for _ in 0..64 {
                    let Some(at) = p else { break };
                    if self.collapsed.contains(&data.nodes[at].key) && !self.alternate {
                        result = at;
                    }
                    p = parent[at];
                }
                result
            })
            .collect::<Vec<_>>();
        // Where a node shows: its collapsed ancestor, and in the folder tree a file's
        // folder. Files at the top have no card.
        let shown = |i: usize| -> Option<usize> {
            if folders && data.nodes[i].kind == MapNodeKind::File {
                parent[i].map(|p| representative[p])
            } else {
                Some(representative[i])
            }
        };
        self.projected_links = if self.collapsed.is_empty() || self.alternate {
            data.links.clone()
        } else {
            let mut lookup = BTreeMap::new();
            let mut edges: Vec<MapLink> = Vec::new();
            for e in &data.links {
                let (a, b) = (representative[e.from], representative[e.to]);
                if a == b {
                    continue;
                }
                let key = (a, b, e.kind as u8);
                if let Some(&i) = lookup.get(&key) {
                    let target: &mut MapLink = &mut edges[i];
                    target.weight = target.weight.saturating_add(e.weight);
                } else {
                    lookup.insert(key, edges.len());
                    let mut edge = e.clone();
                    edge.from = a;
                    edge.to = b;
                    edges.push(edge);
                }
            }
            edges
        };
        let anchor = self
            .neighbor_anchor
            .as_ref()
            .and_then(|key| data.nodes.iter().position(|n| &n.key == key))
            .and_then(shown);
        let mut neighborhood = HashSet::new();
        if let Some(i) = anchor {
            neighborhood.insert(i);
            for e in &self.projected_links {
                if self.edge_kind(e.kind) {
                    if e.from == i {
                        neighborhood.insert(e.to);
                    }
                    if e.to == i {
                        neighborhood.insert(e.from);
                    }
                }
            }
        }
        let allowed = |i: usize| anchor.is_none() || neighborhood.contains(&i);
        let kept = |i: usize| -> bool {
            !(folders && data.nodes[i].kind == MapNodeKind::File)
        };
        let mut matched = HashSet::new();
        let query = self.query.to_lowercase();
        for (i, n) in data.nodes.iter().enumerate() {
            let Some(r) = shown(i) else { continue };
            if !allowed(r) {
                continue;
            }
            if match data.kind {
                MapKind::Code => self.alternate && n.kind != MapNodeKind::Crate,
                MapKind::People => self.alternate && n.kind != MapNodeKind::Person,
                MapKind::History | MapKind::Modules => false,
            } {
                continue;
            }
            if query.is_empty()
                || [
                    label(&n.label),
                    label(&n.detail),
                    n.path.as_ref().map(|p| label(p)).unwrap_or_default(),
                    n.commit
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    n.refs.iter().map(|r| label(r)).collect::<Vec<_>>().join(" "),
                    n.evidence
                        .iter()
                        .map(|p| label(p))
                        .collect::<Vec<_>>()
                        .join(" "),
                ]
                .join(" ")
                .to_lowercase()
                .contains(&query)
            {
                matched.insert(r);
            }
        }
        let mut context = matched.clone();
        if !query.is_empty() {
            for e in &self.projected_links {
                if self.edge_kind(e.kind) && (matched.contains(&e.from) || matched.contains(&e.to))
                {
                    if allowed(e.from) && kept(e.from) {
                        context.insert(e.from);
                    }
                    if allowed(e.to) && kept(e.to) {
                        context.insert(e.to);
                    }
                }
            }
        }
        let mut ordered = Vec::new();
        let mut chosen = HashSet::new();
        if let Some(i) = anchor.filter(|i| matched.contains(i)) {
            chosen.insert(i);
            ordered.push(i);
        }
        for set in [&matched, &context] {
            for i in 0..data.nodes.len() {
                if set.contains(&i) && chosen.insert(i) {
                    ordered.push(i);
                }
            }
        }
        self.view_total = ordered.len();
        if self.display_offset >= self.view_total {
            self.display_offset = self.view_total.saturating_sub(1) / DISPLAY_NODES * DISPLAY_NODES;
        }
        self.visible = ordered
            .into_iter()
            .skip(self.display_offset)
            .take(DISPLAY_NODES)
            .collect();
        self.visible.sort_unstable();
        if let Some(i) = self.selected {
            self.selected = shown(i);
        }
        if !self.selected.is_some_and(|i| self.visible.contains(&i)) {
            self.selected = self.visible.first().copied();
        }
        if !query.is_empty() && !self.selected.is_some_and(|i| matched.contains(&i)) {
            self.selected = self.visible.iter().copied().find(|i| matched.contains(i));
        }
        // Layout the same expanded/collapsed structure, independently of search
        // and neighbor filtering. A retained node does not jump between lanes.
        let layout_nodes = (0..data.nodes.len())
            .filter(|i| {
                representative[*i] == *i
                    && kept(*i)
                    && match data.kind {
                        MapKind::Code => {
                            !self.alternate || data.nodes[*i].kind == MapNodeKind::Crate
                        }
                        MapKind::People => {
                            !self.alternate || data.nodes[*i].kind == MapNodeKind::Person
                        }
                        _ => true,
                    }
            })
            .collect::<Vec<_>>();
        let (positions, groups) = drawing::layout(data, &layout_nodes, self.alternate, &self.collapsed);
        self.positions = positions;
        // A folder's files card shows only while the folder does.
        let visible: HashSet<usize> = self.visible.iter().copied().collect();
        self.file_groups = groups
            .into_iter()
            .filter(|g| g.folder.is_none_or(|p| visible.contains(&p)) && (query.is_empty() || g.folder.is_some()))
            .collect();
        self.topology = None;
        self.fit = true;
    }
}
