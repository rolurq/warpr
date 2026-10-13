use serde_json::{Value, json};

/// What a [`crate::FakeSession`] reports about its Herdr session.
#[derive(Clone, Debug, Default)]
pub struct SessionScript {
    workspace_labels: Vec<String>,
    focused_label: Option<String>,
}

impl SessionScript {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a workspace, after those added before it. Each has one Herdr tab holding one pane.
    pub fn workspace(mut self, label: &str) -> Self {
        self.workspace_labels.push(label.to_owned());
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
                self.workspace_labels
                    .iter()
                    .position(|candidate| candidate == label)
                    .unwrap_or_else(|| panic!("no workspace labelled {label:?} to focus")),
            ),
            None => (!self.workspace_labels.is_empty()).then_some(0),
        }
    }

    /// The `snapshot` object of a `session.snapshot` result.
    pub(crate) fn snapshot(&self) -> Value {
        let focused = self.focused_index();
        let mut workspaces = Vec::new();
        let mut tabs = Vec::new();
        let mut panes = Vec::new();
        let mut layouts = Vec::new();

        for (index, label) in self.workspace_labels.iter().enumerate() {
            let number = index + 1;
            let workspace_id = format!("w{number}");
            let tab_id = format!("{workspace_id}:t1");
            let pane_id = format!("{workspace_id}:p1");
            let is_focused = focused == Some(index);

            workspaces.push(json!({
                "workspace_id": workspace_id,
                "number": number,
                "label": label,
                "focused": is_focused,
                "pane_count": 1,
                "tab_count": 1,
                "active_tab_id": tab_id,
                "agent_status": "unknown",
            }));
            tabs.push(json!({
                "tab_id": tab_id,
                "workspace_id": workspace_id,
                "number": 1,
                "label": "1",
                "focused": is_focused,
                "pane_count": 1,
                "agent_status": "unknown",
            }));
            panes.push(json!({
                "pane_id": pane_id,
                "terminal_id": format!("term_fake{number}"),
                "workspace_id": workspace_id,
                "tab_id": tab_id,
                "focused": is_focused,
                "cwd": "/",
                "foreground_cwd": "/",
                "agent_status": "unknown",
                "scroll": {
                    "offset_from_bottom": 0,
                    "max_offset_from_bottom": 0,
                    "viewport_rows": 40,
                },
                "revision": 0,
            }));
            layouts.push(json!({
                "workspace_id": workspace_id,
                "tab_id": tab_id,
                "zoomed": false,
                "area": { "x": 0, "y": 0, "width": 120, "height": 40 },
                "focused_pane_id": pane_id,
                "panes": [{
                    "pane_id": pane_id,
                    "focused": true,
                    "rect": { "x": 0, "y": 0, "width": 120, "height": 40 },
                }],
                "splits": [],
            }));
        }

        let focused_id = |kind: &str| {
            focused
                .map(|index| format!("w{}{kind}", index + 1))
                .map_or(Value::Null, Value::String)
        };
        json!({
            "version": "0.9.3",
            "protocol": 22,
            "focused_workspace_id": focused_id(""),
            "focused_tab_id": focused_id(":t1"),
            "focused_pane_id": focused_id(":p1"),
            "workspaces": workspaces,
            "tabs": tabs,
            "panes": panes,
            "layouts": layouts,
            "agents": [],
        })
    }
}
