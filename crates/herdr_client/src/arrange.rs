use serde_json::json;

use crate::{ConnectionFailure, PaneId, SessionTarget, socket};

/// Where a split puts the new pane, next to the pane it splits. Herdr splits no other way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Right,
    Down,
}

impl SplitDirection {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

/// A direction within the layout of a Herdr tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

impl PaneDirection {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// Asks the session of `target` to split `pane`, putting a new pane in `direction` of it. Herdr's
/// focus stays where it is.
///
/// Like every request here, nothing is returned to present: the change reaches a front-end
/// through the session's own state, and a refused request is [`ConnectionFailure::Rejected`].
pub fn split_pane(
    target: &SessionTarget,
    pane: &PaneId,
    direction: SplitDirection,
) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "pane.split",
        json!({ "target_pane_id": pane.as_str(), "direction": direction.wire_name() }),
    )
    .map(drop)
}

/// Asks the session of `target` to zoom `pane` in its Herdr tab, or to unzoom the tab when
/// `zoomed` is false. Herdr focuses the pane it zooms, and does not zoom a tab's only pane.
pub fn set_pane_zoom(
    target: &SessionTarget,
    pane: &PaneId,
    zoomed: bool,
) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "pane.zoom",
        json!({ "pane_id": pane.as_str(), "mode": if zoomed { "on" } else { "off" } }),
    )
    .map(drop)
}

/// Asks the session of `target` to move the divider of the nearest split around `pane` that runs
/// across `direction`, one step in that direction (Herdr's step is 5% of the split).
pub fn resize_pane(
    target: &SessionTarget,
    pane: &PaneId,
    direction: PaneDirection,
) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "pane.resize",
        json!({ "pane_id": pane.as_str(), "direction": direction.wire_name() }),
    )
    .map(drop)
}

#[cfg(test)]
#[path = "arrange_tests.rs"]
mod tests;
