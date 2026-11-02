//! Per-checkout presentation settings; no Git configuration or draft ownership changes.
use crate::app::Tab;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub branch_filter: String,
    pub history_ref: Option<String>,
    pub history_all: bool,
    pub history_order: rocket_git::HistoryOrder,
    pub hide_tags: bool,
    pub hide_stashes: bool,
    /// Branches, remotes, stashes, repository.
    pub sections: [bool; 4],
    pub split_diff: bool,
    pub folding: crate::graph::Folding,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            branch_filter: String::new(),
            history_ref: None,
            history_all: false,
            history_order: Default::default(),
            hide_tags: false,
            hide_stashes: false,
            sections: [true; 4],
            split_diff: false,
            folding: Default::default(),
        }
    }
}
impl Settings {
    pub fn capture(tab: &Tab) -> Self {
        Self {
            branch_filter: tab.branch_filter.clone(),
            history_ref: tab.history_ref.clone(),
            history_all: tab.history_all,
            history_order: tab.history_order,
            split_diff: tab.split_diff,
            folding: tab.folding.clone(),
            ..tab.worktree_view.clone()
        }
    }
    pub fn restore(self, tab: &mut Tab) {
        tab.branch_filter = self.branch_filter.clone();
        tab.history_ref = self.history_ref.clone();
        tab.history_all = self.history_all;
        tab.history_order = self.history_order;
        tab.split_diff = self.split_diff;
        tab.folding = self.folding.clone();
        tab.worktree_view = self;
    }
    pub fn valid(&self) -> bool {
        self.branch_filter.len() <= 4096
            && !self.branch_filter.contains('\0')
            && self.history_ref.as_ref().is_none_or(|r| {
                r.starts_with("refs/") && r.len() <= 4096 && !r.chars().any(char::is_control)
            })
            && !(self.history_all && self.history_ref.is_some())
            && self.folding.except.len() <= 2048
    }
}
