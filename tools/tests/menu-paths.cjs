'use strict';
// Repository-menu items that live in a submenu, and the row that opens it. Items not
// listed sit on the menu itself.
const SUBMENU = Object.freeze({
  'commit-amend': 'repository-commit-menu',
  'commit-cherry-pick': 'repository-commit-menu',
  'commit-revert': 'repository-commit-menu',
  'repository-merge': 'repository-branch-menu',
  'repository-rebase': 'repository-branch-menu',
  'repository-interactive-rebase': 'repository-branch-menu',
  'repository-delete-branches': 'repository-branch-menu',
  'repository-workflows': 'repository-branch-menu',
  'repository-patch-import': 'repository-patches-menu',
  'repository-series-import': 'repository-patches-menu',
  'repository-series-export': 'repository-patches-menu',
  'repository-patch-clipboard': 'repository-patches-menu',
  'repository-patch-export': 'repository-patches-menu',
  'repository-recovery': 'repository-history-menu',
  'repository-file-backups': 'repository-history-menu',
  'repository-tree': 'repository-history-menu',
  'repository-bisect': 'repository-history-menu',
  'repository-manage-remotes': 'repository-remotes-menu',
  'repository-hosting': 'repository-remotes-menu',
  'repository-github': 'repository-remotes-menu',
  'repository-settings': 'repository-settings-menu',
  'repository-signing': 'repository-settings-menu',
  'repository-ignore': 'repository-settings-menu',
  'repository-dependencies': 'repository-settings-menu',
});
// Pages that left the Repository menu for View → Go to, by their old id.
const GO_TO = Object.freeze({
  'repository-tags': 'goto-tags',
  'repository-worktrees': 'goto-worktrees',
  'repository-submodules': 'goto-submodules',
  'repository-lfs': 'goto-lfs',
  'repository-statistics': 'goto-statistics',
  'repository-review': 'goto-review',
  'menu-integrations': 'goto-integrations',
});
// The clicks that open a Repository-menu item (by its old id) from the menu bar.
function menuPath(id) {
  if (GO_TO[id]) return ['menu-view', 'view-goto', GO_TO[id]];
  return SUBMENU[id] ? ['menu-repository', SUBMENU[id], id] : ['menu-repository', id];
}
module.exports = { SUBMENU, GO_TO, menuPath };
