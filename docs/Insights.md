# Insights

Repository activity accepts author dates from years 1 through 9999, including the
author's local time after its timezone offset. Invalid offsets and out-of-range
dates return a readable error rather than overflowing or displaying wrapped dates.
The repository and local work remain unchanged.

Insights extends the former **Statistics** page. This is pending in the next build;
the published 0.92.3 packages still show Statistics. Open **Insights** in the sidebar
or **View → Go to → Insights**, or search for Insights in the command palette.
Reads do not modify the repository or upload local Git history.

**Activity** keeps the commit timeline, contributors, weekday/hour charts and
table view. Choose a period and this branch or all branches. Counts use author
dates and mailmapped identities. The walk is limited to 100,000 commits and
discloses truncation; it cannot claim completeness beyond that bound. Contributor
counts describe recorded commits, not productivity. **Files** retains the existing
treemap of files at HEAD and their change frequency.

**Pull requests** uses the Hosting account matched to the repository's remote.
Connect an account in Hosting if needed, then Refresh. Rocket reads 50 PRs per
page. **Load more**, in the Partial sample strip, extends the sample, up to 10 pages /
500 unique PRs. It does
not crawl every PR or automatically load each PR's comments/reviews. API reads go to the configured
host using Rocket's existing guarded transport; credentials and PR payloads are
excluded from diagnostics. No new analytics service is involved.

| Metric | Definition |
| --- | --- |
| Opened | Loaded PRs whose creation date falls in the selected period |
| Merged | Loaded PRs with a known merge date in the selected period |
| Open now | Open PRs across the entire loaded sample, regardless of period |
| Median merge time | Creation to merge, for PRs merged during the period with both valid dates |
| Median open age | Creation to last refresh, for loaded open PRs with valid creation dates |

Missing, invalid, future or backwards dates are excluded from the affected timing
calculations. Zero is not substituted for unknown data. Each median shows its sample
count, and a row whose dates can't be used is marked **No dates**, or **No merge
date** when only that one is unusable. Drafts are included
in open PRs. The **Author** dropdown selects an exact
author from the loaded sample and updates both the summary and visible rows.
These are hosting identities, not locally mailmapped Git authors. **All / Open /
Merged**, each with its count for the chosen author, changes only the rows. Period
choices apply to the summary; rows
still include all loaded dates. A selected author that disappears after Refresh
stays selected with an empty result; choose All authors to reset it.
**Partial sample** means more pages exist or pagination observed a
duplicate while the host changed. Offset pagination is not an atomic snapshot;
Refresh starts a fresh sample. Duplicate PR numbers are counted once.

**Trends** (the List | Trends switch) charts opened and merged PRs per week of the
chosen period, weeks from Monday in UTC, for the chosen author; past 53 weeks (as in
All) per month, and past 48 months per year. The bars add up to the Opened and Merged
tiles. Beside the chart, the period is compared with the period of the same length
before it: Opened, Merged and Median merge time with both medians' sample counts, and
the tiles show the change. A comparison needs both periods in the loaded sample.
GitHub, GHES, GitLab, Gitea and Bitbucket Cloud lists are read newest-updated first,
and opening or merging a PR updates it, so a partial sample holds every PR opened or
merged after the oldest update loaded (rows updated exactly then may continue on the
next page). Weeks before that are drawn as **Not loaded**, the change is left out and
**Load more** is offered. Azure DevOps and Bitbucket Server don't document their list
order, so their partial samples are not compared. A list that changed while it was
read (duplicates across pages) is not compared either.

**Saved views**: each repository remembers its Insights tab and choices (Activity's
period, branches and table; the PR period, author, state filter and List/Trends;
Files' period) in Rocket's state file (format v30, written only when a repository's
choices differ from the defaults). While the repository's tab is open the page keeps
its numbers: leaving Insights and returning shows them without a read, and each
filter row says when they were refreshed, with the local time in its tooltip. Nothing
reads by itself; Refresh, or a choice that needs a new read, does. Activity and Files also
read again when shown after HEAD (or, for all branches, any branch tip) moved, compared
with the tab's last status read; the numbers they kept are dropped first. The numbers on screen
stay, labelled for the choices they were read for, until the new ones replace them.
Leaving the page stops its reads and closes an export review. Numbers are kept in
memory only, never on disk, so a restart reads them again.

Select a **PR row** to read its description, commits, reviews, changed files,
discussion and reported checks. The existing Hosting adapters bound each detail
collection and validate the revision again before displaying it. Missing,
permission-denied and truncated collections are labeled explicitly; no missing
check is counted as success. The view is read-only and cannot merge or submit a
review. Text is plain data: descriptions cannot load external images or execute
commands. Large text previews disclose their 8,192-character display limit.
The refresh button on the details card reads again. Switching PRs/authors, closing
details, or leaving the repository cancels obsolete reads; late replies cannot replace
the new selection. From about 930 points of page width the list and the details sit
beside each other and fill the rest of the window, each scrolling by itself; below
that the details open under a bounded, scrollable list and the page scrolls.

**Reviews** (pending in the next build). The **Reviewer** field beside Author
narrows both the numbers and the rows to the pull requests a person or team was
asked to review or reviewed, and lists each with how many pull requests wait on
them and how many they reviewed. **Waiting**, beside Open and Merged, lists open pull
requests with a review request still waiting (on the chosen reviewer, when there
is one). Saved views keep the Waiting filter; the chosen reviewer lasts while the
repository's tab is open and is not saved.

| Figure | Definition |
| --- | --- |
| Waiting for review | Open PRs across the loaded sample with a request still waiting, and the number of requests |
| Time to first review | Median from creation to the first review by someone other than the author, for PRs opened in the period whose reviews were read |

Waiting requests come with the list Rocket already reads, so they cost no extra
request: GitHub and GitHub Enterprise Server `requested_reviewers`/`requested_teams`
(GitHub drops a requested reviewer once they submit a review), Azure DevOps reviewers with vote 0
that have not declined, Bitbucket Server reviewers with status `UNAPPROVED`, and
Bitbucket Cloud reviewers without an approval or request for changes. Bitbucket Cloud
leaves `reviewers` and `participants` out of list responses unless asked, so the
Insights read alone asks for them with `fields=+values.reviewers,+values.participants`;
the Hosting page's list does not. GitLab's list `reviewers` carry the account's
state, not a review state, and Gitea's list `requested_reviewers` hold everyone whose
latest review is a request, an approval or a rejection (and its list leaves the teams
empty), so neither says who still waits: those hosts have no Waiting for review and the
filter is disabled; their reviewers still populate the Reviewer field. A row whose reviewers are
missing, malformed or more than 100 is treated as not saying, never as waiting on
no one.

Time to first review needs each PR's reviews, one request per PR, so it is read only
when **Read reviews** is chosen: the PRs opened in the period, newest first, at most
100 per click; **Read more** continues and **Stop** keeps what was read. Refresh
clears them with the sample. Only GitHub, GitHub Enterprise Server and Gitea report
when a review was submitted
([GitHub reviews](https://docs.github.com/en/rest/pulls/reviews#list-reviews-for-a-pull-request),
[Gitea reviews](https://docs.gitea.com/api/1.25/operations/repo-list-pull-reviews/)).
GitHub returns reviews in chronological order, so its pages are read, up to three of
100, only until a review counts; Gitea's are read two pages of 50 at most. More reviews
than that, with no first review told, mark the pull request incomplete. Each read
checks review times against the clock at that moment, allowing five minutes for a host
clock that runs ahead; a later time counts only as unusable, and only when no earlier
review is usable. Pending (unsubmitted) reviews, Gitea review requests, the author's own
reviews and bots (GitHub `type` Bot or a `[bot]` login) never count as the first
review. A review time that is missing, in the future or before the PR's creation
leaves that PR out. Time as a draft counts. With a reviewer chosen, the time is still
each PR's first review by anyone. The reviewer narrows the list only: Trends and its
period comparison count every reviewer, because a GitHub reviewer's PRs are known only
as far as their reviews were read. On other hosts the figure says it is not measured:
Azure DevOps votes and Bitbucket approvals carry no time in the list.

Missing data is named rather than estimated: the figure shows a dash and why
(reviews not read, not on the host), the strip names PRs whose reviews could not be
read, have no usable time or have more reviews than read, and those rows say
**Reviews unavailable**, **No review date** or **Some reviews not read**. A row whose
reviews were not read shows nothing about them. A refused sign-in stops the whole
read and offers Open Hosting; a fault in one PR's reviews (not found or malformed)
marks that PR and the read continues.

**Export** opens a review of the currently visible rows. The reviewed sample is
frozen: saving does not fetch more pages or follow links. Choose CSV or JSON,
an existing folder and a new filename. Existing files are never overwritten,
including a file created by another process while the export is prepared. Close
the review to discard it; **Stop** cancels an active save before publication. A
Stop arriving after the complete file has been published reports success, rather
than claiming no file was created. Saving requires a filesystem that supports
hard links (such as NTFS or APFS); unsupported filesystems return an error.

- CSV uses UTF-8 with a BOM and quoted fields. It includes provider, server,
  repository, snapshot time, coverage, filters and PR row metadata. Formula-like
  text receives an apostrophe prefix for spreadsheet safety.
- JSON schema version 1 preserves the original strings, includes the same scope
  and rows, and adds summary metrics with their sample counts. The summary uses
  the author- and reviewer-filtered sample before the state filter. Timestamps are
  Unix seconds; unknown dates remain null. It is the exact-text alternative to
  spreadsheet CSV.
- Review data is additive. CSV appends `reviewer_filter`, `review_requests`
  (names joined by `; `, empty for a closed PR, or `not_in_list` / `no_review_state`), `reviews`, `first_review_unix_seconds`
  and `first_reviewer` after the earlier columns. JSON adds `filters.reviewer`,
  each item's `review_requests` (only open PRs wait; null when the list does not say,
with `review_requests_unknown` saying `not_in_list` or `no_review_state`), `reviews`,
  `first_review_unix_seconds` and `first_reviewer`, and the review figures under
  `metrics`. `reviews` is `read`, `not_read`, `unavailable`, `no_usable_time`,
  `incomplete` or `not_measured`; the state filter can be `waiting`.

**Export** on the Activity row reviews the commit activity shown, under the same rules
(frozen copy, new file only, Stop, 16 MiB). CSV holds one table a file: Contributors
(name after .mailmap and commits, most first; with email only when *Include email
addresses* is ticked) or Commits per week (every week from the first to the last, empty
weeks included, dated by the Monday of each commit's own date). Every row repeats the
scope: repository folder name, branches, period, since and snapshot times, commits
counted and whether the 100,000-commit bound was reached. JSON (`kind`
`commit_activity`, schema version 1) holds the scope, coverage, totals, contributors,
weeks, months, weekdays, hours and the weekday-by-hour grid. Names that aren't valid
UTF-8 are converted lossily. The repository's path is never written.

Both formats include repository/PR metadata that may be private. Review before
sharing. Credentials, PR descriptions, comments, reviews and file contents are
excluded. Exports are limited to 16 MiB; automated exports stay inside the MCP
fixture root. Neither format claims a complete remote history when the sample is
partial, and offset pagination never claims an atomic repository snapshot.

The date fields are checked against official
[GitHub](https://docs.github.com/en/rest/pulls/pulls#list-pull-requests),
[GitLab](https://docs.gitlab.com/api/merge_requests/),
[Gitea](https://docs.gitea.com/api/1.25/operations/repo-list-pull-requests/),
[Azure DevOps](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get-pull-requests?view=azure-devops-rest-7.1)
and [Bitbucket](https://developer.atlassian.com/cloud/bitbucket/rest/api-group-pullrequests/)
list contracts. GHES and Azure DevOps Server use their existing adapters. Bitbucket
Cloud does not expose a reliable merge-event timestamp in this list response;
updated time is deliberately not used. Server responses without merge dates are
likewise marked incomplete. Bitbucket Server's `createdDate` and `closedDate` fields
are documented in its [REST model](https://docs.atlassian.com/bitbucket-server/javadoc/5.16.0/rest-model/reference/com/atlassian/bitbucket/rest/pull/RestPullRequest.html);
closed dates contribute only for a merged PR. CI delivery/DORA and AI-usage metrics
are outside these screens.

**Stop** cancels the current read and keeps any previous successful sample. Errors
also retain that sample and identify the failed request. Closing the page or
switching the account/repository cancels pending work; late replies cannot replace
the new view. Refresh retries explicitly.

Automation keeps the compatible `view-statistics`, `goto-statistics` and
`stats-section-activity`/`stats-section-treemap` IDs. New controls include
`stats-section-pulls`, `insights-period-30/90/365/all`, `insights-more`,
`insights-filter-0/1/2/3` (All, Open, Merged, Waiting) and `insights-hosting`. The
author dropdown uses `insights-author`, `insights-author-all` and sorted
`insights-author-{index}`; the reviewer dropdown `insights-reviewer`,
`insights-reviewer-all` and `insights-reviewer-{index}` (people, then teams). Read
reviews, Read more and Read reviews again are `insights-reviews-read`, and a refused sign-in
during it offers `insights-reviews-hosting`;
rows use `insights-pr-{number}`. Details use `insights-detail-tab-0/1/2/3`,
`insights-detail-refresh/stop/close/retry`; `insights-again` reads the list again
after a failed or changed read. Export uses `insights-export`,
`insights-export-csv/json`, `insights-export-folder/name/browse/save/stop/close`. Trends uses
`insights-view-list/trends`, `insights-trend-chart`, `insights-compare-more` and
`insights-compare-again`; the activity export opens from `stats-export` and adds
`insights-export-table-contributors/weeks` and `insights-export-emails`. Activity and
Files show `stats-refreshed`. MCP state reports `trends`, `comparison`, `coverage`,
`previous` and `trend` under `pulls`, `activity_at`/`treemap_at`, and every export
under `statistics.export` (a PR export also under `pulls.export`).
Read counters and detail/export lifecycle state through
`dialog.statistics.pulls` in localhost MCP. Tests use synthetic temporary repos and
loopback APIs; they do not establish live-provider acceptance. The new labels use
Rocket's English fallback where preview translations are not available yet.
