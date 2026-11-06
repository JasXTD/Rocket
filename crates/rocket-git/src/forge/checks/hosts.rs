//! CI reads for GitLab, Gitea, Bitbucket Cloud and Azure DevOps. GitLab and
//! Gitea can re-run; Bitbucket and Azure only show results.
use super::*;

impl ForgeSession {
    fn versioned_path(&self, path: &str) -> String {
        if self.config.provider().is_azure() {
            format!("{path}{}api-version={}", if path.contains('?') { "&" } else { "?" }, self.config.azure_api_version())
        } else {
            path.into()
        }
    }
    pub(super) fn host_marks(&self, repo: &ForgeRepository, asked: &[String], c: &CancellationToken) -> Result<Vec<(String, ForgeCiState)>> {
        let p = self.config.provider();
        if p == ForgeProvider::GitLab {
            // One list of the newest pipelines answers every recent commit.
            let j = self.request(MARKS, "read_ci_pipelines", "GET", &format!("{}/pipelines?per_page=100&order_by=id&sort=desc", self.prefix(repo)), None, false, c)?.0;
            let rows = j.array().filter(|a| a.len() <= 100).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            let mut marks = Vec::new();
            for id in asked {
                let newest = rows.iter().find(|r| r.str("sha").map(str::to_ascii_lowercase).as_deref() == Some(id.as_str()));
                if let Some(row) = newest {
                    marks.push((id.clone(), gitlab_pipeline_state(row.str("status").ok_or(Error::Forge(ForgeFailure::InvalidResponse))?)?));
                }
            }
            return Ok(marks);
        }
        if p.is_azure() {
            // Azure Pipelines reports builds; one list of recent builds answers every commit.
            let builds = self.azure_builds(repo, MARKS, c)?;
            let mut marks = Vec::new();
            for id in asked {
                let states: Vec<ForgeCheckState> = builds.iter().filter(|(sha, _)| sha == id).map(|(_, b)| b.state).collect();
                if !states.is_empty() {
                    marks.push((id.clone(), ci_rollup(states.into_iter())));
                }
            }
            return Ok(marks);
        }
        let mut marks = Vec::new();
        for id in asked {
            c.check()?;
            match self.host_statuses(repo, id, c) {
                Ok(Some(checks)) => marks.push((id.clone(), ci_rollup(checks.iter().map(|c| c.state)))),
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(marks)
    }
    /// The commit statuses of one commit, or None when the host doesn't know it.
    fn host_statuses(&self, repo: &ForgeRepository, sha: &str, c: &CancellationToken) -> Result<Option<Vec<ForgeCheck>>> {
        let p = self.config.provider();
        let (path, key, fields) = match p {
            ForgeProvider::Gitea => (format!("/repos/{}/commits/{sha}/status", repo.encoded()), "statuses", ("context", "status", "description", "target_url")),
            ForgeProvider::Bitbucket => (format!("{}/commit/{sha}/statuses?pagelen=50", self.prefix(repo)), "values", ("name", "state", "description", "url")),
            ForgeProvider::Azure | ForgeProvider::AzureServer => (self.versioned_path(&format!("{}/commits/{sha}/statuses?latestOnly=true&$top=100", self.prefix(repo))), "value", ("genre", "state", "description", "targetUrl")),
            _ => return fail(ForgeFailure::Unsupported),
        };
        let j = match self.request(MARKS, "read_ci_statuses", "GET", &path, None, false, c) {
            Ok((j, _)) => j,
            Err(Error::Forge(ForgeFailure::NotFound)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let rows = j.get(key).and_then(Json::array).filter(|a| a.len() <= CHECK_LIMIT).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        let mut out = Vec::new();
        for row in rows {
            let check = match p {
                // Gitea's statuses name their state "status"; older servers "state".
                ForgeProvider::Gitea => status_check(row, &self.config, fields.0, if row.get("status").is_some() { "status" } else { "state" }, fields.2, fields.3),
                // Azure names a status by its context: genre/name.
                ForgeProvider::Azure | ForgeProvider::AzureServer => azure_status(row, &self.config),
                _ => status_check(row, &self.config, fields.0, fields.1, fields.2, fields.3).or_else(|| status_check(row, &self.config, "key", fields.1, fields.2, fields.3)),
            }
            .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            out.push(check);
        }
        Ok(Some(out))
    }
    pub(super) fn host_checks(&self, out: &mut ForgeCommitChecks, c: &CancellationToken) -> Result<()> {
        let repo = out.repository.clone();
        let sha = out.commit.clone();
        match self.config.provider() {
            ForgeProvider::GitLab => self.gitlab_checks(out, c),
            ForgeProvider::Gitea => {
                match self.host_statuses(&repo, &sha, c) {
                    Ok(Some(checks)) => out.checks = checks,
                    Ok(None) => {}
                    Err(e) => self.keep_failure(out, e)?,
                }
                self.gitea_runs(out, c)
            }
            _ => {
                match self.host_statuses(&repo, &sha, c) {
                    Ok(Some(checks)) => out.checks = checks,
                    Ok(None) => {}
                    Err(e) => self.keep_failure(out, e)?,
                }
                if self.config.provider().is_azure() {
                    match self.azure_builds(&repo, DETAILS, c) {
                        Ok(builds) => out.checks.extend(builds.into_iter().filter(|(id, _)| *id == sha).map(|(_, b)| b)),
                        Err(e) => self.keep_failure(out, e)?,
                    }
                }
                if out.checks.iter().any(|c| c.state == ForgeCheckState::Failed) {
                    out.rerun = Some(ForgeRerunBlock::Unsupported);
                }
                Ok(())
            }
        }
    }
    /// The builds API filters by repository ID, so the repository is read first.
    fn azure_repository_id(&self, repo: &ForgeRepository, op: &'static str, c: &CancellationToken) -> Result<String> {
        let j = self.request(op, "read_ci_repository", "GET", &self.versioned_path(&self.prefix(repo)), None, false, c)?.0;
        Ok(j.str("id").filter(|v| !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?.to_owned())
    }
    /// The newest builds of this repository, with the commit each built.
    fn azure_builds(&self, repo: &ForgeRepository, op: &'static str, c: &CancellationToken) -> Result<Vec<(String, ForgeCheck)>> {
        let id = self.azure_repository_id(repo, op, c)?;
        let project = encode(repo.parts().0);
        let j = self.request(op, "read_ci_builds", "GET", &self.versioned_path(&format!("/{project}/_apis/build/builds?repositoryId={id}&repositoryType=TfsGit&$top=100&queryOrder=queueTimeDescending")), None, false, c)?.0;
        let rows = j.get("value").and_then(Json::array).filter(|a| a.len() <= 100).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        let mut out: Vec<(String, ForgeCheck)> = Vec::new();
        for row in rows {
            let Some(sha) = row.str("sourceVersion").and_then(oid) else { continue };
            let build = azure_build(row, &self.config).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            // Builds come newest first; an older build of the same pipeline and commit is history.
            if !out.iter().any(|(s, b)| *s == sha && b.name == build.name) {
                out.push((sha, build));
            }
        }
        Ok(out)
    }
    fn gitlab_checks(&self, out: &mut ForgeCommitChecks, c: &CancellationToken) -> Result<()> {
        let repo = out.repository.clone();
        let prefix = self.prefix(&repo);
        let j = match self.request(DETAILS, "read_ci_pipelines", "GET", &format!("{prefix}/pipelines?sha={}&per_page=5&order_by=id&sort=desc", out.commit), None, false, c) {
            Ok((j, _)) => j,
            Err(e) => return self.keep_failure(out, e),
        };
        let rows = j.array().filter(|a| a.len() <= 5).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        let Some(pipeline) = rows.first() else { return Ok(()) };
        if pipeline.str("sha").map(str::to_ascii_lowercase).as_deref() != Some(out.commit.as_str()) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let mut run = gitlab_run(pipeline, &self.config).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
        // A commit's checks keep its mark's words: a canceled pipeline failed,
        // a waiting one is running.
        run.state = match run.state {
            ForgeCheckState::Cancelled => ForgeCheckState::Failed,
            ForgeCheckState::Queued => ForgeCheckState::Running,
            state => state,
        };
        let id = run.id;
        let group = run.title();
        match self.request(DETAILS, "read_ci_jobs", "GET", &format!("{prefix}/pipelines/{id}/jobs?per_page={CHECK_LIMIT}"), None, false, c) {
            Ok((j, headers)) => {
                let jobs = j.array().filter(|a| a.len() <= CHECK_LIMIT).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                out.limited |= header(&headers, "x-next-page").is_some_and(|v| !v.trim().is_empty());
                for job in jobs {
                    let check = gitlab_job(job, &self.config, &group, id).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                    if matches!(check.state, ForgeCheckState::Failed | ForgeCheckState::Cancelled) {
                        run.failed.push(check.name.clone());
                    }
                    out.checks.push(check);
                }
            }
            Err(e) => self.keep_failure(out, e)?,
        }
        out.runs.push(run);
        if out.failed_runs().next().is_none() {
            if out.runs.iter().any(|r| r.state == ForgeCheckState::Running && !r.failed.is_empty()) {
                out.rerun = Some(ForgeRerunBlock::StillRunning);
            }
            return Ok(());
        }
        match self.rerun_access(DETAILS, &repo, c)? {
            Ok(access) => out.rerun = access,
            Err(e) => {
                self.keep_failure(out, e)?;
                out.rerun = Some(ForgeRerunBlock::Unverified);
            }
        }
        Ok(())
    }
    fn gitea_runs(&self, out: &mut ForgeCommitChecks, c: &CancellationToken) -> Result<()> {
        let repo = out.repository.clone();
        let base = format!("/repos/{}", repo.encoded());
        let rows = match self.request(DETAILS, "read_ci_runs", "GET", &format!("{base}/actions/runs?head_sha={}&limit=20&page=1", out.commit), None, false, c) {
            Ok((j, _)) => j.get("workflow_runs").and_then(Json::array).filter(|a| a.len() <= 20).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?.to_vec(),
            // Servers without Gitea Actions' API answer 404: no runs, no re-run.
            Err(Error::Forge(ForgeFailure::NotFound)) => {
                if out.checks.iter().any(|c| c.state == ForgeCheckState::Failed) {
                    out.rerun = Some(ForgeRerunBlock::Unsupported);
                }
                return Ok(());
            }
            Err(e) => return self.keep_failure(out, e),
        };
        for row in &rows {
            if row.str("head_sha").map(str::to_ascii_lowercase).as_deref() != Some(out.commit.as_str()) {
                return fail(ForgeFailure::InvalidResponse);
            }
            let mut run = github_run(row, &self.config, &repo).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            if run.state == ForgeCheckState::Failed {
                match self.request(DETAILS, "read_ci_jobs", "GET", &format!("{base}/actions/runs/{}/jobs?limit=100&page=1", run.id), None, false, c) {
                    Ok((j, _)) => {
                        let jobs = j.get("jobs").and_then(Json::array).filter(|a| a.len() <= CHECK_LIMIT).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
                        for job in jobs {
                            if matches!(job.str("conclusion"), Some("failure" | "timed_out")) {
                                run.failed.push(text(job, "name", 256).ok_or(Error::Forge(ForgeFailure::InvalidResponse))?);
                            }
                        }
                    }
                    Err(e) => self.keep_failure(out, e)?,
                }
            }
            out.runs.push(run);
        }
        if out.failed_runs().next().is_none() {
            return Ok(());
        }
        match self.rerun_access(DETAILS, &repo, c)? {
            Ok(access) => out.rerun = rerun_block(&out.runs, ci_now(), false).or(access),
            Err(e) => {
                self.keep_failure(out, e)?;
                out.rerun = Some(ForgeRerunBlock::Unverified);
            }
        }
        Ok(())
    }
    /// One page of the repository's runs on GitHub, Gitea, GitLab, Bitbucket
    /// Cloud or Azure DevOps.
    pub(super) fn host_runs(&self, list: &mut ForgeRunList, branch: Option<&str>, c: &CancellationToken) -> Result<()> {
        let repo = list.repository.clone();
        let page = list.page;
        let p = self.config.provider();
        let invalid = || Error::Forge(ForgeFailure::InvalidResponse);
        if p.is_github() || p == ForgeProvider::Gitea {
            let size = if p.is_github() { "per_page" } else { "limit" };
            let mut path = format!("/repos/{}/actions/runs?{size}={CI_RUN_PAGE}&page={page}", repo.encoded());
            if let Some(b) = branch {
                path.push_str(&format!("&branch={}", encode(b)));
            }
            let j = self.request(RUNS, "read_ci_run_list", "GET", &path, None, false, c)?.0;
            let rows = j.get("workflow_runs").and_then(Json::array).filter(|a| a.len() <= CI_RUN_PAGE).ok_or_else(invalid)?;
            for row in rows {
                list.runs.push(github_run(row, &self.config, &repo).ok_or_else(invalid)?);
            }
            list.more = j.u64("total_count").is_some_and(|t| t > (page * CI_RUN_PAGE) as u64);
        } else if p == ForgeProvider::GitLab {
            let mut path = format!("{}/pipelines?per_page={CI_RUN_PAGE}&page={page}&order_by=id&sort=desc", self.prefix(&repo));
            if let Some(b) = branch {
                path.push_str(&format!("&ref={}", encode(b)));
            }
            let (j, headers) = self.request(RUNS, "read_ci_run_list", "GET", &path, None, false, c)?;
            let rows = j.array().filter(|a| a.len() <= CI_RUN_PAGE).ok_or_else(invalid)?;
            for row in rows {
                list.runs.push(gitlab_run(row, &self.config).ok_or_else(invalid)?);
            }
            list.more = header(&headers, "x-next-page").is_some_and(|v| !v.trim().is_empty());
        } else if p == ForgeProvider::Bitbucket {
            let mut path = format!("{}/pipelines?sort=-created_on&pagelen={CI_RUN_PAGE}&page={page}", self.prefix(&repo));
            if let Some(b) = branch {
                path.push_str(&format!("&target.branch={}", encode(b)));
            }
            let j = self.request(RUNS, "read_ci_run_list", "GET", &path, None, false, c)?.0;
            let rows = j.get("values").and_then(Json::array).filter(|a| a.len() <= CI_RUN_PAGE).ok_or_else(invalid)?;
            for row in rows {
                list.runs.push(bitbucket_run(row, &self.config, &repo).ok_or_else(invalid)?);
            }
            // The next page's URL is only a sign there is one; it is never followed.
            list.more = j.get("next").is_some_and(|n| *n != Json::Null);
        } else if p.is_azure() {
            let id = self.azure_repository_id(&repo, RUNS, c)?;
            let project = encode(repo.parts().0);
            // Builds have no page number: read the newest page × 30 and keep this page.
            let top = CI_RUN_PAGE * page;
            let mut path = format!("/{project}/_apis/build/builds?repositoryId={id}&repositoryType=TfsGit&$top={top}&queryOrder=queueTimeDescending");
            if let Some(b) = branch {
                path.push_str(&format!("&branchName={}", encode(&format!("refs/heads/{b}"))));
            }
            let j = self.request(RUNS, "read_ci_run_list", "GET", &self.versioned_path(&path), None, false, c)?.0;
            let rows = j.get("value").and_then(Json::array).filter(|a| a.len() <= top).ok_or_else(invalid)?;
            for row in rows.iter().skip(top - CI_RUN_PAGE) {
                list.runs.push(azure_run(row, &self.config).ok_or_else(invalid)?);
            }
            list.more = rows.len() == top;
        } else {
            return fail(ForgeFailure::Unsupported);
        }
        Ok(())
    }
    /// The run read again and its jobs. A failed read is kept for the page.
    pub(super) fn run_details(&self, out: &mut ForgeRunDetails, c: &CancellationToken) -> Result<()> {
        let repo = out.repository.clone();
        let p = self.config.provider();
        let id = out.run.id;
        let invalid = || Error::Forge(ForgeFailure::InvalidResponse);
        // (the run's own path, the jobs' path) for this host
        let (run_path, jobs_path) = if p.is_github() || p == ForgeProvider::Gitea {
            let base = format!("/repos/{}/actions/runs/{id}", repo.encoded());
            let jobs = if p.is_github() { format!("{base}/jobs?filter=latest&per_page={CHECK_LIMIT}") } else { format!("{base}/jobs?limit={CHECK_LIMIT}&page=1") };
            (base, jobs)
        } else if p == ForgeProvider::GitLab {
            let base = format!("{}/pipelines/{id}", self.prefix(&repo));
            (base.clone(), format!("{base}/jobs?per_page={CHECK_LIMIT}"))
        } else if p == ForgeProvider::Bitbucket {
            let uuid = out.run.uuid.clone().ok_or_else(invalid)?;
            let base = format!("{}/pipelines/{}", self.prefix(&repo), encode(&uuid));
            (base.clone(), format!("{base}/steps?pagelen={CHECK_LIMIT}"))
        } else if p.is_azure() {
            let base = format!("/{}/_apis/build/builds/{id}", encode(repo.parts().0));
            (self.versioned_path(&base), self.versioned_path(&format!("{base}/timeline")))
        } else {
            return fail(ForgeFailure::Unsupported);
        };
        match self.request(RUN, "read_ci_run", "GET", &run_path, None, false, c) {
            Ok((j, _)) => {
                let fresh = match p {
                    ForgeProvider::GitLab => gitlab_run(&j, &self.config),
                    ForgeProvider::Bitbucket => bitbucket_run(&j, &self.config, &repo),
                    ForgeProvider::Azure | ForgeProvider::AzureServer => azure_run(&j, &self.config),
                    _ => github_run(&j, &self.config, &repo),
                }
                .ok_or_else(invalid)?;
                if fresh.id != id {
                    return fail(ForgeFailure::InvalidResponse);
                }
                out.run = fresh;
            }
            Err(e) => self.keep(&mut out.error, &mut out.missing, e)?,
        }
        let group = out.run.title();
        let (j, headers) = match self.request(RUN, "read_ci_jobs", "GET", &jobs_path, None, false, c) {
            Ok(r) => r,
            Err(e) => return self.keep(&mut out.error, &mut out.missing, e),
        };
        let key = match p {
            ForgeProvider::GitLab => None,
            ForgeProvider::Bitbucket => Some("values"),
            ForgeProvider::Azure | ForgeProvider::AzureServer => Some("records"),
            _ => Some("jobs"),
        };
        let rows = match key {
            Some(k) => j.get(k).and_then(Json::array),
            None => j.array(),
        };
        // Pages are bounded by the page size asked for. A timeline lists stages,
        // phases and tasks too; only its first 100 jobs are kept.
        let rows = rows.filter(|a| p.is_azure() || a.len() <= CHECK_LIMIT).ok_or_else(invalid)?;
        for row in rows {
            if p.is_azure() && row.str("type") != Some("Job") {
                continue;
            }
            if out.jobs.len() == CHECK_LIMIT {
                out.limited = true;
                break;
            }
            let mut job = match p {
                ForgeProvider::GitLab => gitlab_job(row, &self.config, &group, id),
                ForgeProvider::Bitbucket => bitbucket_step(row, out.run.link.clone()),
                ForgeProvider::Azure | ForgeProvider::AzureServer => azure_job(row, out.run.link.clone()),
                _ => github_check_run(row, &self.config, &repo),
            }
            .ok_or_else(invalid)?;
            job.group = group.clone();
            job.run = Some(id);
            out.jobs.push(job);
        }
        out.limited |= match p {
            ForgeProvider::GitLab => header(&headers, "x-next-page").is_some_and(|v| !v.trim().is_empty()),
            ForgeProvider::Bitbucket => j.get("next").is_some_and(|n| *n != Json::Null),
            ForgeProvider::Azure | ForgeProvider::AzureServer => false,
            _ => j.u64("total_count").is_some_and(|t| t > rows.len() as u64),
        };
        Ok(())
    }
}
/// A GitLab pipeline, from a list or read alone.
fn gitlab_run(row: &Json, config: &ForgeConfig) -> Option<ForgeCiRun> {
    let id = row.u64("id").filter(|n| *n > 0)?;
    let state = match row.str("status")? {
        "success" => ForgeCheckState::Passed,
        "failed" => ForgeCheckState::Failed,
        "canceled" => ForgeCheckState::Cancelled,
        "running" | "canceling" => ForgeCheckState::Running,
        "pending" | "created" | "preparing" | "waiting_for_resource" | "waiting_for_callback" | "scheduled" | "manual" => ForgeCheckState::Queued,
        "skipped" => ForgeCheckState::Skipped,
        _ => return None,
    };
    let mut run = ForgeCiRun::new(id, text(row, "name", 128).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Pipeline".into()), state);
    run.number = row.u64("iid");
    run.link = link(row.str("web_url"), config);
    let reference = row.str("ref");
    let merge = reference.and_then(|r| r.strip_prefix("refs/merge-requests/")).and_then(|r| r.split('/').next()).and_then(|n| n.parse::<u64>().ok());
    let tag = row.bool("tag") == Some(true);
    if merge.is_none() && !tag {
        run.branch = branch_name(reference);
    }
    run.event = row.str("source").map(|s| match s {
        "push" if tag => "tag".into(),
        "push" => "push".into(),
        "merge_request_event" => merge.map_or_else(|| "merge request".into(), |n| format!("merge request !{n}")),
        "schedule" => "schedule".into(),
        "web" => "manual".into(),
        "api" => "API".into(),
        "parent_pipeline" | "pipeline" => "parent pipeline".into(),
        other => words(other),
    });
    run.commit = row.str("sha").and_then(oid);
    run.created = row.str("created_at").and_then(timestamp);
    run.started = row.str("started_at").and_then(timestamp);
    run.took = seconds(row.get("duration"));
    Some(run)
}
/// A duration in seconds, as a JSON number that may have a fraction.
fn seconds(value: Option<&Json>) -> Option<u64> {
    match value {
        Some(Json::Number(n)) => n.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(|s| s as u64),
        _ => None,
    }
}
/// A Bitbucket Cloud pipeline. Its build number is its id here; the UUID reads it.
fn bitbucket_run(row: &Json, config: &ForgeConfig, repo: &ForgeRepository) -> Option<ForgeCiRun> {
    let number = row.u64("build_number").filter(|n| *n > 0)?;
    let uuid = row.str("uuid").filter(|u| bitbucket_uuid(u))?.to_owned();
    let state = row.get("state")?;
    let result = state.get("result").and_then(|r| r.str("name"));
    let state = match state.str("name")? {
        "COMPLETED" => match result? {
            "SUCCESSFUL" => ForgeCheckState::Passed,
            "FAILED" | "ERROR" => ForgeCheckState::Failed,
            "STOPPED" | "EXPIRED" => ForgeCheckState::Cancelled,
            _ => ForgeCheckState::Neutral,
        },
        "IN_PROGRESS" | "RUNNING" => match state.get("stage").and_then(|s| s.str("name")) {
            Some("PAUSED" | "HALTED") => ForgeCheckState::Queued,
            _ => ForgeCheckState::Running,
        },
        "PENDING" | "PARSING" | "PAUSED" | "HALTED" => ForgeCheckState::Queued,
        _ => return None,
    };
    let mut run = ForgeCiRun::new(number, "Pipeline".into(), state);
    run.number = Some(number);
    run.uuid = Some(uuid);
    // The page is built from the account's server, never taken from the response.
    run.link = link(Some(&format!("{}/{}/pipelines/results/{number}", config.server(), repo.encoded())), config);
    let target = row.get("target");
    let pull = target.and_then(|t| t.get("pullrequest")).and_then(|p| p.u64("id"));
    run.branch = branch_name(target.and_then(|t| match pull {
        Some(_) => t.str("source"),
        None => (t.str("ref_type") == Some("branch")).then(|| t.str("ref_name")).flatten(),
    }));
    run.event = match pull {
        Some(n) => Some(format!("pull request #{n}")),
        None => row.get("trigger").and_then(|t| t.str("name")).map(|t| match t {
            "PUSH" => "push".into(),
            "MANUAL" => "manual".into(),
            "SCHEDULE" => "schedule".into(),
            other => words(&other.to_ascii_lowercase()),
        }),
    };
    run.commit = target.and_then(|t| t.get("commit")).and_then(|c| c.str("hash")).and_then(oid);
    run.created = row.str("created_on").and_then(timestamp);
    if !matches!(run.state, ForgeCheckState::Running | ForgeCheckState::Queued) {
        run.took = row.u64("duration_in_seconds");
    }
    Some(run)
}
/// "{0f8e0a22-6a9e-4b5e-8a8f-2d1c3b4a5e6f}"
fn bitbucket_uuid(value: &str) -> bool {
    let Some(inner) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) else { return false };
    let parts: Vec<&str> = inner.split('-').collect();
    parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn bitbucket_step(row: &Json, link: Option<ForgeCheckLink>) -> Option<ForgeCheck> {
    let name = text(row, "name", 256).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Step".into());
    let state = row.get("state")?;
    let took = span(row.str("started_on").and_then(timestamp), row.str("completed_on").and_then(timestamp)).map(ci_duration_text);
    let (state, detail) = match state.str("name")? {
        "COMPLETED" => match state.get("result").and_then(|r| r.str("name"))? {
            "SUCCESSFUL" => (ForgeCheckState::Passed, took.unwrap_or_else(|| "Passed".into())),
            "FAILED" | "ERROR" => (ForgeCheckState::Failed, took.map_or("Failed".into(), |t| format!("Failed after {t}"))),
            "STOPPED" | "EXPIRED" => (ForgeCheckState::Cancelled, "Stopped".into()),
            "NOT_RUN" => (ForgeCheckState::Skipped, "Not run".into()),
            _ => (ForgeCheckState::Neutral, "Completed".into()),
        },
        "IN_PROGRESS" | "RUNNING" => (ForgeCheckState::Running, "Running".into()),
        "PENDING" | "READY" => (ForgeCheckState::Queued, "Pending".into()),
        "PAUSED" | "HALTED" => (ForgeCheckState::Queued, "Paused".into()),
        "NOT_RUN" => (ForgeCheckState::Skipped, "Not run".into()),
        _ => return None,
    };
    Some(ForgeCheck { name, group: String::new(), state, detail, link, run: None })
}
/// An Azure Pipelines build as a run.
fn azure_run(row: &Json, config: &ForgeConfig) -> Option<ForgeCiRun> {
    let id = row.u64("id").filter(|n| *n > 0)?;
    let state = match (row.str("status")?, row.str("result")) {
        ("completed", Some("succeeded")) => ForgeCheckState::Passed,
        ("completed", Some("failed")) => ForgeCheckState::Failed,
        ("completed", Some("canceled")) => ForgeCheckState::Cancelled,
        ("completed", _) => ForgeCheckState::Neutral,
        ("inProgress" | "cancelling", _) => ForgeCheckState::Running,
        ("notStarted" | "postponed" | "none", _) => ForgeCheckState::Queued,
        _ => return None,
    };
    let mut run = ForgeCiRun::new(id, row.get("definition").and_then(|d| text(d, "name", 200)).unwrap_or_else(|| "Build".into()), state);
    run.build = text(row, "buildNumber", 64);
    run.link = link(row.get("_links").and_then(|l| l.get("web")).and_then(|w| w.str("href")), config);
    let source = row.str("sourceBranch");
    let pull = source.and_then(|s| s.strip_prefix("refs/pull/")).and_then(|r| r.split('/').next()).and_then(|n| n.parse::<u64>().ok());
    if pull.is_none() && source.is_some_and(|s| s.starts_with("refs/heads/")) {
        run.branch = branch_name(source);
    }
    run.event = row.str("reason").map(|r| match r {
        "individualCI" | "batchedCI" => "push".into(),
        "pullRequest" => pull.map_or_else(|| "pull request".into(), |n| format!("pull request !{n}")),
        "schedule" => "schedule".into(),
        "manual" | "userCreated" => "manual".into(),
        "buildCompletion" => "after another pipeline".into(),
        other => words(other),
    });
    run.commit = row.str("sourceVersion").and_then(oid);
    run.created = row.str("queueTime").and_then(timestamp);
    run.started = row.str("startTime").and_then(timestamp);
    run.took = span(run.started, row.str("finishTime").and_then(timestamp));
    Some(run)
}
/// One job of an Azure build's timeline.
fn azure_job(row: &Json, link: Option<ForgeCheckLink>) -> Option<ForgeCheck> {
    let name = text(row, "name", 256)?;
    let took = span(row.str("startTime").and_then(timestamp), row.str("finishTime").and_then(timestamp)).map(ci_duration_text);
    let (state, detail) = match (row.str("state")?, row.str("result")) {
        ("completed", Some("succeeded")) => (ForgeCheckState::Passed, took.unwrap_or_else(|| "Succeeded".into())),
        ("completed", Some("succeededWithIssues")) => (ForgeCheckState::Passed, "Succeeded with issues".into()),
        ("completed", Some("failed")) => (ForgeCheckState::Failed, took.map_or("Failed".into(), |t| format!("Failed after {t}"))),
        ("completed", Some("canceled" | "abandoned")) => (ForgeCheckState::Cancelled, "Canceled".into()),
        ("completed", Some("skipped")) => (ForgeCheckState::Skipped, "Skipped".into()),
        ("completed", _) => (ForgeCheckState::Neutral, "Completed".into()),
        ("inProgress", _) => (ForgeCheckState::Running, "Running".into()),
        ("pending", _) => (ForgeCheckState::Queued, "Pending".into()),
        _ => return None,
    };
    Some(ForgeCheck { name, group: String::new(), state, detail, link, run: None })
}
fn gitlab_pipeline_state(status: &str) -> Result<ForgeCiState> {
    Ok(match status {
        "success" => ForgeCiState::Passed,
        "failed" | "canceled" => ForgeCiState::Failed,
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" | "waiting_for_callback" | "scheduled" | "canceling" | "manual" => ForgeCiState::Running,
        "skipped" => ForgeCiState::NotChecked,
        _ => return fail(ForgeFailure::InvalidResponse),
    })
}
fn gitlab_job(job: &Json, config: &ForgeConfig, group: &str, pipeline: u64) -> Option<ForgeCheck> {
    let name = text(job, "name", 256)?;
    let took = seconds(job.get("duration")).map(ci_duration_text);
    let allowed = job.bool("allow_failure") == Some(true);
    let (state, detail) = match job.str("status")? {
        "success" => (ForgeCheckState::Passed, took.unwrap_or_else(|| "Passed".into())),
        "failed" if allowed => (ForgeCheckState::Neutral, "Failed, allowed to fail".into()),
        "failed" => (ForgeCheckState::Failed, took.map_or("Failed".into(), |t| format!("Failed after {t}"))),
        "canceled" | "canceling" => (ForgeCheckState::Cancelled, "Canceled".into()),
        "running" => (ForgeCheckState::Running, "Running".into()),
        "manual" => (ForgeCheckState::Queued, "Manual".into()),
        "pending" | "created" | "preparing" | "waiting_for_resource" | "waiting_for_callback" | "scheduled" => (ForgeCheckState::Queued, "Queued".into()),
        "skipped" => (ForgeCheckState::Skipped, "Skipped".into()),
        _ => return None,
    };
    let stage = text(job, "stage", 64);
    Some(ForgeCheck {
        name: match stage { Some(s) if !s.is_empty() => format!("{s} · {name}"), _ => name },
        group: group.into(),
        state,
        detail,
        link: link(job.str("web_url"), config),
        run: Some(pipeline),
    })
}
fn azure_build(row: &Json, config: &ForgeConfig) -> Option<ForgeCheck> {
    let name = row.get("definition").and_then(|d| text(d, "name", 200)).unwrap_or_else(|| "Build".into());
    let number = text(row, "buildNumber", 64);
    let took = span(row.str("startTime").and_then(timestamp), row.str("finishTime").and_then(timestamp)).map(ci_duration_text);
    let (state, detail) = match (row.str("status")?, row.str("result")) {
        ("completed", Some("succeeded")) => (ForgeCheckState::Passed, took.unwrap_or_else(|| "Succeeded".into())),
        ("completed", Some("partiallySucceeded")) => (ForgeCheckState::Neutral, "Partially succeeded".into()),
        ("completed", Some("failed")) => (ForgeCheckState::Failed, took.map_or("Failed".into(), |t| format!("Failed after {t}"))),
        ("completed", Some("canceled")) => (ForgeCheckState::Cancelled, "Canceled".into()),
        ("completed", _) => (ForgeCheckState::Neutral, "Completed".into()),
        ("inProgress" | "cancelling", _) => (ForgeCheckState::Running, "Running".into()),
        ("notStarted" | "postponed" | "none", _) => (ForgeCheckState::Queued, "Queued".into()),
        _ => return None,
    };
    let web = row.get("_links").and_then(|l| l.get("web")).and_then(|w| w.str("href"));
    Some(ForgeCheck { name, group: number.map_or_else(|| "Build".into(), |n| format!("Build {n}")), state, detail, link: link(web, config), run: None })
}
fn azure_status(row: &Json, config: &ForgeConfig) -> Option<ForgeCheck> {
    let context = row.get("context")?;
    let name = text(context, "name", 200)?;
    let label = match text(context, "genre", 100) {
        Some(genre) if !genre.is_empty() => format!("{genre}/{name}"),
        _ => name,
    };
    let mut check = status_check(row, config, "state", "state", "description", "targetUrl")?;
    check.name = label;
    Some(check)
}
