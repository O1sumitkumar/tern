//! Find in scrollback. Written against `alacritty_terminal`'s search API
//! (`RegexSearch`, `RegexIter`), not adapted from Zed's `terminal.rs`: that
//! file is GPL-3.0-or-later and this module is meant to stay free of it.
//!
//! [`Search`] keeps the query, every match in the grid (history included, soft
//! wraps followed), and the current match. Matches are rescanned whenever new
//! output arrives, and the current match is tracked by its row counted from
//! the top of the buffer so it stays on the same text while history grows.
//!
//! The find bar's key-driven query buffer. The view draws it; this decides what
//! a keystroke means. There is no text-input widget here (tern-term cannot
//! depend on the app crate's), only typing, backspace, Enter and Escape.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use gpui::Keystroke;

/// Cap on stored matches: a one-letter query over a full 10k-line history
/// would otherwise hold (and rescan, per chunk of output) a huge list.
const MAX_MATCHES: usize = 10_000;

/// What a cell is, as far as the search overlay is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMark {
    #[default]
    None,
    /// Part of a match.
    Match,
    /// Part of the current match.
    Current,
}

/// A match start counted from the top of the buffer: (row, column). Stable
/// while history grows, because the line index drops by exactly what the
/// history gains.
type Anchor = (i64, usize);

fn anchor_of<T>(term: &Term<T>, m: &Match) -> Anchor {
    let start = m.start();
    (
        i64::from(start.line.0) + term.history_size() as i64,
        start.column.0,
    )
}

pub(crate) struct Search {
    query: String,
    regex: Option<RegexSearch>,
    matches: Vec<Match>,
    current: Option<usize>,
    anchor: Option<Anchor>,
}

/// Escape regex metacharacters so the query is matched literally.
fn escape(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Search {
    /// Start a search. An empty query matches nothing. Matching is literal and
    /// smart-case (case-insensitive unless the query has an uppercase letter).
    pub(crate) fn new<T>(term: &Term<T>, query: &str) -> Self {
        let regex = if query.is_empty() {
            None
        } else {
            RegexSearch::new(&escape(query)).ok()
        };
        let mut search = Self {
            query: query.to_string(),
            regex,
            matches: Vec::new(),
            current: None,
            anchor: None,
        };
        search.matches = search.collect(term);
        // Start from the newest match: the thing just printed is the thing
        // usually looked for.
        search.select(term, search.matches.len().checked_sub(1));
        search
    }

    fn collect<T>(&mut self, term: &Term<T>) -> Vec<Match> {
        let Some(regex) = self.regex.as_mut() else {
            return Vec::new();
        };
        let start = Point::new(term.topmost_line(), Column(0));
        let end = Point::new(term.bottommost_line(), term.last_column());
        RegexIter::new(start, end, Direction::Right, term, regex)
            .take(MAX_MATCHES)
            .collect()
    }

    fn select<T>(&mut self, term: &Term<T>, index: Option<usize>) {
        self.current = index;
        self.anchor = index
            .and_then(|i| self.matches.get(i))
            .map(|m| anchor_of(term, m));
    }

    /// Rescan after the grid changed, keeping the current match on the same
    /// text where it still exists (else the next one after it).
    pub(crate) fn refresh<T>(&mut self, term: &Term<T>) {
        self.matches = self.collect(term);
        let index = match self.anchor {
            _ if self.matches.is_empty() => None,
            None => Some(self.matches.len() - 1),
            Some(anchor) => {
                let ix = self
                    .matches
                    .partition_point(|m| anchor_of(term, m) < anchor);
                Some(ix.min(self.matches.len() - 1))
            }
        };
        // The anchor stays where it was: re-anchoring would pin it to
        // whatever slid underneath once the history cap is reached.
        self.current = index;
        if self.anchor.is_none() {
            self.select(term, index);
        }
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn count(&self) -> usize {
        self.matches.len()
    }

    /// Zero-based index of the current match.
    pub(crate) fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub(crate) fn current(&self) -> Option<&Match> {
        self.current.and_then(|i| self.matches.get(i))
    }

    /// Move to the next match going up into older output (`older`) or down
    /// toward newer output, wrapping at either end.
    pub(crate) fn step<T>(&mut self, term: &Term<T>, older: bool) -> Option<&Match> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        let ix = self.current.unwrap_or(n - 1);
        let next = if older {
            (ix + n - 1) % n
        } else {
            (ix + 1) % n
        };
        self.select(term, Some(next));
        self.current()
    }

    /// Matches touching grid lines `top..=bottom`, each flagged whether it is
    /// the current one. Matches are sorted and disjoint, so the lower bound is
    /// binary-searched.
    pub(crate) fn visible(&self, top: i32, bottom: i32) -> Vec<(Match, bool)> {
        let first = self.matches.partition_point(|m| m.end().line.0 < top);
        self.matches[first..]
            .iter()
            .enumerate()
            .take_while(|(_, m)| m.start().line.0 <= bottom)
            .map(|(i, m)| (m.clone(), self.current == Some(first + i)))
            .collect()
    }
}

/// The mark for the cell at `point` given the visible matches.
pub(crate) fn mark_at(visible: &[(Match, bool)], point: Point) -> SearchMark {
    for (m, current) in visible {
        if m.contains(&point) {
            return if *current {
                SearchMark::Current
            } else {
                SearchMark::Match
            };
        }
    }
    SearchMark::None
}

/// What a keystroke did to the find bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindKey {
    /// Escape: close the bar and drop the highlights.
    Close,
    /// Enter: the next match (older output).
    Next,
    /// Shift+Enter: the previous match (newer output).
    Prev,
    /// The query text changed: search again.
    Edited,
    /// Nothing for the bar; the key is still swallowed so it never reaches
    /// the remote while the bar has focus.
    Ignored,
}

/// Apply `keystroke` to `query`.
pub(crate) fn apply_key(query: &mut String, keystroke: &Keystroke) -> FindKey {
    let mods = &keystroke.modifiers;
    match keystroke.key.as_str() {
        "escape" => return FindKey::Close,
        "enter" => {
            return if mods.shift {
                FindKey::Prev
            } else {
                FindKey::Next
            };
        }
        "backspace" => {
            return if query.pop().is_some() {
                FindKey::Edited
            } else {
                FindKey::Ignored
            };
        }
        _ => {}
    }
    if mods.platform || mods.control {
        return FindKey::Ignored;
    }
    match keystroke.key_char.as_deref() {
        Some(text) if !text.is_empty() && !text.chars().any(char::is_control) => {
            query.push_str(text);
            FindKey::Edited
        }
        _ => FindKey::Ignored,
    }
}

/// Append pasted text: first line only, since a query is one line.
pub(crate) fn paste_into(query: &mut String, text: &str) -> FindKey {
    let line = text.lines().next().unwrap_or("");
    if line.is_empty() {
        return FindKey::Ignored;
    }
    query.push_str(line);
    FindKey::Edited
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use alacritty_terminal::index::{Column, Line};

    use super::*;
    use crate::terminal::Terminal;

    fn point(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    /// The text of every cell marked `mark` in the viewport, row by row.
    fn marked(t: &Terminal, mark: SearchMark) -> Vec<String> {
        t.lines()
            .iter()
            .map(|row| {
                row.iter()
                    .filter(|c| c.search == mark)
                    .map(|c| c.ch)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .collect()
    }

    #[test]
    fn match_spanning_a_soft_wrap_is_one_match() {
        // 10 columns: "hello" starts at column 7 and wraps onto the next row.
        let mut t = Terminal::new(10, 4);
        t.process(b"xxxxxxxhello world");
        assert_eq!(t.search("hello"), 1);
        let visible = t.visible_matches();
        assert_eq!(visible.len(), 1);
        let (m, current) = &visible[0];
        assert!(*current);
        assert_eq!((*m.start(), *m.end()), (point(0, 7), point(1, 1)));
        assert_eq!(marked(&t, SearchMark::Current), ["hel", "lo"]);
    }

    #[test]
    fn hard_newline_does_not_join_a_match() {
        let mut t = Terminal::new(10, 4);
        t.process(b"xxxxxxxhel\r\nlo world");
        assert_eq!(t.search("hello"), 0);
    }

    #[test]
    fn finds_matches_in_scrollback_and_scrolls_to_them() {
        let mut t = Terminal::new(20, 3);
        for i in 1..=9 {
            let tag = if i == 2 || i == 8 { "needle" } else { "hay" };
            t.process(format!("line{i} {tag}\r\n").as_bytes());
        }
        assert_eq!(t.search("needle"), 2);
        // Newest match first: line8 is on screen, nothing scrolled.
        assert_eq!(t.search_index(), Some(1));
        assert_eq!(t.display_offset(), 0);
        assert_eq!(marked(&t, SearchMark::Current), ["needle"]);
        // Older: line2 sits in history, so the view must scroll up to show it.
        assert!(t.search_step(true));
        assert_eq!(t.search_index(), Some(0));
        assert!(t.display_offset() > 0);
        assert_eq!(marked(&t, SearchMark::Current), ["needle"]);
        let row = t
            .lines()
            .iter()
            .position(|r| r.iter().any(|c| c.search == SearchMark::Current))
            .unwrap();
        assert!(t.row_text(row).starts_with("line2"), "{}", t.row_text(row));
        // The other match is not on screen, so it carries no mark.
        assert!(marked(&t, SearchMark::Match).is_empty());
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut t = Terminal::new(20, 6);
        t.process(b"a1\r\nb\r\na2\r\nb\r\na3");
        assert_eq!(t.search("a"), 3);
        assert_eq!(t.search_index(), Some(2));
        t.search_step(false);
        assert_eq!(t.search_index(), Some(0));
        t.search_step(true);
        assert_eq!(t.search_index(), Some(2));
        t.search_step(true);
        assert_eq!(t.search_index(), Some(1));
    }

    #[test]
    fn query_is_literal_and_smart_case() {
        let mut t = Terminal::new(30, 3);
        t.process(b"axb a.b Foo foo");
        assert_eq!(t.search("a.b"), 1, "dot is not a wildcard");
        assert_eq!(t.search("foo"), 2, "lowercase query ignores case");
        assert_eq!(t.search("Foo"), 1, "uppercase query is exact");
        assert_eq!(t.search("(a"), 0, "unbalanced paren is not an error");
    }

    #[test]
    fn new_output_refreshes_matches_and_keeps_the_current_one() {
        let mut t = Terminal::new(20, 3);
        t.process(b"err one\r\nok\r\nerr two\r\n");
        assert_eq!(t.search("err"), 2);
        t.search_step(true);
        assert_eq!(t.search_index(), Some(0));
        t.process(b"err three\r\nmore\r\n");
        assert_eq!(t.search_count(), 3);
        assert_eq!(t.search_index(), Some(0), "still on 'err one'");
    }

    #[test]
    fn refresh_keeps_a_middle_match_current_as_history_grows() {
        let mut t = Terminal::new(20, 3);
        t.process(b"err 1\r\nerr 2\r\nerr 3\r\n");
        assert_eq!(t.search("err"), 3);
        t.search_step(true);
        assert_eq!(t.search_index(), Some(1), "on 'err 2'");
        t.process(b"x\r\ny\r\nz\r\n");
        assert_eq!(t.search_count(), 3);
        assert_eq!(t.search_index(), Some(1), "still on 'err 2'");
    }

    #[test]
    fn refresh_with_no_matches_left_has_no_current_match() {
        let mut t = Terminal::new(20, 6);
        t.process(b"err one");
        assert_eq!(t.search("err"), 1);
        assert_eq!(t.search_index(), Some(0));
        t.process(b"\x1bc");
        assert_eq!(t.search_count(), 0);
        assert_eq!(t.search_index(), None);
        // A query that never matched stays at no match through new output.
        assert_eq!(t.search("zebra"), 0);
        t.process(b"more");
        assert_eq!(t.search_count(), 0);
        assert_eq!(t.search_index(), None);
    }

    #[test]
    fn first_matches_arriving_after_an_empty_search_select_the_newest() {
        let mut t = Terminal::new(20, 6);
        t.process(b"nothing yet");
        assert_eq!(t.search("err"), 0);
        assert_eq!(t.search_index(), None);
        t.process(b"\r\nerr a\r\nerr b");
        assert_eq!(t.search_count(), 2);
        assert_eq!(t.search_index(), Some(1));
    }

    #[test]
    fn current_match_clamps_when_every_remaining_match_is_older() {
        let mut t = Terminal::new(20, 6);
        t.process(b"err a\r\nx\r\nerr b");
        assert_eq!(t.search("err"), 2);
        assert_eq!(t.search_index(), Some(1), "on 'err b' at row 2");
        // The screen is wiped and one match is written above where 'err b' was.
        t.process(b"\x1bc");
        t.process(b"err c");
        assert_eq!(t.search_count(), 1);
        assert_eq!(t.search_index(), Some(0));
    }

    #[test]
    fn clearing_removes_marks() {
        let mut t = Terminal::new(20, 3);
        t.process(b"find me");
        assert_eq!(t.search("find"), 1);
        assert!(!marked(&t, SearchMark::Current).is_empty());
        t.clear_search();
        assert!(marked(&t, SearchMark::Current).is_empty());
        assert_eq!(t.search_count(), 0);
        assert_eq!(t.search(""), 0);
    }

    #[test]
    fn every_occurrence_is_found_at_its_own_position() {
        // "ab" at (0,0) and (0,4), then on the next row at (1,1).
        let mut t = Terminal::new(20, 3);
        t.process(b"ab cab\r\nxab");
        assert_eq!(t.search("ab"), 3);
        let spans: Vec<_> = t
            .visible_matches()
            .iter()
            .map(|(m, _)| (*m.start(), *m.end()))
            .collect();
        assert_eq!(
            spans,
            [
                (point(0, 0), point(0, 1)),
                (point(0, 4), point(0, 5)),
                (point(1, 1), point(1, 2)),
            ]
        );
    }

    #[test]
    fn occurrences_do_not_overlap() {
        // "aa" in "aaaa" is two matches, not three: the search resumes past each match.
        let mut t = Terminal::new(10, 2);
        t.process(b"aaaa");
        assert_eq!(t.search("aa"), 2);
        let spans: Vec<_> = t
            .visible_matches()
            .iter()
            .map(|(m, _)| (*m.start(), *m.end()))
            .collect();
        assert_eq!(
            spans,
            [(point(0, 0), point(0, 1)), (point(0, 2), point(0, 3))]
        );
    }

    #[test]
    fn several_matches_after_a_soft_wrap_are_all_found() {
        // 10 columns: the first "hello" wraps onto row 1; the second follows on that row.
        let mut t = Terminal::new(10, 4);
        t.process(b"xxxxxxxhello hello");
        assert_eq!(t.search("hello"), 2);
        let spans: Vec<_> = t
            .visible_matches()
            .iter()
            .map(|(m, _)| (*m.start(), *m.end()))
            .collect();
        assert_eq!(
            spans,
            [(point(0, 7), point(1, 1)), (point(1, 3), point(1, 7))]
        );
    }

    #[test]
    fn empty_query_ends_an_active_search_and_marks_nothing() {
        let mut t = Terminal::new(20, 3);
        t.process(b"find me");
        assert_eq!(t.search("find"), 1);
        assert_eq!(t.search(""), 0);
        assert_eq!(t.search_query(), None);
        assert_eq!(t.search_index(), None);
        assert!(t.visible_matches().is_empty());
        assert!(marked(&t, SearchMark::Match).is_empty());
        assert!(marked(&t, SearchMark::Current).is_empty());
    }

    #[test]
    fn query_with_no_match_has_no_current_match() {
        let mut t = Terminal::new(20, 3);
        t.process(b"find me");
        assert_eq!(t.search("zebra"), 0);
        assert_eq!(t.search_query(), Some("zebra"));
        assert_eq!(t.search_index(), None);
        assert!(marked(&t, SearchMark::Match).is_empty());
        assert!(marked(&t, SearchMark::Current).is_empty());
        assert!(!t.search_step(false), "nothing to step to");
    }

    fn key(source: &str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    /// A printable keystroke as the platform delivers it: key plus typed text.
    fn typed(key: &str, text: &str) -> Keystroke {
        Keystroke {
            key_char: Some(text.into()),
            ..self::key(key)
        }
    }

    #[test]
    fn typing_builds_the_query_and_backspace_removes() {
        let mut q = String::new();
        assert_eq!(apply_key(&mut q, &typed("f", "f")), FindKey::Edited);
        assert_eq!(apply_key(&mut q, &typed("shift-o", "O")), FindKey::Edited);
        assert_eq!(q, "fO");
        assert_eq!(apply_key(&mut q, &key("backspace")), FindKey::Edited);
        assert_eq!(q, "f");
        apply_key(&mut q, &key("backspace"));
        assert_eq!(apply_key(&mut q, &key("backspace")), FindKey::Ignored);
        assert_eq!(q, "");
    }

    #[test]
    fn enter_and_escape_are_navigation_not_text() {
        let mut q = "x".to_string();
        assert_eq!(apply_key(&mut q, &key("enter")), FindKey::Next);
        assert_eq!(apply_key(&mut q, &key("shift-enter")), FindKey::Prev);
        assert_eq!(apply_key(&mut q, &key("escape")), FindKey::Close);
        assert_eq!(q, "x");
    }

    #[test]
    fn command_chords_do_not_type() {
        let mut q = String::new();
        let cmd_a = Keystroke {
            key_char: Some("a".into()),
            ..key("cmd-a")
        };
        let ctrl_c = Keystroke {
            key_char: Some("\u{3}".into()),
            ..key("ctrl-c")
        };
        assert_eq!(apply_key(&mut q, &cmd_a), FindKey::Ignored);
        assert_eq!(apply_key(&mut q, &ctrl_c), FindKey::Ignored);
        assert_eq!(q, "");
    }

    #[test]
    fn empty_or_control_text_does_not_type() {
        let mut q = "a".to_string();
        assert_eq!(apply_key(&mut q, &typed("b", "")), FindKey::Ignored);
        assert_eq!(apply_key(&mut q, &typed("tab", "\t")), FindKey::Ignored);
        assert_eq!(apply_key(&mut q, &typed("x", "x\u{7}")), FindKey::Ignored);
        assert_eq!(q, "a");
    }

    #[test]
    fn paste_takes_the_first_line() {
        let mut q = "a".to_string();
        assert_eq!(paste_into(&mut q, "bc\nrm -rf"), FindKey::Edited);
        assert_eq!(q, "abc");
        assert_eq!(paste_into(&mut q, ""), FindKey::Ignored);
    }

    mod property {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            // The oracle is std's non-overlapping `matches`, an independent implementation.
            #[test]
            fn match_count_equals_non_overlapping_occurrences(text in "[ab ]{0,30}") {
                let mut t = Terminal::new(40, 3);
                t.process(text.as_bytes());
                prop_assert_eq!(t.search("aa"), text.matches("aa").count());
            }
        }
    }
}
