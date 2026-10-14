use herdr_client::{PaneInput, ScrollDirection};
use warpui::geometry::vector::Vector2F;
use warpui::keymap::Keystroke;

/// Keys Herdr's `pane send-keys` accepts, which it encodes for the modes the program in the pane
/// set, such as application cursor keys.
const HERDR_KEYS: &[&str] = &[
    "up",
    "down",
    "left",
    "right",
    "enter",
    "tab",
    "backspace",
    "escape",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// Navigation keys `pane send-keys` rejects, with the final byte or number of their xterm
/// sequences. They go as those sequences in their normal mode.
const RAW_KEYS: &[(&str, Sequence)] = &[
    ("home", Sequence::Final(b'H')),
    ("end", Sequence::Final(b'F')),
    ("insert", Sequence::Tilde(2)),
    ("delete", Sequence::Tilde(3)),
    ("pageup", Sequence::Tilde(5)),
    ("pagedown", Sequence::Tilde(6)),
];

enum Sequence {
    /// `CSI final`, or `CSI 1 ; modifier final` with modifiers.
    Final(u8),
    /// `CSI number ~`, or `CSI number ; modifier ~` with modifiers.
    Tilde(u8),
}

/// What `keystroke` sends to the program in a pane, or `None` for a keystroke the pane has no use
/// for. Printable text is not translated here: it arrives separately as typed characters.
pub(super) fn keystroke_input(keystroke: &Keystroke) -> Option<PaneInput> {
    if keystroke.cmd {
        return None;
    }
    let key = keystroke.key.as_str();
    let alt = keystroke.alt || keystroke.meta;
    let unmodified = !keystroke.ctrl && !alt && !keystroke.shift;
    // These encode the same in every mode, and as text they stay in order with the typing around
    // them, which a separate `send-keys` process does not.
    let editing_key = match key {
        "enter" | "numpadenter" => Some("\r"),
        "tab" => Some("\t"),
        "backspace" => Some("\x7f"),
        "escape" => Some("\x1b"),
        _ => None,
    };
    if let (true, Some(bytes)) = (unmodified, editing_key) {
        return Some(PaneInput::Text(bytes.to_owned()));
    }
    if HERDR_KEYS.contains(&key) || key == "numpadenter" {
        let name = match key {
            "escape" => "esc",
            "numpadenter" => "enter",
            key => key,
        };
        let modifiers = [
            (keystroke.ctrl, "ctrl+"),
            (alt, "alt+"),
            (keystroke.shift, "shift+"),
        ]
        .into_iter()
        .filter_map(|(held, prefix)| held.then_some(prefix))
        .collect::<String>();
        return Some(PaneInput::Key(format!("{modifiers}{name}")));
    }
    if let Some((_, sequence)) = RAW_KEYS.iter().find(|(name, _)| *name == key) {
        let modifier =
            1 + u8::from(keystroke.shift) + 2 * u8::from(alt) + 4 * u8::from(keystroke.ctrl);
        let sequence = match (sequence, modifier) {
            (Sequence::Final(last), 1) => format!("\x1b[{}", char::from(*last)),
            (Sequence::Final(last), modifier) => format!("\x1b[1;{modifier}{}", char::from(*last)),
            (Sequence::Tilde(number), 1) => format!("\x1b[{number}~"),
            (Sequence::Tilde(number), modifier) => format!("\x1b[{number};{modifier}~"),
        };
        return Some(PaneInput::Text(sequence));
    }

    let mut chars = key.chars();
    let (Some(character), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if keystroke.ctrl && !alt && !keystroke.shift {
        // Herdr's key path refuses some control keys, such as ctrl+d, so these go as their bytes.
        return control_byte(character).map(|byte| PaneInput::Text(char::from(byte).to_string()));
    }
    (alt && !keystroke.ctrl).then(|| PaneInput::Text(format!("\x1b{character}")))
}

/// Whether `keystroke` is the platform's paste shortcut: Cmd+V, or Ctrl+Shift+V where Ctrl+V
/// belongs to the program in the pane.
pub(super) fn is_paste(keystroke: &Keystroke) -> bool {
    let modifiers = (
        keystroke.cmd,
        keystroke.ctrl,
        keystroke.alt,
        keystroke.shift,
    );
    keystroke.key.eq_ignore_ascii_case("v")
        && matches!(
            modifiers,
            (true, false, false, false) | (false, true, false, true)
        )
}

fn control_byte(character: char) -> Option<u8> {
    match character {
        'a'..='z' => Some(character as u8 - b'a' + 1),
        ' ' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        _ => None,
    }
}

/// The zero-based cell of a grid drawn with cells of `cell` pixels that `offset` pixels from its
/// top left falls in, kept inside a grid of `cols` by `rows`.
pub(super) fn cell_at(offset: Vector2F, cell: Vector2F, cols: u16, rows: u16) -> (u16, u16) {
    let index = |pixels: f32, cell: f32, count: u16| {
        if cell <= 0. {
            return 0;
        }
        ((pixels / cell).floor().max(0.) as u16).min(count.saturating_sub(1))
    };
    (
        index(offset.x(), cell.x(), cols),
        index(offset.y(), cell.y(), rows),
    )
}

/// The lines a scroll wheel event scrolls, or `None` when it does not move. A precise delta is in
/// pixels and a coarse one already in lines.
pub(super) fn scroll_lines(
    delta_y: f32,
    precise: bool,
    cell_height: f32,
) -> Option<(ScrollDirection, u16)> {
    let lines = if precise {
        if cell_height <= 0. {
            return None;
        }
        delta_y.abs() / cell_height
    } else {
        delta_y.abs()
    };
    let lines = lines.round().min(f32::from(u16::MAX)) as u16;
    if lines == 0 {
        return None;
    }
    let direction = if delta_y > 0. {
        ScrollDirection::Up
    } else {
        ScrollDirection::Down
    };
    Some((direction, lines))
}

#[cfg(test)]
#[path = "pane_input_tests.rs"]
mod tests;
