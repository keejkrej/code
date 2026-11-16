//! The editor's state and everything that changes it: opening a folder, listing it, opening
//! files and diffs in tabs, and running git.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use notify::Watcher as _;
use rune::prefs::Prefs;
use rune::{TextEdit, Tx, Waker};
use rune_kit::diff::{DiffMode, DiffState, FileDiff};
use rune_kit::syntax::{self, Highlight, Highlighter, Paint};
use rune_kit::{FileNode, TreeModel, natural_cmp};

use crate::doc::Doc;
use crate::git::{self, Change, FileStatus, Rev, Status};
use crate::jobs::Jobs;
use crate::workspace::{self, Content, Ignored, Listing};

/// Where settings are kept. Not `code`: `%APPDATA%\Code` belongs to Visual Studio Code.
pub const APP_ID: &str = "code-editor-rune";

/// Seconds a burst of file changes settles for before the folder is listed again.
const RESCAN_DELAY: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Explorer,
    Scm,
}

/// What the explorer knows about a row beyond its name.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub ignored: bool,
    /// A folder whose contents are listed when it is first opened.
    pub unlisted: bool,
}

pub type Node = FileNode<Meta>;

/// Git's marks on the explorer's rows, by path in the workspace.
#[derive(Default)]
pub struct Decor {
    pub files: HashMap<String, Change>,
    /// Folders with a change somewhere inside.
    pub dirs: HashSet<String>,
}

pub struct GitState {
    pub repo: PathBuf,
    /// The workspace's own path inside the repository: `""`, or `sub/dir/`.
    pub prefix: String,
    pub status: Status,
}

#[derive(Clone, Debug)]
pub enum Source {
    File,
    /// A changed file against what it changed from: the index against `HEAD` when `staged`,
    /// else the working tree against the index.
    Diff {
        file: FileStatus,
        staged: bool,
    },
}

pub enum Body {
    Loading,
    Code(Doc),
    Diff(Box<DiffState>),
    /// Why there is nothing to show: a binary file, one too large, one that is gone.
    Note(String),
}

pub struct Tab {
    /// Names its elements; never reused.
    pub uid: u64,
    /// The file or diff it shows: opening the same one again selects this tab.
    pub id: String,
    pub title: String,
    /// Shown in the breadcrumb: the path in the workspace, and for a diff which sides.
    pub detail: String,
    pub path: PathBuf,
    pub source: Source,
    pub body: Body,
    /// Opened by a single click: the next file opened that way takes its place.
    pub preview: bool,
    /// The file or the repository changed since this was loaded.
    stale: bool,
    /// The newest load asked for; older answers are dropped.
    load: u64,
}

pub enum Loaded {
    File { content: Content, highlights: Vec<Highlight>, lang: Option<&'static str> },
    Diff { old_path: Option<String>, new_path: Option<String>, old: Content, new: Content },
}

pub struct Scan {
    root: PathBuf,
    listing: Listing,
    git: Option<GitState>,
}

pub enum Event {
    Scanned(Box<Scan>),
    Loaded { uid: u64, load: u64, loaded: Box<Loaded> },
    GitDone { what: &'static str, result: Result<(), String> },
    FsChanged,
}

/// Watches the workspace for changes made outside the editor.
struct Watch {
    _watcher: notify::RecommendedWatcher,
    filter: Arc<Mutex<WatchFilter>>,
    /// A change is queued and not yet handled, so further ones need no event of their own.
    pending: Arc<AtomicBool>,
}

#[derive(Default)]
struct WatchFilter {
    root: PathBuf,
    /// Ignored folders: build output changes constantly and never shows in the editor.
    ignored: Vec<PathBuf>,
}

impl WatchFilter {
    fn relevant(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else { return true };
        let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy());
        if parts.next().as_deref() == Some(".git") {
            // Of git's own files only those that move HEAD, a branch or the index matter.
            let name = rel.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            let moved = matches!(name.as_ref(), "index" | "HEAD" | "ORIG_HEAD" | "MERGE_HEAD") || parts.any(|p| p == "refs");
            return moved && !name.ends_with(".lock");
        }
        !self.ignored.iter().any(|dir| path.starts_with(dir))
    }
}

pub struct App {
    pub jobs: Jobs<Event>,
    prefs: Prefs,
    watch: Option<Watch>,

    pub root: Option<PathBuf>,
    pub name: String,
    pub tree: TreeModel<Node>,
    pub decor: Rc<Decor>,
    pub git: Option<GitState>,
    /// The folder holds more than the explorer lists.
    pub truncated: bool,
    /// The first listing of this folder has not arrived yet.
    pub loading: bool,
    scanning: bool,
    rescan_queued: bool,
    rescan_due: Option<f64>,
    /// Ignored folders the explorer has opened, listed like any other from then on.
    open_ignored: HashSet<String>,
    /// Folders to open once the listing that holds their contents arrives.
    expand_next: Vec<String>,

    pub panel: Panel,
    pub sidebar_open: bool,
    pub sidebar_w: f32,
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_uid: u64,
    /// Diffs side by side rather than in one column.
    pub split: bool,
    dark: bool,
    no_diff: DiffState,

    pub commit: TextEdit,
    /// A git command is running.
    pub git_busy: bool,
    pub staged_closed: bool,
    pub changes_closed: bool,
    /// Something went wrong; shown until dismissed.
    pub notice: Option<String>,
    title: Option<String>,
    clock: f64,
}

impl App {
    /// `inline` runs every job where it is asked for, as a headless run needs. Such a run
    /// keeps no settings either: it must not read or replace the last real folder.
    pub fn new(inline: bool) -> App {
        App::with_prefs(inline, if inline { Prefs::memory() } else { Prefs::open(APP_ID) })
    }

    pub fn with_prefs(inline: bool, prefs: Prefs) -> App {
        App {
            jobs: Jobs::new(inline),
            split: prefs.get("diff_split").unwrap_or(true),
            sidebar_w: prefs.get::<f32>("sidebar_width").unwrap_or(260.0).clamp(160.0, 640.0),
            prefs,
            watch: None,
            root: None,
            name: String::new(),
            tree: TreeModel::new().key("files"),
            decor: Rc::default(),
            git: None,
            truncated: false,
            loading: false,
            scanning: false,
            rescan_queued: false,
            rescan_due: None,
            open_ignored: HashSet::new(),
            expand_next: Vec::new(),
            panel: Panel::Explorer,
            sidebar_open: true,
            tabs: Vec::new(),
            active: None,
            next_uid: 1,
            dark: true,
            no_diff: DiffState::new(Vec::new()),
            commit: TextEdit::new(""),
            git_busy: false,
            staged_closed: false,
            changes_closed: false,
            notice: None,
            title: None,
            clock: 0.0,
        }
    }

    /// The folder opened last time, if it is still there.
    pub fn last_workspace(&self) -> Option<PathBuf> {
        self.prefs.get::<String>("workspace").map(PathBuf::from).filter(|p| p.is_dir())
    }

    pub fn set_waker(&mut self, waker: Waker) {
        self.jobs.set_waker(waker);
    }

    // The workspace ------------------------------------------------------------------------

    pub fn open_workspace(&mut self, root: PathBuf) {
        let root = std::path::absolute(&root).unwrap_or(root);
        if !root.is_dir() {
            self.notice = Some(format!("{} is not a folder.", root.display()));
            return;
        }
        self.name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.title = Some(format!("{} - code", self.name));
        let _ = self.prefs.set("workspace", &root.to_string_lossy().into_owned());
        self.root = Some(root);
        self.tree = TreeModel::new().key("files");
        self.decor = Rc::default();
        self.git = None;
        self.truncated = false;
        self.loading = true;
        self.open_ignored.clear();
        self.expand_next.clear();
        self.tabs.clear();
        self.active = None;
        self.commit = TextEdit::new("");
        self.notice = None;
        self.watch = None;
        self.start_watching();
        self.rescan();
        self.drain();
    }

    fn start_watching(&mut self) {
        let Some(root) = self.root.clone() else { return };
        if self.jobs.is_inline() {
            // A headless run is one moment: nothing changes under it.
            return;
        }
        let (queue, waker) = (self.jobs.sender(), self.jobs.waker());
        let filter = Arc::new(Mutex::new(WatchFilter { root: root.clone(), ignored: Vec::new() }));
        let pending = Arc::new(AtomicBool::new(false));
        let (seen, queued) = (filter.clone(), pending.clone());
        let queue = Mutex::new(queue);
        let handler = move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            let relevant = seen.lock().is_ok_and(|f| event.paths.iter().any(|p| f.relevant(p)));
            if relevant && !queued.swap(true, Ordering::SeqCst) {
                if let Ok(q) = queue.lock() {
                    let _ = q.send(Event::FsChanged);
                }
                waker();
            }
        };
        let watcher = notify::recommended_watcher(handler).and_then(|mut w| w.watch(&root, notify::RecursiveMode::Recursive).map(|()| w));
        match watcher {
            Ok(w) => self.watch = Some(Watch { _watcher: w, filter, pending }),
            // The folder still refreshes when the window regains focus, and on demand.
            Err(e) => eprintln!("code: not watching {}: {e}", root.display()),
        }
    }

    /// Lists the folder and reads git's status again, now or as soon as the listing in flight
    /// is back.
    pub fn rescan(&mut self) {
        let Some(root) = self.root.clone() else { return };
        if self.scanning {
            self.rescan_queued = true;
            return;
        }
        self.scanning = true;
        let open = self.open_ignored.clone();
        self.jobs.spawn("scan", move || Event::Scanned(Box::new(scan(root, &open))));
    }

    /// The same, once changes have stopped arriving for a moment.
    pub fn rescan_soon(&mut self) {
        self.rescan_due = Some(self.clock + RESCAN_DELAY);
    }

    fn apply_scan(&mut self, scan: Scan) {
        self.scanning = false;
        if self.root.as_deref() != Some(scan.root.as_path()) {
            // The answer is for a folder that has since been closed.
            self.rescan();
            return;
        }
        self.loading = false;
        self.truncated = scan.listing.truncated;
        self.rebuild_tree(&scan.listing);
        self.decor = Rc::new(scan.git.as_ref().map(decorate).unwrap_or_default());
        if let Some(watch) = &self.watch
            && let Ok(mut f) = watch.filter.lock()
        {
            f.ignored = scan.listing.entries.iter().filter(|e| e.dir && e.ignored).map(|e| scan.root.join(&e.path)).collect();
        }
        self.git = scan.git;
        for tab in &mut self.tabs {
            tab.stale = true;
        }
        self.refresh_active();
        if std::mem::take(&mut self.rescan_queued) {
            self.rescan();
        }
    }

    fn rebuild_tree(&mut self, listing: &Listing) {
        let mut open = Vec::new();
        open_folders(&self.tree, None, &mut open);
        open.append(&mut self.expand_next);
        let cursor = self.tree.cursor().map(str::to_owned);
        let mut tree = TreeModel::new().key("files");
        for e in &listing.entries {
            let (parent, name) = match e.path.rsplit_once('/') {
                Some((parent, name)) => (Some(parent), name),
                None => (None, e.path.as_str()),
            };
            let meta = Meta { ignored: e.ignored, unlisted: e.unlisted };
            tree.insert(parent, e.path.clone(), FileNode { name: name.to_owned(), path: e.path.clone(), dir: e.dir, data: Some(meta) });
        }
        tree.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| natural_cmp(&a.name, &b.name)));
        for id in &open {
            tree.set_expanded(id, true);
        }
        if let Some(id) = cursor.filter(|id| tree.contains(id)) {
            tree.select(&id);
        }
        self.tree = tree;
    }

    /// A row of the explorer was chosen. A file opens (as a preview on a single click); a
    /// folder that was listed closed is listed and opened.
    pub fn pick(&mut self, id: &str, preview: bool) {
        let Some(node) = self.tree.get(id) else { return };
        let Some(root) = &self.root else { return };
        if !node.dir {
            self.open_file(root.join(id), preview);
        } else if node.data.is_some_and(|m| m.unlisted) && self.open_ignored.insert(id.to_owned()) {
            self.expand_next.push(id.to_owned());
            self.rescan();
            self.drain();
        }
    }

    pub fn collapse_all(&mut self) {
        self.tree.collapse_all();
    }

    // Tabs ---------------------------------------------------------------------------------

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.uid) == self.active)
    }

    fn tab_mut(&mut self, uid: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.uid == uid)
    }

    /// The active tab's diff, or an empty one: what the diff view's accessors hand out.
    pub fn diff(&self) -> &DiffState {
        match self.active_tab().map(|t| &t.body) {
            Some(Body::Diff(d)) => d,
            _ => &self.no_diff,
        }
    }

    pub fn diff_mut(&mut self) -> &mut DiffState {
        let active = self.active;
        match self.tabs.iter_mut().find(|t| Some(t.uid) == active).map(|t| &mut t.body) {
            Some(Body::Diff(d)) => d,
            _ => &mut self.no_diff,
        }
    }

    /// The active tab's file as code, when that is what it shows.
    pub fn doc(&self) -> Option<&Doc> {
        match self.active_tab().map(|t| &t.body) {
            Some(Body::Code(doc)) => Some(doc),
            _ => None,
        }
    }

    /// The path to show for `path`: inside the workspace when it is there.
    fn display_path(&self, path: &Path) -> String {
        self.root.as_deref().and_then(|root| workspace::relative(root, path)).unwrap_or_else(|| path.display().to_string())
    }

    pub fn open_file(&mut self, path: PathBuf, preview: bool) {
        let title = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let detail = self.display_path(&path);
        self.open_tab(format!("file:{}", path.display()), title, detail, path, Source::File, preview);
    }

    pub fn open_diff(&mut self, file: &FileStatus, staged: bool, preview: bool) {
        let Some(git) = &self.git else { return };
        let path = git.repo.join(&file.path);
        let name = file.path.rsplit('/').next().unwrap_or(&file.path);
        let (title, sides) = if staged {
            (format!("{name} (Index)"), "HEAD \u{2194} Index")
        } else {
            (format!("{name} (Working Tree)"), "Index \u{2194} Working Tree")
        };
        let detail = format!("{} \u{2022} {sides}", self.display_path(&path));
        let id = format!("diff:{}:{}", if staged { "index" } else { "tree" }, file.path);
        self.open_tab(id, title, detail, path, Source::Diff { file: file.clone(), staged }, preview);
    }

    fn open_tab(&mut self, id: String, title: String, detail: String, path: PathBuf, source: Source, preview: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.preview &= preview;
            self.active = Some(tab.uid);
            self.reveal_active(None);
            self.refresh_active();
            return;
        }
        let uid = self.next_uid;
        self.next_uid += 1;
        let tab = Tab { uid, id, title, detail, path, source, body: Body::Loading, preview, stale: false, load: 0 };
        // A preview takes the place of the last one; anything else opens beside the active tab.
        match self.tabs.iter().position(|t| t.preview).filter(|_| preview) {
            Some(i) => self.tabs[i] = tab,
            None => {
                let at = self.tabs.iter().position(|t| Some(t.uid) == self.active).map_or(self.tabs.len(), |i| i + 1);
                self.tabs.insert(at, tab);
            }
        }
        self.active = Some(uid);
        self.reveal_active(None);
        self.load(uid);
        self.drain();
    }

    pub fn select_tab(&mut self, uid: u64) {
        if self.tabs.iter().any(|t| t.uid == uid) {
            self.active = Some(uid);
            self.refresh_active();
        }
    }

    pub fn keep_tab(&mut self, uid: u64) {
        if let Some(tab) = self.tab_mut(uid) {
            tab.preview = false;
        }
    }

    pub fn close_tab(&mut self, uid: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.uid == uid) else { return };
        self.tabs.remove(i);
        if self.active == Some(uid) {
            self.active = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map(|t| t.uid);
            self.reveal_active(None);
            self.refresh_active();
        }
    }

    pub fn close_active_tab(&mut self) {
        if let Some(uid) = self.active {
            self.close_tab(uid);
        }
    }

    /// Moves to the tab `step` places along, wrapping at the ends.
    pub fn cycle_tab(&mut self, step: isize) {
        let n = self.tabs.len() as isize;
        let Some(i) = self.tabs.iter().position(|t| Some(t.uid) == self.active) else { return };
        let next = (i as isize + step).rem_euclid(n) as usize;
        self.select_tab(self.tabs[next].uid);
        self.reveal_active(None);
    }

    /// Opens the file at `path` in the workspace, or with `None` the first file of the tree,
    /// as a double click on its row would: for stills and tests.
    pub fn open_listed(&mut self, path: Option<&str>) {
        let first =
            || (0..self.tree.visible_len()).filter_map(|i| self.tree.visible_id(i)).find(|id| self.tree.get(id).is_some_and(|n| !n.dir));
        let Some(id) = path.filter(|p| self.tree.contains(p)).or_else(first).map(str::to_owned) else { return };
        self.pick(&id, false);
    }

    /// Shows the active tab's file in the explorer: its folders open and its row selected,
    /// and scrolled to when a handler's `tx` is at hand.
    pub fn reveal_active(&mut self, tx: Option<&mut Tx>) {
        let Some(id) = self.active_tab().and_then(|t| workspace::relative(self.root.as_deref()?, &t.path)) else { return };
        if !self.tree.contains(&id) {
            return;
        }
        match tx {
            Some(tx) => drop(self.tree.reveal(&id, tx)),
            None => {
                self.tree.expand_to(&id);
                self.tree.select(&id);
            }
        }
    }

    /// Opens the diff of the first staged or unstaged change: for stills and tests.
    pub fn open_first_change(&mut self, staged: bool) {
        let first =
            self.git.as_ref().and_then(|g| if staged { g.status.staged().next().cloned() } else { g.status.unstaged().next().cloned() });
        if let Some(file) = first {
            self.open_diff(&file, staged, false);
        }
    }

    /// Loads the active tab again if what it shows has changed since.
    fn refresh_active(&mut self) {
        let Some(uid) = self.active else { return };
        if self.tab_mut(uid).is_some_and(|t| std::mem::take(&mut t.stale)) {
            self.load(uid);
            self.drain();
        }
    }

    fn load(&mut self, uid: u64) {
        let repo = self.git.as_ref().map(|g| g.repo.clone());
        let current = self.git.as_ref().map(|g| g.status.files.clone()).unwrap_or_default();
        let Some(tab) = self.tabs.iter_mut().find(|t| t.uid == uid) else { return };
        tab.load += 1;
        let (load, path) = (tab.load, tab.path.clone());
        match &mut tab.source {
            Source::File => {
                self.jobs.spawn("read", move || {
                    let content = workspace::read(&path);
                    let lang = syntax::lang_for_path(&path.to_string_lossy());
                    let highlights = content.text().map(|text| syntax::builtin(lang, text)).unwrap_or_default();
                    Event::Loaded { uid, load, loaded: Box::new(Loaded::File { content, highlights, lang }) }
                });
            }
            Source::Diff { file, staged } => {
                // The file's change as git reports it now; none means the two sides have met.
                let none = FileStatus { path: file.path.clone(), orig_path: None, staged: None, unstaged: None };
                *file = current.into_iter().find(|f| f.path == file.path).unwrap_or(none);
                let (file, staged) = (file.clone(), *staged);
                let Some(repo) = repo else { return };
                self.jobs.spawn("diff", move || Event::Loaded { uid, load, loaded: Box::new(read_diff(&repo, &file, staged)) });
            }
        }
    }

    fn apply_loaded(&mut self, uid: u64, load: u64, loaded: Loaded) {
        let (split, dark) = (self.split, self.dark);
        let Some(tab) = self.tab_mut(uid).filter(|t| t.load == load) else { return };
        tab.body = match loaded {
            Loaded::File { content: Content::Text(text), highlights, lang } => Body::Code(Doc::new(text, highlights, lang)),
            Loaded::File { content, .. } => Body::Note(describe(&content)),
            Loaded::Diff { old_path, new_path, old, new } => match (side(&old), side(&new)) {
                (Ok(old), Ok(new)) if old == new => Body::Note(match (&old_path, &new_path) {
                    (Some(from), Some(to)) if from != to => format!("Renamed from {from}. Its contents are unchanged."),
                    _ => "No changes: both sides are the same.".into(),
                }),
                (Ok(old), Ok(new)) => {
                    let file = FileDiff::between(old_path, new_path, old, new);
                    match &mut tab.body {
                        // Loading it again keeps the view's place and mode.
                        Body::Diff(state) => {
                            state.set_files(vec![file]);
                            return;
                        }
                        _ => Body::Diff(Box::new(new_diff(file, split, dark))),
                    }
                }
                (Err(why), _) | (_, Err(why)) => Body::Note(why),
            },
        };
    }

    pub fn set_split(&mut self, split: bool) {
        self.split = split;
        let _ = self.prefs.set("diff_split", &split);
        let mode = diff_mode(split);
        for tab in &mut self.tabs {
            if let Body::Diff(d) = &mut tab.body {
                d.set_mode(mode);
            }
        }
    }

    /// Follows the system's appearance: diffs colour their code with the theme's palette.
    fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        for tab in &mut self.tabs {
            if let Body::Diff(d) = &mut tab.body {
                d.set_highlighter(highlighter(dark));
            }
        }
    }

    pub fn set_sidebar_width(&mut self, width: f32, done: bool) {
        self.sidebar_w = width.clamp(160.0, 640.0).round();
        if done {
            let _ = self.prefs.set("sidebar_width", &self.sidebar_w);
        }
    }

    /// Shows `panel`, or hides the side bar when it already shows it.
    pub fn toggle_panel(&mut self, panel: Panel) {
        if self.panel == panel && self.sidebar_open {
            self.sidebar_open = false;
        } else {
            self.panel = panel;
            self.sidebar_open = true;
        }
    }

    // Git ----------------------------------------------------------------------------------

    fn git_run(&mut self, what: &'static str, work: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else { return };
        if self.git_busy {
            return;
        }
        self.git_busy = true;
        self.jobs.spawn(what, move || Event::GitDone { what, result: work(&repo) });
        self.drain();
    }

    pub fn stage(&mut self, paths: Vec<String>) {
        self.git_run("stage", move |repo| git::stage(repo, &paths));
    }

    pub fn unstage(&mut self, paths: Vec<String>) {
        self.git_run("unstage", move |repo| git::unstage(repo, &paths));
    }

    pub fn stage_all(&mut self) {
        self.git_run("stage", git::stage_all);
    }

    pub fn unstage_all(&mut self) {
        let paths: Vec<String> = self.git.iter().flat_map(|g| g.status.staged()).map(|f| f.path.clone()).collect();
        self.unstage(paths);
    }

    pub fn can_commit(&self) -> bool {
        !self.git_busy && !self.commit.text.trim().is_empty() && self.git.as_ref().is_some_and(|g| g.status.staged().next().is_some())
    }

    pub fn commit(&mut self) {
        if !self.can_commit() {
            return;
        }
        let message = self.commit.text.trim().to_owned();
        self.git_run("commit", move |repo| git::commit(repo, &message));
    }

    pub fn init_repository(&mut self) {
        let Some(root) = self.root.clone() else { return };
        if self.git_busy {
            return;
        }
        self.git_busy = true;
        self.jobs.spawn("init", move || Event::GitDone { what: "init", result: git::init(&root) });
        self.drain();
    }

    // Events -------------------------------------------------------------------------------

    fn handle(&mut self, event: Event) {
        match event {
            Event::Scanned(scan) => self.apply_scan(*scan),
            Event::Loaded { uid, load, loaded } => self.apply_loaded(uid, load, *loaded),
            Event::GitDone { what, result } => {
                self.git_busy = false;
                match result {
                    Ok(()) if what == "commit" => self.commit.replace_text(""),
                    Ok(()) => {}
                    Err(e) => self.notice = Some(format!("git {what} failed: {e}")),
                }
                self.rescan();
            }
            Event::FsChanged => {
                if let Some(watch) = &self.watch {
                    watch.pending.store(false, Ordering::SeqCst);
                }
                self.rescan_soon();
            }
        }
    }

    /// Applies the answers that are in. With jobs run inline that is all of them, so the state
    /// is settled when the call that started them returns.
    pub fn drain(&mut self) -> bool {
        let mut any = false;
        while let Some(event) = self.jobs.try_recv() {
            any = true;
            self.handle(event);
        }
        any
    }

    /// Before each frame: applies answers from the workers and starts a listing that is due.
    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        self.clock = clock;
        let mut changed = self.drain();
        let dark = crate::theme::system_dark();
        if dark != self.dark {
            self.set_dark(dark);
            changed = true;
        }
        if let Some(due) = self.rescan_due {
            if clock >= due {
                self.rescan_due = None;
                self.rescan();
                changed |= self.drain();
            } else {
                tx.wake_after(std::time::Duration::from_secs_f64(due - clock));
            }
        }
        if let Some(title) = self.title.take() {
            tx.set_title(title);
        }
        changed
    }
}

fn diff_mode(split: bool) -> DiffMode {
    if split { DiffMode::Split } else { DiffMode::Unified }
}

fn new_diff(file: FileDiff, split: bool, dark: bool) -> DiffState {
    let mut state = DiffState::new(vec![file]);
    state.set_mode(diff_mode(split));
    state.set_highlighter(highlighter(dark));
    state
}

/// The built-in highlighter in the editor's own colours, so a diff's code matches a file's.
fn highlighter(dark: bool) -> Highlighter {
    Highlighter::new(move |lang, code| {
        let theme = crate::theme::theme(dark);
        syntax::builtin(lang, code).into_iter().map(|(range, paint)| (range, Paint::Color(theme.syntax.paint(paint)))).collect()
    })
    .key(dark)
}

/// The ids of the open folders of `tree` under `parent`, parents first.
fn open_folders(tree: &TreeModel<Node>, parent: Option<&str>, out: &mut Vec<String>) {
    for id in tree.children(parent) {
        if tree.has_children(id) {
            if tree.is_expanded(id) {
                out.push(id.to_owned());
            }
            open_folders(tree, Some(id), out);
        }
    }
}

fn scan(root: PathBuf, open: &HashSet<String>) -> Scan {
    let git = git::discover(&root).and_then(|repo| {
        let status = git::status(&repo).ok()?;
        let (top, here) = (std::fs::canonicalize(&repo).ok()?, std::fs::canonicalize(&root).ok()?);
        let inside = workspace::relative(&top, &here)?;
        let prefix = if inside.is_empty() { inside } else { format!("{inside}/") };
        Some(GitState { repo, prefix, status })
    });
    let ignored = git.as_ref().map(|g| Ignored::new(g.status.ignored.iter().map(String::as_str), &g.prefix)).unwrap_or_default();
    let listing = workspace::list(&root, &ignored, open);
    Scan { root, listing, git }
}

/// Git's marks for the explorer: each changed file in the workspace, and the folders above it.
fn decorate(git: &GitState) -> Decor {
    let mut decor = Decor::default();
    for file in &git.status.files {
        let Some(path) = file.path.strip_prefix(&git.prefix) else { continue };
        let Some(change) = file.unstaged.or(file.staged) else { continue };
        decor.files.insert(path.to_owned(), change);
        let mut dir = path;
        while let Some((parent, _)) = dir.rsplit_once('/') {
            if !decor.dirs.insert(parent.to_owned()) {
                break;
            }
            dir = parent;
        }
    }
    decor
}

/// Both sides of a change. A side that doesn't exist (an added file's old side, a deleted
/// file's new one) is [`Content::Missing`].
fn read_diff(repo: &Path, file: &FileStatus, staged: bool) -> Loaded {
    let stored = |rev: Rev, path: &str| git::show(repo, rev, path).map_or(Content::Missing, Content::from_bytes);
    let from = file.orig_path.as_deref().unwrap_or(&file.path);
    let (old, new) = if staged {
        let old = if file.staged == Some(Change::Added) { Content::Missing } else { stored(Rev::Head, from) };
        let new = if file.staged == Some(Change::Deleted) { Content::Missing } else { stored(Rev::Index, &file.path) };
        (old, new)
    } else {
        let old = match file.unstaged {
            Some(Change::Untracked) => Content::Missing,
            Some(Change::Conflicted) => stored(Rev::Head, &file.path),
            _ => stored(Rev::Index, &file.path),
        };
        let new = if file.unstaged == Some(Change::Deleted) { Content::Missing } else { workspace::read(&repo.join(&file.path)) };
        (old, new)
    };
    let old_path = (old != Content::Missing).then(|| from.to_owned());
    let new_path = (new != Content::Missing).then(|| file.path.clone());
    Loaded::Diff { old_path, new_path, old: unix_lines(old), new: unix_lines(new) }
}

/// With `core.autocrlf` the index holds LF where the working tree has CR LF; compared as they
/// are, every line would differ.
fn unix_lines(content: Content) -> Content {
    match content {
        Content::Text(text) if text.contains('\r') => Content::Text(text.replace("\r\n", "\n")),
        other => other,
    }
}

/// One side of a diff as text, or why it can't be compared.
fn side(content: &Content) -> Result<&str, String> {
    match content {
        Content::Text(text) => Ok(text),
        Content::Missing => Ok(""),
        other => Err(describe(other)),
    }
}

fn describe(content: &Content) -> String {
    match content {
        Content::Text(_) => String::new(),
        Content::Binary => "The file is not displayed because it is binary.".into(),
        Content::TooLarge(bytes) => format!("The file is not displayed because it is too large ({:.1} MB).", *bytes as f64 / 1_048_576.0),
        Content::Missing => "The file no longer exists.".into(),
        Content::Error(e) => format!("The file could not be read: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_marks_its_file_and_every_folder_above_it() {
        let records =
            ["1 .M N... 100644 100644 100644 a b app/src/ui/view.rs", "? app/notes.md", "1 M. N... 100644 100644 100644 a b other/x.rs"];
        let status = git::parse_status(records.map(|r| format!("{r}\0")).concat().as_bytes());
        let decor = decorate(&GitState { repo: PathBuf::from("repo"), prefix: "app/".into(), status });
        assert_eq!(decor.files.get("src/ui/view.rs"), Some(&Change::Modified));
        assert_eq!(decor.files.get("notes.md"), Some(&Change::Untracked));
        assert_eq!(decor.files.len(), 2, "a change outside the workspace marks nothing in it");
        let mut dirs: Vec<&str> = decor.dirs.iter().map(String::as_str).collect();
        dirs.sort_unstable();
        assert_eq!(dirs, ["src", "src/ui"]);
    }

    #[test]
    fn only_git_files_that_move_a_branch_or_the_index_count_as_changes() {
        let root = PathBuf::from("ws");
        let f = WatchFilter { root: root.clone(), ignored: vec![root.join("target")] };
        assert!(f.relevant(&root.join("src").join("main.rs")));
        assert!(!f.relevant(&root.join("target").join("debug").join("code.exe")));
        assert!(f.relevant(&root.join(".git").join("index")));
        assert!(f.relevant(&root.join(".git").join("HEAD")));
        assert!(f.relevant(&root.join(".git").join("refs").join("heads").join("main")));
        assert!(!f.relevant(&root.join(".git").join("index.lock")));
        assert!(!f.relevant(&root.join(".git").join("objects").join("ab").join("cdef")));
    }

    #[test]
    fn line_endings_do_not_count_as_changes() {
        assert_eq!(unix_lines(Content::Text("a\r\nb\r\n".into())), Content::Text("a\nb\n".into()));
        assert_eq!(unix_lines(Content::Binary), Content::Binary);
    }
}
