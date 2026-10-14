use std::collections::{HashMap, HashSet};

use crate::{PaneId, WorkspaceId};

/// A place a front-end shows panes in, such as one of its windows. The front-end picks the
/// numbers; they only have to differ between its windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewerId(pub u64);

/// Which panes the user can see, as far as the front-end's windows tell.
///
/// Each window reports separately what it shows and which workspace it has selected, and a pane
/// counts as seen only when it is shown by the selected workspace of the window that has focus.
/// Because every report is kept per window, the result never depends on which window reported
/// last.
#[derive(Debug, Default)]
pub struct ViewedPanes {
    /// The panes each workspace view shows, whether or not it is selected right now.
    shown: HashMap<(ViewerId, WorkspaceId), Vec<PaneId>>,
    selected: HashMap<ViewerId, WorkspaceId>,
}

impl ViewedPanes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the panes `viewer`'s view of `workspace` shows, replacing what it showed before.
    pub fn show(
        &mut self,
        viewer: ViewerId,
        workspace: WorkspaceId,
        panes: impl IntoIterator<Item = PaneId>,
    ) {
        self.shown
            .insert((viewer, workspace), panes.into_iter().collect());
    }

    /// Records which workspace `viewer` has selected, or that it has none (it shows something
    /// else, such as a message instead of a workspace).
    pub fn select(&mut self, viewer: ViewerId, workspace: Option<WorkspaceId>) {
        match workspace {
            Some(workspace) => {
                self.selected.insert(viewer, workspace);
            }
            None => {
                self.selected.remove(&viewer);
            }
        }
    }

    /// Forgets what views showed, as when the session they showed is no longer followed.
    pub fn clear(&mut self) {
        self.shown.clear();
    }

    /// The panes in view when `focused` is the viewer with the user's attention; none when no
    /// viewer has it.
    pub fn viewed(&self, focused: Option<ViewerId>) -> HashSet<PaneId> {
        let Some(focused) = focused else {
            return HashSet::new();
        };
        let Some(workspace) = self.selected.get(&focused) else {
            return HashSet::new();
        };
        self.shown
            .get(&(focused, workspace.clone()))
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }
}
