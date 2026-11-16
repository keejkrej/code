//! A text file as the editor draws it: its lines and its syntax colours, a line at a time.

use std::ops::Range;

use rune::Span;
use rune_kit::syntax::{Highlight, SyntaxTheme};

/// Columns between tab stops.
pub const TAB: usize = 4;

pub struct Doc {
    pub text: String,
    /// Each line's bytes in `text`, without its line ending.
    pub lines: Vec<Range<usize>>,
    /// Coloured ranges of `text`, in order, none overlapping.
    highlights: Vec<Highlight>,
    pub lang: Option<&'static str>,
    /// Lines end in CR LF.
    pub crlf: bool,
    /// The widest line, in columns.
    pub max_cols: usize,
}

impl Doc {
    pub fn new(text: String, mut highlights: Vec<Highlight>, lang: Option<&'static str>) -> Doc {
        let (lines, crlf) = split_lines(&text);
        let max_cols = lines.iter().map(|r| columns(&text[r.clone()])).max().unwrap_or(0);
        highlights
            .retain(|(r, _)| r.start < r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        highlights.sort_by_key(|(r, _)| r.start);
        let mut end = 0;
        highlights.retain(|(r, _)| {
            let keep = r.start >= end;
            if keep {
                end = r.end;
            }
            keep
        });
        Doc { text, lines, highlights, lang, crlf, max_cols }
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", |r| &self.text[r.clone()])
    }

    /// Line `i` as drawn: its text with tabs padded out to their stops, its colours, and the
    /// padding's byte ranges (drawn, never copied).
    pub fn render(&self, i: usize, theme: &SyntaxTheme) -> (String, Vec<Span>, Vec<Range<usize>>) {
        let Some(line) = self.lines.get(i).cloned() else { return (String::new(), Vec::new(), Vec::new()) };
        let first = self.highlights.partition_point(|(r, _)| r.end <= line.start);
        let spans = self.highlights[first..]
            .iter()
            .take_while(|(r, _)| r.start < line.end)
            .map(|(r, paint)| {
                let mut sp = Span::new(r.start.max(line.start) - line.start, r.end.min(line.end) - line.start);
                sp.color = Some(theme.paint(*paint));
                if let rune_kit::syntax::Paint::Kind(kind) = paint {
                    sp.weight = theme.weight(*kind);
                }
                sp
            })
            .collect();
        expand_tabs(&self.text[line], spans)
    }
}

/// The byte range of each line of `text` without its ending, and whether lines end in CR LF.
/// A final line ending doesn't start another line; an empty text is one empty line.
pub fn split_lines(text: &str) -> (Vec<Range<usize>>, bool) {
    let mut lines = Vec::new();
    let mut crlf = false;
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let cr = i > start && text.as_bytes()[i - 1] == b'\r';
            crlf |= cr;
            lines.push(start..i - cr as usize);
            start = i + 1;
        }
    }
    if start < text.len() || lines.is_empty() {
        lines.push(start..text.len());
    }
    (lines, crlf)
}

/// How many columns `line` takes with tabs running to the next stop.
pub fn columns(line: &str) -> usize {
    line.chars().fold(0, |col, ch| if ch == '\t' { col + TAB - col % TAB } else { col + 1 })
}

/// Text layout draws a tab as one space, so each tab is followed by spaces up to its stop.
/// Returns the text to draw, `spans` moved to match it, and the ranges of the added spaces.
pub fn expand_tabs(line: &str, mut spans: Vec<Span>) -> (String, Vec<Span>, Vec<Range<usize>>) {
    if !line.contains('\t') {
        return (line.to_owned(), spans, Vec::new());
    }
    let mut shown = String::with_capacity(line.len() + 8);
    // (byte in `line` after a tab, bytes added up to and including its padding)
    let mut shifts: Vec<(usize, usize)> = Vec::new();
    let mut pads = Vec::new();
    let mut col = 0;
    for (i, ch) in line.char_indices() {
        shown.push(ch);
        if ch != '\t' {
            col += 1;
            continue;
        }
        let pad = TAB - 1 - col % TAB;
        col += pad + 1;
        if pad > 0 {
            let start = shown.len();
            shown.extend(std::iter::repeat_n(' ', pad));
            pads.push(start..shown.len());
            shifts.push((i + 1, shifts.last().map_or(0, |s| s.1) + pad));
        }
    }
    let shift = |b: u32| match shifts.partition_point(|s| s.0 <= b as usize) {
        0 => b,
        n => b + shifts[n - 1].1 as u32,
    };
    for sp in &mut spans {
        (sp.start, sp.end) = (shift(sp.start), shift(sp.end));
    }
    (shown, spans, pads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune_kit::syntax::{Paint, Syntax};

    #[test]
    fn lines_lose_their_endings_and_a_final_ending_adds_no_line() {
        let text = "one\ntwo\n";
        let (lines, crlf) = split_lines(text);
        assert_eq!(lines.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["one", "two"]);
        assert!(!crlf);
        let text = "one\r\n\r\nthree";
        let (lines, crlf) = split_lines(text);
        assert_eq!(lines.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["one", "", "three"]);
        assert!(crlf);
        let bounds = |text: &str| split_lines(text).0.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>();
        assert_eq!(bounds(""), [(0, 0)], "an empty text is one empty line");
        assert_eq!(bounds("\n"), [(0, 0)]);
    }

    #[test]
    fn tabs_run_to_the_next_stop() {
        assert_eq!(columns("\tx"), 5);
        assert_eq!(columns("ab\tx"), 5);
        assert_eq!(columns("abcd\tx"), 9);
        let (shown, _, pads) = expand_tabs("ab\tx\ty", Vec::new());
        assert_eq!(shown, "ab\t x\t  y");
        assert_eq!(pads, [3..4, 6..8]);
        // Copying leaves the padding out and gets the tabs back.
        let copied: String = shown.char_indices().filter(|(i, _)| !pads.iter().any(|p| p.contains(i))).map(|(_, c)| c).collect();
        assert_eq!(copied, "ab\tx\ty");
    }

    #[test]
    fn colours_follow_the_text_past_a_tab() {
        let (shown, spans, _) = expand_tabs("\tlet x", vec![Span::new(1, 4)]);
        assert_eq!(&shown[spans[0].start as usize..spans[0].end as usize], "let");
        // A span that ends at a tab doesn't grow over its padding.
        let (shown, spans, _) = expand_tabs("a\tb", vec![Span::new(0, 1), Span::new(2, 3)]);
        assert_eq!(&shown[spans[0].start as usize..spans[0].end as usize], "a");
        assert_eq!(&shown[spans[1].start as usize..spans[1].end as usize], "b");
    }

    #[test]
    fn a_line_gets_the_part_of_each_colour_that_falls_in_it() {
        let text = "/* a\n b */ let\n".to_owned();
        let doc = Doc::new(
            text,
            vec![(11..14, Paint::Kind(Syntax::Keyword)), (0..10, Paint::Kind(Syntax::Comment)), (2..5, Paint::Kind(Syntax::String))],
            None,
        );
        let theme = SyntaxTheme::neutral(true);
        let (shown, spans, _) = doc.render(0, &theme);
        assert_eq!(shown, "/* a");
        assert_eq!(spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(), [(0, 4)], "the overlapping range is dropped");
        assert_eq!(spans[0].color, Some(theme.color(Syntax::Comment)));
        let (shown, spans, _) = doc.render(1, &theme);
        assert_eq!(shown, " b */ let");
        assert_eq!(spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(), [(0, 5), (6, 9)]);
        assert_eq!(spans[1].color, Some(theme.color(Syntax::Keyword)));
        assert_eq!(doc.max_cols, 9);
    }
}
