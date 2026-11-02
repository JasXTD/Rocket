//! Reviewed stash removal and selected-line saves. Content is retained before cleanup.
use crate::{runner::args, *};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StashControl {
    Pop(StashEntry),
    Clear,
}
#[derive(Clone, Debug)]
pub struct StashControlPreview {
    repository: PathBuf,
    action: StashControl,
    entries: Vec<StashEntry>,
    head: Head,
}
impl StashControlPreview {
    pub fn action(&self) -> &StashControl {
        &self.action
    }
    pub fn entries(&self) -> &[StashEntry] {
        &self.entries
    }
}
pub struct StashControlReport {
    pub result: Result<()>,
    pub recovery: String,
    pub removed: bool,
    pub stashes: Result<Vec<StashEntry>>,
    pub status: Result<Status>,
}
#[derive(Clone, Debug)]
pub struct StashSelectionPreview {
    head: ObjectId,
    review: ChangeReview,
    selected: Vec<usize>,
    discard: PatchPreview,
    forward: Vec<u8>,
    message: String,
}
impl StashSelectionPreview {
    pub fn files(&self) -> &[FileEditPreview] {
        self.discard.files()
    }
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
pub struct StashSelectionReport {
    pub result: Result<()>,
    pub stash: Option<ObjectId>,
    pub recovery: String,
    pub cleanup: Option<PatchReport>,
    pub stashes: Result<Vec<StashEntry>>,
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
fn namespace() -> String {
    format!(
        "refs/rocket/stash/{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
impl Repository {
    pub fn preview_stash_control(
        &self,
        action: StashControl,
        c: &CancellationToken,
    ) -> Result<StashControlPreview> {
        self.runner.step("stash-control-preview", "review_stash_control", || {
            self.require_worktree(c)?; self.require_idle(c)?;
            let status = self.status(c)?;
            let entries = self.stashes(c)?;
            if entries.is_empty() { return Err(invalid("There are no stashes to remove.")); }
            if let StashControl::Pop(entry) = &action {
                if entry.repository != self.info().common_dir || entries.get(entry.index) != Some(entry) {
                    return Err(invalid("stash_control_stale: select the stash again."));
                }
                if !status.entries.is_empty() { return Err(invalid("Commit or stash local changes before popping a stash. Apply without dropping remains available.")); }
            }
            Ok(StashControlPreview {repository:self.info().common_dir.clone(), action, entries, head:status.head})
        })
    }
    pub fn apply_stash_control(
        &self,
        p: &StashControlPreview,
        c: &CancellationToken,
    ) -> Result<StashControlReport> {
        let _permit = self.acquire_write(c)?;
        let fresh = self.preview_stash_control(p.action.clone(), c)?;
        if p.repository != fresh.repository || p.entries != fresh.entries || p.head != fresh.head {
            return Err(invalid(
                "stash_control_stale: checkout or stash list changed; review again.",
            ));
        }
        let recovery = namespace();
        let mut removed = false;
        let result = self.runner.step("stash-control", "backup_and_apply_stash", || {
            let mut input = String::from("start\noption no-deref\n");
            for (i,e) in p.entries.iter().enumerate() { input.push_str(&format!("create {recovery}/{i} {}\n",e.id)); }
            if matches!(p.action,StashControl::Clear) {
                input.push_str(&format!("delete refs/stash {}\n",p.entries[0].id));
            } else { input.push_str(&format!("verify refs/stash {}\n",p.entries[0].id)); }
            input.push_str("prepare\ncommit\n");
            self.runner.write(self.cwd(), &args(&["update-ref","--no-deref","--stdin"]),input.into_bytes(),c)?.success("stash-backup")?;
            match &p.action {
                StashControl::Clear => { removed = true; }
                StashControl::Pop(entry) => {
                    let applied = self.apply_stash_locked(&entry.id,true,c)?;
                    applied.write.result?;
                    c.check()?;
                    if self.stashes(c)? != p.entries { return Err(invalid("Stash applied, but the stash list changed. The entry was retained; inspect it before dropping.")); }
                    self.drop_stash_locked(entry,c)?.write.result?;
                    removed = true;
                }
            }
            Ok(())
        });
        let observe = CancellationToken::new();
        Ok(StashControlReport {
            result,
            recovery,
            removed,
            stashes: self.stashes(&observe),
            status: self.status(&observe),
        })
    }
    pub fn preview_stash_selection(
        &self,
        review: &ChangeReview,
        selected: &[usize],
        message: &str,
        c: &CancellationToken,
    ) -> Result<StashSelectionPreview> {
        self.runner.step("stash-selection-preview","review_stash_selection",|| {
            self.require_idle(c)?;
            if message.trim().is_empty() || message.len()>4096 || message.chars().any(char::is_control) { return Err(invalid("Choose a nonblank stash message up to 4096 bytes without control characters.")); }
            let status=self.status(c)?;
            if status.entries.iter().any(|e| e.kind==EntryKind::Unmerged || e.submodule.is_some() || (&e.path==review.path() && e.staged())) {
                return Err(invalid("Resolve conflicts and unstage this file before saving its selected hunks. Other staged files are preserved."));
            }
            let head=match status.head { Head::Branch{target,..}|Head::Detached{target}=>target, _=>return Err(invalid("Create an initial commit before saving selected hunks.")) };
            let forward=review.selected_patch(selected)?;
            let discard=self.preview_discard_selection(review,selected,c)?;
            Ok(StashSelectionPreview{head,review:review.clone(),selected:selected.to_vec(),discard,forward,message:message.into()})
        })
    }
    pub fn stash_selection(
        &self,
        p: &StashSelectionPreview,
        c: &CancellationToken,
    ) -> Result<StashSelectionReport> {
        let _permit = self.acquire_write(c)?;
        let fresh = self.preview_stash_selection(&p.review, &p.selected, &p.message, c)?;
        if fresh.head != p.head {
            return Err(invalid(
                "stash_selection_stale: HEAD changed; review again.",
            ));
        }
        for file in p.discard.files() {
            self.recheck_file_edit(file, c)?;
        }
        let recovery = namespace();
        let mut stash = None;
        let mut cleanup = None;
        let result=self.runner.step("stash-selection","save_selected_hunks",||{
            let scratch=crate::patches::PreviewScratch::new()?;
            let index=scratch.path.join("selection-index");
            let create=(||{
                self.runner.private_index(self.cwd(),&index,&args(&["read-tree",p.head.as_str()]),vec![],c)?.success("stash-selection")?;
                self.runner.private_index(self.cwd(),&index,&args(&["apply","--cached","--unidiff-zero","--whitespace=nowarn","-"]),p.forward.clone(),c)?.success("stash-selection")?;
                let tree=self.runner.private_index(self.cwd(),&index,&args(&["write-tree"]),vec![],c)?.success("stash-selection")?;
                let tree=crate::parse::oid(crate::parse::line(&tree),"stash-selection")?;
                let base=self.runner.run(self.cwd(),&args(&["rev-parse",&format!("{}^{{tree}}",p.head)]),vec![],c)?.success("stash-selection")?;
                let base=crate::parse::oid(crate::parse::line(&base),"stash-selection")?;
                let index_commit=self.runner.write(self.cwd(),&args(&["commit-tree","--no-gpg-sign",base.as_str(),"-p",p.head.as_str()]),b"Rocket selected-hunk index\n".to_vec(),c)?.success("stash-selection")?;
                let index_commit=crate::parse::oid(crate::parse::line(&index_commit),"stash-selection")?;
                let object=self.runner.write(self.cwd(),&args(&["commit-tree","--no-gpg-sign",tree.as_str(),"-p",p.head.as_str(),"-p",index_commit.as_str()]),p.message.as_bytes().to_vec(),c)?.success("stash-selection")?;
                crate::parse::oid(crate::parse::line(&object),"stash-selection")
            })();
            let cleaned=scratch.cleanup(); let object=create?; cleaned?;
            for file in p.discard.files() { self.recheck_file_edit(file,c)?; }
            self.runner.write(self.cwd(),&args(&["update-ref","--no-deref",&format!("{recovery}/0"),object.as_str(),""]),vec![],c)?.success("stash-selection-backup")?;
            // Retain the object even if store is interrupted; only clean after confirmed storage.
            stash=Some(object.clone());
            self.runner.write(self.cwd(),&args(&["stash","store","--message",&p.message,object.as_str()]),vec![],c)?.success("stash-selection-store")?;
            c.check()?;
            let report=self.apply_patch_preview_locked(&p.discard,c)?;
            let ok=report.result.is_ok()&&report.verification.is_ok(); cleanup=Some(report);
            if !ok {return Err(invalid("Selected hunks were saved, but working-file cleanup stopped. Inspect the retained stash and recovery backups before retrying."));}
            Ok(())
        });
        Ok(StashSelectionReport {
            result,
            stash,
            recovery,
            cleanup,
            stashes: self.stashes(&CancellationToken::new()),
        })
    }
}
