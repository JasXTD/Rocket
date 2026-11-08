use crate::{
    capture, diagnostics,
    input::{click_events, key_event, prepare_fixture_input, replacement_events, slider_target},
    policy::{GITHUB_TEST_SOURCES, Policy},
    protocol::{Request, Shared, Ticket},
    request_timing::Phase,
    validation::{Job, Kind as ValidationKind, Spec as ValidationSpec},
};
use eframe::{App, Frame, egui::*};
use rocket_ui::app::{DialogKind, Intent, Rocket};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::Ordering,
        mpsc::{Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

struct Input {
    request: Request,
    events: Vec<Event>,
    keys_after_focus: Vec<Event>,
    /// Further pointer frames of a drag or a slider click, delivered one frame each
    /// after `events`.
    later: std::collections::VecDeque<Vec<Event>>,
    /// Files or folders dropped on the window with the first delivered frame.
    files: Vec<PathBuf>,
    text: Option<String>,
    delivered: bool,
    frame: u64,
    validation: Option<PendingValidation>,
    validated: bool,
    refusal: Option<&'static str>,
}
impl Input {
    /// A slider click whose press went out: only its pointer release remains.
    fn holds_pointer(&self) -> bool {
        self.request.name == "rocket_click" && self.events.is_empty() && !self.later.is_empty()
    }
}
/// A fixture file dropped through MCP. Rocket reads only its path.
#[derive(Debug)]
struct FixtureFile(PathBuf);
impl DroppedFile for FixtureFile {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
struct PendingValidation {
    job: Job,
    binding: Value,
    ready_at: Option<Instant>,
}
struct Capture {
    ticket: Ticket,
    id: u64,
}
struct ReviewInput { request: Request, window:u64, phase:u8, point:Pos2 }
pub struct Ui {
    pub app: Rocket,
    policy: Policy,
    shared: Arc<Shared>,
    queue: Arc<Mutex<Receiver<Request>>>,
    opening: Option<Ticket>,
    deferred: Option<Request>,
    input: Option<Input>,
    capture: Option<Capture>,
    review_input: Option<ReviewInput>,
    // A review has no Close button: `review-close` asks its window to close as the frame's × does.
    review_close: Option<(u64,Request)>,
    review_capture: Option<(u64,Capture)>,
    waits: Vec<(Ticket, Instant)>,
    frame: Option<u64>,
    next_capture: u64,
    viewport: [f32; 2],
}
impl Ui {
    fn pump_requests(&mut self, ctx:&Context) {
        // Native eframe keeps logic ticking when the root is occluded, but
        // skips its UI pass. State and child-window requests must not wait for
        // a root paint. Root inputs still wait for freshly rendered controls.
        // Windows only reports a minimized window as occluded, so restoring is
        // enough. Never take the foreground: fixture input needs no OS focus, and
        // the user's typing would land in the fixture.
        if self.deferred.is_none() {
            let queued = self.queue.lock().unwrap().try_recv();
            match queued {
                Ok(request) if matches!(request.name.as_str(),
                    "rocket_state" | "rocket_review_click" | "rocket_review_resize" | "rocket_review_capture") => self.request(request, ctx),
                Ok(request) => {
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Minimized(false));
                    self.deferred = Some(request);
                }
                Err(TryRecvError::Disconnected) => self.shared.shutdown(),
                Err(TryRecvError::Empty) => {}
            }
        }
    }
    fn poll_review_requests(&mut self, ctx:&Context) {
        if let Some((id,request))=self.review_close.take() {
            let open=self.app.reviews.iter().find(|w|{let r=w.lock().unwrap_or_else(|e|e.into_inner());r.id==id&&!r.closed}).cloned();
            match open {
                None => {request.ticket.timing.mark(Phase::Delivered);request.ticket.ok(json!({"input_applied":true,"review_closed":true}));}
                Some(_) if request.ticket.inactive()||Instant::now()>request.ticket.deadline => request.ticket.fail("Review window did not close"),
                Some(window) => {ctx.request_repaint_of(window.lock().unwrap_or_else(|e|e.into_inner()).viewport());self.review_close=Some((id,request));}
            }
        }
        if let Some(pending)=self.review_input.take() {
            let current=self.app.reviews.iter().find(|w|w.lock().unwrap_or_else(|e|e.into_inner()).id==pending.window);
            let closed=current.is_none_or(|w|w.lock().unwrap_or_else(|e|e.into_inner()).closed);
            if closed && pending.phase>=2 && pending.request.args["target"]=="review-close" {
                pending.request.ticket.ok(json!({"input_applied":true,"review_closed":true}));
            } else if !closed && pending.phase==2 {
                pending.request.ticket.ok(json!({"input_applied":true,"state":self.state()}));
            } else if closed || (pending.phase != 1 && (pending.request.ticket.inactive() || Instant::now()>pending.request.ticket.deadline)) {
                pending.request.ticket.fail("Review input expired or its window closed");
            } else {
                ctx.request_repaint_of(current.unwrap().lock().unwrap_or_else(|e|e.into_inner()).viewport());
                self.review_input=Some(pending);
            }
        }
        if let Some((id,capture))=self.review_capture.take() {
            let window=self.app.reviews.iter().find(|w|w.lock().unwrap_or_else(|e|e.into_inner()).id==id).cloned();
            let Some(window)=window else{capture.ticket.fail("Review closed before capture");return;};
            let mut r=window.lock().unwrap_or_else(|e|e.into_inner());
            if r.closed || self.policy.repository(&r.tab.path).is_err() {capture.ticket.fail("Review capture outside active fixture scope");return;}
            let image=r.screenshots.drain(..).find_map(|event|match event {
                Event::Screenshot{image,user_data,..} if user_data.data.as_ref().and_then(|d|d.downcast_ref::<u64>())==Some(&capture.id)=>Some(image),_=>None,
            });
            if let Some(image)=image {
                match capture::png(&image) {Ok(bytes)=>capture.ticket.result(json!({"content":[{"type":"image","mimeType":"image/png","data":capture::base64(&bytes)}],"isError":false})),Err(e)=>capture.ticket.fail(&e)}
            } else if capture.ticket.inactive() || Instant::now()>capture.ticket.deadline {capture.ticket.fail("Review screenshot expired");}
            else {ctx.request_repaint_of(r.viewport());self.review_capture=Some((id,capture));}
        }
    }
    pub fn new(
        ctx: &Context,
        policy: Policy,
        shared: Arc<Shared>,
        queue: Arc<Mutex<Receiver<Request>>>,
        open: Request,
        logs: PathBuf,
        scope: rocket_git::TransportScope,
    ) -> Self {
        let path = PathBuf::from(open.args["path"].as_str().unwrap());
        *shared.context.lock().unwrap() = Some(ctx.clone());
        shared.window.store(2, Ordering::Release);
        let storage = path
            .parent()
            .expect("fixture parent")
            .join("rocket-ui-state");
        let mut app = Rocket::with_storage_scoped(ctx, vec![path.clone()], logs, storage.clone(), Some(scope));
        *shared.diagnostics.lock().unwrap() = app.backend.as_ref().map(|b| b.log.clone());
        open.ticket.timing.mark(Phase::Ui);
        app.terminal_history_file = Some(storage.join("terminal-history.log"));
        app.background_reviews = true;
        app.secret_protection_editable = false;
        app.terminal_shell.editable = false;
        app.dependencies.editable = false;
        app.submodule_fixture_root = Some(policy.root.clone());
        // Open and Browse list only fixture folders.
        app.browse_root = Some(policy.root.clone());
        app.lfs_source_scope = Some(rocket_git::LfsSourceScope {
            root: policy.root.clone(),
            network_sources: GITHUB_TEST_SOURCES.iter().map(|s| (*s).into()).collect(),
        });
        app.lfs_lock_source_scope = Some(rocket_git::LfsSourceScope {
            root: policy.root.clone(),
            network_sources: policy.lfs_lock_source().into_iter().collect(),
        });
        // GitHub links are recorded instead of opening a browser.
        app.github_endpoint = policy.github_endpoint();
        // A runner may lower the preview limit to test where previews stop.
        if let Some(limit) = policy.preview_limit() {
            app.preview_limit = limit;
        }
        if let Some(port)=policy.update_fixture(){
            app.updates.endpoint=Some(rocket_git::app_update::UpdateEndpoint::Loopback(port));
            app.updates.fixture_install=Some(policy.root.join("installation"));
        }
        app.forge.fixture = policy.hosting_fixture();
        app.integrations.fixture = policy.hosting_fixture();
        app.integrations.isolated = true;
        if app.forge.fixture.is_some() { app.forge.publish.fixture_root = Some(policy.root.join("publish-remotes")); }
        app.forge.account_check_only = policy.hosting_account_check();
        app.forge.publish_check = policy.github_publish_check().map(|(repository,_)| repository);
        app.forge.enabled = app.forge.fixture.is_some() || app.forge.account_check_only || app.forge.publish_check.is_some();
        app.ssh.enabled = policy.ssh_fixture().is_some();
        if let Some(root) = policy.ssh_fixture() {
            app.ssh.scan_directory = root.parent().expect("SSH fixture has a parent").to_path_buf();
            app.ssh.root = root;
        }
        app.auto_fetch.test_interval = policy.auto_fetch_fixture();
        app.auto_fetch.blocked = app.auto_fetch.test_interval.is_none();
        app.opened_urls = Some(Vec::new());
        Self {
            app,
            policy,
            shared,
            queue,
            opening: Some(open.ticket),
            deferred: None,
            input: None,
            capture: None,
            review_input: None,
            review_close: None,
            review_capture: None,
            waits: vec![],
            frame: None,
            next_capture: 0,
            viewport: [0.0, 0.0],
        }
    }
    fn scope_ok(&self) -> bool {
        self.app
            .tabs
            .iter()
            .all(|tab| self.tab_in_scope(tab))
            && self.app.reviews.iter().all(|w|self.policy.repository(&w.lock().unwrap_or_else(|e|e.into_inner()).tab.path).is_ok())
    }
    /// A marked fixture, or a preview in the test root's preview folder: previews
    /// refuse every write, and their source is checked before each download.
    fn tab_in_scope(&self, tab: &rocket_ui::app::Tab) -> bool {
        if tab.is_preview() {
            return self.policy.preview_repository(&tab.path).is_ok();
        }
        self.policy.repository(&tab.path).is_ok()
    }
    /// The repository a click downloads from, when `target` opens a preview.
    fn preview_source(&self, target: &str) -> Option<Result<String, String>> {
        let explore = &self.app.explore;
        let text = |source: &rocket_git::CloneSource| match source {
            rocket_git::CloneSource::Url(url) => url.clone(),
            rocket_git::CloneSource::Local(path) => path.to_string_lossy().into_owned(),
        };
        Some(match target {
            "explore-preview" | "explore-preview-anyway" => explore
                .selected
                .and_then(|i| explore.results.get(i))
                .map(|r| r.clone_url.clone())
                .ok_or_else(|| "Select a result first".to_owned()),
            "explore-address-preview" => rocket_ui::explore::address(&explore.query)
                .map(|a| text(&a.source))
                .ok_or_else(|| "Type a repository address first".to_owned()),
            "github-repository-preview" => self
                .app
                .dialog
                .as_ref()
                .and_then(|d| d.github.as_ref())
                .and_then(|s| s.selected.and_then(|i| s.repositories.get(i)))
                .map(|r| r.clone_url.clone())
                .ok_or_else(|| "Select a repository first".to_owned()),
            "preview-retry" | "preview-unlimited" => self
                .app
                .tab()
                .and_then(|t| t.preview.as_ref())
                .map(|p| text(&p.request.source))
                .ok_or_else(|| "Open a preview first".to_owned()),
            _ => return None,
        })
    }
    /// Keep downloads from the preview's source, names it as origin and makes a new
    /// folder: the source must be a local fixture and the folder inside the root.
    fn keep_scope(&self) -> Result<(), String> {
        let Some(dialog) = self.app.dialog.as_ref() else {
            return Ok(());
        };
        let DialogKind::KeepPreview(id) = dialog.kind else {
            return Ok(());
        };
        let preview = self
            .app
            .tabs
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.preview.as_ref())
            .ok_or("The preview's tab is closed")?;
        if let rocket_git::CloneSource::Local(path) = &preview.request.source {
            self.local_remote_url(path.to_string_lossy().as_bytes())?;
        } else {
            return Err("Keep under MCP needs a local fixture source".into());
        }
        self.local_remote_url(preview.request.url.as_bytes())?;
        let destination = Path::new(dialog.second.trim());
        let inside = destination
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|p| p.starts_with(&self.policy.root));
        if !destination.is_absolute() || !inside || destination.exists() {
            return Err("Keep makes a new folder inside the test root".into());
        }
        Ok(())
    }
    fn state(&self) -> Value {

        let base = json!({"window":"running","window_count":1,"pid":std::process::id(),"busy":self.app.busy() || self.input.as_ref().is_some_and(|p|p.validation.is_some()),"input_pending":self.input.is_some(),"quitting":self.app.quitting});
        if !self.scope_ok() {
            let mut base = base;
            base["scope_error"] = json!(
                "This test window contains a repository outside the configured fixture scope"
            );
            return base;
        }
        let mut state = base;
        state["integrations"] = self.app.integrations.snapshot();
        if self.policy.private_state_allowed() {
            state["integrations"]["playground"]["text"] = self.app.integrations.playground.private_snapshot();
        }
        state["input_validation"] = self
            .input
            .as_ref()
            .and_then(|p| p.validation.as_ref())
            .map_or(Value::Null, |v| {
                json!({"operation":v.job.spec.kind.operation(),"step":v.job.step(),
                "elapsed_ms":v.job.started.elapsed().as_millis(),"cancelling":v.job.cancelling()})
            });
        state["app_version"] = json!(env!("CARGO_PKG_VERSION"));
        state["guided_setup"] = json!({"open":self.app.setup.open,"loading":self.app.setup.running(),"identity_ready":self.app.setup.identity.as_ref().is_some_and(|i|i.effective.name.is_some()&&i.effective.email.is_some()),"error":self.app.setup.error});
        let manager_open = self.app.manager_open_paths();
        state["repository_manager"] = json!({"visible":self.app.page()==Some(rocket_ui::pages::Page::RepositoryManager),
            "running":self.app.manager.running(),"opening":self.app.manager.opening(),"remaining":self.app.manager.remaining(),
            "selected":self.app.manager.selected,"selection":format!("{:?}",self.app.manager.selection),
            "include_nested":self.app.manager.include_nested,"collapsed":self.app.manager.collapsed,
            "open_count":manager_open.as_ref().ok().map(Vec::len),"open_error":manager_open.as_ref().err(),
            "error":self.app.manager.error,"notice":self.app.manager.notice,
            "scan":self.app.manager.scan.as_ref().map(scan_state_value),
            "folders":self.app.library.folders.iter().map(|f|json!({"id":f.id,"name":f.name})).collect::<Vec<_>>(),
            "repositories":self.app.library.repositories.iter().map(|r|json!({"id":r.id,"name":r.name,"path":r.path,"folder":r.folder})).collect::<Vec<_>>(),
            "workspaces":self.app.library.workspaces.iter().map(|w|json!({"id":w.id,"name":w.name,"repositories":w.repositories,"active":w.active,"parent":w.parent})).collect::<Vec<_>>(),
            "form":self.app.manager.form.as_ref().map(|f|json!({"kind":format!("{:?}",f.kind),"name":f.name,"path":f.path,"members":f.members,"active":f.active,"parent":f.parent,"error":f.error}))});
        state["repository_manager"]["status"] = self.app.manager.status.mcp_state(&self.app.library);
        state["mcp"] = json!({"visible":self.app.preferences_page() == Some(rocket_ui::preferences::Page::Mcp),"busy":self.app.mcp.busy(),"port":self.app.mcp.port,"root":self.app.mcp.root,"running":self.app.mcp.status.running,"hosted":self.app.mcp.hosted,"error":self.app.mcp.error,"notice":self.app.mcp.notice});
        state["viewport"] = json!(self.viewport);
        state["updates"]=json!({"enabled":self.app.updates.endpoint.is_some(),"open":self.app.updates.open,"startup_checked":self.app.updates.started,"running":self.app.updates.running(),"status":self.app.updates.status,"phase":self.app.updates.phase(),"notes_expanded":self.app.updates.notes_expanded(),"details_expanded":self.app.updates.details_expanded(),"notice_visible":self.app.frame_widgets.contains_key("update-notice-open"),"message":self.app.updates.message,"error":self.app.updates.error,"ready":self.app.updates.ready(),"installable":self.app.updates.installable(),"pending_restart":self.app.updates.pending_restart(),"release":self.app.updates.release.as_ref().map(|r|json!({"version":r.version.text(),"size":r.size,"page":r.page}))});
        state["review_windows"] = json!(self.app.reviews.iter().map(|w| {
            let r=w.lock().unwrap_or_else(|e|e.into_inner());
            let controls:Vec<_>=r.widgets.iter().map(|(id,c)|json!({"id":id,"enabled":c.enabled,"rect":[c.rect.min.x,c.rect.min.y,c.rect.max.x,c.rect.max.y]})).collect();
            json!({"id":r.id,"commit":r.request.commit.as_str(),"path":r.tab.path.to_string_lossy(),
                "file":r.request.path.as_ref().map(|p|p.display_lossy()),"closed":r.closed,"loading":r.running(),"viewport":r.size,
                "error":r.error,"notice":r.notice,"file_history":file_history_state(&r.tab.file_history),
                "patch_files":r.tab.patch.as_ref().map(|p|p.files.iter().map(|f|&f.label).collect::<Vec<_>>()),"controls":controls})
        }).collect::<Vec<_>>());
        state["window_count"] = json!(1+self.app.reviews.len());
        state["review_input_pending"] = json!(self.review_input.is_some() || self.review_close.is_some());
        state["commit_link"] = self.app.commit_link.as_ref().map_or(Value::Null,|s|json!({"commit":s.commit.as_str(),"loading":s.running(),
            "selected":s.selected,"links":s.links.iter().map(|l|json!({"remote":String::from_utf8_lossy(&l.remote),"url":l.url,"provider":l.provider.key()})).collect::<Vec<_>>(),"error":s.error,"notice":s.notice}));
        let tabs: Vec<_> = self.app.tabs.iter().map(|tab| {
            let snapshot = tab.snapshot.as_ref();
            let status = snapshot.and_then(|s| s.status.as_ref());
            let head = snapshot.and_then(|s| s.head.as_ref()).map(|h| match h {
                rocket_git::Head::Branch { name, target } => json!({"branch":String::from_utf8_lossy(name),"oid":target.as_str()}),
                _ => json!({"detached":true}),
            });
            let mut value=json!({"id":tab.id,"path":tab.path.to_string_lossy(),"view":format!("{:?}",tab.view),"head":head,"loading":tab.loading(),"error":tab.error,
                "warnings":snapshot.map(|s| &s.warnings),"change_count":status.map(|s| s.entries.len()),"draft_present":!tab.draft.is_empty(),
                "upstream":status.and_then(|s| s.upstream.as_ref()).map(|s| String::from_utf8_lossy(s)),"commit_count":tab.commits().len(),
                "selected_commit":tab.selected_commit.as_ref().map(|id|id.as_str()),"background_refresh":tab.background_refresh && tab.refresh.is_some(),
                "history_scope":if tab.history_all { "all" } else { tab.history_ref.as_deref().unwrap_or("HEAD") },
                "worktree_view": {"branch_filter":tab.branch_filter, "hide_tags":tab.worktree_view.hide_tags,
                    "hide_stashes":tab.worktree_view.hide_stashes, "sections":tab.worktree_view.sections,
                    "split_diff":tab.split_diff, "fold_all":tab.folding.all},
                "history_ids":tab.commits().iter().take(2000).map(|c|c.id.as_str()).collect::<Vec<_>>(),
                "history_more":snapshot.and_then(|s|s.history.as_ref()).is_some_and(|h|h.next.is_some()),
                "file_history":file_history_state(&tab.file_history),
                "repository_tree":repository_tree_state(&tab.tree),
                "repository_map":repository_map_state(&tab.map),
                "conflict_editor":{"running":tab.conflict.running(),"path":tab.conflict.path.as_ref().map(|p|p.display_lossy()),"dirty":tab.conflict.dirty,"bound":tab.conflict.preview.is_some(),"blocks":tab.conflict.blocks.blocks.len(),"malformed":tab.conflict.blocks.malformed,"selected":tab.conflict.selected,"part":tab.conflict.part,"draft":tab.conflict.draft.chars().take(16000).collect::<String>(),"draft_bytes":tab.conflict.draft.len(),"error":tab.conflict.error,"notice":tab.conflict.notice},
                "stashes":snapshot.map(|s|s.stashes.iter().map(|e|json!({"index":e.index,"id":e.id.as_str(),"subject":String::from_utf8_lossy(&e.subject)})).collect::<Vec<_>>()),
                "stash_preview":{"running":tab.stash.running(),"selected":tab.stash.selected.as_ref().map(|e|e.id.as_str()),"part":format!("{:?}",tab.stash.part),"error":tab.stash.error,"notice":tab.stash.notice,"limited":tab.stash.patch.as_ref().is_some_and(|p|p.limited),"files":tab.stash.patch.as_ref().map(|p|p.files.iter().map(|f|json!({"path":f.label,"added":f.added,"removed":f.removed})).collect::<Vec<_>>())},
                "file_search":file_search_state(&tab.tree.search),
                "search":{"query":tab.search,"kind":tab.search_kind.label(),"regex":tab.search_regex,"match_case":tab.search_case,"path":tab.search_path,"stopped":tab.search_state.stopped,"bad_pattern":tab.search_state.bad_pattern,"diff_find":tab.diff_tools.query,"scanning":tab.search_state.scanning(),"running":tab.search_state.task.is_some(),"complete":tab.search_state.complete,"paused":tab.search_state.paused,"scanned":tab.search_state.scanned,"error":tab.search_state.error,"more":tab.search_state.next.is_some()},
                "remotes":snapshot.map(|s|s.remotes.iter().map(|r|json!({"name":String::from_utf8_lossy(&r.name),"fetch_urls":r.urls.len(),"push_urls":r.push_urls.len()})).collect::<Vec<_>>()),
                "refresh_count":tab.refresh_count,"watching":snapshot.is_some_and(|s|s.watcher.is_some()),"patch_error":tab.patch_error,
                "patch_added":tab.patch.as_ref().map(|p|p.files.iter().map(|f|f.added).sum::<usize>()),
                "diff_tools": {"query":tab.diff_tools.query,"syntax":tab.diff_tools.syntax,"searching":tab.diff_tools.running(),"matches":tab.diff_tools.hits.len(),"limited":tab.diff_tools.limited,"current":tab.diff_tools.current,"error":tab.diff_tools.error},
                "split_diff":tab.split_diff,"conflict_side":tab.conflict_side,
                "operation_state":snapshot.and_then(|s|s.state.as_ref()).map(operation_state),
                "selected_lines":tab.patch.as_ref().map(|p|p.selected.len()),
                "partial_selection":tab.patch.as_ref().and_then(|p|p.review.as_ref()).is_some_and(|r|r.selection_unavailable().is_none()),
                "patch_removed":tab.patch.as_ref().map(|p|p.files.iter().map(|f|f.removed).sum::<usize>())});
            value["preview"] = tab.preview.as_ref().map_or(Value::Null, |p| preview_state(p, &self.app));
            value["history_order"]=json!(tab.history_order.key());
            value["undo_available"] = json!(tab.undo_commit.is_some());
            value["summary"] = status.map_or(Value::Null, |status| {
                let s = rocket_ui::guided::summarize(status, snapshot.and_then(|s|s.state.as_ref()));
                json!({"staged":s.staged,"unstaged":s.unstaged,"untracked":s.untracked,"conflicts":s.conflicts,"remote":s.remote,"next":s.next,
                    "refreshing":rocket_ui::guided::refreshing(tab)})
            });
            value["word_diff"]=json!({"enabled":tab.diff_tools.words,"running":tab.diff_tools.words_running(),"lines":tab.diff_tools.word_highlights.spans.len(),"spans":tab.diff_tools.word_highlights.spans.values().map(|s|s.len()).sum::<usize>(),"limited":tab.diff_tools.word_highlights.limited,"error":tab.diff_tools.word_error});
            value["image_diff"]=json!({"running":tab.image.running(),"loaded":tab.image.loaded,"cancelled":tab.image.cancelled,"error":tab.image.error,"fit":tab.image.fit,"zoom":tab.image.zoom,"checker":tab.image.checker,"before":tab.image.before.as_ref().map(|i|json!({"width":i.width,"height":i.height,"bytes":i.encoded_bytes,"format":i.format,"alpha":i.alpha})),"after":tab.image.after.as_ref().map(|i|json!({"width":i.width,"height":i.height,"bytes":i.encoded_bytes,"format":i.format,"alpha":i.alpha}))});
            value["patch_files"]=json!(tab.patch.as_ref().map(|p|p.files.iter().take(500).map(|f|json!({"path":f.label,"image":f.image.is_some(),"syntax_language":f.syntax_language,"syntax_limited":f.syntax_limited,"syntax_spans":f.syntax_spans})).collect::<Vec<_>>()));
            value["image_diff"]["mode"]=json!(tab.image.mode.code());
            value["image_diff"]["mix"]=json!(tab.image.mix);
            value["image_diff"]["difference_error"]=json!(tab.image.difference_error);
            value["image_diff"]["pixels"]=json!(tab.image.stats.as_ref().map(|s|json!({"changed":s.changed,"compared":s.compared,"bounds":s.bounds})));
            value["patch_file"]=json!(tab.patch_file);
            value["diff_tools"]["overview"]=overview_state(&tab.diff_tools.overview);
            value["submodule_diff"] = json!(tab.patch.as_ref().and_then(|p|p.files.get(tab.patch_file)).and_then(|f|f.submodule.as_ref()).map(|s|json!({"path":s.path.display_lossy(),"before":s.before.as_ref().map(|id|id.as_str()),"after":s.after.as_ref().map(|id|id.as_str()),"limited":s.limited,"notice":s.notice,"commits":s.commits.iter().map(|c|json!({"id":c.id.as_str(),"added":c.added})).collect::<Vec<_>>()})));
            let compare = &tab.compare;
            value["compare"] = json!({"base":compare.base,"target":compare.target,"local_staged":compare.local_staged,"filter":compare.filter,
                "running":compare.running(),"listing":compare.listing(),"content_running":compare.content_running(),
                "cancelling":compare.cancelling,"selected":compare.selected,"error":compare.error,"notice":compare.notice,
                "changed_inputs":compare.changed_inputs(),"show_files":compare.show_files,
                "loaded":compare.loaded.as_ref().map(|r|json!({"base_input":r.base_input,"target_input":r.target_input,
                    "base":r.comparison.base.as_str(),"target":r.comparison.target.as_str(),"local_staged":r.comparison.local_staged,"count":r.comparison.files.len(),
                    "files":r.comparison.files.iter().take(1000).enumerate().map(|(i,f)|json!({"index":i,
                        "path":f.path.display_lossy(),"original_path":f.original_path.as_ref().map(|p|p.display_lossy()),
                        "change":rocket_ui::compare::change_label(f.change),"similarity":f.similarity})).collect::<Vec<_>>() }))});
            // Commit message helpers: counts and flags only, never message text.
            value["composer"]=json!({"loading":tab.composer.running(),"pending":tab.composer.needs_read || tab.composer.prefill_pending,
                "template_error":tab.composer.template_error,"recent":tab.composer.recent.len(),
                "template":tab.composer.template.as_ref().map(|t|json!({"message_bytes":t.message.len(),"guidance_lines":t.guidance.lines().count()}))});
            let draft = &tab.composer.draft;
            value["composer"]["draft"]=json!({"available":draft.available,"running":draft.running(),"requested":draft.requested,"provider":draft.provider.code(),
                "failure":draft.failure,"error":draft.error.is_some(),"notice":draft.notice,"restorable":draft.previous.is_some(),
                "scope":draft.scope.as_ref().map(|s|json!({"files":s.files,"binary_files":s.binary_files,"omitted_files":s.omitted_files,"diff_bytes":s.diff_bytes,"sent_bytes":s.sent_bytes}))});
            value["selected_commits"]=json!(tab.selected_commits.iter().map(|id|id.as_str()).collect::<Vec<_>>());
            // Collapsed merges shown in the history list, with how many loaded commits each hides.
            value["collapsed_merges"]=json!(tab.behind.iter().map(|(id,n)|json!({"id":id.as_str(),"hidden":n})).collect::<Vec<_>>());
            value["all_merges_collapsed"]=json!(tab.folding.all);
            value["issue_links"]=json!({"rules":tab.issue_links.links.len(),"loading":tab.issue_links.running()});
            value["sequence"]=json!(snapshot.and_then(|s|s.sequence.as_ref()).map(|q|json!({"operation":q.operation.label(),"commits":q.commits.iter().map(|id|id.as_str()).collect::<Vec<_>>(),"remaining":q.remaining.as_ref().map(|r|r.iter().map(|id|id.as_str()).collect::<Vec<_>>())})));
            value["branches"]=json!(snapshot.map(|s|s.branches.iter().take(1000).enumerate().map(|(index,b)|json!({
                "index":index,"reference":String::from_utf8_lossy(&b.reference),"target":b.target.as_str(),"current":b.is_head
            })).collect::<Vec<_>>()));
            value["conflict_editor"]["whole"]=conflict_whole_state(&tab.conflict);
            value["conflict_editor"]["ai"]=tab.conflict.ai.snapshot();
            value["conflict_editor"]["kept"]=json!(tab.conflict.kept.map(|k|k.label()));
            value["conflict_editor"]["linked"]=json!(!tab.conflict.unlinked);
            value["conflict_editor"]["three_columns"]=json!(tab.conflict.three_columns);
            value["conflict_editor"]["scroll_y"]=json!(tab.conflict.scroll_y);
            value["conflict_editor"]["scroll_max"]=json!(tab.conflict.scroll_max);
            value["conflict_editor"]["overview"]=overview_state(&tab.conflict.overview_probe);
            value["branch_review"]=tab.branch_review.snapshot();
            value
        }).collect();
        let mut controls:Vec<_> = self.app.frame_widgets.iter().map(|(id, widget)| json!({"id":id,"enabled":widget.enabled,"rect":[widget.rect.min.x,widget.rect.min.y,widget.rect.max.x,widget.rect.max.y]})).collect();
        controls.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        state["tabs"] = json!(tabs);
        state["active_tab"] = json!(self.app.tab().map(|tab| tab.id));
        state["controls"] = json!(controls);
        let repository_visible =
            !self.app.activity_history.open
                && self.app.page() != Some(rocket_ui::pages::Page::RepositoryManager);
        // A collapsed drawer shows only its Terminal/Activity strip, so neither pane
        // counts as visible; the toolbar buttons open it again.
        let drawer_open = repository_visible && !self.app.drawer_collapsed;
        state["console_visible"] = json!(self.app.show_console && drawer_open);
        state["drawer_collapsed"] = json!(repository_visible && self.app.drawer_collapsed
            && (self.app.show_console || self.app.show_terminal));
        state["terminal"] = self.app.tab().and_then(|tab|tab.terminal.as_ref()).map_or(Value::Null, |terminal| {
            // Hidden panes skip render snapshots, but explicit state queries still
            // need current output. Snapshot is nonblocking and shares unchanged rows.
            let fresh = terminal.session.snapshot(terminal.screen.revision);
            let screen = fresh.as_ref().unwrap_or(&terminal.screen);
            let text=screen.text();
            let mut start=text.len().saturating_sub(8192);
            while !text.is_char_boundary(start) {start+=1;}
            let (grid, grid_truncated) = terminal_grid(screen);
            json!({"visible":self.app.show_terminal && drawer_open,"shell":terminal.name,"running":terminal.status.running,"closed":terminal.status.closed,
                "pid":terminal.status.pid,"exit_code":terminal.status.exit_code,"error":terminal.status.error.as_ref().or(terminal.error.as_ref()),
                "columns":screen.columns,"rows":screen.rows,"scrollback_rows":screen.scrollback.len(),"tail":&text[start..],"screen_focused":self.app.terminal_screen_focused(),
                "cursor":screen.cursor,"grid":grid,"grid_truncated":grid_truncated})
        });
        state["git_settings"] = json!({"active_executable":self.app.backend.as_ref().map(|b|&b.executable),"loaded":self.app.git_settings.loaded,"busy":self.app.git_settings.running(),"stopping":self.app.git_settings.stopping,"dirty":self.app.git_settings.dirty(),"automatic":self.app.git_settings.automatic,"path":self.app.git_settings.path,"checked":self.app.git_settings.checked.as_ref().map(|(_,p,v)|json!({"executable":p,"version":v})),"identity_loaded":self.app.git_settings.identity.is_some(),"identity_path":self.app.git_settings.identity.as_ref().map(|i|&i.path),"error":self.app.git_settings.error,"notice":self.app.git_settings.notice});
        state["git_settings"]["managed"] = json!(self.app.git_settings.managed);
        state["git_settings"]["bundled"] = json!({"version":rocket_git::MANAGED_GIT_VERSION,
            "manifest":rocket_git::MANAGED_GIT_MANIFEST_SHA256,"available":self.app.git_settings.managed_available,
            "verified":self.app.git_settings.managed_verified,"progress":self.app.git_settings.managed_progress.snapshot()});
        state["preferences"] = json!({"page":self.app.preferences_page().map(|p|p.key()),"sidebar":self.app.show_sidebar,"details":self.app.show_details,"row_height":self.app.row_height,"auto_refresh":self.app.auto_refresh,"secret_protection":self.app.secret_protection,"guard_ai_api_keys":self.app.guard_ai_api_keys,"secret_protection_user_control":self.app.secret_protection_editable});
        state["shortcuts"] = self.app.keybindings.snapshot();
        state["preferences"]["secret_scan_mib"] = json!(self.app.secret_scan_mib);
        state["appearance"] = self.app.appearance_snapshot();
        state["language"] = json!({"shown":rocket_ui::i18n::current().tag(),"choice":self.app.language.map(|l|l.tag()),
            "system":rocket_ui::i18n::system().tag(),"offered":rocket_ui::i18n::offered().iter().map(|l|l.tag()).collect::<Vec<_>>()});
        state["dependencies"] = self.app.dependencies.summary(self.app.tab().map(|t| t.id));
        state["state_pending"] = json!(self.app.state_pending());
        let drafting = &self.app.drafting;
        state["drafting"] = json!({"enabled":drafting.enabled,"provider":drafting.provider.code(),"notice_accepted":drafting.notice_accepted,
            "codex_notice_accepted":drafting.codex_notice_accepted,"style":format!("{:?}",drafting.style),
            "executable":drafting.executable,"codex_executable":drafting.codex_executable,"language":drafting.language});
        state["state_error"] = json!(self.app.state_error);
        state["benchmark"] = self.app.benchmark.snapshot();
        state["activity_history"] = self
            .app
            .activity_history
            .state(self.app.backend.as_ref().map_or("", |b| b.log.session_id()));
        state["crash_reports"] = self.app.crash_reports.snapshot();
        let tools = &self.app.external;
        state["external_tools"] = json!({"open":tools.open,"kind":tools.kind.label(),"owner":tools.owner,"running":tools.running(),"pending":tools.pending(),"cancelling":tools.cancelling,"label":tools.label,"error":tools.error,"notice":tools.notice,"imported":tools.imported,"executable":tools.config().executable,"arguments":tools.config().arguments,"workspace":tools.report.as_ref().and_then(|r|r.workspace.as_ref()).map(|p|p.display().to_string()),"success":tools.report.as_ref().map(|r|r.result.is_ok()),"result":tools.report.as_ref().and_then(|r|r.text.as_ref()).map(|s|s.chars().take(16000).collect::<String>())});
        state["external_tools"]["profiles"] = json!(tools.editing.0[tools.kind.index()].entries.iter().map(|p| &p.name).collect::<Vec<_>>());
        state["external_tools"]["selected_profile"] = json!(tools.selected[tools.kind.index()]);
        state["external_tools"]["default_profile"] = json!(tools.editing.0[tools.kind.index()].default);
        state["external_tools"]["settings_dirty"] = json!(tools.editing != tools.profiles);
        let form = &tools.editing.0[tools.kind.index()].entries[tools.selected[tools.kind.index()]];
        state["external_tools"]["form"] = json!({"name":form.name,"executable":form.config.executable,"arguments":form.config.arguments});
        state["external_tools"]["removing_profile"] = json!(tools.removing);
        let shell = &self.app.terminal_shell;
        state["terminal_settings"] = json!({"choice":shell.saved.choice.key(),"editable":shell.editable,"dirty":shell.dirty()});
        state["startup_error"] = json!(self.app.startup_error);
        state["notice"] = json!(self.app.notice);
        let general = &self.app.general;
        state["general"] = json!({"source_folder":general.source_folder.as_ref().map(|p|p.display().to_string()),
            "fetch_tags":match general.fetch_tags {rocket_git::FetchTags::None=>"none",rocket_git::FetchTags::RemoteDefault=>"default",rocket_git::FetchTags::All=>"all"},
            "branch_space":match general.branch_space {rocket_ui::general::BranchSpace::Keep=>"none",rocket_ui::general::BranchSpace::Hyphen=>"hyphen",rocket_ui::general::BranchSpace::Underscore=>"underscore"},
            "submodules_on_checkout":general.submodules_on_checkout,"push_after_commit":general.push_after_commit,
            "diff_marks":general.diff_marks,"diff_font":general.diff_font,"diff_syntax":general.diff_syntax,"compact_labels":general.compact_labels});
        state["auto_fetch"] = json!({"minutes":self.app.auto_fetch.minutes,"blocked":self.app.auto_fetch.blocked,
            "tabs":self.app.auto_fetch.tabs.iter().map(|(id,t)|json!({"tab":id,"running":t.running(),"due_ms":t.due_in().map(|d|d.as_millis()),
                "finished":t.last.is_some(),"failures":t.last.as_ref().map_or(0,|(_,f)|f.len())})).collect::<Vec<_>>()});
        state["commands"] = json!(self.app.commands.iter().map(|c| json!({"name":c.name,"scope":c.scope.key(),"action":c.action.key(),"group":c.group,"inputs":c.inputs.len(),"arguments":c.argument_lines(),"test_command":test_command(&self.app, c)})).collect::<Vec<_>>());
        state["change_selection"] = json!(self.app.tab().map(|tab| json!({
            "count":tab.change_selection.paths.len(),"staged":tab.change_selection.staged,
            "notice":tab.change_selection.notice,
        })));
        // A dropped branch or commit waiting for its menu choice.
        state["drop"] = self.app.pending_drop.as_ref().map_or(Value::Null, |d| match &d.kind {
            rocket_ui::drag::DropKind::Branch { source, target } => json!({"kind":"branch","tab":d.tab,"source":String::from_utf8_lossy(source),"target":String::from_utf8_lossy(target)}),
            rocket_ui::drag::DropKind::Commit { source, target } => json!({"kind":"commit","tab":d.tab,"source":source.as_str(),"target":target.as_str()}),
        });
        state["github"] = github_state(&self.app);
        state["explore"] = explore_state(&self.app);
        state["hosting"] = hosting_state(&self.app);
        state["ci"] = ci_state(&self.app);
        state["actions"] = actions_state(&self.app);
        state["ssh"] = ssh_state(&self.app);
        state["dialog"] = self.app.dialog.as_ref().map_or(Value::Null, |d| {
            let supported = dialog_name(&d.kind) != "other"
                && !matches!(d.kind, DialogKind::AddRemote | DialogKind::EditRemote(..) | DialogKind::RepositorySettings);
            let mut value = json!({"kind":dialog_name(&d.kind),"owner":d.owner,
                "first":supported.then_some(&d.first),"second":supported.then_some(&d.second),
                "signing":d.signing.as_ref().map(signing_state_value),
                "pull":d.pull.as_ref().map(pull_state_value),
                "force_push":d.force_push.as_ref().map(force_push_state_value),
                "flag":d.flag,"error":(supported || matches!(d.kind, DialogKind::RepositorySettings)).then_some(&d.error),"identity_loaded":d.identity.is_some(),"identity_loading":d.identity_task.is_some(),
                "commit_preview_loaded":d.commit.as_ref().is_some_and(|c|c.preview.is_some()),"commit_preview_loading":d.commit.as_ref().is_some_and(|c|c.task.is_some()),
                "commit_applying":d.commit.as_ref().is_some_and(|c|c.applying),
                "merge":d.merge.as_ref().map(|m|json!({
                    "loading":m.task.is_some(),"applying":m.applying,"revision":d.first,"mode":format!("{:?}",d.mode),
                    "preview":m.preview.as_ref().map(|p|json!({
                        "target":p.target().id.as_str(),"revision":p.revision(),"head":format!("{:?}",p.head()),
                        "relation":format!("{:?}",p.relation()),"incoming":p.incoming(),
                        "current_only":p.current_only(),"can_apply":p.can_apply()
                    }))
                })),
                "bisect":d.bisect.as_ref().map(|s|json!({
                    "loading":s.task.is_some(),"loaded":s.loaded,"applying":s.applying,"notice":s.notice,
                    "automatic":bisect_auto_state(&s.automatic),
                    "preview":s.preview.as_ref().map(|p|json!({"action":p.action().label(),"good":p.good().as_str(),"bad":p.bad().as_str(),"candidates":p.candidates()})),
                    "session":s.session.as_ref().map(|s|json!({
                        "phase":format!("{:?}",s.phase),"original":format!("{:?}",s.original),
                        "recovery_reference":s.recovery_reference,
                        "target":s.target.as_ref().map(|id|id.as_str()),"can_mark":s.can_mark(),
                        "candidates":s.candidates.iter().take(100).map(|id|id.as_str()).collect::<Vec<_>>(),"candidate_count":s.candidates.len(),
                        "decisions":s.decisions.iter().map(|d|json!({"action":d.action.label(),"commit":d.commit.as_str()})).collect::<Vec<_>>()
                    }))
                })),
                "patch":d.patch.as_ref().map(|s|json!({"mode":s.mode.label(),"loading":s.running(),"applying":s.applying,"path":s.path,"base":s.base,"revision":s.revision,"source":s.source,"notice":s.notice,"preview_loaded":s.preview.is_some(),"paths":s.preview.as_ref().map(|p|p.paths()),"selected_file":s.file,"series":s.preview.as_ref().and_then(|p|if let rocket_ui::patches::Preview::Series(p)=p{Some(json!({"base":p.base().as_str(),"tip":p.tip().as_str(),"bytes":p.total_bytes(),"entries":p.entries().iter().map(|e|json!({"id":e.id.as_str(),"filename":e.filename,"bytes":e.patch().len()})).collect::<Vec<_>>()}))}else{None})})),
                "rebase_preview_loaded":d.rebase.as_ref().is_some_and(|r|r.preview.is_some()),
                "file_edit":d.file_edit.as_ref().map(|s|json!({"loading":s.running(),"applying":s.applying,"path":s.path.as_ref().map(|p|p.display_lossy()),"source":s.source.as_ref().map(|p|p.label()),"revision":s.revision,"notice":s.notice,"preview_loaded":s.preview.is_some(),"preview":s.preview.as_ref().map(|p|json!({"before_bytes":p.before().map(|b|b.len()),"after_bytes":p.after().map(|b|b.len()),"commit":p.commit().map(|c|c.as_str()),"conversion":p.conversion()})),"backups":s.backups.iter().map(|b|json!({"id":b.id(),"path":b.path().display_lossy(),"present":b.original_present()})).collect::<Vec<_>>()})),
                "worktrees":d.worktrees.as_ref().map(|w|json!({"loading":w.running(),"applying":w.applying,"creating":w.creating,"removing":w.removing,"maintaining":w.maintaining.map(|k|k.label()),"existing":w.existing,"branch":w.branch,"target":w.target,"selected":w.selected.as_ref().map(|p|p.to_string_lossy()),"entries":w.entries.iter().map(worktree_value).collect::<Vec<_>>(),"preview_loaded":w.preview.is_some(),"preview":w.preview.as_ref().map(|p|json!({"path":p.path().to_string_lossy(),"target":p.target().as_str(),"removing":p.removing(),"recovery_reference":p.recovery_reference()}))})),
                "remote_tags":d.tags.as_ref().filter(|t|t.remote.active).map(|t|remote_tag_state(&t.remote)),
                "submodules":d.submodules.as_ref().map(submodule_state),
                "lfs":d.lfs.as_ref().map(lfs_state),
                "tags":d.tags.as_ref().map(tags_state_value),
                "recovery":d.recovery.as_ref().map(|r|json!({"scope":format!("{:?}",r.scope),"loading":r.log_task.is_some()||r.task.is_some(),"applying":r.applying,"limit":r.limit,"selected_row":r.selected_row,"target":r.target.as_ref().map(|id|id.as_str()),"action":r.action.as_ref().map(|a|a.label()),"preview_loaded":r.preview.is_some(),"preview":r.preview.as_ref().map(|p|json!({"target":p.target().id.as_str(),"recovery_reference":p.recovery_reference()})),"more":r.page.as_ref().is_some_and(|p|p.more),"entries":r.page.as_ref().map(|p|p.entries.iter().map(|e|json!({"id":e.id.as_str(),"selector":String::from_utf8_lossy(&e.selector),"subject":String::from_utf8_lossy(&e.subject)})).collect::<Vec<_>>())})),
                "rebase_preview_loading":d.rebase.as_ref().is_some_and(|r|r.task.is_some()),
                "rebase_plan":d.rebase.as_ref().map(|r|json!({"reviewed":r.reviewed,"selected":r.selected,"entries":r.entries.iter().map(|e|json!({"commit":e.commit.as_str(),"action":e.instruction.label()})).collect::<Vec<_>>()})),
                "rebase_preview":d.rebase.as_ref().and_then(|r|r.preview.as_ref()).map(|p|json!({"branch":String::from_utf8_lossy(p.branch()),"original":p.original().as_str(),"onto":p.onto().map(|o|o.as_str()),"commits":p.commits().iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),"stopped":p.stopped().map(|s|s.as_str()),"fast_forward":p.fast_forward(),"up_to_date":p.up_to_date(),"message_retry":p.message_recovery().map(|(retry,_)|retry)}))});
            // Separate statements keep each json! literal within the macro's recursion limit.
            value["atomic_push"] = d.atomic_push.as_ref().map_or(Value::Null, atomic_push_state);
            value["ignore"] = d.ignore.as_ref().map_or(Value::Null, ignore_state);
            value["workflow"] = d.workflow.as_ref().map_or(Value::Null, workflow_state);
            value["fetch_tags"] = json!(match d.fetch_tags {rocket_git::FetchTags::None=>"none",rocket_git::FetchTags::RemoteDefault=>"default",rocket_git::FetchTags::All=>"all"});
            value["allow_unrelated"] = json!(d.merge.as_ref().is_some_and(|s|s.allow_unrelated));
            value["stash_selected"] = json!(d.stash_paths.as_ref().map(Vec::len));
            value["stash_files"] = json!(d.stash_paths.as_ref().map(|_| {
                let entries = self.app.tabs.iter().find(|t| Some(t.id) == d.owner).and_then(|t| t.snapshot.as_ref()).and_then(|s| s.status.as_ref()).map_or(&[][..], |s| s.entries.as_slice());
                rocket_ui::stash_save::candidates(entries, d.flag).iter().map(|e| e.path.display_lossy().into_owned()).collect::<Vec<_>>()
            }));
            value["stash_blocked"] = json!(d.stash_paths.is_some() && self.app.tabs.iter().find(|t| Some(t.id) == d.owner).and_then(|t| t.snapshot.as_ref()).and_then(|s| s.status.as_ref()).is_some_and(|s| rocket_ui::stash_save::blocked(&s.entries)));
            value["delete_branches"] = json!(matches!(d.kind, DialogKind::DeleteBranches).then(|| {
                let branches = self.app.tabs.iter().find(|t| Some(t.id) == d.owner).and_then(|t| t.snapshot.as_ref()).map_or(&[][..], |s| s.branches.as_slice());
                json!({"rows":rocket_ui::batch::branch_items(branches, &d.first).iter().map(|i| &i.label).collect::<Vec<_>>(),"selected":d.selected_branches.iter().map(|n| String::from_utf8_lossy(n)).collect::<Vec<_>>()})
            }));
            value["tag_batch"] = d.tags.as_ref().map_or(Value::Null, tag_batch_state);
            value["command_editor"] = json!(d.commands.as_ref().map(|e| json!({"selected":e.selected,"count":e.list.len(),"problem":e.problem(),"transferring":e.transfer.is_some(),"import_count":e.imported.as_ref().map(Vec::len),"notice":e.notice,"error":e.error})));
            value["issue_links"] = json!(d.issue_links.as_ref().map(|e| json!({"shown":e.shown,"loading":e.running(),"loaded":e.settings.is_some(),
                "changed":e.changed(),"problem":e.problem(),"error":e.error,
                "rules":e.rules.iter().map(|r| json!({"name":r.name,"pattern":r.pattern,"url":r.url,"valid":r.validate().is_ok()})).collect::<Vec<_>>(),
                "inherited":e.settings.as_ref().map(|s| s.inherited.iter().map(|(scope, r)| json!({"scope":format!("{scope:?}"),"name":r.name})).collect::<Vec<_>>()),
                "problems":e.settings.as_ref().map(|s| s.problems.len())})));
            value["connection"] = json!(d.connection.as_ref().map(|c| json!({"running":c.running(),
                "current":rocket_ui::remote_check::target(d).as_deref() == Some(c.target.as_str()),
                "result":c.result.as_ref().map(|r| match r {
                    Ok(check) => json!({"connected":true,"branches":check.branches,"elapsed_ms":check.elapsed.as_millis(),
                        "default_branch":check.default_branch.as_ref().map(|b| String::from_utf8_lossy(b))}),
                    Err(f) => json!({"connected":false,"code":f.code,"message":f.message}),
                })})));
            // Repository Statistics: choices, read state and the shown results.
            value["statistics"] = json!(d.statistics.as_ref().map(statistics_state));
            // Git Flow: request, settings, branches and the reviewed steps.
            value["gitflow"] = json!(d.gitflow.as_ref().map(gitflow_state));
            // A patch pasted from the clipboard: its size only, never its text.
            value["patch_clipboard_bytes"] = json!(d.patch.as_ref().and_then(|s| s.text.as_ref()).map(|t| t.len()));
            value["mail_import"]=json!(d.mail_import.as_ref().map(|s|json!({"loading":s.task.is_some(),"applying":s.applying,"stopping":s.stopping,"preview":s.preview.as_ref().map(|p|json!({"action":p.action().label(),"current":p.current(),"entries":p.entries().len(),"original":p.original().as_str(),"recovery":p.recovery_reference(),"changed_files":p.changed_files(),"truncation_required":p.requires_truncation_acknowledgement(),"truncation_acknowledged":p.truncation_acknowledged()}))})));
            value["rebase_preserve_merges"]=json!(d.rebase.as_ref().map(|s|json!({"choice":s.preserve_merges,"preview":s.preview.as_ref().map(|p|p.preserves_merges())})));
            value["run_command"] = json!(match d.kind {
                DialogKind::RunCommand(index) => Some(rocket_ui::custom_commands::command_line(&self.app.commands, &self.app.tabs, d.owner, index, &d.command_values, d.command_context.as_ref())
                    .map_or_else(|| rocket_ui::custom_commands::command_url(&self.app.commands,&self.app.tabs,d.owner,index,&d.command_values,d.command_context.as_ref())
                        .map_or(json!({"ready":false}),|url|json!({"ready":true,"action":"url","url":url})),
                        |(name, _, arguments)| json!({"ready":true,"action":"program","name":name,"historical_file":d.command_context.as_ref().is_some_and(|c|c.historical_file),"arguments":arguments.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>()}))),
                _ => None,
            });
            value["rebase_plan_extras"] = json!(d.rebase.as_ref().map(|r|r.entries.iter().map(|e|json!({"exec":e.exec,"stop_after":e.stop_after})).collect::<Vec<_>>()));
            // Rebase `--update-refs`: the choice and the branches in the replayed commits.
            value["rebase_update_refs"] = json!(d.rebase.as_ref().map(|r| json!({"choice":r.update_refs,
                "preview":r.preview.as_ref().map(|p| p.update_refs()),
                "branches":r.preview.as_ref().map(|p| p.dependents().iter().map(|b| json!({
                    "branch":String::from_utf8_lossy(&b.reference),"target":b.target.as_str(),"moves":b.moves()})).collect::<Vec<_>>())})));
            value["rebase_planned_stop"] = json!(d.rebase.as_ref().and_then(|r|r.preview.as_ref()).and_then(|p|p.planned_stop()).map(|stop|match stop {
                rocket_git::RebaseStop::Edit(id) => json!({"kind":"edit","commit":id.as_str()}),
                rocket_git::RebaseStop::Break => json!({"kind":"break"}),
                rocket_git::RebaseStop::Command(command) => json!({"kind":"command","command":command}),
            }));
            value["recovery_references"] = json!(d.recovery.as_ref().and_then(|r|r.page.as_ref()).map(|p|p.entries.iter().map(|e|e.reference.as_ref().map(|n|String::from_utf8_lossy(n))).collect::<Vec<_>>()));
            value["undo"] = json!(d.undo.as_ref().map(|s|json!({"loading":s.task.is_some(),"applying":s.applying,"preview_loaded":s.preview.is_some(),"preview":s.preview.as_ref().map(|p|json!({"commit":p.record().commit().as_str(),"previous":p.record().previous().map(|id|id.as_str()),"recovery_reference":p.recovery_reference()}))})));
            if let Some(r) = &d.recovery {
                value["recovery"]["scan_stage"] = json!(r.scan_stage);
                value["recovery"]["read_notice"] = json!(r.read_notice);
                value["recovery"]["scan_total"] = json!(r.lost_scan.as_ref().map(|s|s.commits().len()));
                if let Some(p) = &r.preview {
                    value["recovery"]["preview"]["lost_commit"] = json!(p.is_lost_commit());
                    value["recovery"]["preview"]["parents"] = json!(p.target().parents.iter().map(|id|id.as_str()).collect::<Vec<_>>());
                    value["recovery"]["preview"]["patch_bytes"] = json!(p.patch().len());
                    value["recovery"]["preview"]["subject"] = json!(p.target().subject_lossy());
                }
            }
            value["commit_preview"] = json!(d.commit.as_ref().and_then(|c|c.preview.as_ref()).map(|p|json!({"commits":p.commits().iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),"sequence":p.is_sequence(),"remaining":p.remaining().iter().map(|id|id.as_str()).collect::<Vec<_>>(),"applied":p.applied().len(),"recovery_reference":p.recovery_reference()})));
            value["commit_prediction"] = json!(d.commit.as_ref().and_then(|c|c.preview.as_ref()).and_then(|p|p.prediction()).map(|p|json!({"outcome":p.kind.code(),"checked":p.checked,"total":p.total,"stopped_at":p.stopped_at,"paths":p.paths.iter().map(|v|v.display_lossy()).collect::<Vec<_>>(),"reason":p.reason.map(|v|v.code())})));
            value["rebase_prediction"] = json!(d.rebase.as_ref().and_then(|s|s.preview.as_ref()).and_then(|p|p.prediction()).map(prediction_value));
            value["rebase_prediction_stopping"] = json!(d.rebase.as_ref().is_some_and(|s|s.cancelling));
            value["commit_prediction_stopping"] = json!(d.commit.as_ref().is_some_and(|c|c.cancelling));
            value
        });
        state["write"] = self.app.write.as_ref().map_or(Value::Null, |w| json!({"operation":w.label,"progress":w.progress,"fraction":w.fraction,"cancelling":w.cancelling,"cancel_enabled":w.can_cancel(),"force_push_phase":w.force_push_phase_code(),"elapsed_ms":w.started.elapsed().as_millis()}));
        state["activities"] = json!(self.app.activities.iter().map(|a| json!({"operation":a.title,"command_succeeded":a.outcome.command_succeeded,"recovery_pending":a.outcome.recovery_pending,"summary":a.outcome.summary,"details":a.outcome.details,"elapsed_ms":a.elapsed.as_millis(),"conflict":a.outcome.conflict.as_ref().map(|c|json!({"file_save_confirmed":c.saved,"staging_confirmed":c.staged})),"whole_conflict":a.outcome.whole_conflict.as_ref().map(|c|json!({"keep":c.keep.label(),"file_change":c.file_change.map(|f|f.label()),"staging_confirmed":c.staged}))})).collect::<Vec<_>>());
        state["log_directory"] = json!(self.app.log_path.to_string_lossy());
        state["dropped_events"] = json!(
            self.app
                .backend
                .as_ref()
                .map(|b| b.log.health().dropped_events)
        );

        crate::state_privacy::protect(state, self.policy.private_state_allowed())
    }
    fn validate_terminal_probe(&self, text: &str) -> Result<(), String> {
        let tab = self.app.tab().ok_or("Open a fixture first")?;
        let (root, _) = self.policy.repository(&tab.path)?;
        terminal_probe_allowed(text, &root)
    }
    /// Git Flow under MCP writes only branches in the fixture's marked namespace.
    fn gitflow_scope(&self, dialog: &rocket_ui::app::Dialog, path: &Path) -> Result<(), String> {
        use rocket_ui::gitflow::Preview;
        let namespace = self
            .policy
            .gitflow_namespace(path)
            .ok_or("Git Flow tests require the .git/rocket-gitflow-fixture namespace marker")?;
        let state = dialog.gitflow.as_ref().ok_or("Git Flow dialog is not ready")?;
        let inside = |name: &str| name.starts_with(&namespace);
        let scoped = match state.preview.as_ref().ok_or("Review Git Flow first")? {
            Preview::Init(p) => {
                let c = p.config();
                [&c.master, &c.develop, &c.feature, &c.release, &c.hotfix, &c.support]
                    .iter()
                    .all(|name| inside(name))
            }
            Preview::Start(p) => inside(p.branch()),
            Preview::Finish(p) => {
                inside(p.branch()) && inside(&p.config().develop) && inside(&p.config().master)
            }
        };
        if scoped {
            Ok(())
        } else {
            Err("Git Flow branches must stay in the fixture's marked namespace".into())
        }
    }
    fn local_remote_url(&self, value: &[u8]) -> Result<(), String> {
        self.policy.remote_url(value)
    }
    fn local_remote(&self, remote: &rocket_git::Remote) -> Result<(), String> {
        for url in remote.urls.iter().chain(&remote.push_urls) {
            self.local_remote_url(url)?;
        }
        Ok(())
    }
    /// Every open repository is in scope and loaded, with only local fixture remotes.
    fn local_remotes_only(&self) -> bool {
        self.scope_ok()
            && self.app.tabs.iter().all(|tab| {
                tab.snapshot
                    .as_ref()
                    .is_some_and(|s| s.remotes.iter().all(|r| self.local_remote(r).is_ok()))
            })
    }
    /// Controls of a modal that is on top: the Browse picker, the Open folder browser,
    /// External tools and MCP server inside Preferences, or the external tool dialog.
    fn modal_owns(&self, target: &str) -> bool {
        if self.app.picker.is_some() {
            return target.starts_with("picker-");
        }
        // Below the picker: Save as in Crash reports opens one.
        if self.app.crash_reports.open { return target.starts_with("crash-"); }
        if self.app.map_export_open() { return target.starts_with("map-export-"); }
        match self.app.dialog.as_ref().map(|d| &d.kind) {
            Some(DialogKind::Preferences) => {
                target.starts_with("tools-") || target.starts_with("mcp-") || target.starts_with("git-") || target == "prefs-scroll"
                    // Accounts and SSH keys keep the control names of their former pages.
                    || (target.starts_with("hosting-") && self.app.preferences_page() == Some(rocket_ui::preferences::Page::Accounts))
                    || (target.starts_with("ssh-") && self.app.preferences_page() == Some(rocket_ui::preferences::Page::SshKeys))
                    || (target.starts_with("theme-") && self.app.preferences_page() == Some(rocket_ui::preferences::Page::Appearance))
                    || (matches!(target, "security-secret-coverage" | "security-ai-key-coverage") && self.app.preferences_page() == Some(rocket_ui::preferences::Page::Security))
            }
            Some(DialogKind::Open) => target.starts_with("open-"),
            Some(DialogKind::Workflows) => target.starts_with("workflow-"),
            Some(_) => false,
            None => self.app.external.open && target.starts_with("tools-"),
        }
    }
    /// A file drop: the fixture scope and active tab, no blocking dialog or busy work,
    /// and each path an owned fixture repository folder or a file inside the root.
    fn drop_guard(&self, request: &Request) -> Result<(), String> {
        request.ticket.timing.mark(Phase::Guard);
        if !self.scope_ok() {
            return Err("Repository outside the fixture scope".into());
        }
        if self.app.quitting {
            return Err("Rocket is closing".into());
        }
        let tab = self.app.tab().ok_or("Open a fixture first")?;
        if request.args["expected_tab"].as_u64() != Some(tab.id) {
            return Err("Active repository changed; read state again".into());
        }
        if self.app.blocking_dialog() {
            return Err("Dismiss or complete the active dialog first".into());
        }
        if self.app.interaction_busy() {
            return Err("Wait for the current repository operation to settle".into());
        }
        for path in request.args["paths"].as_array().into_iter().flatten() {
            let path = Path::new(path.as_str().unwrap_or_default());
            if path.is_dir() {
                self.policy.repository(path)?;
            } else if path.is_file() {
                self.policy.patch_file(path)?;
            } else {
                return Err("Dropped paths must exist inside the fixture root".into());
            }
        }
        Ok(())
    }
    fn guard(&self, request: &Request) -> Result<(), String> {
        request.ticket.timing.mark(Phase::Guard);
        if !self.scope_ok() {
            return Err("Repository outside the fixture scope".into());
        }
        if self.app.quitting {
            return Err("Rocket is closing".into());
        }
        let tab = self.app.tab().ok_or("Open a fixture first")?;
        if request.args["expected_tab"].as_u64() != Some(tab.id) {
            return Err("Active repository changed; read state again".into());
        }
        if request.name == "rocket_shortcut" {
            let binding = rocket_ui::shortcuts::model::Binding::parse(request.args["binding"].as_str().unwrap_or(""))?;
            if let Some(serial) = request.args["recording_id"].as_u64() {
                if !self.app.shortcut_recording() || self.app.keybindings.capture_serial() != Some(serial) {
                    return Err("Shortcut recording ended or changed; no input was delivered".into());
                }
            } else {
                binding.validate()?;
                if self.app.shortcut_recording() || !self.app.shortcut_probe_safe(binding) {
                    return Err("Shortcut probes allow only safe navigation outside dialogs; recording requires its current recording_id".into());
                }
                if !matches!(binding.trigger, rocket_ui::shortcuts::model::Trigger::Key(_)) {
                    let pos = request.args["target"].as_str().and_then(|id| self.app.frame_widgets.get(id)).map_or(pos2(4.0,4.0), |c| c.rect.center());
                    if ["terminal-screen", "terminal-command"].iter().any(|id|self.app.frame_widgets.get(*id).is_some_and(|c|c.rect.contains(pos))) {
                        return Err("Shortcut probes cannot pass mouse gestures into the terminal".into());
                    }
                }
            }
            if let Some(target) = request.args["target"].as_str()
                && self.app.frame_widgets.get(target).is_none_or(|c| !c.enabled || c.rect.width()<1.0 || c.rect.height()<1.0) {
                return Err("Shortcut pointer target is not visible and enabled".into());
            }
            return Ok(());
        }
        let target = request.args["target"].as_str().unwrap();
        if request.args.get("button").is_some() && !matches!(request.args["button"].as_str(),Some("left"|"right")) {
            return Err("Click button must be left or right".into());
        }
        // Choosing a Git row checks and saves that program at once.
        if let Some(choice) = self.app.git_settings.choice_for(target).filter(|c| !c.is_empty()) {
            let candidate = Path::new(&choice).canonicalize().ok();
            // This named choice resolves beside this app and verifies the complete
            // compiled inventory before executing or saving anything. It cannot
            // be redirected through a request argument or repository setting.
            let bundled = target == "git-managed" && choice == "managed";
            if !bundled && !self.app.backend.as_ref().is_some_and(|b|candidate.as_deref()==Some(b.executable.as_path())) {
                return Err("Automation checks only the Git executable Rocket already uses".into());
            }
        }
        if request.args["button"].as_str()==Some("right")
            && !(target.starts_with("commit-") || target.starts_with("branch-row-") || target.starts_with("sidebar-branch-refs/") || change_row_target(target) || target.starts_with("dialog-submodule-row-") || target.starts_with("patch-file-") || target.starts_with("tree-entry-") || target.starts_with("files-match-") || target.starts_with("file-commit-")) {
            return Err("Right-click automation is limited to commit, branch, changed-file and submodule rows".into());
        }
        if target == "manager-open" {
            for path in self.app.manager_open_paths().map_err(str::to_owned)? { self.policy.repository(&path)?; }
        }
        // Fetch all, Pull all and Open N in tabs reach every repository they list: each must be a
        // marked fixture. A folder that is gone is skipped without running Git.
        let listed = match target {
            "manager-fetch-all" | "manager-pull-all" => Some(self.app.manager_batch_rows()),
            "manager-result-open" => Some(self.app.manager_result_paths()),
            _ => None,
        };
        for (_, path) in listed.into_iter().flatten() {
            if !matches!(path.try_exists(), Ok(false)) { self.policy.repository(&path)?; }
        }
        if target == "manager-scan-start" && let Some(s)=&self.app.manager.scan {
            rocket_ui::repository_scan::discovery::check_root(Path::new(s.root.trim()),Some(&self.policy.root)).map_err(|_|"Choose a normal folder inside the MCP test root".to_owned())?;
        }
        if target == "manager-scan-add" && let Some(s)=&self.app.manager.scan {
            for path in s.selected() {self.policy.repository(&path)?;}
        }
        if target == "manager-save" && let Some(f)=&self.app.manager.form
            && matches!(f.kind, rocket_ui::repository_manager::FormKind::Repository(_)) {
            self.policy.repository(Path::new(f.path.trim()))?;
        }
        // Publish on the local tab reads the remote straight away.
        let local_publish = target == "dialog-tag-publish";
        if (local_publish || matches!(
            target,
            "dialog-remote-tag-refresh"
                | "dialog-remote-tag-retry"
                | "dialog-remote-tag-publish"
                | "dialog-remote-tag-delete"
                | "dialog-batch-review"
                | "dialog-submit"
        )) && let Some(dialog) = self
            .app
            .dialog
            .as_ref()
            .filter(|d| d.tags.as_ref().is_some_and(|t| t.remote.active || local_publish))
        {
            let name = dialog
                .remotes
                .get(dialog.remote)
                .ok_or("Select a local fixture remote")?;
            // Fresh remote/configuration reads run once on the validation worker.
            if !tab
                .snapshot
                .as_ref()
                .is_some_and(|s| s.remotes.iter().any(|r| &r.name == name))
            {
                return Err("Remote no longer exists".into());
            }
            let batch = dialog.tags.as_ref().and_then(|t| t.remote.batch.as_ref());
            if target == "dialog-submit"
                && let Some(batch) = batch
            {
                let review = batch.ready().ok_or("Review the selected tags first")?;
                for p in &review.ready {
                    self.local_remote_url(p.destination())?;
                }
            } else if target == "dialog-submit" {
                let p = dialog
                    .tags
                    .as_ref()
                    .and_then(|t| t.remote.preview.as_ref())
                    .ok_or("Review the remote tag first")?;
                self.local_remote_url(p.destination())?;
            }
        }
        if target.starts_with("pref-auto-fetch") && self.app.auto_fetch.blocked {
            return Err("Automatic fetch through MCP requires the runner's rocket-auto-fetch marker and local fixture remotes".into());
        }
        if matches!(target, "pref-secret-protection" | "pref-ai-key-guard") {
            return Err("Content guards are user preferences. MCP cannot disable or change them; correct the blocked content before retrying.".into());
        }
        if matches!(target, "pref-dependency-push" | "dependency-push-continue") {
            return Err("Pre-push dependency checks and warning approval are user controls in the normal Rocket application.".into());
        }
        if target.starts_with("shell-") {
            return Err(format!("The shell is a user preference. MCP cannot change it, and an MCP window always runs {}.", if cfg!(windows) { "Windows PowerShell" } else { "bash" }));
        }
        if target == "tools-start" {
            self.policy
                .external_executable(Path::new(&self.app.external.config().executable))?;
            if self.app.external.owner != Some(tab.id) {
                return Err("Select the tool's source fixture tab before launching".into());
            }
        }
        if matches!(target, "dialog-patch-review" | "dialog-submit")
            && let Some(state) = self.app.dialog.as_ref().and_then(|d| d.patch.as_ref())
            // Selections are already pinned to this fixture's diff; they have no patch file.
            && !matches!(state.mode, rocket_ui::patches::Mode::Discard(..) | rocket_ui::patches::Mode::Stash(..))
            // A patch pasted from the clipboard has no file to check.
            && !(matches!(state.mode, rocket_ui::patches::Mode::Import) && state.text.is_some())
        {
            self.policy.patch_file(Path::new(state.path.trim()))?;
        }
        if target == "mcp-copy-token" {
            return Err("Access tokens are not exposed through automation or its clipboard".into());
        }
        if request.name == "rocket_type" && target == "mcp-root" {
            self.policy
                .remote_url(request.args["text"].as_str().unwrap_or("").as_bytes())?;
        }
        if target == "mcp-save" {
            self.policy.remote_url(self.app.mcp.root.as_bytes())?;
        }
        if request.name == "rocket_type" && target == "pref-ai-executable" {
            let text = request.args["text"].as_str().unwrap_or("").trim();
            if !text.is_empty() {
                self.policy.external_executable(Path::new(text))?;
            }
        }
        if request.name=="rocket_type"&&target=="dialog-bisect-program" {
            let text=request.args["text"].as_str().unwrap_or("").trim();
            if !text.is_empty(){self.policy.external_executable(Path::new(text))?;}
        }
        if matches!(target,"dialog-preview-refresh"|"dialog-submit")
            && let Some(s)=self.app.dialog.as_ref().and_then(|d|d.bisect.as_ref())
            && (target=="dialog-preview-refresh"&&s.automatic.enabled||target=="dialog-submit"&&s.automatic.preview.is_some()) {
            let program=s.automatic.preview.as_ref().map_or_else(||Path::new(s.automatic.program.trim()),|p|p.command().program.as_path());
            self.policy.external_executable(program).map_err(|_|"Automated bisect under MCP requires a disposable test executable inside the fixture root".to_owned())?;
        }
        // A draft runs Claude Code: under MCP only the fixture's stub inside the root.
        if target == "review-send" {
            let review = &self.app.tab().ok_or("Open a fixture repository first")?.branch_review;
            let program = self.app.drafting.program_for(review.provider).ok_or("No AI CLI program is configured")?;
            self.policy.external_executable(&program).map_err(|_| "AI review under MCP requires a fixture stub executable inside the fixture root".to_owned())?;
        }
        // A conflict proposal starts Claude Code or Codex too: the same stub-only rule.
        let proposer = match target {
            "conflict-ai-claude" => Some(rocket_git::DraftProvider::Claude),
            "conflict-ai-codex" => Some(rocket_git::DraftProvider::Codex),
            _ => None,
        };
        if let Some(provider) = proposer {
            let name = provider.label();
            let program = self.app.drafting.program_for(provider).ok_or(format!("{name} has no program path"))?;
            self.policy.external_executable(&program).map_err(|_| {
                format!("{name} under MCP must be the fixture's stub .exe inside the fixture root")
            })?;
        }
        if target == "composer-draft"
            || (target == "dialog-submit"
                && self
                    .app
                    .dialog
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DialogKind::DraftNotice(_))))
        {
            let program = self
                .app
                .drafting
                .program()
                .ok_or("Claude Code has no program path")?;
            self.policy.external_executable(&program).map_err(|_| {
                "Claude Code under MCP must be the fixture's stub .exe inside the fixture root"
                    .to_owned()
            })?;
        }
        if request.name == "rocket_type" && target == "dialog-remote-url" {
            self.local_remote_url(request.args["text"].as_str().unwrap_or("").as_bytes())?;
        }
        if request.name == "rocket_type" && target == "dialog-clone-source" {
            self.local_remote_url(request.args["text"].as_str().unwrap_or("").as_bytes())?;
        }
        // Explore's box: a search goes to the configured GitHub endpoint; an address
        // is previewed or cloned, so it must be a local fixture repository.
        if request.name == "rocket_type" && target == "explore-query" {
            let text = request.args["text"].as_str().unwrap_or("");
            if text.contains("://") || rocket_ui::explore::address(text).is_some() {
                self.local_remote_url(text.trim().as_bytes())?;
            }
        }
        // Previews download from their source: only local fixture repositories.
        if let Some(source) = self.preview_source(target) {
            self.local_remote_url(source?.as_bytes())?;
        }
        if target == "dialog-submit" {
            self.keep_scope()?;
        }
        if target == "dialog-test-connection" {
            // Test connection runs Git's transport: only local fixture paths.
            let dialog = self
                .app
                .dialog
                .as_ref()
                .ok_or("Open a remote or Clone dialog")?;
            let url = rocket_ui::remote_check::target(dialog).ok_or("Enter a URL or path first")?;
            self.local_remote_url(url.as_bytes())?;
        }
        if target == "dialog-init-open" {
            // New repository's "Open it" switches the window to the repository it found.
            let path = self
                .app
                .dialog
                .as_ref()
                .and_then(|d| d.create.as_ref())
                .and_then(|f| f.repository())
                .ok_or("Check a project folder first")?;
            self.policy.repository(path)?;
        }
        if request.name == "rocket_type" && target == "dialog-submodule-source" {
            self.policy
                .remote_url(request.args["text"].as_str().unwrap_or("").as_bytes())?;
        }
        if request.name == "rocket_type" && target == "dialog-commands-program" {
            let text = request.args["text"].as_str().unwrap_or("");
            let url_edit=self.app.opened_urls.is_some() && text==COMMAND_TEST_URL
                && self.app.dialog.as_ref().and_then(|d|d.commands.as_ref()).and_then(|e|e.selected.and_then(|i|e.list.get(i))).is_some_and(|c|c.action==rocket_git::CommandAction::Url);
            if !self
                .app
                .backend
                .as_ref()
                .is_some_and(|b| Path::new(text) == b.executable) && !url_edit
            {
                return Err(COMMAND_REFUSAL.into());
            }
        }
        if request.name == "rocket_type" && target == "dialog-commands-arguments" {
            let text = request.args["text"]
                .as_str()
                .unwrap_or("")
                .replace("\r\n", "\n");
            if !COMMAND_TEST_ARGUMENTS.contains(&text.as_str()) && text != COMMAND_FORM_TEST_ARGUMENTS {
                return Err(COMMAND_REFUSAL.into());
            }
        }
        if request.name == "rocket_type" && target == "dialog-rebase-exec" {
            let text = request.args["text"].as_str().unwrap_or("");
            if !text.is_empty() && !REBASE_TEST_COMMANDS.contains(&text) {
                return Err(REBASE_COMMAND_REFUSAL.into());
            }
        }
        if target == "dialog-submit" {
            rebase_commands_allowed(self.app.dialog.as_ref())?;
        }
        if target == "dialog-preview-refresh"
            && let Some(s) = self.app.dialog.as_ref().and_then(|d| d.submodules.as_ref())
            && matches!(s.mode, rocket_ui::submodules::ManagementMode::Add | rocket_ui::submodules::ManagementMode::SetUrl)
        {
            self.policy.remote_url(s.source.as_bytes())?;
        }
        if request.name == "rocket_type" && target == "dialog-series-folder" {
            let text=request.args["text"].as_str().unwrap_or("");
            if !text.is_empty(){self.policy.patch_file(&Path::new(text).join("0001-review.patch"))?;}
        }
        if target == "dialog-preview-refresh" && self.app.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::MailImport(rocket_git::MailImportAction::Start))) {
            self.policy.patch_file(&Path::new(&self.app.dialog.as_ref().unwrap().first).join("0001-review.patch"))?;
        }
        if target == "terminal-command" {
            self.validate_terminal_probe(request.args["text"].as_str().unwrap_or(""))?;
        }
        if target == "terminal-send" {
            let input = self
                .app
                .tab()
                .and_then(|t| t.terminal.as_ref())
                .ok_or("Open the fixture terminal first")?;
            self.validate_terminal_probe(&input.command)?;
        }
        if request.name == "rocket_terminal_enter" {
            let input = self
                .app
                .tab()
                .and_then(|t| t.terminal.as_ref())
                .ok_or("Open the fixture terminal first")?;
            self.validate_terminal_probe(&input.command)?;
        }
        if request.name == "rocket_terminal_probe" {
            if self.app.terminal_history_file.is_none()
                || !tab.terminal.as_ref().is_some_and(|t| t.status.running)
            {
                return Err("Open an owned terminal with isolated history first".into());
            }
            if crate::terminal_probes::keyboard(request.args["probe"].as_str().unwrap_or(""))
                .is_none()
            {
                return Err("Unknown fixed terminal keyboard probe".into());
            }
        }
        let control = self
            .app
            .frame_widgets
            .get(target)
            .ok_or(if history_row_target(target) {
                "Commit row is not visible. Scroll history-scroll or filter commits, then read state before retrying."
            } else {
                "Control is not present in this UI state"
            })?;
        if history_row_target(target)
            && !self.app.frame_widgets.get("history-scroll")
                .is_some_and(|viewport| viewport.rect.contains_rect(control.rect))
        {
            return Err("Commit row is outside the visible history. Read state before retrying.".into());
        }
        if !control.enabled {
            return Err("Control is disabled in this UI state".into());
        }
        if target.starts_with("ssh-") {
            if self.app.preferences_page() != Some(rocket_ui::preferences::Page::SshKeys)
                || !self.app.ssh.enabled || self.policy.ssh_fixture().as_ref() != Some(&self.app.ssh.root)
                || self.app.ssh.root.parent() != Some(self.app.ssh.scan_directory.as_path()) {
                return Err("SSH controls require the marked disposable SSH fixture".into());
            }
            // Discovery and Stop must remain usable if a previously selected key disappeared.
            // The scan directory is fixed to the marked fixture above.
            if matches!(target, "ssh-scan" | "ssh-cancel") { return Ok(()); }
            if let Some(p) = self.app.ssh.profile() { self.policy.patch_file(&p.key)?; self.policy.patch_file(&p.config)?; }
            if let Some(k) = self.app.ssh.found_key() { self.policy.patch_file(&k.key)?; }
            // A key's tick acts on its own row's key, which needn't be the chosen one.
            let ssh = &self.app.ssh;
            if let Some(i) = target.strip_prefix("ssh-tick-profile-") {
                let p = i.parse::<usize>().ok().and_then(|i| ssh.profiles.get(i)).ok_or("Unknown SSH profile")?;
                self.policy.patch_file(&p.key)?; self.policy.patch_file(&p.config)?;
            }
            if let Some(i) = target.strip_prefix("ssh-tick-found-") {
                let k = i.parse::<usize>().ok().and_then(|i| ssh.discovery.as_ref()?.keys.get(i)).ok_or("Unknown SSH key")?;
                self.policy.patch_file(&k.key)?;
            }
            if target == "ssh-create" && self.app.ssh.use_existing { self.policy.patch_file(std::path::Path::new(&self.app.ssh.existing))?; }
            return Ok(());
        }
        if target.starts_with("int-") {
            if !self.app.integrations.isolated || self.app.integrations.fixture != self.policy.hosting_fixture()
                || self.app.integrations.directory != self.app.state_directory {
                return Err("Integration controls require this isolated profile and its configured loopback service".into());
            }
            // A review opens over whichever page its row was on.
            if self.app.preferences_page() != Some(rocket_ui::preferences::Page::Integrations)
                && self.app.page() != Some(rocket_ui::pages::Page::Integrations)
                && !self.app.integrations.review_open() {
                return Err("Open the integration preferences or panel first".into());
            }
            return Ok(());
        }
        // CI marks, their popover, the re-run review and Actions: only on the marked loopback API.
        if target.starts_with("ci-") || target.starts_with("actions-") {
            let s = &self.app.forge;
            if !s.enabled || s.fixture.is_none() || s.fixture != self.policy.hosting_fixture() || s.account_check_only || s.publish_check.is_some() {
                return Err("CI controls require the marked loopback hosting fixture".into());
            }
            return Ok(());
        }
        if target.starts_with("hosting-") {
            if let Some((repository, branch)) = self.policy.github_publish_check() {
                let s=&self.app.forge;
                if !s.enabled || s.fixture.is_some() || s.account_check_only || s.publish_check.as_deref()!=Some(repository.as_str())
                    || s.remember || s.provider!=rocket_git::ForgeProvider::GitHub || s.server!="https://github.com"
                    || !(self.app.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::Hosting))
                        || self.app.preferences_page()==Some(rocket_ui::preferences::Page::Accounts)) {
                    return Err("Live publishing requires its exact private repository marker and nonpersistent GitHub session".into());
                }
                if !matches!(target,"hosting-tab-accounts"|"hosting-token"|"hosting-connect"|"hosting-cancel"|"hosting-disconnect"|"hosting-scroll"|"hosting-tab-publish"
                    |"hosting-publish-owner"|"hosting-publish-name"|"hosting-publish-description"|"hosting-publish-branch"|"hosting-publish-remote"|"hosting-publish-upstream"
                    |"hosting-publish-private"|"hosting-publish-public-ack"|"hosting-publish-ssh"|"hosting-publish-review"|"hosting-publish-create"|"hosting-publish-back"|"hosting-publish-reset"|"hosting-publish-open") {
                    return Err("Live publishing does not permit other hosting actions or saved credentials".into());
                }
                if matches!(target,"hosting-publish-review"|"hosting-publish-create") {
                    let p=&s.publish;let tab=self.app.tab().ok_or("Open the marked disposable repository")?;
                    let(root,marked)=self.policy.repository(&tab.path)?;
                    let allowed=|d:&rocket_git::GitHubPublishDraft|d.private&&d.transport==rocket_git::GitHubPublishTransport::Ssh&&d.branch==branch&&format!("{}/{}",d.owner,d.name)==repository;
                    if marked!=branch || p.root.as_ref().and_then(|r|r.canonicalize().ok()).as_ref()!=Some(&root)
                        || p.branch!=branch || !p.private || !p.ssh || format!("{}/{}",p.owner,p.name)!=repository
                        || p.preview.as_ref().is_some_and(|v|!allowed(v.draft())) {
                        return Err("Live publishing is confined to the marked private repository, SSH transport and temporary branch".into());
                    }
                }
                return Ok(());
            }
            if self.app.forge.account_check_only && self.policy.hosting_account_check()
                && self.app.forge.enabled && self.app.forge.fixture.is_none()
                && self.app.forge.provider == rocket_git::ForgeProvider::GitHub
                && self.app.forge.server == "https://github.com" && !self.app.forge.remember
                && (self.app.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Hosting))
                    || self.app.preferences_page() == Some(rocket_ui::preferences::Page::Accounts)) {
                return if matches!(target, "hosting-tab-accounts" | "hosting-browser" | "hosting-cancel" | "hosting-disconnect" | "hosting-scroll") {
                    Ok(())
                } else { Err("Live browser acceptance permits only account sign-in, cancellation and disconnect; hosting writes and persistence are disabled".into()) };
            }
            // Hosting, or its accounts in Preferences.
            if !(self.app.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Hosting))
                || self.app.preferences_page() == Some(rocket_ui::preferences::Page::Accounts))
                || !self.app.forge.enabled || self.app.forge.fixture.is_none()
                || self.app.forge.fixture != self.policy.hosting_fixture() {
                return Err("Hosting MCP controls require the marked loopback fixture in the active hosting page".into());
            }
            if matches!(target, "hosting-publish-review" | "hosting-publish-create") {
                let p = &self.app.forge.publish;
                let tab = self.app.tab().ok_or("Open a marked local repository first")?;
                let (root, branch) = self.policy.repository(&tab.path)?;
                if p.root.as_ref().and_then(|r| r.canonicalize().ok()).as_ref() != Some(&root)
                    || p.branch != branch
                    || p.preview.as_ref().is_some_and(|v| v.draft().branch != branch)
                    || p.fixture_root.as_ref() != Some(&self.policy.root.join("publish-remotes")) {
                    return Err("Publishing is confined to this marked fixture branch and local destination".into());
                }
                self.policy.remote_url(self.policy.root.join("publish-remotes").join(format!("{}.git",p.name)).to_string_lossy().as_bytes())?;
            }
            return Ok(());
        }
        if target.starts_with("manager-") {
            if self.app.page() != Some(rocket_ui::pages::Page::RepositoryManager) {
                return Err("Open the repository manager first".into());
            }
            if self.app.manager.scan.is_some() && !target.starts_with("manager-scan-") {
                return Err("Finish or cancel the folder scan first".into());
            }
            if self.app.manager.form.is_some() && !matches!(target,"manager-name"|"manager-path"|"manager-browse"|"manager-folder"|"manager-save"|"manager-cancel"|"manager-form-scroll"|"manager-parent")
                && !target.starts_with("manager-parent-") && !target.starts_with("manager-folder-choice-") && !target.starts_with("manager-member-") && !target.starts_with("manager-active-") {
                return Err("Finish or cancel the repository manager form first".into());
            }
            return Ok(());
        }
        if target.starts_with("commit-link-") {
            if self.app.commit_link.is_none() || self.app.opened_urls.is_none() {return Err("Open the fixture commit-link review first".into());}
            return Ok(());
        }
        if target.starts_with("update-"){
            let notice = matches!(target, "update-notice-open" | "update-notice-close" | "update-notice-later" | "update-status")
                && self.app.frame_widgets.get(target).is_some_and(|c| c.enabled);
            if (!self.app.updates.open && !notice)||self.policy.update_fixture().is_none(){return Err("App-update actions require the owned loopback update fixture.".into());}
            return Ok(());
        }
        // Benchmark controls are read-only, scoped to the page's original fixture.
        // Stopping must still work if an external change removed its repository.
        if matches!(target,"benchmark-stop"|"benchmark-close"|"benchmark-copy"|"benchmark-scroll") {
            if self.app.page()!=Some(rocket_ui::pages::Page::Benchmark) {return Err("Open Repository Benchmark first".into());}
            return Ok(());
        }
        // A management page that is only browsing leaves navigation and menus
        // free in the UI; only reviews, forms, writes and modal dialogs own input.
        if self.app.blocking_dialog()
            && !matches!(target, "cancel-operation" | "status-cancel")
            && !target.starts_with("dialog-")
            && !target.starts_with("push-")
            && !target.starts_with("pref-")
            && !target.starts_with("rows-")
            && !self.modal_owns(target)
        {
            return Err("Dismiss or complete the active dialog first".into());
        }
        // This panel reads only this host's configured diagnostic directory and copies
        // a bounded structured summary; it cannot execute Git or select arbitrary files.
        if target.starts_with("crash-") || target == "help-crash-reports" { return Ok(()); }
        if target.starts_with("activity-") || target == "menu-view" {
            return Ok(());
        }
        if matches!(
            target,
            "dialog-cancel"
                | "dialog-prediction-stop"
                | "dialog-bisect-cancel-read"
                | "dialog-recovery-cancel-read"
                | "dialog-remote-tag-cancel"
                | "cancel-operation"
                | "status-cancel"
                | "diff-search-cancel"
                | "diff-words-cancel"
                | "diff-cancel"
                | "image-cancel"
                | "history-stop"
                | "file-cancel"
                | "tree-cancel"
                | "map-cancel"
                | "map-export-stop"
                | "compare-cancel"
                | "stash-cancel"
                | "conflict-cancel"
                | "conflict-ai-cancel" | "conflict-ai-back" | "conflict-ai-copy" | "conflict-ai-scroll"
                | "conflict-cancel-save"
                | "tools-cancel"
                | "dialog-pull-cancel"
                | "push-lease-cancel"
                | "dialog-atomic-cancel"
                | "dialog-ignore-cancel"
                | "workflow-stop"
                | "dialog-stash-stop"
                | "manager-stop"
                | "manager-cancel"
                | "git-cancel"
                | "dialog-stop"
                | "preview-stop"
                | "explore-stop"
        ) {
            return Ok(());
        }
        // Preferences → Custom commands saves the list: under automation only while every
        // command is one of the fixed Git test commands.
        if target == "dialog-commands-save" {
            let editor = self.app.dialog.as_ref().and_then(|d| d.commands.as_ref()).ok_or(COMMAND_REFUSAL)?;
            if !editor.list.iter().all(|c| test_command(&self.app, c)) {
                return Err(COMMAND_REFUSAL.into());
            }
            return Ok(());
        }
        if target == "dialog-submit"
            && self.app.dialog.as_ref().is_some_and(|d| {
                matches!(
                    d.kind,
                    DialogKind::Quit
                        | DialogKind::Preferences
                        | DialogKind::About
                        | DialogKind::Legal
                        | DialogKind::Changelog
                )
            })
        {
            return Ok(());
        }
        if self.app.interaction_busy()
            && !matches!(
                target,
                "toolbar-console"
                    | "menu-terminal"
                    | "terminal-console"
                    | "console-close"
                    | "drawer-collapse"
                    | "drawer-expand"
                    | "drawer-summary"
                    | "terminal-command"
                    | "terminal-send"
                    | "terminal-interrupt"
                    | "terminal-interrupt-menu"
                    | "terminal-hide"
                    | "terminal-end"
                    | "terminal-end-menu"
            )
        {
            return Err("Wait for the current repository operation to settle".into());
        }
        // A preview refuses every write in the UI, reads only its own folder and
        // downloads only from a source checked above.
        if tab.is_preview() {
            return Ok(());
        }
        let (_, branch) = self.policy.repository(&tab.path)?;
        let snapshot = tab.snapshot.as_ref().ok_or("Repository is still loading")?;
        let recovering_fixture = matches!(snapshot.head, Some(rocket_git::Head::Detached { .. }))
            && snapshot.state.as_ref().is_some_and(|s| s.rebase)
            && self.policy.rebase_branch(&tab.path, &branch)
            && rebase_control_allowed(target, self.app.dialog.as_ref().map(|d| &d.kind));
        let bisect_fixture = matches!(snapshot.head, Some(rocket_git::Head::Detached { .. }))
            && snapshot.state.as_ref().is_some_and(|s| s.bisect)
            && self.policy.bisect_branch(&tab.path, &branch)
            && bisect_control_allowed(target, self.app.dialog.as_ref().map(|d| &d.kind));
        let gitflow_fixture = matches!(snapshot.head.as_ref(), Some(rocket_git::Head::Branch { name, .. })
            if self.policy.gitflow_namespace(&tab.path).is_some_and(|ns| name.starts_with(ns.as_bytes())));
        if !recovering_fixture
            && !bisect_fixture
            && !gitflow_fixture
            && !(matches!(snapshot.head, Some(rocket_git::Head::Detached { .. }))
                && self.policy.is_submodule_fixture(&tab.path))
            && !matches!(snapshot.head.as_ref(), Some(rocket_git::Head::Branch {name,..} | rocket_git::Head::Unborn {branch:name}) if name == branch.as_bytes())
        {
            return Err("Active branch differs from the fixture ownership marker".into());
        }
        if matches!(target,"workflow-review"|"dialog-submit")
            && let Some(dialog)=self.app.dialog.as_ref().filter(|d|d.workflow.is_some()) {
            let workflow=dialog.workflow.as_ref().unwrap();
            if matches!(workflow.mode,6|7|16|17) {
                let base=dialog.first.trim();
                if base!=branch && !base.starts_with(&format!("{branch}-")) {
                    return Err("Branch helpers must keep the base inside the fixture branch namespace".into());
                }
            }
            if workflow.mode==8 && target=="dialog-submit" {
                for reference in &workflow.selected {let name=String::from_utf8_lossy(reference);let expected=format!("refs/heads/{branch}");if name!=expected&&!name.starts_with(&format!("{expected}-")){return Err("Bulk branch updates must stay in the fixture branch namespace".into());}}
            }
            // Maintenance tests operate only on local remotes inside this fixture.
            // Backend transport scope rechecks URLs immediately before execution.
            if matches!(workflow.mode,0|1|3|9|12|16) {for remote in &snapshot.remotes {self.local_remote(remote)?;}}
            if workflow.mode == 5 {
                for remote in &snapshot.remotes {
                    for url in remote.urls.iter().chain(&remote.push_urls) {
                        self.policy.lfs_prune_source(&tab.path, url)?;
                    }
                }
                if target == "dialog-submit" && let Some(preview) = &workflow.preview {
                    if let rocket_ui::workflows::Review::Lfs(p) = preview.as_ref() {
                        self.policy.lfs_prune_preview(&tab.path, p)?;
                    } else { return Err("LFS cleanup requires its own reviewed preview".into()); }
                }
            }
            if workflow.mode==1 && dialog.first!=branch && !dialog.first.starts_with(&format!("{branch}-")) {
                return Err("Remote branch cleanup must stay in the fixture branch namespace".into());
            }
            if target=="dialog-submit" && workflow.preview.is_none(){return Err("Review the workflow first".into());}
            return Ok(());
        }
        if matches!(target, "dialog-atomic-review" | "dialog-submit")
            && let Some(dialog) = self.app.dialog.as_ref().filter(|d| d.atomic_push.is_some())
        {
            let selection = dialog.atomic_push.as_ref().unwrap().selection();
            // A branch cannot coexist with children below its own ref path.
            // Additional test branches use the exact marker plus a hyphen suffix.
            let owned = |name: &[u8]| name == branch.as_bytes() || name.starts_with(format!("{branch}-").as_bytes());
            if selection.is_empty() || selection.iter().any(|s| !owned(&s.local_branch) || !owned(&s.remote_branch)) {
                return Err("Atomic push must keep every branch inside the owned fixture namespace".into());
            }
            dialog.remotes.get(dialog.remote).ok_or("Select a fixture remote")?;
            if target == "dialog-submit" {
                let p = dialog.atomic_push.as_ref().unwrap().preview.as_ref().ok_or("Review the selected branches first")?;
                self.policy.atomic_push_url(&tab.path, p.destination())?;
                if p.entries().iter().map(|e|e.selection.clone()).collect::<Vec<_>>() != selection {
                    return Err("Atomic selection changed; review again".into());
                }
            }
            return Ok(());
        }
        if matches!(target, "push-lease-review" | "dialog-submit")
            && let Some(dialog) = self.app.dialog.as_ref().filter(|d| d.force_push.is_some())
        {
            if dialog.branch != branch.as_bytes() || dialog.second != branch {
                return Err(
                    "Force-with-lease must select the owned fixture branch on both sides".into(),
                );
            }
            dialog
                .remotes
                .get(dialog.remote)
                .ok_or("Select a fixture remote")?;
            if target == "dialog-submit" {
                let s = dialog.force_push.as_ref().unwrap();
                let p = s.preview.as_ref().ok_or("Review the remote branch first")?;
                self.policy.force_push_url(&tab.path, p.destination())?;
                if p.local_branch() != branch.as_bytes()
                    || p.remote_branch() != branch.as_bytes()
                    || s.confirmation.as_bytes() != p.remote_branch()
                {
                    return Err("Confirm the exact reviewed fixture branch first".into());
                }
            }
            return Ok(());
        }
        if target == "dialog-worktree-open" {
            let entry = self
                .app
                .dialog
                .as_ref()
                .and_then(|d| d.worktrees.as_ref())
                .and_then(|s| s.selected_entry())
                .ok_or("Select a worktree first")?;
            let (_, branch) = self.policy.repository(&entry.path)?;
            if entry.branch.as_deref() != Some(format!("refs/heads/{branch}").as_bytes()) {
                return Err("Selected worktree branch differs from its fixture marker".into());
            }
            return Ok(());
        }
        if target == "dialog-submodule-open" {
            let entry = self
                .app
                .dialog
                .as_ref()
                .and_then(|d| d.submodules.as_ref())
                .and_then(|s| s.selected_entry())
                .ok_or("Select a submodule first")?;
            self.policy.repository(&entry.directory)?;
            return Ok(());
        }
        if matches!(
            target,
            "toolbar-fetch"
                | "toolbar-pull"
                | "toolbar-push"
                | "dialog-submit"
                | "dialog-pull-fetch"
        ) {
            if let Some(dialog) = self
                .app
                .dialog
                .as_ref()
                .filter(|_| matches!(target, "dialog-submit" | "dialog-pull-fetch"))
            {
                match &dialog.kind {
                    DialogKind::Lfs => {
                        if let Some(s) = dialog.lfs.as_ref().filter(|s| s.locks_tab) {
                            let p = s
                                .locks
                                .preview
                                .as_ref()
                                .ok_or("Review the LFS lock first")?;
                            if p.source() != self.policy.lfs_lock_source()? {
                                return Err(
                                    "LFS lock endpoint differs from the marked loopback server"
                                        .into(),
                                );
                            }
                            if p.action().force() && !s.locks.force_confirmed {
                                return Err("Confirm the reviewed force unlock first".into());
                            }
                            return Ok(());
                        }
                        let p = dialog
                            .lfs
                            .as_ref()
                            .and_then(|s| s.preview.as_ref())
                            .ok_or("Review LFS first")?;
                        if matches!(
                            p.action(),
                            rocket_git::LfsAction::Fetch(_) | rocket_git::LfsAction::Push(_)
                        ) {
                            if matches!(p.action(), rocket_git::LfsAction::Push(_))
                                && p.local_source().is_none()
                            {
                                self.policy.lfs_upload(&tab.path, p)?;
                            }
                            if let Some(source) = p.local_source() {
                                if !source.starts_with(&self.policy.root) {
                                    return Err(
                                        "LFS source must stay inside the fixture root".into()
                                    );
                                }
                            } else if !p.source().is_some_and(|s| GITHUB_TEST_SOURCES.contains(&s))
                            {
                                return Err(
                                    "LFS remote is outside the approved GitHub test scope".into()
                                );
                            } else if !matches!(p.action(), rocket_git::LfsAction::Fetch(name) | rocket_git::LfsAction::Push(name) if name == "origin")
                                || snapshot.remotes.len() != 1
                                || snapshot.remotes[0].fetch_refspecs
                                    != vec![
                                        format!(
                                            "+refs/heads/{branch}:refs/remotes/origin/{branch}"
                                        )
                                        .into_bytes(),
                                    ]
                            {
                                return Err("Network LFS tests require origin mapped only to the owned fixture branch".into());
                            }
                        }
                        return Ok(());
                    }
                    DialogKind::Submodules => {
                        if let Some(p) = dialog
                            .submodules
                            .as_ref()
                            .and_then(|s| s.action_preview.as_ref())
                        {
                            if !matches!(p.action(), rocket_git::SubmoduleAction::Remove(_) | rocket_git::SubmoduleAction::Deinitialize(_) | rocket_git::SubmoduleAction::Move { .. }) {
                                self.policy.remote_url(p.source().as_bytes())?;
                            }
                            if !p.directory().starts_with(&self.policy.root)
                                || !p.parent().starts_with(&self.policy.root)
                                || p.move_destination().is_some_and(|d| !d.starts_with(&self.policy.root))
                            {
                                return Err("Submodule must stay inside the fixture root".into());
                            }
                            return Ok(());
                        }
                        let p = dialog
                            .submodules
                            .as_ref()
                            .and_then(|s| s.preview.as_ref())
                            .ok_or("Review the submodule first")?;
                        self.policy.remote_url(p.source.as_bytes())?;
                        if !p.entry.directory.starts_with(&self.policy.root)
                            || !p.entry.parent.starts_with(&self.policy.root)
                        {
                            return Err("Submodule must stay inside the fixture root".into());
                        }
                        return Ok(());
                    }
                    DialogKind::NewRepository => {
                        self.policy
                            .init_destination(Path::new(dialog.first.trim()), &dialog.second)?;
                        return Ok(());
                    }
                    DialogKind::NewBranch => {
                        if !dialog.first.starts_with(&format!("{branch}-")) {
                            return Err("New branch must stay in the fixture's marked branch namespace (marker followed by a hyphen).".into());
                        }
                        return Ok(());
                    }
                    // Creates a local branch and switches to it: the same namespace, for the
                    // trimmed name the action uses.
                    DialogKind::TrackBranch { .. } => {
                        if !dialog.first.trim().starts_with(&format!("{branch}-")) {
                            return Err("New branch must stay in the fixture's marked branch namespace (marker followed by a hyphen).".into());
                        }
                        return Ok(());
                    }
                    DialogKind::Worktrees => {
                        let state = dialog
                            .worktrees
                            .as_ref()
                            .ok_or("Worktree dialog is not ready")?;
                        let preview = state
                            .preview
                            .as_ref()
                            .ok_or("Review the worktree operation first")?;
                        if preview.maintenance().is_some() {
                            let entry = preview.entry().ok_or("Missing maintenance entry")?;
                            let branch = entry
                                .branch
                                .as_deref()
                                .and_then(|b| b.strip_prefix(b"refs/heads/"))
                                .and_then(|b| std::str::from_utf8(b).ok())
                                .ok_or("Maintenance fixture requires a local branch")?;
                            self.policy
                                .worktree_destination(&entry.path, branch, &tab.path)?;
                            self.policy
                                .worktree_destination(preview.path(), branch, &tab.path)?;
                        } else if let Some(checkout) = preview.checkout() {
                            self.policy.worktree_destination(
                                preview.path(),
                                checkout.branch(),
                                &tab.path,
                            )?;
                        } else {
                            let (_, branch) = self.policy.repository(preview.path())?;
                            if preview.entry().and_then(|e| e.branch.as_deref())
                                != Some(format!("refs/heads/{branch}").as_bytes())
                            {
                                return Err(
                                    "Selected worktree branch differs from its fixture marker"
                                        .into(),
                                );
                            }
                        }
                        return Ok(());
                    }
                    DialogKind::Ignore(_)
                    | DialogKind::Signing(_)
                    | DialogKind::RepositorySettings
                    | DialogKind::Patch(_)
                    | DialogKind::CommitAction(_)
                    | DialogKind::Rebase(_)
                    | DialogKind::Bisect
                    | DialogKind::Merge
                    | DialogKind::Recovery
                    | DialogKind::UndoCommit
                    | DialogKind::FileEdit(_)
                    | DialogKind::Tags
                    | DialogKind::DeleteBranches
                    | DialogKind::Stash => return Ok(()),
                    DialogKind::MailImport(action) => {
                        if *action == rocket_git::MailImportAction::Start {
                            self.policy.patch_file(&Path::new(&dialog.first).join("0001-review.patch"))?;
                        }
                        return Ok(());
                    }
                    DialogKind::RunCommand(index) => {
                        if !self
                            .app
                            .commands
                            .get(*index)
                            .is_some_and(|c| test_command(&self.app, c))
                        {
                            return Err(COMMAND_REFUSAL.into());
                        }
                        if !test_command_values(&self.app, dialog, *index) { return Err(COMMAND_REFUSAL.into()); }
                        return Ok(());
                    }
                    DialogKind::ConflictReload(_) | DialogKind::CloseTab(_) => return Ok(()),
                    // Local writes only; every branch they name stays in the marked namespace.
                    DialogKind::GitFlow(_) => {
                        self.gitflow_scope(dialog, &tab.path)?;
                        return Ok(());
                    }
                    // The program was checked above; a draft writes nothing.
                    DialogKind::DraftNotice(_) => return Ok(()),
                    DialogKind::Confirm(
                        rocket_ui::backend::Action::ApplyStash { .. }
                        | rocket_ui::backend::Action::SaveConflict { .. }
                        | rocket_ui::backend::Action::KeepConflictSide { .. }
                        | rocket_ui::backend::Action::AbortMerge
                        | rocket_ui::backend::Action::FinishMerge
                        | rocket_ui::backend::Action::DropStash(_),
                        _,
                    ) => return Ok(()),
                    DialogKind::Open => {
                        self.policy.repository(Path::new(&dialog.first))?;
                        return Ok(());
                    }
                    // Reviewed pull application is local and may proceed with an
                    // offline remote. Only its explicit fetch uses transport policy.
                    DialogKind::Pull if target == "dialog-submit" => return Ok(()),
                    DialogKind::Fetch | DialogKind::Pull => {}
                    DialogKind::Push
                        if dialog.branch == branch.as_bytes() && dialog.second == branch => {}
                    DialogKind::Quit => return Ok(()),
                    DialogKind::AddRemote => {
                        self.local_remote_url(dialog.second.as_bytes())?;
                        return Ok(());
                    }
                    DialogKind::EditRemote(remote, kind) => {
                        self.local_remote(remote)?;
                        if !(*kind == rocket_git::RemoteUrlKind::Push && dialog.flag) {
                            self.local_remote_url(dialog.first.as_bytes())?;
                        }
                        return Ok(());
                    }
                    DialogKind::Confirm(rocket_ui::backend::Action::RemoveRemote(remote), _) => {
                        self.local_remote(remote)?;
                        return Ok(());
                    }
                    _ => {
                        return Err(
                            "This dialog is outside the supported fixture operations".into()
                        );
                    }
                }
            }
            // Delivery is preceded by the fresh worker check for pull fetches.
            let remotes = &snapshot.remotes;
            if remotes.len() != 1 || remotes[0].name != b"origin" {
                return Err("Transport tests require one explicit origin remote".into());
            }
            let remote = &remotes[0];
            let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
            if remote.fetch_refspecs != vec![refspec.into_bytes()] {
                return Err(
                    "Origin fetch mapping must select only the owned fixture branch".into(),
                );
            }
            for url in remote.urls.iter().chain(&remote.push_urls) {
                let text =
                    std::str::from_utf8(url).map_err(|_| "Unsupported fixture remote URL")?;
                if !GITHUB_TEST_SOURCES.contains(&text)
                    && !self.policy.github_publish_check().is_some_and(|(repository, _)| {
                        text == format!("git@github.com:{repository}.git")
                    })
                    && !Path::new(text)
                        .canonicalize()
                        .is_ok_and(|p| p.starts_with(&self.policy.root))
                {
                    return Err("Remote is outside the approved local/GitHub test scope".into());
                }
            }
        }
        Ok(())
    }
    // Only immutable values cross to the worker. This binding covers the exact
    // dialog selection, preview and confirmation, not a reusable permission.
    fn validation_binding(&self, request: &Request) -> Value {
        let dialog = self.app.dialog.as_ref();
        json!({
            "tab":self.app.tab().map(|t|t.id),
            "path":self.app.tab().map(|t|&t.path),
            "manager": {"paths":self.app.manager_open_paths().ok(),"include_nested":self.app.manager.include_nested,"form":self.app.manager.form.as_ref().map(|f|json!({"kind":format!("{:?}",f.kind),"path":f.path,"name":f.name,"members":f.members,"active":f.active,"parent":f.parent}))},
            "target":request.args["target"],
            "viewport":self.viewport,
            "owner":dialog.map(|d|d.owner),
            "kind":dialog.map(|d|dialog_name(&d.kind)),
            "first":dialog.map(|d|&d.first),"second":dialog.map(|d|&d.second),
            "branch":dialog.map(|d|&d.branch),"remote":dialog.map(|d|d.remote),
            "remotes":dialog.map(|d|&d.remotes),"flag":dialog.map(|d|d.flag),
            "force_push":dialog.and_then(|d|d.force_push.as_ref()).map(|s|json!({
                "running":s.running(),"confirmation":s.confirmation,
                "preview":s.preview.as_ref().map(|p|json!({"remote":p.remote(),"destination":p.destination(),
                    "local_branch":p.local_branch(),"remote_branch":p.remote_branch(),
                    "source":p.source().as_str(),"expected":p.expected().as_str()}))
            })),
            "atomic_push":dialog.and_then(|d|d.atomic_push.as_ref()).map(atomic_push_state),
            "undo":dialog.and_then(|d|d.undo.as_ref()).map(|s|json!({"loading":s.task.is_some(),"applying":s.applying,"review":s.preview.as_ref().map(|p|p.recovery_reference())})),
            "ignore":dialog.and_then(|d|d.ignore.as_ref()).map(ignore_state),
            "workflow":dialog.and_then(|d|d.workflow.as_ref()).map(workflow_state),
            "remote_tags":dialog.and_then(|d|d.tags.as_ref()).map(|t|remote_tag_validation_binding(&t.remote)),
            "tag_batch":dialog.and_then(|d|d.tags.as_ref()).map(tag_batch_binding),
            "pull":dialog.and_then(|d|d.pull.as_ref()).map(pull_state_value),
        })
    }
    fn validation_spec(&self, request: &Request) -> Result<Option<ValidationSpec>, String> {
        let target = request.args["target"].as_str().unwrap_or("");
        let Some(dialog) = self.app.dialog.as_ref() else {
            return Ok(None);
        };
        let kind = if matches!(target, "dialog-atomic-review" | "dialog-submit") && dialog.atomic_push.is_some() {
            ValidationKind::AtomicPush
        } else if matches!(target, "push-lease-review" | "dialog-submit")
            && dialog.force_push.is_some()
        {
            ValidationKind::ForcePush
        } else if (target == "dialog-tag-publish" && dialog.tags.is_some())
            || (matches!(
                target,
                "dialog-remote-tag-refresh"
                    | "dialog-remote-tag-retry"
                    | "dialog-remote-tag-publish"
                    | "dialog-remote-tag-delete"
                    | "dialog-batch-review"
                    | "dialog-submit"
            ) && dialog.tags.as_ref().is_some_and(|t| t.remote.active))
        {
            ValidationKind::RemoteTag
        } else if target == "dialog-pull-fetch" {
            ValidationKind::Pull
        } else {
            return Ok(None);
        };
        let tab = self.app.tab().ok_or("Open a fixture first")?;
        let (_, branch) = self.policy.repository(&tab.path)?;
        let remote = dialog
            .remotes
            .get(dialog.remote)
            .cloned()
            .ok_or("Select a fixture remote")?;
        Ok(Some(ValidationSpec {
            kind,
            path: tab.path.clone(),
            remote,
            branch,
        }))
    }
    fn cancel_input(&mut self, reason: &'static str) -> bool {
        let Some(pending) = self
            .input
            .as_mut()
            .filter(|p| p.validation.is_some() && !p.delivered)
        else {
            return false;
        };
        pending.refusal.get_or_insert(reason);
        pending.validation.as_ref().unwrap().job.cancel();
        true
    }
    fn request(&mut self, request: Request, ctx: &Context) {
        if request.ticket.inactive() {
            return;
        }
        if Instant::now() > request.ticket.deadline {
            let elapsed = request.ticket.timing.metrics().queue_ms;
            request.ticket.fail(&format!("Request expired before input delivery after {elapsed} ms in the UI queue; no retry was performed. Inspect state before issuing another action. Timing details are in result._meta.rocket_request and the diagnostic log."));
            return;
        }
        request.ticket.timing.mark(Phase::Ui);
        match request.name.as_str() {
            "rocket_review_click" | "rocket_review_resize" | "rocket_review_capture" => {
                let id=request.args["window"].as_u64().unwrap();
                let Some(window)=self.app.reviews.iter().find(|w|w.lock().unwrap_or_else(|e|e.into_inner()).id==id).cloned() else {request.ticket.fail("Review window is closed or unknown");return;};
                let r=window.lock().unwrap_or_else(|e|e.into_inner());
                if r.closed || self.policy.repository(&r.tab.path).is_err() {request.ticket.fail("Review is closed or outside fixture scope");return;}
                let viewport=r.viewport();
                // As for the root in pump_requests: restoring is enough for its UI
                // pass, and the foreground would take the user's typing.
                ctx.send_viewport_cmd_to(viewport,ViewportCommand::Minimized(false));
                match request.name.as_str() {
                    "rocket_review_resize" => {
                        ctx.send_viewport_cmd_to(viewport,ViewportCommand::InnerSize(vec2(request.args["width"].as_u64().unwrap() as f32,request.args["height"].as_u64().unwrap() as f32)));
                        request.ticket.ok(json!({"resize_requested":true}));
                    }
                    "rocket_review_capture" => {
                        if self.review_capture.is_some(){request.ticket.fail("Review capture already pending");return;}
                        self.next_capture+=1;
                        self.review_capture=Some((id,Capture{ticket:request.ticket,id:self.next_capture}));
                        ctx.send_viewport_cmd_to(viewport,ViewportCommand::Screenshot(UserData::new(self.next_capture)));
                    }
                    _ if request.args["target"]=="review-close" => {
                        if self.review_input.is_some()||self.review_close.is_some(){request.ticket.fail("Review input already pending");return;}
                        request.ticket.timing.mark(Phase::Input);
                        ctx.send_viewport_cmd_to(viewport,ViewportCommand::Close);
                        self.review_close=Some((id,request));
                    }
                    _ => {
                        if self.review_input.is_some()||self.review_close.is_some(){request.ticket.fail("Review input already pending");return;}
                        let target=request.args["target"].as_str().unwrap();
                        let Some(control)=r.widgets.get(target).filter(|c|c.enabled && Rect::from_min_size(Pos2::ZERO,vec2(r.size[0],r.size[1])).contains_rect(c.rect)) else {request.ticket.fail("Review control is disabled, clipped or absent");return;};
                        request.ticket.timing.mark(Phase::Input);
                        self.review_input=Some(ReviewInput{request,window:id,phase:0,point:control.rect.center()});
                    }
                }
                ctx.request_repaint_of(viewport);
            }
            "rocket_state" => request.ticket.ok(self.state()),
            "rocket_cancel_input" => {
                let cancelled =
                    self.cancel_input("Input validation cancelled; no input was delivered");
                request
                    .ticket
                    .ok(json!({"cancel_requested":cancelled,"state":self.state()}));
            }
            "rocket_resize" => {
                if !self.scope_ok()
                    || self.app.quitting
                    || self.input.as_ref().is_some_and(|p| p.validation.is_none())
                {
                    request
                        .ticket
                        .fail("Wait for input to settle in the fixture window before resizing");
                    return;
                }
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(
                    request.args["width"].as_u64().unwrap() as f32,
                    request.args["height"].as_u64().unwrap() as f32,
                )));
                request.ticket.ok(json!({"resize_requested":true}));
            }
            "rocket_terminal_enter" | "rocket_terminal_probe" => {
                let mut request = request;
                let target = if request.name == "rocket_terminal_probe" {
                    "terminal-screen"
                } else {
                    "terminal-command"
                };
                request.args["target"] = json!(target);
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                if let Err(error) = self.guard(&request) {
                    request.ticket.fail(&error);
                    return;
                }
                let keys = if request.name == "rocket_terminal_probe" {
                    crate::terminal_probes::keyboard(request.args["probe"].as_str().unwrap())
                        .unwrap()
                } else {
                    vec![key_event(Key::Enter, true), key_event(Key::Enter, false)]
                };
                let pos = self.app.frame_widgets[target].rect.center();
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events: vec![
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        },
                        Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    keys_after_focus: keys,
                    text: None,
                    later: Default::default(),
                    files: Vec::new(),
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_open" => {
                if self.app.busy()
                    || self.input.is_some()
                    || self.app.blocking_dialog()
                    || self.app.quitting
                {
                    request
                        .ticket
                        .fail("Wait for the current UI operation and dismiss its dialog first");
                    return;
                }
                match self
                    .policy
                    .repository(Path::new(request.args["path"].as_str().unwrap()))
                {
                    Ok((path, _)) => {
                        self.app.dispatch(Intent::Open(path));
                        request.ticket.ok(self.state());
                    }
                    Err(error) => request.ticket.fail(&error),
                }
            }
            "rocket_wait" => {
                let until = Instant::now()
                    + Duration::from_millis(request.args["timeout_ms"].as_u64().unwrap_or(10000));
                self.waits.push((request.ticket, until));
            }
            "rocket_scroll" => {
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                let target = request.args["target"].as_str().unwrap();
                if !matches!(
                    target,
                    "changes-scroll"
                        | "history-scroll"
                        | "setup-scroll"
                        | "summary-scroll"
                        | "dialog-undo-scroll"
                        | "dialog-scroll"
                        | "image-scroll"
                        | "image-side-0"
                        | "image-side-1"
                        | "image-composite"
                        | "dialog-pull-scroll"
                        | "dialog-force-push-scroll"
                        | "dialog-atomic-scroll"
                        | "dialog-ignore-scroll"
                        | "dialog-workflow-scroll"
                        | "dialog-stash-apply-scroll"
                        | "dialog-workflow-nav"
                        | "commit-message-scroll"
                        | "commit-link-scroll"
                        | "crash-scroll"
                        | "update-scroll"
                        | "dialog-commit-scroll"
                        | "dialog-rebase-scroll"
                        | "dialog-bisect-scroll"
                        | "dialog-merge-scroll"
                        | "dialog-recovery-scroll"
                        | "dialog-tags-scroll"
                        | "dialog-remote-tag-feedback"
                        | "dialog-file-scroll"
                        | "dialog-patch-dialog-scroll"
                        | "dialog-patch-commits-scroll"
                        | "dialog-file-backups-scroll"
                        | "dialog-worktrees-scroll"
                        | "dialog-submodules-scroll"
                        | "dialog-lfs-scroll"
                        | "page-details-scroll"
                        | "hosting-scroll"
                        | "int-panel-scroll"
                        | "int-play-code-scroll" | "int-play-preview-scroll" | "conflict-ai-scroll"
                        | "int-review-scroll" | "int-package-scroll" | "int-package-code-scroll"
                        | "ssh-scroll"
                        | "ssh-keys-scroll"
                        | "repository-menu-scroll"
                        | "dialog-submodule-list-scroll"
                        | "dialog-worktree-list-scroll"
                        | "dialog-tag-list-scroll"
                        | "file-list-scroll"
                        | "file-source-scroll"
                        | "tree-list-scroll"
                        | "files-results-scroll"
                        | "files-source-scroll"
                        | "map-canvas" | "map-details-scroll" | "map-export-scroll"
                        | "compare-files-scroll"
                        | "review-files-scroll" | "review-diff-scroll" | "review-findings-scroll" | "review-exact-scroll"
                        | "review-page-scroll"
                        | "conflict-ours-scroll"
                        | "conflict-theirs-scroll"
                        | "tree-source-scroll"
                        | "stash-diff-scroll"
                        | "conflict-editor-scroll"
                        | "conflict-whole-scroll"
                        | "tools-scroll"
                        | "prefs-scroll"
                        | "open-list-scroll"
                        | "manager-navigation"
                        | "manager-repositories"
                        | "manager-form-scroll"
                        | "manager-scan-scroll"
                        | "manager-scan-results"
                        | "manager-scan-issues"
                        | "manager-parent-options"
                        | "benchmark-scroll"
                        | "picker-list-scroll"
                        | "activity-list"
                        | "activity-details"
                        | "explore-results-scroll"
                ) {
                    request.ticket.fail(
                        "Scroll target must be a supported visible repository, preview, file, or stash viewport",
                    );
                    return;
                }
                let direction = match request.args["direction"].as_str() {
                    Some("down") => -1.0,
                    Some("up") => 1.0,
                    _ => {
                        request.ticket.fail("Scroll direction must be up or down");
                        return;
                    }
                };
                if let Err(error) = self.guard(&request) {
                    request.ticket.fail(&error);
                    return;
                }
                let pos = self.app.frame_widgets[target].rect.center();
                let pixels = request.args["pixels"].as_u64().unwrap() as f32;
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events: vec![
                        Event::PointerMoved(pos),
                        Event::MouseWheel {
                            unit: MouseWheelUnit::Point,
                            delta: vec2(0.0, direction * pixels),
                            phase: TouchPhase::Move,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    keys_after_focus: Vec::new(),
                    text: None,
                    later: Default::default(),
                    files: Vec::new(),
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_shortcut" => {
                if self.input.is_some() { request.ticket.fail("Previous UI input is still pending"); return; }
                if let Err(error) = self.guard(&request) { request.ticket.fail(&error); return; }
                use rocket_ui::shortcuts::model::{Binding, Trigger, Wheel};
                let binding = Binding::parse(request.args["binding"].as_str().unwrap()).expect("guarded binding");
                let modifiers = binding.egui_modifiers();
                let pos = request.args["target"].as_str().and_then(|id| self.app.frame_widgets.get(id)).map_or(pos2(4.0,4.0), |c| c.rect.center());
                let mut events = vec![Event::ModifiersChanged(modifiers)];
                match binding.trigger {
                    Trigger::Key(key) => for pressed in [true,false] { events.push(Event::Key {key,physical_key:None,pressed,repeat:false,modifiers}); },
                    Trigger::Mouse(button) => { events.push(Event::PointerMoved(pos)); for pressed in [true,false] { events.push(Event::PointerButton {pos,button,pressed,modifiers}); } },
                    Trigger::Wheel(wheel) => { events.push(Event::PointerMoved(pos)); events.push(Event::MouseWheel { unit:MouseWheelUnit::Line, delta:match wheel { Wheel::Up=>vec2(0.0,1.0),Wheel::Down=>vec2(0.0,-1.0),Wheel::Left=>vec2(1.0,0.0),Wheel::Right=>vec2(-1.0,0.0) },modifiers,phase:TouchPhase::Move }); },
                }
                events.push(Event::ModifiersChanged(Modifiers::NONE));
                request.ticket.timing.mark(Phase::Input);
                self.input=Some(Input {request,events,keys_after_focus:Vec::new(),later:Default::default(),files:Vec::new(),text:None,delivered:false,frame:ctx.cumulative_frame_nr(),validation:None,validated:false,refusal:None});
            }
            "rocket_click" | "rocket_type" => {
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                if let Err(error) = self.guard(&request) {
                    request.ticket.fail(&error);
                    return;
                }
                let target = request.args["target"].as_str().unwrap();
                if request.name == "rocket_type"
                    && !self.app.integrations.text_control(target)
                    && !matches!(
                        target,
                        "manager-name" | "manager-path" | "manager-filter" | "manager-scan-root" | "manager-scan-filter"
                            | "commit-link-server" | "commit-link-name"
                            | "dialog-path"
                            | "dialog-merge-target"
                            | "dialog-stash-message"
                            | "dialog-branches-filter"
                            | "dialog-batch-filter"
                            | "git-program" | "git-name" | "git-email"
                            | "dialog-commands-name"
                            | "dialog-commands-program"
                            | "dialog-commands-arguments"
                            | "dialog-commands-group"
                            | "dialog-commands-filter"
                            | "dialog-commands-share-path"
                            | "sidebar-branch-filter"
                            // Only filters the branch switcher; choosing a row is a click.
                            | "branch-switch-filter"
                            | "conflict-result"
                            | "conflict-ai-instructions"
                            | "file-path"
                            | "tree-revision"
                            | "files-search" | "files-search-path" | "commit-search-path"
                            | "map-revision" | "map-search" | "map-base" | "map-path-scope" | "map-export-folder" | "map-export-name" | "crash-export-folder" | "crash-export-name"
                            | "compare-base" | "compare-target" | "compare-filter"
                            | "review-base" | "review-target" | "review-base-pick-filter" | "review-target-pick-filter" | "review-choose-filter"
                            | "compare-base-pick-filter" | "compare-target-pick-filter"
                            | "dialog-file-path"
                            | "open-filter"
                            | "picker-path"
                            | "picker-filter"
                            | "pref-search"
                            | "pref-secret-scan-mib"
                            | "pref-source-folder"
                            | "pref-shortcuts-filter"
                            | "theme-accent-code"
                            | "dialog-legal-filter"
                            // Only filters the palette; running a row is a click.
                            | "palette-input"
                            | "dialog-patch-path"
                            | "dialog-patch-revision"
                            | "dialog-patch-base"
                            | "dialog-file-revision"
                            | "diff-search"
                            | "dialog-amend-message"
                            | "dialog-rebase-target"
                            | "dialog-bisect-good-input"
                            | "dialog-bisect-bad-input"
                            | "dialog-bisect-program"
                            | "dialog-bisect-arguments"
                            | "dialog-bisect-timeout"
                            | "dialog-rebase-message"
                            | "dialog-rebase-exec"
                            | "dialog-recovery-name"
                            | "dialog-tag-name"
                            | "dialog-remote-tag-filter"
                            | "github-filter"
                            | "explore-query"
                            | "ssh-alias" | "ssh-host" | "ssh-user" | "ssh-port" | "ssh-existing" | "ssh-passphrase"
                            | "hosting-label" | "hosting-server" | "hosting-username" | "hosting-token"
                            | "hosting-repository" | "hosting-source" | "hosting-target" | "hosting-title"
                            | "hosting-body" | "hosting-filter" | "hosting-work-state" | "hosting-review-body"
                            | "hosting-source-repository" | "hosting-inline-body"
                            | "hosting-thread-body" | "hosting-inline-start" | "hosting-inline-end"
                            | "hosting-search-text" | "hosting-search-author" | "hosting-search-assignee"
                            | "dialog-series-folder" | "dialog-tags-filter"
                            | "hosting-publish-owner" | "hosting-publish-name" | "hosting-publish-description"
                            | "hosting-publish-branch" | "hosting-publish-remote"
                            | "dialog-remote-tag-confirm"
                            | "dialog-worktree-path"
                            | "dialog-submodule-source"
                            | "dialog-submodule-path"
                            | "dialog-lfs-pattern"
                            | "dialog-lfs-lock-path"
                            | "dialog-worktree-branch"
                            | "dialog-worktree-target"
                            | "dialog-tag-target"
                            | "dialog-tag-message"
                            | "dialog-init-path"
                            | "dialog-init-branch"
                            | "dialog-sign-key"
                            | "dialog-sign-allowed"
                            | "dialog-identity-name"
                            | "dialog-identity-email"
                            | "push-destination"
                            | "push-lease-confirm"
                            | "dialog-ignore-text"
                            | "dialog-first"
                            | "dialog-ignore-path"
                            | "commit-message"
                            | "commit-description"
                            | "workflow-message"
                            | "terminal-command"
                            | "commit-filter"
                            | "dialog-remote-name"
                            | "dialog-remote-url"
                            | "mcp-port"
                            | "mcp-root"
                            | "tools-executable"
                            | "tools-arguments"
                            | "tools-profile-name"
                            | "activity-search"
                            | "dialog-clone-source"
                            | "dialog-issue-sample"
                            | "dialog-gitflow-master"
                            | "dialog-gitflow-develop"
                            | "dialog-gitflow-feature"
                            | "dialog-gitflow-release"
                            | "dialog-gitflow-hotfix"
                            | "dialog-gitflow-support"
                            | "dialog-gitflow-version-tag"
                            | "dialog-gitflow-name"
                            | "dialog-gitflow-base"
                            | "dialog-gitflow-message"
                            | "pref-ai-executable"
                            | "pref-ai-language"
                    )
                    && !command_input_editable(target)
                    && ![
                        "dialog-atomic-destination-",
                        "dialog-issue-name-",
                        "dialog-issue-pattern-",
                        "dialog-issue-url-",
                    ]
                    .iter()
                    .any(|prefix| {
                        target.strip_prefix(prefix).is_some_and(|n| {
                            let max_digits = if *prefix == "dialog-atomic-destination-" { 6 } else { 2 };
                            !n.is_empty() && n.len() <= max_digits && n.bytes().all(|b| b.is_ascii_digit())
                        })
                    })
                {
                    request.ticket.fail("Control is not an editable field");
                    return;
                }
                // Ctrl/Shift-click extends a history-row selection (`commit-<full id>`);
                // nothing else takes modifiers, so other targets refuse them.
                let commit_row = target.strip_prefix("commit-").is_some_and(|id| {
                    matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit())
                });
                let modifiers = match request.args["modifiers"].as_str() {
                    None => Modifiers::NONE,
                    Some(_) if !commit_row && !change_row_target(target) => {
                        request
                            .ticket
                            .fail("Click modifiers apply only to commit or changed-file rows");
                        return;
                    }
                    Some("ctrl") => Modifiers::CTRL | Modifiers::COMMAND,
                    Some("shift") => Modifiers::SHIFT,
                    Some(_) => {
                        request.ticket.fail("Click modifiers must be ctrl or shift");
                        return;
                    }
                };
                let text = (request.name == "rocket_type")
                    .then(|| request.args["text"].as_str().unwrap().to_owned());
                let pos = self.app.frame_widgets[target].rect.center();
                let button = if request.args["button"].as_str()==Some("right") { PointerButton::Secondary } else { PointerButton::Primary };
                // A slider gets its release a frame after the press; see click_events.
                let slider = request.name == "rocket_click" && slider_target(target);
                let (mut events, later) = click_events(pos, button, modifiers, slider);
                // egui reads held modifiers from these events; release them in the
                // next frame so the click itself still sees them.
                let mut keys_after_focus = Vec::new();
                if modifiers != Modifiers::NONE {
                    events.insert(0, Event::ModifiersChanged(modifiers));
                    keys_after_focus.push(Event::ModifiersChanged(Modifiers::NONE));
                }
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events,
                    keys_after_focus,
                    text,
                    later,
                    files: Vec::new(),
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_drop_files" => {
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                if let Err(error) = self.drop_guard(&request) {
                    request.ticket.fail(&error);
                    return;
                }
                let files = request.args["paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str().map(PathBuf::from))
                    .collect();
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events: Vec::new(),
                    keys_after_focus: Vec::new(),
                    later: Default::default(),
                    files,
                    text: None,
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_drag" => {
                let mut request = request;
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                // Branch/commit drops open reviewed menus. File drops use the same
                // fixture and checkout guard as the existing Stage/Unstage buttons.
                let source = request.args["source"].as_str().unwrap().to_owned();
                let target = request.args["target"].as_str().unwrap().to_owned();
                let commit = |id: &str| {
                    id.strip_prefix("commit-").is_some_and(|id| {
                        matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                };
                let branch = |id: &str| id.starts_with("sidebar-branch-");
                let file_drop = change_row_target(&source) && matches!(target.as_str(), "change-drop-staged" | "change-drop-unstaged");
                if source == target
                    || !((branch(&source) && branch(&target))
                        || (commit(&source) && commit(&target)) || file_drop)
                {
                    request.ticket.fail("Drag branch-to-branch, commit-to-commit, or a changed file row onto a Staged/Unstaged drop target");
                    return;
                }
                // Both ends pass the click guard: fixture scope, active tab, an enabled
                // visible control and no blocking dialog.
                request.args["target"] = json!(source);
                let start = self.guard(&request);
                request.args["target"] = json!(target);
                if let Err(error) = start.and_then(|_| self.guard(&request)) {
                    request.ticket.fail(&error);
                    return;
                }
                let from = self.app.frame_widgets[&source].rect.center();
                let to = self.app.frame_widgets[&target].rect.center();
                let button = |pos, pressed| Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                let mut later: std::collections::VecDeque<Vec<Event>> = (1..=4)
                    .map(|step| {
                        vec![Event::PointerMoved(
                            from + (to - from) * (step as f32 / 4.0),
                        )]
                    })
                    .collect();
                if request.args["cancel"].as_bool() == Some(true) {
                    later.push_back(vec![key_event(Key::Escape, true), key_event(Key::Escape, false)]);
                }
                later.push_back(vec![button(to, false)]);
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events: vec![Event::PointerMoved(from), button(from, true)],
                    keys_after_focus: Vec::new(),
                    later,
                    files: Vec::new(),
                    text: None,
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_escape" => {
                if self.cancel_input("Input validation cancelled; no input was delivered") {
                    request
                        .ticket
                        .ok(json!({"cancel_requested":true,"state":self.state()}));
                    return;
                }
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                request.ticket.timing.mark(Phase::Input);
                self.input = Some(Input {
                    request,
                    events: vec![key_event(Key::Escape, true), key_event(Key::Escape, false)],
                    keys_after_focus: Vec::new(),
                    text: None,
                    later: Default::default(),
                    files: Vec::new(),
                    delivered: false,
                    frame: ctx.cumulative_frame_nr(),
                    validation: None,
                    validated: false,
                    refusal: None,
                });
            }
            "rocket_capture" => {
                if !self.scope_ok() {
                    request
                        .ticket
                        .fail("Capture refused outside the fixture scope");
                    return;
                }
                if self.capture.is_some() {
                    request.ticket.fail("A capture is already pending");
                    return;
                }
                self.next_capture += 1;
                self.capture = Some(Capture {
                    ticket: request.ticket,
                    id: self.next_capture,
                });
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(
                    self.next_capture,
                )));
            }
            "rocket_logs" => match self.logs(
                request.args["after_seq"].as_u64().unwrap_or(0),
                request.args["limit"].as_u64().unwrap_or(100) as usize,
            ) {
                Ok(value) => request.ticket.ok(value),
                Err(error) => request.ticket.fail(&error),
            },
            "rocket_close" => {
                if self.input.is_some() {
                    request.ticket.fail("Previous UI input is still pending");
                    return;
                }
                self.app.request_close();
                request.ticket.ok(json!({"close_requested":true}));
            }
            _ => request.ticket.fail("Unknown UI request"),
        }
    }
    fn logs(&self, after: u64, limit: usize) -> Result<Value, String> {
        let session = self
            .app
            .backend
            .as_ref()
            .ok_or("Diagnostic session is not ready")?
            .log
            .session_id();
        diagnostics::read(&self.app.log_path, session, after, limit)
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        self.app.begin_quit();
        if let Some(request) = self.deferred.take() {
            request.ticket.fail("Rocket closed before rendering the requested control");
        }
        if let Some(input) = self.input.take() {
            input.request.ticket.fail("Rocket window closed before input acknowledgement; inspect repository state before retrying");
        }
        if let Some(capture) = self.capture.take() {
            capture
                .ticket
                .fail("Rocket window closed before capture completed");
        }
        if let Some(input) = self.review_input.take() {
            input.request.ticket.fail("Rocket closed before review input acknowledgement");
        }
        if let Some((_, capture)) = self.review_capture.take() {
            capture.ticket.fail("Rocket closed before review capture completed");
        }
        for (ticket, _) in self.waits.drain(..) {
            ticket.fail("Rocket window closed while waiting");
        }
        // The host drains queued requests after native teardown under the
        // transition lock. In particular, a queued state read must observe the
        // closed window rather than fail while this destructor is running.
    }
}
impl Ui {
    fn deliver_raw_input(&mut self, ctx: &Context, input: &mut RawInput) {
        if input.viewport_id != ViewportId::ROOT {
            // Independent viewport input remains live while the parent is occluded.
            self.pump_requests(ctx);
            if let Some(mut pending)=self.review_input.take() {
                let window=self.app.reviews.iter().find(|w|w.lock().unwrap_or_else(|e|e.into_inner()).id==pending.window).cloned();
                let Some(window)=window else{pending.request.ticket.fail("Review closed before input");return;};
                let r=window.lock().unwrap_or_else(|e|e.into_inner());
                if input.viewport_id!=r.viewport(){self.review_input=Some(pending);return;}
                // The release frame can close this viewport before the root
                // polls. Acknowledge that completed click, not a stale target.
                if r.closed && pending.phase==2 && pending.request.args["target"]=="review-close" {
                    pending.request.ticket.ok(json!({"input_applied":true,"review_closed":true}));return;
                }
                if r.closed || self.policy.repository(&r.tab.path).is_err() || pending.request.ticket.inactive() || Instant::now()>pending.request.ticket.deadline {
                    input.events.push(Event::PointerButton{pos:pending.point,button:PointerButton::Primary,pressed:false,modifiers:Modifiers::NONE});
                    pending.request.ticket.fail("Review input cancelled or no longer in fixture scope");return;
                }
                if pending.phase==2 {
                    drop(r);pending.request.ticket.ok(json!({"input_applied":true,"state":self.state()}));return;
                }
                if pending.phase==0 {
                    let target=pending.request.args["target"].as_str().unwrap();
                    let Some(c)=r.widgets.get(target).filter(|c|c.enabled && Rect::from_min_size(Pos2::ZERO,vec2(r.size[0],r.size[1])).contains_rect(c.rect)) else{pending.request.ticket.fail("Review control changed before input");return;};
                    pending.point=c.rect.center();
                }
                prepare_fixture_input(input);
                input.events.push(Event::PointerMoved(pending.point));
                input.events.push(Event::PointerButton{pos:pending.point,button:PointerButton::Primary,pressed:pending.phase==0,modifiers:Modifiers::NONE});
                if pending.phase==1 {
                    pending.request.ticket.timing.mark(Phase::Delivered);
                    ctx.request_repaint_of(ViewportId::ROOT);
                }
                pending.phase+=1;ctx.request_repaint_of(r.viewport());self.review_input=Some(pending);
            }
            return;
        }
        let Some(mut pending) = self.input.take() else {
            return;
        };
        // A modifier click that ends early must not leave Ctrl/Shift held, nor a
        // slider click the pointer pressed.
        let release = |pending: &mut Input, input: &mut RawInput| {
            input.events.extend(
                pending
                    .keys_after_focus
                    .drain(..)
                    .filter(|e| matches!(e, Event::ModifiersChanged(_))),
            );
            if pending.holds_pointer() {
                input.events.extend(pending.later.drain(..).flatten());
            }
        };
        if let Some(validation) = pending.validation.as_mut() {
            // Human input may change selection or close a dialog while the
            // worker runs. It takes precedence; never replay the old click.
            let shown = validation.ready_at.is_none() && validation.job.started.elapsed() >= CHECK_SHOWS_AFTER;
            let interruption = validation_interruption(input, shown);
            if pending.request.ticket.inactive() || self.app.quitting {
                pending
                    .refusal
                    .get_or_insert("Input validation cancelled; no input was delivered");
            } else if Instant::now() > pending.request.ticket.deadline {
                pending.refusal.get_or_insert("Input validation exceeded its 20-second deadline; no input was delivered. Inspect diagnostics before retrying");
            } else if let Some(reason) = interruption {
                pending.refusal.get_or_insert(reason);
            } else if self.validation_binding(&pending.request) != validation.binding {
                pending.refusal.get_or_insert("UI selection changed during validation; no input was delivered. Read state and review again");
            } else if validation
                .ready_at
                .is_some_and(|t| t.elapsed() > Duration::from_secs(1))
            {
                pending.refusal.get_or_insert(
                    "Input validation became stale before delivery; read state and review again",
                );
            }
            if pending.refusal.is_some() {
                validation.job.cancel();
            }
            let Some(result) = (if validation.ready_at.is_some() {
                Some(Ok(()))
            } else {
                validation.job.poll()
            }) else {
                self.input = Some(pending);
                ctx.request_repaint_after(Duration::from_millis(16));
                return;
            };
            let result = pending.refusal.map_or(result, |reason| Err(reason.into()));
            if let Err(error) = result {
                release(&mut pending, input);
                self.app.notice = Some(dropped_input_notice(&error));
                pending.request.ticket.fail(&error);
                return;
            }
            if validation.ready_at.is_none() {
                // Retire the validation modal for a complete frame before the
                // click: egui hit-tests against the preceding frame's layers.
                validation.ready_at = Some(Instant::now());
                self.input = Some(pending);
                ctx.request_repaint();
                return;
            }
            let target = pending.request.args["target"].as_str().unwrap_or("");
            let Some(control) = self.app.frame_widgets.get(target).filter(|c| c.enabled) else {
                // egui may need another layout pass after the top modal retires.
                // Keep the exact binding/deadline checks above while it settles.
                self.input = Some(pending);
                ctx.request_repaint();
                return;
            };
            // Requests name a logical control, not a screen coordinate. Never
            // reuse its pre-validation position after the layout has settled.
            for event in &mut pending.events {
                match event {
                    Event::PointerMoved(pos) | Event::PointerButton { pos, .. } => {
                        *pos = control.rect.center()
                    }
                    _ => {}
                }
            }
            pending.validation = None;
            pending.request.ticket.timing.mark(Phase::Input);
            pending.validated = true;
        }
        if pending.request.ticket.inactive() {
            release(&mut pending, input);
            return;
        }
        if Instant::now() > pending.request.ticket.deadline {
            release(&mut pending, input);
            pending
                .request
                .ticket
                .fail("UI input expired; inspect state before retrying");
            return;
        }
        if pending.delivered {
            self.input = Some(pending);
            return;
        }
        // After a modifier click, only releasing the modifier remains, and after a
        // slider press only its pointer release. That is always safe, and the click
        // may itself have started work the guard would report as busy.
        let releasing = pending.events.is_empty()
            && pending.text.is_none()
            && (!pending.keys_after_focus.is_empty() || pending.holds_pointer())
            && pending
                .keys_after_focus
                .iter()
                .all(|e| matches!(e, Event::ModifiersChanged(_)));
        if pending.request.name != "rocket_escape"
            && !releasing
            && let Err(error) = if pending.request.name == "rocket_drop_files" {
                self.drop_guard(&pending.request)
            } else {
                self.guard(&pending.request)
            }
        {
            release(&mut pending, input);
            pending.request.ticket.fail(&error);
            return;
        }
        if !pending.validated && !releasing {
            match self.validation_spec(&pending.request) {
                Ok(Some(spec)) => {
                    let Some(backend) = self.app.backend.as_ref() else {
                        pending
                            .request
                            .ticket
                            .fail("Backend unavailable; no input was delivered");
                        return;
                    };
                    let Some(repository) = self
                        .app
                        .tab()
                        .and_then(|t| t.snapshot.as_ref())
                        .map(|s| s.repository.clone())
                    else {
                        pending
                            .request
                            .ticket
                            .fail("Repository is still loading; no input was delivered");
                        return;
                    };
                    let binding = self.validation_binding(&pending.request);
                    match Job::start(
                        spec,
                        self.policy.clone(),
                        repository,
                        backend.log.clone(),
                        pending.request.ticket.deadline,
                    ) {
                        Ok(job) => {
                            pending.request.ticket.timing.mark(Phase::Validation);
                            pending.validation = Some(PendingValidation {
                                job,
                                binding,
                                ready_at: None,
                            });
                            self.input = Some(pending);
                            ctx.request_repaint();
                        }
                        Err(error) => pending.request.ticket.fail(&error),
                    }
                    return;
                }
                Err(error) => {
                    pending.request.ticket.fail(&error);
                    return;
                }
                Ok(None) => {}
            }
        }
        // A bounded policy read can outlive the ticket while checking configuration.
        // Never deliver the click after the caller timed out or disconnected.
        if pending.request.ticket.inactive() || Instant::now() > pending.request.ticket.deadline {
            release(&mut pending, input);
            pending
                .request
                .ticket
                .fail("Input expired during validation; no input was delivered");
            return;
        }
        if pending.request.name == "rocket_click" && !releasing {
            let target = pending.request.args["target"].as_str().unwrap_or("");
            refresh_click_position(target, self.app.frame_widgets[target].rect, &mut pending.events);
        }
        prepare_fixture_input(input);
        pending.request.ticket.timing.mark(Phase::Delivered);
        if !pending.files.is_empty() {
            // Dropped files arrive with one frame's input, as from File Explorer.
            input.dropped_files.extend(
                pending
                    .files
                    .drain(..)
                    .map(|path| Arc::new(FixtureFile(path)) as DroppedFileHandle),
            );
            pending.delivered = true;
        } else if !pending.events.is_empty() {
            input.events.append(&mut pending.events);
            pending.delivered = pending.text.is_none()
                && pending.keys_after_focus.is_empty()
                && pending.later.is_empty();
        } else if let Some(mut next) = pending.later.pop_front() {
            // A drag: egui hit-tests the press where the pointer is, so the press,
            // the moves and the release each get a frame of their own.
            input.events.append(&mut next);
            pending.delivered = pending.later.is_empty()
                && pending.text.is_none()
                && pending.keys_after_focus.is_empty();
        } else if !pending.keys_after_focus.is_empty() {
            if pending.request.name == "rocket_terminal_probe"
                && !self.app.terminal_screen_focused()
            {
                pending
                    .request
                    .ticket
                    .fail("Terminal screen did not receive keyboard focus; no keys were sent");
                return;
            }
            // Focus/click is rendered in its own frame before keyboard delivery,
            // just like replacement text. Never depend on screenshot timing.
            input.events.append(&mut pending.keys_after_focus);
            pending.delivered = true;
        } else if let Some(text) = pending.text.take() {
            input.events.extend(replacement_events(text));
            pending.delivered = true;
        }
        pending.frame = ctx.cumulative_frame_nr();
        self.input = Some(pending);
    }
}
impl App for Ui {
    fn raw_input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        self.deliver_raw_input(ctx, input);
        self.app.shortcut_raw_input(ctx, input);
    }
    fn logic(&mut self, ctx: &Context, frame: &mut Frame) {

        self.poll_review_requests(ctx);

        if self.shared.stop.load(Ordering::Acquire) && !self.app.quitting {
            self.app.begin_quit();
        }
        // Automatic fetch contacts every remote of every open repository, so it
        // needs the runner's marker and only local fixture remotes in each snapshot.
        self.app.auto_fetch.blocked =
            self.app.auto_fetch.test_interval.is_none() || !self.local_remotes_only();
        self.app.logic(ctx, frame);

        // Backend discovery finishes asynchronously. Ui::new normally sees None;
        // attach the logger once it is ready so later tickets reach the viewer.
        if let Some(backend) = &self.app.backend {
            let mut log = self.shared.diagnostics.lock().unwrap();
            if log.is_none() {
                *log = Some(backend.log.clone());
            }
        }
        self.pump_requests(ctx);
        ctx.request_repaint_after(Duration::from_millis(30));

    }
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut Frame) {

        let ctx = ui.ctx().clone();
        let size = ctx.content_rect().size();
        self.viewport = [size.x, size.y];
        self.app.ui(ui, frame);

        if let Some(validation) = self
            .input
            .as_ref()
            .and_then(|p| p.validation.as_ref())
            .filter(|v| v.ready_at.is_none() && v.job.started.elapsed() >= CHECK_SHOWS_AFTER)
        {
            let (kind, steps): (&str, &[&str]) = match validation.job.spec.kind {
                ValidationKind::ForcePush => ("Force push", &CHECKS),
                ValidationKind::AtomicPush => ("Push branches together", &CHECKS),
                ValidationKind::RemoteTag => ("Remote tag", &CHECKS),
                ValidationKind::Pull => ("Pull", &PULL_CHECKS),
            };
            let label = match validation.job.step() {
                "read_configuration" => CHECKS[1],
                "validate_remotes" => CHECKS[2],
                "resolve_destination" => CHECKS[3],
                "recheck_configuration" => CHECKS[4],
                _ => CHECKS[0],
            };
            let step = steps.iter().position(|s| *s == label).unwrap_or(0);
            let stopping = validation.job.cancelling();
            if rocket_ui::pages::input_check(&ctx, kind, steps, step, stopping) {
                self.cancel_input("Input validation cancelled; no input was delivered");
            }
        }
        // egui can run several layout passes in one frame. Advance input and
        // capture state only once, so no command is accidentally repeated.
        let nr = ctx.cumulative_frame_nr();
        if self.frame == Some(nr) {
            return;
        }
        self.frame = Some(nr);
        if let Some(ticket) = self.opening.take() {
            ticket.ok(self.state());
        }
        if self
            .input
            .as_ref()
            .is_some_and(|p| p.delivered && nr >= p.frame)
        {
            let input = self.input.take().unwrap();
            input
                .request
                .ticket
                .ok(json!({"input_applied":true,"state":self.state()}));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot {
                image, user_data, ..
            } = event
                && self.capture.as_ref().is_some_and(|c| {
                    user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<u64>())
                        == Some(&c.id)
                })
            {
                let capture = self.capture.take().unwrap();
                if !self.scope_ok() {
                    capture.ticket.fail("Capture refused outside fixture scope");
                } else {
                    match capture::png(&image) {
                    Ok(bytes) => capture.ticket.result(json!({"content":[{"type":"image","mimeType":"image/png","data":capture::base64(&bytes)}],"isError":false})),
                    Err(error) => capture.ticket.fail(&error),
                }
                }
            }
        }
        if self
            .capture
            .as_ref()
            .is_some_and(|c| c.ticket.inactive() || Instant::now() > c.ticket.deadline)
        {
            self.capture
                .take()
                .unwrap()
                .ticket
                .fail("Native capture timed out; no input was retried");
        }
        let waits = std::mem::take(&mut self.waits);
        for (ticket, until) in waits {
            if ticket.inactive() {
                continue;
            }
            if (!self.app.busy() && self.input.is_none()) || Instant::now() >= until {
                ticket.ok(self.state());
            } else {
                self.waits.push((ticket, until));
            }
        }
        if let Some(request) = self.deferred.take() {
            self.request(request, &ctx);
        }
    }
}
/// A quick check shows nothing, so a click doesn't flash a dialog.
const CHECK_SHOWS_AFTER: Duration = Duration::from_secs(1);
const CHECKS: [&str; 5] = [
    "Test repository is marked",
    "Git configuration",
    "Remote is in scope",
    "Where it really goes",
    "Configuration unchanged",
];
const PULL_CHECKS: [&str; 4] = [CHECKS[0], CHECKS[1], CHECKS[2], CHECKS[4]];

/// What the status bar says when an input check drops a click. MCP clients get the full
/// error text.
fn dropped_input_notice(error: &str) -> String {
    if error.contains("New user input") {
        "You used the window, so the MCP click was dropped. Nothing ran.".into()
    } else if error.contains("20-second deadline") {
        "The MCP check took over 20 s, so the click was dropped. Nothing ran.".into()
    } else if error.contains("became stale") || error.contains("selection changed") {
        "The screen changed during the MCP check, so the click was dropped.".into()
    } else if error.contains("Input validation cancelled") {
        "MCP click stopped. Nothing ran.".into()
    } else {
        error.to_owned()
    }
}

/// `shown`: the dialog is up, so a click can only reach its Stop.
fn validation_interruption(input: &mut RawInput, shown: bool) -> Option<&'static str> {
    if input.events.iter().any(|e| {
        matches!(
            e,
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            }
        )
    }) {
        // Cancel only the check, leaving the underlying review available.
        input.events.retain(|e| {
            !matches!(
                e,
                Event::Key {
                    key: Key::Escape,
                    ..
                }
            )
        });
        Some("Input validation cancelled; no input was delivered")
    } else if input.events.iter().any(|e| {
        matches!(
            e,
            Event::Key { pressed: true, .. }
                | Event::Text(_)
                | Event::Paste(_)
                | Event::MouseWheel { .. }
        ) || (!shown && matches!(e, Event::PointerButton { pressed: true, .. }))
    }) {
        Some("New user input cancelled validation; no automated input was delivered")
    } else {
        None
    }
}
// Bounded recent cell metadata makes column alignment inspectable without
// turning terminal contents into diagnostic logs. Negative rows are scrollback.
fn terminal_grid(screen: &rocket_terminal::Screen) -> (Vec<Value>, bool) {
    let mut cells = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    for (offset, row) in screen
        .cells
        .iter()
        .rev()
        .chain(screen.scrollback.iter().rev())
        .enumerate()
        .filter(|(_, row)| {
            row.iter()
                .any(|cell| cell.width != 0 && (cell.ch != ' ' || !cell.suffix.is_empty()))
        })
        .take(12)
    {
        for (column, cell) in row.iter().enumerate().rev() {
            if cell.width == 0 || (cell.ch == ' ' && cell.suffix.is_empty()) {
                continue;
            }
            let text = cell.text();
            if cells.len() == 512 || bytes + text.len() > 8192 {
                truncated = true;
                break;
            }
            bytes += text.len();
            cells.push(json!({"row":screen.rows as isize - 1 - offset as isize,
                "column":column,"width":cell.width,"text":text}));
        }
        if truncated {
            break;
        }
    }
    cells.reverse();
    (cells, truncated)
}

/// The fixed probes for the shell an MCP window runs: Windows PowerShell, or on Linux
/// Rocket's bash, whose probes print the same markers.
#[cfg(not(target_os = "linux"))]
const SHELL_PROBES: &[&str] = &[
    "",
    "Write-Output ('ROCKET_' + 'TERMINAL_OK')",
    "$global:RocketProbe = 42",
    "Write-Output ('ROCKET_VALUE_' + $global:RocketProbe)",
    "(Get-Location).Path",
    "[Console]::WriteLine([char]0x00E9)",
    "Start-Sleep -Seconds 30",
    "Write-Output ('AFTER_' + 'INTERRUPT')",
    "1..1200 | ForEach-Object { [Console]::WriteLine('ROCKET_OUTPUT_BOUND') }",
    "Write-Error 'ROCKET_EXPECTED_ERROR'",
    "exit",
];
#[cfg(target_os = "linux")]
const SHELL_PROBES: &[&str] = &[
    "",
    "printf 'ROCKET_%s\\n' TERMINAL_OK",
    "RocketProbe=42",
    "printf 'ROCKET_VALUE_%s\\n' \"$RocketProbe\"",
    "pwd",
    "printf '\\303\\251\\n'",
    "sleep 30",
    "printf 'AFTER_%s\\n' INTERRUPT",
    "for n in $(seq 1200); do echo ROCKET_OUTPUT_BOUND; done",
    "printf 'ROCKET_EXPECTED_%s\\n' ERROR >&2",
    "exit",
];

// The native terminal is interactive. MCP intentionally offers only fixed,
// synthetic acceptance probes, not a general shell/evaluation interface.
fn terminal_probe_allowed(text: &str, root: &Path) -> Result<(), String> {
    if crate::terminal_probes::allowed(text) || SHELL_PROBES.contains(&text) {
        return Ok(());
    }
    let file = root.join("rocket-terminal-fixture.txt");
    let path = file.to_string_lossy();
    #[cfg(not(target_os = "linux"))]
    let path = path
        .strip_prefix("\\\\?\\")
        .unwrap_or(&path)
        .replace('\'', "''");
    #[cfg(not(target_os = "linux"))]
    let expected = format!("[IO.File]::WriteAllText('{path}', 'ROCKET_TERMINAL_EDIT')");
    #[cfg(target_os = "linux")]
    let path = path.replace('\'', "'\\''");
    #[cfg(target_os = "linux")]
    let expected = format!("printf %s ROCKET_TERMINAL_EDIT > '{path}'");
    if text == expected {
        if let Ok(metadata) = std::fs::symlink_metadata(&file) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("Probe target is not a regular fixture file".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("Probe target is a reparse point".into());
                }
            }
        }
        return Ok(());
    }
    Err("MCP terminal input is limited to the fixed synthetic probes described in tools/README.md; arbitrary shell commands are not supported.".into())
}

fn bisect_auto_state(s:&rocket_ui::bisect::automatic::State)->Value {
    let p=s.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    json!({"enabled":s.enabled,"program":s.program,"arguments":s.arguments,"timeout":s.seconds,
        "reviewed":s.preview.is_some(),"running":p.running.as_ref().map(|id|id.as_str()),
        "results":p.results.iter().map(|r|json!({"commit":r.commit.as_str(),"outcome":r.outcome.label(),"exit":r.exit_code,"elapsed_ms":r.elapsed_ms,"recorded":r.recorded,"stdout_bytes":r.stdout.len(),"stderr_bytes":r.stderr.len(),"truncated":r.truncated})).collect::<Vec<_>>()})
}
fn bisect_control_allowed(target: &str, dialog: Option<&DialogKind>) -> bool {
    match target {
        "dialog-submit"
        | "dialog-preview-refresh"
        | "dialog-bisect-scroll"
        | "dialog-bisect-good"
        | "dialog-bisect-bad"
        | "dialog-bisect-skip"
        | "dialog-bisect-resume"
        | "dialog-bisect-restore"
        | "dialog-bisect-auto"
        | "dialog-bisect-program"
        | "dialog-bisect-arguments"
        | "dialog-bisect-timeout"
        | "dialog-bisect-cancel-read" => matches!(dialog, Some(DialogKind::Bisect)),
        "operation-bisect"
        | "repository-bisect"
        | "menu-repository"
        | "repository-menu-scroll"
        // Repository → History & Recovery, the submenu that holds Bisect.
        | "repository-history-menu"
        | "view-changes"
        | "view-commits"
        | "refresh"
        | "toolbar-console"
        | "console-close"
        | "drawer-collapse"
        | "drawer-expand" => true,
        _ => false,
    }
}

/// Planned rebase commands automation may run: shell built-ins with a fixed exit
/// status and no other effect. People can plan any command in the real UI.
const REBASE_TEST_COMMANDS: [&str; 2] = ["true", "false"];
/// Under MCP a custom command may only be one of these Git invocations (one
/// argument per line), run with the Git executable Rocket itself uses.
const COMMAND_TEST_ARGUMENTS: [&str; 7] = [
    "--version",
    "rev-parse\nHEAD",
    "rev-parse\n--verify\n{branch}",
    "log\n-1\n--format=%H\n{commit}",
    "ls-files\n--\n{file}",
    "-C\n{submodule}\nstatus\n--short",
    "rev-parse\n--verify\nrefs/heads/rocket-missing",
];
const COMMAND_REFUSAL: &str = "Automation configures and runs only the fixed Git test commands in UI.md, with the Git executable Rocket uses";
const COMMAND_FORM_TEST_ARGUMENTS: &str = "log\n-1\n{input:compact}\n--format={input:format}\n{input:base}\n--\n{input:path}";
const COMMAND_TEST_URL: &str = "https://example.invalid/rocket-command?commit={commit}&file={path}";
fn change_row_target(target: &str) -> bool {
    target.strip_prefix("change-staged-")
        .or_else(|| target.strip_prefix("change-unstaged-"))
        .is_some_and(|path| !path.is_empty() && path.len() % 2 == 0 && path.bytes().all(|b|b.is_ascii_hexdigit()))
}

fn command_input_editable(target: &str) -> bool {
    if let Some(key) = target.strip_prefix("dialog-command-input-") {
        return !key.is_empty() && key.len() <= 32
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    }
    target.strip_prefix("dialog-commands-input-")
        .and_then(|part| part.split_once('-'))
        .is_some_and(|(index, field)| index.len() == 1 && matches!(index.as_bytes()[0], b'0'..=b'7')
            && matches!(field, "key" | "label" | "default" | "checked"))
}
fn test_command(app: &Rocket, command: &rocket_git::CustomCommand) -> bool {
    if command.action == rocket_git::CommandAction::Url {
        return app.opened_urls.is_some() && command.program==COMMAND_TEST_URL
            && command.arguments.is_empty() && command.inputs.is_empty() && command.validate().is_ok();
    }
    app.backend
        .as_ref()
        .is_some_and(|b| Path::new(&command.program) == b.executable)
        && (COMMAND_TEST_ARGUMENTS.contains(&command.argument_lines().join("\n").as_str())
            || command.argument_lines().join("\n") == COMMAND_FORM_TEST_ARGUMENTS)
}
fn test_command_values(app: &Rocket, dialog: &rocket_ui::app::Dialog, index: usize) -> bool {
    let Some(c) = app.commands.get(index) else { return false };
    if c.argument_lines().join("\n") != COMMAND_FORM_TEST_ARGUMENTS { return c.inputs.is_empty(); }
    let Some((_, _, args)) = rocket_ui::custom_commands::command_line(&app.commands, &app.tabs, dialog.owner, index, &dialog.command_values, dialog.command_context.as_ref()) else { return false };
    let mut args: Vec<_> = args.iter().map(|s|s.to_string_lossy().into_owned()).collect();
    if args.get(2).is_some_and(|s|s=="--oneline") { args.remove(2); }
    args.len()==6 && args[0]=="log" && args[1]=="-1"
        && matches!(args[2].as_str(), "--format=%H" | "--format=%s" | "--format=%H 文")
        && (args[3]=="HEAD" || args[3].starts_with("refs/heads/")) && !args[3].chars().any(char::is_control)
        && args[4]=="--"
}
const REBASE_COMMAND_REFUSAL: &str =
    "Automation runs only the fixed planned rebase commands in UI.md (true or false)";

/// Every command in the plan being edited, or preserved by a stopped rebase, must
/// be a fixed test command before automation can submit it.
fn rebase_commands_allowed(dialog: Option<&rocket_ui::app::Dialog>) -> Result<(), String> {
    let Some(r) = dialog.and_then(|d| d.rebase.as_ref()) else {
        return Ok(());
    };
    let preserved = r
        .preview
        .as_ref()
        .and_then(|p| p.plan())
        .into_iter()
        .flatten();
    for entry in r.entries.iter().chain(preserved) {
        if let Some(command) = &entry.exec
            && !REBASE_TEST_COMMANDS.contains(&command.as_str())
        {
            return Err(REBASE_COMMAND_REFUSAL.into());
        }
    }
    Ok(())
}

fn rebase_control_allowed(target: &str, dialog: Option<&DialogKind>) -> bool {
    use rocket_git::RebaseAction;
    if target == "dialog-submit" {
        return matches!(
            dialog,
            Some(
                DialogKind::Rebase(
                    RebaseAction::Continue | RebaseAction::Skip | RebaseAction::Abort
                ) | DialogKind::ConflictReload(_)
                    | DialogKind::Confirm(rocket_ui::backend::Action::SaveConflict { .. }, _)
                    | DialogKind::Confirm(rocket_ui::backend::Action::KeepConflictSide { .. }, _)
            )
        );
    }
    matches!(
        target,
        "operation-continue"
            | "operation-skip"
            | "operation-abort"
            | "commit"
            | "dialog-preview-refresh"
            | "dialog-rebase-edits"
            | "dialog-rebase-scroll"
            | "view-changes"
            | "view-commits"
            | "toolbar-branch"
            | "refresh"
            | "toolbar-console"
            | "console-close"
            | "drawer-collapse"
            | "drawer-expand"
            | "stage-all"
            | "unstage-all"
    ) || target.starts_with("conflict-")
        || target.starts_with("change-")
}

/// History rows and their merge-expansion pills carry a full object ID.
fn history_row_target(target: &str) -> bool {
    target.strip_prefix("commit-expand-").or_else(|| target.strip_prefix("commit-"))
        .is_some_and(|id| matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn refresh_click_position(target: &str, current: Rect, events: &mut [Event]) {
    if history_row_target(target) || matches!(target, "cancel-operation" | "status-cancel") {
        // Delivery has already rechecked visibility, ownership and the deadline.
        // A later layout pass can move Stop after the request captured its centre.
        for event in events {
            if let Event::PointerMoved(point) | Event::PointerButton { pos: point, .. } = event {
                *point = current.center();
            }
        }
    }
}

fn operation_state(s: &rocket_git::OperationState) -> Value {
    json!({"idle":s.is_idle(),"merge_committed":s.merge_committed,"cherry_pick":s.cherry_pick,"revert":s.revert,"sequencer":s.sequencer,"rebase":s.rebase,"mail_import":s.mail_import,"bisect":s.bisect,"step":s.step.as_ref().map(|t|json!({"at":t.at,"total":t.total}))})
}

fn overview_state(p: &rocket_ui::overview::Probe) -> Value {
    json!({"visible":p.visible,"rows":p.rows,"marked_rows":p.marked_rows,
        "offset":p.view.offset,"extent":p.view.extent,"viewport":p.view.viewport})
}

fn worktree_value(w: &rocket_git::WorktreeEntry) -> Value {
    json!({"path":w.path.to_string_lossy(),"head":w.head.as_ref().map(|id|id.as_str()),"branch":w.branch.as_ref().map(|b|String::from_utf8_lossy(b)),"main":w.main,"bare":w.bare,"locked":w.locked.as_ref().map(|b|String::from_utf8_lossy(b)),"prunable":w.prunable.as_ref().map(|b|String::from_utf8_lossy(b))})
}
// Input validation compares only the selected action, not thousands of display
// rows every frame. Raw bytes preserve distinctions that lossy labels can hide.
fn remote_tag_validation_binding(s: &rocket_ui::tags::RemoteState) -> Value {
    let entry = |e: &rocket_git::RemoteTagEntry| {
        json!({
            "name":e.name,"remote_id":e.remote_id.as_ref().map(|id|id.as_str()),
            "local_id":e.local.as_ref().map(|t|t.id.as_str()),"symbolic":e.symbolic_target
        })
    };
    json!({
        "active":s.active,"loading":s.running(),"stale":s.stale,
        "selected":s.selected,"confirmation":s.confirmation,"action":s.action.map(|a|a.label()),
        "entry":s.selected_entry().map(entry),
        "preview":s.preview.as_ref().map(|p|json!({
            "remote":p.remote(),"destination":p.destination(),
            "action":p.action().label(),"entry":entry(p.entry())
        }))
    })
}
// A tag batch: the rows offered for its filter, the selection and the review.
fn tag_batch_state(s: &rocket_ui::tags::DialogState) -> Value {
    fn review<P>(r: &rocket_git::TagBatchReview<P>, name: fn(&P) -> &[u8]) -> Value {
        json!({"ready":r.ready.iter().map(|p|String::from_utf8_lossy(name(p))).collect::<Vec<_>>(),
            "refused":r.refused.iter().map(|(n,e)|json!({"name":String::from_utf8_lossy(n),"reason":rocket_ui::backend::user_error(e)})).collect::<Vec<_>>()})
    }
    fn local(p: &rocket_git::TagPreview) -> &[u8] {
        &p.details().entry.name
    }
    fn remote(p: &rocket_git::RemoteTagPreview) -> &[u8] {
        &p.entry().name
    }
    let selected = |names: &[Vec<u8>]| {
        names
            .iter()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect::<Vec<_>>()
    };
    let labels =
        |items: Vec<rocket_ui::batch::Item>| items.into_iter().map(|i| i.label).collect::<Vec<_>>();
    if s.remote.active {
        s.remote.batch.as_ref().map_or(Value::Null, |b| json!({"remote":true,"filter":b.filter,"rows":labels(rocket_ui::batch::publish_items(&s.remote,&b.filter)),
            "selected":selected(&b.selected),"loading":b.running(),"ready":b.ready().is_some(),"review":b.review.as_ref().map(|r|review(r,remote))}))
    } else {
        s.batch.as_ref().map_or(Value::Null, |b| json!({"remote":false,"filter":b.filter,"rows":labels(rocket_ui::batch::delete_items(s,&b.filter)),
            "selected":selected(&b.selected),"loading":b.running(),"ready":b.ready().is_some(),"review":b.review.as_ref().map(|r|review(r,local))}))
    }
}
// Raw names, object IDs and destinations of the batch an input would apply.
fn tag_batch_binding(s: &rocket_ui::tags::DialogState) -> Value {
    if s.remote.active {
        s.remote.batch.as_ref().map_or(Value::Null, |b| json!({"remote":true,"selected":b.selected,"loading":b.running(),
            "ready":b.ready().map(|r|r.ready.iter().map(|p|json!([p.entry().name,p.destination(),p.expected_result().map(|i|i.as_str())])).collect::<Vec<_>>())}))
    } else {
        s.batch.as_ref().map_or(Value::Null, |b| json!({"remote":false,"selected":b.selected,"loading":b.running(),
            "ready":b.ready().map(|r|r.ready.iter().map(|p|json!([p.details().entry.name,p.details().entry.id.as_str()])).collect::<Vec<_>>())}))
    }
}
fn remote_tag_state(s: &rocket_ui::tags::RemoteState) -> Value {
    let entry = |e: &rocket_git::RemoteTagEntry| json!({"name":String::from_utf8_lossy(&e.name),"remote_id":e.remote_id.as_ref().map(|id|id.as_str()),"peeled":e.peeled.as_ref().map(|id|id.as_str()),"symbolic_target":e.symbolic_target.as_ref().map(|v|String::from_utf8_lossy(v)),"local_id":e.local.as_ref().map(|l|l.id.as_str()),"relation":e.relation()});
    json!({"loading":s.running(),"stale":s.stale,"action":s.action.map(|a|a.label()),"selected":s.selected.as_ref().map(|n|String::from_utf8_lossy(n)),"selected_entry":s.selected_entry().map(entry),"filter":s.filter,"show":format!("{:?}",s.show),"notice":s.notice,"done":s.done,
        "remote":s.snapshot.as_ref().map(|p|String::from_utf8_lossy(&p.remote)),"destination":s.snapshot.as_ref().map(|p|p.destination_display()),"total":s.snapshot.as_ref().map_or(0,|p|p.entries.len()),
        "entries":s.snapshot.as_ref().map(|p|p.entries.iter().take(2000).map(entry).collect::<Vec<_>>()),"entries_truncated":s.snapshot.as_ref().is_some_and(|p|p.entries.len()>2000),
        "preview":s.preview.as_ref().map(|p|json!({"action":p.action().label(),"entry":entry(p.entry()),"remote":String::from_utf8_lossy(p.remote()),"destination":p.destination_display(),"expected_result":p.expected_result().map(|id|id.as_str())}))})
}

fn tag_details_value(details: &rocket_git::TagDetails) -> Value {
    let annotation = details
        .annotation
        .as_ref()
        .map(|b| rocket_ui::tags::preview_text(b));
    let subject = details.commit.as_ref().map(|c| {
        let (text, truncated) = rocket_ui::tags::preview_text(&c.message);
        let (first, more) = text
            .split_once('\n')
            .map_or((text.as_str(), false), |(first, _)| (first, true));
        if truncated && !more {
            format!("{first}… [display limited]")
        } else {
            first.to_owned()
        }
    });
    json!({"name":String::from_utf8_lossy(&details.entry.name),"id":details.entry.id.as_str(),"object_type":details.entry.object_type,"target":details.target.as_str(),"target_type":details.target_type,"annotation":annotation.as_ref().map(|(text,_)|text),"annotation_truncated":annotation.as_ref().is_some_and(|(_,truncated)|*truncated),"subject":subject,"signature_verified":false})
}
fn lfs_state(s: &rocket_ui::lfs::DialogState) -> Value {
    json!({"running":s.running(),"applying":s.applying,"remote":s.remote,"selected":s.selected.as_ref().map(|p|p.display_lossy().to_string()),"notice":s.notice,"tracking":s.tracking,"pattern":s.pattern,"literal":s.literal,
        "locks_tab":s.locks_tab,"locks":{"path":s.locks.path,"selected":s.locks.selected,"force_confirmed":s.locks.force_confirmed,"loaded":s.locks.list.is_some(),"entries":s.locks.list.as_ref().map(|l|l.entries.iter().map(|e|json!({"id":e.id,"path":e.path,"owner":e.owner,"locked_at":e.locked_at,"ours":e.ours})).collect::<Vec<_>>()),"preview":s.locks.preview.as_ref().map(|p|json!({"operation":p.action().label(),"remote":p.remote(),"source":p.source(),"path":p.action().path(),"force":p.action().force(),"makes_writable":p.makes_writable()}))},
        "status":s.status.as_ref().map(|p|json!({"version":p.version,"entries":p.entries.iter().map(|e|json!({"path":e.path.display_lossy(),"oid":e.object.oid,"size":e.object.size,"cached":e.cached,"state":e.state.label()})).collect::<Vec<_>>(),"remotes":p.remotes,"tracking_error":p.tracking.as_ref().err(),"tracking_rules":p.tracking.as_ref().ok().map(|rules|rules.iter().map(|r|json!({"line":r.line,"text":r.text})).collect::<Vec<_>>())})),
        "preview":s.preview.as_ref().map(|p|json!({"operation":p.action().label(),"source":p.source(),"object_count":p.object_count(),"total_bytes":p.total_bytes(),"head":match p.head(){rocket_git::Head::Branch{name,target}=>json!({"branch":String::from_utf8_lossy(name),"commit":target.as_str()}),rocket_git::Head::Detached{target}=>json!({"commit":target.as_str()}),_=>Value::Null},"entries":p.entries().iter().map(|e|json!({"path":e.path.display_lossy(),"oid":e.object.oid,"size":e.object.size})).collect::<Vec<_>>(),"attributes":p.attributes().map(|a|json!({"before_bytes":a.before().map(|b|b.len()),"after_bytes":a.after().len(),"removed":a.removed(),"added":a.added()}))}))})
}
fn submodule_state(s: &rocket_ui::submodules::DialogState) -> Value {
    let preview = s.action_preview.as_ref().map(|p|json!({"label":p.label(),"source":p.source(),"parent":p.parent(),"directory":p.directory(),"destination":p.move_destination(),"path":p.path().display_lossy(),"target":p.target().map(|id|id.as_str()),"previous_urls":p.previous_urls(),"descendants":p.descendants()}));
    let entries = s.entries.iter().map(|e|json!({"path":e.path.display_lossy(),"directory":e.directory,"parent":e.parent,"depth":e.depth,"url":e.url,"target":e.target.as_str(),"head":e.head.as_ref().map(|h|h.as_str()),"initialized":e.initialized,"dirty":e.dirty,"state":e.state(),"blocked":e.blocked.map(|f|f.code())})).collect::<Vec<_>>();
    let checkout = s.preview.as_ref().map(
        |p| json!({"source":p.source,"local":p.local_source,"target":p.entry.target.as_str()}),
    );
    json!({"mode":s.mode.name(),"source":s.source,"new_path":s.new_path,"add_parent":s.add_parent,"action_preview":preview,"loading":s.running(),"applying":s.applying,"reviewing":s.reviewing,"selected":s.selected.as_ref().map(|p|p.display_lossy()),"notice":s.notice,"preview_loaded":s.preview.is_some()||s.action_preview.is_some(),"preview":checkout,"entries":entries})
}
/// The whole-version review: stage metadata and choices only, never file content.
fn conflict_whole_state(s: &rocket_ui::conflicts::State) -> Value {
    use rocket_git::ConflictKeep;
    let Some(w) = s.whole.as_ref() else {
        return json!({"loaded":false,"error":s.whole_error});
    };
    let version = |stage: u8| {
        w.version(stage).map(|v| json!({"id":v.id.as_str(),"size":v.size,"binary":v.binary,"executable":v.executable,"gitlink":v.gitlink,"lfs":v.lfs.as_ref().map(|p|json!({"oid":p.oid,"size":p.size})),"unavailable":v.unavailable.map(|f|f.code()),"working_holds":w.working_matches(stage)}))
    };
    let choices: Vec<_> = [
        ConflictKeep::Ours,
        ConflictKeep::Theirs,
        ConflictKeep::Deletion,
    ]
    .into_iter()
    .filter(|k| w.can_keep(*k))
    .map(|k| json!({"keep":k.label(),"change":w.planned_change(k).map(|c|c.label())}))
    .collect();
    json!({"loaded":true,"kind":format!("{:?}",w.kind()),"error":s.whole_error,"path":w.path().display_lossy(),"base":version(1),"ours":version(2),"theirs":version(3),
        "working":w.working_size().map(|size|json!({"size":size,"binary":w.working_binary()})),"binary":w.binary(),"deleted":w.deleted(),"choices":choices})
}
/// GitHub session and page. A live check against api.github.com reports counts
/// and flags only, so account data never reaches test output.
/// Explore's search and results. A live check against api.github.com reports
/// counts and flags only, like the GitHub page.
fn explore_state(app: &Rocket) -> Value {
    use rocket_ui::explore::Problem;
    let s = &app.explore;
    let live = app.github_endpoint == Some(rocket_git::GitHubEndpoint::Public);
    let problem = s.problem.as_ref().map(|p| match p {
        Problem::Offline => "offline",
        Problem::Refused => "refused",
        Problem::NoResults => "no_results",
        Problem::Failed(_) => "failed",
    });
    let mut value = json!({"open":app.page() == Some(rocket_ui::pages::Page::Explore),"running":s.running(),
        "result_count":s.results.len(),"total":s.total,"incomplete":s.incomplete,"more":s.next_page.is_some(),
        "selected":s.selected,"remaining":s.remaining,"wait_seconds":s.wait_seconds(),"problem":problem,
        "signed_in":app.github.is_some(),"anonymous":s.anonymous.is_some(),"confirm_large":s.confirm_large,
        "sort":format!("{:?}", s.sort),"language":s.language,"limit":app.preview_limit});
    if !live {
        value["query"] = json!(s.query);
        value["searched"] = json!(s.searched);
        value["error"] = json!(match &s.problem {
            Some(Problem::Failed(error)) => Some(error),
            _ => None,
        });
        value["results"] = json!(s.results.iter().take(200).map(|r| json!({"name":r.remote.full_name(),
            "stars":r.stars,"language":r.language,"size_kb":r.size_kb,"clone_url":r.clone_url,
            "archived":r.archived,"fork":r.fork})).collect::<Vec<_>>());
        value["address"] = json!(rocket_ui::explore::address(&s.query).map(|a| json!({"name":a.name,"host":a.host,"url":a.url})));
    }
    value
}

/// A preview tab: its state, downloads and source. Live runs keep names out.
fn preview_state(p: &rocket_ui::preview_tab::Preview, app: &Rocket) -> Value {
    use rocket_ui::preview_tab::{Failure, PreviewState};
    let live = app.github_endpoint == Some(rocket_git::GitHubEndpoint::Public);
    let downloads = p.meter.status();
    let opening = match &p.state {
        PreviewState::Opening(o) => Some(o),
        _ => None,
    };
    let mut value = json!({"state":p.state_name(),"unlimited":p.unlimited,"limit":p.meter.limit(),
        "size":downloads.size,"downloading":downloads.running,"downloaded_files":downloads.files,
        "received":downloads.received,"downloaded_bytes":downloads.downloaded,
        "download_failure":downloads.failure.map(|f| f.code()),"signing":p.signing.is_some(),
        "step":opening.map(|o| format!("{:?}", o.step)),"files":opening.map(|o| o.files),
        "stopping":opening.is_some_and(|o| o.stopping),
        "branch":match &p.state { PreviewState::Ready { branch } => branch.as_deref(), _ => None },
        "folder":p.slot.directory().to_string_lossy()});
    if !live {
        value["name"] = json!(p.request.name);
        value["url"] = json!(p.request.url);
        value["host"] = json!(p.request.host);
        value["error"] = json!(match &p.state {
            PreviewState::Failed(Failure::Other(error)) => Some(error),
            _ => None,
        });
        value["sign_in_error"] = json!(p.sign_in_error);
    }
    value
}

fn github_state(app: &Rocket) -> Value {
    let live = app.github_endpoint == Some(rocket_git::GitHubEndpoint::Public);
    let opened = app.opened_urls.as_deref().unwrap_or_default();
    let mut value = json!({"enabled":app.github_endpoint.is_some(),"live":live,"signed_in":app.github.is_some(),
        "opened_count":opened.len(),"opened_urls":(!live).then_some(opened)});
    let Some(s) = app.dialog.as_ref().and_then(|d| d.github.as_ref()) else {
        return value;
    };
    value["page"] = json!({"section":format!("{:?}",s.section),"loading":s.running(),"sign_in":s.sign_in,"notice":s.notice,
        "no_remote":s.no_remote,"loaded":s.loaded,"selected":s.selected,"all_notifications":s.all_notifications,
        "repository_count":s.repositories.len(),"repositories_more":s.repositories_next.is_some(),
        "notification_count":s.notifications.len(),"unread_count":s.notifications.iter().filter(|n|n.unread).count(),
        "notifications_more":s.notifications_next.is_some(),"linked":s.links.is_some(),
        "has_pull":s.links.as_ref().is_some_and(|l|l.pull.is_some())});
    if !live {
        value["page"]["filter"] = json!(s.filter);
        value["page"]["login"] = json!(app.github.as_ref().map(|g| &g.account().login));
        value["page"]["links"] = json!(s.links.as_ref().map(|l|json!({"remote":l.remote,"repository":l.repository.remote.full_name(),
            "branch":l.branch,"pushed":l.pushed,"ahead":l.ahead,"base":l.base.full_name(),"base_branch":l.base_branch,
            "pull":l.pull.as_ref().map(|p|json!({"number":p.number,"url":p.html_url,"draft":p.draft}))})));
        value["page"]["repositories"] = json!(
            s.repositories
                .iter()
                .take(500)
                .map(|r| json!({"name":r.remote.full_name(),
            "private":r.private,"fork":r.fork,"archived":r.archived,"clone_url":r.clone_url}))
                .collect::<Vec<_>>()
        );
        value["page"]["notifications"] = json!(
            s.notifications
                .iter()
                .take(500)
                .map(|n| json!({"repository":n.repository.full_name(),
            "title":n.title,"kind":n.kind,"unread":n.unread,"url":n.web_url}))
                .collect::<Vec<_>>()
        );
    }
    value
}
fn ssh_state(app: &Rocket) -> Value {
    let s=&app.ssh;
    json!({"enabled":s.enabled,"loaded":s.loaded,"busy":s.running(),"operation":s.operation,
        "authentication":if s.authentication==rocket_git::SshAuthentication::Direct {"direct"}else{"agent"},
        "selected":s.selected,"found":s.found,"creating":s.creating,"review":s.preview.is_some(),
        "restoring":s.preview.as_ref().is_some_and(|p|p.restoring),"agent_running":s.agent_running,
        "stopping":s.stopping,"prompt":s.prompt.map(|p|format!("{p:?}")),"invalid":s.invalid,
        "error":s.error,"notice":s.notice,"discovery_error":s.discovery_error,"using_discovered":s.discovered_choice.is_some(),
        "discovery":s.discovery.as_ref().map(|d|json!({"directory":d.directory,"skipped":d.skipped,"truncated":d.truncated,
            "keys":d.keys.iter().map(|k|json!({"name":k.key.file_name().unwrap_or_default().to_string_lossy(),"path":k.key,
                "kind":k.kind,"fingerprint":k.fingerprint,"used":s.profiles.iter().any(|p|p.fingerprint==k.fingerprint),
                "loaded":s.loaded_keys.contains(&k.fingerprint)})).collect::<Vec<_>>()})),
        "profiles":s.profiles.iter().map(|p|json!({"alias":p.alias,"host":p.host,"user":p.user,"port":p.port,
            "fingerprint":p.fingerprint,"encrypted":p.encrypted,"passphrase_saved":p.passphrase_saved,"loaded":s.loaded_keys.contains(&p.fingerprint)})).collect::<Vec<_>>()})
}
/// CI marks of the active tab, the popover and the re-run review. Only the
/// loopback fixture reports commit IDs and check names.
fn ci_state(app: &Rocket) -> Value {
    use rocket_ui::ci_checks::Whole;
    let fixture = app.forge.fixture.is_some();
    let checks = |d: &rocket_git::ForgeCommitChecks| json!({
        "commit": d.commit(), "state": d.state().label(), "summary": rocket_ui::ci_checks::summary_text(d),
        "count": d.checks.len(), "limited": d.limited, "error": d.error.map(|e| e.code()), "missing": d.missing,
        "rerun_block": d.rerun.map(|b| b.code()), "failed_runs": d.failed_runs().map(|r| json!({"title": r.title(), "failed": r.failed})).collect::<Vec<_>>(),
        "checks": d.checks.iter().map(|c| json!({"name": c.name, "group": c.group, "state": c.state.key(), "detail": c.detail,
            "link": c.link.is_some(), "external": c.link.as_ref().and_then(|l| l.external.clone())})).collect::<Vec<_>>(),
    });
    let tab = app.tab().map(|t| {
        let mut marks: Vec<_> = t.ci.marks.iter().map(|(k, v)| (k.clone(), v.label())).collect();
        marks.sort();
        json!({"active": t.ci.active(), "repository": t.ci.repository.as_ref().map(|r| r.path().to_owned()),
            "marks": if fixture { json!(marks.into_iter().collect::<std::collections::BTreeMap<_, _>>()) } else { json!(t.ci.marks.len()) },
            "faded": t.ci.faded, "quiet": t.ci.quiet, "reading": t.ci.reading(), "refreshes": t.ci.refreshes, "limited": t.ci.limited,
            "whole": t.ci.whole.as_ref().map(|w| match w { Whole::Hidden(p) => json!({"kind":"hidden","missing":p}), Whole::Paused(at) => json!({"kind":"paused","until":at}), Whole::Stale(at) => json!({"kind":"stale","since":at}), Whole::Rejected => json!({"kind":"rejected"}) })})
    });
    let popover = app.ci.popover.as_ref().map(|p| json!({"commit": p.commit, "reading": p.reading(), "error": p.error, "notice": p.notice,
        "details": p.details.as_deref().filter(|_| fixture).map(checks)}));
    let rerun = app.ci.rerun.as_ref().map(|r| json!({"reviewing": r.reviewing(), "sending": r.sending(), "ready": r.preview.is_some(),
        "error": r.error, "unknown": r.unknown, "stopping": r.stopping, "runs": r.failed_runs().iter().map(|run| run.title()).collect::<Vec<_>>(),
        "from_run": matches!(r.source, rocket_ui::ci_checks::RerunSource::Run(_))}));
    let pull = &app.forge.pull;
    json!({"tab": tab, "popover": popover, "rerun": rerun, "signing_in": app.ci.signing_in(),
        "pull": {"reading": pull.ci_task.is_some(), "error": pull.ci_error, "checks": pull.ci.as_deref().filter(|_| fixture).map(checks)}})
}
/// The Actions page of the active tab. Only the loopback fixture reports run
/// titles, branches and job names.
fn actions_state(app: &Rocket) -> Value {
    let fixture = app.forge.fixture.is_some();
    let Some(t) = app.tab() else { return Value::Null };
    let a = &t.actions;
    let run = |r: &rocket_git::ForgeCiRun| json!({"id": r.id, "title": r.title(), "state": r.state.key(), "event": r.event, "branch": r.branch,
        "commit": r.commit, "took": r.took, "attempt": r.attempt});
    let details = a.details.as_deref().map(|d| json!({"run": d.run.id, "jobs": d.jobs.len(), "limited": d.limited, "error": d.error.map(|e| e.code()),
        "rerun_block": d.rerun.map(|b| b.code()), "failed": if fixture { json!(d.run.failed) } else { json!(d.run.failed.len()) },
        "names": if fixture { json!(d.jobs.iter().map(|j| json!({"name": j.name, "state": j.state.key(), "detail": j.detail})).collect::<Vec<_>>()) } else { Value::Null }}));
    json!({"shown": t.view == rocket_ui::app::View::Actions, "reach": a.reach.as_ref().map(|r| r.key()),
        "on_branch": a.on_branch, "branch": if fixture { json!(a.branch) } else { Value::Null }, "show": a.show.key(),
        "count": a.runs.len(), "more": a.more, "pages": a.pages, "reading": a.reading(), "reads": a.reads,
        "failure": a.failure.map(|f| f.code()), "error": a.error.is_some(), "hidden": a.hidden.is_some(),
        "missing": a.hidden.clone().flatten(), "paused": a.paused_until.is_some(), "live": a.live, "focused": a.focused,
        "selected": a.selected, "reading_details": a.reading_details(), "details_error": a.details_error, "details": details,
        "dot": rocket_ui::ci_checks::head_dot(t).map(|(s, _)| s.label()),
        "runs": if fixture { json!(a.runs.iter().map(run).collect::<Vec<_>>()) } else { Value::Null }})
}
fn hosting_state(app: &Rocket) -> Value {
    let s = &app.forge;
    // Tokens and stored account bodies are never returned, even in fixtures.
    let mut value = json!({"enabled":s.enabled,"fixture":s.fixture.is_some(),"section":format!("{:?}",s.section),
        "provider":s.provider.key(),"connected":s.session.is_some(),"busy":s.running(),"stopping":s.stopping,
        "enterprise_version":s.session.as_ref().and_then(|session|session.enterprise_version()),
        "azure_api_version":(s.provider==rocket_git::ForgeProvider::AzureServer).then_some(&s.azure_api_version),
        "operation":s.operation,"error":s.error,"notice":s.notice,"uncertain":s.uncertain,"loaded":s.loaded,
        "count":s.items.len(),"visible":s.visible(),"more":s.next.is_some(),"reviewing":s.preview.is_some(),
        "total":s.total,"incomplete":s.incomplete,"recent_searches":s.searches.len(),
        "accounts_loaded":s.accounts_loaded,"accounts_count":s.accounts.len(),"remember":s.remember,
        "created":s.created.as_ref().map(|i|i.number)});
    if s.fixture.is_some() || s.publish_check.is_some() {
        // The live operator scope exposes its disposable publish form/result,
        // never credentials, saved accounts or unrelated hosting metadata.
        let p=&s.publish;
        value["publish"]=json!({"owner":p.owner,"name":p.name,"branch":p.branch,"remote":p.remote,"private":p.private,"public_ack":p.public_ack,"ssh":p.ssh,"upstream":p.upstream,
            "phase":p.progress.phase(),"reviewing":p.preview.is_some(),"commit":p.preview.as_ref().map(|v|v.commit().as_str()),
            "report":p.report.as_ref().map(|r|json!({"creation":r.creation.code(),"remote_added":r.remote_added,"push":r.push.as_ref().map(|v|v.remote_outcome.code()),"url":r.url,"errors":p.report_errors()}))});
    }
    if s.fixture.is_some() {
        let p = &s.pull;
        value["drafts"]=json!({"loaded":s.drafts.loaded,"saving":s.drafts.saving(),"error":s.drafts.error,"count":s.drafts.records.len(),"restored_comments":s.drafts.restored.len(),"pending_write":s.drafts.pending.is_some()});
        let collection = |count: usize, limited: bool, error: Option<rocket_git::ForgeFailure>| json!({"count":count,"limited":limited,"error":error.map(|e|e.code())});
        value["pull"]=json!({"number":p.number,"tab":p.tab,"file":p.file,"composing":p.composing,"action":p.action.event(),"body_bytes":p.body.len(),"reviewing":p.preview.is_some(),"uncertain":p.unknown.is_some(),"can_acknowledge":p.checked_unknown,
            "comments":p.comments.iter().map(|c|json!({"path":c.path(),"start":c.start_line(),"line":c.line(),"side":c.side().key(),"head":c.head(),"body_bytes":c.body().len()})).collect::<Vec<_>>(),
            "inline_body_bytes":p.inline_body.len(),"inline_editing":p.inline_editing,
            "inline_selection":p.inline_selection.as_ref().map(|c|json!({"path":c.path,"line":c.line,"side":c.side.key(),"head":c.head})),
            "merge":{"method":p.merge.method.key(),"reviewing":p.merge.preview.is_some(),"head":p.merge.preview.as_ref().map(|v|v.summary().head.clone()),"base":p.merge.preview.as_ref().map(|v|v.summary().base.clone()),"result":p.merge.result,"uncertain":p.merge.unknown.is_some(),"can_acknowledge":p.merge.checked_unknown},
            "preview_head":p.preview.as_ref().map(|x|x.summary().head.clone()),"submitted":p.submitted.as_ref().map(|r|json!({"id":r.id,"state":r.state,"commit":r.commit})),
            "details":p.details.as_ref().map(|d|json!({"head":d.summary().head,"base":d.summary().base,"title":d.summary().title,"state":d.summary().state,"merged":d.summary().merged,"mergeable":d.summary().mergeable,"mergeable_state":d.summary().mergeable_state,
                "commits":collection(d.commits.items.len(),d.commits.limited,d.commits.error),"files":collection(d.files.items.len(),d.files.limited,d.files.error),
                "discussion":collection(d.discussion.items.len(),d.discussion.limited,d.discussion.error),"inline":collection(d.inline_comments.items.len(),d.inline_comments.limited,d.inline_comments.error),
                "reviews":collection(d.reviews.items.len(),d.reviews.limited,d.reviews.error),"checks":collection(d.checks.items.len(),d.checks.limited,d.checks.error),"statuses":collection(d.statuses.items.len(),d.statuses.limited,d.statuses.error),
                "paths":d.files.items.iter().map(|f|json!({"path":f.path,"previous":f.previous,"patch":f.patch.is_some(),"lines":f.lines.iter().map(|l|json!({"old":l.old,"new":l.new})).collect::<Vec<_>>()})).collect::<Vec<_>>() }))});
        value["pull"]["threads"]=json!({"loaded":p.threads.snapshot.is_some(),"count":p.threads.snapshot.as_ref().map(|s|s.threads.len()),"limited":p.threads.snapshot.as_ref().is_some_and(|s|s.limited),"selected":p.threads.selected,"body_bytes":p.threads.body.len(),"reviewing":p.threads.preview.is_some(),"uncertain":p.threads.unknown,"can_acknowledge":p.threads.checked,"resolved":p.threads.snapshot.as_ref().map(|s|s.threads.iter().map(|t|t.resolved).collect::<Vec<_>>())});
        value["repository"]=json!(s.repository); value["filter"]=json!(s.filter);
        value["from_fork"]=json!(s.from_fork); value["source_repository"]=json!(s.source_repository);
        value["state"]=json!(format!("{:?}",s.state));
        value["items"]=json!(s.items.iter().take(500).map(|i|json!({"number":i.number,"title":i.title,"state":i.state,"source":i.source,"target":i.target,"url":i.url})).collect::<Vec<_>>());
        value["review"]=json!(s.preview.as_ref().map(|p|json!({"source":p.draft().source,"target":p.draft().target,"source_id":p.source_id(),"target_id":p.target_id(),"title":p.draft().title,"source_repository":p.source_repository().path(),"target_repository":p.draft().repository.path()})));
    }
    value
}
fn statistics_state(s: &rocket_ui::statistics::State) -> Value {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    json!({"section":format!("{:?}",s.section),"range":s.range.key(),"all_branches":s.all_branches,"period":s.period.key(),
        "activity_running":s.activity_running(),"treemap_running":s.treemap_running(),
        "activity_error":s.activity_error,"treemap_error":s.treemap_error,
        "activity":s.activity.as_deref().map(|a| json!({"commits":a.commits,"merges":a.merges,"authors":a.authors.len(),
            "top_authors":a.authors.iter().take(10).map(|x| json!({"name":text(&x.name),"commits":x.commits})).collect::<Vec<_>>(),
            "first":a.first,"last":a.last,"months":a.months.iter().map(|(y,m,n)| json!([y,m,n])).collect::<Vec<_>>(),
            "weekdays":a.weekdays,"hours":a.hours,"limited":a.limited})),
        "treemap":s.treemap.as_deref().map(|m| json!({"commit":m.commit.as_str(),"commits":m.commits,"limited":m.limited,
            "zoom":s.zoom.iter().map(|n| text(n)).collect::<Vec<_>>(),
            "folder":s.shown().map(|f| json!({"files":f.files,"size":f.size,"changes":f.changes,
                "children":f.children.iter().take(400).map(|c| json!({"name":text(&c.name),"folder":c.folder,"size":c.size,"changes":c.changes,"files":c.files})).collect::<Vec<_>>()}))}))})
}
fn dialog_name(kind: &DialogKind) -> &'static str {
    match kind {
        DialogKind::RepositoryManager => "repository-manager",
        DialogKind::Open => "open",
        DialogKind::NewRepository => "init",
        DialogKind::NewBranch => "new-branch",
        DialogKind::TrackBranch { .. } => "track-branch",
        DialogKind::RepositorySettings => "identity",
        DialogKind::Signing(_) => "signing",
        DialogKind::CommitAction(request) => request.label(),
        DialogKind::Rebase(action) => action.label(),
        DialogKind::MailImport(action) => action.label(),
        DialogKind::Bisect => "bisect",
        DialogKind::Merge => "merge",
        DialogKind::Recovery => "recovery",
        DialogKind::UndoCommit => "undo-commit",
        DialogKind::Tags => "tags",
        DialogKind::Worktrees => "worktrees",
        DialogKind::Submodules => "submodules",
        DialogKind::Lfs => "lfs",
        DialogKind::GitHub => "github",
        DialogKind::Hosting => "hosting",
        DialogKind::Integrations => "integrations",
        DialogKind::Statistics => "statistics",
        DialogKind::Benchmark => "benchmark",
        DialogKind::GitFlow(_) => "gitflow",
        DialogKind::DraftNotice(_) => "draft-notice",
        DialogKind::Clone => "clone",
        DialogKind::Explore => "explore",
        DialogKind::KeepPreview(_) => "keep-preview",
        DialogKind::FileEdit(_) => "file-edit",
        DialogKind::Patch(mode) => mode.label(),
        DialogKind::Fetch => "fetch",
        DialogKind::Pull => "pull",
        DialogKind::Push => "push",
        DialogKind::AtomicPush => "atomic-push",
        DialogKind::Ignore(_) => "ignore",
        DialogKind::Workflows => "workflows",
        DialogKind::Quit => "quit",
        DialogKind::About => "about",
        DialogKind::Legal => "legal",
        DialogKind::Changelog => "changelog",
        DialogKind::Stash => "stash-save",
        DialogKind::DeleteBranches => "delete-branches",
        DialogKind::RunCommand(_) => "run-command",
        DialogKind::ConflictReload(_) => "conflict-reload",
        DialogKind::CloseTab(_) => "close-tab",
        DialogKind::Confirm(rocket_ui::backend::Action::SaveConflict { .. }, _) => "conflict-save",
        DialogKind::Confirm(rocket_ui::backend::Action::KeepConflictSide { .. }, _) => {
            "conflict-keep"
        }
        DialogKind::Confirm(rocket_ui::backend::Action::ApplyStash { .. }, _) => "stash-apply",
        DialogKind::Confirm(rocket_ui::backend::Action::DropStash(_), _) => "stash-drop",
        DialogKind::Confirm(rocket_ui::backend::Action::AbortMerge, _) => "merge-abort",
        DialogKind::Confirm(rocket_ui::backend::Action::FinishMerge, _) => "merge-finish",
        DialogKind::AddRemote => "remote-add",
        DialogKind::EditRemote(..) => "remote-edit",
        DialogKind::Confirm(rocket_ui::backend::Action::RemoveRemote(_), _) => "remote-remove",
        _ => "other",
    }
}

fn gitflow_state(s: &rocket_ui::gitflow::DialogState) -> Value {
    use rocket_git::{GitFlowDevelop, GitFlowKeep, GitFlowNote, GitFlowStep};
    use rocket_ui::gitflow::Preview;
    let preview = s.preview.as_ref().map(|p| match p {
        Preview::Init(p) => json!({"kind":"init","unchanged":p.unchanged(),
            "changes":p.changes().iter().map(|(k, v)| json!({"key":k,"value":v})).collect::<Vec<_>>(),
            "develop":match p.develop() {
                GitFlowDevelop::Exists(id) => json!({"exists":id.as_str()}),
                GitFlowDevelop::Create { source, target, track } => json!({"source":source,"target":target.as_str(),"track":track}),
            }}),
        Preview::Start(p) => json!({"kind":"start","branch":p.branch(),"base":p.base(),"base_id":p.base_id().as_str(),
            "tag":p.tag(),"carried":p.carried(),"unpushed":p.unpushed()}),
        Preview::Finish(p) => json!({"kind":"finish","branch":p.branch(),"tip":p.tip().as_str(),"ends_on":p.ends_on(),
            "keep":p.keep().map(|k| match k {
                GitFlowKeep::Requested => "requested",
                GitFlowKeep::CheckedOut(_) => "checked-out",
                GitFlowKeep::Unpushed(_) => "unpushed",
            }),
            "steps":p.steps().iter().map(|step| match step {
                GitFlowStep::Switch { branch } => json!({"step":"switch","branch":branch}),
                GitFlowStep::Merge { branch, into, mode, fast_forward, .. } => json!({"step":"merge","branch":branch,"into":into,"mode":format!("{mode:?}"),"fast_forward":fast_forward}),
                GitFlowStep::Tag { name, message } => json!({"step":"tag","name":name,"message_bytes":message.len()}),
                GitFlowStep::Delete { branch } => json!({"step":"delete","branch":branch}),
            }).collect::<Vec<_>>(),
            "notes":p.notes().iter().map(|note| match note {
                GitFlowNote::Merged { into } => json!({"note":"merged","into":into}),
                GitFlowNote::Tagged { name, .. } => json!({"note":"tagged","name":name}),
                GitFlowNote::Unpushed { branch, commits } => json!({"note":"unpushed","branch":branch,"commits":commits}),
                GitFlowNote::ReleaseBranch { branch } => json!({"note":"release-branch","branch":branch}),
            }).collect::<Vec<_>>()}),
    });
    json!({"request":s.request.code(),"loading":s.loading_state(),"reviewing":s.reviewing(),"applying":s.applying,
        "initialized":s.state.as_ref().map(|st| st.config.is_some()),"incomplete":s.state.as_ref().map(|st| st.incomplete()),
        "branches":s.state.as_ref().map(|st| st.branches.iter().map(|b| json!({"kind":b.kind.label(),"name":b.name,"branch":b.branch,"current":b.is_head})).collect::<Vec<_>>()),
        "config":{"master":s.config.master,"develop":s.config.develop,"feature":s.config.feature,"release":s.config.release,
            "hotfix":s.config.hotfix,"support":s.config.support,"version_tag":s.config.version_tag},
        "name":s.name,"base":s.base,"branch":s.branch,
        "options":{"no_ff":s.options.no_ff,"delete_branch":s.options.delete_branch,"tag_message_bytes":s.options.tag_message.len()},
        "preview":preview})
}

fn file_history_state(state: &rocket_ui::file_history::State) -> Value {
    json!({
        "running":state.running(),"history_running":state.history_running(),"detail_running":state.detail_running(),"cancelling":state.cancelling,
        "directory":state.directory,"directory_input":state.directory_input,"path":state.path.as_ref().map(|p|p.display_lossy()),"input":state.path_input,"tip":state.tip.as_ref().map(|id|id.as_str()),
        "mode":format!("{:?}",state.mode),"selected":state.selected,"more":state.next.is_some(),"limited":state.limited,"error":state.error,"notice":state.notice,
        "entries":state.entries.iter().take(1000).map(|e|json!({"id":e.commit.id.as_str(),"path":e.path.display_lossy(),"previous_path":e.previous_path.as_ref().map(|p|p.display_lossy()),"deleted":e.deleted})).collect::<Vec<_>>(),
        "content_kind":state.content.as_ref().map(|c|format!("{:?}",c.kind)),"content_lines":state.content.as_ref().map(|c|c.lines.len()),"content_limited":state.content.as_ref().map(|c|c.limited),
        "patch_added":state.patch.as_ref().map(|p|p.files.iter().map(|f|f.added).sum::<usize>()),"patch_removed":state.patch.as_ref().map(|p|p.files.iter().map(|f|f.removed).sum::<usize>()),
        "blame":state.blame.as_ref().map(|b|json!({"total":b.total_lines,"next":b.next_line,"lines":b.lines.iter().map(|l|json!({"id":l.commit.as_str(),"line":l.line,"original_line":l.original_line,"path":l.path.display_lossy(),"author":String::from_utf8_lossy(&l.author),"content":String::from_utf8_lossy(&l.content).chars().take(256).collect::<String>()})).collect::<Vec<_>>()})),
        "origin":state.origin.as_ref().map(|l|json!({"id":l.commit.as_str(),"path":l.path.display_lossy(),"line":l.original_line}))
    })
}

fn file_search_state(s: &rocket_ui::code_search::State) -> Value {
    let ended = s.ended.map(|e| format!("{e:?}"));
    json!({
        "query":s.query,"regex":s.regex,"match_case":s.match_case,"scope":format!("{:?}",s.scope),"path":s.path,
        "results_pane":s.results,"running":s.running(),"stopping":s.stopping,"opening":s.opening(),"ended":ended,
        "error":s.error,"bad_pattern":s.bad_pattern,"matches":s.matches,"file_count":s.groups.len(),
        "searched":s.ran.as_ref().map(|(_, label)| label),"selected":s.selected,"show_file":s.show_file,
        "files":s.groups.iter().take(200).map(|g| json!({
            "path":rocket_ui::tree::path_label(&g.file.path),"count":g.file.lines.len(),"capped":g.file.capped,"open":g.open,
            "size":g.detail.size,"lfs_pointer":g.detail.lfs_pointer,"not_utf8":g.not_utf8,
            "lines":g.file.lines.iter().take(20).map(|l| json!({"number":l.number,"text":l.text,"marks":l.marks.iter().map(|m|[m.start,m.end]).collect::<Vec<_>>(),"cut_start":l.cut_start,"cut_end":l.cut_end})).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "open":s.opened.as_ref().map(|o| json!({
            "path":rocket_ui::tree::path_label(&o.path),"number":o.number,"group":o.group,"line":o.line,"error":o.error,
            "content":o.content.as_ref().map(|c| json!({"kind":format!("{:?}",c.kind),"bytes":c.bytes,"first":c.first,"line_count":c.lines.len(),
                "window":c.window,"lossy":c.lossy,"limited":c.limited,"marked_lines":c.marks.len(),
                "focus_text":(o.number as usize).checked_sub(c.first).and_then(|i| c.lines.get(i)).map(|l| l.chars().take(300).collect::<String>())}))
        }))
    })
}

fn repository_tree_state(state: &rocket_ui::tree::State) -> Value {
    json!({
        "running":state.running(),"directory_running":state.directory_running(),"cancelling":state.cancelling,
        "revision_input":state.revision,"tip":state.listing.as_ref().map(|l| l.directory.commit().as_str()),
        "directory":state.listing.as_ref().map(|l| rocket_ui::tree::path_label(l.directory.path())),
        "entry_count":state.listing.as_ref().map(|l|l.entries.len()),"entries_limited":state.listing.as_ref().is_some_and(|l|l.entries.len()>1000),
        "entries":state.listing.as_ref().map(|l|l.entries.iter().take(1000).enumerate().map(|(i,e)|json!({"index":i,"name":rocket_ui::tree::path_label(e.name()),"kind":format!("{:?}",e.kind()),"object":e.object().as_str()})).collect::<Vec<_>>()),
        "selected":state.selected,"selected_path":state.selected_entry().map(|e|rocket_ui::tree::path_label(e.path())),
        "show_list":state.show_list,"error":state.error,"notice":state.notice,
        "content":state.content.as_ref().map(|c|json!({"kind":format!("{:?}",c.kind),"bytes":c.bytes,"line_count":c.lines.len(),"limited":c.limited,"lossy":c.lossy,"preview":c.lines.iter().take(30).map(|l|l.chars().take(300).collect::<String>()).collect::<Vec<_>>()}))
    })
}

fn scan_state_value(s:&rocket_ui::repository_scan::State)->Value {
    let selection = s.selection();
    json!({"root":s.root,"recursive":s.recursive,"phase":s.phase(),"error":s.error,"notice":s.notice,"folder":s.folder,
        "filter":s.filter,"view":s.view.code(),"match_disk":s.match_disk,"new_folders":s.new_folders(),"left_out":s.left_out(),"visible":s.visible(),
        "selection":{"eligible":selection.eligible,"shown":selection.selected,"total":selection.total,"hidden":selection.hidden,"checked":selection.checked(),"partial":selection.partial()},
        "skipped":s.summary.skipped,"invalid":s.summary.invalid,"limited":s.summary.limited,
        "issues":s.summary.issues.iter().map(|i|json!({"path":i.path,"reason":i.reason})).collect::<Vec<_>>(),
        "folders_checked":s.progress.folders.load(std::sync::atomic::Ordering::Relaxed),"found":s.progress.found.load(std::sync::atomic::Ordering::Relaxed),
        "selection_checked":s.progress.checked.load(std::sync::atomic::Ordering::Relaxed),
        "rows":s.rows.iter().map(|r|json!({"path":r.repository.path,"relative_path":r.relative_path,"name":r.name,"kind":r.repository.kind,"selected":r.selected,"saved":r.saved,"group":r.group,"meta":r.meta})).collect::<Vec<_>>()})
}

fn repository_map_state(state:&rocket_ui::repository_map::State)->Value {
    let p=state.page();
    let data=p.data.as_ref().map(|d|json!({"kind":format!("{:?}",d.kind),"tip":d.tip.as_ref().map(|id|id.as_str()),
        "scanned":d.scanned,"limited":d.limited,"node_count":d.nodes.len(),"link_count":d.links.len(),"notes":d.notes,
        "has_more":d.has_more,"history_limit":d.history_limit,"boundary_parents":d.boundary_parents,
        "nodes":d.nodes.iter().enumerate().take(4096).map(|(i,n)|json!({"index":i,"kind":format!("{:?}",n.kind),"label":rocket_ui::tree::path_label(&n.label),"detail":rocket_ui::tree::path_label(&n.detail),"path":n.path.as_ref().map(|v|rocket_ui::tree::path_label(v)),"commit":n.commit.as_ref().map(|v|v.as_str()),"count":n.count,"files":n.files,"time":n.time,"evidence":n.evidence.iter().take(24).map(|v|rocket_ui::tree::path_label(v)).collect::<Vec<_>>(),"refs":n.refs.iter().map(|v|rocket_ui::tree::path_label(v)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "links":d.links.iter().take(12000).map(|e|json!({"from":e.from,"to":e.to,"kind":format!("{:?}",e.kind),"weight":e.weight})).collect::<Vec<_>>(),"links_limited":d.links.len()>12000
    }));
    json!({"kind":format!("{:?}",state.kind),"running":p.running(),"cancelling":p.cancelling,
        "revision":p.revision,"query":p.query,"all_branches":p.all_branches,"period":p.period.key(),"alternate":p.alternate,"condensed":p.condensed,"cell":p.cell,"line":p.line,
        "base":p.base,"path_scope":p.path_scope,"can_more":p.can_more(),"collapsed":p.collapsed.iter().map(|v|rocket_ui::tree::path_label(v)).collect::<Vec<_>>(),"neighbor_anchor":p.neighbor_anchor.as_ref().map(|v|rocket_ui::tree::path_label(v)),
        "projected_links":p.projected_links.iter().take(12000).map(|e|json!({"from":e.from,"to":e.to,"kind":format!("{:?}",e.kind),"weight":e.weight})).collect::<Vec<_>>(),
        "export":p.export.as_ref().map(|e|json!({"dir":e.dir,"name":e.name,"nodes":e.nodes,"links":e.links,"bytes":e.text.len(),"running":e.running(),"stopping":e.stopping,"error":e.error,"notice":e.notice})),
        "error":p.error,"notice":p.notice,"stale":p.stale,"selected":p.selected,"show_details":p.show_details,"zoom":p.zoom,
        "visible":p.visible,"file_groups":p.file_groups.iter().map(|g|json!({"folder":g.folder,"files":g.files})).collect::<Vec<_>>(),"display_offset":p.display_offset,"view_total":p.view_total,"data":data})
}

fn force_push_state_value(s: &rocket_ui::force_push::DialogState) -> Value {
    json!({"running":s.running(),"loading":s.task.is_some(),"applying":s.applying,"confirmation":s.confirmation,"notice":s.notice,
    "preview":s.preview.as_ref().map(|p|json!({"ready":p.can_apply(),"source":p.source().as_str(),"expected":p.expected().as_str(),
        "remote":String::from_utf8_lossy(p.remote()),"local_branch":String::from_utf8_lossy(p.local_branch()),"remote_branch":String::from_utf8_lossy(p.remote_branch()),
        "destination":p.destination_display(),"removed_count":p.removed_count(),"added_count":p.added_count(),
        "removed":p.removed().iter().map(|c|json!({"id":c.id.as_str(),"subject":c.subject_lossy()})).collect::<Vec<_>>(),
        "added":p.added().iter().map(|c|json!({"id":c.id.as_str(),"subject":c.subject_lossy()})).collect::<Vec<_>>()
    }))})
}
fn prediction_value(p: &rocket_git::CommitPrediction) -> Value {
    json!({"outcome":p.kind.code(),"checked":p.checked,"total":p.total,"stopped_at":p.stopped_at,"paths":p.paths.iter().map(|v|v.display_lossy()).collect::<Vec<_>>(),"reason":p.reason.map(|v|v.code())})
}
fn pull_state_value(s: &rocket_ui::pull::DialogState) -> Value {
    json!({"strategy":s.strategy.code(),"running":s.running(),"fetching":s.fetching,"applying":s.applying,"notice":s.notice,"stopping":s.cancelling,"checking":s.task.is_some(),"prediction":s.preview.as_ref().and_then(|p|p.prediction()).map(prediction_value),
        "fetched":s.fetched.as_ref().map(|p|json!({"original":p.original().as_str(),"target":p.target().as_str(),"branch":String::from_utf8_lossy(&p.upstream().branch),"upstream":String::from_utf8_lossy(&p.upstream().tracking_reference)})),
        "preview":s.preview.as_ref().map(|p|json!({"ready":p.can_apply(),"incoming":p.incoming(),"local":p.current_only(),"relation":format!("{:?}",p.relation()),"replay":p.replay().len(),"recovery":p.recovery_reference()}))})
}
fn signing_state_value(s: &rocket_ui::signing::DialogState) -> Value {
    json!({"loading":s.running(),"applying":s.applying,"reviewed":s.reviewed,"loaded":s.settings.is_some(),"format":s.options.format,"key":s.key,"allowed":s.allowed,"sign_commits":s.options.sign_commits,"notice":s.notice,"verification":s.verification.as_ref().map(|v|json!({"id":v.id.as_str(),"kind":v.kind,"status":v.status.code(),"format":v.format,"trust":v.trust,"fingerprint":v.fingerprint,"signer":v.signer}))})
}

fn tags_state_value(t: &rocket_ui::tags::DialogState) -> Value {
    json!({"loading":t.running(),"applying":t.applying,"action":t.action.map(|a|a.label()),"limit":t.limit,"more":t.page.as_ref().is_some_and(|p|p.more),"selected":t.selected.as_ref().map(|e|String::from_utf8_lossy(&e.name)),"target":t.target,"annotated":t.annotated,"sign":t.sign,"message":t.message,"preview_loaded":t.preview.is_some(),"details":t.details.as_ref().map(tag_details_value),"preview":t.preview.as_ref().map(|p|json!({"details":tag_details_value(p.details()),"signed":p.signing().is_some(),"recovery_reference":p.recovery_reference()})),"entries":t.page.as_ref().map(|p|p.entries.iter().map(|e|json!({"name":String::from_utf8_lossy(&e.name),"id":e.id.as_str(),"object_type":e.object_type,"symbolic":e.symbolic})).collect::<Vec<_>>())})
}

fn atomic_push_state(s: &rocket_ui::atomic_push::DialogState) -> Value {
    json!({"loading":s.task.is_some(),"applying":s.applying,"stopping":s.stopping,"notice":s.notice,
        "selection":s.selection().iter().map(|r|json!({"local":r.local_branch,"remote":r.remote_branch})).collect::<Vec<_>>(),
        "rows":s.rows.iter().take(500).map(|r|json!({"local":String::from_utf8_lossy(&r.local),"selected":r.selected,"destination":r.destination})).collect::<Vec<_>>(),
        "preview":s.preview.as_ref().map(|p|json!({"remote":String::from_utf8_lossy(p.remote()),"destination":p.destination_display(),"can_apply":p.can_apply(),
            "entries":p.entries().iter().map(|e|json!({"local":String::from_utf8_lossy(&e.selection.local_branch),"remote":String::from_utf8_lossy(&e.selection.remote_branch),"source":e.source.as_str(),"expected":e.expected.as_ref().map(|id|id.as_str()),"kind":e.kind.code()})).collect::<Vec<_>>()}))})
}
fn ignore_state(s: &rocket_ui::ignore::DialogState) -> Value {
    json!({"loading":s.read.is_some()||s.task.is_some(),"applying":s.applying,"stopping":s.stopping,"notice":s.notice,
        "loaded":s.snapshot.is_some(),"text":s.text,"path":s.path,"folder":s.folder,
        "candidates":s.candidates.iter().take(100).zip(&s.selected).map(|(p,yes)|json!({"path":p.display_lossy(),"selected":yes})).collect::<Vec<_>>(),
        "preview":s.preview.as_ref().map(|p|json!({"before_bytes":p.before().len(),"after_bytes":p.after().len(),"existed":p.existed()}))})
}
fn workflow_state(s:&rocket_ui::workflows::DialogState)->Value {
    use rocket_ui::workflows::Review;
    json!({"mode":s.mode,"strategy":s.strategy,"stash":s.stash,"loading":s.task.is_some()||s.stash_apply.as_ref().is_some_and(|a|a.task.is_some()),"applying":s.applying,"stopping":s.stopping,"notice":s.notice,"selected":s.selected.iter().map(|r|String::from_utf8_lossy(r)).collect::<Vec<_>>(),
        "saved_selection":s.stash_apply.as_ref().map(|a|json!({"files":a.review.as_ref().map(|r|r.files().iter().map(|f|json!({"path":f.path().display_lossy(),"hunks":f.hunk_count(),"mode_change":f.mode_change(),"hunk_lines":(0..f.hunk_count().min(16)).map(|h|&f.hunk_line_ids(h)[..f.hunk_line_ids(h).len().min(64)]).collect::<Vec<_>>(),"line_ids_limited":f.hunk_count()>16||(0..f.hunk_count()).any(|h|f.hunk_line_ids(h).len()>64)})).collect::<Vec<_>>()),"selected":a.selection().iter().map(|s|json!({"file":s.file,"hunks":s.hunks,"lines":s.lines})).collect::<Vec<_>>()})),
        "preview":s.preview.as_ref().map(|p|match p.as_ref(){
        Review::StashApply(p)=>json!({"kind":"stash-apply-selection","id":p.entry().id.as_str(),"paths":p.files().iter().map(|f|f.path().display_lossy()).collect::<Vec<_>>()}),
        Review::StashSnapshot(p)=>json!({"kind":"stash-snapshot","include_untracked":p.include_untracked(),"paths":p.files().iter().map(|f|f.path().display_lossy()).collect::<Vec<_>>()}),
        Review::IgnoreTracked(p)=>json!({"kind":"ignore-tracked","paths":p.files().iter().map(|f|f.path().display_lossy()).collect::<Vec<_>>(),"rules_before_bytes":p.rules().before().len(),"rules_after_bytes":p.rules().after().len()}),
        Review::FastForward(p)=>json!({"kind":"fast-forward","entries":p.entries().iter().map(|e|json!({"branch":String::from_utf8_lossy(&e.branch),"before":e.before.as_str(),"after":e.after.as_ref().map(|id|id.as_str()),"blocked":e.blocked})).collect::<Vec<_>>()}),
        Review::RenameRemote(p)=>json!({"kind":"remote-rename","old":String::from_utf8_lossy(&p.remote().name),"new":String::from_utf8_lossy(p.name()),"upstreams":p.affected_branches().len()}),
        Review::StashControl(p)=>json!({"kind":if matches!(p.action(),rocket_git::StashControl::Clear){"stash-clear"}else{"stash-pop"},"entries":p.entries().iter().map(|e|json!({"index":e.index,"id":e.id.as_str()})).collect::<Vec<_>>()}),
        Review::SubmoduleBatch(p)=>json!({"kind":"submodule-batch","paths":p.entries().map(|e|e.path.display_lossy()).collect::<Vec<_>>()}),
        Review::StashSelection(p)=>json!({"kind":"stash-selection","lines":p.selected_count()}),
        Review::Prune(p)=>json!({"kind":"prune","obsolete":p.obsolete().iter().map(|r|json!({"reference":String::from_utf8_lossy(&r.reference),"id":r.id.as_str()})).collect::<Vec<_>>(),"local_kept":p.stale_local_branches().iter().map(|r|String::from_utf8_lossy(r)).collect::<Vec<_>>()}),
        Review::Delete(p)=>json!({"kind":"delete","branch":String::from_utf8_lossy(p.branch()),"expected":p.expected().as_str()}),
        Review::Rename(p)=>json!({"kind":"rename","id":p.entry().id.as_str(),"name":p.name()}),
        Review::Saved(p)=>json!({"kind":if matches!(p.action(),rocket_git::SavedWorkAction::Switch(_)){"saved-switch"}else{"saved"},"target":p.target().as_str(),"changed_files":p.changed_files()}),
        Review::CommitPush(p)=>json!({"kind":"commit-push","remote":String::from_utf8_lossy(p.remote()),"branch":String::from_utf8_lossy(p.branch()),"destination":String::from_utf8_lossy(p.destination()),"staged_files":p.staged_files()}),
        Review::Branch(p)=>json!({"kind":p.kind().label(),"feature":p.feature(),"base":p.base(),"tip":p.tip().as_str(),"base_tip":p.base_tip().as_str(),"commits":p.commits(),"save_changes":p.options().save_changes,"saved_files":p.saved_changes(),"allow_fast_forward":p.options().allow_fast_forward,"can_fast_forward":p.can_fast_forward()}),
        Review::Lfs(p)=>json!({"kind":"lfs","bytes":p.bytes(),"objects":p.objects().iter().map(|o|json!({"oid":o.oid,"size":o.size})).collect::<Vec<_>>()}),
    })})
}

#[cfg(test)]
mod terminal_grid_tests {
    use super::*;
    #[test]
    fn terminal_probes_are_exact_for_this_platforms_shell() {
        let root = std::env::temp_dir().join("rocket-probe-fixture");
        for probe in SHELL_PROBES {
            assert!(terminal_probe_allowed(probe, &root).is_ok(), "{probe}");
        }
        let modified = format!("{}; ls", SHELL_PROBES[1]);
        for text in [
            modified.as_str(),
            "ls /",
            "Get-ChildItem C:/",
            "echo 'arbitrary'",
        ] {
            assert!(terminal_probe_allowed(text, &root).is_err(), "{text}");
        }
        #[cfg(target_os = "linux")]
        {
            let quoted = |p: &Path| format!("printf %s ROCKET_TERMINAL_EDIT > '{}'", p.display());
            let fixture = quoted(&root.join("rocket-terminal-fixture.txt"));
            assert!(terminal_probe_allowed(&fixture, &root).is_ok());
            assert!(terminal_probe_allowed(&quoted(&root.join("elsewhere.txt")), &root).is_err());
        }
    }
    #[test]
    fn custom_command_form_text_fields_are_editable_but_buttons_are_not() {
        for id in ["dialog-command-input-format", "dialog-command-input-base_1", "dialog-commands-input-1-label", "dialog-commands-input-7-checked"] {
            assert!(command_input_editable(id), "{id}");
        }
        for id in ["dialog-command-input-", "dialog-command-input-path-browse", "dialog-commands-input-1-remove", "dialog-commands-input-8-key", "dialog-commands-input-1-checkbox"] {
            assert!(!command_input_editable(id), "{id}");
        }
    }
    #[test]
    fn validation_binding_preserves_exact_tag_bytes_and_confirmation() {
        let mut tags = rocket_ui::tags::RemoteState {
            selected: Some(vec![0xff]),
            ..Default::default()
        };
        let before = remote_tag_validation_binding(&tags);
        tags.selected = Some(vec![0xfe]);
        assert_ne!(before, remote_tag_validation_binding(&tags));
        let before = remote_tag_validation_binding(&tags);
        tags.confirmation = "new confirmation".into();
        assert_ne!(before, remote_tag_validation_binding(&tags));
    }
    #[test]
    fn validation_escape_is_consumed_without_dismissing_the_underlying_review() {
        let mut input = RawInput {
            events: vec![key_event(Key::Escape, true), key_event(Key::Escape, false)],
            ..Default::default()
        };
        assert!(
            validation_interruption(&mut input, false)
                .unwrap()
                .contains("cancelled")
        );
        assert!(input.events.is_empty());
        assert!(validation_interruption(&mut input, false).is_none());
        input.events = vec![Event::Text("human edit".into())];
        assert!(
            validation_interruption(&mut input, false)
                .unwrap()
                .contains("New user input")
        );
        assert_eq!(input.events.len(), 1); // Preserve the person's intended edit.
    }
    #[test]
    fn a_click_only_cancels_a_check_before_its_dialog_shows() {
        let click = Event::PointerButton {
            pos: Pos2::ZERO,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        let mut input = RawInput {
            events: vec![click],
            ..Default::default()
        };
        assert!(validation_interruption(&mut input, false).is_some());
        assert!(validation_interruption(&mut input, true).is_none(), "Stop reaches the dialog");
        input.events = vec![Event::Text("human edit".into())];
        assert!(validation_interruption(&mut input, true).is_some());
    }
    #[test]
    fn detached_rebase_scope_allows_resolution_but_not_transport_or_unrelated_writes() {
        use rocket_git::RebaseAction;
        assert!(rebase_control_allowed("conflict-stage", None));
        assert!(rebase_control_allowed("operation-abort", None));
        assert!(rebase_control_allowed("toolbar-branch", None)); // Read-only menu.
        assert!(rebase_control_allowed(
            "dialog-submit",
            Some(&DialogKind::Rebase(RebaseAction::Continue))
        ));
        for target in [
            "toolbar-push",
            "toolbar-pull",
            "toolbar-fetch",
            "commit-submit",
            "repository-rebase",
            "remote-add",
            "toolbar-branch-new",
            "toolbar-branch-sync",
            "toolbar-branch-finish",
        ] {
            assert!(!rebase_control_allowed(target, None));
        }
        for dialog in [
            DialogKind::Rebase(RebaseAction::Start),
            DialogKind::Push,
            DialogKind::NewBranch,
        ] {
            assert!(!rebase_control_allowed("dialog-submit", Some(&dialog)));
        }
    }
    #[test]
    fn history_target_recognizes_only_rows_and_expansion_pills() {
        for length in [40, 64] {
            let id = "a".repeat(length);
            assert!(history_row_target(&format!("commit-{id}")));
            assert!(history_row_target(&format!("commit-expand-{id}")));
        }
        for target in ["commit-filter", "commit-menu-fold", "commit-abc", "history-scroll"] {
            assert!(!history_row_target(target));
        }
        assert!(!history_row_target(&format!("commit-{}", "g".repeat(40))));
    }
    #[test]
    fn queued_stop_click_uses_the_last_rendered_position() {
        for target in ["status-cancel", "cancel-operation"] {
            let ctx = Context::default();
            let render = |offset, events| {
                let mut result = (Rect::NOTHING, false);
                let mut output = ctx.run_ui(RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                    events, ..Default::default()
                }, |ui| {
                    ui.add_space(offset);
                    let stop = ui.button("Stop");
                    result = (stop.rect, stop.clicked());
                });
                output.textures_delta.clear();
                result
            };
            let (old, _) = render(0.0, vec![]);
            let (stale, _) = click_events(old.center(), PointerButton::Primary, Modifiers::NONE, false);
            let (current, _) = render(100.0, vec![]);
            assert!(!render(100.0, stale.clone()).1, "the saved coordinate misses the moved button");
            let mut events = stale;
            refresh_click_position(target, current, &mut events);
            assert!(render(100.0, events).1, "one {target} click must reach the current button");
            assert!(!ctx.input(|i| i.pointer.any_down()));
        }
    }
    #[test]
    fn recent_grid_has_actual_columns_and_bounded_payloads() {
        let mut screen = rocket_terminal::Screen::new(240, 100);
        screen.feed("中é👩🏽‍💻|".as_bytes());
        let (grid, truncated) = terminal_grid(&screen);
        assert!(!truncated);
        assert_eq!(grid.len(), 4);
        assert_eq!(grid[1]["column"], 2);
        assert_eq!(grid[2]["width"], 2);
        assert_eq!(grid[3]["column"], 5);
        // Bottom blank rows do not hide the visible output near the top.
        assert_eq!(grid[0]["row"], 0);
        screen.feed("X".repeat(20_000).as_bytes());
        let (grid, truncated) = terminal_grid(&screen);
        assert!(truncated);
        assert_eq!(grid.len(), 512);
        let mut screen = rocket_terminal::Screen::new(240, 4);
        screen.feed(format!("a{}", "\u{301}".repeat(60)).repeat(200).as_bytes());
        let (grid, truncated) = terminal_grid(&screen);
        assert!(truncated);
        assert!(
            grid.iter()
                .map(|c| c["text"].as_str().unwrap().len())
                .sum::<usize>()
                <= 8192
        );
    }
    #[test]
    fn detached_bisect_controls_do_not_allow_unrelated_writes() {
        for target in [
            "dialog-submit",
            "dialog-bisect-good",
            "dialog-bisect-skip",
            "dialog-bisect-restore",
            "dialog-bisect-auto",
            "dialog-bisect-program",
            "dialog-bisect-arguments",
            "dialog-bisect-timeout",
        ] {
            assert!(bisect_control_allowed(target, Some(&DialogKind::Bisect)));
            assert!(!bisect_control_allowed(target, Some(&DialogKind::Push)));
        }
        // Bisect opens from Repository → History & Recovery.
        for target in ["menu-repository", "repository-history-menu", "repository-bisect"] {
            assert!(bisect_control_allowed(target, None));
        }
        for target in [
            "toolbar-fetch",
            "toolbar-push",
            "commit-submit",
            "stage-all",
            "dialog-bisect-surprise",
        ] {
            assert!(!bisect_control_allowed(target, Some(&DialogKind::Bisect)));
        }
    }
}

