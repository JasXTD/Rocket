//! GitHub thread IDs are read from a bounded, repository-scoped GraphQL query.
//! No arbitrary query or server-supplied URL can be executed by this adapter.
use super::*;
const READ: &str = "hosting-thread-details";
const PREVIEW: &str = "hosting-thread-preview";
const SUBMIT: &str = "hosting-thread-submit";
#[derive(Clone, Debug)]
pub struct ForgeReviewThread {
    pub id: String,
    pub path: String,
    pub start: Option<u64>,
    pub line: Option<u64>,
    pub resolved: bool,
    pub outdated: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
    pub comments: Vec<ForgePullComment>,
    pub limited: bool,
}
#[derive(Clone, Debug)]
pub struct ForgePullThreads {
    repository: ForgeRepository,
    session: u64,
    summary: ForgePullSummary,
    pub threads: Vec<ForgeReviewThread>,
    pub limited: bool,
}
impl ForgePullThreads {
    pub fn repository(&self) -> &ForgeRepository {
        &self.repository
    }
    pub fn number(&self) -> u64 {
        self.summary.number
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgeThreadAction {
    Reply,
    Resolve,
    Unresolve,
}
impl ForgeThreadAction {
    pub fn key(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Resolve => "resolve",
            Self::Unresolve => "unresolve",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Reply => "Reply to thread",
            Self::Resolve => "Resolve thread",
            Self::Unresolve => "Reopen thread",
        }
    }
}
pub struct ForgeThreadPreview {
    snapshot: ForgePullThreads,
    thread: ForgeReviewThread,
    action: ForgeThreadAction,
    body: String,
    created: Instant,
    used: AtomicBool,
}
impl ForgeThreadPreview {
    pub fn repository(&self) -> &ForgeRepository {
        &self.snapshot.repository
    }
    pub fn number(&self) -> u64 {
        self.snapshot.summary.number
    }
    pub fn thread(&self) -> &ForgeReviewThread {
        &self.thread
    }
    pub fn action(&self) -> ForgeThreadAction {
        self.action
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn head(&self) -> &str {
        &self.snapshot.summary.head
    }
}
fn required<T>(v: Option<T>) -> Result<T> {
    v.ok_or(Error::Forge(ForgeFailure::InvalidResponse))
}
fn node_id(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 256
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'='))
}
fn query_failure(response: &Json) -> Option<ForgeFailure> {
    let errors = response.get("errors")?;
    let Some(rows) = errors.array() else {
        return Some(ForgeFailure::InvalidResponse);
    };
    let first = rows.first()?;
    let code = first
        .str("type")
        .or_else(|| first.get("extensions").and_then(|v| v.str("code")));
    Some(match code {
        Some("FORBIDDEN") => ForgeFailure::Forbidden,
        Some("NOT_FOUND") => ForgeFailure::NotFound,
        Some("RATE_LIMITED") => ForgeFailure::RateLimited,
        Some("GRAPHQL_VALIDATION_FAILED") => ForgeFailure::Unsupported,
        _ => ForgeFailure::InvalidResponse,
    })
}
fn parse_thread(j: &Json) -> Result<ForgeReviewThread> {
    let id = required(j.str("id").filter(|s| node_id(s)))?.to_owned();
    let comments = required(j.get("comments"))?;
    let rows = required(
        comments
            .get("nodes")
            .and_then(Json::array)
            .filter(|a| a.len() <= 20),
    )?;
    let mut out = Vec::new();
    let mut ids = HashSet::new();
    for row in rows {
        let id = required(row.u64("databaseId").filter(|n| *n > 0))?;
        if !ids.insert(id) {
            return fail(ForgeFailure::InvalidResponse);
        }
        out.push(ForgePullComment {
            id,
            author: row
                .get("author")
                .and_then(|a| string(a, "login", 256))
                .unwrap_or_else(|| "Deleted user".into()),
            body: body(row, 8192),
            date: required(string(row, "createdAt", 64))?,
            path: optional_string(row, "path", 8192),
            line: row.u64("line"),
            reply_to: row.get("replyTo").and_then(|r| r.u64("databaseId")),
        });
    }
    Ok(ForgeReviewThread {
        id,
        path: required(string(j, "path", 8192))?,
        start: j.u64("startLine"),
        line: j.u64("line"),
        resolved: required(j.bool("isResolved"))?,
        outdated: required(j.bool("isOutdated"))?,
        can_resolve: required(j.bool("viewerCanResolve"))?,
        can_unresolve: required(j.bool("viewerCanUnresolve"))?,
        comments: out,
        limited: required(comments.get("pageInfo").and_then(|p| p.bool("hasNextPage")))?,
    })
}
impl ForgeSession {
    pub fn pull_threads(
        &self,
        repo: &ForgeRepository,
        number: u64,
        c: &CancellationToken,
    ) -> Result<ForgePullThreads> {
        self.runner.step(READ,"read_review_threads",||{
            if !self.config.provider().is_github(){return fail(ForgeFailure::Unsupported);}
            if number>i32::MAX as u64{return Err(invalid("Pull request number exceeds GraphQL's integer limit."));}
            let path=self.pull_scope(repo,number)?;let first=self.pull_summary(READ,&path,number,c)?;
            let(owner,name)=repo.path().split_once('/').ok_or_else(||invalid("Select a valid repository."))?;
            const QUERY:&str="query RocketReviewThreads($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){nameWithOwner pullRequest(number:$number){number headRefOid baseRefOid reviewThreads(first:50){pageInfo{hasNextPage} nodes{id path startLine line isResolved isOutdated viewerCanResolve viewerCanUnresolve comments(first:20){pageInfo{hasNextPage} nodes{databaseId author{login} body createdAt path line replyTo{databaseId}}}}}}}}";
            let payload=format!("{{\"query\":{},\"variables\":{{\"owner\":{},\"name\":{},\"number\":{number}}}}}",quote(QUERY),quote(owner),quote(name));
            let response=self.request(READ,"read_review_threads","POST","/graphql",Some(&payload),false,c)?.0;
            if let Some(failure)=query_failure(&response){return fail(failure);}
            let repository=required(response.get("data").and_then(|d|d.get("repository")))?;
            if !repository.str("nameWithOwner").is_some_and(|p|p.eq_ignore_ascii_case(repo.path())){return fail(ForgeFailure::InvalidResponse);}
            let pr=required(repository.get("pullRequest"))?;
            if pr.u64("number")!=Some(number)||pr.str("headRefOid")!=Some(&first.head)||pr.str("baseRefOid")!=Some(&first.base){return fail(ForgeFailure::PullChanged);}
            let threads=required(pr.get("reviewThreads"))?;let rows=required(threads.get("nodes").and_then(Json::array).filter(|a|a.len()<=50))?;
            let mut items=Vec::new();let mut ids=HashSet::new();for row in rows {let t=parse_thread(row)?;if !ids.insert(t.id.clone()){return fail(ForgeFailure::InvalidResponse);}items.push(t);}
            let limited=required(threads.get("pageInfo").and_then(|p|p.bool("hasNextPage")))?;
            let last=self.pull_summary(READ,&path,number,c)?;if !same_revision(&first,&last){return fail(ForgeFailure::PullChanged);}
            Ok(ForgePullThreads{repository:repo.clone(),session:self.id,summary:last,threads:items,limited})
        })
    }
    pub fn preview_pull_thread(
        &self,
        snapshot: &ForgePullThreads,
        index: usize,
        action: ForgeThreadAction,
        text: &str,
        c: &CancellationToken,
    ) -> Result<ForgeThreadPreview> {
        self.runner.step(PREVIEW, "validate_thread_action", || {
            if snapshot.session != self.id {
                return fail(ForgeFailure::PullReviewStale);
            }
            if action == ForgeThreadAction::Reply
                && (text.trim().is_empty() || text.len() > 8192 || text.contains('\0'))
            {
                return Err(invalid("A reply needs text of at most 8 KiB without NUL."));
            }
            let selected = snapshot
                .threads
                .get(index)
                .ok_or_else(|| invalid("Select a loaded thread."))?;
            let current = self.pull_threads(&snapshot.repository, snapshot.summary.number, c)?;
            if !same_revision(&snapshot.summary, &current.summary) {
                return fail(ForgeFailure::PullChanged);
            }
            let thread = current
                .threads
                .iter()
                .find(|t| t.id == selected.id)
                .ok_or(Error::Forge(ForgeFailure::PullChanged))?;
            thread_matches(selected, thread)?;
            allowed(thread, action)?;
            Ok(ForgeThreadPreview {
                thread: thread.clone(),
                snapshot: current,
                action,
                body: if action == ForgeThreadAction::Reply {
                    text.into()
                } else {
                    String::new()
                },
                created: Instant::now(),
                used: AtomicBool::new(false),
            })
        })
    }
    pub fn submit_pull_thread(
        &self,
        preview: &ForgeThreadPreview,
        c: &CancellationToken,
    ) -> Result<()> {
        self.runner.step(SUBMIT,"submit_thread_action",||{
            if preview.snapshot.session!=self.id||preview.created.elapsed()>Duration::from_secs(120)||preview.used.load(Ordering::Acquire){return fail(ForgeFailure::PullReviewStale);}
            let current=self.pull_threads(&preview.snapshot.repository,preview.number(),c)?;
            if !same_revision(&preview.snapshot.summary,&current.summary){return fail(ForgeFailure::PullChanged);}
            let thread=current.threads.iter().find(|t|t.id==preview.thread.id).ok_or(Error::Forge(ForgeFailure::PullChanged))?;
            thread_matches(&preview.thread,thread)?;allowed(thread,preview.action)?;c.check()?;
            if preview.used.swap(true,Ordering::AcqRel){return fail(ForgeFailure::PullReviewStale);}
            if preview.action==ForgeThreadAction::Reply {
                let root=thread.comments.first().ok_or_else(||invalid("The original comment is unavailable."))?.id;
                let response=self.request(SUBMIT,"reply_to_review_thread","POST",&format!("/repos/{}/pulls/{}/comments/{root}/replies",preview.repository().encoded(),preview.number()),Some(&format!("{{\"body\":{}}}",quote(&preview.body))),true,c)?.0;
                let reply=comment(&response,true).filter(|r|r.reply_to==Some(root)&&r.author.eq_ignore_ascii_case(&self.login));
                if reply.is_none(){return fail(ForgeFailure::PullReviewUnknown);}Ok(())
            } else {
                let name=if preview.action==ForgeThreadAction::Resolve{"resolveReviewThread"}else{"unresolveReviewThread"};
                let query=format!("mutation RocketThread($id:ID!){{{name}(input:{{threadId:$id}}){{thread{{id isResolved}}}}}}");
                let response=self.request(SUBMIT,"change_review_thread_state","POST","/graphql",Some(&format!("{{\"query\":{},\"variables\":{{\"id\":{}}}}}",quote(&query),quote(&thread.id))),true,c)?.0;
                let result=response.get("data").and_then(|d|d.get(name)).and_then(|v|v.get("thread"));
                if response.get("errors").is_some()||result.is_none_or(|t|t.str("id")!=Some(&thread.id)||t.bool("isResolved")!=Some(preview.action==ForgeThreadAction::Resolve)){return fail(ForgeFailure::PullReviewUnknown);}Ok(())
            }
        }).map_err(|e|match e{Error::Forge(ForgeFailure::OutcomeUnknown)=>Error::Forge(ForgeFailure::PullReviewUnknown),Error::Forge(ForgeFailure::Rejected)=>Error::Forge(ForgeFailure::PullReviewRejected),other=>other})
    }
}
fn thread_matches(a: &ForgeReviewThread, b: &ForgeReviewThread) -> Result<()> {
    if a.id != b.id
        || a.path != b.path
        || a.start != b.start
        || a.line != b.line
        || a.outdated != b.outdated
        || a.resolved != b.resolved
        || a.comments.first().map(|c| c.id) != b.comments.first().map(|c| c.id)
    {
        return fail(ForgeFailure::PullChanged);
    }
    Ok(())
}
fn allowed(t: &ForgeReviewThread, action: ForgeThreadAction) -> Result<()> {
    if match action {
        ForgeThreadAction::Resolve => t.resolved || !t.can_resolve,
        ForgeThreadAction::Unresolve => !t.resolved || !t.can_unresolve,
        ForgeThreadAction::Reply => t.comments.first().is_none_or(|c| c.reply_to.is_some()),
    } {
        return fail(ForgeFailure::Forbidden);
    }
    Ok(())
}
