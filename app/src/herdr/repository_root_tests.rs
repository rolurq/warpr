use std::fs;
use std::path::Path;

use super::repository_root;

fn mkdir(path: &Path) {
    fs::create_dir_all(path).unwrap();
}

#[test]
fn a_directory_inside_a_repository_has_the_directory_holding_dot_git_as_its_root() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("app");
    mkdir(&repo.join(".git"));
    mkdir(&repo.join("src/deep"));

    assert_eq!(repository_root(&repo), Some(repo.clone()));
    assert_eq!(repository_root(&repo.join("src/deep")), Some(repo));
}

#[test]
fn a_linked_worktree_has_the_main_checkout_as_its_root() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("app");
    let linked = temp.path().join("checkouts/feature");
    mkdir(&main.join(".git/worktrees/feature"));
    mkdir(&linked);
    fs::write(main.join(".git/worktrees/feature/commondir"), "../..\n").unwrap();
    fs::write(
        linked.join(".git"),
        "gitdir: ../../app/.git/worktrees/feature\n",
    )
    .unwrap();

    assert_eq!(repository_root(&linked), Some(main));
}

#[test]
fn a_directory_outside_any_repository_has_no_root() {
    let temp = tempfile::tempdir().unwrap();
    mkdir(&temp.path().join("notes"));

    assert_eq!(repository_root(&temp.path().join("notes")), None);
}
