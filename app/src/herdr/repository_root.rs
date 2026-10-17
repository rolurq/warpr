use std::path::{Path, PathBuf};

/// The root of the main checkout of the Git repository holding `directory`: the nearest
/// ancestor, `directory` included, that has a `.git`, followed to the main checkout when that
/// is a linked worktree. `None` outside any repository.
///
/// Only the filesystem is read, so it never blocks on a `git` process.
pub(crate) fn repository_root(directory: &Path) -> Option<PathBuf> {
    directory.ancestors().find_map(|ancestor| {
        let dot_git = ancestor.join(".git");
        if dot_git.is_dir() {
            return Some(ancestor.to_path_buf());
        }
        let pointer = std::fs::read_to_string(&dot_git).ok()?;
        let linked_gitdir = pointer
            .strip_prefix("gitdir:")
            .map(|gitdir| ancestor.join(gitdir.trim()));
        let main_checkout = linked_gitdir
            .as_deref()
            .and_then(main_checkout_of_linked_gitdir);
        Some(main_checkout.unwrap_or_else(|| ancestor.to_path_buf()))
    })
}

/// The main checkout of a linked worktree whose git directory is `gitdir`, found through the
/// `commondir` pointer to the repository's own `.git`. `None` when that is not a `.git`
/// directory, as in a bare repository.
fn main_checkout_of_linked_gitdir(gitdir: &Path) -> Option<PathBuf> {
    let commondir = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common_git_dir = gitdir.join(commondir.trim()).canonicalize().ok()?;
    if common_git_dir.file_name()? != ".git" {
        return None;
    }
    Some(common_git_dir.parent()?.to_path_buf())
}

#[cfg(test)]
#[path = "repository_root_tests.rs"]
mod tests;
