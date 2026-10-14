use warpui::geometry::vector::vec2f;

use super::*;

fn input(keystroke: &str) -> Option<PaneInput> {
    keystroke_input(&Keystroke::parse(keystroke).expect("valid keystroke"))
}

fn key(name: &str) -> Option<PaneInput> {
    Some(PaneInput::Key(name.to_owned()))
}

fn text(text: &str) -> Option<PaneInput> {
    Some(PaneInput::Text(text.to_owned()))
}

#[test]
fn keys_whose_encoding_depends_on_the_programs_modes_go_by_herdrs_names() {
    assert_eq!(input("up"), key("up"));
    assert_eq!(input("f5"), key("f5"));
    assert_eq!(input("ctrl-left"), key("ctrl+left"));
    assert_eq!(input("shift-tab"), key("shift+tab"));
    assert_eq!(input("ctrl-enter"), key("ctrl+enter"));
}

#[test]
fn editing_keys_go_as_text_so_they_stay_in_order_with_typing() {
    assert_eq!(input("enter"), text("\r"));
    assert_eq!(input("numpadenter"), text("\r"));
    assert_eq!(input("tab"), text("\t"));
    assert_eq!(input("backspace"), text("\x7f"));
    assert_eq!(input("escape"), text("\x1b"));
}

#[test]
fn navigation_keys_herdr_rejects_go_as_xterm_sequences() {
    assert_eq!(input("home"), text("\x1b[H"));
    assert_eq!(input("end"), text("\x1b[F"));
    assert_eq!(input("delete"), text("\x1b[3~"));
    assert_eq!(input("pageup"), text("\x1b[5~"));
    assert_eq!(input("ctrl-pagedown"), text("\x1b[6;5~"));
    assert_eq!(input("shift-home"), text("\x1b[1;2H"));
}

#[test]
fn control_letters_go_as_their_bytes() {
    assert_eq!(input("ctrl-c"), text("\x03"));
    assert_eq!(input("ctrl-d"), text("\x04"));
    assert_eq!(input("ctrl-space"), text("\0"));
}

#[test]
fn alt_prefixes_escape_and_app_shortcuts_are_left_alone() {
    assert_eq!(input("alt-b"), text("\x1bb"));
    assert_eq!(input("cmd-t"), None);
    assert_eq!(input("cmd-up"), None);
}

#[test]
fn the_platform_paste_shortcut_is_recognised_and_plain_ctrl_v_is_the_programs() {
    let is_paste = |keystroke| is_paste(&Keystroke::parse(keystroke).expect("valid keystroke"));

    assert!(is_paste("cmd-v"));
    assert!(is_paste("ctrl-shift-V"));
    assert!(!is_paste("ctrl-v"));
    assert!(!is_paste("cmd-shift-V"));
}

#[test]
fn printable_keys_are_left_to_the_typed_characters_event() {
    assert_eq!(input("a"), None);
    assert_eq!(input("shift-1"), None);
}

#[test]
fn a_pointer_falls_in_the_cell_under_it_and_stays_inside_the_grid() {
    let cell = vec2f(10., 20.);

    assert_eq!(cell_at(vec2f(0., 0.), cell, 80, 24), (0, 0));
    assert_eq!(cell_at(vec2f(19.9, 39.9), cell, 80, 24), (1, 1));
    assert_eq!(cell_at(vec2f(-5., -5.), cell, 80, 24), (0, 0));
    assert_eq!(cell_at(vec2f(5000., 5000.), cell, 80, 24), (79, 23));
}

#[test]
fn a_trackpad_scrolls_by_cell_heights_and_a_wheel_by_lines() {
    assert_eq!(scroll_lines(40., true, 20.), Some((ScrollDirection::Up, 2)));
    assert_eq!(
        scroll_lines(-3., false, 20.),
        Some((ScrollDirection::Down, 3))
    );
    assert_eq!(scroll_lines(4., true, 20.), None);
}
