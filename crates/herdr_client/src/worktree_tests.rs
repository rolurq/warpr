use std::path::{Path, PathBuf};

use herdr_fake::{FakeHerdr, HerdrStatus, PaneTree, SessionScript, SplitDirection, TabScript};

use crate::{HerdrCli, PresentedSession, SessionName, SessionTarget, WorkspaceId, present_session};

const REPO: &str = "/src/app/.git";

fn present(script: SessionScript) -> PresentedSession {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    let _session = fake.start_session("main", script);
    let target = SessionTarget::new(&fake.config_dir(), SessionName::new("main").unwrap());
    present_session(&HerdrCli::new(fake.binary_path()), &target)
}

fn parent_of(session: &PresentedSession, workspace: &str) -> Option<String> {
    session
        .worktree_parent(&WorkspaceId::from(workspace))
        .map(|parent| parent.as_str().to_owned())
}

#[test]
fn linked_worktrees_belong_to_the_main_checkout_of_their_repository() {
    let session = present(
        SessionScript::new()
            .workspace("app")
            .workspace("feature")
            .workspace("fix")
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("feature", REPO, true)
            .workspace_worktree("fix", REPO, true),
    );

    assert_eq!(parent_of(&session, "w1"), None);
    assert_eq!(parent_of(&session, "w2").as_deref(), Some("w1"));
    assert_eq!(parent_of(&session, "w3").as_deref(), Some("w1"));
}

#[test]
fn a_workspaces_directory_is_the_cwd_of_the_focused_pane_of_its_active_tab() {
    let two_panes = PaneTree::split(
        SplitDirection::Right,
        0.5,
        PaneTree::pane(),
        PaneTree::pane(),
    );
    let session = present(
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [TabScript::new("1", two_panes)
                    .cwd(0, "/src/api")
                    .cwd(1, "/src/api/docs")
                    .focused_pane(1)],
            )
            .workspace_with_tabs(
                "empty",
                [TabScript::new("1", PaneTree::pane()).without_cwd(0)],
            ),
    );

    let directory = |workspace: &str| {
        session
            .workspace(&WorkspaceId::from(workspace))
            .and_then(|workspace| workspace.directory().map(Path::to_path_buf))
    };

    assert_eq!(directory("w1"), Some(PathBuf::from("/src/api/docs")));
    assert_eq!(directory("w2"), None);
}

#[test]
fn repositories_lists_each_repository_once_with_its_root_in_herdrs_order() {
    let session = present(
        SessionScript::new()
            .workspace("app")
            .workspace("feature")
            .workspace("lib")
            .workspace("scratch")
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("feature", REPO, true)
            .workspace_worktree("lib", "/src/lib/.git", false),
    );

    let repositories: Vec<_> = session
        .repositories()
        .into_iter()
        .map(|repository| (repository.repo_key, repository.root))
        .collect();

    assert_eq!(
        repositories,
        [
            (REPO.to_owned(), PathBuf::from("/src/app")),
            ("/src/lib/.git".to_owned(), PathBuf::from("/src/lib")),
        ]
    );
}

#[test]
fn the_repository_of_a_workspace_is_its_checkouts_repository() {
    let session = present(
        SessionScript::new()
            .workspace("app")
            .workspace("feature")
            .workspace("scratch")
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("feature", REPO, true),
    );

    let root_of = |workspace: &str| {
        session
            .repository_of(&WorkspaceId::from(workspace))
            .map(|repository| repository.root)
    };

    assert_eq!(root_of("w2"), Some(PathBuf::from("/src/app")));
    assert_eq!(root_of("w3"), None);
}

#[test]
fn a_linked_worktree_whose_main_checkout_is_not_open_has_no_parent() {
    let session = present(
        SessionScript::new()
            .workspace("feature")
            .workspace_worktree("feature", REPO, true),
    );

    assert_eq!(parent_of(&session, "w1"), None);
}

#[test]
fn workspaces_of_other_repositories_or_outside_any_repository_are_not_nested() {
    let session = present(
        SessionScript::new()
            .workspace("app")
            .workspace("other-feature")
            .workspace("notes")
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("other-feature", "/src/other/.git", true),
    );

    assert_eq!(parent_of(&session, "w2"), None);
    assert_eq!(parent_of(&session, "w3"), None);
}

#[test]
fn a_main_checkout_is_never_nested_even_when_another_has_the_same_repository() {
    let session = present(
        SessionScript::new()
            .workspace("app")
            .workspace("clone")
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("clone", REPO, false),
    );

    assert_eq!(parent_of(&session, "w2"), None);
}

fn groups_of(session: &PresentedSession) -> Vec<Vec<String>> {
    session
        .workspace_groups()
        .into_iter()
        .map(|group| group.iter().map(|id| id.as_str().to_owned()).collect())
        .collect()
}

#[test]
fn every_workspace_is_a_group_led_by_itself_with_its_linked_worktrees_after_it_in_herdrs_order() {
    let session = present(
        SessionScript::new()
            .workspace("feature")
            .workspace("notes")
            .workspace("app")
            .workspace("fix")
            .workspace("orphan")
            .workspace_worktree("feature", REPO, true)
            .workspace_worktree("app", REPO, false)
            .workspace_worktree("fix", REPO, true)
            .workspace_worktree("orphan", "/src/gone/.git", true),
    );

    assert_eq!(
        groups_of(&session),
        [vec!["w2"], vec!["w3", "w1", "w4"], vec!["w5"],]
    );
}
