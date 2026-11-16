//! Timings on a large workspace and a large file. Not run by `cargo test`: they take a while
//! and print rather than assert.
//!
//! ```sh
//! cargo test --release perf -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rune::{Env, Runtime};

use crate::app::{self, App, Panel};
use crate::testing::TempDir;
use crate::ui::view;
use crate::workspace::{self, Ignored};
use crate::{doc, git};

type Rt = Runtime<App>;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let out = f();
    (out, ms(start.elapsed()))
}

/// The median and the worst of `runs`, in milliseconds.
fn spread(mut runs: Vec<f64>) -> (f64, f64) {
    runs.sort_by(f64::total_cmp);
    (runs[runs.len() / 2], runs[runs.len() - 1])
}

fn sample(n: usize, mut f: impl FnMut()) -> (f64, f64) {
    spread((0..n).map(|_| time(&mut f).1).collect())
}

fn report(what: &str, (median, worst): (f64, f64)) {
    println!("{what:<58} median {median:>8.2} ms   worst {worst:>8.2} ms");
}

fn window(app: App) -> Rt {
    let mut rt = Runtime::new(app, view, (1280.0, 800.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

/// A frame after the state changed, as after any click: the view runs again and is laid out.
fn rebuild(rt: &mut Rt) {
    rt.update(|_, _| {});
    rt.frame(1.0 / 60.0);
}

/// A rescan's two halves, each timed `n` times: the worker's scan, and what the window's
/// thread does to put its answer in place.
fn rescans(rt: &mut Rt, n: usize) -> ((f64, f64), (f64, f64)) {
    let Some(root) = rt.state.root.clone() else { return ((0.0, 0.0), (0.0, 0.0)) };
    let (mut scans, mut applies) = (Vec::new(), Vec::new());
    for _ in 0..n {
        let known = rt.state.known();
        let (scan, scanned) = time(|| app::scan(root.clone(), &HashSet::new(), &known));
        let mut applied = 0.0;
        rt.update(|s, _| applied = time(|| s.apply_scan(scan)).1);
        scans.push(scanned);
        applies.push(applied);
    }
    (spread(scans), spread(applies))
}

/// `dirs` folders of `files` source files each, committed, then every tenth file changed.
fn big_repo(name: &str, dirs: usize, files: usize) -> TempDir {
    let t = TempDir::new(name);
    for d in 0..dirs {
        let dir = t.0.join(format!("pkg{d:03}")).join("src");
        std::fs::create_dir_all(&dir).expect("dir");
        for f in 0..files {
            std::fs::write(dir.join(format!("module{f}.rs")), format!("pub fn f{f}() -> u32 {{\n    {f}\n}}\n")).expect("file");
        }
    }
    t.git(&["init", "-q", "-b", "main"]);
    t.git(&["config", "user.name", "Perf"]);
    t.git(&["config", "user.email", "perf@example.com"]);
    t.git(&["config", "commit.gpgsign", "false"]);
    t.git(&["add", "-A"]);
    t.git(&["commit", "-q", "-m", "All of it"]);
    for d in 0..dirs {
        for f in (0..files).step_by(10) {
            let path = t.0.join(format!("pkg{d:03}")).join("src").join(format!("module{f}.rs"));
            std::fs::write(path, format!("pub fn f{f}() -> u32 {{\n    {f} + 1\n}}\n")).expect("file");
        }
    }
    t
}

#[test]
#[ignore = "a benchmark: run it by name with --ignored --nocapture"]
fn perf_large_workspace() {
    let (dirs, files) = (400, 50);
    let (t, made) = time(|| big_repo("perf-ws", dirs, files));
    println!("\n{} files in {dirs} folders, {} changed (made in {:.0} ms)", dirs * files, dirs * files / 10, made);

    report("git status", sample(5, || drop(git::status(&t.0))));
    report("list the folder", sample(5, || drop(workspace::list(&t.0, &Ignored::default(), &HashSet::new()))));

    let mut app = App::new(true);
    let (_, open) = time(|| app.open_workspace(t.0.clone()));
    println!("{:<58} {open:>15.2} ms", "open the folder, all of it");
    let mut rt = window(app);

    let (scan, apply) = rescans(&mut rt, 5);
    report("rescan, nothing changed: worker", scan);
    report("rescan, nothing changed: window thread", apply);
    t.write("pkg000/src/added.rs", "pub fn added() {}\n");
    let (scan, apply) = rescans(&mut rt, 1);
    report("rescan, one file added: worker", scan);
    report("rescan, one file added: window thread", apply);
    t.write("pkg000/src/module1.rs", "pub fn f1() -> u32 {\n    11\n}\n");
    let (scan, apply) = rescans(&mut rt, 1);
    report("rescan, one more file modified: worker", scan);
    report("rescan, one more file modified: window thread", apply);

    report("frame: view rebuilt, folders closed", sample(20, || rebuild(&mut rt)));
    rt.update(|s, _| s.tree.expand_all());
    rt.frame(0.0);
    println!("tree rows: {}", rt.state.tree.visible_len());
    report("frame: view rebuilt, every folder open", sample(20, || rebuild(&mut rt)));
    t.write("pkg001/src/added.rs", "pub fn added() {}\n");
    let (_, apply) = rescans(&mut rt, 1);
    report("rescan, one file added, every folder open: window thread", apply);
    report(
        "frame: tree scrolled by a page",
        sample(30, || {
            rt.wheel(100.0, 400.0, -600.0);
            rt.frame(1.0 / 60.0);
        }),
    );

    rt.update(|s, _| s.panel = Panel::Scm);
    rt.frame(0.0);
    let changes = rt.state.git.as_ref().map_or(0, |g| g.status.files.len());
    report(&format!("frame: view rebuilt, source control with {changes} changes"), sample(20, || rebuild(&mut rt)));
    report(
        "frame: source control scrolled by a page",
        sample(30, || {
            rt.wheel(100.0, 400.0, -600.0);
            rt.frame(1.0 / 60.0);
        }),
    );
}

#[test]
#[ignore = "a benchmark: run it by name with --ignored --nocapture"]
fn perf_large_file() {
    let t = TempDir::new("perf-file");
    let lines = 50_000;
    let mut text = String::new();
    for i in 0..lines {
        text.push_str(&format!("    let value_{i}: u32 = compute({i}, \"label {i}\") + offset; // step {i}\n"));
    }
    t.write("big.rs", &text);
    t.write("small.rs", "fn main() {}\n");
    println!("\n{lines} lines, {:.1} MB", text.len() as f64 / 1_048_576.0);

    report("read the file", sample(5, || drop(workspace::read(&t.0.join("big.rs")))));
    report("highlight it", sample(3, || drop(rune_kit::syntax::builtin(Some("rust"), &text))));
    let highlights = rune_kit::syntax::builtin(Some("rust"), &text);
    println!("coloured ranges: {}", highlights.len());
    report("split it into lines (Doc::new)", sample(5, || drop(doc::Doc::new(text.clone(), highlights.clone(), Some("rust")))));

    let mut app = App::new(true);
    app.open_workspace(t.0.clone());
    let (_, open) = time(|| app.open_listed(Some("big.rs")));
    println!("{:<58} {open:>15.2} ms", "open it in a tab (read + highlight + split)");
    let mut rt = window(app);
    report("frame: view rebuilt with the file open", sample(30, || rebuild(&mut rt)));
    report(
        "frame: code scrolled by a page",
        sample(60, || {
            rt.wheel(700.0, 400.0, -600.0);
            rt.frame(1.0 / 60.0);
        }),
    );
    let (_, apply) = rescans(&mut rt, 5);
    report("rescan with the file open and unchanged: window thread", apply);

    // How long the window keeps drawing after a click: frames at 120 a second until it rests.
    let mut frames = Vec::new();
    for name in ["small.rs", "big.rs", "small.rs", "big.rs"] {
        let row = rt.elements().into_iter().find(|e| e.role == "treeitem" && e.label.as_deref() == Some(name)).expect("row");
        let (x, y) = (row.rect[0] + row.rect[2] * 0.5, row.rect[1] + row.rect[3] * 0.5);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        let mut n = 0;
        while rt.frame(1.0 / 120.0) && n < 2000 {
            n += 1;
        }
        frames.push(n);
        rt.frame(1.0);
    }
    println!("{:<58} {frames:?}", "frames drawn after a click opens a file");
    report(
        "switch to another tab and back",
        sample(5, || {
            rt.update(|s, _| s.open_listed(Some("small.rs")));
            rt.frame(1.0 / 60.0);
            rt.update(|s, _| s.open_listed(Some("big.rs")));
            rt.frame(1.0 / 60.0);
        }),
    );
}
