//! Read-only repository statistics: commit activity from one bounded `git log`
//! read, and an activity treemap of the files of HEAD (`git ls-tree`) with how many
//! commits changed each path in a period (one bounded `git log`).
use crate::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
};

/// At most this many commits are counted for commit activity; more set `limited`.
pub const ACTIVITY_COMMITS: usize = 100_000;
/// At most this many commits are read for the treemap's change counts.
pub const TREEMAP_COMMITS: usize = 20_000;

/// Which history `commit_activity` reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityScope {
    /// Local and remote-tracking branches instead of HEAD.
    pub all_branches: bool,
    /// Only commits authored at or after this time (Unix seconds).
    pub since: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorActivity {
    /// The name on the author's newest commit (after `.mailmap`).
    pub name: Vec<u8>,
    pub email: Vec<u8>,
    pub commits: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitActivity {
    /// Commits authored in the range, merges included.
    pub commits: usize,
    pub merges: usize,
    /// Oldest and newest author dates (Unix seconds).
    pub first: Option<i64>,
    pub last: Option<i64>,
    /// Authors grouped by email (ignoring case), most commits first, then by name.
    pub authors: Vec<AuthorActivity>,
    /// (year, month 1–12, commits) for months with commits, oldest first.
    pub months: Vec<(i32, u32, usize)>,
    /// Monday first, each commit in its own time zone.
    pub weekdays: [usize; 7],
    /// Hour of day, each commit in its own time zone.
    pub hours: [usize; 24],
    /// Commits by weekday (Monday first) and hour, each in its own time zone.
    pub heat: [[usize; 24]; 7],
    /// (day number of the week's Monday, commits) for weeks with commits, oldest
    /// first. Days count from 1970-01-01, by each commit's own date.
    pub weeks: Vec<(i64, usize)>,
    /// Calendar days with at least one commit, by each commit's own date.
    pub days: usize,
    /// More commits matched than were counted.
    pub limited: bool,
}

/// A file of the treemap's commit, or a folder with everything below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreemapNode {
    /// The last path component; the root's is empty.
    pub name: Vec<u8>,
    pub folder: bool,
    /// Bytes of the file, or of every file below the folder.
    pub size: u64,
    /// Files below a folder; 1 for a file.
    pub files: usize,
    /// Non-merge commits in the period that changed this file, or anything below this
    /// folder (each commit once).
    pub changes: usize,
    /// A folder's entries, largest first; empty for a file.
    pub children: Vec<TreemapNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityTreemap {
    /// The commit whose files are shown (HEAD when read).
    pub commit: ObjectId,
    pub root: TreemapNode,
    /// Commits read for the change counts.
    pub commits: usize,
    /// More commits changed files in the period than were read.
    pub limited: bool,
}

impl Repository {
    /// Commit activity of HEAD or of all branches, read with one bounded `git log`.
    pub fn commit_activity(
        &self,
        scope: &ActivityScope,
        cancel: &CancellationToken,
    ) -> Result<CommitActivity> {
        self.runner.step("commit-activity", "read-history", || {
            let mut arguments: Vec<OsString> = [
                "log",
                "-z",
                "--no-show-signature",
                "--date=raw",
                "--format=%ad%x00%P%x00%aE%x00%aN",
            ]
            .into_iter()
            .map(OsString::from)
            .collect();
            arguments.push(format!("--max-count={}", ACTIVITY_COMMITS + 1).into());
            // This view counts author dates. Git's --since filters committer dates
            // and can prune ancestry at a skewed timestamp, hiding valid rows.
            // Read a bounded walk and apply the author-date filter in tally().
            if scope.all_branches {
                arguments.extend(["--branches", "--remotes"].map(OsString::from));
            } else {
                if matches!(self.head(cancel)?, Head::Unborn { .. }) {
                    return Ok(CommitActivity::default());
                }
                arguments.push("HEAD".into());
            }
            arguments.extend(["--"].map(OsString::from));
            let output = self
                .runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?
                .success("commit-activity")?;
            cancel.check()?;
            tally(&output, scope.since, ACTIVITY_COMMITS)
        })
    }

    /// The files of HEAD as a treemap, with how many commits since `since` (Unix
    /// seconds; all commits when `None`) changed each file and folder. Paths are
    /// counted as they are named at each commit; renames are not followed.
    pub fn activity_treemap(
        &self,
        since: Option<i64>,
        cancel: &CancellationToken,
    ) -> Result<ActivityTreemap> {
        let commit = match self.head(cancel)? {
            Head::Branch { target, .. } | Head::Detached { target } => target,
            Head::Unborn { .. } => {
                return Err(Error::InvalidInput(
                    "The repository has no commits yet.".into(),
                ));
            }
        };
        self.prepare_tree(commit.as_str(), cancel)?;
        let listing = self.runner.step("activity-treemap", "list-files", || {
            self.runner
                .run(
                    self.cwd(),
                    &["ls-tree", "-r", "-l", "-z", "--full-tree", commit.as_str()]
                        .map(OsString::from),
                    Vec::new(),
                    cancel,
                )?
                .success("activity-treemap")
        })?;
        let log = self.runner.step("activity-treemap", "count-changes", || {
            let mut arguments: Vec<OsString> = [
                "log",
                "-z",
                "--no-merges",
                "--no-renames",
                "--name-only",
                "--format=%x01",
            ]
            .into_iter()
            .map(OsString::from)
            .collect();
            arguments.push(format!("--max-count={}", TREEMAP_COMMITS + 1).into());
            if let Some(since) = since {
                arguments.push(format!("--since={since}").into());
            }
            arguments.extend([commit.as_str(), "--"].map(OsString::from));
            self.runner
                .run(self.cwd(), &arguments, Vec::new(), cancel)?
                .success("activity-treemap")
        })?;
        cancel.check()?;
        let files = parse_listing(&listing)?;
        let (counts, commits) = count_changes(&log, TREEMAP_COMMITS);
        cancel.check()?;
        Ok(ActivityTreemap {
            commit,
            root: build_tree(&files, &counts),
            commits: commits.min(TREEMAP_COMMITS),
            limited: commits > TREEMAP_COMMITS,
        })
    }
}

/// Count `git log -z --date=raw --format=%ad%x00%P%x00%aE%x00%aN` records, keeping
/// commits authored at or after `since` within the first `limit` walked records.
fn tally(output: &[u8], since: Option<i64>, limit: usize) -> Result<CommitActivity> {
    let mut fields: Vec<&[u8]> = output.split(|b| *b == 0).collect();
    if fields.last().is_some_and(|f| f.is_empty()) {
        fields.pop();
    }
    if !fields.len().is_multiple_of(4) {
        return Err(Error::parse("commit-activity", "invalid record"));
    }
    let mut activity = CommitActivity::default();
    let mut authors: HashMap<Vec<u8>, AuthorActivity> = HashMap::new();
    let mut months: BTreeMap<(i32, u32), usize> = BTreeMap::new();
    let mut weeks: BTreeMap<i64, usize> = BTreeMap::new();
    let mut dates: HashSet<i64> = HashSet::new();
    activity.limited = fields.len() / 4 > limit;
    for record in fields.chunks(4).take(limit) {
        let date = std::str::from_utf8(record[0])
            .map_err(|_| Error::parse("commit-activity", "invalid record"))?;
        let (seconds, zone) = date
            .trim_start_matches('\n')
            .split_once(' ')
            .ok_or_else(|| Error::parse("commit-activity", "invalid record"))?;
        let seconds: i64 = seconds
            .parse()
            .map_err(|_| Error::parse("commit-activity", "invalid record"))?;
        let offset =
            zone_minutes(zone).ok_or_else(|| Error::parse("commit-activity", "invalid record"))?;
        if since.is_some_and(|s| seconds < s) {
            continue;
        }
        // Git permits dates well outside the calendar used by Insights. Validate
        // before any bucketing/casts, including an offset that crosses a boundary.
        const CIVIL_SECONDS: std::ops::RangeInclusive<i64> = -62_135_596_800..=253_402_300_799;
        let local = seconds.checked_add(i64::from(offset) * 60)
            .filter(|local| CIVIL_SECONDS.contains(&seconds) && CIVIL_SECONDS.contains(local))
            .ok_or_else(|| Error::parse("commit-activity", "author date is outside supported years 1 through 9999"))?;
        activity.commits += 1;
        if record[1]
            .split(|b| *b == b' ')
            .filter(|p| !p.is_empty())
            .count()
            > 1
        {
            activity.merges += 1;
        }
        activity.first = Some(activity.first.map_or(seconds, |f| f.min(seconds)));
        activity.last = Some(activity.last.map_or(seconds, |l| l.max(seconds)));
        let key = if record[2].is_empty() {
            record[3].to_vec()
        } else {
            record[2].to_ascii_lowercase()
        };
        // The log lists the newest commit first, so its name is kept.
        authors
            .entry(key)
            .or_insert_with(|| AuthorActivity {
                name: record[3].to_vec(),
                email: record[2].to_vec(),
                commits: 0,
            })
            .commits += 1;
        let days = local.div_euclid(86_400);
        *months.entry(year_month(days)).or_default() += 1;
        // 1970-01-01 was a Thursday; Monday is 0.
        let weekday = (days + 3).rem_euclid(7);
        let hour = (local.rem_euclid(86_400) / 3600) as usize;
        activity.weekdays[weekday as usize] += 1;
        activity.hours[hour] += 1;
        activity.heat[weekday as usize][hour] += 1;
        *weeks.entry(days - weekday).or_default() += 1;
        dates.insert(days);
    }
    activity.weeks = weeks.into_iter().collect();
    activity.days = dates.len();
    activity.authors = authors.into_values().collect();
    activity
        .authors
        .sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
    activity.months = months.into_iter().map(|((y, m), n)| (y, m, n)).collect();
    Ok(activity)
}

/// `+hhmm` or `-hhmm` as minutes east of UTC.
fn zone_minutes(zone: &str) -> Option<i32> {
    let (sign, digits) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => return None,
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i32 = digits.parse().ok()?;
    (value / 100 < 24 && value % 100 < 60)
        .then_some(sign * (value / 100 * 60 + value % 100))
}

/// The day, counted from 1970-01-01, of a civil date (Howard Hinnant's algorithm, the
/// inverse of [`year_month`]'s).
pub(crate) fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// The civil year and month of a day counted from 1970-01-01.
pub(crate) fn year_month(days: i64) -> (i32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as i32, month as u32)
}

/// `git ls-tree -r -l -z` entries as (path, size); submodules have no size and are left out.
fn parse_listing(output: &[u8]) -> Result<Vec<(Vec<u8>, u64)>> {
    let mut files = Vec::new();
    for entry in output.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| Error::parse("activity-treemap", "invalid file entry"))?;
        let meta = std::str::from_utf8(&entry[..tab])
            .map_err(|_| Error::parse("activity-treemap", "invalid file entry"))?;
        let mut parts = meta.split_ascii_whitespace();
        let (_mode, kind, _object, size) = (parts.next(), parts.next(), parts.next(), parts.next());
        match (kind, size) {
            (Some("blob"), Some(size)) => {
                let size = size
                    .parse()
                    .map_err(|_| Error::parse("activity-treemap", "invalid file entry"))?;
                files.push((entry[tab + 1..].to_vec(), size));
            }
            (Some("commit"), _) => {}
            _ => return Err(Error::parse("activity-treemap", "invalid file entry")),
        }
    }
    Ok(files)
}

/// Change counts from `git log -z --name-only --format=%x01`: for each file path and for
/// each folder (with a trailing `/`; the root is empty), how many commits changed it.
/// Only the first `limit` commits count. Also returns how many commits were listed.
fn count_changes(output: &[u8], limit: usize) -> (HashMap<Vec<u8>, usize>, usize) {
    let mut counts: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut commits = 0;
    let mut touched: HashSet<Vec<u8>> = HashSet::new();
    let mut first = false;
    let flush = |touched: &mut HashSet<Vec<u8>>, counts: &mut HashMap<Vec<u8>, usize>| {
        for key in touched.drain() {
            *counts.entry(key).or_default() += 1;
        }
    };
    for token in output.split(|b| *b == 0) {
        if token.first() == Some(&1) {
            flush(&mut touched, &mut counts);
            commits += 1;
            first = true;
            continue;
        }
        // Git puts a line break between the format line and the first name.
        let path = if std::mem::take(&mut first) {
            token.strip_prefix(b"\n").unwrap_or(token)
        } else {
            token
        };
        if path.is_empty() || commits > limit {
            continue;
        }
        touched.insert(Vec::new());
        for (i, b) in path.iter().enumerate() {
            if *b == b'/' {
                touched.insert(path[..=i].to_vec());
            }
        }
        touched.insert(path.to_vec());
    }
    flush(&mut touched, &mut counts);
    (counts, commits)
}

#[derive(Default)]
struct Folder {
    files: Vec<(Vec<u8>, u64)>,
    folders: BTreeMap<Vec<u8>, Folder>,
}

fn build_tree(files: &[(Vec<u8>, u64)], counts: &HashMap<Vec<u8>, usize>) -> TreemapNode {
    let mut root = Folder::default();
    for (path, size) in files {
        let mut folder = &mut root;
        let mut parts = path.split(|b| *b == b'/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                folder.files.push((part.to_vec(), *size));
            } else {
                folder = folder.folders.entry(part.to_vec()).or_default();
            }
        }
    }
    node(Vec::new(), &[], root, counts)
}

fn node(
    name: Vec<u8>,
    prefix: &[u8],
    folder: Folder,
    counts: &HashMap<Vec<u8>, usize>,
) -> TreemapNode {
    let mut children: Vec<TreemapNode> = folder
        .files
        .into_iter()
        .map(|(file, size)| {
            let path = [prefix, &file].concat();
            TreemapNode {
                changes: counts.get(&path).copied().unwrap_or(0),
                name: file,
                folder: false,
                size,
                files: 1,
                children: Vec::new(),
            }
        })
        .collect();
    for (child, inner) in folder.folders {
        let path = [prefix, &child, b"/"].concat();
        children.push(node(child, &path, inner, counts));
    }
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    TreemapNode {
        changes: counts.get(prefix).copied().unwrap_or(0),
        size: children.iter().map(|c| c.size).sum(),
        files: children.iter().map(|c| c.files).sum(),
        name,
        folder: true,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_rejects_extreme_dates_and_invalid_timezones_without_panicking() {
        for date in [
            "9223372036854775807 +0100",
            "-9223372036854775808 -0100",
            "9223372036854775807 +0000",
            "253402300800 +0000",
            "-62135596801 +0000",
            "253402300799 +0001",
            "-62135596800 -0001",
            "1705307400 +1260",
            "1705307400 -2400",
        ] {
            let record = format!("{date}\0\0test@example.invalid\0Test\0");
            assert!(tally(record.as_bytes(), None, 1).is_err(), "{date}");
        }
        for (date, month) in [
            ("-62135596800 +0000", (1, 1, 1)),
            ("253402300799 +0000", (9999, 12, 1)),
            ("-1 +0000", (1969, 12, 1)),
        ] {
            let record = format!("{date}\0\0test@example.invalid\0Test\0");
            let activity = tally(record.as_bytes(), None, 1).unwrap();
            assert_eq!(activity.months, [month]);
            assert_eq!(activity.commits, 1);
        }
    }

    #[test]
    fn activity_groups_authors_and_buckets_by_local_time() {
        // Newest first: Bob's merge (Sunday 12:00 +09:00), Alice as "Alice A."
        // (Saturday 08:00 UTC), Bob (Saturday 23:15 -05:00), Alice (Monday 10:30 +02:00).
        let p = "a".repeat(40);
        let output = [
            format!("1709434800 +0900\0{p} {p}\0bob@example.invalid\0Bob\0"),
            format!("1709366400 +0000\0{p}\0ALICE@example.invalid\0Alice A.\0"),
            format!("1705810500 -0500\0{p}\0bob@example.invalid\0Bob\0"),
            "1705307400 +0200\0\0alice@example.invalid\0Alice\0".to_owned(),
        ]
        .concat();
        let a = tally(output.as_bytes(), None, 10).unwrap();
        assert_eq!((a.commits, a.merges, a.limited), (4, 1, false));
        assert_eq!((a.first, a.last), (Some(1705307400), Some(1709434800)));
        let authors: Vec<_> = a
            .authors
            .iter()
            .map(|x| (String::from_utf8_lossy(&x.name).into_owned(), x.commits))
            .collect();
        assert_eq!(authors, [("Alice A.".to_owned(), 2), ("Bob".to_owned(), 2)]);
        assert_eq!(a.months, [(2024, 1, 2), (2024, 3, 2)]);
        assert_eq!(a.weekdays, [1, 0, 0, 0, 0, 2, 1]);
        let hours: Vec<usize> = (0..24).filter(|&h| a.hours[h] > 0).collect();
        assert_eq!(hours, [8, 10, 12, 23]);
        // Weekday by hour, weeks by their Monday (2024-01-15 and 2024-02-26), and days.
        let cells: Vec<_> = (0..7)
            .flat_map(|d| (0..24).map(move |h| (d, h)))
            .filter(|&(d, h)| a.heat[d][h] > 0)
            .collect();
        assert_eq!(cells, [(0, 10), (5, 8), (5, 23), (6, 12)]);
        assert_eq!(a.weeks, [(19_737, 2), (19_779, 2)]);
        assert_eq!(a.days, 4);
        // Only commits authored since 2024-02-01, and at most one of them.
        let recent = tally(output.as_bytes(), Some(1706745600), 10).unwrap();
        assert_eq!((recent.commits, recent.merges), (2, 1));
        let limited = tally(output.as_bytes(), None, 1).unwrap();
        assert_eq!((limited.commits, limited.limited), (1, true));
        assert_eq!(tally(b"", None, 10).unwrap(), CommitActivity::default());
        assert!(tally(b"1 +0000\0\0x\0", None, 10).is_err());
        assert!(tally(b"soon +0000\0\0x\0y\0", None, 10).is_err());
    }
    #[test]
    fn dates_and_zones() {
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(-1), (1969, 12));
        assert_eq!(year_month(19_782), (2024, 2)); // 2024-02-29
        assert_eq!(year_month(19_783), (2024, 3));
        // The inverse, across a leap day, the epoch and a century that isn't leap.
        for (date, day) in [((1970, 1, 1), 0), ((2024, 2, 29), 19_782), ((2024, 3, 1), 19_783), ((1969, 12, 31), -1), ((2100, 3, 1), 47_541)] {
            assert_eq!(days_from_civil(date.0, date.1, date.2), day, "{date:?}");
            assert_eq!(year_month(day), (date.0, date.1));
        }
        assert_eq!(zone_minutes("+0530"), Some(330));
        assert_eq!(zone_minutes("-0800"), Some(-480));
        assert_eq!(zone_minutes("0800"), None);
    }
    #[test]
    fn change_counts_cover_files_and_folders_once_per_commit() {
        let output = b"\x01\0\nsrc/a.rs\0src/b.rs\0\x01\0\nsrc/a.rs\0docs/x.md\0\x01\0\nold.txt\0";
        let (counts, commits) = count_changes(output, 10);
        assert_eq!(commits, 3);
        let count = |k: &str| counts.get(k.as_bytes()).copied().unwrap_or(0);
        assert_eq!(
            (count("src/a.rs"), count("src/b.rs"), count("docs/x.md")),
            (2, 1, 1)
        );
        assert_eq!((count("src/"), count("docs/"), count("")), (2, 1, 3));
        // Only the newest commit counts with a limit of one.
        let (counts, commits) = count_changes(output, 1);
        assert_eq!(
            (commits, counts.get(&b"src/b.rs"[..]).copied()),
            (3, Some(1))
        );
        assert_eq!(counts.get(&b"docs/x.md"[..]), None);
    }
    #[test]
    fn listings_build_a_sized_tree_largest_first() {
        let listing = b"100644 blob 1111111111111111111111111111111111111111     100\tsrc/a.rs\0\
100644 blob 2222222222222222222222222222222222222222      50\tsrc/b.rs\0\
160000 commit 3333333333333333333333333333333333333333       -\tvendor/lib\0\
120000 blob 4444444444444444444444444444444444444444       7\tlink\0\
100644 blob 5555555555555555555555555555555555555555      10\tdocs/x.md\0";
        let files = parse_listing(listing).unwrap();
        assert_eq!(files.len(), 4);
        let counts = HashMap::from([
            (b"src/a.rs".to_vec(), 2),
            (b"src/".to_vec(), 2),
            (Vec::new(), 3),
        ]);
        let root = build_tree(&files, &counts);
        assert_eq!((root.size, root.files, root.changes), (167, 4, 3));
        let names: Vec<_> = root
            .children
            .iter()
            .map(|c| String::from_utf8_lossy(&c.name).into_owned())
            .collect();
        assert_eq!(names, ["src", "docs", "link"]);
        let src = &root.children[0];
        assert_eq!(
            (src.folder, src.size, src.files, src.changes),
            (true, 150, 2, 2)
        );
        assert_eq!((src.children[0].changes, src.children[1].changes), (2, 0));
        assert!(parse_listing(b"100644 blob x\0").is_err());
    }
}
