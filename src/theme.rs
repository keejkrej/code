//! The editor's look: neutral greys with hairline rules, the editor a rounded card on a darker
//! backdrop, and selection drawn as a quiet tint rather than a colour. One accent, used
//! sparingly. The rune-kit tokens for the kit's components (tree, diff view, buttons, fields)
//! are derived from the same few colours.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::{Color, TextStyle};
use rune_kit::syntax::{Syntax, SyntaxTheme};
use rune_kit::{Kit, PaletteSeed};

use crate::git::Change;

/// The height of one line of code, in the editor and in diffs.
pub const LINE_H: f32 = 20.0;
/// The corner radius of rows, tabs and small buttons.
pub const RADIUS: f32 = 6.0;
/// The corner radius of the editor's card.
pub const CARD_RADIUS: f32 = 10.0;

pub struct Theme {
    pub kit: Kit,
    pub syntax: SyntaxTheme,

    /// The window itself: behind the side bar and the status bar.
    pub backdrop: Color,
    /// The editor's card.
    pub surface: Color,
    /// Rules and outlines, a step from the surface.
    pub hairline: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    /// Code that no token colours.
    pub code_text: Color,
    pub accent: Color,
    pub on_accent: Color,

    /// A row or button under the pointer.
    pub hover: Color,
    /// The selected row, the active tab, the open view.
    pub selected: Color,
    pub line_number: Color,
    /// Selected text.
    pub selection: Color,

    pub git_modified: Color,
    pub git_untracked: Color,
    pub git_deleted: Color,
    pub git_added: Color,
    pub git_conflict: Color,
    pub git_ignored: Color,

    /// 13 point interface text.
    pub ui: TextStyle,
    pub ui_medium: TextStyle,
    /// 12 point secondary text: paths, the status bar.
    pub small: TextStyle,
    pub small_medium: TextStyle,
    /// 10 point counts.
    pub tiny: TextStyle,
    pub code: TextStyle,
}

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::BLACK)
}

impl Theme {
    fn new(dark: bool) -> Theme {
        let pick = |d: &str, l: &str| hex(if dark { d } else { l });
        let backdrop = pick("#121212", "#f2f2f1");
        let surface = pick("#1a1a1a", "#ffffff");
        let hairline = pick("#292929", "#e2e2e0");
        let text = pick("#e2e2e2", "#1f1f1f");
        let text_muted = pick("#9b9b9b", "#6a6a6a");
        let text_faint = pick("#6a6a6a", "#9c9c9c");
        let accent = pick("#7aa7f5", "#2f6fde");
        // The tint over whatever lies under it, so one token serves the backdrop and the card.
        let ink = if dark { Color::WHITE } else { Color::BLACK };

        let mut kit = Kit::from_palette(PaletteSeed { bg: surface, fg: text, accent });
        kit.c.bg_deep = backdrop;
        kit.c.text_muted = text_muted;
        kit.c.text_faint = text_faint;
        kit.c.border_weak = hairline;
        kit.radius.sm = RADIUS;
        kit.radius.md = RADIUS;

        let syntax = SyntaxTheme::from_kit(&kit)
            .with(Syntax::Keyword, pick("#83d6c5", "#1f7a6b"))
            .with(Syntax::String, pick("#e394dc", "#a3399a"))
            .with(Syntax::Number, pick("#ebc88d", "#946200"))
            .with(Syntax::Comment, pick("#6d6d6d", "#9a9a9a"))
            .with(Syntax::Type, pick("#87c3ff", "#2466a8"))
            .with(Syntax::Function, pick("#efb080", "#b0521b"))
            .with(Syntax::Property, pick("#aaa0fa", "#5f47c2"))
            .with(Syntax::Attribute, pick("#aaa0fa", "#5f47c2"))
            .with(Syntax::Parameter, pick("#d6d6dd", "#2b2b2b"))
            .with(Syntax::Variable, pick("#d6d6dd", "#2b2b2b"))
            .with(Syntax::Operator, pick("#d6d6dd", "#2b2b2b"))
            .with(Syntax::Punctuation, pick("#a4a4ad", "#6b6b6b"))
            .with(Syntax::Regex, pick("#e394dc", "#a3399a"))
            .with(Syntax::Heading, pick("#87c3ff", "#2466a8"))
            .with(Syntax::Link, pick("#87c3ff", "#2466a8"));

        let ui = TextStyle::new(13.0, 400.0).lh(18.0);
        let small = TextStyle::new(12.0, 400.0).lh(16.0);
        Theme {
            syntax,
            backdrop,
            surface,
            hairline,
            text,
            text_muted,
            text_faint,
            code_text: pick("#d6d6dd", "#2b2b2b"),
            accent,
            on_accent: pick("#10141c", "#ffffff"),
            hover: ink.alpha(if dark { 0.05 } else { 0.04 }),
            selected: ink.alpha(if dark { 0.09 } else { 0.07 }),
            line_number: pick("#4f4f4f", "#b4b4b4"),
            selection: pick("#39404f", "#d5e2fb"),
            git_modified: pick("#d9a55b", "#946200"),
            git_untracked: pick("#79b882", "#2d7d46"),
            git_deleted: pick("#e06c75", "#c0392b"),
            git_added: pick("#79b882", "#2d7d46"),
            git_conflict: pick("#e58a6a", "#b3541e"),
            git_ignored: pick("#5e5e5e", "#a9a9a9"),
            ui,
            ui_medium: TextStyle { weight: 500.0, ..ui },
            small,
            small_medium: TextStyle { weight: 500.0, ..small },
            tiny: TextStyle::new(10.0, 600.0).lh(12.0),
            code: TextStyle { size: 13.0, line_height: LINE_H, ..kit.ty.mono },
            kit,
        }
    }

    pub fn git_color(&self, change: Change) -> Color {
        match change {
            Change::Modified | Change::TypeChanged => self.git_modified,
            Change::Added => self.git_added,
            Change::Deleted => self.git_deleted,
            Change::Renamed | Change::Copied | Change::Untracked => self.git_untracked,
            Change::Conflicted => self.git_conflict,
        }
    }
}

/// The theme for the system's appearance, built once per appearance.
pub fn theme(dark: bool) -> Rc<Theme> {
    thread_local! {
        static THEMES: [Rc<Theme>; 2] = [Rc::new(Theme::new(false)), Rc::new(Theme::new(true))];
    }
    THEMES.with(|t| t[dark as usize].clone())
}

static SYSTEM_DARK: AtomicBool = AtomicBool::new(true);

/// The view reports the appearance it drew with; the state reads it before the next frame,
/// since only a view is told the system's appearance.
pub fn note_system_dark(dark: bool) {
    SYSTEM_DARK.store(dark, Ordering::Relaxed);
}

pub fn system_dark() -> bool {
    SYSTEM_DARK.load(Ordering::Relaxed)
}
