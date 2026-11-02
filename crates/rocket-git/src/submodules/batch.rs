use super::*;

#[derive(Clone, Debug)]
pub struct SubmoduleBatchPreview {
    pub(crate) previews: Vec<SubmodulePreview>,
}
impl SubmoduleBatchPreview {
    pub fn entries(&self) -> impl Iterator<Item = &SubmoduleEntry> {
        self.previews.iter().map(|p| &p.entry)
    }
}
#[derive(Default)]
pub struct SubmoduleBatchReport {
    pub results: Vec<(RepoPath, Result<SubmoduleWriteReport>)>,
    pub skipped: Vec<RepoPath>,
}
impl Repository {
    pub fn preview_submodule_batch(
        &self,
        paths: &[RepoPath],
        source_root: Option<&Path>,
        c: &CancellationToken,
    ) -> Result<SubmoduleBatchPreview> {
        self.runner.step("submodule-batch-preview","review_submodule_batch",||{
            if paths.is_empty() || paths.len()>100 || paths.iter().enumerate().any(|(i,p)|paths[..i].contains(p)) {
                return Err(Error::InvalidInput("submodule_batch_selection: select 1–100 distinct submodules.".into()));
            }
            let all=self.submodules(c)?;let mut previews=Vec::new();
            for path in paths {
                let entry=all.iter().find(|e|&e.path==path).ok_or(Error::Submodule(SubmoduleFailure::StalePreview))?;
                if all.iter().any(|other|paths.contains(&other.path) && other.path!=entry.path && entry.directory.starts_with(&other.directory)) {
                    return Err(Error::InvalidInput("submodule_batch_overlap: select a parent or its nested child, then review descendants after updating the parent.".into()));
                }
                let p=self.preview_submodule(entry,c)?;
                if let Some(root)=source_root && (!p.local_source || !Path::new(&p.source).canonicalize().is_ok_and(|p|p.starts_with(root))) {
                    return fail(SubmoduleFailure::UnsupportedSource);
                }
                previews.push(p);
            }
            if self.submodules(c)?!=all {return fail(SubmoduleFailure::StalePreview);}
            Ok(SubmoduleBatchPreview {previews})
        })
    }
    pub fn apply_submodule_batch(
        &self,
        p: &SubmoduleBatchPreview,
        c: &CancellationToken,
    ) -> Result<SubmoduleBatchReport> {
        let _permit = self.acquire_write(c)?;
        self.runner
            .step("submodule-batch", "revalidate_submodule_batch", || {
                for old in &p.previews {
                    if self.preview_submodule(&old.entry, c)? != *old {
                        return fail(SubmoduleFailure::StalePreview);
                    }
                }
                Ok(())
            })?;
        let mut out = SubmoduleBatchReport::default();
        let mut stopped = false;
        for p in &p.previews {
            if stopped || c.is_cancelled() {
                out.skipped.push(p.entry.path.clone());
                continue;
            }
            let result = self.apply_submodule_locked(p, c);
            stopped = result
                .as_ref()
                .map_or(true, |r| r.result.is_err() || r.verification.is_err());
            out.results.push((p.entry.path.clone(), result));
        }
        Ok(out)
    }
}
