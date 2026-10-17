use std::path::PathBuf;

use serde::Deserialize;

use crate::{PresentedSession, WorkspaceId};

/// The Git checkout a workspace's directory is in, as Herdr reports it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PresentedWorktree {
    /// Identifies the repository, shared by its main checkout and every linked worktree.
    pub repo_key: String,
    /// The repository's main checkout, which Herdr creates worktrees from.
    pub repo_root: PathBuf,
    #[serde(rename = "is_linked_worktree")]
    pub is_linked: bool,
}

/// A Git repository of the session's workspaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedRepository {
    pub repo_key: String,
    pub root: PathBuf,
}

impl PresentedRepository {
    fn of(worktree: &PresentedWorktree) -> Self {
        Self {
            repo_key: worktree.repo_key.clone(),
            root: worktree.repo_root.clone(),
        }
    }
}

impl PresentedSession {
    /// The repositories the workspaces are checkouts of, each once, in the order their first
    /// workspace is listed. Workspaces outside any repository contribute none.
    pub fn repositories(&self) -> Vec<PresentedRepository> {
        let mut repositories: Vec<PresentedRepository> = Vec::new();
        for worktree in self.workspaces.iter().filter_map(|w| w.worktree.as_ref()) {
            if repositories
                .iter()
                .all(|known| known.repo_key != worktree.repo_key)
            {
                repositories.push(PresentedRepository::of(worktree));
            }
        }
        repositories
    }

    /// The repository `workspace` is a checkout of, if it is one.
    pub fn repository_of(&self, workspace: &WorkspaceId) -> Option<PresentedRepository> {
        self.workspace(workspace)?
            .worktree
            .as_ref()
            .map(PresentedRepository::of)
    }

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
