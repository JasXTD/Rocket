//! Code → Folders as a code map (canvas section 47). An open folder is a box holding
//! what is inside it, a closed folder or package is a card, and arrows show which
//! files use which, gathered between the cards shown: the number on an arrow counts
//! the files that use. What uses sits above what it uses; cards that connect to
//! nothing at their level sit under "Not connected". Colour by kind, activity (commits
//! in a period), owner (CODEOWNERS, else most commits) or changes against a base.
use super::*;
use std::collections::BTreeMap;

const GAP_X: f32 = 34.;
const GAP_Y: f32 = 58.;
const PAD: f32 = 14.;
const HEADER: f32 = 40.;
/// A box is at least this wide, so its title fits.
const BOX_MIN: f32 = 236.;
/// Cards in a row inside a box before the row wraps.
const ROW_ITEMS: usize = 4;
/// A folder's own files show one by one up to this many; more share one card.
const FILE_CARDS: usize = 8;
/// Cards and boxes laid out at most; past this, folders stay closed.
const ITEMS: usize = 400;
const CAPTION: f32 = 24.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Colour {
    #[default]
    Kind,
    Activity,
    Owner,
    Changes,
}
impl Colour {
    pub const ALL: [Colour; 4] = [Self::Kind, Self::Activity, Self::Owner, Self::Changes];
    pub fn name(self) -> &'static str {
        match self {
            Self::Kind => tr("Kind"),
            // Not "Activity": that is Rocket's own operations in some languages.
            Self::Activity => tr("Commit activity"),
            Self::Owner => tr("Owner"),
            Self::Changes => tr("Changes"),
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Activity => "activity",
            Self::Owner => "owner",
            Self::Changes => "changes",
        }
    }
}
/// A read the code map asks for.
pub enum CodeRead {
    /// Which files use which: the whole snapshot, or the packages in these folders.
    Links(Option<Vec<Vec<u8>>>),
    History,
    Changes,
}

/// The code map's reads and choices, beside the folders Code reads.
pub struct CodeMap {
    pub colour: Colour,
    pub period: Period,
    /// Changes: what the snapshot is compared with, as typed.
    pub base: String,
    /// Changes: Changed only, in place of Connected only.
    pub changed_only: bool,
    /// Changes: Compare was pressed; a running read gives way to one for the base typed now.
    pub compare_again: bool,
    /// Folders whose own files show one by one, by path; empty for the top folder.
    pub files_open: HashSet<Vec<u8>>,
    /// The chosen "N files" card, by its folder's node (None: the top folder).
    pub group: Option<Option<usize>>,
    pub links: Option<Arc<CodeLinks>>,
    pub history: Option<Arc<CodeHistory>>,
    pub changes: Option<Arc<CodeChanges>>,
    /// Why the links, history or Changes read failed, by read.
    pub errors: [Option<String>; 3],
    links_task: Option<ReadTask<CodeLinks>>,
    /// The packages the running links read is for; empty for the whole snapshot.
    requested: Vec<Vec<u8>>,
    /// Packages a folder read left unread: more code than one read takes.
    pub too_large: HashSet<Vec<u8>>,
    history_task: Option<ReadTask<CodeHistory>>,
    changes_task: Option<ReadTask<CodeChanges>>,
    /// A read that failed or was stopped for this snapshot is not asked for again until
    /// Refresh.
    failed: [bool; 3],
    /// Stop ended a read, which Refresh starts again.
    pub stopped: bool,
    /// The snapshot the reads are for.
    tip: Option<ObjectId>,
    pub layout: Option<Arc<Picture>>,
    /// Screen rectangles of the link counts painted in the last Folders frame.
    pub link_count_rects: Vec<Rect>,
    key: Option<Inputs>,
    /// The snapshot the view was last fitted to, so a Refresh keeps the view.
    fitted: Option<ObjectId>,
    pub facts: Option<Arc<Facts>>,
}
impl Default for CodeMap {
    fn default() -> Self {
        Self {
            colour: Colour::Kind,
            period: Period::Days30,
            base: String::new(),
            changed_only: false,
            compare_again: false,
            files_open: HashSet::new(),
            group: None,
            links: None,
            history: None,
            changes: None,
            errors: [None, None, None],
            links_task: None,
            requested: Vec::new(),
            too_large: HashSet::new(),
            history_task: None,
            changes_task: None,
            failed: [false; 3],
            stopped: false,
            tip: None,
            layout: None,
            link_count_rects: Vec::new(),
            key: None,
            fitted: None,
            facts: None,
        }
    }
}
/// What the layout was worked out for.
#[derive(Clone, PartialEq)]
struct Inputs {
    data: usize,
    links: usize,
    history: usize,
    changes: usize,
    colour: Colour,
    period: Period,
    changed_only: bool,
    collapsed: usize,
    files_open: usize,
    anchor: Option<Vec<u8>>,
    /// Cards carry translated text: a new language paints them again.
    language: crate::i18n::Language,
    /// Cards carry theme colours: a new theme paints them again.
    theme: u64,
    /// Search chooses the card that shows the first match.
    query: String,
}
fn ptr<T>(a: &Option<Arc<T>>) -> usize {
    a.as_ref().map_or(0, |a| Arc::as_ptr(a) as usize)
}
fn set_hash(set: &HashSet<Vec<u8>>) -> usize {
    use std::hash::{Hash, Hasher};
    let mut keys: Vec<&Vec<u8>> = set.iter().collect();
    keys.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    keys.hash(&mut h);
    h.finish() as usize
}

impl CodeMap {
    pub fn running(&self) -> bool {
        self.links_task.is_some() || self.history_task.is_some() || self.changes_task.is_some()
    }
    pub fn reading_links(&self) -> bool {
        self.links_task.is_some()
    }
    pub fn reading_history(&self) -> bool {
        self.history_task.is_some()
    }
    pub fn reading_changes(&self) -> bool {
        self.changes_task.is_some()
    }
    /// The first failure of the links, history and Changes reads.
    pub fn error(&self) -> Option<&String> {
        self.errors.iter().flatten().next()
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.links_task {
            t.cancel();
        }
        if let Some(t) = &self.history_task {
            t.cancel();
        }
        if let Some(t) = &self.changes_task {
            t.cancel();
        }
    }
    /// Takes finished reads; true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(result) = self.links_task.as_mut().and_then(ReadTask::try_finish) {
            self.links_task = None;
            changed = true;
            let requested = std::mem::take(&mut self.requested);
            match result {
                Ok(read) => match &self.links {
                    // A folder read joins what was read before, as a new value, so the
                    // map lays out again.
                    Some(links) if !requested.is_empty() => {
                        let mut joined = (**links).clone();
                        joined.merge(read);
                        // The first package asked for went first; still unread, it holds
                        // more code than one read takes.
                        if let Some(first) = requested.first()
                            && joined.unread.iter().any(|u| &u.folder == first)
                        {
                            self.too_large.insert(first.clone());
                        }
                        self.links = Some(Arc::new(joined));
                    }
                    _ => self.links = Some(Arc::new(read)),
                },
                Err(e) => self.ended(0, &e),
            }
        }
        if let Some(result) = self.history_task.as_mut().and_then(ReadTask::try_finish) {
            self.history_task = None;
            changed = true;
            match result {
                Ok(read) => self.history = Some(Arc::new(read)),
                Err(e) => self.ended(1, &e),
            }
        }
        if let Some(result) = self.changes_task.as_mut().and_then(ReadTask::try_finish) {
            self.changes_task = None;
            changed = true;
            match result {
                Ok(read) => self.changes = Some(Arc::new(read)),
                Err(e) => self.ended(2, &e),
            }
        }
        changed
    }
    /// A read that ended without a result is not asked for again until Refresh, or
    /// Compare for Changes: Stop means stop.
    fn ended(&mut self, read: usize, e: &Error) {
        self.failed[read] = true;
        match e {
            Error::Cancelled => self.stopped = true,
            e => self.errors[read] = Some(map_error(e)),
        }
    }
    /// A Code read finished: what was read for another commit no longer applies, and a
    /// read that failed or was stopped may be tried again. Changes are read again, since
    /// the base may have moved.
    pub fn renew(&mut self, tip: Option<&ObjectId>) {
        if self.tip.as_ref() != tip {
            self.stop();
            self.links_task = None;
            self.history_task = None;
            self.requested.clear();
            self.links = None;
            self.history = None;
            self.too_large.clear();
            self.tip = tip.cloned();
        }
        if let Some(t) = self.changes_task.take() {
            t.cancel();
        }
        self.changes = None;
        self.failed = [false; 3];
        self.stopped = false;
        self.errors = [None, None, None];
        // The next visible frame lays the map out again: MCP must not see the indices of
        // the last snapshot in the meantime.
        self.layout = None;
        self.key = None;
        self.facts = None;
        self.group = None;
    }
    pub fn start(&mut self, read: CodeRead, tip: ObjectId, ctx: &Context, backend: Backend, repo: Repository) {
        match read {
            CodeRead::Links(only) => {
                if self.links_task.is_some() {
                    return;
                }
                self.errors[0] = None;
                self.requested = only.clone().unwrap_or_default();
                match crate::read_task::spawn(ctx, move |cancel| {
                    backend.log.request("map-code-links", |log| repo.with_log(log.clone()).code_links(&tip, only.as_deref(), &cancel))
                }) {
                    Ok(task) => self.links_task = Some(task),
                    Err(e) => self.ended(0, &e),
                }
            }
            CodeRead::History => {
                if self.history_task.is_some() {
                    return;
                }
                self.errors[1] = None;
                match crate::read_task::spawn(ctx, move |cancel| backend.log.request("map-code-history", |log| repo.with_log(log.clone()).code_history(&tip, &cancel))) {
                    Ok(task) => self.history_task = Some(task),
                    Err(e) => self.ended(1, &e),
                }
            }
            CodeRead::Changes => {
                self.compare_again = false;
                if let Some(t) = self.changes_task.take() {
                    t.cancel();
                }
                let base = self.base.trim().to_owned();
                if base.is_empty() {
                    return;
                }
                self.changes = None;
                self.errors[2] = None;
                match crate::read_task::spawn(ctx, move |cancel| backend.log.request("map-code-changes", |log| repo.with_log(log.clone()).code_changes(&base, &tip, &cancel))) {
                    Ok(task) => self.changes_task = Some(task),
                    Err(e) => self.ended(2, &e),
                }
            }
        }
    }
}

#[cfg(test)]
impl CodeMap {
    /// Tests: a links read for `requested` (empty: the whole snapshot) that ends with
    /// `result`, as the read thread would.
    pub(super) fn links_read_for_test(&mut self, ctx: &Context, requested: Vec<Vec<u8>>, result: rocket_git::Result<CodeLinks>) {
        self.requested = requested;
        self.links_task = Some(crate::read_task::spawn(ctx, move |_| result).expect("a read thread"));
    }
}

/// The read the map needs next and has not asked for.
pub(super) fn wanted(s: &MapPage, tip: &ObjectId) -> Option<CodeRead> {
    let c = &s.code;
    if c.links.is_none() && c.links_task.is_none() && !c.failed[0] {
        return Some(CodeRead::Links(None));
    }
    // Packages past the first read's limit are read once their folder is open, however it
    // was opened: its arrow, Expand all, or a folder shown from elsewhere.
    if let (Some(links), Some(layout), Some(f)) = (&c.links, &c.layout, &c.facts)
        && c.links_task.is_none()
        && !c.failed[0]
    {
        let mut open: Vec<Vec<u8>> = links
            .unread
            .iter()
            .filter(|u| !c.too_large.contains(&u.folder))
            .filter(|u| u.folder.is_empty() || f.by_path.get(&u.folder).and_then(|&n| layout.index(Item::Node(n))).is_some_and(|i| layout.placed[i].open))
            .map(|u| u.folder.clone())
            .collect();
        if !open.is_empty() {
            // In path order, as the read takes them.
            open.sort();
            open.dedup();
            return Some(CodeRead::Links(Some(open)));
        }
    }
    let owners = c.links.as_ref().is_some_and(|l| l.owners.is_some());
    let history = c.colour == Colour::Activity || c.colour == Colour::Owner && !owners && c.links.is_some();
    if history && c.history.as_ref().is_none_or(|h| h.tip.as_ref() != Some(tip)) && c.history_task.is_none() && !c.failed[1] {
        return Some(CodeRead::History);
    }
    if c.colour == Colour::Changes && !c.base.trim().is_empty() && (c.compare_again || c.changes.is_none() && c.changes_task.is_none() && !c.failed[2]) {
        return Some(CodeRead::Changes);
    }
    None
}
/// What the Code read's tree says, worked out once per read.
pub struct Facts {
    pub parent: Vec<Option<usize>>,
    /// A folder's folders and its own files, by name.
    pub folders: Vec<Vec<usize>>,
    pub files: Vec<Vec<usize>>,
    pub top_folders: Vec<usize>,
    pub top_files: Vec<usize>,
    pub by_path: HashMap<Vec<u8>, usize>,
    pub kind: Vec<Option<FileKind>>,
    /// Bytes by kind: a file's own, a folder's with everything under it.
    pub sizes: Vec<[u64; 5]>,
    /// Bytes by kind of a folder's own files; the top folder's last.
    pub own: Vec<[u64; 5]>,
    pub top_own: [u64; 5],
}
fn kind_index(k: FileKind) -> usize {
    FileKind::ALL.iter().position(|x| *x == k).unwrap_or(4)
}
/// A file of the snapshot. Cargo reads a Cargo.toml at the top as its package, but here it
/// stays a file: the top folder has no card.
pub fn is_file(n: &MapNode) -> bool {
    n.kind == MapNodeKind::File || n.kind == MapNodeKind::Crate && n.path.as_deref() == Some(b"Cargo.toml")
}
fn is_folder(n: &MapNode) -> bool {
    matches!(n.kind, MapNodeKind::Directory | MapNodeKind::Crate) && !is_file(n)
}
pub fn facts(data: &RepositoryMap) -> Facts {
    let n = data.nodes.len();
    let mut f = Facts {
        parent: vec![None; n],
        folders: vec![Vec::new(); n],
        files: vec![Vec::new(); n],
        top_folders: Vec::new(),
        top_files: Vec::new(),
        by_path: HashMap::new(),
        kind: vec![None; n],
        sizes: vec![[0; 5]; n],
        own: vec![[0; 5]; n],
        top_own: [0; 5],
    };
    for e in data.links.iter().filter(|e| e.kind == MapLinkKind::Contains) {
        f.parent[e.to] = Some(e.from);
    }
    for (i, node) in data.nodes.iter().enumerate() {
        if let Some(p) = &node.path {
            f.by_path.insert(p.clone(), i);
        }
        let file = is_file(node);
        if !file && !is_folder(node) {
            continue;
        }
        match (f.parent[i], file) {
            (Some(p), true) => f.files[p].push(i),
            (Some(p), false) => f.folders[p].push(i),
            (None, true) => f.top_files.push(i),
            (None, false) => f.top_folders.push(i),
        }
        if file {
            let k = file_kind(node.path.as_deref().unwrap_or(&node.label));
            f.kind[i] = Some(k);
            let bytes = node.count.max(1) as u64;
            let at = kind_index(k);
            f.sizes[i][at] += bytes;
            match f.parent[i] {
                Some(p) => f.own[p][at] += bytes,
                None => f.top_own[at] += bytes,
            }
            let mut up = f.parent[i];
            for _ in 0..64 {
                let Some(p) = up else { break };
                f.sizes[p][at] += bytes;
                up = f.parent[p];
            }
        }
    }
    let name = |i: &usize| drawing::folder_name(&data.nodes[*i]).to_lowercase();
    f.top_folders.sort_by_key(name);
    f.top_files.sort_by_key(name);
    for list in f.folders.iter_mut().chain(f.files.iter_mut()) {
        list.sort_by_key(name);
    }
    f
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    /// A folder (card or box), a package folder or a file.
    Node(usize),
    /// A folder's own files on one card; None for the top folder.
    Files(Option<usize>),
}
#[derive(Clone, Debug, Default)]
pub struct Paint {
    pub fill: Option<Color32>,
    /// Colour and share, left to right.
    pub band: Vec<(Color32, f32)>,
    pub sub: String,
    pub badge: Option<char>,
    /// Uses a change.
    pub hatched: bool,
    /// Nobody owns it, or its code was not read.
    pub dashed: bool,
}
#[derive(Clone, Debug)]
pub struct Placed {
    pub item: Item,
    pub rect: Rect,
    pub open: bool,
    pub parent: Option<usize>,
    /// Folders merged into this box's title, outermost first: `crates / rocket-git`.
    pub chain: Vec<usize>,
    pub paint: Paint,
}
#[derive(Clone, Debug)]
pub struct Arrow {
    pub from: usize,
    pub to: usize,
    /// Files on the using side.
    pub files: usize,
    pub links: usize,
    /// Points up: the two use each other.
    pub up: bool,
    pub names: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Legend {
    pub kinds: Vec<FileKind>,
    pub most: usize,
    /// Owner: colour and name of the coloured owners, then Others, Unowned and Nobody
    /// when they occur.
    pub owners: Vec<(Option<Color32>, String)>,
    pub owners_from: Option<String>,
    /// CODEOWNERS lines left out.
    pub owners_skipped: usize,
    /// CODEOWNERS is past what is read, so most commits decide.
    pub owners_too_large: bool,
    pub owners_limited: bool,
    pub added: usize,
    pub changed: usize,
    pub uses: usize,
}
#[derive(Clone, Debug)]
pub struct Picture {
    pub placed: Vec<Placed>,
    pub arrows: Vec<Arrow>,
    /// "Not connected" captions.
    pub captions: Vec<Pos2>,
    pub bounds: Rect,
    pub at: HashMap<Item, usize>,
    /// Some folders stay closed: too many cards.
    pub cut: bool,
    pub legend: Legend,
}
impl Default for Picture {
    fn default() -> Self {
        Self { placed: Vec::new(), arrows: Vec::new(), captions: Vec::new(), bounds: Rect::NOTHING, at: HashMap::new(), cut: false, legend: Legend::default() }
    }
}
impl Picture {
    pub fn index(&self, item: Item) -> Option<usize> {
        self.at.get(&item).copied()
    }
    /// What the placed card or box at `i` uses and what uses it, most files first.
    pub fn connections(&self, i: usize) -> (Vec<&Arrow>, Vec<&Arrow>) {
        let mut uses: Vec<&Arrow> = self.arrows.iter().filter(|a| a.from == i).collect();
        let mut used: Vec<&Arrow> = self.arrows.iter().filter(|a| a.to == i).collect();
        uses.sort_by_key(|a| std::cmp::Reverse(a.files));
        used.sort_by_key(|a| std::cmp::Reverse(a.files));
        (uses, used)
    }
    fn inside(&self, outer: usize, mut i: usize) -> bool {
        for _ in 0..128 {
            match self.placed[i].parent {
                Some(p) if p == outer => return true,
                Some(p) => i = p,
                None => return false,
            }
        }
        false
    }
}

pub fn kind_colour(k: FileKind) -> Option<Color32> {
    match k {
        FileKind::Code => Some(MARK_1()),
        FileKind::Tests => Some(MARK_2()),
        FileKind::Docs => Some(MARK_3()),
        FileKind::Config => Some(DIM()),
        FileKind::Other => None,
    }
}
pub fn kind_name(k: FileKind) -> &'static str {
    match k {
        FileKind::Code => crate::i18n::tr("Code"),
        FileKind::Tests => tr("Tests"),
        FileKind::Docs => tr("Docs"),
        FileKind::Config => tr("Config and data"),
        FileKind::Other => crate::i18n::tr("Other"),
    }
}
const OWNER_SLOTS: usize = 3;
fn owner_colour(slot: usize) -> Color32 {
    [MARK_1(), MARK_2(), MARK_3()][slot.min(2)]
}
fn ramp(v: usize, most: usize) -> Option<Color32> {
    if v == 0 || most == 0 {
        return None;
    }
    let share = v as f32 / most as f32;
    Some(if share > 0.75 {
        BLUE()
    } else if share > 0.3 {
        RAMP_3()
    } else if share > 0.12 {
        RAMP_2()
    } else {
        RAMP_1()
    })
}
fn tint(c: Color32, amount: f32) -> Color32 {
    let base = VIEW_RAISED();
    let mix = |a: u8, b: u8| (a as f32 * (1. - amount) + b as f32 * amount).round() as u8;
    Color32::from_rgb(mix(base.r(), c.r()), mix(base.g(), c.g()), mix(base.b(), c.b()))
}

/// The folder path an item stands for; empty for the top folder.
pub fn item_path(data: &RepositoryMap, item: Item) -> Vec<u8> {
    match item {
        Item::Node(i) | Item::Files(Some(i)) => data.nodes[i].path.clone().unwrap_or_default(),
        Item::Files(None) => Vec::new(),
    }
}
/// Files a card counts: a file, everything under a folder, or a folder's own files.
pub(super) fn under(f: &Facts, data: &RepositoryMap, item: Item, out: &mut Vec<usize>) {
    match item {
        Item::Node(i) if is_file(&data.nodes[i]) => out.push(i),
        Item::Node(i) => {
            out.extend(f.files[i].iter().copied());
            for &k in &f.folders[i] {
                under(f, data, Item::Node(k), out);
            }
        }
        Item::Files(Some(i)) => out.extend(f.files[i].iter().copied()),
        Item::Files(None) => out.extend(f.top_files.iter().copied()),
    }
}

/// Lays out the folders shown, gathers the arrows between them, and paints them.
pub(super) fn rebuild(s: &mut MapPage) {
    let Some(data) = s.data.clone() else { return };
    // Connected only rests while Changes shows Changed only in its place.
    let anchor = if s.code.colour == Colour::Changes { None } else { s.neighbor_anchor.clone() };
    let key = Inputs {
        data: Arc::as_ptr(&data) as usize,
        links: ptr(&s.code.links),
        history: ptr(&s.code.history),
        changes: ptr(&s.code.changes),
        colour: s.code.colour,
        period: s.code.period,
        changed_only: s.code.changed_only && s.code.colour == Colour::Changes,
        collapsed: set_hash(&s.collapsed),
        files_open: set_hash(&s.code.files_open),
        anchor: anchor.clone(),
        language: crate::i18n::current(),
        theme: crate::style::generation(),
        query: s.query.to_lowercase(),
    };
    if s.code.key.as_ref() == Some(&key) && s.filter_key.is_some() && s.code.layout.is_some() {
        return;
    }
    if s.code.facts.is_none() || s.code.key.as_ref().is_none_or(|k| k.data != key.data) {
        s.code.facts = Some(Arc::new(facts(&data)));
    }
    let f = s.code.facts.clone().expect("facts");
    // Changes: worked out once for both Changed only and the colours.
    let marks = match (&s.code.changes, s.code.colour) {
        (Some(c), Colour::Changes) => change_marks(s, &data, &f, c),
        _ => HashMap::new(),
    };
    let mut layout = build(s, &data, &f, key.changed_only.then_some(&marks), None);
    // Connected only: the selection, what it connects to, and the boxes around them.
    if let Some(anchor) = &anchor
        && let Some(node) = data.nodes.iter().position(|n| &n.key == anchor)
        && let Some(sel) = layout.index(Item::Node(node))
    {
        let mut keep: HashSet<Item> = HashSet::new();
        keep.insert(layout.placed[sel].item);
        for a in &layout.arrows {
            if a.from == sel {
                keep.insert(layout.placed[a.to].item);
            }
            if a.to == sel {
                keep.insert(layout.placed[a.from].item);
            }
        }
        layout = build(s, &data, &f, key.changed_only.then_some(&marks), Some(&keep));
    }
    paint(s, &data, &f, &mut layout, &marks);
    // What the rest of the Map works from: the cards shown and the arrows between
    // them, for search, MCP and Export.
    s.visible = layout.placed.iter().filter_map(|p| if let Item::Node(i) = p.item { Some(i) } else { None }).collect();
    s.visible.sort_unstable();
    s.positions = vec![Pos2::ZERO; data.nodes.len()];
    for p in &layout.placed {
        if let Item::Node(i) = p.item {
            s.positions[i] = p.rect.min;
        }
    }
    s.file_groups.clear();
    let mut links: Vec<MapLink> = Vec::new();
    for p in &layout.placed {
        if let (Item::Node(child), Some(parent)) = (p.item, p.parent)
            && let Item::Node(folder) = layout.placed[parent].item
        {
            links.push(MapLink { from: folder, to: child, kind: MapLinkKind::Contains, weight: 1, evidence: Vec::new() });
        }
    }
    for a in &layout.arrows {
        if let (Item::Node(x), Item::Node(y)) = (layout.placed[a.from].item, layout.placed[a.to].item) {
            links.push(MapLink { from: x, to: y, kind: MapLinkKind::DependsOn, weight: a.files, evidence: a.names.iter().map(|n| n.as_bytes().to_vec()).collect() });
        }
    }
    s.projected_links = links;
    // Every card placed is drawn: no pages of cards.
    s.view_total = 0;
    s.display_offset = 0;
    // A selection without a card of its own (a file inside a closed folder, chosen in
    // Details) goes to the card that shows it.
    if s.code.group.is_none() && !s.selected.is_some_and(|i| s.visible.contains(&i)) {
        let shown = s.selected.and_then(|i| represent(&layout, &f, &data, data.nodes[i].path.as_deref()?, is_folder(&data.nodes[i])));
        match shown.map(|at| layout.placed[at].item) {
            Some(Item::Node(n)) => s.selected = Some(n),
            Some(Item::Files(c)) => s.code.group = Some(c),
            None => s.selected = s.visible.first().copied(),
        }
    }
    // Search chooses the card that shows the first match, a closed folder for a file
    // inside it, unless the chosen card matches already.
    if !key.query.is_empty() {
        let hit = |n: &MapNode| label(&n.label).to_lowercase().contains(&key.query) || n.path.as_ref().is_some_and(|p| label(p).to_lowercase().contains(&key.query));
        let chosen_matches = s.code.group.is_none() && s.selected.is_some_and(|i| hit(&data.nodes[i]));
        if !chosen_matches
            && let Some(at) = data
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| (is_file(n) || is_folder(n)) && hit(n))
                .find_map(|(_, n)| represent(&layout, &f, &data, n.path.as_deref()?, is_folder(n)))
        {
            match layout.placed[at].item {
                Item::Node(n) => {
                    s.selected = Some(n);
                    s.code.group = None;
                }
                Item::Files(c) => s.code.group = Some(c),
            }
        }
    }
    if let Some(g) = s.code.group
        && layout.index(Item::Files(g)).is_none()
    {
        s.code.group = None;
    }
    // A new snapshot is fitted to the window; a Refresh of the same one keeps the view.
    if s.code.fitted != data.tip {
        s.code.fitted = data.tip.clone();
        s.fit = true;
    }
    s.code.layout = Some(Arc::new(layout));
    s.code.key = Some(key);
    s.filter_key = Some((s.query.clone(), false, None, false));
    s.topology = None;
}

struct Build<'a> {
    data: &'a RepositoryMap,
    f: &'a Facts,
    collapsed: &'a HashSet<Vec<u8>>,
    files_open: &'a HashSet<Vec<u8>>,
    /// Changed only: the files kept and the folders on the way to them.
    keep_files: Option<HashSet<usize>>,
    keep_folders: HashSet<usize>,
    /// Connected only: the items kept.
    keep_items: Option<&'a HashSet<Item>>,
    layout: Picture,
}
impl Build<'_> {
    fn open(&self, n: usize) -> bool {
        if self.keep_files.is_some() {
            return self.keep_folders.contains(&n);
        }
        !self.collapsed.contains(&self.data.nodes[n].key)
    }
    fn shown_folder(&self, n: usize) -> bool {
        if self.keep_files.is_some() {
            return self.keep_folders.contains(&n);
        }
        match self.keep_items {
            Some(keep) => keep.contains(&Item::Node(n)) || self.keeps_inside(n),
            None => true,
        }
    }
    /// Connected only: a folder stays when it holds a kept card.
    fn keeps_inside(&self, n: usize) -> bool {
        let Some(keep) = self.keep_items else { return true };
        keep.iter().any(|item| match item {
            Item::Node(i) => self.ancestor(n, *i),
            // A folder's own "N files" card sits in its box.
            Item::Files(Some(i)) => *i == n || self.ancestor(n, *i),
            Item::Files(None) => false,
        })
    }
    fn ancestor(&self, folder: usize, mut i: usize) -> bool {
        for _ in 0..64 {
            match self.f.parent[i] {
                Some(p) if p == folder => return true,
                Some(p) => i = p,
                None => return false,
            }
        }
        false
    }
    fn shown_file(&self, n: usize) -> bool {
        if let Some(keep) = &self.keep_files {
            return keep.contains(&n);
        }
        self.keep_items.is_none_or(|keep| keep.contains(&Item::Node(n)))
    }
    fn push(&mut self, item: Item, open: bool, parent: Option<usize>, chain: Vec<usize>) -> usize {
        let i = self.layout.placed.len();
        self.layout.placed.push(Placed { item, rect: Rect::ZERO, open, parent, chain: chain.clone(), paint: Paint::default() });
        self.layout.at.insert(item, i);
        for c in chain {
            self.layout.at.insert(Item::Node(c), i);
        }
        i
    }
    fn place(&mut self, container: Option<usize>, parent: Option<usize>) {
        let (folders, files) = match container {
            Some(c) => (self.f.folders[c].clone(), self.f.files[c].clone()),
            None => (self.f.top_folders.clone(), self.f.top_files.clone()),
        };
        for folder in folders {
            if !self.shown_folder(folder) {
                continue;
            }
            if self.open(folder) && self.layout.placed.len() < ITEMS {
                // A chain of open folders holding one folder each shares one box.
                let mut chain = Vec::new();
                let mut inner = folder;
                loop {
                    let subs: Vec<usize> = self.f.folders[inner].iter().copied().filter(|k| self.shown_folder(*k)).collect();
                    let own = self.f.files[inner].iter().any(|k| self.shown_file(*k));
                    if !own && subs.len() == 1 && self.open(subs[0]) {
                        chain.push(inner);
                        inner = subs[0];
                    } else {
                        break;
                    }
                }
                let at = self.push(Item::Node(inner), true, parent, chain);
                self.place(Some(inner), Some(at));
            } else {
                if self.layout.placed.len() >= ITEMS {
                    self.layout.cut = true;
                }
                self.push(Item::Node(folder), false, parent, Vec::new());
            }
        }
        // Connected only kept the "N files" card itself: it stays, standing for its files.
        if self.keep_items.is_some_and(|keep| keep.contains(&Item::Files(container))) {
            self.push(Item::Files(container), false, parent, Vec::new());
            return;
        }
        let files: Vec<usize> = files.into_iter().filter(|k| self.shown_file(*k)).collect();
        if files.is_empty() {
            return;
        }
        let path = container.and_then(|c| self.data.nodes[c].path.clone()).unwrap_or_default();
        if files.len() <= FILE_CARDS || self.files_open.contains(&path) || self.keep_files.is_some() || self.keep_items.is_some() {
            for k in files {
                self.push(Item::Node(k), false, parent, Vec::new());
            }
        } else {
            self.push(Item::Files(container), false, parent, Vec::new());
        }
    }
}

/// `changed_only`: the Changes marks, to keep only the files they mark.
fn build(s: &MapPage, data: &RepositoryMap, f: &Facts, changed_only: Option<&HashMap<usize, char>>, keep_items: Option<&HashSet<Item>>) -> Picture {
    let mut b = Build {
        data,
        f,
        collapsed: &s.collapsed,
        files_open: &s.code.files_open,
        keep_files: None,
        keep_folders: HashSet::new(),
        keep_items,
        layout: Picture::default(),
    };
    if let Some(marks) = changed_only
        && s.code.changes.is_some()
    {
        let keep: HashSet<usize> = marks.keys().copied().collect();
        for &k in &keep {
            let mut up = f.parent[k];
            for _ in 0..64 {
                let Some(p) = up else { break };
                b.keep_folders.insert(p);
                up = f.parent[p];
            }
        }
        b.keep_files = Some(keep);
    }
    b.place(None, None);
    let mut layout = b.layout;
    arrows(s, data, f, &mut layout);
    arrange(&mut layout, data);
    layout
}

/// Each link of the read, from the card or box that shows its ends.
fn arrows(s: &MapPage, data: &RepositoryMap, f: &Facts, layout: &mut Picture) {
    let Some(links) = &s.code.links else { return };
    let mut rep: Vec<Option<usize>> = Vec::with_capacity(links.ends.len());
    for end in &links.ends {
        rep.push(represent(layout, f, data, &end.path, end.folder));
    }
    // By the two cards: the using files, the links and a few names.
    type Gathered<'a> = HashMap<(usize, usize), (HashSet<&'a [u8]>, usize, Vec<String>)>;
    let mut gathered: Gathered = HashMap::new();
    for l in &links.links {
        let (Some(a), Some(b)) = (rep[l.from], rep[l.to]) else { continue };
        if a == b || layout.inside(a, b) || layout.inside(b, a) {
            continue;
        }
        let e = gathered.entry((a, b)).or_insert_with(|| (HashSet::new(), 0, Vec::new()));
        e.0.insert(&links.ends[l.from].path);
        e.1 += 1;
        for n in &l.names {
            if e.2.len() < 6 && !e.2.contains(n) {
                e.2.push(n.clone());
            }
        }
    }
    let mut list: Vec<Arrow> = gathered.into_iter().map(|((from, to), (files, links, names))| Arrow { from, to, files: files.len(), links, up: false, names }).collect();
    list.sort_by_key(|a| (a.from, a.to));
    layout.arrows = list;
}
/// The card or box an end shows on: its own card, its folder's "N files" card, or the
/// nearest closed folder around it. An end a filter hides inside an open box (Changed
/// only, Connected only) shows nowhere, however deep it sits.
fn represent(layout: &Picture, f: &Facts, data: &RepositoryMap, path: &[u8], folder: bool) -> Option<usize> {
    let mut n = *f.by_path.get(path)?;
    if !folder && is_file(&data.nodes[n]) {
        if let Some(i) = layout.index(Item::Node(n)) {
            return Some(i);
        }
        let parent = f.parent[n];
        if let Some(i) = layout.index(Item::Files(parent)) {
            return Some(i);
        }
        n = parent?;
    }
    let own = n;
    for _ in 0..64 {
        if let Some(i) = layout.index(Item::Node(n)) {
            // A package's own box takes its arrows; any other open box hides the end.
            return (!layout.placed[i].open || folder && n == own).then_some(i);
        }
        n = f.parent[n]?;
    }
    None
}
/// Rows inside each box, from the inside out, then positions from the top.
fn arrange(layout: &mut Picture, data: &RepositoryMap) {
    let count = layout.placed.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut top = Vec::new();
    for (i, p) in layout.placed.iter().enumerate() {
        match p.parent {
            Some(q) => kids[q].push(i),
            None => top.push(i),
        }
    }
    // Relative position inside the parent's content area, and size.
    let mut rel: Vec<Rect> = vec![Rect::ZERO; count];
    let mut captions: Vec<(Option<usize>, Pos2)> = Vec::new();
    fn name(layout: &Picture, data: &RepositoryMap, i: usize) -> String {
        match layout.placed[i].item {
            Item::Node(n) => drawing::folder_name(&data.nodes[n]).to_lowercase(),
            Item::Files(_) => "\u{10ffff}".into(),
        }
    }
    // Inside out: a box's size needs its contents first.
    let mut order: Vec<usize> = (0..count).filter(|i| layout.placed[*i].open).collect();
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = layout.placed[i].parent {
            d += 1;
            i = p;
        }
        d
    };
    order.sort_by_key(|i| std::cmp::Reverse(depth(*i)));
    let mut sizes: Vec<Vec2> = layout.placed.iter().map(|_| drawing::CARD).collect();
    // Each arrow joins two members of one box, or of the top: the members holding its ends,
    // in the innermost box that holds both. One pass over the arrows, not one a box.
    let mut edges_by: HashMap<Option<usize>, Vec<(usize, usize)>> = HashMap::new();
    let mut seen: HashSet<(Option<usize>, usize, usize)> = HashSet::new();
    for a in &layout.arrows {
        let mut around: HashMap<Option<usize>, usize> = HashMap::new();
        let mut i = a.from;
        for _ in 0..128 {
            let p = layout.placed[i].parent;
            around.insert(p, i);
            match p {
                Some(q) => i = q,
                None => break,
            }
        }
        let mut j = a.to;
        for _ in 0..128 {
            let p = layout.placed[j].parent;
            if let Some(&x) = around.get(&p) {
                if x != j && seen.insert((p, x, j)) {
                    edges_by.entry(p).or_default().push((x, j));
                }
                break;
            }
            match p {
                Some(q) => j = q,
                None => break,
            }
        }
    }
    let place_group = |container: Option<usize>, members: &[usize], sizes: &mut Vec<Vec2>, rel: &mut Vec<Rect>, captions: &mut Vec<(Option<usize>, Pos2)>| -> Vec2 {
        let edges: &[(usize, usize)] = edges_by.get(&container).map(Vec::as_slice).unwrap_or_default();
        let connected: HashSet<usize> = edges.iter().flat_map(|(x, y)| [*x, *y]).collect();
        let mut lonely: Vec<usize> = members.iter().copied().filter(|m| !connected.contains(m)).collect();
        lonely.sort_by_key(|m| name(layout, data, *m));
        let mut joined: Vec<usize> = members.iter().copied().filter(|m| connected.contains(m)).collect();
        joined.sort_by_key(|m| name(layout, data, *m));
        let rows_of = |list: &[usize], sizes: &Vec<Vec2>| -> Vec<Vec<usize>> {
            let widest = list.iter().map(|i| sizes[*i].x).fold(0., f32::max);
            let limit = (ROW_ITEMS as f32 * (drawing::CARD.x + GAP_X) - GAP_X).max(widest);
            let mut rows: Vec<Vec<usize>> = Vec::new();
            let mut width = 0.;
            for &i in list {
                let w = sizes[i].x;
                match rows.last_mut() {
                    Some(row) if width + GAP_X + w <= limit + 0.5 => {
                        row.push(i);
                        width += GAP_X + w;
                    }
                    _ => {
                        rows.push(vec![i]);
                        width = w;
                    }
                }
            }
            rows
        };
        let mut rows: Vec<Vec<usize>> = Vec::new();
        if !joined.is_empty() {
            // Layers: what a card uses sits below it; circles are broken first.
            let kept = without_circles(&joined, edges, |i| name(layout, data, i));
            let mut layer: HashMap<usize, usize> = joined.iter().map(|&i| (i, 0)).collect();
            for _ in 0..joined.len().min(64) {
                let mut changed = false;
                for &(user, used) in &kept {
                    let next = layer[&user] + 1;
                    if layer[&used] < next && next <= joined.len() {
                        layer.insert(used, next);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            let mut layers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &i in &joined {
                layers.entry(layer[&i]).or_default().push(i);
            }
            let mut slot: HashMap<usize, f32> = HashMap::new();
            for list in layers.values_mut() {
                let weight = |i: usize| {
                    let users: Vec<f32> = edges.iter().filter(|(_, used)| *used == i).filter_map(|(user, _)| slot.get(user).copied()).collect();
                    if users.is_empty() { f32::MAX } else { users.iter().sum::<f32>() / users.len() as f32 }
                };
                let mut keyed: Vec<(f32, usize)> = list.iter().map(|&i| (weight(i), i)).collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| name(layout, data, a.1).cmp(&name(layout, data, b.1))));
                *list = keyed.into_iter().map(|(_, i)| i).collect();
                for (k, &i) in list.iter().enumerate() {
                    slot.insert(i, k as f32);
                }
                rows.extend(rows_of(list, sizes));
            }
        }
        let lonely_rows = rows_of(&lonely, sizes);
        let row_width = |row: &Vec<usize>| row.iter().map(|i| sizes[*i].x).sum::<f32>() + GAP_X * (row.len().saturating_sub(1)) as f32;
        let width = rows.iter().chain(lonely_rows.iter()).map(row_width).fold(0., f32::max);
        let mut y = 0.;
        let place_rows = |rows: &[Vec<usize>], y: &mut f32, rel: &mut Vec<Rect>| {
            for row in rows {
                let mut x = (width - row_width(row)) / 2.;
                let height = row.iter().map(|i| sizes[*i].y).fold(0., f32::max);
                for &i in row {
                    rel[i] = Rect::from_min_size(pos2(x, *y), sizes[i]);
                    x += sizes[i].x + GAP_X;
                }
                *y += height + GAP_Y;
            }
        };
        place_rows(&rows, &mut y, rel);
        if !lonely_rows.is_empty() {
            if !rows.is_empty() {
                captions.push((container, pos2(0., y - GAP_Y / 2. + 4.)));
                y += CAPTION - GAP_Y / 2. + 8.;
            }
            place_rows(&lonely_rows, &mut y, rel);
        }
        vec2(width, (y - GAP_Y).max(0.))
    };
    for &b in &order {
        let content = place_group(Some(b), &kids[b], &mut sizes, &mut rel, &mut captions);
        sizes[b] = vec2((content.x + 2. * PAD).max(BOX_MIN), HEADER + content.y + if content.y > 0. { PAD } else { 0. });
    }
    place_group(None, &top, &mut sizes, &mut rel, &mut captions);
    // From the top: a box's content starts under its header.
    let mut absolute: Vec<Rect> = vec![Rect::ZERO; count];
    let mut todo: Vec<(Option<usize>, Pos2)> = vec![(None, Pos2::ZERO)];
    while let Some((container, origin)) = todo.pop() {
        let members = match container {
            Some(c) => &kids[c],
            None => &top,
        };
        for &i in members {
            absolute[i] = rel[i].translate(origin.to_vec2());
            if layout.placed[i].open {
                todo.push((Some(i), absolute[i].min + vec2(PAD, HEADER)));
            }
        }
    }
    for (i, p) in layout.placed.iter_mut().enumerate() {
        p.rect = absolute[i];
    }
    layout.captions = captions
        .into_iter()
        .map(|(c, at)| match c {
            Some(c) => absolute[c].min + vec2(PAD, HEADER) + at.to_vec2(),
            None => at,
        })
        .collect();
    layout.bounds = layout.placed.iter().fold(Rect::NOTHING, |r, p| r.union(p.rect));
    let rects: Vec<Rect> = layout.placed.iter().map(|p| p.rect).collect();
    for a in &mut layout.arrows {
        a.up = rects[a.to].bottom() <= rects[a.from].top() + 1.;
    }
}

/// Connections that close a circle are left out of the layering: depth first from the
/// biggest users, a connection back to a card still being walked closes one.
fn without_circles(nodes: &[usize], edges: &[(usize, usize)], name: impl Fn(usize) -> String) -> Vec<(usize, usize)> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut net: HashMap<usize, i64> = HashMap::new();
    for &(a, b) in edges {
        out.entry(a).or_default().push(b);
        *net.entry(a).or_default() += 1;
        *net.entry(b).or_default() -= 1;
    }
    let mut order = nodes.to_vec();
    order.sort_by(|a, b| net.get(b).copied().unwrap_or(0).cmp(&net.get(a).copied().unwrap_or(0)).then_with(|| name(*a).cmp(&name(*b))));
    for list in out.values_mut() {
        list.sort_by_key(|&i| name(i));
    }
    let mut state: HashMap<usize, u8> = HashMap::new();
    let mut kept = Vec::new();
    for &start in &order {
        if state.contains_key(&start) {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state.insert(start, 1);
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let targets = out.get(&node).map(Vec::as_slice).unwrap_or_default();
            if let Some(&to) = targets.get(*next) {
                *next += 1;
                match state.get(&to) {
                    Some(1) => {}
                    Some(_) => kept.push((node, to)),
                    None => {
                        kept.push((node, to));
                        state.insert(to, 1);
                        stack.push((to, 0));
                    }
                }
            } else {
                state.insert(node, 2);
                stack.pop();
            }
        }
    }
    kept
}

/// Changes: A, M, R and so on by file node, and `u` for a file that uses a changed one.
pub fn change_marks(s: &MapPage, data: &RepositoryMap, f: &Facts, changes: &CodeChanges) -> HashMap<usize, char> {
    let mut marks: HashMap<usize, char> = HashMap::new();
    for c in &changes.files {
        let letter = match c.change {
            Change::Added => 'A',
            Change::Renamed => 'R',
            Change::Copied => 'C',
            Change::Deleted => continue,
            _ => 'M',
        };
        if let Some(&n) = f.by_path.get(c.path.as_bytes()) {
            marks.insert(n, letter);
        }
    }
    // One step: a file that uses a changed file, or a package holding one.
    if let Some(links) = &s.code.links {
        let changed: HashSet<&[u8]> = changes.files.iter().filter(|c| c.change != Change::Deleted).map(|c| c.path.as_bytes()).collect();
        // Every folder holding a changed file, worked out once for the package ends.
        let mut holding: HashSet<&[u8]> = HashSet::new();
        for path in &changed {
            let mut at: &[u8] = path;
            while let Some(cut) = at.iter().rposition(|b| *b == b'/') {
                at = &at[..cut];
                if !holding.insert(at) {
                    break;
                }
            }
        }
        let hit = |end: &CodeEnd| if end.folder { holding.contains(end.path.as_slice()) } else { changed.contains(end.path.as_slice()) };
        for l in &links.links {
            if !hit(&links.ends[l.to]) {
                continue;
            }
            let from = &links.ends[l.from];
            if from.folder {
                // A Go or Java package and a C# namespace use as a whole: each of their own
                // files in the folder uses the change.
                if let Some(&n) = f.by_path.get(&from.path) {
                    for &k in &f.files[n] {
                        let whole = |p: &[u8]| [b".go".as_slice(), b".java", b".kt", b".cs"].iter().any(|e| p.ends_with(e)) && !p.ends_with(b"_test.go");
                        if data.nodes[k].path.as_deref().is_some_and(|p| whole(p) && !changed.contains(p)) {
                            marks.entry(k).or_insert('u');
                        }
                    }
                }
            } else if !changed.contains(from.path.as_slice())
                && let Some(&n) = f.by_path.get(&from.path)
            {
                marks.entry(n).or_insert('u');
            }
        }
    }
    marks
}
fn paint(s: &MapPage, data: &RepositoryMap, f: &Facts, layout: &mut Picture, marks: &HashMap<usize, char>) {
    let mut legend = Legend::default();
    match s.code.colour {
        Colour::Kind => {
            let mut present = [false; 5];
            for p in &mut layout.placed {
                let sizes = match p.item {
                    Item::Node(i) => f.sizes[i],
                    Item::Files(Some(i)) => f.own[i],
                    Item::Files(None) => f.top_own,
                };
                let total: u64 = sizes.iter().sum::<u64>().max(1);
                for (k, &v) in sizes.iter().enumerate() {
                    present[k] |= v > 0;
                }
                p.paint.band = FileKind::ALL
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| sizes[*k] > 0)
                    .map(|(k, kind)| (kind_colour(*kind).unwrap_or(Color32::TRANSPARENT), sizes[k] as f32 / total as f32))
                    .collect();
                let top = (0..4).filter(|k| sizes[*k] > 0).max_by_key(|k| sizes[*k]);
                p.paint.fill = top.and_then(|k| kind_colour(FileKind::ALL[k])).map(|c| tint(c, 0.13));
                p.paint.sub = sub_line(s, data, f, p.item);
            }
            legend.kinds = FileKind::ALL.iter().enumerate().filter(|(k, kind)| present[*k] && **kind != FileKind::Other).map(|(_, k)| *k).collect();
        }
        Colour::Activity => {
            let since = s.code.period.since();
            let values: Vec<Option<usize>> = layout
                .placed
                .iter()
                .map(|p| {
                    let h = s.code.history.as_ref()?;
                    Some(match p.item {
                        Item::Node(i) if is_file(&data.nodes[i]) => h.file_commits(&item_path(data, p.item), since),
                        Item::Node(_) => h.folder_commits(&item_path(data, p.item), false, since),
                        Item::Files(_) => h.folder_commits(&item_path(data, p.item), true, since),
                    })
                })
                .collect();
            // Against the busiest card: a box counts everything inside it.
            let most = values.iter().zip(&layout.placed).filter(|(_, p)| !p.open).filter_map(|(v, _)| *v).max().unwrap_or(0);
            for (p, v) in layout.placed.iter_mut().zip(values) {
                match v {
                    Some(v) => {
                        let step = ramp(v, most);
                        p.paint.fill = Some(step.map_or(CHIP(), |c| tint(c, 0.42)));
                        p.paint.band = step.map(|c| vec![(c, 1.)]).unwrap_or_default();
                        p.paint.sub = if v == 0 { tr("No commits").into() } else { trn!(v, "{n} commit", "{n} commits") };
                    }
                    None if s.code.reading_history() => p.paint.sub = tr("Reading commits").into(),
                    None => p.paint.sub = sub_line(s, data, f, p.item),
                }
            }
            legend.most = most;
        }
        Colour::Owner => owners(s, data, f, layout, &mut legend),
        Colour::Changes => {
            for p in &mut layout.placed {
                let mut list = Vec::new();
                under(f, data, p.item, &mut list);
                let changed = list.iter().filter(|k| marks.get(k).is_some_and(|m| *m != 'u')).count();
                let uses = list.iter().filter(|k| marks.get(k) == Some(&'u')).count();
                match p.item {
                    Item::Node(i) if is_file(&data.nodes[i]) => match marks.get(&i) {
                        Some('u') => {
                            p.paint.hatched = true;
                            p.paint.sub = tr("Uses a change").into();
                        }
                        Some(&letter) => {
                            let colour = status_color(letter);
                            p.paint.badge = Some(letter);
                            p.paint.fill = Some(tint(colour, 0.12));
                            p.paint.band = vec![(colour, 1.)];
                            let lines = s.code.changes.as_ref().and_then(|c| c.lines.get(&item_path(data, p.item))).copied();
                            p.paint.sub = match (letter, lines) {
                                ('A', Some((a, _))) => trf!("+{added} · new file", added = pages::number_text(a)),
                                ('A', None) => tr("New file").into(),
                                (_, Some((a, r))) => format!("+{} \u{2212}{}", pages::number_text(a), pages::number_text(r)),
                                _ => tr("Changed").into(),
                            };
                        }
                        None => p.paint.sub = sub_line(s, data, f, p.item),
                    },
                    _ => {
                        if changed > 0 {
                            p.paint.fill = Some(tint(AMBER(), 0.12));
                            p.paint.band = vec![(AMBER(), 1.)];
                        }
                        p.paint.sub = match (changed, uses) {
                            (0, 0) if s.code.changes.is_some() => tr("No changes").into(),
                            (0, 0) => sub_line(s, data, f, p.item),
                            (c, 0) => trn!(c, "{n} changed", "{n} changed"),
                            (0, u) => trn!(u, "{n} uses a change", "{n} use a change"),
                            (c, u) => format!("{} · {}", trn!(c, "{n} changed", "{n} changed"), trn!(u, "{n} uses a change", "{n} use a change")),
                        };
                    }
                }
            }
            for m in marks.values() {
                match m {
                    'A' => legend.added += 1,
                    'u' => legend.uses += 1,
                    _ => legend.changed += 1,
                }
            }
        }
    }
    layout.legend = legend;
}

/// The line under a card's name when its colour says nothing more.
fn sub_line(s: &MapPage, data: &RepositoryMap, f: &Facts, item: Item) -> String {
    match item {
        Item::Node(i) if is_file(&data.nodes[i]) => {
            let path = item_path(data, item);
            match s.code.links.as_ref().and_then(|l| l.lines.get(&path)) {
                Some(&n) => trn!(n, "{n} line", "{n} lines"),
                None => f.kind[i].map(kind_name).unwrap_or_default().to_owned(),
            }
        }
        Item::Node(i) => trn!(data.nodes[i].files, "{n} file", "{n} files"),
        Item::Files(Some(i)) => trf!("in {folder}", folder = drawing::folder_name(&data.nodes[i])),
        Item::Files(None) => tr("in the top folder").into(),
    }
}

/// Owner: CODEOWNERS when the snapshot has one, else the person with the most commits
/// to each file. Three owners get colours, the most files first; the rest are Others.
fn owners(s: &MapPage, data: &RepositoryMap, f: &Facts, layout: &mut Picture, legend: &mut Legend) {
    legend.owners_limited = s.code.links.as_ref().is_some_and(|l| l.owners_limited);
    // Each file's owner: a slot (0-2 coloured, 3 others), 4 unowned, 5 nobody.
    let rules = s.code.links.as_ref().and_then(|l| l.owners.as_ref());
    let history = s.code.history.as_ref();
    if rules.is_none() && history.is_none() {
        let reading = s.code.reading_history() || s.code.reading_links();
        for p in &mut layout.placed {
            p.paint.sub = if reading { tr("Reading commits").into() } else { sub_line(s, data, f, p.item) };
        }
        legend.owners_too_large = s.code.links.as_ref().is_some_and(|l| l.owners_too_large);
        return;
    }
    let names: Vec<String>;
    let mut slot_of_file: HashMap<usize, usize> = HashMap::new();
    let file_nodes: Vec<usize> = (0..data.nodes.len()).filter(|i| is_file(&data.nodes[*i])).collect();
    match rules {
        Some(r) => {
            names = r.sets.iter().take(OWNER_SLOTS).map(|set| set.join(" ")).collect();
            for &i in &file_nodes {
                let slot = match r.files.get(&item_path(data, Item::Node(i))) {
                    Some(Some(set)) => (*set as usize).min(OWNER_SLOTS),
                    Some(None) => 4,
                    None => 5,
                };
                slot_of_file.insert(i, slot);
            }
            legend.owners_from = Some(label(&r.file));
            legend.owners_skipped = r.skipped;
        }
        None => {
            legend.owners_too_large = s.code.links.as_ref().is_some_and(|l| l.owners_too_large);
            let h = history.expect("history");
            let mut owner: HashMap<usize, u32> = HashMap::new();
            let mut files_by: HashMap<u32, usize> = HashMap::new();
            for &i in &file_nodes {
                if let Some((a, _)) = h.owner(&item_path(data, Item::Node(i))) {
                    owner.insert(i, a);
                    *files_by.entry(a).or_default() += 1;
                }
            }
            let mut ranked: Vec<(u32, usize)> = files_by.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            names = ranked.iter().take(OWNER_SLOTS).map(|(a, _)| label(&h.authors[*a as usize])).collect();
            let slot: HashMap<u32, usize> = ranked.iter().enumerate().map(|(k, (a, _))| (*a, k.min(OWNER_SLOTS))).collect();
            for &i in &file_nodes {
                slot_of_file.insert(i, owner.get(&i).map_or(5, |a| slot[a]));
            }
        }
    }
    let mut seen = [false; 6];
    for p in &mut layout.placed {
        let mut list = Vec::new();
        under(f, data, p.item, &mut list);
        let mut counts = [0usize; 6];
        for k in list {
            counts[slot_of_file.get(&k).copied().unwrap_or(5)] += 1;
        }
        for (k, c) in counts.iter().enumerate() {
            seen[k] |= *c > 0;
        }
        let total = counts.iter().sum::<usize>().max(1) as f32;
        p.paint.band = (0..4)
            .filter(|k| counts[*k] > 0)
            .map(|k| (if k < OWNER_SLOTS { owner_colour(k) } else { DIM() }, counts[k] as f32 / total))
            .chain((4..6).filter(|k| counts[*k] > 0).map(|k| (Color32::TRANSPARENT, counts[k] as f32 / total)))
            .collect();
        let top = (0..6).max_by_key(|k| (counts[*k], std::cmp::Reverse(*k))).unwrap_or(5);
        p.paint.fill = (top < OWNER_SLOTS).then(|| tint(owner_colour(top), 0.13)).or((top == 3).then(|| tint(DIM(), 0.13)));
        p.paint.dashed = top >= 4;
        p.paint.sub = match top {
            k if k < names.len() => names[k].clone(),
            3 => tr("Others").into(),
            4 => tr("Unowned").into(),
            _ => tr("Nobody").into(),
        };
    }
    legend.owners = names.iter().enumerate().filter(|(k, _)| seen[*k]).map(|(k, n)| (Some(owner_colour(k)), n.clone())).collect();
    if seen[3] {
        legend.owners.push((Some(DIM()), tr("Others").into()));
    }
    if seen[4] {
        legend.owners.push((None, tr("Unowned").into()));
    }
    if seen[5] {
        legend.owners.push((None, tr("Nobody").into()));
    }
}

/// The colour button beside the views: Kind, Activity, Owner or Changes.
pub(super) fn colour_menu(ui: &mut Ui, s: &mut MapPage, widgets: &mut HashMap<String, UiControl>) {
    let text = trf!("Colour: {name}", name = s.code.colour.name());
    room(ui, text_width(ui, &text) + 46.);
    let response = ui.menu_button(text, |ui| {
        for c in Colour::ALL {
            let r = ui.radio(s.code.colour == c, c.name());
            register(ui, widgets, &format!("map-colour-{}", c.id()), &r);
            if r.clicked() {
                s.code.colour = c;
                s.code.changed_only &= c == Colour::Changes;
                ui.close();
            }
        }
    });
    register(ui, widgets, "map-colour", &response.response);
    response.response.on_hover_text(tr("Colour the cards by kind, activity, owner or changes"));
}

/// Whether the chip beside the views shows Changed only.
pub(super) fn changed_only_chip(s: &MapPage) -> bool {
    s.data.as_ref().is_some_and(|d| d.kind == MapKind::Code) && !s.alternate && s.code.colour == Colour::Changes
}

/// The default base when Changes is first chosen: the main branch, or its upstream when
/// the snapshot is the main branch.
pub(super) fn default_base(tab: &Tab) -> String {
    let branches: Vec<String> = revision_choices(tab).into_iter().map(|(name, _)| name).collect();
    let main = ["main", "master", "trunk", "develop"].into_iter().find(|m| branches.iter().any(|b| b == m)).map(str::to_owned);
    let current = crate::style::head_name(tab.snapshot.as_ref().and_then(|s| s.head.as_ref()));
    match main {
        Some(m) if m != current => m,
        Some(m) => format!("origin/{m}"),
        None => String::new(),
    }
}

fn band_shape(painter: &Painter, rect: Rect, band: &[(Color32, f32)], height: f32, radius: f32) {
    let r = Rect::from_min_size(rect.min, vec2(rect.width(), height));
    let mut x = r.left();
    let total: f32 = band.iter().map(|(_, v)| *v).sum::<f32>().max(0.0001);
    let clip = painter.with_clip_rect(painter.clip_rect().intersect(Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + radius.max(height)))));
    for (k, (c, v)) in band.iter().enumerate() {
        let w = r.width() * v / total;
        if *c != Color32::TRANSPARENT && w > 0.5 {
            let gap = if k + 1 < band.len() { 1.5 } else { 0. };
            clip.rect_filled(Rect::from_min_size(pos2(x, r.top()), vec2((w - gap).max(0.5), height)), 0., *c);
        }
        x += w;
    }
}
fn hatch(painter: &Painter, rect: Rect, color: Color32, zoom: f32) {
    let step = (10. * zoom).max(4.);
    let clip = painter.with_clip_rect(painter.clip_rect().intersect(rect.shrink(1.)));
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        clip.line_segment([pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())], Stroke::new(2.5 * zoom.min(1.), color));
        x += step;
    }
}
fn dashed_rect(painter: &Painter, rect: Rect, stroke: Stroke) {
    let points = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    painter.extend(Shape::dashed_line(&points, stroke, 5., 3.));
}

/// How many files a folder holds itself; the top folder for None.
pub fn own_files(f: &Facts, folder: Option<usize>) -> usize {
    folder.map_or(f.top_files.len(), |c| f.files[c].len())
}
pub fn title(data: &RepositoryMap, f: &Facts, item: Item) -> (Icon, Color32, String) {
    match item {
        Item::Node(i) => {
            let n = &data.nodes[i];
            match n.kind {
                MapNodeKind::File => (Icon::File, MUTED(), label(&n.label)),
                // The top Cargo.toml, which Cargo names after its package.
                _ if is_file(n) => (Icon::File, MUTED(), "Cargo.toml".into()),
                MapNodeKind::Crate => (Icon::Package, TEAL(), drawing::folder_name(n)),
                _ => (Icon::Folder, TEAL(), drawing::folder_name(n)),
            }
        }
        Item::Files(c) => (Icon::File, HINT(), trn!(own_files(f, c), "{n} file", "{n} files")),
    }
}

/// Index visible obstacles by horizontal strip so each candidate only checks
/// nearby cards and labels. All dimensions here are in screen points.
struct CountSpace {
    viewport: Rect,
    rows: Vec<Vec<Rect>>,
}

/// When a fitted card still has room for one line, give the filename that line.
fn compact_title(painter: &Painter, bounds: Rect, name: String) {
    let ink = painter.with_clip_rect(bounds.intersect(painter.clip_rect()));
    let text = ink.layout_job(drawing::title_job(name, 0.75, bounds.width().max(1.)));
    ink.galley(pos2(bounds.left(), bounds.center().y - text.size().y / 2.), text, TEXT());
}

impl CountSpace {
    const ROW_HEIGHT: f32 = 32.;

    fn new(viewport: Rect) -> Self {
        let rows = (viewport.height() / Self::ROW_HEIGHT).ceil().max(1.) as usize;
        Self { viewport, rows: vec![Vec::new(); rows] }
    }

    fn bands(&self, r: Rect) -> std::ops::RangeInclusive<usize> {
        let row = |y: f32| (((y - self.viewport.top()) / Self::ROW_HEIGHT) as usize).min(self.rows.len() - 1);
        row(r.top())..=row(r.bottom())
    }

    fn reserve(&mut self, r: Rect) {
        let r = r.expand(2.).intersect(self.viewport);
        if r.is_positive() {
            for row in self.bands(r) { self.rows[row].push(r); }
        }
    }

    fn place(&mut self, curve: &epaint::CubicBezierShape, size: Vec2) -> Option<Rect> {
        for fraction in [0.5, 0.38, 0.62, 0.26, 0.74, 0.14, 0.86] {
            let pill = Rect::from_center_size(curve.sample(fraction), size);
            if self.viewport.contains_rect(pill) && !self.rows[self.bands(pill)].iter().flatten().any(|r| r.intersects(pill)) {
                self.reserve(pill);
                return Some(pill);
            }
        }
        None
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    #[test]
    fn count_badges_move_along_the_arrow_and_yield_to_card_labels() {
        let viewport = Rect::from_min_max(pos2(0., 0.), pos2(240., 90.));
        let mut space = CountSpace::new(viewport);
        let curve = epaint::CubicBezierShape::from_points_stroke(
            [pos2(0., 32.), pos2(80., 32.), pos2(160., 32.), pos2(240., 32.)], false, Color32::TRANSPARENT, Stroke::NONE,
        );
        let card = Rect::from_center_size(curve.sample(0.5), vec2(50., 26.));
        space.reserve(card);
        let size = vec2(20., 14.);
        let first = space.place(&curve, size).unwrap();
        let second = space.place(&curve, size).unwrap();
        assert!(viewport.contains_rect(first) && viewport.contains_rect(second));
        assert!(!first.intersects(card) && !second.intersects(card) && !first.intersects(second));
        assert!((first.center().y - 32.).abs() < 0.001);
        space.reserve(viewport);
        assert!(space.place(&curve, size).is_none());
    }
}

pub(super) fn canvas(ui: &mut Ui, s: &mut MapPage, rect: Rect, covered: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    s.code.link_count_rects.clear();
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let canvas = ui.interact(rect, ui.scope_id().with("repository-map-canvas"), if covered { Sense::hover() } else { Sense::click_and_drag() });
    if !covered {
        register(ui, widgets, "map-canvas", &canvas);
    }
    for x in (rect.left() as i32..rect.right() as i32).step_by(24) {
        for y in (rect.top() as i32..rect.bottom() as i32).step_by(24) {
            painter.circle_filled(pos2(x as f32, y as f32), 0.7, BORDER().gamma_multiply(0.5));
        }
    }
    let Some(data) = s.data.clone() else {
        painter.text(rect.center(), Align2::CENTER_CENTER, if s.running() { tr("Reading the repository") } else { tr("Choose Refresh to read this map") }, FontId::proportional(13.), MUTED());
        return;
    };
    // Reads the map needs: links first, then what the colour asks for.
    if let Some(tip) = &data.tip
        && let Some(read) = wanted(s, tip)
    {
        emit(intents, MapIntent::CodeRead(read));
    }
    let (Some(layout), Some(facts)) = (s.code.layout.clone(), s.code.facts.clone()) else { return };
    let size = rect.size();
    if s.fit || s.last_canvas != size {
        fit(s, &layout, rect);
        if s.zoom < 0.45 || size.x < 700. && s.zoom < 0.75 {
            focus(s, &layout, rect);
        }
        s.fit = false;
        s.last_canvas = size;
    }
    if s.focus {
        focus(s, &layout, rect);
        s.focus = false;
    }
    if !covered {
        if canvas.dragged() {
            s.pan += ui.input(|i| i.pointer.delta());
        }
        if canvas.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0. {
                zoom_at(s, (delta * 0.002).exp(), ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center()) - rect.min);
            }
        }
        if canvas.clicked() {
            canvas.request_focus();
        }
    }
    let zoom = s.zoom;
    let to_screen = |p: Pos2| rect.min + s.pan + p.to_vec2() * zoom;
    let screen = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
    let selected = match s.code.group {
        Some(g) => layout.index(Item::Files(g)),
        None => s.selected.and_then(|i| layout.index(Item::Node(i))),
    };
    let query = s.query.to_lowercase();
    let matched = |p: &Placed| -> bool {
        if query.is_empty() {
            return true;
        }
        match p.item {
            Item::Node(i) => {
                let n = &data.nodes[i];
                label(&n.label).to_lowercase().contains(&query) || n.path.as_ref().is_some_and(|x| label(x).to_lowercase().contains(&query))
            }
            Item::Files(_) => false,
        }
    };
    // Boxes, outermost first.
    let mut boxes: Vec<usize> = (0..layout.placed.len()).filter(|i| layout.placed[*i].open).collect();
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = layout.placed[i].parent {
            d += 1;
            i = p;
        }
        d
    };
    boxes.sort_by_key(|i| depth(*i));
    let mut toggled: Option<usize> = None;
    let mut picked: Option<usize> = None;
    let hit_area = rect.shrink2(vec2(0., 36.));
    let mut count_space = (zoom >= 0.30 && !layout.arrows.is_empty()).then(|| CountSpace::new(hit_area));
    for &b in &boxes {
        let p = &layout.placed[b];
        let r = screen(p.rect);
        if !r.intersects(rect) {
            continue;
        }
        let on = selected == Some(b);
        painter.rect(r, 10. * zoom.min(1.), VIEW_PANEL().gamma_multiply(0.72), Stroke::new(if on { 1.8 } else { 1. }, if on { BLUE() } else { BORDER() }), StrokeKind::Inside);
        band_shape(&painter, r, &p.paint.band, 3. * zoom.max(0.5), 10. * zoom.min(1.));
        let head = Rect::from_min_size(r.min, vec2(r.width(), HEADER * zoom));
        if head.width() >= 30. && head.height() >= 11. {
            let ink = painter.with_clip_rect(r.shrink(2.).intersect(rect));
            if let Some(space) = &mut count_space { space.reserve(head); }
            icon(&ink, Rect::from_min_size(head.min + vec2(10., 13.) * zoom, vec2(11., 11.) * zoom), Icon::Chevron, HINT());
            let (kind_icon, tint, name) = title(&data, &facts, p.item);
            if zoom < 0.55 {
                compact_title(&ink, Rect::from_min_max(head.min + vec2(26. * zoom, 1.), head.max - vec2(2., 0.)), name.clone());
            } else {
                icon(&ink, Rect::from_min_size(head.min + vec2(27., 11.) * zoom, vec2(15., 15.) * zoom), kind_icon, if on { BLUE() } else { tint });
                let mut job = text::LayoutJob::default();
                let chain: String = p.chain.iter().map(|c| format!("{} / ", drawing::folder_name(&data.nodes[*c]))).collect();
                if !chain.is_empty() {
                    job.append(&chain, 0., TextFormat::simple(FontId::proportional(13. * zoom), HINT()));
                }
                job.append(&name, 0., TextFormat::simple(semibold(13. * zoom), TEXT()));
                if !p.paint.sub.is_empty() {
                    job.append(&format!("   {}", p.paint.sub), 0., TextFormat::simple(FontId::proportional(11.5 * zoom), HINT()));
                }
                job.wrap = text::TextWrapping::truncate_at_width((r.width() - 56. * zoom).max(1.));
                ink.galley(head.min + vec2(48., 11.) * zoom, ink.layout_job(job), TEXT());
            }
            if !covered {
                let header = head.intersect(hit_area);
                if header.width() > 12. && header.height() > 8. {
                    let item = p.item;
                    let id = match item {
                        Item::Node(i) => i,
                        Item::Files(_) => continue,
                    };
                    let chevron = Rect::from_min_size(header.min, vec2(26. * zoom, header.height())).intersect(header);
                    let rest = Rect::from_min_max(pos2(chevron.right(), header.top()), header.max);
                    let r1 = ui.interact(chevron, ui.scope_id().with(("map-toggle", id)), Sense::click()).on_hover_text(tr("Close folder"));
                    register(ui, widgets, &format!("map-toggle-{id}"), &r1);
                    if r1.clicked() {
                        toggled = Some(b);
                    }
                    if rest.width() > 8. {
                        let r2 = ui.interact(rest, ui.scope_id().with(("map-node", id)), Sense::click()).on_hover_text(format!("{name}\n{}", p.paint.sub));
                        register(ui, widgets, &format!("map-node-{id}"), &r2);
                        if r2.clicked() {
                            picked = Some(b);
                        }
                    }
                }
            }
        }
    }
    for c in &layout.captions {
        if zoom >= 0.4 {
            let caption = painter.text(to_screen(*c), Align2::LEFT_TOP, tr("NOT CONNECTED"), semibold(11. * zoom.min(1.2)), HINT());
            if let Some(space) = &mut count_space { space.reserve(caption); }
        }
    }
    // Arrows: from the user's bottom to the top of what it uses, spread along both edges.
    let mut out_ports: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut in_ports: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, a) in layout.arrows.iter().enumerate() {
        out_ports.entry(a.from).or_default().push(k);
        in_ports.entry(a.to).or_default().push(k);
    }
    let centre_x = |i: usize| layout.placed[i].rect.center().x;
    for list in out_ports.values_mut() {
        list.sort_by(|x, y| centre_x(layout.arrows[*x].to).total_cmp(&centre_x(layout.arrows[*y].to)));
    }
    for list in in_ports.values_mut() {
        list.sort_by(|x, y| centre_x(layout.arrows[*x].from).total_cmp(&centre_x(layout.arrows[*y].from)));
    }
    let port = |list: &[usize], k: usize, r: Rect| {
        let at = list.iter().position(|x| *x == k).unwrap_or(0) as f32;
        let n = list.len() as f32;
        r.left() + r.width() * (at + 1.) / (n + 1.)
    };
    let mut pills = Vec::new();
    for (k, a) in layout.arrows.iter().enumerate() {
        let (from, to) = (layout.placed[a.from].rect, layout.placed[a.to].rect);
        let active = selected.is_some_and(|sel| a.from == sel || a.to == sel);
        let faded = selected.is_some() && !active;
        let color = if active { BLUE() } else { HINT().gamma_multiply(if faded { 0.35 } else { 0.8 }) };
        let (start, end) = if a.up {
            (pos2(port(&out_ports[&a.from], k, from), from.top()), pos2(port(&in_ports[&a.to], k, to), to.bottom()))
        } else {
            (pos2(port(&out_ports[&a.from], k, from), from.bottom()), pos2(port(&in_ports[&a.to], k, to), to.top()))
        };
        let (s0, e0) = (to_screen(start), to_screen(end));
        if !Rect::from_two_pos(s0, e0).intersects(rect) {
            continue;
        }
        let bend = ((e0.y - s0.y).abs() * 0.5).max(24. * zoom);
        let dir = if a.up { -1. } else { 1. };
        let points = [s0, s0 + vec2(0., bend * dir), e0 - vec2(0., bend * dir), e0];
        let stroke = Stroke::new(if active { 1.8 } else { 1.2 }, color);
        let shape = epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke);
        if a.up {
            let samples: Vec<Pos2> = (0..=24).map(|t| shape.sample(t as f32 / 24.)).collect();
            painter.extend(Shape::dashed_line(&samples, stroke, 5., 4.));
        } else {
            painter.add(shape.clone());
        }
        let tip = vec2(0., dir);
        let cross = vec2(-tip.y, tip.x);
        painter.add(Shape::convex_polygon(vec![e0, e0 - tip * 7. + cross * 3.5, e0 - tip * 7. - cross * 3.5], color, Stroke::NONE));
        if count_space.is_some() {
            pills.push((shape, a.files, active));
        }
    }
    // Cards.
    for (i, p) in layout.placed.iter().enumerate() {
        if p.open {
            continue;
        }
        let r = screen(p.rect);
        if !r.intersects(rect) {
            continue;
        }
        let on = selected == Some(i);
        if let Some(space) = &mut count_space { space.reserve(r); }
        let fade = if matched(p) { 1. } else { 0.45 };
        let radius = 8. * zoom.min(1.);
        painter.rect_filled(r, radius, p.paint.fill.unwrap_or(VIEW_RAISED()).gamma_multiply(fade));
        if p.paint.hatched {
            hatch(&painter, r, AMBER().gamma_multiply(0.16 * fade), zoom);
        }
        band_shape(&painter, r, &p.paint.band, 4. * zoom.max(0.5), radius);
        let border = if on { BLUE() } else if p.paint.hatched { AMBER() } else { BORDER() };
        if p.paint.dashed || p.paint.hatched || matches!(p.item, Item::Files(_)) {
            dashed_rect(&painter, r, Stroke::new(1., border.gamma_multiply(fade)));
        } else {
            painter.rect_stroke(r, radius, Stroke::new(if on { 2. } else { 1. }, border.gamma_multiply(fade)), StrokeKind::Inside);
        }
        if on {
            painter.rect_stroke(r.expand(3.), radius + 3., Stroke::new(3., BLUE().gamma_multiply(0.18)), StrokeKind::Outside);
        }
        let folder = matches!(p.item, Item::Node(n) if is_folder(&data.nodes[n]));
        let group = matches!(p.item, Item::Files(_));
        if zoom < 0.4 && r.width() >= 30. && r.height() >= 13. {
            compact_title(&painter, r.shrink2(vec2(3., 1.)), title(&data, &facts, p.item).2);
        } else if zoom >= 0.4 {
            let ink = painter.with_clip_rect(r.shrink(3.).intersect(rect));
            let mut left = 10.;
            if folder || group {
                icon(&ink, Rect::from_min_size(r.min + vec2(8., 13.) * zoom, vec2(10., 10.) * zoom), Icon::ChevronRight, HINT().gamma_multiply(fade));
                left += 14.;
            }
            let (kind_icon, tint, name) = title(&data, &facts, p.item);
            icon(&ink, Rect::from_min_size(r.min + vec2(left, 11.) * zoom, vec2(14., 14.) * zoom), kind_icon, if on { BLUE() } else { tint.gamma_multiply(fade) });
            let badge = if p.paint.badge.is_some() { 26. } else { 0. };
            let at = r.min + vec2(left + 20., 9.) * zoom;
            ink.galley(at, ink.layout_job(drawing::title_job(name.clone(), zoom, (r.right() - at.x - (8. + badge) * zoom).max(1.))), TEXT().gamma_multiply(fade));
            if zoom > 0.7 {
                let mut job = text::LayoutJob::simple_singleline(p.paint.sub.clone(), FontId::proportional(11.5 * zoom), HINT().gamma_multiply(fade));
                job.wrap = text::TextWrapping::truncate_at_width((r.width() - 20. * zoom).max(1.));
                ink.galley(r.min + vec2(10., 35.) * zoom, ink.layout_job(job), HINT());
            }
            if let Some(letter) = p.paint.badge {
                status_badge(&ink, pos2(r.right() - 16. * zoom, r.top() + 18. * zoom), letter);
            }
        }
        let hit = r.intersect(hit_area);
        if !covered && hit.width() > 20. && hit.height() >= 13. {
            let id = match p.item {
                Item::Node(n) => format!("map-node-{n}"),
                Item::Files(Some(c)) => format!("map-files-{c}"),
                Item::Files(None) => "map-files-top".into(),
            };
            let name = title(&data, &facts, p.item).2;
            let response = ui.interact(hit, ui.scope_id().with(("map-card", i)), Sense::click()).on_hover_text(format!("{name}\n{}", p.paint.sub));
            register(ui, widgets, &id, &response);
            if response.clicked() {
                picked = Some(i);
            }
            if folder || group {
                let chevron = Rect::from_min_size(r.min, vec2(24. * zoom, r.height())).intersect(hit);
                if chevron.width() > 8. {
                    let toggle_id = match p.item {
                        Item::Node(n) => format!("map-toggle-{n}"),
                        Item::Files(Some(c)) => format!("map-files-open-{c}"),
                        Item::Files(None) => "map-files-open-top".into(),
                    };
                    let r = ui.interact(chevron, ui.scope_id().with(("map-card-toggle", i)), Sense::click()).on_hover_text(crate::i18n::tr("Open folder"));
                    register(ui, widgets, &toggle_id, &r);
                    if r.clicked() {
                        toggled = Some(i);
                        picked = None;
                    }
                }
            }
        }
    }
    if let Some(mut space) = count_space {
        // Prioritize selected edges, then counts that summarize several files.
        pills.sort_by_key(|(_, files, active)| (!*active, std::cmp::Reverse(*files)));
        for (curve, files, active) in pills {
            let galley = painter.layout_no_wrap(pages::number_text(files), FontId::proportional(10.5), if active { BLUE() } else { MUTED() });
            let Some(pill) = space.place(&curve, galley.size() + vec2(12., 4.)) else { continue };
            painter.rect(pill, 9., VIEW_PANEL(), Stroke::new(1., if active { BLUE().gamma_multiply(0.4) } else { CARD_STROKE() }), StrokeKind::Inside);
            painter.galley(pill.center() - galley.size() / 2., galley, MUTED());
            s.code.link_count_rects.push(pill);
        }
    }
    // A click toggles or chooses once everything is drawn.
    if let Some(t) = toggled {
        let p = &layout.placed[t];
        match p.item {
            Item::Node(n) => {
                s.toggle_group(n);
                s.selected = Some(n);
                s.code.group = None;
            }
            Item::Files(c) => {
                let path = item_path(&data, Item::Files(c));
                if !s.code.files_open.remove(&path) {
                    s.code.files_open.insert(path);
                }
                s.filter_key = None;
            }
        }
    } else if let Some(i) = picked {
        match layout.placed[i].item {
            Item::Node(n) => {
                s.selected = Some(n);
                s.code.group = None;
            }
            Item::Files(c) => s.code.group = Some(c),
        }
        s.cell = None;
        s.show_details = true;
    }
    strips(ui, s, &data, rect, intents, widgets);
    if covered {
        return;
    }
    legend(ui, s, &layout, rect, widgets);
    zoom_bar(ui, s, &layout, rect, widgets);
}

/// Reading, the read's limit, nothing to connect, and errors: one line over the map.
fn strips(ui: &mut Ui, s: &mut MapPage, data: &RepositoryMap, rect: Rect, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let _ = data;
    let cut = s.code.layout.as_ref().is_some_and(|l| l.cut);
    let (text, tone, stop): (String, Color32, bool) = if s.code.reading_links() {
        (tr("Reading code").into(), MUTED(), true)
    } else if let Some(e) = s.code.error() {
        (e.clone(), RED(), false)
    } else if s.code.stopped && !s.code.running() {
        (tr("Stopped. Refresh reads the code again").into(), MUTED(), false)
    } else if let Some(links) = &s.code.links {
        if cut {
            (trf!("Some folders stay closed: the map shows up to {count} cards and boxes", count = pages::number_text(ITEMS)), AMBER(), false)
        } else if !s.code.too_large.is_empty() {
            (tr("Some packages hold more code than one read takes, so they have no arrows").into(), AMBER(), false)
        } else if !links.unread.is_empty() {
            (trf!("Arrows cover the first {count} code files. Open a folder to read the code in it.", count = pages::number_text(CODE_MAP_FILES)), AMBER(), false)
        } else if links.limited {
            (tr("Code map read limit reached. Some connections or ownership data are unavailable.").into(), AMBER(), false)
        } else if links.packages.is_empty() {
            (tr("No package here the map can read, so there are no arrows").into(), MUTED(), false)
        } else {
            return;
        }
    } else {
        return;
    };
    let painter = ui.painter().with_clip_rect(rect);
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.5), tone);
    let width = (galley.size().x + if stop { 96. } else { 26. }).min(rect.width() - 20.);
    let strip = Rect::from_min_size(rect.left_top() + vec2(10., 10.), vec2(width, 34.));
    painter.rect(strip, 8., VIEW_PANEL(), Stroke::new(1., if tone == AMBER() { AMBER().gamma_multiply(0.4) } else { CARD_STROKE() }), StrokeKind::Inside);
    let mut x = strip.left() + 12.;
    if stop {
        Spinner::new().size(14.).paint_at(ui, Rect::from_center_size(pos2(x + 7., strip.center().y), vec2(14., 14.)));
        x += 22.;
    }
    painter.galley(pos2(x, strip.center().y - galley.size().y / 2.), galley, tone);
    if stop {
        let button_rect = Rect::from_min_max(pos2(strip.right() - 64., strip.top() + 4.), strip.right_bottom() - vec2(6., 4.));
        let mut child = ui.new_child(UiBuilder::new().max_rect(button_rect).layout(Layout::right_to_left(Align::Center)));
        if button(&mut child, widgets, "map-code-stop", crate::i18n::tr("Stop"), true) {
            s.code.stop();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }
    let _ = intents;
}

/// The key at the bottom left: what the colour means, with the period or the base.
fn legend(ui: &mut Ui, s: &mut MapPage, layout: &Picture, rect: Rect, widgets: &mut HashMap<String, UiControl>) {
    let area = Rect::from_min_max(pos2(rect.left() + 10., rect.bottom() - 42.), pos2(rect.right() - 330., rect.bottom() - 8.));
    if area.width() < 160. {
        return;
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 8.;
    let frame = Frame::new().fill(PANEL()).stroke(Stroke::new(1., CARD_STROKE())).corner_radius(8.).inner_margin(Margin::symmetric(10, 4));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            let swatch = |ui: &mut Ui, c: Option<Color32>, text: &str| {
                let (r, _) = ui.allocate_exact_size(vec2(10., 10.), Sense::hover());
                match c {
                    Some(c) => {
                        ui.painter().rect_filled(r, 3., c);
                    }
                    None => dashed_rect(ui.painter(), r, Stroke::new(1., HINT())),
                }
                ui.label(RichText::new(text).size(12.).color(MUTED()));
            };
            match s.code.colour {
                Colour::Kind => {
                    for k in &layout.legend.kinds {
                        swatch(ui, kind_colour(*k), kind_name(*k));
                    }
                    ui.label(RichText::new(tr("Arrows point to what is used · numbers count the files using it")).size(12.).color(HINT()));
                }
                Colour::Activity => {
                    ui.label(RichText::new(tr("Commits in")).size(12.).color(MUTED()));
                    let periods = [(Period::Days30, crate::i18n::tr("30 days"), "30"), (Period::Days90, crate::i18n::tr("90 days"), "90"), (Period::All, tr("All time"), "all")];
                    let shown = periods.iter().find(|p| p.0 == s.code.period).map_or(tr("All time"), |p| p.1);
                    let r = ui.menu_button(RichText::new(shown).size(12.), |ui| {
                        for (period, text, id) in periods {
                            let r = ui.radio(s.code.period == period, text);
                            register(ui, widgets, &format!("map-code-period-{id}"), &r);
                            if r.clicked() {
                                s.code.period = period;
                                ui.close();
                            }
                        }
                    });
                    register(ui, widgets, "map-code-period", &r.response);
                    ui.label(RichText::new("1").size(12.).color(HINT()));
                    for c in [RAMP_1(), RAMP_2(), RAMP_3(), BLUE()] {
                        let (r, _) = ui.allocate_exact_size(vec2(16., 10.), Sense::hover());
                        ui.painter().rect_filled(r, 2., c);
                    }
                    ui.label(RichText::new(pages::number_text(layout.legend.most.max(1))).size(12.).color(HINT()));
                }
                Colour::Owner => {
                    let mut from = match &layout.legend.owners_from {
                        Some(file) => trf!("CODEOWNERS · {file}", file = file),
                        None if layout.legend.owners_limited => tr("CODEOWNERS matching limit reached · most commits").into(),
                        None if layout.legend.owners_too_large => tr("CODEOWNERS is too large to read · most commits").into(),
                        None => tr("No CODEOWNERS file · most commits").into(),
                    };
                    if layout.legend.owners_skipped > 0 {
                        from = format!("{from} · {}", trn!(layout.legend.owners_skipped, "{n} line skipped", "{n} lines skipped"));
                    }
                    ui.label(RichText::new(from).size(12.).color(MUTED()));
                    for (c, name) in &layout.legend.owners {
                        swatch(ui, *c, name);
                    }
                }
                Colour::Changes => {
                    ui.label(RichText::new(tr("Since")).size(12.).color(MUTED()));
                    let field = TextEdit::singleline(&mut s.code.base).desired_width(130.).hint_text(tr("Branch, tag or commit")).font(FontId::proportional(12.));
                    let r = ui.add(field);
                    register(ui, widgets, "map-code-base", &r);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let compare = ui.add_enabled(!s.code.base.trim().is_empty(), Button::new(RichText::new(crate::i18n::tr("Compare")).size(12.)));
                    register(ui, widgets, "map-code-compare", &compare);
                    if enter || compare.clicked() {
                        // Read again against what is typed now.
                        s.code.changes = None;
                        s.code.failed[2] = false;
                        s.code.compare_again = true;
                    }
                    let legend = &layout.legend;
                    let outcome = s.code.changes.as_ref().map(|c| (c.outcome, c.files.is_empty(), c.base.clone()));
                    match outcome {
                        Some((ChangesOutcome::BaseNotFound, _, base)) => {
                            ui.label(RichText::new(trf!("No branch, tag or commit named {base}", base = base)).size(12.).color(RED()));
                        }
                        Some((ChangesOutcome::NoCommonHistory, _, base)) => {
                            ui.label(RichText::new(trf!("{base} has no history in common with this snapshot", base = base)).size(12.).color(RED()));
                        }
                        Some((ChangesOutcome::SeveralSplits, _, _)) => {
                            let commits = s.code.changes.as_ref().map(|c| c.splits.iter().map(|id| id.as_str()[..id.as_str().len().min(10)].to_owned()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                            ui.label(RichText::new(trf!("These split at {commits}. Type one of them as the base", commits = commits)).size(12.).color(AMBER()));
                        }
                        Some((ChangesOutcome::Ready, true, base)) => {
                            ui.label(RichText::new(trf!("Nothing changed since {base}", base = base)).size(12.).color(MUTED()));
                        }
                        Some((ChangesOutcome::Ready, false, _)) => {
                            for (letter, count, text) in [('A', legend.added, tr("Added")), ('M', legend.changed, tr("Changed"))] {
                                let (r, _) = ui.allocate_exact_size(vec2(16., 16.), Sense::hover());
                                status_badge(ui.painter(), r.center(), letter);
                                ui.label(RichText::new(format!("{text} {}", pages::number_text(count))).size(12.).color(MUTED()));
                            }
                            let (r, _) = ui.allocate_exact_size(vec2(14., 14.), Sense::hover());
                            hatch(ui.painter(), r, AMBER().gamma_multiply(0.5), 0.5);
                            dashed_rect(ui.painter(), r, Stroke::new(1., AMBER()));
                            ui.label(RichText::new(format!("{} {}", tr("Uses a change"), pages::number_text(legend.uses))).size(12.).color(MUTED()));
                        }
                        // Reading, or about to: a read that ended without a result says why above the map.
                        None if s.code.reading_changes() || !s.code.base.trim().is_empty() && !s.code.failed[2] => {
                            ui.add(Spinner::new().size(12.));
                        }
                        None if s.code.failed[2] => {}
                        None => {
                            ui.label(RichText::new(tr("Type a branch, tag or commit and press Enter")).size(12.).color(HINT()));
                        }
                    }
                }
            }
        });
    });
}

fn zoom_bar(ui: &mut Ui, s: &mut MapPage, layout: &Picture, rect: Rect, widgets: &mut HashMap<String, UiControl>) {
    let fit_label = tr("Fit");
    let focus_label = tr("Focus");
    let buttons_width: f32 = ["−", "+", fit_label, focus_label]
        .into_iter()
        .map(|text| {
            let galley = WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
            (galley.size().x + 2. * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x)
        })
        .sum();
    let bar_width = buttons_width + 40. + 4. * 4. + 6.;
    let bar = Rect::from_min_max(pos2((rect.right() - 10. - bar_width).max(rect.left() + 6.), rect.bottom() - 40.), rect.right_bottom() - vec2(10., 8.));
    ui.painter().rect(bar, 8., PANEL(), Stroke::new(1., CARD_STROKE()), StrokeKind::Inside);
    ui.scope_builder(UiBuilder::new().max_rect(bar.shrink(3.)).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
        if button(ui, widgets, "map-zoom-out", "−", true) {
            zoom_at(s, 0.8, rect.size() / 2.);
        }
        ui.add_sized([40., 20.], Label::new(RichText::new(format!("{}%", (s.zoom * 100.).round())).size(11.).color(MUTED())));
        if button(ui, widgets, "map-zoom-in", "+", true) {
            zoom_at(s, 1.25, rect.size() / 2.);
        }
        if button(ui, widgets, "map-fit", fit_label, true) {
            fit(s, layout, rect);
        }
        if button(ui, widgets, "map-focus", focus_label, s.selected.is_some() || s.code.group.is_some()) {
            s.focus = true;
        }
    });
}

fn fit(s: &mut MapPage, layout: &Picture, rect: Rect) {
    let b = layout.bounds;
    if b.is_finite() && b.is_positive() {
        let top = (rect.height() * 0.1).clamp(36., 65.);
        let available = vec2((rect.width() - 60.).max(1.), (rect.height() - top - 45.).max(1.));
        s.zoom = (available.x / b.width()).min(available.y / b.height()).clamp(0.00001, 1.);
        s.pan = vec2(30., top) + (available - b.size() * s.zoom) / 2. - b.min.to_vec2() * s.zoom;
    }
}
fn focus(s: &mut MapPage, layout: &Picture, rect: Rect) {
    let at = match s.code.group {
        Some(g) => layout.index(Item::Files(g)),
        None => s.selected.and_then(|i| layout.index(Item::Node(i))),
    };
    if let Some(i) = at {
        let r = layout.placed[i].rect;
        s.zoom = s.zoom.clamp(0.9, 1.);
        let target = if layout.placed[i].open { Rect::from_min_size(r.min, vec2(r.width(), r.height().min(rect.height() * 0.8 / s.zoom))) } else { r };
        s.pan = rect.size() / 2. - target.center().to_vec2() * s.zoom;
    }
}
fn zoom_at(s: &mut MapPage, factor: f32, point: Vec2) {
    let next = (s.zoom * factor).clamp(0.00001, 2.);
    s.pan = point - (point - s.pan) * (next / s.zoom);
    s.zoom = next;
}
