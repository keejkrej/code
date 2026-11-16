//! The editor's card: a strip of tabs over the active tab's file, drawn a line at a time, or
//! its diff.

use std::borrow::Cow;
use std::rc::Rc;

use rune::{Cursor, El, GrowthSpec, ScrollSpec, TextStyle, Tx, col, el, icon, rich, row, text};
use rune_kit::diff::DiffView;
use rune_kit::{Button, ButtonVariant, Keybind, SegmentedControl, file_icon, glyphs};

use super::{GUTTER, open_folder, tool_button};
use crate::app::{App, Body, Source, Tab};
use crate::doc::Doc;
use crate::theme::{CARD_RADIUS, LINE_H, RADIUS, Theme};

const TABS_H: f32 = 40.0;
const TAB_H: f32 = 28.0;
const CRUMBS_H: f32 = 24.0;
/// A column of the code face, as a share of its size. A little generous, so the widest line
/// always fits the width worked out for it.
const ADVANCE: f32 = 0.62;

pub fn area(s: &App, t: &Rc<Theme>) -> El {
    let card = col()
        .key("editor")
        .grow(1.0)
        .min_w(0.0)
        .mt(GUTTER)
        .mr(GUTTER)
        .ml(if s.sidebar_open { 0.0 } else { GUTTER })
        .radius(CARD_RADIUS)
        .clip_rounded(CARD_RADIUS)
        .fill(t.surface)
        .border(1.0, t.hairline);
    let Some(tab) = s.active_tab() else {
        return card.child(welcome(s, t));
    };
    let body = match &tab.body {
        // A file is here within a frame or two: an empty card, not a word that flashes by.
        Body::Loading => col().key("loading").w_full().grow(1.0),
        Body::Note(why) => note(t, why),
        Body::Code(doc) => code(tab.uid, doc, t),
        Body::Diff(_) => DiffView::new(App::diff, App::diff_mut).key(format!("diff-{}", tab.uid)).build(s, &t.kit),
    };
    card.child(tab_strip(s, tab, t)).child(crumbs(tab, t)).child(body)
}

/// What shows before anything is open: the name, and the keys that get things going.
fn welcome(s: &App, t: &Theme) -> El {
    let shortcut = |keys: &[&str], what: &str| {
        row()
            .w(280.0)
            .h(30.0)
            .center_y()
            .child(text(what.to_owned()).font(t.ui).color(t.text_muted).grow(1.0))
            .child(Keybind::new(keys).build(&t.kit))
    };
    let keys = col()
        .key("hints")
        .child(shortcut(&["Ctrl", "O"], "Open a folder"))
        .child(shortcut(&["Ctrl", "Shift", "E"], "Browse files"))
        .child(shortcut(&["Ctrl", "Shift", "G"], "Review changes"))
        .child(shortcut(&["Ctrl", "B"], "Toggle the side bar"))
        .child(shortcut(&["Ctrl", "W"], "Close the editor"));
    let logo = TextStyle { tracking: -0.02, ..TextStyle::new(34.0, 600.0).lh(40.0) };
    let tagline =
        if s.root.is_some() { "Pick a file from the side bar to read it." } else { "Open a folder to browse its files and changes." };
    let mut head = col()
        .key("head")
        .gap(6.0)
        .items_center()
        .child(text("code").key("logo").font(logo).color(t.text))
        .child(text(tagline).key("tagline").font(t.ui).color(t.text_muted));
    if s.root.is_none() {
        let open = Button::new("Open Folder").variant(ButtonVariant::Contrast).on_press_tx(open_folder).build(&t.kit).key("welcome-open");
        head = head.child(row().pt(14.0).child(open));
    }
    col().key("welcome").w_full().grow(1.0).center().gap(32.0).child(head).child(keys)
}

fn note(t: &Theme, message: &str) -> El {
    col()
        .key("note")
        .w_full()
        .grow(1.0)
        .center()
        .padding(24.0)
        .child(text(message.to_owned()).key("message").font(t.ui).color(t.text_muted).wrap())
}

fn tab_strip(s: &App, active: &Tab, t: &Rc<Theme>) -> El {
    let tabs = row()
        .key("tabs")
        .h_full()
        .gap(2.0)
        .center_y()
        .role("tablist")
        .label("Editors")
        .children(s.tabs.iter().map(|tab| tab_button(tab, tab.uid == active.uid, t)));
    let strip = row().key("tab-scroll").h_full().grow(1.0).min_w(0.0).scroll_x().child(tabs);
    let mut actions = row().key("tab-actions").h_full().pl(8.0).gap(4.0).center_y();
    if matches!(active.body, Body::Diff(_)) {
        let mode = SegmentedControl::new(&["Split", "Inline"], Some(!s.split as usize)).width(Some(116.0)).on_pick(
            |s: &mut App, i, tx: &mut Tx| {
                tx.motion(t_motion());
                s.set_split(i == 0);
            },
        );
        actions = actions.child(mode.build(&t.kit).key("diff-mode"));
    }
    if let Source::Diff { .. } = active.source {
        let path = active.path.clone();
        actions = actions.child(tool_button(t, "open-file", &glyphs::FILE, "Open File", move |s, tx| {
            tx.instant();
            s.open_file(path.clone(), false)
        }));
    }
    row().key("tab-strip").w_full().h(TABS_H).px(GUTTER).child(strip).child(actions)
}

/// The spring a diff's rows move on when its layout changes.
fn t_motion() -> rune::Spring {
    rune::tok::motion::SNAPPY
}

fn tab_button(tab: &Tab, active: bool, t: &Theme) -> El {
    let uid = tab.uid;
    let font = TextStyle { italic: tab.preview, ..if active { t.ui_medium } else { t.ui } };
    let close = row()
        .key("close")
        .size(18.0, 18.0)
        .center()
        .radius(4.0)
        .hover(t.selected)
        .role("button")
        .label(format!("Close {}", tab.title))
        .child(icon(&glyphs::CLOSE, 14.0).color(t.text_muted))
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            tx.instant();
            s.close_tab(uid)
        });
    let e = row()
        .key(format!("tab-{uid}"))
        .h(TAB_H)
        .pl(9.0)
        .pr(5.0)
        .gap(6.0)
        .center_y()
        .radius(RADIUS)
        .cursor(Cursor::Pointer)
        .hover_group()
        .role("tab")
        .label(tab.title.clone())
        .help(tab.detail.clone())
        .child(icon(file_icon(&tab.title), 16.0).key("icon").color(if active { t.text_muted } else { t.text_faint }))
        .child(text(tab.title.clone()).key("name").font(font).color(if active { t.text } else { t.text_muted }).single_line())
        .child(if active { close } else { close.group_opacity(0.0, 1.0) })
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            // An editor takes the last one's place; it doesn't fade in over it.
            tx.instant();
            s.select_tab(uid);
            s.reveal_active(Some(tx));
            tx.scroll_into_view(format!("tab-{uid}"));
        })
        .on_double_press(move |s: &mut App| s.keep_tab(uid))
        .on_middle_press(move |s: &mut App, tx: &mut Tx| {
            tx.instant();
            s.close_tab(uid)
        });
    if active { e.fill(t.selected) } else { e.hover(t.hover) }
}

/// The path of what the tab shows, a folder at a time, over a rule that sets it off from the
/// code.
fn crumbs(tab: &Tab, t: &Theme) -> El {
    let path = tab.detail.replace('/', "  \u{203a}  ");
    row()
        .key("crumbs")
        .w_full()
        .h(CRUMBS_H)
        .px(GUTTER + 10.0)
        .pb(4.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], t.hairline)
        .child(text(path).key("path").font(t.small).color(t.text_faint).truncate_head())
}

/// A file's lines, built only where they are on screen: a number held at the left edge and
/// the line's coloured text, which can be selected and copied across lines.
fn code(uid: u64, doc: &Doc, t: &Rc<Theme>) -> El {
    let advance = t.code.size * ADVANCE;
    let digits = doc.lines.len().to_string().len().max(2);
    let gutter = (digits as f32 * advance).ceil() + 40.0;
    let width = gutter + (doc.max_cols as f32 * advance).ceil() + 48.0;
    let count = doc.lines.len();
    let theme = t.clone();
    let lines = col()
        .key("lines")
        .w_full()
        .min_w(width)
        .growth(GrowthSpec::default())
        .selection_group(true)
        .role("list")
        .label("Code")
        .virtual_rows(count, LINE_H, |_: &App, i| Cow::Owned(format!("L{i}")), move |s: &App, i| line(s, &theme, i, gutter))
        .virtual_rows_version((uid, count));
    col()
        .key(format!("code-{uid}"))
        .dir(rune::Direction::Ltr)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .pt(8.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_xy()
        .child(lines)
}

fn line(s: &App, t: &Theme, i: usize, gutter: f32) -> El {
    let Some(doc) = s.doc() else { return el() };
    let (shown, spans, pads) = doc.render(i, &t.syntax);
    let number = row()
        .key("gutter")
        .w(gutter)
        .h(LINE_H)
        .pr(20.0)
        .justify_end()
        .fill(t.surface)
        .sticky_left(0.0)
        .a11y_hidden()
        .child(text((i + 1).to_string()).key("number").font(t.code).color(t.line_number).single_line());
    let mut code =
        rich(shown, spans).key("text").font(t.code).color(t.code_text).single_line().selectable(true).selection_color(t.selection);
    for pad in pads {
        code = code.uncopied(pad);
    }
    row().w_full().h(LINE_H).role("listitem").label(format!("line {}: {}", i + 1, doc.line(i))).child(number).child(code)
}
