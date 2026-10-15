use herdr_fake::{FakeHerdr, HerdrStatus, SessionScript};

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
