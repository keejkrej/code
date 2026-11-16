//! code: a basic code editor laid out like Visual Studio Code. It opens a folder, lists its
//! files beside the code, and shows git's changes and their diffs.
//!
//! `code [FOLDER]` opens the folder, or the one open last time. Rune's headless flags work too:
//! `code FOLDER --scene diff --snap out.png` renders a still without a window.

// A release build is a window, not a console program with a window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod doc;
mod git;
mod icons;
mod jobs;
#[cfg(test)]
mod perf;
#[cfg(test)]
mod testing;
mod theme;
mod ui;
mod workspace;

use std::path::PathBuf;

use rune::{AppSpec, Tx, Waker};

use app::{App, Panel};

/// The folder named on the command line, if one was.
fn folder_argument() -> Option<PathBuf> {
    rune::tooling::extra_args().iter().find(|a| !a.starts_with('-')).map(PathBuf::from)
}

fn init() -> App {
    // Headless, nothing waits for a worker: each job runs where it is asked for.
    let headless = rune::tooling::headless_run().is_some();
    let mut app = App::new(headless);
    match folder_argument() {
        Some(folder) => app.open_workspace(folder),
        None if headless => {}
        None => {
            if let Some(last) = app.last_workspace() {
                app.open_workspace(last);
            }
        }
    }
    app
}

fn setup(app: &mut App, waker: Waker) {
    app.set_waker(waker);
}

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    app.poll(clock, tx)
}

/// The window came to the front: files may have changed while it was away.
fn on_focus(app: &mut App, focused: bool, _: &mut Tx) {
    if focused {
        app.rescan_soon();
    }
}

type Scene = (&'static str, fn(&mut App));

/// States for headless stills, each applied to the folder named on the command line.
const SCENES: &[Scene] = &[
    ("file", |s| s.open_listed(std::env::var("CODE_SCENE_FILE").ok().as_deref())),
    ("scm", |s| s.panel = Panel::Scm),
    ("diff", |s| {
        s.panel = Panel::Scm;
        s.open_first_change(false);
    }),
    ("diff-inline", |s| {
        s.panel = Panel::Scm;
        s.split = false;
        s.open_first_change(false);
    }),
    ("staged", |s| {
        s.panel = Panel::Scm;
        s.open_first_change(true);
    }),
];

fn main() {
    // Starts the GPU device while the platform starts up, where Rune can (macOS).
    rune::prewarm_gpu();
    let spec = AppSpec {
        min_size: Some((640.0, 400.0)),
        scenes: SCENES,
        families: ("Inter", "ui-monospace, Inter"),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("code", (1280.0, 800.0), init, ui::view)
    };
    rune::App::new(spec).on_focus(on_focus).run();
}
