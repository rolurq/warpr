use serde_json::{Value, json};

/// The size Herdr reports for every tab's layout area in a headless session.
const AREA_WIDTH: u16 = 120;
const AREA_HEIGHT: u16 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    /// The first pane takes the left part, the second the right.
    Right,
    /// The first pane takes the upper part, the second the lower.
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

/// How a tab's area is divided among its panes.
#[derive(Clone, Debug)]
pub enum PaneTree {
    Pane,
    Split {
        direction: SplitDirection,
        /// The share of the split's extent that goes to `first`.
        ratio: f32,
        first: Box<PaneTree>,
        second: Box<PaneTree>,
    },
}

impl PaneTree {
    pub fn pane() -> Self {
        Self::Pane
    }

    pub fn split(direction: SplitDirection, ratio: f32, first: PaneTree, second: PaneTree) -> Self {
        Self::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn pane_count(&self) -> usize {
        match self {
            Self::Pane => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }
}

/// One Herdr tab of a scripted workspace.
#[derive(Clone, Debug)]
pub struct TabScript {
    pub(crate) label: String,
    pub(crate) tree: PaneTree,
    pub(crate) focused_pane: usize,
    pub(crate) zoomed: bool,
}

impl TabScript {
    pub fn new(label: &str, tree: PaneTree) -> Self {
        Self {
            label: label.to_owned(),
            tree,
            focused_pane: 0,
            zoomed: false,
        }
    }

    /// Focuses the pane at `index` in the tree's left-to-right, top-to-bottom order; the first
    /// pane is focused otherwise.
    pub fn focused_pane(mut self, index: usize) -> Self {
        assert!(
            index < self.tree.pane_count(),
            "tab {:?} has no pane at index {index}",
            self.label
        );
        self.focused_pane = index;
        self
    }

    /// Zooms the focused pane, as Herdr's pane zoom does.
    pub fn zoomed(mut self) -> Self {
        self.zoomed = true;
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceScript {
    /// Herdr keeps a workspace's id for its whole life, so it is not derived from its position.
    pub(crate) id: usize,
    pub(crate) label: String,
    pub(crate) tabs: Vec<TabScript>,
    pub(crate) active_tab_label: Option<String>,
}

impl WorkspaceScript {
    fn active_tab_index(&self) -> Option<usize> {
        match &self.active_tab_label {
            Some(label) => Some(
                self.tabs
                    .iter()
                    .position(|tab| &tab.label == label)
                    .unwrap_or_else(|| {
                        panic!("workspace {:?} has no tab labelled {label:?}", self.label)
                    }),
            ),
            None => (!self.tabs.is_empty()).then_some(0),
        }
    }
}

/// What a [`crate::FakeSession`] reports about its Herdr session.
#[derive(Clone, Debug, Default)]
pub struct SessionScript {
    pub(crate) workspaces: Vec<WorkspaceScript>,
    pub(crate) focused_label: Option<String>,
    /// Ids are handed out in increasing order and never reused, as Herdr does.
    pub(crate) next_workspace_id: usize,
}

impl SessionScript {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a workspace, after those added before it. It has one Herdr tab holding one pane.
    pub fn workspace(self, label: &str) -> Self {
        self.workspace_with_tabs(label, [TabScript::new("1", PaneTree::pane())])
    }

    /// Adds a workspace with the given Herdr tabs, in order. The first tab is its active one
    /// unless [`Self::active_tab`] says otherwise.
    pub fn workspace_with_tabs(
        mut self,
        label: &str,
        tabs: impl IntoIterator<Item = TabScript>,
    ) -> Self {
        self.next_workspace_id += 1;
        self.workspaces.push(WorkspaceScript {
            id: self.next_workspace_id,
            label: label.to_owned(),
            tabs: tabs.into_iter().collect(),
            active_tab_label: None,
        });
        self
    }

    /// Makes the tab labelled `tab_label` the one Herdr has focused in the workspace labelled
    /// `workspace_label`.
    pub fn active_tab(mut self, workspace_label: &str, tab_label: &str) -> Self {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.label == workspace_label)
            .unwrap_or_else(|| panic!("no workspace labelled {workspace_label:?}"));
        workspace.active_tab_label = Some(tab_label.to_owned());
        self
    }

    /// Removes the workspace labelled `label`, leaving the others' ids untouched.
    pub fn without_workspace(mut self, label: &str) -> Self {
        self.workspaces.retain(|workspace| workspace.label != label);
        if self.focused_label.as_deref() == Some(label) {
            self.focused_label = None;
        }
        self
    }

    /// Renames the workspace labelled `from`, keeping its id.
    pub fn renamed_workspace(mut self, from: &str, to: &str) -> Self {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.label == from)
            .unwrap_or_else(|| panic!("no workspace labelled {from:?}"));
        workspace.label = to.to_owned();
        if self.focused_label.as_deref() == Some(from) {
            self.focused_label = Some(to.to_owned());
        }
        self
    }

    /// Focuses the workspace labelled `label`; the first workspace is focused otherwise.
    pub fn focused(mut self, label: &str) -> Self {
        self.focused_label = Some(label.to_owned());
        self
    }

    fn focused_index(&self) -> Option<usize> {
        match &self.focused_label {
            Some(label) => Some(
                self.workspaces
                    .iter()
                    .position(|candidate| &candidate.label == label)
                    .unwrap_or_else(|| panic!("no workspace labelled {label:?} to focus")),
            ),
            None => (!self.workspaces.is_empty()).then_some(0),
        }
    }

    /// The `snapshot` object of a `session.snapshot` result.
    ///
    /// Like Herdr, `focused` on a tab or pane is relative to its container: it marks the
    /// workspace's active tab and the tab's focused pane, whether or not the workspace is focused.
    pub(crate) fn snapshot(&self) -> Value {
        let focused = self.focused_index();
        let mut workspaces = Vec::new();
        let mut tabs = Vec::new();
        let mut panes = Vec::new();
        let mut layouts = Vec::new();
        let mut focused_tab_id = Value::Null;
        let mut focused_pane_id = Value::Null;

        for (index, workspace) in self.workspaces.iter().enumerate() {
            let number = index + 1;
            let workspace_id = format!("w{}", workspace.id);
            let active_tab = workspace.active_tab_index();
            let tab_id_of = |tab_index: usize| format!("{workspace_id}:t{}", tab_index + 1);
            let mut next_pane_number = 1;

            for (tab_index, tab) in workspace.tabs.iter().enumerate() {
                let tab_id = tab_id_of(tab_index);
                let pane_ids: Vec<String> = (0..tab.tree.pane_count())
                    .map(|offset| format!("{workspace_id}:p{}", next_pane_number + offset))
                    .collect();
                let is_active = active_tab == Some(tab_index);
                let tab_focused_pane = &pane_ids[tab.focused_pane];

                let mut geometry = Geometry::default();
                geometry.place(&tab.tree, Rect::area(), "root");

                tabs.push(json!({
                    "tab_id": tab_id,
                    "workspace_id": workspace_id,
                    "number": tab_index + 1,
                    "label": tab.label,
                    "focused": is_active,
                    "pane_count": pane_ids.len(),
                    "agent_status": "unknown",
                }));
                for (pane_index, pane_id) in pane_ids.iter().enumerate() {
                    let terminal_number = next_pane_number + pane_index;
                    panes.push(json!({
                        "pane_id": pane_id,
                        "terminal_id": terminal_id(workspace.id, terminal_number),
                        "workspace_id": workspace_id,
                        "tab_id": tab_id,
                        "focused": pane_id == tab_focused_pane,
                        "cwd": "/",
                        "foreground_cwd": "/",
                        "agent_status": "unknown",
                        "scroll": {
                            "offset_from_bottom": 0,
                            "max_offset_from_bottom": 0,
                            "viewport_rows": AREA_HEIGHT,
                        },
                        "revision": 0,
                    }));
                }
                layouts.push(json!({
                    "workspace_id": workspace_id,
                    "tab_id": tab_id,
                    "zoomed": tab.zoomed,
                    "area": Rect::area().json(),
                    "focused_pane_id": tab_focused_pane,
                    "panes": geometry
                        .pane_rects
                        .iter()
                        .zip(&pane_ids)
                        .map(|(rect, pane_id)| json!({
                            "pane_id": pane_id,
                            "focused": pane_id == tab_focused_pane,
                            "rect": rect.json(),
                        }))
                        .collect::<Vec<_>>(),
                    "splits": geometry.splits,
                }));

                if focused == Some(index) && is_active {
                    focused_tab_id = json!(tab_id);
                    focused_pane_id = json!(tab_focused_pane);
                }
                next_pane_number += pane_ids.len();
            }

            workspaces.push(json!({
                "workspace_id": workspace_id,
                "number": number,
                "label": workspace.label,
                "focused": focused == Some(index),
                "pane_count": next_pane_number - 1,
                "tab_count": workspace.tabs.len(),
                "active_tab_id": active_tab.map(tab_id_of),
                "agent_status": "unknown",
            }));
        }

        json!({
            "version": "0.9.3",
            "protocol": 22,
            "focused_workspace_id": focused.map(|index| format!("w{}", self.workspaces[index].id)),
            "focused_tab_id": focused_tab_id,
            "focused_pane_id": focused_pane_id,
            "workspaces": workspaces,
            "tabs": tabs,
            "panes": panes,
            "layouts": layouts,
            "agents": [],
        })
    }
}

fn terminal_id(workspace_id: usize, pane_number: usize) -> String {
    if pane_number == 1 {
        format!("term_fake{workspace_id}")
    } else {
        format!("term_fake{workspace_id}p{pane_number}")
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl Rect {
    fn area() -> Self {
        Self {
            x: 0,
            y: 0,
            width: AREA_WIDTH,
            height: AREA_HEIGHT,
        }
    }

    fn json(self) -> Value {
        json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }

    /// Splits at `ratio` of the extent along `direction`, rounding to whole cells as Herdr does.
    fn divide(self, direction: SplitDirection, ratio: f32) -> (Self, Self) {
        match direction {
            SplitDirection::Right => {
                let first = (f32::from(self.width) * ratio).round() as u16;
                (
                    Self {
                        width: first,
                        ..self
                    },
                    Self {
                        x: self.x + first,
                        width: self.width - first,
                        ..self
                    },
                )
            }
            SplitDirection::Down => {
                let first = (f32::from(self.height) * ratio).round() as u16;
                (
                    Self {
                        height: first,
                        ..self
                    },
                    Self {
                        y: self.y + first,
                        height: self.height - first,
                        ..self
                    },
                )
            }
        }
    }
}

/// The rects of a tree's panes in tree order, and the splits between them in the pre-order and
/// with the ids Herdr reports (`split_<depth>_<path of 0/1 turns>`, `root` for the root).
#[derive(Default)]
struct Geometry {
    pane_rects: Vec<Rect>,
    splits: Vec<Value>,
}

impl Geometry {
    fn place(&mut self, tree: &PaneTree, rect: Rect, path: &str) {
        match tree {
            PaneTree::Pane => self.pane_rects.push(rect),
            PaneTree::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let turns = if path == "root" { "" } else { path };
                let depth = turns.len();
                self.splits.push(json!({
                    "id": format!("split_{depth}_{path}"),
                    "direction": direction.wire_name(),
                    "ratio": ratio,
                    "rect": rect.json(),
                }));
                let (first_rect, second_rect) = rect.divide(*direction, *ratio);
                self.place(first, first_rect, &format!("{turns}0"));
                self.place(second, second_rect, &format!("{turns}1"));
            }
        }
    }
}
