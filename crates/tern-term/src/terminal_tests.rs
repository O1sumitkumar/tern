#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows)
}

/// The bytes a `process` call sends back to the remote.
fn output(notices: &[Notice]) -> Vec<Vec<u8>> {
    notices
        .iter()
        .filter_map(|n| match n {
            Notice::Event(TerminalEvent::Output(b)) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn plain_text_lands_on_grid_and_moves_cursor() {
    let mut t = term(20, 5);
    t.process(b"one\r\ntwo");
    assert_eq!(t.row_text(0), "one");
    assert_eq!(t.row_text(1), "two");
    let c = t.cursor().unwrap();
    assert_eq!((c.row, c.col), (1, 3));
}

#[test]
fn dsr_cursor_report_is_written_back() {
    let mut t = term(20, 5);
    assert!(output(&t.process(b"\x1b[2;3H")).is_empty());
    assert_eq!(output(&t.process(b"\x1b[6n")), [b"\x1b[2;3R".to_vec()]);
}

#[test]
fn sgr_colors_and_attributes() {
    let mut t = term(40, 4);
    t.process(b"\x1b[1;31mred\x1b[0m \x1b[4mu\x1b[0m\x1b[38;2;10;20;30mT");
    let line = t.line(0);
    assert_eq!(line[0].fg, CellColor::Indexed(1));
    assert!(line[0].bold);
    assert_eq!(line[3].fg, CellColor::Foreground);
    assert!(line[4].underline);
    assert_eq!(line[5].fg, CellColor::Rgb(10, 20, 30));
}

#[test]
fn title_and_bell_become_events() {
    let mut t = term(20, 2);
    let n = t.process(b"\x1b]0;my title\x07\x07");
    assert_eq!(
        n,
        vec![
            Notice::Event(TerminalEvent::TitleChanged("my title".into())),
            Notice::Event(TerminalEvent::Bell),
        ]
    );
}

#[test]
fn osc11_background_query_answers_with_theme_color() {
    let mut t = term(20, 2);
    let w = output(&t.process(b"\x1b]11;?\x07"));
    assert_eq!(w.len(), 1);
    let s = String::from_utf8_lossy(&w[0]).to_string();
    assert!(s.starts_with("\x1b]11;rgb:1616/1818/1a1a"), "{s:?}");
}

#[test]
fn app_cursor_mode_toggles() {
    let mut t = term(10, 2);
    assert!(!t.mode().contains(TermMode::APP_CURSOR));
    t.process(b"\x1b[?1h");
    assert!(t.mode().contains(TermMode::APP_CURSOR));
}

#[test]
fn resize_changes_grid_and_remote_callback_reports_pixels() {
    let mut t = term(20, 5);
    assert!(!t.resize(20, 5, 8.0, 16.0));
    assert!(t.resize(30, 10, 8.0, 16.0));
    assert_eq!((t.cols(), t.rows()), (30, 10));
    assert_eq!(
        t.resized_event(),
        TerminalEvent::Resized {
            cols: 30,
            rows: 10,
            pixel_width: 240,
            pixel_height: 160
        }
    );
}

#[test]
fn selection_yields_text() {
    let mut t = term(20, 3);
    t.process(b"hello world");
    let start = t.grid_point(0, 0);
    t.start_selection(SelectionType::Simple, start, Side::Left);
    let end = t.grid_point(0, 4);
    t.update_selection(end, Side::Right);
    assert_eq!(t.selection_text().as_deref(), Some("hello"));
    assert!(t.line(0)[..5].iter().all(|c| c.selected));
    assert!(!t.line(0)[5].selected);
}

#[test]
fn scrollback_scrolls_and_clamps() {
    let mut t = term(10, 3);
    for i in 1..=8 {
        t.process(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(t.row_text(0), "line7");
    t.scroll(100);
    assert_eq!(t.display_offset(), t.history_size());
    assert_eq!(t.row_text(0), "line1");
    t.scroll_to_bottom();
    assert_eq!(t.display_offset(), 0);
}

#[test]
fn utf8_split_across_feeds_reassembles() {
    let mut t = term(10, 2);
    let b = "é".as_bytes();
    t.process(&b[..1]);
    t.process(&b[1..]);
    assert_eq!(t.row_text(0), "é");
}

#[test]
fn history_is_capped_by_the_constructor_argument() {
    let mut t = Terminal::with_scrollback(10, 3, 5);
    for i in 1..=40 {
        t.process(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(t.history_size(), 5);
    t.scroll(100);
    assert_eq!(t.row_text(0), "line34");
    assert_eq!(Terminal::new(10, 3).history_size(), 0);
}

#[test]
fn scrollback_resizes_at_runtime_without_losing_what_fits() {
    let mut t = Terminal::with_scrollback(10, 3, 20);
    for i in 1..=30 {
        t.process(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(t.history_size(), 20);
    t.set_scrollback(8);
    assert_eq!(t.history_size(), 8, "shrinking drops the oldest lines");
    t.scroll(100);
    assert_eq!(
        t.row_text(0),
        "line21",
        "the newest 8 history lines survive"
    );
    t.scroll_to_bottom();
    t.set_scrollback(50);
    for i in 31..=60 {
        t.process(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(t.history_size(), 38);
}

#[test]
fn the_cursor_line_is_what_precedes_the_cursor_on_its_row() {
    let mut t = term(40, 5);
    t.process(b"old output\r\n[sudo] password for test: ");
    assert_eq!(
        t.cursor_line_text().as_deref(),
        Some("[sudo] password for test:")
    );
    // Text after the cursor is not part of the line being typed.
    t.process(b"\r\x1b[2Cab\x1b[2D");
    assert_eq!(t.cursor_line_text().as_deref(), Some("[s"));
    // A hidden cursor means no line to speak of.
    t.process(b"\x1b[?25l");
    assert_eq!(t.cursor_line_text(), None);
}

mod fuzz {
    use proptest::prelude::*;

    use super::*;

    /// One well-formed chunk of terminal input: text, or an escape drawn from a curated set.
    /// Numeric params run past the grid size so clamping is exercised, not just in-range moves.
    fn chunk() -> impl Strategy<Value = String> {
        let n = 0u16..300;
        prop_oneof![
            "[ -~é世\u{1f600}]{0,24}",
            Just("\r\n".to_string()),
            Just("\n".to_string()),
            Just("\r".to_string()),
            Just("\t".to_string()),
            Just("\x08".to_string()),
            (n.clone(), n.clone()).prop_map(|(r, c)| format!("\x1b[{r};{c}H")),
            (
                n.clone(),
                prop::sample::select(vec!['A', 'B', 'C', 'D', 'G', 'd'])
            )
                .prop_map(|(k, d)| format!("\x1b[{k}{d}")),
            (
                n.clone(),
                prop::sample::select(vec!['L', 'M', '@', 'P', 'X', 'S', 'T'])
            )
                .prop_map(|(k, d)| format!("\x1b[{k}{d}")),
            (n.clone(), n.clone()).prop_map(|(t, b)| format!("\x1b[{t};{b}r")),
            (0u16..110).prop_map(|k| format!("\x1b[{k}m")),
            (0u16..256, 0u16..256, 0u16..256)
                .prop_map(|(r, g, b)| format!("\x1b[38;2;{r};{g};{b}m")),
            (0u8..4).prop_map(|k| format!("\x1b[{k}J")),
            (0u8..3).prop_map(|k| format!("\x1b[{k}K")),
            prop::sample::select(vec![
                "\x1b[?25l",
                "\x1b[?25h",
                "\x1b[?1049h",
                "\x1b[?1049l",
                "\x1b[?7l",
                "\x1b[?7h",
                "\x1b7",
                "\x1b8",
                "\x1bM",
                "\x1bD",
                "\x1bE",
                "\x1bc"
            ])
            .prop_map(str::to_string),
            "[ -~]{0,12}".prop_map(|t| format!("\x1b]0;{t}\x07")),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn process_never_panics_and_keeps_cursor_and_grid_in_bounds(
            cols in 2u16..60,
            rows in 1u16..30,
            chunks in prop::collection::vec(chunk(), 0..60),
        ) {
            let mut t = term(cols, rows);
            // Feed in two calls so a sequence can straddle the boundary, as real reads do.
            let bytes = chunks.concat().into_bytes();
            let (a, b) = bytes.split_at(bytes.len() / 2);
            t.process(a);
            t.process(b);

            prop_assert_eq!((t.cols(), t.rows()), (cols as usize, rows as usize));
            let point = t.term.grid().cursor.point;
            prop_assert!(point.line.0 >= 0 && (point.line.0 as usize) < rows as usize, "line {point:?}");
            prop_assert!(point.column.0 < cols as usize, "column {point:?}");
            // The public viewport view stays consistent with the configured size too.
            prop_assert_eq!(t.lines().len(), rows as usize);
            prop_assert!(t.lines().iter().all(|l| l.len() == cols as usize));
        }
    }
}
