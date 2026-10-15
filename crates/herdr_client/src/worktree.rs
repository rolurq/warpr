use serde::Deserialize;

use crate::{PresentedSession, WorkspaceId};

/// The Git checkout a workspace's directory is in, as Herdr reports it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PresentedWorktree {
    /// Identifies the repository, shared by its main checkout and every linked worktree.
    pub repo_key: String,
    #[serde(rename = "is_linked_worktree")]
    pub is_linked: bool,
}

impl PresentedSession {
    /// The workspace `workspace` is nested under: the first workspace holding the main checkout
    /// of the repository that `workspace` is a linked worktree of. `None` for any other workspace,
    /// including a linked worktree whose main checkout is not open.
    pub fn worktree_parent(&self, workspace: &WorkspaceId) -> Option<&WorkspaceId> {
        let worktree = self.workspace(workspace)?.worktree.as_ref()?;
        if !worktree.is_linked {
            return None;
        }
        self.workspaces
            .iter()
            .find(|candidate| {
                candidate.worktree.as_ref().is_some_and(|candidate| {
                    !candidate.is_linked && candidate.repo_key == worktree.repo_key
                })
            })
            .map(|candidate| &candidate.id)
    }

    /// The workspaces grouped as a front-end nests them, in Herdr's order of each group's lead.
    /// Every workspace that is not a nested worktree leads a group: the lead first, then the
    /// linked worktrees nested under it in Herdr's order.
    pub fn workspace_groups(&self) -> Vec<Vec<&WorkspaceId>> {
        self.workspaces
            .iter()
            .filter(|lead| self.worktree_parent(&lead.id).is_none())
            .map(|lead| {
                std::iter::once(&lead.id)
                    .chain(
                        self.workspaces
                            .iter()
                            .map(|member| &member.id)
                            .filter(|member| self.worktree_parent(member) == Some(&lead.id)),
                    )
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
