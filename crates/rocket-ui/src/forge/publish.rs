use super::*;
use std::path::PathBuf;

pub struct State {
    pub owner: String,
    pub name: String,
    pub description: String,
    pub branch: String,
    pub remote: String,
    pub private: bool,
    pub public_ack: bool,
    pub ssh: bool,
    pub upstream: bool,
    pub root: Option<PathBuf>,
    pub fixture_root: Option<PathBuf>,
    pub preview: Option<Arc<GitHubPublishPreview>>,
    pub report: Option<GitHubPublishReport>,
    pub progress: Arc<GitHubPublishProgress>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            owner: String::new(),
            name: String::new(),
            description: String::new(),
            branch: String::new(),
            remote: "origin".into(),
            private: true,
            public_ack: false,
            ssh: false,
            upstream: true,
            root: None,
            fixture_root: None,
            preview: None,
            report: None,
            progress: Arc::default(),
        }
    }
}
impl State {
    pub fn report_errors(&self) -> Vec<String> {
        let Some(r) = &self.report else { return vec![] };
        let mut errors = Vec::new();
        if let Some(e) = &r.error {
            errors.push(user_error(e));
        }
        if let Some(p) = &r.push {
            if let Err(e) = &p.result {
                errors.push(user_error(e));
            }
            if let Some(Err(e)) = &p.update {
                errors.push(format!("Push status: {}", user_error(e)));
            }
            if let Some(Err(e)) = &p.upstream {
                errors.push(format!("Upstream: {}", user_error(e)));
            }
            if let Err(e) = &p.branches {
                errors.push(format!("Branch refresh: {}", user_error(e)));
            }
            if let Some(Err(e)) = &p.status {
                errors.push(format!("Status refresh: {}", user_error(e)));
            }
            if p.error_diagnostics {
                errors.push("Git reported additional diagnostics. Check local tracking before retrying; the remote result is shown separately.".into());
            }
        }
        errors
    }
}
impl Rocket {
    pub(super) fn prepare_publish_form(&mut self) {
        if self.forge.running()
            || self.forge.publish.preview.is_some()
            || self.forge.publish.report.is_some()
        {
            return;
        }
        let root = self.tab().map(|t| t.path.clone());
        let branch = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|snapshot| snapshot.head.as_ref())
            .and_then(|head| match head {
                Head::Branch { name, .. } => String::from_utf8(name.clone()).ok(),
                _ => None,
            })
            .unwrap_or_default();
        let s = &mut self.forge;
        if s.publish.root != root {
            s.publish.root = root;
            s.publish.branch = branch.clone();
        }
        if s.publish.branch.is_empty() {
            s.publish.branch = branch;
        }
        if s.publish.owner.is_empty()
            && let Some(session) = &s.session
        {
            s.publish.owner = session.login().into();
        }
    }
    pub(super) fn start_github_publish(&mut self, action: &str) {
        if self.forge.running() || !self.can_write() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let s = &mut self.forge;
        s.error = None;
        s.notice = None;
        let task = (|| -> rocket_git::Result<WriteTask<Done>> {
            if !s.enabled || s.account_check_only {
                return Err(Error::InvalidInput(
                    "Publishing is disabled in this window.".into(),
                ));
            }
            let mut session = s
                .session
                .as_ref()
                .ok_or_else(|| Error::InvalidInput("Connect a GitHub account first.".into()))?
                .as_ref()
                .clone();
            if s.fixture.is_some() {
                let root = s.publish.fixture_root.as_ref().ok_or_else(|| {
                    Error::InvalidInput("Publishing requires a marked local Git fixture.".into())
                })?;
                session = session.with_publish_fixture_root(root)?;
            }
            let p = &mut s.publish;
            if p.report.is_some() {
                return Err(Error::InvalidInput(
                    "Inspect the last publish result before starting a new review.".into(),
                ));
            }
            let root = p.root.clone().ok_or_else(|| {
                Error::InvalidInput("Open the local repository to publish.".into())
            })?;
            if !p.private && !p.public_ack {
                return Err(Error::InvalidInput(
                    "Confirm that this branch's committed history may be public.".into(),
                ));
            }
            match action {
                "publish-review" => {
                    p.preview = None;
                    let draft = GitHubPublishDraft {
                        owner: p.owner.clone(),
                        name: p.name.clone(),
                        description: p.description.clone(),
                        private: p.private,
                        branch: p.branch.clone(),
                        remote: p.remote.clone(),
                        transport: if p.ssh {
                            GitHubPublishTransport::Ssh
                        } else {
                            GitHubPublishTransport::Https
                        },
                        set_upstream: p.upstream,
                    };
                    s.operation = "publish-review";
                    Ok(WriteTask::spawn(move |c| {
                        backend.publish_request(
                            "github-publish-preview",
                            &root,
                            &session,
                            |repo, session| {
                                repo.preview_github_publish(session, draft, &c)
                                    .map(Done::PublishReview)
                            },
                            &c,
                        )
                    })?)
                }
                "publish-create" => {
                    let preview = p.preview.clone().ok_or_else(|| {
                        Error::InvalidInput("Review the publication first.".into())
                    })?;
                    p.progress = Arc::default();
                    let progress = p.progress.clone();
                    s.operation = "publish";
                    Ok(WriteTask::spawn(move |c| {
                        backend.publish_request(
                            "github-publish",
                            &root,
                            &session,
                            |repo, session| {
                                repo.publish_github(session, &preview, &progress, &c)
                                    .map(|report| Done::Published(Box::new(report)))
                            },
                            &c,
                        )
                    })?)
                }
                _ => Err(Error::InvalidInput("Unknown publishing action.".into())),
            }
        })();
        match task {
            Ok(t) => s.task = Some(t),
            Err(e) => {
                s.error = Some(user_error(&e));
                s.operation = "";
            }
        }
    }
}
pub(super) fn section(
    ui: &mut Ui,
    s: &mut super::State,
    w: &mut HashMap<String, UiControl>,
    busy: bool,
    writable: bool,
    sources: &[String],
    out: &mut PageOut,
) {
    let p = &mut s.publish;
    ui.heading("Publish to GitHub");
    ui.label(RichText::new("Create a repository and publish one reviewed branch.").color(HINT()));
    if let Some(error) = &s.error {
        banner(ui, w, RED(), "Publish needs attention", error, None);
    }
    if let Some(notice) = &s.notice {
        notice_line(ui, notice);
    }
    if busy {
        let phase = if s.stopping {
            "Stopping… Checking which steps completed."
        } else if s.operation == "publish-review" {
            "Checking branch, history and destination…"
        } else {
            p.progress.phase()
        };
        notice_line(ui, phase);
    }
    if let Some(report) = &p.report {
        let creation = match report.creation {
            GitHubCreation::Created => "Created",
            GitHubCreation::Rejected => "Rejected",
            GitHubCreation::Unknown => "Unknown — inspect GitHub before retrying",
            GitHubCreation::NotAttempted => "Not attempted",
        };
        let push = report
            .push
            .as_ref()
            .map(|r| match r.remote_outcome {
                PushRemoteOutcome::Confirmed => "Confirmed",
                PushRemoteOutcome::Rejected => "Rejected — inspect the remote branch",
                PushRemoteOutcome::Unknown => "Unknown — check the remote before retrying",
                PushRemoteOutcome::NotAttempted => "Not attempted",
            })
            .unwrap_or("Not attempted");
        ui.group(|ui| {
            ui.label(format!("GitHub repository: {creation}"));
            ui.label(format!("Local remote: {}",if report.remote_added {"Added"}else{"Not added"}));
            ui.label(format!("Branch push: {push}"));
            ui.label("Staged changes and working files were not included in this push.");
            if report.remote_added {ui.label("The remote stays configured. Use the normal Push review if another push is needed.");}
            if report.creation!=GitHubCreation::Rejected {ui.label("A created repository is retained. Rocket never automatically deletes it or retries creation.");}
        });
        for error in p.report_errors() {
            banner(ui, w, RED(), "Publish detail", &error, None);
        }
        ui.horizontal_wrapped(|ui| {
            if btn(
                ui,
                w,
                "hosting-publish-open",
                "Inspect repository on GitHub",
                !busy,
            ) {
                out.open = Some(report.url.clone());
            }
            if btn(
                ui,
                w,
                "hosting-publish-reset",
                "I've checked the result · new review",
                !busy,
            ) {
                out.action = Some("publish-reset");
            }
        });
        if out.action == Some("publish-reset") {
            p.report = None;
            p.preview = None;
            p.public_ack = false;
            out.action = None;
        }
        return;
    }
    let edit = !busy && p.preview.is_none();
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(760.0));
        let width=ui.available_width();
        if let Some(session)=&s.session {ui.label(format!("Account: {} · github.com",session.login()));}
        if let Some(root)=&p.root {ui.add(Label::new(format!("Local repository: {}",crate::views::display_path(root))).wrap());}
        form_field(ui,w,("hosting-publish-owner","Owner","your login or an organization"),&mut p.owner,width,edit,false);
        form_field(ui,w,("hosting-publish-name","Repository name","new repository"),&mut p.name,width,edit,false);
        form_field(ui,w,("hosting-publish-description","Description","optional"),&mut p.description,width,edit,false);
        ui.horizontal_wrapped(|ui| {
            let r=ui.add_enabled(edit,Checkbox::new(&mut p.private,"Private repository"));
            control(w,"hosting-publish-private",&r,ui.clip_rect());
            if r.changed() {p.public_ack=false;}
            let r=ui.add_enabled(edit,Checkbox::new(&mut p.ssh,"Use system SSH (otherwise HTTPS)"));
            control(w,"hosting-publish-ssh",&r,ui.clip_rect());
        });
        label(ui,"Local branch", "only committed history is published");
        branch_field(ui,w,"hosting-publish-branch",&mut p.branch,sources,width,edit);
        form_field(ui,w,("hosting-publish-remote","New remote name","existing remotes are preserved"),&mut p.remote,width,edit,false);
        let r=ui.add_enabled(edit,Checkbox::new(&mut p.upstream,"Set upstream after a confirmed push"));
        control(w,"hosting-publish-upstream",&r,ui.clip_rect());
        if !p.private {
            banner(ui,w,AMBER(),"Public repository","Anyone can read all committed history on the selected branch.",None);
            let r=ui.add_enabled(edit,Checkbox::new(&mut p.public_ack,"I intend to publish this committed history publicly"));
            control(w,"hosting-publish-public-ack",&r,ui.clip_rect());
        }
        ui.label(RichText::new("Your staged, unstaged and untracked files stay local. No README, license or ignore rules are generated on GitHub. HTTPS uses your existing Git credentials; SSH uses system SSH. The hosting API token is never passed to Git.").color(HINT()));
        if let Some(review)=&p.preview {
            ui.group(|ui| {
                ui.strong(format!("Create {}/{} · {}",review.draft().owner,review.draft().name,if review.draft().private {"Private"}else{"Public"}));
                ui.label(format!("Push {} at {}",review.draft().branch,review.commit()));
                ui.label(format!("Add {} → {}",review.draft().remote,review.destination()));
                ui.label("1 · Create repository   2 · Add remote   3 · Push branch");
            });
        }
        ui.horizontal_wrapped(|ui| {
            if p.preview.is_some() {
                let r=pages::primary(ui,!busy&&writable,"Create repository and push");control(w,"hosting-publish-create",&r,ui.clip_rect());
                if r.clicked() {out.action=Some("publish-create");}
                if btn(ui,w,"hosting-publish-back","Back to edit",!busy) {p.preview=None;}
            } else {
                let r=pages::primary(ui,!busy&&writable&&s.session.is_some()&&(p.private||p.public_ack),"Review publish");control(w,"hosting-publish-review",&r,ui.clip_rect());
                if r.clicked() {out.action=Some("publish-review");}
            }
        });
    });
}
