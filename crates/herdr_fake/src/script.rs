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

    /// The tree without its pane at `index` in tree order: the sibling takes the freed space.
    /// `None` when that pane is the whole tree.
    fn without_pane(self, index: usize) -> Option<PaneTree> {
        match self {
            Self::Pane => None,
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first_count = first.pane_count();
                if index < first_count {
                    Some(match first.without_pane(index) {
                        Some(first) => Self::split(direction, ratio, first, *second),
                        None => *second,
                    })
                } else {
                    Some(match second.without_pane(index - first_count) {
                        Some(second) => Self::split(direction, ratio, *first, second),
                        None => *first,
                    })
                }
            }
        }
    }
}

/// An agent state as Herdr reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentState {
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

/// An agent Herdr recognizes in a pane.
#[derive(Clone, Debug)]
pub struct AgentScript {
    pub(crate) kind: String,
    pub(crate) name: Option<String>,
    pub(crate) state: AgentState,
}

impl AgentScript {
    pub fn new(kind: &str, state: AgentState) -> Self {
        Self {
            kind: kind.to_owned(),
            name: None,
            state,
        }
    }

    /// Gives the agent a live name, as `agent.rename` does. Herdr reports none otherwise.
    pub fn named(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }
}

/// One Herdr tab of a scripted workspace.
#[derive(Clone, Debug)]
pub struct TabScript {
    pub(crate) label: String,
    pub(crate) tree: PaneTree,
    pub(crate) focused_pane: usize,
    pub(crate) zoomed: bool,
    /// By pane index in the tree's order.
    pub(crate) agents: Vec<(usize, AgentScript)>,
    /// By pane index in the tree's order; `None` is a pane Herdr reports no `cwd` for. Panes
    /// without an entry report `/`.
    pub(crate) cwds: Vec<(usize, Option<String>)>,
    /// By pane index in the tree's order; panes without an entry have no name.
    pub(crate) pane_labels: Vec<(usize, String)>,
    /// By pane index in the tree's order; panes without an entry report no terminal title.
    pub(crate) terminal_titles: Vec<(usize, String)>,
    /// The Herdr pane number of each pane in the tree's order. Empty until a pane of the
    /// workspace is closed: numbers are derived from position until then, and fixed from then on
    /// so that closing a pane does not renumber the others, as in Herdr.
    pub(crate) pane_numbers: Vec<usize>,
    /// The Herdr tab number, or zero until a tab of the workspace is closed: numbers are derived
    /// from position until then, and fixed from then on so that closing a tab does not renumber
    /// the others, as in Herdr.
    pub(crate) number: usize,
}

impl TabScript {
    pub fn new(label: &str, tree: PaneTree) -> Self {
        Self {
            label: label.to_owned(),
            tree,
            focused_pane: 0,
            zoomed: false,
            agents: Vec::new(),
            terminal_titles: Vec::new(),
            cwds: Vec::new(),
            pane_labels: Vec::new(),
            pane_numbers: Vec::new(),
            number: 0,
        }
    }

    /// Names the pane at `index` in the tree's order, as `pane.rename` does.
    pub fn pane_label(mut self, index: usize, label: &str) -> Self {
        assert!(
            index < self.tree.pane_count(),
            "tab {:?} has no pane at index {index}",
            self.label
        );
        self.pane_labels.retain(|(existing, _)| *existing != index);
        self.pane_labels.push((index, label.to_owned()));
        self
    }

    fn label_at(&self, index: usize) -> Option<&str> {
        self.pane_labels
            .iter()
            .find(|(existing, _)| *existing == index)
            .map(|(_, label)| label.as_str())
    }

    /// Gives the pane at `index` in the tree's order the title its program set, as Herdr
    /// reports it in `terminal_title` and `terminal_title_stripped`.
    pub fn terminal_title(mut self, index: usize, title: &str) -> Self {
        assert!(
            index < self.tree.pane_count(),
            "tab {:?} has no pane at index {index}",
            self.label
        );
        self.terminal_titles
            .retain(|(existing, _)| *existing != index);
        self.terminal_titles.push((index, title.to_owned()));
        self
    }

    fn terminal_title_at(&self, index: usize) -> Option<&str> {
        self.terminal_titles
            .iter()
            .find(|(existing, _)| *existing == index)
            .map(|(_, title)| title.as_str())
    }

    /// Takes the pane at `index` out of the tab, with what the tab holds for it. Returns `false`
    /// when it was the tab's only pane, which leaves the tab untouched.
    fn close_pane(&mut self, index: usize) -> bool {
        let tree = std::mem::replace(&mut self.tree, PaneTree::Pane);
        let Some(tree) = tree.without_pane(index) else {
            return false;
        };
        self.tree = tree;
        shift_entries(&mut self.agents, index);
        shift_entries(&mut self.cwds, index);
        shift_entries(&mut self.pane_labels, index);
        shift_entries(&mut self.terminal_titles, index);
        self.pane_numbers.remove(index);
        let remaining = self.tree.pane_count();
        if self.focused_pane > index {
            self.focused_pane -= 1;
        }
        self.focused_pane = self.focused_pane.min(remaining - 1);
        self.zoomed &= remaining > 1;
        true
    }

    /// Puts `agent` in the pane at `index` in the tree's order, replacing any agent there.
    pub fn agent(mut self, index: usize, agent: AgentScript) -> Self {
        assert!(
            index < self.tree.pane_count(),
            "tab {:?} has no pane at index {index}",
            self.label
        );
        self.agents.retain(|(existing, _)| *existing != index);
        self.agents.push((index, agent));
        self
    }

    /// Sets the working directory Herdr reports for the pane at `index` in the tree's order.
    pub fn cwd(self, index: usize, cwd: &str) -> Self {
        self.with_cwd(index, Some(cwd.to_owned()))
    }

    /// Makes Herdr report no working directory for the pane at `index` in the tree's order.
    pub fn without_cwd(self, index: usize) -> Self {
        self.with_cwd(index, None)
    }

    fn with_cwd(mut self, index: usize, cwd: Option<String>) -> Self {
        assert!(
            index < self.tree.pane_count(),
            "tab {:?} has no pane at index {index}",
            self.label
        );
        self.cwds.retain(|(existing, _)| *existing != index);
        self.cwds.push((index, cwd));
        self
    }

    fn cwd_at(&self, index: usize) -> Option<&str> {
        match self.cwds.iter().find(|(existing, _)| *existing == index) {
            Some((_, cwd)) => cwd.as_deref(),
            None => Some("/"),
        }
    }

    fn agent_at(&self, index: usize) -> Option<&AgentScript> {
        self.agents
            .iter()
            .find(|(existing, _)| *existing == index)
            .map(|(_, agent)| agent)
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
    pub(crate) tokens: Vec<(String, String)>,
    /// The repository's key and whether this is a linked worktree, when Herdr reports a checkout.
    pub(crate) worktree: Option<(String, bool)>,
    /// The highest pane number Herdr has given a pane of this workspace once panes have been
    /// closed, so a later pane never takes a closed one's number; zero before that.
    pub(crate) highest_pane_number: usize,
    /// The highest tab number Herdr has given a tab of this workspace once tabs have been closed,
    /// so a later tab never takes a closed one's number; zero before that.
    pub(crate) highest_tab_number: usize,
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

    /// The pane number of each pane of each tab, in tab and tree order. Herdr numbers a
    /// workspace's panes in the order it created them; here that is the order they are listed in
    /// until [`Self::close_pane`] fixes the numbers.
    pub(crate) fn pane_numbers(&self) -> Vec<Vec<usize>> {
        let mut next = 1;
        self.tabs
            .iter()
            .map(|tab| {
                let numbers = if tab.pane_numbers.is_empty() {
                    let first = next.max(self.highest_pane_number + 1);
                    (first..first + tab.tree.pane_count()).collect()
                } else {
                    tab.pane_numbers.clone()
                };
                next = numbers.last().map_or(next, |last| last + 1);
                numbers
            })
            .collect()
    }

    /// Closes the pane numbered `number`, as `pane.close` does: the Herdr tab goes with its only
    /// pane, and the workspace is left with no tabs when that was its last one. Returns `false`
    /// when the workspace has no such pane.
    pub(crate) fn close_pane(&mut self, number: usize) -> bool {
        let numbers = self.pane_numbers();
        let Some((tab_index, pane_index)) = numbers.iter().enumerate().find_map(|(tab, panes)| {
            Some((
                tab,
                panes.iter().position(|candidate| *candidate == number)?,
            ))
        }) else {
            return false;
        };
        self.pin_pane_numbers();
        if !self.tabs[tab_index].close_pane(pane_index) {
            self.remove_tab(tab_index);
        }
        true
    }

    /// The Herdr tab number of each tab, in tab order.
    pub(crate) fn tab_numbers(&self) -> Vec<usize> {
        let mut next = 1;
        self.tabs
            .iter()
            .map(|tab| {
                let number = if tab.number == 0 {
                    next.max(self.highest_tab_number + 1)
                } else {
                    tab.number
                };
                next = number + 1;
                number
            })
            .collect()
    }

    /// Fixes the pane numbers of every tab, so that removing panes leaves the others theirs.
    fn pin_pane_numbers(&mut self) {
        let numbers = self.pane_numbers();
        self.highest_pane_number = self
            .highest_pane_number
            .max(numbers.iter().flatten().copied().max().unwrap_or(0));
        for (tab, panes) in self.tabs.iter_mut().zip(numbers) {
            tab.pane_numbers = panes;
        }
    }

    /// Takes the tab at `index` out of the workspace, leaving the others their numbers and the
    /// numbers of their panes.
    fn remove_tab(&mut self, index: usize) {
        self.pin_pane_numbers();
        let numbers = self.tab_numbers();
        self.highest_tab_number = self
            .highest_tab_number
            .max(numbers.last().copied().unwrap_or(0));
        for (tab, number) in self.tabs.iter_mut().zip(numbers) {
            tab.number = number;
        }
        let closed = self.tabs.remove(index);
        if self.active_tab_label.as_ref() == Some(&closed.label) {
            self.active_tab_label = None;
        }
    }
}

/// Drops the entry for the pane at `removed` and moves those of the panes after it up by one.
fn shift_entries<T>(entries: &mut Vec<(usize, T)>, removed: usize) {
    entries.retain(|(index, _)| *index != removed);
    for (index, _) in entries.iter_mut() {
        if *index > removed {
            *index -= 1;
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
            tokens: Vec::new(),
            worktree: None,
            highest_pane_number: 0,
            highest_tab_number: 0,
        });
        self
    }

    /// Makes the workspace labelled `workspace_label` a checkout of the Git repository
    /// identified by `repo_key`, as Herdr reports a workspace whose directory is in a work tree:
    /// the main checkout, or a linked worktree when `is_linked` is set.
    pub fn workspace_worktree(
        mut self,
        workspace_label: &str,
        repo_key: &str,
        is_linked: bool,
    ) -> Self {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.label == workspace_label)
            .unwrap_or_else(|| panic!("no workspace labelled {workspace_label:?}"));
        workspace.worktree = Some((repo_key.to_owned(), is_linked));
        self
    }

    /// Gives the workspace labelled `workspace_label` the token `name`, replacing one of the same
    /// name. Herdr reports a token until it is cleared or expires, and then stops reporting it,
    /// so a token is cleared by scripting the session again without it.
    pub fn workspace_token(mut self, workspace_label: &str, name: &str, value: &str) -> Self {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.label == workspace_label)
            .unwrap_or_else(|| panic!("no workspace labelled {workspace_label:?}"));
        workspace.tokens.retain(|(existing, _)| existing != name);
        workspace.tokens.push((name.to_owned(), value.to_owned()));
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

    /// Puts `agent` in pane `pane_index` of the Herdr tab labelled `tab_label` in the workspace
    /// labelled `workspace_label`, replacing any agent already there.
    pub fn agent_in(
        self,
        workspace_label: &str,
        tab_label: &str,
        pane_index: usize,
        agent: AgentScript,
    ) -> Self {
        self.edit_tab(workspace_label, tab_label, |tab| {
            tab.agents.retain(|(existing, _)| *existing != pane_index);
            tab.agents.push((pane_index, agent));
        })
    }

    /// Takes the agent out of pane `pane_index` of the named Herdr tab, if there is one.
    pub fn without_agent_in(
        self,
        workspace_label: &str,
        tab_label: &str,
        pane_index: usize,
    ) -> Self {
        self.edit_tab(workspace_label, tab_label, |tab| {
            tab.agents.retain(|(existing, _)| *existing != pane_index);
        })
    }

    fn edit_tab(
        mut self,
        workspace_label: &str,
        tab_label: &str,
        edit: impl FnOnce(&mut TabScript),
    ) -> Self {
        let tab = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.label == workspace_label)
            .and_then(|workspace| workspace.tabs.iter_mut().find(|tab| tab.label == tab_label))
            .unwrap_or_else(|| panic!("no tab {tab_label:?} in workspace {workspace_label:?}"));
        edit(tab);
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

    /// Adds a workspace with one Herdr tab holding one pane, as `workspace.create` does, and
    /// returns the id Herdr gives it. The new workspace becomes the focused one when `focus` is set.
    pub(crate) fn create_workspace(&mut self, label: Option<&str>, focus: bool) -> String {
        self.next_workspace_id += 1;
        let id = self.next_workspace_id;
        let label = label.map_or_else(|| id.to_string(), str::to_owned);
        self.workspaces.push(WorkspaceScript {
            id,
            label: label.clone(),
            tabs: vec![TabScript::new("1", PaneTree::pane())],
            active_tab_label: None,
            tokens: Vec::new(),
            worktree: None,
            highest_pane_number: 0,
            highest_tab_number: 0,
        });
        if focus {
            self.focused_label = Some(label);
        }
        format!("w{id}")
    }

    /// Adds a linked worktree workspace of the repository whose main checkout is `repo_root`, as
    /// `worktree.create` does, and returns the id Herdr gives it, or `None` when no workspace is a
    /// checkout of such a repository. The new workspace becomes the focused one when `focus` is
    /// set.
    pub(crate) fn create_worktree(
        &mut self,
        repo_root: &str,
        label: &str,
        focus: bool,
    ) -> Option<String> {
        let repo_key = self
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.worktree.as_ref())
            .find(|(repo_key, _)| repo_key.trim_end_matches("/.git") == repo_root)
            .map(|(repo_key, _)| repo_key.clone())?;
        let id = self.create_workspace(Some(label), focus);
        let workspace = self
            .workspaces
            .last_mut()
            .expect("the workspace was just added");
        workspace.worktree = Some((repo_key, true));
        Some(id)
    }

    /// Adds a Herdr tab with one pane to the workspace `workspace_id`, as `tab.create` does, and
    /// returns the id Herdr gives it, or `None` when there is no such workspace. With `focus` the
    /// tab becomes the workspace's active one and the workspace becomes the focused one.
    pub(crate) fn create_tab(
        &mut self,
        workspace_id: &str,
        label: Option<&str>,
        focus: bool,
    ) -> Option<String> {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| format!("w{}", workspace.id) == workspace_id)?;
        let number = workspace
            .tab_numbers()
            .last()
            .copied()
            .unwrap_or(0)
            .max(workspace.highest_tab_number)
            + 1;
        let label = label.map_or_else(
            || {
                (number..)
                    .map(|candidate| candidate.to_string())
                    .find(|candidate| workspace.tabs.iter().all(|tab| &tab.label != candidate))
                    .expect("an unused label exists")
            },
            str::to_owned,
        );
        workspace
            .tabs
            .push(TabScript::new(&label, PaneTree::pane()));
        if focus {
            workspace.active_tab_label = Some(label);
            self.focused_label = Some(workspace.label.clone());
        }
        Some(format!("{workspace_id}:t{number}"))
    }

    /// Closes the pane `pane_id` as `pane.close` does and returns the id of its workspace, or
    /// `None` when there is no such pane. A workspace left with no panes goes too.
    pub(crate) fn close_pane(&mut self, pane_id: &str) -> Option<String> {
        let (workspace_id, number) = pane_id.split_once(":p")?;
        let number = number.parse().ok()?;
        let index = self
            .workspaces
            .iter()
            .position(|workspace| format!("w{}", workspace.id) == workspace_id)?;
        if !self.workspaces[index].close_pane(number) {
            return None;
        }
        self.drop_workspace_if_empty(index);
        Some(workspace_id.to_owned())
    }

    /// The workspace and tab index of the Herdr tab `tab_id`, with the id of its workspace.
    fn find_tab<'a>(&self, tab_id: &'a str) -> Option<(usize, usize, &'a str)> {
        let (workspace_id, number) = tab_id.split_once(":t")?;
        let number: usize = number.parse().ok()?;
        let workspace = self
            .workspaces
            .iter()
            .position(|workspace| format!("w{}", workspace.id) == workspace_id)?;
        let tab = self.workspaces[workspace]
            .tab_numbers()
            .iter()
            .position(|candidate| *candidate == number)?;
        Some((workspace, tab, workspace_id))
    }

    /// Gives the Herdr tab `tab_id` the label `label` as `tab.rename` does and returns the id of
    /// its workspace, or `None` when there is no such tab.
    pub(crate) fn rename_tab(&mut self, tab_id: &str, label: &str) -> Option<String> {
        let (workspace, tab, workspace_id) = self.find_tab(tab_id)?;
        let workspace_id = workspace_id.to_owned();
        let workspace = &mut self.workspaces[workspace];
        let previous = std::mem::replace(&mut workspace.tabs[tab].label, label.to_owned());
        if workspace.active_tab_label.as_ref() == Some(&previous) {
            workspace.active_tab_label = Some(label.to_owned());
        }
        Some(workspace_id)
    }

    /// Closes the Herdr tab `tab_id` as `tab.close` does and returns the id of its workspace, or
    /// `None` when there is no such tab. A workspace left with no tabs goes too.
    pub(crate) fn close_tab(&mut self, tab_id: &str) -> Option<String> {
        let (workspace, tab, workspace_id) = self.find_tab(tab_id)?;
        let workspace_id = workspace_id.to_owned();
        self.workspaces[workspace].remove_tab(tab);
        self.drop_workspace_if_empty(workspace);
        Some(workspace_id)
    }

    fn drop_workspace_if_empty(&mut self, index: usize) {
        if self.workspaces[index].tabs.is_empty() {
            let closed = self.workspaces.remove(index);
            if self.focused_label.as_ref() == Some(&closed.label) {
                self.focused_label = None;
            }
        }
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
        let mut agents = Vec::new();
        let mut panes = Vec::new();
        let mut layouts = Vec::new();
        let mut focused_tab_id = Value::Null;
        let mut focused_pane_id = Value::Null;

        for (index, workspace) in self.workspaces.iter().enumerate() {
            let number = index + 1;
            let workspace_id = format!("w{}", workspace.id);
            let active_tab = workspace.active_tab_index();
            let tab_numbers = workspace.tab_numbers();
            let tab_id_of =
                |tab_index: usize| format!("{workspace_id}:t{}", tab_numbers[tab_index]);
            let pane_numbers = workspace.pane_numbers();
            let mut pane_count = 0;

            for (tab_index, tab) in workspace.tabs.iter().enumerate() {
                let tab_id = tab_id_of(tab_index);
                let numbers = &pane_numbers[tab_index];
                let pane_ids: Vec<String> = numbers
                    .iter()
                    .map(|number| format!("{workspace_id}:p{number}"))
                    .collect();
                let is_active = active_tab == Some(tab_index);
                let tab_focused_pane = &pane_ids[tab.focused_pane];

                let mut geometry = Geometry::default();
                geometry.place(&tab.tree, Rect::area(), "root");

                tabs.push(json!({
                    "tab_id": tab_id,
                    "workspace_id": workspace_id,
                    "number": tab_numbers[tab_index],
                    "label": tab.label,
                    "focused": is_active,
                    "pane_count": pane_ids.len(),
                    "agent_status": "unknown",
                }));
                for (pane_index, pane_id) in pane_ids.iter().enumerate() {
                    let terminal_number = numbers[pane_index];
                    let mut pane = json!({
                        "pane_id": pane_id,
                        "terminal_id": terminal_id(workspace.id, terminal_number),
                        "workspace_id": workspace_id,
                        "tab_id": tab_id,
                        "focused": pane_id == tab_focused_pane,
                        "foreground_cwd": "/",
                        "agent_status": tab
                            .agent_at(pane_index)
                            .map_or(AgentState::Unknown, |agent| agent.state)
                            .wire_name(),
                        "scroll": {
                            "offset_from_bottom": 0,
                            "max_offset_from_bottom": 0,
                            "viewport_rows": AREA_HEIGHT,
                        },
                        "revision": 0,
                    });
                    if let Some(cwd) = tab.cwd_at(pane_index) {
                        pane["cwd"] = json!(cwd);
                    }
                    if let Some(label) = tab.label_at(pane_index) {
                        pane["label"] = json!(label);
                    }
                    if let Some(title) = tab.terminal_title_at(pane_index) {
                        pane["terminal_title"] = json!(title);
                        pane["terminal_title_stripped"] = json!(title.trim());
                    }
                    if let Some(agent) = tab.agent_at(pane_index) {
                        agents.push(agent_info(&pane, agent));
                    }
                    panes.push(pane);
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
                pane_count += pane_ids.len();
            }

            let mut info = json!({
                "workspace_id": workspace_id,
                "number": number,
                "label": workspace.label,
                "focused": focused == Some(index),
                "pane_count": pane_count,
                "tab_count": workspace.tabs.len(),
                "active_tab_id": active_tab.map(tab_id_of),
                "agent_status": "unknown",
            });
            // Herdr leaves `tokens` out of a workspace that has none.
            if !workspace.tokens.is_empty() {
                info["tokens"] = workspace.tokens.iter().cloned().collect();
            }
            if let Some((repo_key, is_linked)) = &workspace.worktree {
                info["worktree"] = json!({
                    "repo_key": repo_key,
                    "repo_name": repo_key.rsplit('/').nth(1).unwrap_or(repo_key),
                    "repo_root": repo_key.trim_end_matches("/.git"),
                    "checkout_path": format!("{}/{}", repo_key.trim_end_matches("/.git"), workspace.label),
                    "is_linked_worktree": is_linked,
                });
            }
            workspaces.push(info);
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
            "agents": agents,
        })
    }
}

/// What `agent.list` reports for the agent in `pane`, a pane as `session.snapshot` reports it.
fn agent_info(pane: &Value, agent: &AgentScript) -> Value {
    let mut info = json!({
        "terminal_id": pane["terminal_id"],
        "agent": agent.kind,
        "agent_status": agent.state.wire_name(),
        "workspace_id": pane["workspace_id"],
        "tab_id": pane["tab_id"],
        "pane_id": pane["pane_id"],
        "focused": pane["focused"],
        "state_change_seq": 1,
        "cwd": pane["cwd"],
        "foreground_cwd": pane["foreground_cwd"],
        "revision": pane["revision"],
    });
    if let Some(name) = &agent.name {
        info["name"] = json!(name);
    }
    info
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
