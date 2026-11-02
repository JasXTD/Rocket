//! Saved shortcuts only. No filesystem operation belongs in this model.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub const MAX_REPOSITORIES: usize = 1024;
pub const MAX_FOLDERS: usize = 128;
pub const MAX_WORKSPACES: usize = 128;
pub const MAX_WORKSPACE_REPOSITORIES: usize = 64;
/// Eight visible levels, counting a top-level workspace as level one.
pub const MAX_WORKSPACE_DEPTH: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: u64,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedRepository {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
    pub folder: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub repositories: Vec<u64>,
    pub active: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub next_id: u64,
    pub folders: Vec<Folder>,
    pub repositories: Vec<SavedRepository>,
    pub workspaces: Vec<Workspace>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            next_id: 1,
            folders: vec![],
            repositories: vec![],
            workspaces: vec![],
        }
    }
}
type Checked<T> = Result<T, &'static str>;
pub fn valid_name(name: &str) -> Checked<()> {
    if name.trim() != name
        || name.is_empty()
        || name.len() > 512
        || name.chars().any(char::is_control)
    {
        Err(
            "Use a name with 1–512 UTF-8 bytes, without surrounding whitespace or control characters.",
        )
    } else {
        Ok(())
    }
}
#[cfg(windows)]
fn path_key(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide = p
        .as_os_str()
        .encode_wide()
        .map(|v| match v {
            65..=90 => v + 32,
            47 => 92,
            _ => v,
        })
        .collect::<Vec<_>>();
    if wide.starts_with(&[92, 92, 63, 92]) {
        wide.drain(..4);
        if wide.starts_with(&[117, 110, 99, 92]) {
            wide.splice(..4, [92, 92]);
        }
    }
    while wide.len() > 3 && wide.last() == Some(&92) {
        wide.pop();
    }
    wide
}
#[cfg(not(windows))]
fn path_key(p: &Path) -> PathBuf {
    p.to_path_buf()
}
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

impl Library {
    pub fn validate(&self) -> Checked<()> {
        if self.repositories.len() > MAX_REPOSITORIES
            || self.folders.len() > MAX_FOLDERS
            || self.workspaces.len() > MAX_WORKSPACES
        {
            return Err("The saved repository library exceeds its size limit.");
        }
        let mut ids = BTreeSet::new();
        for (id, name) in self
            .folders
            .iter()
            .map(|v| (v.id, &v.name))
            .chain(self.repositories.iter().map(|v| (v.id, &v.name)))
            .chain(self.workspaces.iter().map(|v| (v.id, &v.name)))
        {
            valid_name(name)?;
            if id == 0 || id >= self.next_id || !ids.insert(id) {
                return Err(
                    "Invalid saved repository identifiers; existing settings are preserved.",
                );
            }
        }
        if self.next_id == 0 || self.next_id == u64::MAX {
            return Err("Saved repository identifiers are exhausted.");
        }
        let unique_names = |names: Vec<&str>| {
            let count = names.len();
            names
                .iter()
                .map(|s| s.to_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                == count
        };
        if !unique_names(self.folders.iter().map(|v| v.name.as_str()).collect()) {
            return Err("Folder names must be unique.");
        }
        let mut names = BTreeSet::new();
        for workspace in &self.workspaces {
            if !names.insert((workspace.parent, workspace.name.to_lowercase())) {
                return Err(
                    "A workspace with that name already exists under this parent. Rename or move it first.",
                );
            }
            let mut seen = BTreeSet::from([workspace.id]);
            let mut parent = workspace.parent;
            while let Some(id) = parent {
                if !seen.insert(id) {
                    return Err(
                        "A workspace cannot be its own parent or be moved inside one of its children.",
                    );
                }
                if seen.len() > MAX_WORKSPACE_DEPTH {
                    return Err(
                        "Workspaces support up to eight nested levels. Choose a higher parent.",
                    );
                }
                parent = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == id)
                    .ok_or("Parent workspace no longer exists.")?
                    .parent;
            }
        }
        let mut paths = BTreeSet::new();
        for repo in &self.repositories {
            if !repo.path.is_absolute()
                || repo.path.as_os_str().is_empty()
                || repo.path.as_os_str().as_encoded_bytes().contains(&0)
            {
                return Err("Use an absolute repository folder path.");
            }
            if !paths.insert(path_key(&repo.path)) {
                return Err("This repository is already saved.");
            }
            if repo
                .folder
                .is_some_and(|id| !self.folders.iter().any(|f| f.id == id))
            {
                return Err("Saved folder no longer exists.");
            }
        }
        for workspace in &self.workspaces {
            if workspace.repositories.len() > MAX_WORKSPACE_REPOSITORIES
                || workspace.repositories.iter().collect::<BTreeSet<_>>().len()
                    != workspace.repositories.len()
                || workspace
                    .repositories
                    .iter()
                    .any(|id| !self.repositories.iter().any(|r| r.id == *id))
                || workspace
                    .active
                    .is_some_and(|id| !workspace.repositories.contains(&id))
            {
                return Err("Invalid workspace membership; review the saved repositories.");
            }
        }
        Ok(())
    }
    fn id(&mut self) -> Checked<u64> {
        if self.next_id == 0 || self.next_id >= u64::MAX - 1 {
            return Err("Saved repository identifiers are exhausted.");
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(id)
    }
    pub fn add_folder(&mut self, name: String) -> Checked<u64> {
        valid_name(&name)?;
        if self.folders.len() >= MAX_FOLDERS {
            return Err("At most 128 folders can be saved.");
        }
        if self
            .folders
            .iter()
            .any(|f| f.name.to_lowercase() == name.to_lowercase())
        {
            return Err("A folder with that name already exists.");
        }
        let id = self.id()?;
        self.folders.push(Folder { id, name });
        Ok(id)
    }
    pub fn rename_folder(&mut self, id: u64, name: String) -> Checked<()> {
        valid_name(&name)?;
        if self
            .folders
            .iter()
            .any(|f| f.id != id && f.name.to_lowercase() == name.to_lowercase())
        {
            return Err("A folder with that name already exists.");
        }
        self.folders
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or("Folder no longer exists.")?
            .name = name;
        Ok(())
    }
    pub fn remove_folder(&mut self, id: u64) -> Checked<()> {
        let i = self
            .folders
            .iter()
            .position(|f| f.id == id)
            .ok_or("Folder no longer exists.")?;
        self.folders.remove(i);
        for repo in &mut self.repositories {
            if repo.folder == Some(id) {
                repo.folder = None;
            }
        }
        Ok(())
    }
    /// Existing shortcuts keep their label and folder when a workspace is saved.
    pub fn save_repository(
        &mut self,
        path: PathBuf,
        name: String,
        folder: Option<u64>,
    ) -> Checked<u64> {
        if let Some(repo) = self.repositories.iter().find(|r| same_path(&r.path, &path)) {
            return Ok(repo.id);
        }
        valid_name(&name)?;
        if !path.is_absolute() {
            return Err("Use an absolute repository folder path.");
        }
        if folder.is_some_and(|id| !self.folders.iter().any(|f| f.id == id)) {
            return Err("Folder no longer exists.");
        }
        if self.repositories.len() >= MAX_REPOSITORIES {
            return Err("At most 1,024 repositories can be saved.");
        }
        let id = self.id()?;
        self.repositories.push(SavedRepository {
            id,
            name,
            path,
            folder,
        });
        Ok(id)
    }
    pub fn edit_repository(
        &mut self,
        id: u64,
        path: PathBuf,
        name: String,
        folder: Option<u64>,
    ) -> Checked<()> {
        valid_name(&name)?;
        if !path.is_absolute() {
            return Err("Use an absolute repository folder path.");
        }
        if self
            .repositories
            .iter()
            .any(|r| r.id != id && same_path(&r.path, &path))
        {
            return Err("That folder already has a saved repository.");
        }
        if folder.is_some_and(|id| !self.folders.iter().any(|f| f.id == id)) {
            return Err("Folder no longer exists.");
        }
        let repo = self
            .repositories
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Saved repository no longer exists.")?;
        repo.name = name;
        repo.path = path;
        repo.folder = folder;
        Ok(())
    }
    pub fn remove_repository(&mut self, id: u64) -> Checked<()> {
        let i = self
            .repositories
            .iter()
            .position(|r| r.id == id)
            .ok_or("Saved repository no longer exists.")?;
        self.repositories.remove(i);
        for workspace in &mut self.workspaces {
            workspace.repositories.retain(|r| *r != id);
            if workspace.active == Some(id) {
                workspace.active = workspace.repositories.first().copied();
            }
        }
        Ok(())
    }
    pub fn save_workspace(
        &mut self,
        id: Option<u64>,
        name: String,
        repositories: Vec<u64>,
        active: Option<u64>,
    ) -> Checked<u64> {
        // Existing callers editing a saved workspace must not flatten its hierarchy.
        let parent = id
            .and_then(|id| self.workspaces.iter().find(|w| w.id == id))
            .and_then(|w| w.parent);
        self.save_workspace_in(id, name, repositories, active, parent)
    }
    pub fn save_workspace_in(
        &mut self,
        id: Option<u64>,
        name: String,
        repositories: Vec<u64>,
        active: Option<u64>,
        parent: Option<u64>,
    ) -> Checked<u64> {
        valid_name(&name)?;
        if repositories.len() > MAX_WORKSPACE_REPOSITORIES {
            return Err(
                "Select up to 64 saved repositories. An empty workspace can organize child workspaces.",
            );
        }
        if repositories.iter().collect::<BTreeSet<_>>().len() != repositories.len()
            || repositories
                .iter()
                .any(|id| !self.repositories.iter().any(|r| r.id == *id))
            || active.is_some_and(|id| !repositories.contains(&id))
        {
            return Err("Review the workspace selection and active repository.");
        }
        // Validate the entire proposed tree before changing IDs, membership or parents.
        // Moving a subtree can exceed the depth limit even when its new root fits.
        let mut draft = self.clone();
        let id = if let Some(id) = id {
            let w = draft
                .workspaces
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or("Workspace no longer exists.")?;
            w.name = name;
            w.parent = parent;
            w.repositories = repositories;
            w.active = active;
            id
        } else {
            if draft.workspaces.len() >= MAX_WORKSPACES {
                return Err("At most 128 workspaces can be saved.");
            }
            let id = draft.id()?;
            draft.workspaces.push(Workspace {
                id,
                name,
                parent,
                repositories,
                active,
            });
            id
        };
        draft.validate()?;
        *self = draft;
        Ok(id)
    }
    pub fn remove_workspace(&mut self, id: u64) -> Checked<()> {
        let i = self
            .workspaces
            .iter()
            .position(|w| w.id == id)
            .ok_or("Workspace no longer exists.")?;
        let parent = self.workspaces[i].parent;
        let mut draft = self.clone();
        draft.workspaces.remove(i);
        for child in &mut draft.workspaces {
            if child.parent == Some(id) {
                child.parent = parent;
            }
        }
        // Promoting children can collide with a sibling name. Reject atomically.
        draft.validate()?;
        *self = draft;
        Ok(())
    }
    /// Stable pre-order. Both malformed input and a future bad caller remain bounded.
    pub fn workspace_tree(&self) -> Vec<(u64, usize)> {
        let mut stack: Vec<_> = self
            .workspaces
            .iter()
            .take(MAX_WORKSPACES)
            .rev()
            .filter(|w| w.parent.is_none())
            .map(|w| (w.id, 0))
            .collect();
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        while let Some((id, depth)) = stack.pop() {
            if depth >= MAX_WORKSPACE_DEPTH || !seen.insert(id) {
                continue;
            }
            rows.push((id, depth));
            for w in self
                .workspaces
                .iter()
                .take(MAX_WORKSPACES)
                .rev()
                .filter(|w| w.parent == Some(id))
            {
                stack.push((w.id, depth + 1));
            }
        }
        rows
    }
    pub fn workspace_contains(&self, ancestor: u64, id: u64) -> bool {
        let mut next = Some(id);
        for _ in 0..MAX_WORKSPACE_DEPTH {
            let Some(id) = next else {
                break;
            };
            if id == ancestor {
                return true;
            }
            next = self
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .and_then(|w| w.parent);
        }
        false
    }
    pub fn workspace_path(&self, id: u64) -> Option<String> {
        let mut names = Vec::new();
        let mut next = Some(id);
        for _ in 0..MAX_WORKSPACE_DEPTH {
            let Some(id) = next else {
                break;
            };
            let w = self.workspaces.iter().find(|w| w.id == id)?;
            names.push(w.name.as_str());
            next = w.parent;
        }
        if next.is_some() {
            return None;
        }
        names.reverse();
        Some(names.join(" / "))
    }
    pub fn workspace_members(&self, id: u64, descendants: bool) -> Checked<Vec<u64>> {
        let tree = self.workspace_tree();
        let start = tree
            .iter()
            .position(|(wid, _)| *wid == id)
            .ok_or("Workspace no longer exists.")?;
        let depth = tree[start].1;
        let mut members = Vec::new();
        let mut seen = BTreeSet::new();
        for (at, (wid, level)) in tree[start..].iter().enumerate() {
            if at > 0 && (!descendants || *level <= depth) {
                break;
            }
            if let Some(w) = self.workspaces.iter().find(|w| w.id == *wid) {
                for id in &w.repositories {
                    if seen.insert(*id) {
                        members.push(*id);
                    }
                    if members.len() > MAX_WORKSPACE_REPOSITORIES {
                        return Err(
                            "This selection contains more than 64 unique repositories. Open a smaller workspace or turn off Include nested workspaces.",
                        );
                    }
                }
            }
        }
        Ok(members)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn p(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }
    #[test]
    fn nested_workspace_tree_and_explicit_opening_keep_stable_unique_members() {
        let mut l = Library::default();
        let a = l
            .save_repository(p("資料 🚀"), "資料 🚀".into(), None)
            .unwrap();
        let b = l.save_repository(p("two"), "Two".into(), None).unwrap();
        let c = l.save_repository(p("three"), "Three".into(), None).unwrap();
        let root = l
            .save_workspace(None, "仕事".into(), vec![a], Some(a))
            .unwrap();
        let other = l
            .save_workspace(None, "Personal".into(), vec![c], Some(c))
            .unwrap();
        let child = l
            .save_workspace_in(None, "Rocket".into(), vec![a, b], Some(b), Some(root))
            .unwrap();
        let grandchild = l
            .save_workspace_in(None, "Notes".into(), vec![c], Some(c), Some(child))
            .unwrap();
        assert_eq!(
            l.workspace_tree(),
            vec![(root, 0), (child, 1), (grandchild, 2), (other, 0)]
        );
        assert_eq!(l.workspace_members(root, false).unwrap(), vec![a]);
        assert_eq!(l.workspace_members(root, true).unwrap(), vec![a, b, c]);
        assert_eq!(
            l.workspace_path(grandchild).as_deref(),
            Some("仕事 / Rocket / Notes")
        );
        assert!(l.workspace_contains(root, grandchild));
        assert!(!l.workspace_contains(child, root));
        let repositories = l.repositories.clone();
        l.save_workspace_in(
            Some(child),
            "Rocket".into(),
            vec![a, b],
            Some(b),
            Some(other),
        )
        .unwrap();
        assert_eq!(
            l.workspace_path(grandchild).as_deref(),
            Some("Personal / Rocket / Notes")
        );
        assert_eq!(l.repositories, repositories);
        // Legacy callers retain the parent while updating an existing workspace.
        l.save_workspace(Some(child), "Renamed".into(), vec![b], Some(b))
            .unwrap();
        assert_eq!(
            l.workspaces.iter().find(|w| w.id == child).unwrap().parent,
            Some(other)
        );
    }
    #[test]
    fn nested_workspace_invalid_moves_and_removal_collisions_are_atomic() {
        let mut l = Library::default();
        let root = l.save_workspace(None, "Work".into(), vec![], None).unwrap();
        let child = l
            .save_workspace_in(None, "Repeated".into(), vec![], None, Some(root))
            .unwrap();
        let sibling = l
            .save_workspace(None, "Repeated".into(), vec![], None)
            .unwrap();
        for (id, parent) in [(root, Some(root)), (root, Some(child)), (child, Some(999))] {
            let before = l.clone();
            assert!(
                l.save_workspace_in(Some(id), "Moved".into(), vec![], None, parent)
                    .is_err()
            );
            assert_eq!(l, before);
        }
        let before = l.clone();
        assert!(l.remove_workspace(root).is_err());
        assert_eq!(l, before);
        assert!(
            l.save_workspace_in(None, "repeated".into(), vec![], None, Some(root))
                .is_err()
        );
        assert_eq!(l, before);
        l.save_workspace(Some(sibling), "Unique".into(), vec![], None)
            .unwrap();
        l.remove_workspace(root).unwrap();
        assert_eq!(
            l.workspaces.iter().find(|w| w.id == child).unwrap().parent,
            None
        );
        l.validate().unwrap();
    }
    #[test]
    fn nested_workspace_depth_and_open_limits_apply_to_the_whole_subtree() {
        let mut l = Library::default();
        let mut parent = None;
        for n in 0..MAX_WORKSPACE_DEPTH {
            parent = Some(
                l.save_workspace_in(None, format!("Level {n}"), vec![], None, parent)
                    .unwrap(),
            );
        }
        let before = l.clone();
        assert!(
            l.save_workspace_in(None, "Too deep".into(), vec![], None, parent)
                .is_err()
        );
        assert_eq!(l, before);
        let moving = l
            .save_workspace(None, "Move me".into(), vec![], None)
            .unwrap();
        l.save_workspace_in(None, "My child".into(), vec![], None, Some(moving))
            .unwrap();
        let before = l.clone();
        let seventh = l.workspaces[MAX_WORKSPACE_DEPTH - 2].id;
        assert!(
            l.save_workspace_in(Some(moving), "Move me".into(), vec![], None, Some(seventh))
                .is_err()
        );
        assert_eq!(l, before);
        let mut members = Vec::new();
        for i in 0..65 {
            members.push(
                l.save_repository(p(&format!("nested-limit-{i}")), format!("Repo {i}"), None)
                    .unwrap(),
            );
        }
        l.save_workspace(
            Some(moving),
            "Move me".into(),
            members[..64].to_vec(),
            Some(members[0]),
        )
        .unwrap();
        let child = l
            .workspaces
            .iter()
            .find(|w| w.parent == Some(moving))
            .unwrap()
            .id;
        l.save_workspace(
            Some(child),
            "My child".into(),
            vec![members[0]],
            Some(members[0]),
        )
        .unwrap();
        assert_eq!(l.workspace_members(moving, true).unwrap().len(), 64);
        l.save_workspace(
            Some(child),
            "My child".into(),
            vec![members[64]],
            Some(members[64]),
        )
        .unwrap();
        assert!(l.workspace_members(moving, true).is_err());
        assert_eq!(l.workspace_members(moving, false).unwrap().len(), 64);
    }
    #[test]
    fn nested_workspace_corrupt_graphs_and_maximum_size_are_bounded() {
        let mut l = Library::default();
        for i in 0..MAX_WORKSPACES {
            l.save_workspace(None, format!("Group {i}"), vec![], None)
                .unwrap();
        }
        assert_eq!(l.workspace_tree().len(), MAX_WORKSPACES);
        let before = l.clone();
        assert!(
            l.save_workspace(None, "Overflow".into(), vec![], None)
                .is_err()
        );
        assert_eq!(l, before);
        l.workspaces[0].parent = Some(l.workspaces[1].id);
        l.workspaces[1].parent = Some(l.workspaces[0].id);
        assert!(l.validate().is_err());
        assert_eq!(l.workspace_tree().len(), MAX_WORKSPACES - 2);
        assert!(l.workspace_path(l.workspaces[0].id).is_none());
        assert!(l.workspace_members(l.workspaces[0].id, true).is_err());
    }
    #[test]
    fn library_removal_only_changes_shortcuts_and_preserves_membership() {
        let mut l = Library::default();
        let f = l.add_folder("仕事 / Work".into()).unwrap();
        let a = l
            .save_repository(p("日本語 🚀"), "日本語 🚀".into(), Some(f))
            .unwrap();
        let b = l.save_repository(p("other"), "Other".into(), None).unwrap();
        let w = l
            .save_workspace(None, "Development".into(), vec![a, b], Some(a))
            .unwrap();
        l.edit_repository(a, p("moved"), "Renamed".into(), Some(f))
            .unwrap();
        assert_eq!(l.workspaces[0].repositories, vec![a, b]);
        l.remove_folder(f).unwrap();
        assert_eq!(l.repositories[0].folder, None);
        l.remove_repository(a).unwrap();
        assert_eq!(l.workspaces[0].repositories, vec![b]);
        assert_eq!(l.workspaces[0].active, Some(b));
        l.remove_workspace(w).unwrap();
        assert_eq!(l.repositories.len(), 1);
        l.validate().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn windows_path_aliases_keep_the_same_shortcut() {
        assert!(same_path(
            Path::new("D:/Projects/Repo/"),
            Path::new(r"\\?\d:\projects\repo")
        ));
        assert!(same_path(
            Path::new(r"\\server\share\repo"),
            Path::new(r"\\?\UNC\SERVER\share\repo")
        ));
    }
    #[test]
    fn library_limit_and_duplicate_paths_are_checked() {
        let mut l = Library::default();
        for i in 0..MAX_REPOSITORIES {
            l.save_repository(p(&format!("repo-{i}")), format!("Repo {i}"), None)
                .unwrap();
        }
        l.validate().unwrap();
        assert!(
            l.save_repository(p("overflow"), "Overflow".into(), None)
                .is_err()
        );
        l.repositories[10].path = l.repositories[0].path.clone();
        assert!(l.validate().is_err());
    }
    #[test]
    fn library_rejects_duplicate_names_dangling_members_and_invalid_ids() {
        let mut l = Library::default();
        l.add_folder("Work".into()).unwrap();
        let before = l.clone();
        assert!(l.add_folder("work".into()).is_err());
        assert_eq!(l, before);
        assert!(
            l.save_workspace(None, "Missing".into(), vec![99], None)
                .is_err()
        );
        assert_eq!(l, before);
        let id = l.save_repository(p("repo"), "Repo".into(), None).unwrap();
        assert_eq!(
            l.save_repository(p("repo"), "Alias".into(), None).unwrap(),
            id
        );
        assert_eq!(l.repositories.len(), 1);
        l.next_id = id;
        assert!(l.validate().is_err());
        assert!(valid_name("bad\nlabel").is_err());
    }
}
