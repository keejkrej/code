//! The window driven headlessly: real folders and repositories, real clicks on what is drawn.

use rune::runtime::ElementInfo;
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::view;
use crate::app::{App, Body, Panel, Source};
use crate::git::Change;
use crate::testing::TempDir;

type Rt = Runtime<App>;

/// The window on `dir`, with every job run inline so each click's effect is in by the next
/// frame.
fn open(dir: &TempDir) -> Rt {
    let mut app = App::new(true);
    app.open_workspace(dir.0.clone());
    let mut rt = Runtime::new(app, view, (1100.0, 700.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn all(rt: &Rt) -> Vec<ElementInfo> {
    rt.elements().into_iter().filter(|e| !e.exiting).collect()
}

fn centre(e: &ElementInfo) -> (f32, f32) {
    (e.rect[0] + e.rect[2] * 0.5, e.rect[1] + e.rect[3] * 0.5)
}

fn click_at(rt: &mut Rt, (x, y): (f32, f32)) {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    // Past the double-click interval, so the next click starts a new one.
    rt.frame(1.0);
}

/// Clicks the first element `pick` accepts.
fn click(rt: &mut Rt, what: &str, pick: impl Fn(&ElementInfo) -> bool) {
    let at = all(rt).iter().find(|e| pick(e)).map(centre).unwrap_or_else(|| panic!("nothing drawn for {what}"));
    click_at(rt, at);
}

fn click_key(rt: &mut Rt, key: &str) {
    click(rt, key, |e| e.key.as_deref() == Some(key));
}

/// Clicks the explorer's row for `name`.
fn click_row(rt: &mut Rt, name: &str) {
    click(rt, name, |e| e.role == "treeitem" && e.label.as_deref() == Some(name));
}

fn tree_rows(rt: &Rt) -> Vec<String> {
    all(rt).into_iter().filter(|e| e.role == "treeitem").filter_map(|e| e.label).collect()
}

/// The lines of code on screen, as the accessibility tree reads them.
fn code_lines(rt: &Rt) -> Vec<String> {
    all(rt).into_iter().filter(|e| e.role == "listitem").filter_map(|e| e.label).filter(|l| l.starts_with("line ")).collect()
}

fn tab_titles(rt: &Rt) -> Vec<String> {
    rt.state.tabs.iter().map(|t| t.title.clone()).collect()
}

fn has_text(rt: &Rt, text: &str) -> bool {
    all(rt).iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains(text)))
}

#[test]
fn with_no_folder_the_window_offers_to_open_one_and_opens_the_one_picked() {
    let dir = TempDir::new("pick");
    dir.write("hello.txt", "hi\n");
    let mut rt = Runtime::new(App::new(true), view, (1100.0, 700.0), Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(has_text(&rt, "No folder is open yet."));
    assert!(tree_rows(&rt).is_empty());

    click_key(&mut rt, "open");
    assert!(rt.file_dialog().is_some(), "the button asks for a folder");
    rt.resolve_file_dialog(Some(dir.0.clone())).expect("a dialog is open");
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.root.as_deref(), Some(dir.0.as_path()));
    assert_eq!(tree_rows(&rt), ["hello.txt"]);
}

#[test]
fn the_explorer_lists_folders_first_and_a_click_opens_a_folder_then_a_file() {
    let dir = TempDir::repo("explorer");
    let mut rt = open(&dir);
    assert_eq!(tree_rows(&rt), ["src", "target", "web", ".env", ".gitignore", "notes.md", "README.md"]);

    click_row(&mut rt, "src");
    assert_eq!(tree_rows(&rt)[..3], ["src", "net", "main.rs"]);
    assert!(rt.state.tabs.is_empty(), "opening a folder opens no editor");

    click_row(&mut rt, "main.rs");
    assert_eq!(tab_titles(&rt), ["main.rs"]);
    assert_eq!(code_lines(&rt), ["line 1: fn main() {", "line 2:     let wait = 2;", "line 3:     println!(\"{wait}\");", "line 4: }"]);
    assert!(has_text(&rt, "Rust"), "the status bar names the language");
    assert!(has_text(&rt, "4 lines"));
}

#[test]
fn a_single_click_previews_and_the_next_one_takes_its_tab() {
    let dir = TempDir::repo("preview");
    let mut rt = open(&dir);
    click_row(&mut rt, "README.md");
    click_row(&mut rt, "notes.md");
    assert_eq!(tab_titles(&rt), ["notes.md"], "a preview is replaced by the next");

    // A double click keeps the tab, so the next preview opens beside it.
    let row = all(&rt).into_iter().find(|e| e.role == "treeitem" && e.label.as_deref() == Some("notes.md")).expect("row");
    let (x, y) = centre(&row);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        rt.frame(0.1);
    }
    rt.frame(1.0);
    click_row(&mut rt, "README.md");
    assert_eq!(tab_titles(&rt), ["notes.md", "README.md"]);

    let uid = rt.state.tabs[1].uid;
    click(&mut rt, "the tab's close button", |e| e.key.as_deref() == Some("close") && e.path.contains(&format!("tab-{uid}")));
    assert_eq!(tab_titles(&rt), ["notes.md"]);
    assert_eq!(rt.state.active, Some(rt.state.tabs[0].uid), "closing the active tab selects its neighbour");
}

#[test]
fn git_marks_changed_files_and_ignored_folders_open_on_demand() {
    let dir = TempDir::repo("marks");
    let mut rt = open(&dir);
    let decor = rt.state.decor.clone();
    assert_eq!(decor.files.get("src/main.rs"), Some(&Change::Modified));
    assert_eq!(decor.files.get("notes.md"), Some(&Change::Untracked));
    assert!(decor.dirs.contains("src") && decor.dirs.contains("web"));

    // `target` is ignored: listed, but its contents only once it is opened.
    assert!(rt.state.tree.get("target").is_some_and(|n| n.data.is_some_and(|m| m.ignored && m.unlisted)));
    assert!(!rt.state.tree.contains("target/debug"));
    click_row(&mut rt, "target");
    assert!(rt.state.tree.contains("target/debug"));
    assert!(rt.state.tree.is_expanded("target"));
    assert!(tree_rows(&rt).contains(&"debug".to_owned()));
}

#[test]
fn source_control_lists_changes_and_a_click_shows_the_diff() {
    let dir = TempDir::repo("scm");
    let mut rt = open(&dir);
    click_key(&mut rt, "view-scm");
    assert_eq!(rt.state.panel, Panel::Scm);
    let keys: Vec<String> = all(&rt).into_iter().filter(|e| e.role == "listitem").filter_map(|e| e.key).collect();
    assert_eq!(keys, ["staged:web/fetch.ts", "unstaged:src/main.rs", "unstaged:notes.md"]);

    click_key(&mut rt, "unstaged:src/main.rs");
    assert_eq!(tab_titles(&rt), ["main.rs (Working Tree)"]);
    let Some(Body::Diff(diff)) = rt.state.active_tab().map(|t| &t.body) else { panic!("the tab shows a diff") };
    let file = &diff.files()[0];
    assert_eq!((file.path(), file.additions(), file.deletions()), ("src/main.rs", 1, 1));
    let rows: Vec<String> = all(&rt).into_iter().filter_map(|e| e.label).filter(|l| l.contains("line 2:")).collect();
    assert_eq!(rows, ["removed line 2:     let wait = 1;", "added line 2:     let wait = 2;"]);

    // The staged side compares the index with HEAD.
    click_key(&mut rt, "staged:web/fetch.ts");
    assert!(matches!(rt.state.active_tab().map(|t| &t.source), Some(Source::Diff { staged: true, .. })));
    let Some(Body::Diff(diff)) = rt.state.active_tab().map(|t| &t.body) else { panic!("the tab shows a diff") };
    assert_eq!(diff.files()[0].text(rune_kit::diff::Side::Old), Some("export const tries = 3\n"));
    assert_eq!(diff.files()[0].text(rune_kit::diff::Side::New), Some("export const tries = 5\n"));
}

#[test]
fn an_untracked_file_diffs_as_all_added() {
    let dir = TempDir::repo("untracked");
    let mut rt = open(&dir);
    click_key(&mut rt, "view-scm");
    click_key(&mut rt, "unstaged:notes.md");
    let Some(Body::Diff(diff)) = rt.state.active_tab().map(|t| &t.body) else { panic!("the tab shows a diff") };
    assert_eq!((diff.files()[0].additions(), diff.files()[0].deletions()), (1, 0));
    assert_eq!(diff.files()[0].old_path, None);
}

#[test]
fn staging_unstaging_and_committing_run_git() {
    let dir = TempDir::repo("commit");
    let mut rt = open(&dir);
    click_key(&mut rt, "view-scm");
    let button = |rt: &Rt, row: &str, key: &str| {
        let found = all(rt).into_iter().find(|e| e.key.as_deref() == Some(key) && e.path.contains(row));
        centre(&found.unwrap_or_else(|| panic!("no {key} button on {row}")))
    };

    let at = button(&rt, "unstaged:src/main.rs", "stage");
    click_at(&mut rt, at);
    assert_eq!(dir.git(&["diff", "--cached", "--name-only"]), "src/main.rs\nweb/fetch.ts\n");
    assert!(rt.state.tabs.is_empty(), "a row's button doesn't open the row");

    let at = button(&rt, "staged:web/fetch.ts", "unstage");
    click_at(&mut rt, at);
    assert_eq!(dir.git(&["diff", "--cached", "--name-only"]), "src/main.rs\n");

    // Nothing commits without a message.
    assert!(!rt.state.can_commit());
    rt.update(|s, _| s.commit.replace_text("Wait longer"));
    rt.frame(0.0);
    click_key(&mut rt, "commit");
    assert_eq!(dir.git(&["log", "-1", "--format=%s"]), "Wait longer\n");
    assert_eq!(rt.state.commit.text, "", "the message is cleared for the next commit");
    let status = &rt.state.git.as_ref().expect("a repository").status;
    assert_eq!(status.staged().count(), 0);
    assert_eq!(status.unstaged().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["web/fetch.ts", "notes.md"]);
    assert_eq!(rt.state.notice, None);
}

#[test]
fn a_folder_without_a_repository_offers_to_make_one() {
    let dir = TempDir::new("init");
    dir.write("a.txt", "a\n");
    let mut rt = open(&dir);
    click_key(&mut rt, "view-scm");
    assert!(has_text(&rt, "not a Git repository yet"));
    click_key(&mut rt, "init");
    assert!(dir.0.join(".git").is_dir());
    let status = &rt.state.git.as_ref().expect("the new repository is read").status;
    assert_eq!(status.unstaged().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}

#[test]
fn a_binary_file_says_why_it_is_not_shown() {
    let dir = TempDir::new("binary");
    std::fs::write(dir.0.join("image.png"), [0x89, b'P', b'N', b'G', 0, 0, 0, 13]).expect("write");
    let mut rt = open(&dir);
    click_row(&mut rt, "image.png");
    assert!(has_text(&rt, "not displayed because it is binary"));
    assert!(code_lines(&rt).is_empty());
}

#[test]
fn a_file_changed_on_disk_is_read_again_on_the_next_listing() {
    let dir = TempDir::repo("reload");
    let mut rt = open(&dir);
    click_row(&mut rt, "README.md");
    assert_eq!(code_lines(&rt), ["line 1: # fixture"]);
    dir.write("README.md", "# fixture\n\nChanged outside the editor.\n");
    dir.write("added.txt", "new\n");
    // What the file watcher's event leads to: a listing once changes have settled.
    rt.update(|s, tx| {
        s.rescan_soon();
        assert!(!s.poll(0.1, tx), "nothing yet: more changes may follow");
        assert!(s.poll(1.0, tx));
    });
    rt.frame(0.0);
    assert_eq!(code_lines(&rt), ["line 1: # fixture", "line 2: ", "line 3: Changed outside the editor."]);
    assert!(tree_rows(&rt).contains(&"added.txt".to_owned()));
    assert_eq!(rt.state.decor.files.get("README.md"), Some(&Change::Modified));
}

/// The primary modifier with `c`, as a window reports it: Ctrl off macOS.
fn command(c: char, shift: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods { cmd: true, ctrl: !cfg!(target_os = "macos"), logo: cfg!(target_os = "macos"), shift, ..Mods::default() },
    }
}

#[test]
fn shortcuts_switch_views_hide_the_side_bar_and_close_the_editor() {
    let dir = TempDir::repo("keys");
    let mut rt = open(&dir);
    click_row(&mut rt, "README.md");
    let press = |rt: &mut Rt, ev: KeyEvent| {
        let mut taken = false;
        rt.update(|s, tx| taken = super::on_key(s, &ev, tx));
        rt.frame(0.0);
        taken
    };
    assert!(press(&mut rt, command('g', true)));
    assert_eq!((rt.state.panel, rt.state.sidebar_open), (Panel::Scm, true));
    assert!(press(&mut rt, command('b', false)));
    assert!(!rt.state.sidebar_open);
    assert!(!all(&rt).iter().any(|e| e.key.as_deref() == Some("sidebar")));
    assert!(press(&mut rt, command('e', true)));
    assert_eq!((rt.state.panel, rt.state.sidebar_open), (Panel::Explorer, true));
    assert!(press(&mut rt, command('w', false)));
    assert!(rt.state.tabs.is_empty());
    assert!(!press(&mut rt, command('q', false)), "other keys go on");
}

#[test]
fn off_the_window_thread_a_folder_is_listed_and_changes_to_it_are_noticed() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    let dir = TempDir::repo("threads");
    let mut app = App::with_prefs(false, rune::prefs::Prefs::memory());
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = wakes.clone();
    app.set_waker(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    app.open_workspace(dir.0.clone());
    assert!(app.loading && app.tree.is_empty(), "the window doesn't wait for the listing");

    let mut rt = Runtime::new(app, view, (1100.0, 700.0), Env::new(true, 1.0));
    let start = Instant::now();
    // The app's `poll`, as the window runs it each frame, until `done` or ten seconds pass.
    let wait = |rt: &mut Rt, what: &str, done: &dyn Fn(&App) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&rt.state) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
            let clock = start.elapsed().as_secs_f64();
            rt.update(|s, tx| {
                s.poll(clock, tx);
            });
        }
        rt.frame(0.0);
    };
    wait(&mut rt, "the first listing", &|s| !s.loading);
    assert!(wakes.load(Ordering::SeqCst) > 0, "the worker asked for a frame");
    assert_eq!(tree_rows(&rt)[..3], ["src", "target", "web"]);
    assert_eq!(rt.state.git.as_ref().map(|g| g.status.head_label()).as_deref(), Some("main"));

    click_row(&mut rt, "README.md");
    wait(&mut rt, "the file", &|s| s.doc().is_some());
    assert_eq!(code_lines(&rt), ["line 1: # fixture"]);

    // A file made by another program shows up by itself, and a commit made outside is seen.
    dir.write(
        "made-outside.txt",
        "hello
",
    );
    wait(&mut rt, "the watcher to notice a new file", &|s| s.tree.contains("made-outside.txt"));
    dir.git(&["add", "-A"]);
    dir.git(&["commit", "-q", "-m", "Outside"]);
    wait(&mut rt, "the watcher to notice a commit", &|s| s.git.as_ref().is_some_and(|g| g.status.files.is_empty()));
}

#[test]
fn a_staged_rename_without_edits_says_so_instead_of_showing_an_empty_diff() {
    let dir = TempDir::repo("rename");
    dir.git(&["mv", "README.md", "GUIDE.md"]);
    let mut rt = open(&dir);
    click_key(&mut rt, "view-scm");
    click_key(&mut rt, "staged:GUIDE.md");
    assert!(has_text(&rt, "Renamed from README.md. Its contents are unchanged."));
}

#[test]
fn the_side_bar_can_be_hidden_and_brought_back_from_the_status_bar() {
    let dir = TempDir::repo("toggle");
    let mut rt = open(&dir);
    let shown = |rt: &Rt| all(rt).iter().any(|e| e.key.as_deref() == Some("sidebar"));
    assert!(shown(&rt));
    click_key(&mut rt, "toggle-sidebar");
    assert!(!shown(&rt));
    click_key(&mut rt, "toggle-sidebar");
    assert!(shown(&rt));
}

#[test]
fn the_folder_opened_last_is_remembered_for_the_next_start() {
    let dir = TempDir::repo("remember");
    let settings = TempDir::new("settings");
    let file = settings.0.join("prefs.json");
    let mut app = App::with_prefs(true, rune::prefs::Prefs::at(&file));
    assert_eq!(app.last_workspace(), None);
    app.open_workspace(dir.0.clone());
    app.set_split(false);

    let next = App::with_prefs(true, rune::prefs::Prefs::at(&file));
    assert_eq!(next.last_workspace().as_deref(), Some(dir.0.as_path()));
    assert!(!next.split, "and so is the diff layout");
    drop(dir);
    assert_eq!(next.last_workspace(), None, "a folder that is gone is not opened again");
}

#[test]
fn a_scan_that_finds_nothing_new_leaves_the_window_at_rest() {
    let dir = TempDir::repo("rest");
    let mut rt = open(&dir);
    click_row(&mut rt, "README.md");
    click_row(&mut rt, "src");
    let text_at = |rt: &Rt| rt.state.doc().map(|d| d.text.as_ptr());
    let before = text_at(&rt);

    // Nothing changed: no frame is asked for, and the open file is not read again.
    rt.update(|s, tx| {
        s.rescan_soon();
        assert!(!s.poll(1.0, tx), "a scan that changes nothing draws nothing");
    });
    assert_eq!(text_at(&rt), before);
    assert!(rt.state.tree.is_expanded("src"));

    // Another file changing redraws (its mark in the explorer) but leaves the open one be.
    dir.write("notes.md", "- more\n- notes\n");
    dir.write("src/new.rs", "pub fn new() {}\n");
    rt.update(|s, tx| {
        s.rescan_soon();
        assert!(s.poll(2.0, tx));
    });
    rt.frame(0.0);
    assert_eq!(text_at(&rt), before);
    assert!(rt.state.tree.is_expanded("src"), "the folders that were open stay open");
    assert!(tree_rows(&rt).contains(&"new.rs".to_owned()));
    assert_eq!(rt.state.tree.cursor(), Some("src"), "and the selected row stays selected");

    // The open file changing is read again.
    dir.write("README.md", "# fixture, edited\n");
    rt.update(|s, tx| {
        s.rescan_soon();
        assert!(s.poll(3.0, tx));
    });
    rt.frame(0.0);
    assert_eq!(code_lines(&rt), ["line 1: # fixture, edited"]);
}

#[test]
fn changes_that_keep_arriving_cannot_put_a_listing_off_for_long() {
    let dir = TempDir::repo("steady");
    let mut rt = open(&dir);
    dir.write("late.txt", "here\n");
    let mut listed_at = None;
    // A change every fifth of a second, each one pushing the listing a little further off.
    for step in 0..20 {
        let clock = 10.0 + step as f64 * 0.2;
        rt.update(|s, tx| {
            s.poll(clock, tx);
            s.rescan_soon();
        });
        if listed_at.is_none() && rt.state.tree.contains("late.txt") {
            listed_at = Some(clock - 10.0);
        }
    }
    let waited = listed_at.expect("the folder is listed while changes still arrive");
    assert!((1.9..=2.3).contains(&waited), "listed after {waited} s");
}
