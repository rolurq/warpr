use std::cmp::Reverse;
use std::path::PathBuf;

use serde::Deserialize;

use crate::{PaneDirection, WorkspaceId};

/// Herdr's identifier for a Herdr tab, stable for the life of the tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct HerdrTabId(String);

impl HerdrTabId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HerdrTabId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

/// Herdr's identifier for a pane, stable for the life of the pane.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct PaneId(String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PaneId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

/// A rectangle in the character cells of a Herdr layout area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct CellRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PresentedPane {
    #[serde(rename = "pane_id")]
    pub id: PaneId,
    pub rect: CellRect,
    /// Whether the pane is the one Herdr has focused in its Herdr tab.
    pub focused: bool,
    /// The working directory Herdr reports for the pane; `None` when it reports none. The
    /// layout carries no `cwd`: it is filled in from the snapshot's pane list.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// The name given to the pane in Herdr; `None` when it has none. Like `cwd`, it is filled in
    /// from the snapshot's pane list.
    #[serde(default)]
    pub name: Option<String>,
    /// The title the pane's program has set, as Herdr reports it without control characters;
    /// `None` when it reports none or a blank one. Like `cwd`, it is filled in from the
    /// snapshot's pane list.
    #[serde(default)]
    pub title: Option<String>,
}

/// How Herdr lays out the panes of one Herdr tab. Herdr owns the geometry; nothing here is
/// computed by the client.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PaneLayout {
    /// The space the pane rects are measured in.
    pub area: CellRect,
    /// In Herdr's order. Rects are those of the unzoomed layout even while a pane is zoomed.
    pub panes: Vec<PresentedPane>,
    pub zoomed: bool,
    #[serde(rename = "focused_pane_id")]
    pub focused_pane: PaneId,
}

impl PaneLayout {
    /// The panes to draw and where. While the layout is zoomed Herdr shows only its focused
    /// pane, filling the whole area.
    pub fn frames(&self) -> Vec<PresentedPane> {
        if !self.zoomed {
            return self.panes.clone();
        }
        self.panes
            .iter()
            .filter(|pane| pane.id == self.focused_pane)
            .map(|pane| PresentedPane {
                rect: self.area,
                ..pane.clone()
            })
            .collect()
    }

    /// The pane drawn next to `pane` in `direction`: of the [frames](Self::frames) wholly on that
    /// side of it and level with it, the nearest, then the one sharing most of its edge, then the
    /// first in Herdr's order. `None` when there is none or `pane` is not drawn.
    pub fn neighbor(&self, pane: &PaneId, direction: PaneDirection) -> Option<PresentedPane> {
        let frames = self.frames();
        let from = frames.iter().find(|candidate| candidate.id == *pane)?.rect;
        let (from_x, from_y) = (span(from.x, from.width), span(from.y, from.height));
        frames
            .into_iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.id != *pane)
            .filter_map(|(order, candidate)| {
                let rect = candidate.rect;
                let (x, y) = (span(rect.x, rect.width), span(rect.y, rect.height));
                let (distance, shared) = match direction {
                    PaneDirection::Left => (from_x.0.checked_sub(x.1)?, overlap(from_y, y)),
                    PaneDirection::Right => (x.0.checked_sub(from_x.1)?, overlap(from_y, y)),
                    PaneDirection::Up => (from_y.0.checked_sub(y.1)?, overlap(from_x, x)),
                    PaneDirection::Down => (y.0.checked_sub(from_y.1)?, overlap(from_x, x)),
                };
                (shared > 0).then_some(((distance, Reverse(shared), order), candidate))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, candidate)| candidate)
    }
}

/// The cells from `start` up to, not including, `start + length`.
fn span(start: u16, length: u16) -> (u32, u32) {
    (u32::from(start), u32::from(start) + u32::from(length))
}

/// How many cells two spans share.
fn overlap((a_start, a_end): (u32, u32), (b_start, b_end): (u32, u32)) -> u32 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

/// A Herdr tab as presented in the strip of its workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedHerdrTab {
    pub id: HerdrTabId,
    pub label: String,
    /// `None` when Herdr reported no layout for the tab.
    pub layout: Option<PaneLayout>,
}

/// The part of a `session.snapshot` tab this client presents.
#[derive(Deserialize)]
pub(crate) struct TabInfo {
    pub(crate) tab_id: HerdrTabId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) label: String,
}

/// A tab's layout as `session.snapshot` reports it, tagged with the tab it belongs to.
#[derive(Deserialize)]
pub(crate) struct TabLayout {
    pub(crate) tab_id: HerdrTabId,
    #[serde(flatten)]
    pub(crate) layout: PaneLayout,
}

#[cfg(test)]
#[path = "tabs_tests.rs"]
mod tests;
