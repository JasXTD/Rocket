//! Topological lanes derived exclusively from the loaded commit IDs and parents.
use rocket_git::{Commit, ObjectId};
use std::collections::{HashMap, HashSet};

const MAX_LANES: usize = 64;
const MAX_RENDERED_PARENTS: usize = 32;

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub color: usize,
    pub from_node: bool,
    pub to_node: bool,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub lane: usize,
    pub color: usize,
    pub edges: Vec<Edge>,
    /// Some ancestry was omitted for a bounded, responsive visualization.
    pub limited: bool,
}
#[derive(Clone)]
struct Lane {
    id: ObjectId,
    color: usize,
}

/// Collapsed merge commits: every merge when `all` is set, except those in
/// `except`; otherwise only those in `except`. Loading more history keeps the choice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folding {
    pub all: bool,
    pub except: HashSet<ObjectId>,
}
impl Folding {
    pub fn collapsed(&self, commit: &Commit) -> bool {
        commit.parents.len() > 1 && self.all != self.except.contains(&commit.id)
    }
    pub fn set(&mut self, merge: &ObjectId, collapse: bool) {
        if collapse == self.all {
            self.except.remove(merge);
        } else {
            self.except.insert(merge.clone());
        }
    }
    pub fn set_all(&mut self, collapse: bool) {
        self.all = collapse;
        self.except.clear();
    }
    pub fn active(&self) -> bool {
        self.all || !self.except.is_empty()
    }
}

/// The loaded commits that stay visible: those reachable from a commit with no
/// loaded child, where a collapsed merge leads only to its first parent.
pub struct Folded {
    /// Indexes of the visible commits, in list order.
    pub visible: Vec<usize>,
    /// The commits no longer shown.
    pub hidden: HashSet<ObjectId>,
    /// For each visible collapsed merge, how many loaded commits it hides.
    pub behind: HashMap<ObjectId, usize>,
    /// For each hidden commit, the shown collapsed merge it is counted for.
    pub owner: HashMap<ObjectId, ObjectId>,
}
pub fn fold(commits: &[Commit], folding: &Folding) -> Folded {
    let index: HashMap<&ObjectId, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (&c.id, i))
        .collect();
    let parents = |c: &Commit| -> Vec<usize> {
        c.parents
            .iter()
            .filter_map(|p| index.get(p).copied())
            .collect()
    };
    let mut has_child = vec![false; commits.len()];
    for c in commits {
        for p in parents(c) {
            has_child[p] = true;
        }
    }
    let mut visible = vec![false; commits.len()];
    let mut stack: Vec<usize> = (0..commits.len()).filter(|&i| !has_child[i]).collect();
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut visible[i], true) {
            continue;
        }
        let c = &commits[i];
        let followed = if folding.collapsed(c) {
            &c.parents[..1]
        } else {
            &c.parents[..]
        };
        stack.extend(followed.iter().filter_map(|p| index.get(p).copied()));
    }
    // Each hidden commit counts once, for the newest collapsed merge it sits behind.
    let mut counted = vec![false; commits.len()];
    let mut behind = HashMap::new();
    let mut owner = HashMap::new();
    for (i, c) in commits.iter().enumerate() {
        if !visible[i] || !folding.collapsed(c) {
            continue;
        }
        let mut count = 0;
        let mut stack: Vec<usize> = c.parents[1..]
            .iter()
            .filter_map(|p| index.get(p).copied())
            .collect();
        while let Some(j) = stack.pop() {
            if visible[j] || std::mem::replace(&mut counted[j], true) {
                continue;
            }
            count += 1;
            owner.insert(commits[j].id.clone(), c.id.clone());
            stack.extend(parents(&commits[j]));
        }
        behind.insert(c.id.clone(), count);
    }
    Folded {
        visible: (0..commits.len()).filter(|&i| visible[i]).collect(),
        hidden: (0..commits.len())
            .filter(|&i| !visible[i])
            .map(|i| commits[i].id.clone())
            .collect(),
        behind,
        owner,
    }
}

/// Lanes for the shown commits. A parent in `hidden` (behind a collapsed merge)
/// gets no lane; parents that are simply not loaded yet keep theirs.
pub fn layout(commits: &[Commit], hidden: &HashSet<ObjectId>) -> Vec<Row> {
    let shown = |c: &Commit| -> Vec<ObjectId> {
        c.parents
            .iter()
            .take(MAX_RENDERED_PARENTS)
            .filter(|p| !hidden.contains(*p))
            .cloned()
            .collect()
    };
    let parents: HashMap<&ObjectId, Vec<ObjectId>> =
        commits.iter().map(|c| (&c.id, shown(c))).collect();
    let mut lanes: Vec<Lane> = Vec::new();
    let mut next_color = 0;
    // Assign the first-parent spine before walking interleaved side branches.
    // Otherwise an earlier side-branch row can recolor the main line at a join.
    let by_id: HashMap<_, _> = commits.iter().map(|c| (&c.id, c)).collect();
    let mut colors = HashMap::new();
    for commit in commits {
        if colors.contains_key(&commit.id) {
            continue;
        }
        let mut cursor = Some(&commit.id);
        while let Some(id) = cursor {
            if colors.contains_key(id) {
                break;
            }
            colors.insert(id, next_color);
            cursor = by_id.get(id).and_then(|c| c.parents.first());
        }
        next_color += 1;
    }
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let mut limited = commit.parents.len() > MAX_RENDERED_PARENTS;
        let lane = lanes
            .iter()
            .position(|lane| lane.id == commit.id)
            .unwrap_or_else(|| {
                if lanes.len() == MAX_LANES {
                    lanes.pop();
                    limited = true;
                }
                let index = lanes.len();
                lanes.push(Lane {
                    id: commit.id.clone(),
                    color: colors[&commit.id],
                });
                index
            });
        let before = lanes.clone();
        let color = lanes.remove(lane).color;
        for (i, parent) in parents[&commit.id].iter().enumerate() {
            if !lanes.iter().any(|l| &l.id == parent) {
                if lanes.len() == MAX_LANES {
                    limited = true;
                    continue;
                }
                let insert = (lane + i).min(lanes.len());
                let parent_color = colors.get(parent).copied().unwrap_or_else(|| {
                    let c = next_color;
                    next_color += 1;
                    c
                });
                lanes.insert(
                    insert,
                    Lane {
                        id: parent.clone(),
                        color: parent_color,
                    },
                );
            }
        }
        let mut edges = Vec::new();
        for (i, previous) in before.iter().enumerate() {
            if i == lane {
                edges.push(Edge {
                    from: i,
                    to: lane,
                    color,
                    from_node: false,
                    to_node: true,
                });
            } else if let Some(to) = lanes.iter().position(|l| l.id == previous.id) {
                edges.push(Edge {
                    from: i,
                    to,
                    color: previous.color,
                    from_node: false,
                    to_node: false,
                });
            }
        }
        for (i, parent) in parents[&commit.id].iter().enumerate() {
            if let Some(to) = lanes.iter().position(|l| &l.id == parent) {
                edges.push(Edge {
                    from: lane,
                    to,
                    color: if i == 0 { color } else { lanes[to].color },
                    from_node: true,
                    to_node: false,
                });
            }
        }
        rows.push(Row { lane, color, edges, limited });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adversarial_parent_lists_and_many_live_branches_stay_bounded() {
        let mut commits = Vec::new();
        for n in 1..=100 {
            let mut c = commit(n, &[]);
            let start = u32::from(n) * 21_000;
            c.parents = (start..start + 20_000).map(|p| ObjectId::parse(&format!("{p:040x}")).unwrap()).collect();
            commits.push(c);
        }
        let rows = layout(&commits, &HashSet::new());
        assert_eq!(rows.len(), commits.len());
        assert!(rows.iter().all(|r| r.limited && r.lane < MAX_LANES));
        assert!(rows.iter().any(|r| r.lane == MAX_LANES - 1), "fixture must reach the lane cap");
        assert!(rows.iter().all(|r| r.edges.len() <= MAX_LANES + MAX_RENDERED_PARENTS));
        assert!(rows.iter().flat_map(|r| &r.edges).all(|e| e.from < MAX_LANES && e.to < MAX_LANES));
    }
    fn commit(n: u8, parents: &[u8]) -> Commit {
        let oid = |n| ObjectId::parse(&format!("{n:040x}")).unwrap();
        let signature = rocket_git::Signature {
            name: vec![],
            email: vec![],
            timestamp: 0,
            offset_minutes: 0,
        };
        Commit {
            id: oid(n),
            tree: oid(0),
            parents: parents.iter().map(|n| oid(*n)).collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: vec![],
        }
    }
    #[test]
    fn first_parent_color_survives_a_side_branch_joining_it() {
        let rows = layout(
            &[
                commit(4, &[2, 3]),
                commit(3, &[1]),
                commit(2, &[1]),
                commit(1, &[]),
            ],
            &HashSet::new(),
        );
        assert_ne!(rows[0].color, rows[1].color);
        assert_eq!(rows[0].color, rows[2].color);
        assert_eq!(rows[0].color, rows[3].color);
    }
    #[test]
    fn merge_lanes_join_the_shared_parent_and_root_ends() {
        let rows = layout(
            &[
                commit(4, &[2, 3]),
                commit(3, &[1]),
                commit(2, &[1]),
                commit(1, &[]),
            ],
            &HashSet::new(),
        );
        assert_eq!(rows[0].edges.iter().filter(|e| e.from_node).count(), 2);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].edges.iter().find(|e| e.from_node).unwrap().to, 0);
        assert!(rows[3].edges.iter().all(|e| !e.from_node));
    }
    /// main: 7 merges 6 (side: 5, 4); 4 is also merged by 3. 2 is a later merge of 1.
    fn merges() -> Vec<Commit> {
        vec![
            commit(9, &[8]),
            commit(8, &[7, 3]),
            commit(7, &[6, 5]),
            commit(6, &[2]),
            commit(5, &[4]),
            commit(3, &[2, 4]),
            commit(4, &[2]),
            commit(2, &[1, 10]),
            commit(10, &[1]),
            commit(1, &[]),
        ]
    }
    fn ids(commits: &[Commit], visible: &[usize]) -> Vec<String> {
        visible
            .iter()
            .map(|&i| commits[i].id.as_str()[38..].to_owned())
            .collect()
    }
    #[test]
    fn collapsed_merges_hide_only_what_no_other_path_reaches() {
        let commits = merges();
        let id = |n: u8| ObjectId::parse(&format!("{n:040x}")).unwrap();
        let mut folding = Folding::default();
        let all = fold(&commits, &folding);
        assert_eq!(all.visible.len(), commits.len());
        assert!(all.hidden.is_empty() && all.behind.is_empty());
        // Collapsing 7 hides 5 only: 4 is still reached through the merge 3.
        folding.set(&id(7), true);
        let folded = fold(&commits, &folding);
        assert_eq!(
            ids(&commits, &folded.visible),
            ["09", "08", "07", "06", "03", "04", "02", "0a", "01"]
        );
        assert_eq!(folded.behind[&id(7)], 1);
        // Collapsing 3 as well hides 4 and 3's side; 8 still reaches 3 itself.
        folding.set(&id(3), true);
        let folded = fold(&commits, &folding);
        assert_eq!(
            ids(&commits, &folded.visible),
            ["09", "08", "07", "06", "03", "02", "0a", "01"]
        );
        assert_eq!((folded.behind[&id(7)], folded.behind[&id(3)]), (2, 0));
        // Collapsing everything keeps the first-parent line; the hidden merge 3
        // is not counted on its own, and 4 (behind both 8 and 7) counts for 8.
        folding.set_all(true);
        let folded = fold(&commits, &folding);
        assert_eq!(
            ids(&commits, &folded.visible),
            ["09", "08", "07", "06", "02", "01"]
        );
        assert_eq!(folded.behind[&id(8)], 2);
        assert_eq!(folded.behind[&id(7)], 1);
        assert_eq!(folded.behind[&id(2)], 1);
        for (hidden, merge) in [(3, 8), (4, 8), (5, 7), (10, 2)] {
            assert_eq!(folded.owner[&id(hidden)], id(merge));
        }
        assert!(!folded.behind.contains_key(&id(3)));
        // One merge expanded again while the rest stay collapsed.
        folding.set(&id(8), false);
        assert!(!folding.collapsed(&commits[1]) && folding.collapsed(&commits[2]));
        folding.set_all(false);
        assert!(!folding.active());
    }
    #[test]
    fn a_collapsed_merge_draws_no_lane_for_hidden_parents() {
        let commits = merges();
        let mut folding = Folding::default();
        folding.set_all(true);
        let folded = fold(&commits, &folding);
        let shown: Vec<Commit> = folded.visible.iter().map(|&i| commits[i].clone()).collect();
        let rows = layout(&shown, &folded.hidden);
        // A single lane: every shown commit follows its first parent.
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(
            rows.iter()
                .all(|r| r.edges.iter().filter(|e| e.from_node).count() <= 1)
        );
        // Expanded, the merges open side lanes.
        let rows = layout(&commits, &HashSet::new());
        assert!(rows.iter().any(|r| r.lane > 0));
    }
}
