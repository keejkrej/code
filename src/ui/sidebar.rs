//! The side bar: a switch between its two views in its header, then the explorer's file tree
//! or source control's changes.

use std::rc::Rc;

use rune::{Cursor, El, Icon, Key, KeyEvent, ScrollSpec, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, FileNode, TextArea, Tree, TreeItem, TreeStyle, file_icon, glyphs};

use super::{open_folder, tool_button};
use crate::app::{App, Node, Panel};
use crate::git::{Change, FileStatus};
use crate::icons;
use crate::theme::{RADIUS, Theme};

const HEADER_H: f32 = 44.0;
const ROW_H: f32 = 26.0;
/// Rows stop short of the side bar's edges, so their rounded ends show.
const INSET: f32 = 8.0;
/// Changes listed per group; a repository with more shows the rest as a count.
const MAX_CHANGES: usize = 500;

pub fn sidebar(s: &App, t: &Rc<Theme>) -> El {
    let changes = s.git.as_ref().map_or(0, |g| g.status.files.len());
    let (actions, content) = match s.panel {
        Panel::Explorer => (explorer_actions(s, t), explorer(s, t)),
        Panel::Scm => (scm_actions(s, t), scm(s, t)),
    };
    let views = row()
        .key("views")
        .gap(2.0)
        .role("tablist")
        .label("Views")
        .child(view_tab(s, t, "view-explorer", &icons::FILES, "Explorer (Ctrl+Shift+E)", Panel::Explorer, 0))
        .child(view_tab(s, t, "view-scm", &icons::BRANCH, "Source Control (Ctrl+Shift+G)", Panel::Scm, changes));
    let header =
        row().key("title").w_full().h(HEADER_H).px(INSET).gap(2.0).center_y().child(views).child(row().grow(1.0)).children(actions);
    col().key("sidebar").w(s.sidebar_w).h_full().child(header).child(content)
}

/// One of the header's view buttons: lit while its view shows, with a count of what waits in it.
fn view_tab(s: &App, t: &Theme, key: &str, glyph: &'static Icon, label: &str, panel: Panel, count: usize) -> El {
    let active = s.panel == panel;
    let mut e = row()
        .key(key)
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center()
        .radius(RADIUS)
        .cursor(Cursor::Pointer)
        .role("tab")
        .label(label)
        .help(label)
        .hover_group()
        .child(icon(glyph, 16.0).color(if active { t.text } else { t.text_muted }).group_color(t.text))
        .on_press(move |s: &mut App| {
            s.panel = panel;
            s.sidebar_open = true;
        });
    e = if active { e.fill(t.selected) } else { e.hover(t.hover) };
    if count > 0 {
        let n = if count > 99 { "99+".to_owned() } else { count.to_string() };
        e = e.child(
            row()
                .key("badge")
                .h(16.0)
                .min_w(16.0)
                .px(4.0)
                .center()
                .radius(8.0)
                .fill(t.accent)
                .child(text(n).font(t.tiny).color(t.on_accent)),
        );
    }
    e
}

/// A view's message when it has nothing to list, with what to do about it.
fn empty(t: &Theme, message: &str, action: Option<El>) -> El {
    let mut e = col()
        .key("empty")
        .w_full()
        .px(INSET + 8.0)
        .py(8.0)
        .gap(12.0)
        .child(text(message.to_owned()).key("message").font(t.ui).color(t.text_muted).wrap());
    if let Some(action) = action {
        e = e.child(action);
    }
    e
}

fn primary_button(t: &Theme, key: &str, label: &str, enabled: bool, on_press: impl Fn(&mut App, &mut Tx) + 'static) -> El {
    Button::new(label).variant(ButtonVariant::Contrast).disabled(!enabled).on_press_tx(on_press).build(&t.kit).key(key).w_full()
}

/// A quiet label over a group of rows.
fn heading(t: &Theme, title: &str) -> El {
    text(title.to_owned()).key("name").font(t.small_medium).color(t.text_faint).grow(1.0).min_w(0.0).truncate()
}

/// A collapsible group's heading: a chevron, a title, and what its `tail` adds at the end.
fn section(t: &Theme, key: &str, title: &str, open: bool, tail: Vec<El>, on_toggle: impl Fn(&mut App) + 'static) -> El {
    let chevron = icon(glyphs::DEFAULT.chevron_down, 16.0).key("chevron").color(t.text_faint).icon_rotation(if open {
        0.0
    } else {
        -std::f32::consts::FRAC_PI_2
    });
    row()
        .key(key)
        .w_full()
        .h(ROW_H)
        .pl(4.0)
        .pr(6.0)
        .gap(2.0)
        .center_y()
        .radius(RADIUS)
        .cursor(Cursor::Pointer)
        .hover_group()
        .role("button")
        .label(title)
        .expanded(open)
        .child(chevron)
        .child(heading(t, title))
        .children(tail)
        .on_press(on_toggle)
}

// Explorer -------------------------------------------------------------------------------

fn explorer_actions(s: &App, t: &Theme) -> Vec<El> {
    let mut actions = Vec::new();
    if s.root.is_some() {
        actions.push(tool_button(t, "refresh", &icons::REFRESH, "Refresh", |s, _| s.rescan()));
        actions.push(tool_button(t, "collapse", &icons::COLLAPSE, "Collapse Folders", |s, _| s.collapse_all()));
    }
    actions.push(tool_button(t, "open-folder", &glyphs::FOLDER_OPEN, "Open Folder\u{2026} (Ctrl+O)", open_folder));
    actions
}

fn explorer(s: &App, t: &Rc<Theme>) -> El {
    if s.root.is_none() {
        let open = primary_button(t, "open", "Open Folder", true, open_folder);
        return empty(t, "No folder is open yet.", Some(open));
    }
    let name = row().key("root").w_full().h(ROW_H).px(INSET + 8.0).center_y().child(heading(t, &s.name));
    if s.loading {
        return col().key("explorer").w_full().grow(1.0).min_h(0.0).child(name).child(empty(t, "Listing files\u{2026}", None));
    }
    if s.tree.is_empty() {
        return col().key("explorer").w_full().grow(1.0).min_h(0.0).child(name).child(empty(t, "This folder is empty.", None));
    }
    let mut style = TreeStyle::from_kit(&t.kit)
        .hover(t.hover)
        .radius(RADIUS)
        .gap(6.0)
        .label(t.ui, t.text, t.text)
        .chevron_only_folders(true)
        .chevron_colors(t.text_faint, t.text_muted)
        .guides_on_hover(1.0);
    (style.row_height, style.indent, style.padding) = (ROW_H, 10.0, 6.0);
    (style.guide, style.selected) = (t.hairline, t.selected);
    let (theme, decor) = (t.clone(), s.decor.clone());
    let tree = Tree::new(|s: &App| &s.tree, |s: &mut App| &mut s.tree, FileNode::label)
        .name("Files")
        .style(style)
        .row(move |node: &Node, r| {
            let t = &theme;
            let meta = node.data.unwrap_or_default();
            let mut item = TreeItem::file(node, r).icon_color(t.text_faint);
            if node.dir && r.expanded.is_none() {
                // An empty folder, or one not listed yet: closed, like its neighbours.
                item = item.icon(&icons::CHEVRON_RIGHT);
            }
            if let Some(&change) = decor.files.get(&node.path) {
                let color = t.git_color(change);
                let letter = text(change.letter()).key("status").font(t.small_medium).color(color).w(10.0);
                item.label_color(color).trailing(letter)
            } else if decor.dirs.contains(&node.path) {
                item.dot(t.git_modified)
            } else if meta.ignored {
                item.label_color(t.git_ignored)
            } else {
                item
            }
        })
        .on_select(|s: &mut App, id, _| s.pick(id, true))
        .on_open(|s: &mut App, id, _| s.pick(id, false))
        .build(s, &t.kit)
        .grow(1.0)
        .min_h(0.0);
    // The rows are inset, not the tree's own scroll area.
    let tree = col().key("tree").w_full().grow(1.0).min_h(0.0).px(INSET).child(tree);
    col().key("explorer").w_full().grow(1.0).min_h(0.0).child(name).child(tree)
}

// Source control -------------------------------------------------------------------------

fn scm_actions(s: &App, t: &Theme) -> Vec<El> {
    if s.git.is_none() {
        return Vec::new();
    }
    vec![tool_button(t, "refresh", &icons::REFRESH, "Refresh", |s, _| s.rescan())]
}

fn scm(s: &App, t: &Rc<Theme>) -> El {
    if s.root.is_none() {
        let open = primary_button(t, "open", "Open Folder", true, open_folder);
        return empty(t, "Open a folder to see its changes.", Some(open));
    }
    if s.loading {
        return empty(t, "Reading the repository\u{2026}", None);
    }
    let Some(git) = &s.git else {
        let init = primary_button(t, "init", "Initialize Repository", !s.git_busy, |s, _| s.init_repository());
        return empty(t, "This folder is not a Git repository yet.", Some(init));
    };
    let branch = git.status.head_label();
    let message = TextArea::new(&s.commit)
        .label("Commit message")
        .placeholder(format!("Message for {branch}"))
        .auto_grow(1, 8)
        // A field is as wide as its text wants unless told: keep it inside the side bar.
        .width(Some(s.sidebar_w - 2.0 * INSET))
        .on_edit(|s: &mut App| &mut s.commit)
        // Return breaks the line; the commit is Ctrl+Enter or the button.
        .on_submit(|s: &mut App, _| s.commit.insert("\n"))
        .on_key(|s: &mut App, ev: &KeyEvent, _| {
            let commit = ev.key == Key::Enter && ev.mods.cmd;
            if commit {
                s.commit();
            }
            commit
        })
        .build(&t.kit)
        .key("message");
    let staged: Vec<&FileStatus> = git.status.staged().collect();
    let unstaged: Vec<&FileStatus> = git.status.unstaged().collect();
    let commit = Button::new(if s.git_busy { "Working\u{2026}" } else { "Commit" })
        .variant(ButtonVariant::Contrast)
        .shortcut("Ctrl+\u{21b5}")
        .disabled(!s.can_commit())
        .on_press_tx(|s: &mut App, _| s.commit())
        .build(&t.kit)
        .key("commit")
        .w_full();
    let hint = match (staged.is_empty(), unstaged.is_empty()) {
        (true, true) => Some("Nothing has changed."),
        (true, false) => Some("Stage changes with + to commit them."),
        _ => None,
    };
    let mut top = col().key("commit-box").w_full().px(INSET).pb(12.0).gap(8.0).child(message).child(commit);
    if let Some(hint) = hint {
        top = top.child(text(hint).key("hint").font(t.small).color(t.text_faint).wrap());
    }

    let count = |n: usize| text(n.to_string()).key("count").font(t.small).color(t.text_faint).px(4.0);
    let mut list = col().key("changes").w_full().px(INSET).pb(8.0);
    if !staged.is_empty() {
        let tail = vec![
            tool_button(t, "unstage-all", &icons::MINUS, "Unstage All", |s, _| s.unstage_all()).group_opacity(0.0, 1.0),
            count(staged.len()),
        ];
        list = list.child(section(t, "staged", "Staged", !s.staged_closed, tail, |s| s.staged_closed ^= true));
        if !s.staged_closed {
            list = list.children(change_rows(t, &staged, true));
        }
    }
    if !unstaged.is_empty() {
        let tail = vec![
            tool_button(t, "stage-all", &icons::PLUS, "Stage All", |s, _| s.stage_all()).group_opacity(0.0, 1.0),
            count(unstaged.len()),
        ];
        list = list.child(section(t, "unstaged", "Changes", !s.changes_closed, tail, |s| s.changes_closed ^= true));
        if !s.changes_closed {
            list = list.children(change_rows(t, &unstaged, false));
        }
    }
    let scroll = col().key("scm-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);
    col().key("scm").w_full().grow(1.0).min_h(0.0).child(top).child(scroll)
}

fn change_rows(t: &Theme, files: &[&FileStatus], staged: bool) -> Vec<El> {
    let mut rows: Vec<El> = files.iter().take(MAX_CHANGES).map(|f| change_row(t, f, staged)).collect();
    if files.len() > MAX_CHANGES {
        let more = format!("\u{2026} and {} more", files.len() - MAX_CHANGES);
        rows.push(row().key("more").w_full().h(ROW_H).pl(26.0).center_y().child(text(more).font(t.ui).color(t.text_muted)));
    }
    rows
}

fn change_row(t: &Theme, file: &FileStatus, staged: bool) -> El {
    let change = if staged { file.staged } else { file.unstaged }.unwrap_or(Change::Modified);
    let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
    let color = t.git_color(change);
    let side = if staged { "staged" } else { "unstaged" };
    let mut label =
        text(name.to_owned()).key("name").font(t.ui).color(if change == Change::Deleted { t.text_muted } else { t.text }).truncate();
    if change == Change::Deleted {
        label = label.strike(0, name.len(), Some(t.text_faint));
    }
    let from = file.orig_path.as_deref().filter(|_| staged).map(|p| format!(" \u{2190} {p}")).unwrap_or_default();
    let detail = text(format!("{dir}{from}")).key("dir").font(t.small).color(t.text_faint).grow(1.0).min_w(0.0).truncate();
    let path = file.path.clone();
    let action = if staged {
        tool_button(t, "unstage", &icons::MINUS, "Unstage", move |s, _| s.unstage(vec![path.clone()]))
    } else {
        tool_button(t, "stage", &icons::PLUS, "Stage", move |s, _| s.stage(vec![path.clone()]))
    };
    let (open, keep) = (file.clone(), file.clone());
    row()
        .key(format!("{side}:{}", file.path))
        .w_full()
        .h(ROW_H)
        .pl(22.0)
        .pr(6.0)
        .gap(6.0)
        .center_y()
        .radius(RADIUS)
        .cursor(Cursor::Pointer)
        .hover(t.hover)
        .hover_group()
        .role("listitem")
        .label(format!("{}, {} ({side})", file.path, change.describe()))
        .child(icon(file_icon(name), 16.0).key("icon").color(t.text_faint))
        .child(label)
        .child(detail)
        .child(action.group_opacity(0.0, 1.0))
        .child(text(change.letter()).key("status").font(t.small_medium).color(color).w(14.0).align(rune::Align::Center))
        .on_press(move |s: &mut App| s.open_diff(&open, staged, true))
        .on_double_press(move |s: &mut App| s.open_diff(&keep, staged, false))
}
