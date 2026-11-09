//! CLI hosting uses the same bounded adapters and single-use previews as the UI.
//! Credentials are explicit process inputs, never Git credentials/settings or argv.
use crate::output::{Outcome, Problem};
use rocket_git::*;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Options {
    pub config: ForgeConfig,
    pub token_env: String,
    pub username: String,
    pub repository: Option<String>,
    pub action: Action,
}
#[derive(Debug)]
pub enum WriteMode {
    Preview,
    Submit(String),
}
#[derive(Debug)]
pub enum Action {
    Account,
    List {
        filter: ForgeFilter,
        pages: u32,
    },
    Pull(u64),
    Create {
        source: String,
        target: String,
        title: String,
        body: String,
        source_repository: Option<String>,
        mode: WriteMode,
    },
    Review {
        number: u64,
        action: ForgeReviewAction,
        body: String,
        comments: Option<PathBuf>,
        mode: WriteMode,
    },
    Merge {
        number: u64,
        method: ForgeMergeMethod,
        mode: WriteMode,
    },
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Account => "hosting-account",
            Self::List { .. } => "hosting-list",
            Self::Pull(_) => "hosting-pull-details",
            Self::Create {
                mode: WriteMode::Preview,
                ..
            } => "hosting-create-preview",
            Self::Create { .. } => "hosting-create",
            Self::Review {
                mode: WriteMode::Preview,
                ..
            } => "hosting-review-preview",
            Self::Review { .. } => "hosting-review-submit",
            Self::Merge {
                mode: WriteMode::Preview,
                ..
            } => "hosting-merge-preview",
            Self::Merge { .. } => "hosting-merge-submit",
        }
    }
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}

/// Called only from single-threaded main, before crash/log/cancellation workers
/// or helper discovery. The parent environment is untouched. No token reaches
/// Git/curl child environments, JSON output, diagnostics or saved credentials.
pub fn take_credential(options: &crate::args::Options) -> Result<Option<String>> {
    let crate::args::Command::Hosting(h) = &options.command else {
        return Ok(None);
    };
    let raw = std::env::var_os(&h.token_env);
    // SAFETY: main calls this before creating any threads or starting helpers.
    unsafe {
        std::env::remove_var(&h.token_env);
    }
    let token=raw.and_then(|v|v.into_string().ok()).ok_or_else(||invalid("The named token environment variable is missing or is not Unicode. Set it in the invoking process; never pass a token in command arguments."))?;
    if token.is_empty()
        || token.len() > 4096
        || !token.is_ascii()
        || token.bytes().any(|b| b <= 32 || b == 127)
    {
        return Err(invalid(
            "The token environment variable is empty or has an unsupported format.",
        ));
    }
    Ok(Some(token))
}

pub fn execute(
    client: &GitClient,
    local: &Path,
    h: &Options,
    token: &str,
    c: &CancellationToken,
    log: Option<&DiagnosticLog>,
) -> Result<Outcome> {
    if h.config.fixture_port().is_some() {
        // An explicit numeric-loopback fixture is available only with a public,
        // synthetic credential and a disposable-repository marker. Never route
        // a user's real credential to a mock API by configuration accident.
        let marker = local.join(".git/rocket-ui-fixture");
        if token != "rocket-disposable-fixture"
            || !std::fs::symlink_metadata(marker)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            return Err(invalid(
                "The fixture API requires a marked temporary repository and the synthetic rocket-disposable-fixture token.",
            ));
        }
    }
    let s = client.forge_session(h.config.clone(), &h.username, token, c)?;
    let account = json!({"provider":h.config.provider().key(),"server":h.config.server(),"login":s.login(),"identity":s.account_identity(),"credentials_saved":false});
    if matches!(h.action, Action::Account) {
        return Ok(Outcome::success(account));
    }
    let repository = ForgeRepository::new(
        h.config.clone(),
        h.repository
            .as_deref()
            .ok_or_else(|| invalid("Choose a hosting repository."))?,
    )?;
    let mut out = match &h.action {
        Action::Account => unreachable!(),
        Action::List { filter, pages } => {
            let (mut items, mut cursor, mut total, mut limited, mut count) =
                (Vec::new(), ForgeCursor::default(), None, false, 0);
            let mut failure = None;
            for _ in 0..*pages {
                match s.list(&repository, filter, cursor, c) {
                    Ok(page) => {
                        count += 1;
                        items.extend(page.items.iter().map(item));
                        total = page.total;
                        limited |= page.incomplete;
                        if let Some(next) = page.next {
                            cursor = next;
                            if count == *pages {
                                limited = true;
                            }
                        } else {
                            break;
                        }
                    }
                    Err(e) => {
                        failure = Some(e);
                        limited = true;
                        break;
                    }
                }
            }
            let mut out = Outcome::success(
                json!({"items":items,"pages_loaded":count,"total":total,"limited":limited,"repository":repository.path()}),
            );
            if let Some(e) = failure {
                out.error = Some(Problem::backend(&e));
            }
            out
        }
        Action::Pull(n) => Outcome::success(details(&s.pull_details(&repository, *n, c)?)),
        Action::Create {
            source,
            target,
            title,
            body,
            source_repository,
            mode,
        } => {
            let draft = ForgeDraft {
                repository: repository.clone(),
                source: source.clone(),
                target: target.clone(),
                title: title.clone(),
                body: body.clone(),
            };
            let p = if let Some(path) = source_repository {
                s.preview_from_fork(draft, ForgeRepository::new(h.config.clone(), path)?, c)?
            } else {
                s.preview(draft, c)?
            };
            let pin = p.review_digest(&s)?;
            let data = json!({"repository":repository.path(),"source_repository":p.source_repository().path(),"source":source,"target":target,"title":title,"body":body,"source_id":p.source_id(),"target_id":p.target_id(),"review_sha256":pin});
            write(mode, &pin, data, log, || s.create(&p, c).map(|i| item(&i)))
        }
        Action::Review {
            number,
            action,
            body,
            comments,
            mode,
        } => {
            let d = s.pull_details(&repository, *number, c)?;
            let drafts = comments
                .as_deref()
                .map(read_comments)
                .transpose()?
                .unwrap_or_default();
            let comments = drafts
                .iter()
                .map(|draft| d.restore_inline_comment(draft))
                .collect::<Result<Vec<_>>>()?;
            let p = s.preview_pull_review_with_comments(&d, *action, body, &comments, c)?;
            let pin = p.review_digest(&s)?;
            let data = json!({"repository":repository.path(),"pull":summary(p.summary()),"action":action.event(),"body":body,"comments":comments.iter().map(|c|draft(&c.draft())).collect::<Vec<_>>(),"review_sha256":pin});
            write(mode, &pin, data, log, || {
                s.submit_pull_review(&p, c).map(|r| review(&r))
            })
        }
        Action::Merge {
            number,
            method,
            mode,
        } => {
            let d = s.pull_details(&repository, *number, c)?;
            let p = s.preview_pull_merge(&d, *method, c)?;
            let pin = p.review_digest(&s)?;
            let data = json!({"repository":repository.path(),"pull":summary(p.summary()),"method":method.key(),"condition":d.merge_condition(),"review_sha256":pin});
            write(mode, &pin, data, log, || {
                s.merge_pull(&p, c).map(|r| json!({"commit":r.commit}))
            })
        }
    };
    out.data["account"] = account;
    Ok(out)
}
fn write(
    mode: &WriteMode,
    pin: &str,
    preview: Value,
    log: Option<&DiagnosticLog>,
    submit: impl FnOnce() -> Result<Value>,
) -> Outcome {
    let mut out = Outcome::success(
        json!({"preview":preview,"remote_outcome":"not_attempted","automatic_retry":false}),
    );
    let WriteMode::Submit(expected) = mode else {
        return out;
    };
    let verify = || {
        if expected == pin {
            Ok(())
        } else {
            Err(invalid(
                "workflow_review_stale: the reviewed hosting account, revisions, locations or text changed. Run --preview and review the new request before submitting; nothing was sent.",
            ))
        }
    };
    let checked = match log {
        Some(log) => log.request("hosting-review-preview", |_| verify()),
        None => verify(),
    };
    if let Err(error) = checked {
        out.error = Some(Problem::backend(&error));
        return out;
    }
    match submit() {
        Ok(value) => {
            out.data["remote_outcome"] = json!("confirmed");
            out.data["result"] = value;
        }
        Err(e) => {
            let unknown = matches!(
                e,
                Error::Forge(
                    ForgeFailure::OutcomeUnknown
                        | ForgeFailure::PullReviewUnknown
                        | ForgeFailure::PullMergeUnknown
                )
            );
            out.data["remote_outcome"] = json!(if unknown { "unknown" } else { "not_confirmed" });
            out.error = Some(Problem::backend(&e));
        }
    }
    out
}
fn item(i: &ForgeItem) -> Value {
    json!({"number":i.number,"title":i.title,"state":i.state,"author":i.author,"source":i.source,"target":i.target,"url":i.url})
}
fn summary(s: &ForgePullSummary) -> Value {
    json!({"number":s.number,"title":s.title,"body":s.body,"author":s.author,"state":s.state,"draft":s.draft,"merged":s.merged,"head":s.head,"base":s.base,"source":s.source,"target":s.target,"mergeable":s.mergeable,"mergeable_state":s.mergeable_state})
}
fn review(r: &ForgePullReview) -> Value {
    json!({"id":r.id,"author":r.author,"body":r.body,"state":r.state,"commit":r.commit,"date":r.date})
}
fn collection<T>(c: &ForgePullCollection<T>, f: impl Fn(&T) -> Value) -> Value {
    json!({"items":c.items.iter().map(f).collect::<Vec<_>>(),"limited":c.limited,"error":c.error.map(|e|json!({"code":e.code(),"guidance":e.guidance()}))})
}
fn details(d: &ForgePullDetails) -> Value {
    let note = |n: &ForgePullComment| json!({"id":n.id,"author":n.author,"body":n.body,"date":n.date,"path":n.path,"line":n.line,"reply_to":n.reply_to});
    json!({"repository":d.repository().path(),"url":d.url(),"pull":summary(d.summary()),
        "capabilities":{"inline":d.supports_inline_review(),"inline_ranges":d.supports_inline_ranges(),"inline_limit":d.inline_comment_limit(),"review_actions":([ForgeReviewAction::Comment,ForgeReviewAction::Approve,ForgeReviewAction::RequestChanges].iter().filter(|a|d.supports_review(**a)).map(|a|a.event()).collect::<Vec<_>>())},
        "commits":collection(&d.commits,|v|json!({"id":v.id,"subject":v.subject,"author":v.author})),
        "files":collection(&d.files,|v|json!({"path":v.path,"previous":v.previous,"status":v.status,"additions":v.additions,"deletions":v.deletions,"patch":v.patch,"lines":v.lines.iter().map(|l|json!({"old":l.old,"new":l.new,"text":l.text})).collect::<Vec<_>>()})),
        "discussion":collection(&d.discussion,note),"inline_comments":collection(&d.inline_comments,note),"reviews":collection(&d.reviews,review),
        "checks":collection(&d.checks,|v|json!({"name":v.name,"status":v.status,"conclusion":v.conclusion})),
        "statuses":collection(&d.statuses,|v|json!({"name":v.name,"status":v.status,"conclusion":v.conclusion})),
        "inline_context":{"head":d.summary().head,"base":d.summary().base,"diff_revision":d.inline_diff_revision()}})
}
fn draft(d: &ForgeInlineDraft) -> Value {
    json!({"path":d.path,"start":d.start,"line":d.line,"side":d.side.key(),"head":d.head,"base":d.base,"body":d.body,"diff_revision":d.diff_revision})
}
fn read_comments(path: &Path) -> Result<Vec<ForgeInlineDraft>> {
    let bad = || {
        invalid(
            "The comments file must be a JSON array of at most 32 comments and 128 KiB, with exact path/start/line/side/head/base/body and optional diff_revision fields from hosting pull.",
        )
    };
    let file = std::fs::File::open(path).map_err(|_| bad())?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= 128 * 1024)
    {
        return Err(bad());
    }
    let mut bytes = Vec::new();
    file.take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    if bytes.len() > 128 * 1024 {
        return Err(bad());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let rows = value.as_array().filter(|r| r.len() <= 32).ok_or_else(bad)?;
    rows.iter()
        .map(|v| {
            let obj = v.as_object().ok_or_else(bad)?;
            if obj.keys().any(|k| {
                !matches!(
                    k.as_str(),
                    "path" | "start" | "line" | "side" | "head" | "base" | "body" | "diff_revision"
                )
            }) {
                return Err(bad());
            }
            let text = |k: &str| v[k].as_str().map(str::to_owned).ok_or_else(bad);
            Ok(ForgeInlineDraft {
                path: text("path")?,
                start: v["start"].as_u64().ok_or_else(bad)?,
                line: v["line"].as_u64().ok_or_else(bad)?,
                side: match text("side")?.as_str() {
                    "LEFT" => ForgeDiffSide::Left,
                    "RIGHT" => ForgeDiffSide::Right,
                    _ => return Err(bad()),
                },
                head: text("head")?,
                base: text("base")?,
                body: text("body")?,
                diff_revision: match v.get("diff_revision") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => return Err(bad()),
                },
            })
        })
        .collect()
}
