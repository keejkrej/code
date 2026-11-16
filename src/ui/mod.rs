//! The window: a side bar (the explorer or source control) beside the editor's card, over a
//! status bar.

mod editor;
mod sidebar;

use std::rc::Rc;

use rune::file_dialog::FileDialogOptions;
use rune::{Chord, Cursor, Cx, Drag, El, Icon, KeyEvent, Phase, Tx, col, el, icon, row, text};
use rune_kit::glyphs;

use crate::app::{App, Body, Panel};
use crate::icons;
use crate::theme::{RADIUS, Theme, theme};

pub const STATUS_H: f32 = 28.0;
/// The space around the editor's card.
pub const GUTTER: f32 = 6.0;

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let dark = cx.system_dark();
    crate::theme::note_system_dark(dark);
    let t = theme(dark);
    let mut body = row().key("body").w_full().grow(1.0).min_h(0.0);
    if s.sidebar_open {
        body = body.child(sidebar::sidebar(s, &t));
    }
    body = body.child(editor::area(s, &t));
    if s.sidebar_open {
        body = body.child(splitter(s, &t));
    }
    let mut root = col().key("window").w_full().h_full().fill(t.backdrop).child(body).child(status_bar(s, &t));
    if let Some(message) = &s.notice {
        root = root.child(notice(message, &t));
    }
    root
}

/// Keys no focused control took.
pub fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let pressed = |chord: &str| Chord::parse(chord).is_ok_and(|c| c.matches_tx(ev, tx));
    if pressed("mod+o") {
        open_folder(s, tx);
    } else if pressed("mod+w") {
        s.close_active_tab();
    } else if pressed("mod+b") {
        s.sidebar_open ^= true;
    } else if pressed("mod+shift+e") {
        s.toggle_panel(Panel::Explorer);
    } else if pressed("mod+shift+g") {
        s.toggle_panel(Panel::Scm);
    } else if pressed("ctrl+tab") || pressed("ctrl+pagedown") {
        s.cycle_tab(1);
    } else if pressed("ctrl+shift+tab") || pressed("ctrl+pageup") {
        s.cycle_tab(-1);
    } else if matches!(s.active_tab().map(|t| &t.body), Some(Body::Diff(_))) {
        // `j`/`k` step through hunks, as in the kit's diff view.
        return s.diff_mut().key(ev, tx);
    } else {
        return false;
    }
    // A shortcut's change lands at once: a view or an editor replaced, not eased in.
    tx.instant();
    true
}

/// Asks for a folder and opens it as the workspace.
pub fn open_folder(s: &mut App, tx: &mut Tx) {
    let options = FileDialogOptions { title: "Open Folder".into(), directory: s.root.clone(), ..Default::default() };
    tx.open_folder(options, |s: &mut App, picked, tx: &mut Tx| match picked {
        Ok(Some(path)) => {
            tx.instant();
            s.open_workspace(path)
        }
        Ok(None) => {}
        Err(e) => s.notice = Some(format!("Could not show the folder dialog: {e}")),
    });
}

/// A small rounded button holding one icon.
pub fn tool_button(t: &Theme, key: &str, glyph: &'static Icon, label: &str, on_press: impl Fn(&mut App, &mut Tx) + 'static) -> El {
    row()
        .key(key)
        .size(24.0, 24.0)
        .center()
        .radius(RADIUS)
        .hover(t.hover)
        .press(t.selected)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label)
        .help(label)
        .hover_group()
        .child(icon(glyph, 16.0).color(t.text_muted).group_color(t.text))
        .on_press_tx(on_press)
}

/// The edge between the side bar and the editor: drag it to resize the side bar.
fn splitter(s: &App, t: &Theme) -> El {
    el().key("splitter")
        .pin(Some(s.sidebar_w - 2.0), Some(0.0), None, Some(0.0))
        .w(GUTTER + 2.0)
        .cursor(Cursor::ResizeColumn)
        .label("Resize side bar")
        .hover_group()
        .child(
            el().key("line").pin(Some(3.0), Some(GUTTER + 8.0), None, Some(8.0)).w(2.0).radius(1.0).fill(t.accent).group_opacity(0.0, 1.0),
        )
        .on_drag(|s: &mut App, d: &mut Drag, _: &mut Tx| match d.phase {
            Phase::Down => d.data = s.sidebar_w,
            Phase::Move => s.set_sidebar_width(d.data + d.dx, false),
            Phase::Up if d.cancelled() => s.set_sidebar_width(d.data, true),
            Phase::Up => s.set_sidebar_width(d.data + d.dx, true),
        })
}

fn status_bar(s: &App, t: &Rc<Theme>) -> El {
    let item = |key: &str, label: String| row().key(key).h_full().px(7.0).center_y().child(text(label).font(t.small).color(t.text_muted));
    let toggle = tool_button(t, "toggle-sidebar", &icons::SIDEBAR, "Toggle Side Bar (Ctrl+B)", |s, tx| {
        // The editor is laid out once at its new width, not on every frame of a slide.
        tx.instant();
        s.sidebar_open ^= true
    });
    let mut bar = row().key("status").w_full().h(STATUS_H).px(GUTTER).gap(2.0).center_y().role("status").child(toggle);
    if let Some(git) = &s.git {
        let mut label = git.status.head_label();
        if git.status.behind > 0 {
            label.push_str(&format!("  \u{2193}{}", git.status.behind));
        }
        if git.status.ahead > 0 {
            label.push_str(&format!("  \u{2191}{}", git.status.ahead));
        }
        let mut branch = row()
            .key("branch")
            .h(22.0)
            .px(7.0)
            .gap(5.0)
            .center_y()
            .radius(RADIUS)
            .cursor(Cursor::Pointer)
            .hover(t.hover)
            .help("Source Control")
            .child(icon(&icons::BRANCH, 14.0).color(t.text_muted))
            .child(text(label).key("name").font(t.small).color(t.text))
            .on_press_tx(|s: &mut App, tx: &mut Tx| {
                tx.instant();
                s.panel = Panel::Scm;
                s.sidebar_open = true;
            });
        let changes = git.status.files.len();
        if changes > 0 {
            // A dot in the colour of a change, and how many there are.
            branch = branch
                .child(el().key("dirty").size(6.0, 6.0).radius(3.0).fill(t.git_modified))
                .child(text(changes.to_string()).key("changes").font(t.small).color(t.text_muted));
        }
        bar = bar.child(branch);
    }
    if s.loading {
        bar = bar.child(item("busy", "Listing files\u{2026}".into()));
    }
    if s.truncated {
        bar = bar.child(item("truncated", format!("Showing the first {} entries", crate::workspace::MAX_ENTRIES)));
    }
    bar = bar.child(row().grow(1.0));
    if let Some(doc) = s.doc() {
        let lines = doc.lines.len();
        bar = bar
            .child(item("lines", format!("{lines} {}", if lines == 1 { "line" } else { "lines" })))
            .child(item("encoding", "UTF-8".into()))
            .child(item("eol", if doc.crlf { "CRLF" } else { "LF" }.into()))
            .child(item("language", language_name(doc.lang)));
    }
    bar
}

/// A language as the status bar names it: the highlighter's name, capitalised.
fn language_name(lang: Option<&str>) -> String {
    let Some(lang) = lang else { return "Plain Text".into() };
    match lang {
        "js" | "javascript" => "JavaScript".into(),
        "ts" | "typescript" => "TypeScript".into(),
        "tsx" => "TypeScript JSX".into(),
        "jsx" => "JavaScript JSX".into(),
        "json" | "html" | "css" | "scss" | "yaml" | "toml" | "sql" | "xml" | "php" => lang.to_uppercase(),
        _ => {
            let mut chars = lang.chars();
            chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
        }
    }
}

/// Something went wrong: a card over the bottom right corner until it is dismissed.
fn notice(message: &str, t: &Theme) -> El {
    let close = row()
        .key("dismiss")
        .size(22.0, 22.0)
        .center()
        .radius(RADIUS)
        .hover(t.hover)
        .cursor(Cursor::Pointer)
        .role("button")
        .label("Dismiss")
        .child(icon(&glyphs::CLOSE, 16.0).color(t.text_muted))
        .on_press(|s: &mut App| s.notice = None);
    let mark = el().key("mark").w(3.0).self_stretch().radius(1.5).fill(t.git_deleted);
    row()
        .key("notice")
        .pin(None, None, Some(GUTTER + 10.0), Some(STATUS_H + 10.0))
        .max_w(460.0)
        .padding(10.0)
        .gap(10.0)
        .items_start()
        .radius(RADIUS + 2.0)
        .fill(t.kit.c.layer_02)
        .border(1.0, t.hairline)
        .shadow(8.0, 24.0, rune::Color::BLACK.alpha(0.35))
        .role("alert")
        .child(mark)
        .child(text(message.to_owned()).key("message").font(t.ui).color(t.text).wrap().grow(1.0).min_w(0.0).selectable(true))
        .child(close)
}

#[cfg(test)]
mod tests;
