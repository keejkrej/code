//! The editor's own icons, drawn on a 16 point grid in the element's colour like rune-kit's.

use rune::Icon;

/// The explorer: two stacked pages.
pub static FILES: Icon = Icon {
    name: "code-files",
    view: 16.0,
    body: concat!(
        r#"<path d="M5.5 4.5V1.5H10.5L13.5 4.5V11.5H10.5" stroke="currentColor" stroke-linejoin="round"/>"#,
        r#"<path d="M2.5 4.5H7.5L10.5 7.5V14.5H2.5V4.5Z" stroke="currentColor" stroke-linejoin="round"/>"#
    ),
};

/// Source control: a branch leaving a line of commits.
pub static BRANCH: Icon = Icon {
    name: "code-branch",
    view: 16.0,
    body: concat!(
        r#"<circle cx="4.5" cy="3.5" r="1.75" stroke="currentColor"/>"#,
        r#"<circle cx="4.5" cy="12.5" r="1.75" stroke="currentColor"/>"#,
        r#"<circle cx="11.5" cy="5.5" r="1.75" stroke="currentColor"/>"#,
        r#"<path d="M4.5 5.25V10.75M11.5 7.25C11.5 9.75 4.5 8.25 4.5 10.75" stroke="currentColor"/>"#
    ),
};

pub static REFRESH: Icon = Icon {
    name: "code-refresh",
    view: 16.0,
    body: concat!(
        r#"<path d="M13 8A5 5 0 1 1 11.2 4.2" stroke="currentColor" stroke-linecap="round"/>"#,
        r#"<path d="M11.5 1.5V4.5H8.5" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    ),
};

/// A closed folder's chevron, for a folder the tree has no children for yet.
pub static CHEVRON_RIGHT: Icon = Icon {
    name: "code-chevron-right",
    view: 16.0,
    body: r#"<path d="M6.5 5L9.5 8L6.5 11" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

/// Folders folding shut: two chevrons closing on each other.
pub static COLLAPSE: Icon = Icon {
    name: "code-collapse",
    view: 16.0,
    body: r#"<path d="M5 3L8 6L11 3M5 13L8 10L11 13" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

/// A window with its side bar marked off.
pub static SIDEBAR: Icon = Icon {
    name: "code-sidebar",
    view: 16.0,
    body: r#"<rect x="2.5" y="3.5" width="11" height="9" rx="2" stroke="currentColor"/><path d="M6.5 3.5V12.5" stroke="currentColor"/>"#,
};

pub static PLUS: Icon = Icon { name: "code-plus", view: 16.0, body: r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor"/>"# };

pub static MINUS: Icon = Icon { name: "code-minus", view: 16.0, body: r#"<path d="M3.5 8H12.5" stroke="currentColor"/>"# };
