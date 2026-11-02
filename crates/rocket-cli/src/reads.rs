//! Read-only views of signatures, the reflog and file history. Nothing here takes
//! repository write ownership. Pages are the first bounded window: the backend's
//! continuation cursors stay in-process, so JSON reports whether more exists.
use crate::{
    args::Command,
    output::{self as out, Outcome, Problem},
    run::repo_path,
};
use rocket_git::*;
use serde_json::json;

pub fn execute(
    command: &Command,
    repo: &Repository,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    match command {
        Command::Verify { tag, name, require_valid } => {
            let target = if *tag {
                SignatureTarget::Tag(repo.find_tag(name.as_bytes(), cancel)?.ok_or_else(|| {
                    Error::InvalidInput("The exact tag name does not exist. Run tag list or check the name.".into())
                })?)
            } else {
                SignatureTarget::Commit(repo.resolve_commit(name, cancel)?)
            };
            let v = repo.verify_signature(&target, cancel)?;
            let mut o = Outcome::success(json!({"id":v.id.as_str(),"kind":v.kind,"status":v.status.code(),"format":v.format,
                "signer":v.signer,"fingerprint":v.fingerprint,"trust":v.trust,"guidance":v.status.guidance()}));
            if *require_valid && v.status != SignatureStatus::Valid {
                o.error = Some(Problem::new("signature_not_valid", "The signature is not valid under the configured trust policy.", v.status.guidance(), 1));
            }
            Ok(o)
        }
        Command::SigningShow => {
            let s = repo.signing(cancel)?;
            let options = |o: &SigningOptions| json!({"format":o.format,"key":o.key,"sign_commits":o.sign_commits,"allowed_signers":o.allowed_signers});
            Ok(Outcome::success(json!({"local":options(&s.local),"effective":options(&s.effective),"format":s.format(),"program":s.program()})))
        }
        Command::Reflog { scope, limit } => {
            let page = repo.reflog_scoped(*scope, *limit, cancel)?;
            Ok(Outcome::success(json!({"scope":match scope {ReflogScope::Head=>"head",ReflogScope::AllRefs=>"all",
                    ReflogScope::DeletedBranches=>"deleted-branches",ReflogScope::Retained=>"retained",ReflogScope::Unreachable=>"unreachable"},
                "entries":page.entries.iter().map(|e|json!({"id":e.id.as_str(),"selector":out::bytes(&e.selector),"subject":out::bytes(&e.subject),
                    "commit_subject":out::bytes(&e.commit_subject),"actor":out::bytes(&e.actor),"reference":e.reference.as_deref().map(out::bytes)})).collect::<Vec<_>>(),
                "has_more":page.more})))
        }
        Command::Blame { revision, path, first, size } => {
            let id = repo.resolve_commit(revision, cancel)?;
            let path = repo_path(path)?;
            let page = repo.file_blame(&id, &path, *first, *size, cancel)?;
            Ok(Outcome::success(json!({"commit":id.as_str(),"path":out::bytes(path.as_bytes()),"total_lines":page.total_lines,
                "next_line":page.next_line,"lines":page.lines.iter().map(|l|json!({"line":l.line,"original_line":l.original_line,
                    "commit":l.commit.as_str(),"path":out::bytes(l.path.as_bytes()),"author":out::bytes(&l.author),
                    "summary":out::bytes(&l.summary),"content":out::bytes(&l.content),"boundary":l.boundary})).collect::<Vec<_>>()})))
        }
        Command::FileHistory { path, revision, limit } => {
            let tip = repo.resolve_commit(revision.as_deref().unwrap_or("HEAD"), cancel)?;
            let page = repo.file_history(FileHistoryStart::File { tip, path: repo_path(path)? }, *limit, cancel)?;
            Ok(Outcome::success(json!({"tip":page.tip.as_str(),"path":out::bytes(page.path.as_bytes()),
                "entries":page.entries.iter().map(|e|json!({"commit":out::commit(&e.commit),"path":out::bytes(e.path.as_bytes()),
                    "previous_path":e.previous_path.as_ref().map(|p|out::bytes(p.as_bytes())),"deleted":e.deleted})).collect::<Vec<_>>(),
                "has_more":page.next.is_some(),"limited":page.limited})))
        }
        Command::Search { query, kind, all, revision } => {
            let start = match (all, revision) {
                (true, _) => HistoryStart::AllBranches,
                (false, Some(revision)) => HistoryStart::Revision(revision.clone()),
                (false, None) => HistoryStart::Head,
            };
            let page = repo.search_history(HistorySearchStart::Query { start, query: query.clone(), kind: *kind }, cancel)?;
            // A next cursor means the bounded window ended before the history did.
            Ok(Outcome::success(json!({"kind":match kind {HistorySearchKind::Text=>"text",HistorySearchKind::Path=>"path",HistorySearchKind::Content=>"content"},
                "commits":page.commits.iter().map(out::commit).collect::<Vec<_>>(),"scanned":page.scanned,"complete":page.next.is_none()})))
        }
        Command::Compare { base, target, staged } => {
            let c = match target {
                Some(target) => repo.compare_revisions(base, target, cancel)?,
                None => repo.compare_local(base, *staged, cancel)?,
            };
            Ok(Outcome::success(json!({"base":c.base.as_str(),"target":c.local_staged.is_none().then(||c.target.as_str()),
                "local":c.local_staged.map(|staged|if staged {"staged"} else {"worktree"}),
                "files":c.files.iter().map(|f|json!({"path":out::bytes(f.path.as_bytes()),"original_path":f.original_path.as_ref().map(|p|out::bytes(p.as_bytes())),
                    "change":out::change(f.change),"similarity":f.similarity})).collect::<Vec<_>>()})))
        }
        _ => unreachable!("only read commands are dispatched here"),
    }
}
