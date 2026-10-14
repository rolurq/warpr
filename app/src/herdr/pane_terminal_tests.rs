use herdr_client::PaneFrame;
use warp_terminal::model::grid::cell::Flags;

use super::*;

fn frame(seq: u64, full: bool, width: u16, height: u16, bytes: &str) -> PaneFrame {
    PaneFrame {
        seq,
        full,
        width,
        height,
        bytes: bytes.as_bytes().to_vec(),
    }
}
/// The screen's lines without the blank rows at its bottom, which the grid's text may or may
/// not include.
fn lines(terminal: &PaneTerminal) -> Vec<String> {
    let mut lines: Vec<_> = terminal
        .text()
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

#[test]
fn a_full_frame_paints_the_whole_screen() {
    let mut terminal = PaneTerminal::new(12, 3);

    terminal.apply(&frame(
        1,
        true,
        12,
        3,
        "\x1b[2J\x1b[1;1Hfirst line\x1b[2;1Hsecond",
    ));

    assert_eq!(lines(&terminal), ["first line", "second"]);
}

#[test]
fn an_incremental_frame_changes_only_what_it_paints() {
    let mut terminal = PaneTerminal::new(12, 3);
    terminal.apply(&frame(
        1,
        true,
        12,
        3,
        "\x1b[2J\x1b[1;1Hfirst line\x1b[2;1Hsecond",
    ));

    terminal.apply(&frame(
        2,
        false,
        12,
        3,
        "\x1b[2;1H\x1b[2Kthird\x1b[1;1HFIRST",
    ));

    assert_eq!(lines(&terminal), ["FIRST line", "third"]);
}

#[test]
fn a_full_frame_mid_stream_replaces_what_was_there() {
    let mut terminal = PaneTerminal::new(12, 3);
    terminal.apply(&frame(
        1,
        true,
        12,
        3,
        "\x1b[2J\x1b[1;1Hold old old\x1b[3;1Hbottom",
    ));

    terminal.apply(&frame(5, true, 12, 3, "\x1b[2J\x1b[1;1Hnew"));

    assert_eq!(lines(&terminal), ["new"]);
}

#[test]
fn a_frame_for_another_grid_resizes_the_screen() {
    let mut terminal = PaneTerminal::new(12, 3);
    terminal.apply(&frame(1, true, 12, 3, "\x1b[2J\x1b[1;1Hsmall"));

    terminal.apply(&frame(
        2,
        true,
        20,
        4,
        "\x1b[2J\x1b[1;1Hlarger\x1b[4;1Hlast",
    ));

    assert_eq!((terminal.cols(), terminal.rows()), (20, 4));
    assert_eq!(lines(&terminal), ["larger", "", "", "last"]);
}

#[test]
fn colours_and_attributes_survive_into_the_cells() {
    let mut terminal = PaneTerminal::new(12, 1);

    terminal.apply(&frame(
        1,
        true,
        12,
        1,
        "\x1b[2J\x1b[1;1H\x1b[1;31mR\x1b[0mp",
    ));

    let row = terminal.screen().grid_handler().row(0).expect("row 0");
    let styled = row.get(0).expect("cell 0");
    let plain = row.get(1).expect("cell 1");
    assert_eq!((styled.c, plain.c), ('R', 'p'));
    assert!(styled.flags.contains(Flags::BOLD));
    assert!(!plain.flags.contains(Flags::BOLD));
    assert_ne!(styled.fg, plain.fg);
}

#[test]
fn wide_characters_occupy_two_cells() {
    let mut terminal = PaneTerminal::new(12, 1);

    terminal.apply(&frame(1, true, 12, 1, "\x1b[2J\x1b[1;1H漢字\x1b[1;5HX"));

    assert_eq!(lines(&terminal), ["漢字X"]);
}

#[test]
fn the_cursor_sits_where_the_frame_left_it_with_the_shape_the_program_chose() {
    let mut terminal = PaneTerminal::new(12, 3);

    terminal.apply(&frame(
        1,
        true,
        12,
        3,
        "\x1b[?25l\x1b[2J\x1b[1;1Hab\x1b[2;5H\x1b[6 q\x1b[?25h",
    ));

    assert_eq!(
        terminal.cursor(),
        Some(PaneCursor {
            row: 1,
            col: 4,
            shape: CursorShape::Beam
        })
    );
}

#[test]
fn a_cursor_the_program_hides_is_not_reported_until_it_shows_it_again() {
    let mut terminal = PaneTerminal::new(12, 3);
    terminal.apply(&frame(1, true, 12, 3, "\x1b[2J\x1b[1;1Hab\x1b[?25l"));

    assert_eq!(terminal.cursor(), None);

    terminal.apply(&frame(2, false, 12, 3, "\x1b[3;3H\x1b[?25h"));
    assert_eq!(
        terminal.cursor().map(|cursor| (cursor.row, cursor.col)),
        Some((2, 2))
    );
}
