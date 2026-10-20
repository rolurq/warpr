use std::path::{Path, PathBuf};
use std::time::Duration;

use herdr_client::{HerdrCli, ReconnectPolicy, Refusal, SessionName, SessionTarget};
use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, TabScript};
use serde_json::json;
use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, ViewHandle};

use super::*;
use crate::tab::NewSessionMenuItem;
use crate::tab_configs::NewWorktreeModalEvent;
use crate::test_util::assert_eventually;
use crate::workspace::WorkspaceAction;
use crate::workspace::view::NewSessionSidecarSelection;
use crate::workspace::view::tests::{initialize_app_without_herdr, mock_workspace};

fn fake_with(status: HerdrStatus) -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&status);
    fake
}

fn connect_to(app: &mut App, fake: &FakeHerdr, session: &str) {
    connect_with(app, HerdrCli::new(fake.binary_path()), fake, session);
}

fn connect_with(app: &mut App, cli: HerdrCli, fake: &FakeHerdr, session: &str) {
    let target = SessionTarget::new(&fake.config_dir(), SessionName::new(session).unwrap());
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| HerdrSessionModel::connect(cli, target, ctx));
    });
}

/// Retries quickly so reconnection is observable within a test's time.
fn connect_to_with_quick_retries(app: &mut App, fake: &FakeHerdr, session: &str) {
    let cli = HerdrCli::new(fake.binary_path());
    let target = SessionTarget::new(&fake.config_dir(), SessionName::new(session).unwrap());
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        disconnected_after: 1000,
    };
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::connect_with_policy(cli, target, policy, ctx)
        });
    });
}

fn workspaces(labels: &[&str]) -> SessionScript {
    labels.iter().fold(SessionScript::new(), |script, label| {
        script.workspace(label)
    })
}

fn announce(session: &FakeSession, event: &str) {
    session.emit_event(event, json!({ "type": event }));
}

fn tab_ids(app: &App, workspace: &ViewHandle<Workspace>) -> Vec<EntityId> {
    workspace.read(app, |workspace, _| {
        workspace
            .tabs
            .iter()
            .map(|tab| tab.pane_group.id())
            .collect()
    })
}

fn connection(app: &App) -> ConnectionStatus {
    HerdrSessionModel::handle(app).read(app, |model, _| model.presented().connection.clone())
}

fn refusal_of(app: &App) -> Option<Refusal> {
    HerdrSessionModel::handle(app).read(app, |model, _| match &model.presented().connection {
        ConnectionStatus::Refused(refusal) => Some(refusal.clone()),
        _ => None,
    })
}

fn tab_titles(app: &App, workspace: &ViewHandle<Workspace>) -> Vec<String> {
    workspace.read(app, |workspace, ctx| {
        workspace
            .tabs
            .iter()
            .map(|tab| tab.pane_group.as_ref(ctx).display_title(ctx))
            .collect()
    })
}

fn active_tab_title(app: &App, workspace: &ViewHandle<Workspace>) -> String {
    workspace.read(app, |workspace, ctx| {
        workspace
            .active_tab_pane_group()
            .as_ref(ctx)
            .display_title(ctx)
    })
}

#[test]
fn each_herdr_workspace_becomes_a_top_level_tab_with_its_label() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let _session = fake.start_session(
            "default",
            SessionScript::new()
                .workspace("api")
                .workspace("web")
                .workspace("docs"),
        );
        connect_to(&mut app, &fake, "default");

        let workspace = mock_workspace(&mut app);

        assert_eventually!(
            200 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
    });
}

#[test]
fn the_workspace_herdr_has_focused_is_the_selected_tab() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let _session = fake.start_session(
            "default",
            SessionScript::new()
                .workspace("api")
                .workspace("web")
                .workspace("docs")
                .focused("docs"),
        );
        connect_to(&mut app, &fake, "default");

        let workspace = mock_workspace(&mut app);

        assert_eventually!(
            200 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(active_tab_title(&app, &workspace), "docs");
    });
}

#[test]
fn a_window_opened_after_connecting_presents_the_workspaces_immediately() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let _session = fake.start_session(
            "default",
            SessionScript::new()
                .workspace("api")
                .workspace("web")
                .focused("web"),
        );
        connect_to(&mut app, &fake, "default");
        assert_eventually!(
            200 => HerdrSessionModel::handle(&app).read(&app, |model, _| {
                model.presented().connection == ConnectionStatus::Connected
            }),
            "Herdr session never connected"
        );

        let workspace = mock_workspace(&mut app);

        assert_eq!(tab_titles(&app, &workspace), ["api", "web"]);
        assert_eq!(active_tab_title(&app, &workspace), "web");
    });
}

#[test]
fn an_unreachable_session_leaves_the_windows_tabs_alone() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::server_not_running());
        connect_to(&mut app, &fake, "stopped");
        assert_eventually!(
            200 => matches!(connection(&app), ConnectionStatus::Reconnecting(_)),
            "connection never reported the failed attempt"
        );

        let workspace = mock_workspace(&mut app);

        assert_eq!(workspace.read(&app, |workspace, _| workspace.tabs.len()), 1);
    });
}

#[test]
fn an_incompatible_herdr_replaces_the_windows_tabs_with_the_reason_and_shows_no_workspaces() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible().server_protocol(21));
        let _session = fake.start_session("default", SessionScript::new().workspace("api"));
        connect_to(&mut app, &fake, "default");
        assert_eventually!(200 => refusal_of(&app).is_some(), "Herdr was never refused");

        let workspace = mock_workspace(&mut app);

        assert_eq!(tab_titles(&app, &workspace), ["Cannot connect to Herdr"]);
        assert!(matches!(
            refusal_of(&app),
            Some(Refusal::IncompatibleProtocol {
                reported: Some(21),
                needed: 22,
                ..
            })
        ));
    });
}

#[test]
fn a_missing_herdr_executable_replaces_the_windows_tabs_with_the_reason() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = FakeHerdr::new();
        let missing = HerdrCli::new(fake.config_dir().join("not-installed").join("herdr"));
        connect_with(&mut app, missing, &fake, "default");
        assert_eventually!(200 => refusal_of(&app).is_some(), "Herdr was never refused");

        let workspace = mock_workspace(&mut app);

        assert_eq!(tab_titles(&app, &workspace), ["Cannot connect to Herdr"]);
        assert!(matches!(
            refusal_of(&app),
            Some(Refusal::ExecutableMissing { .. })
        ));
    });
}

#[test]
fn a_workspace_created_in_herdr_gets_a_tab_at_its_position_without_moving_the_selection() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        // Selecting the first tab makes Warp's own new-tab placement differ from Herdr's order.
        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(0, ctx));
        let before = tab_ids(&app, &workspace);

        session.set_script(workspaces(&["api", "web", "docs"]));
        announce(&session, "workspace_created");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(tab_ids(&app, &workspace)[..2], before[..]);
        assert_eq!(active_tab_title(&app, &workspace), "api");
    });
}

#[test]
fn a_workspace_closed_in_herdr_closes_only_its_tab() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web", "docs"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(1, ctx));
        let before = tab_ids(&app, &workspace);

        session.set_script(workspaces(&["api", "web"]));
        announce(&session, "workspace_closed");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(tab_ids(&app, &workspace)[..], before[..2]);
        assert_eq!(active_tab_title(&app, &workspace), "web");
    });
}

#[test]
fn a_workspace_closed_in_the_middle_keeps_the_tabs_of_the_others() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web", "docs"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let before = tab_ids(&app, &workspace);

        session.set_script(
            SessionScript::new()
                .workspace("api")
                .workspace("web")
                .workspace("docs")
                .without_workspace("web"),
        );
        announce(&session, "workspace_closed");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(tab_ids(&app, &workspace), [before[0], before[2]]);
    });
}

#[test]
fn closing_the_selected_workspace_selects_the_one_herdr_has_focused() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web", "docs"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(2, ctx));

        session.set_script(workspaces(&["api", "web"]).focused("api"));
        announce(&session, "workspace_closed");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(active_tab_title(&app, &workspace), "api");
    });
}

#[test]
fn a_workspace_renamed_in_herdr_retitles_its_tab_in_place() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let before = tab_ids(&app, &workspace);

        session.set_script(workspaces(&["api", "web"]).renamed_workspace("web", "backend"));
        announce(&session, "workspace_renamed");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "backend"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(tab_ids(&app, &workspace), before);
    });
}

#[test]
fn changes_whose_events_were_lost_are_presented_after_the_resync() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        session.set_script(workspaces(&["api", "web", "docs"]));
        session.lose_events();

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
    });
}

#[test]
fn a_lost_connection_is_shown_and_the_tabs_follow_herdr_once_it_returns() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to_with_quick_retries(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        session.go_offline();

        assert_eventually!(
            600 => matches!(connection(&app), ConnectionStatus::Reconnecting(_)),
            "connection was {:?}", connection(&app)
        );
        assert_eq!(tab_titles(&app, &workspace), ["api", "web"]);

        session.set_script(workspaces(&["web", "docs"]));
        session.go_online();

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["web", "docs"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(connection(&app), ConnectionStatus::Connected);
    });
}

#[test]
fn a_server_restarted_as_an_incompatible_release_is_refused_until_it_is_compatible_again() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let mut fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to_with_quick_retries(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        session.go_offline();
        fake.script_status(&HerdrStatus::compatible().server_protocol(21));
        session.go_online();

        assert_eventually!(
            600 => refusal_of(&app).is_some(),
            "connection was {:?}", connection(&app)
        );
        assert_eq!(tab_titles(&app, &workspace), ["Cannot connect to Herdr"]);

        fake.script_status(&HerdrStatus::compatible());

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eq!(connection(&app), ConnectionStatus::Connected);
    });
}

/// A window as warpr runs it: Warp's hosted agent off.
fn agent_panel_flags() -> impl Sized {
    (
        FeatureFlag::AgentManagementView.override_enabled(true),
        FeatureFlag::DisableHostedAgent.override_enabled(true),
    )
}

fn is_agent_panel_open(app: &App, workspace: &ViewHandle<Workspace>) -> bool {
    workspace.read(app, |workspace, _| {
        workspace
            .current_workspace_state
            .is_agent_management_view_open
    })
}

fn toggle_agent_panel(app: &mut App, workspace: &ViewHandle<Workspace>) {
    app.update(|ctx| {
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::ToggleAgentManagementView, ctx);
        });
    });
}

#[test]
fn the_agent_panel_opens_without_warps_hosted_agent_and_selecting_an_agent_goes_to_its_workspace() {
    let _flags = agent_panel_flags();
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session(
            "default",
            workspaces(&["api", "web"]).agent_in(
                "web",
                "1",
                0,
                herdr_fake::AgentScript::new("claude", herdr_fake::AgentState::Blocked),
            ),
        );
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eventually!(
            600 => HerdrSessionModel::handle(&app)
                .read(&app, |model, _| model.agent_jump_target(&"w2:p1".into()).is_some()),
            "agents were {:?}", HerdrSessionModel::handle(&app).read(&app, |model, _| model.agents().clone())
        );
        assert_eq!(active_tab_title(&app, &workspace), "api");

        toggle_agent_panel(&mut app, &workspace);
        assert!(is_agent_panel_open(&app, &workspace));
        app.update(|ctx| {
            HerdrSessionModel::handle(ctx).update(ctx, |model, ctx| {
                model.jump_to_agent(&"w2:p1".into(), ctx);
            });
        });

        assert_eq!(active_tab_title(&app, &workspace), "web");
        assert!(!is_agent_panel_open(&app, &workspace));
        let moved_focus = session
            .served_methods()
            .iter()
            .any(|method| method.ends_with(".focus") || method.ends_with(".focused"));
        assert!(!moved_focus, "{:?}", session.served_methods());
    });
}

fn has_toasts(app: &App, workspace: &ViewHandle<Workspace>) -> bool {
    workspace.read(app, |workspace, ctx| {
        workspace.toast_stack.as_ref(ctx).has_toasts()
    })
}

fn dispatch(app: &mut App, workspace: &ViewHandle<Workspace>, action: WorkspaceAction) {
    workspace.update(app, |workspace, ctx| workspace.handle_action(&action, ctx));
}

#[test]
fn the_new_tab_action_creates_a_herdr_workspace_that_takes_focus_through_herdrs_state() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(0, ctx));

        dispatch(&mut app, &workspace, WorkspaceAction::AddDefaultTab);

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web", "3"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eventually!(
            600 => active_tab_title(&app, &workspace) == "3",
            "active tab was {:?}", active_tab_title(&app, &workspace)
        );
        assert_eq!(
            session
                .served_methods()
                .iter()
                .filter(|method| method.as_str() == "workspace.create")
                .count(),
            1
        );
    });
}

#[test]
fn a_refused_workspace_creation_shows_an_error_and_adds_no_tab() {
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        session.reject_next_creation("server_busy", "try again later");

        dispatch(&mut app, &workspace, WorkspaceAction::AddDefaultTab);

        assert_eventually!(600 => has_toasts(&app, &workspace), "no error was shown");
        assert_eq!(tab_titles(&app, &workspace), ["api", "web"]);
    });
}

/// A session whose workspaces are in repositories under `root`, which exist on disk: `lib` and
/// `app` are main checkouts Herdr reports, `app-feature` is linked to `app`, and `notes` is in a
/// repository only its directory shows.
fn repository_script(root: &Path) -> (SessionScript, PathBuf, PathBuf) {
    let app_root = root.join("app");
    let lib_root = root.join("lib");
    let notes_dir = root.join("notes/src");
    for dir in [&app_root, &lib_root, &notes_dir, &root.join("notes/.git")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let app_key = format!("{}/.git", app_root.display());
    let lib_key = format!("{}/.git", lib_root.display());
    let notes = TabScript::new("1", PaneTree::pane()).cwd(0, &notes_dir.to_string_lossy());
    let script = SessionScript::new()
        .workspace_with_tabs("notes", [notes])
        .workspace("lib")
        .workspace("app")
        .workspace("app-feature")
        .workspace_worktree("lib", &lib_key, false)
        .workspace_worktree("app", &app_key, false)
        .workspace_worktree("app-feature", &app_key, true)
        .focused("app");
    (script, app_root, lib_root)
}

fn picked_repos(app: &App, workspace: &ViewHandle<Workspace>) -> (Vec<String>, Option<String>) {
    workspace.read(app, |workspace, ctx| {
        let body = workspace
            .new_worktree_modal
            .view
            .as_ref(ctx)
            .body()
            .as_ref(ctx);
        body.picked_repos(ctx)
    })
}

#[test]
fn the_new_worktree_modal_lists_each_repository_of_the_session_and_preselects_the_shown_one() {
    let temp = tempfile::tempdir().unwrap();
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let (script, app_root, lib_root) = repository_script(temp.path());
        let _session = fake.start_session("default", script);
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["notes", "lib", "app", "app-feature"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let notes_root = temp.path().join("notes");
        let listed =
            [&lib_root, &app_root, &notes_root].map(|root| root.to_string_lossy().into_owned());

        dispatch(&mut app, &workspace, WorkspaceAction::OpenNewWorktreeModal);
        assert_eq!(
            picked_repos(&app, &workspace),
            (listed.to_vec(), Some(listed[1].clone()))
        );

        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(1, ctx));
        dispatch(&mut app, &workspace, WorkspaceAction::OpenNewWorktreeModal);
        assert_eq!(
            picked_repos(&app, &workspace),
            (listed.to_vec(), Some(listed[0].clone()))
        );

        workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(0, ctx));
        dispatch(&mut app, &workspace, WorkspaceAction::OpenNewWorktreeModal);
        assert_eq!(
            picked_repos(&app, &workspace),
            (listed.to_vec(), Some(listed[2].clone()))
        );
    });
}

fn submit_worktree(app: &mut App, workspace: &ViewHandle<Workspace>, repo: &Path) {
    let event = NewWorktreeModalEvent::Submit {
        repo: repo.to_string_lossy().into_owned(),
        branch: "main".to_owned(),
        worktree_branch_name: Some("feature".to_owned()),
    };
    workspace.update(app, |workspace, ctx| {
        workspace.handle_new_worktree_modal_body_event(&event, ctx);
    });
}

#[test]
fn confirming_a_new_worktree_creates_it_in_herdr_and_presents_it_through_herdrs_state() {
    let temp = tempfile::tempdir().unwrap();
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let (script, app_root, _) = repository_script(temp.path());
        let session = fake.start_session("default", script);
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace).len() == 4,
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        submit_worktree(&mut app, &workspace, &app_root);

        assert_eventually!(
            600 => tab_titles(&app, &workspace)
                == ["notes", "lib", "app", "app-feature", "feature"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert_eventually!(
            600 => active_tab_title(&app, &workspace) == "feature",
            "active tab was {:?}", active_tab_title(&app, &workspace)
        );
        let methods = session.served_methods();
        assert!(methods.contains(&"worktree.create".to_owned()));
        assert!(!methods.contains(&"workspace.create".to_owned()));
    });
}

#[test]
fn a_refused_worktree_creation_shows_an_error_and_adds_nothing() {
    let temp = tempfile::tempdir().unwrap();
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let (script, app_root, _) = repository_script(temp.path());
        let session = fake.start_session("default", script);
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace).len() == 4,
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        session.reject_next_creation("worktree_exists", "branch feature is already checked out");

        submit_worktree(&mut app, &workspace, &app_root);

        assert_eventually!(600 => has_toasts(&app, &workspace), "no error was shown");
        assert_eq!(
            tab_titles(&app, &workspace),
            ["notes", "lib", "app", "app-feature"]
        );
    });
}

#[test]
fn the_new_worktree_menu_lists_the_sessions_repositories_and_creates_through_herdr() {
    let temp = tempfile::tempdir().unwrap();
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let (script, app_root, lib_root) = repository_script(temp.path());
        let session = fake.start_session("default", script);
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace).len() == 4,
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        let listed = workspace.read(&app, |workspace, ctx| workspace.worktree_sidecar_repos(ctx));
        assert_eq!(
            listed,
            [lib_root, app_root.clone(), temp.path().join("notes")]
        );

        let selection = NewSessionSidecarSelection::OpenWorktreeRepo {
            repo_path: app_root.to_string_lossy().into_owned(),
        };
        workspace.update(&mut app, |workspace, ctx| {
            workspace.execute_new_session_sidecar_selection(selection, ctx);
        });

        assert_eventually!(
            600 => tab_titles(&app, &workspace).len() == 5,
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let methods = session.served_methods();
        assert!(methods.contains(&"worktree.create".to_owned()));
        assert!(!methods.contains(&"workspace.create".to_owned()));
    });
}

#[cfg(feature = "local_tty")]
#[test]
fn adding_a_tab_with_a_shell_creates_a_herdr_workspace_instead_of_a_local_tab() {
    use crate::server::telemetry::AddTabWithShellSource;
    use crate::terminal::available_shells::AvailableShell;
    use crate::terminal::shell::ShellType;

    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let shell = AvailableShell::new_custom_shell(
            "zsh".to_owned(),
            PathBuf::from("/bin/zsh"),
            ShellType::Zsh,
        );

        dispatch(
            &mut app,
            &workspace,
            WorkspaceAction::AddTabWithShell {
                shell,
                source: AddTabWithShellSource::CommandPalette,
            },
        );

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "2"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        assert!(
            session
                .served_methods()
                .contains(&"workspace.create".to_owned())
        );
    });
}

/// The name of the group each top-level tab belongs to, in tab order.
fn group_names(app: &App, workspace: &ViewHandle<Workspace>) -> Vec<Option<String>> {
    workspace.read(app, |workspace, _| {
        workspace
            .tabs
            .iter()
            .map(|tab| {
                let group = workspace.tab_groups.get(&tab.group_id?)?;
                group.name.clone()
            })
            .collect()
    })
}

fn group_count(app: &App, workspace: &ViewHandle<Workspace>) -> usize {
    workspace.read(app, |workspace, _| workspace.tab_groups.len())
}

fn worktree_script() -> SessionScript {
    const REPO: &str = "/src/app/.git";
    SessionScript::new()
        .workspace("feature")
        .workspace("notes")
        .workspace("app")
        .workspace_worktree("feature", REPO, true)
        .workspace_worktree("app", REPO, false)
}

#[test]
fn each_workspace_is_a_group_with_its_linked_worktrees_nested_after_it() {
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let _session = fake.start_session("default", worktree_script());
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["notes", "app", "feature"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let named = |name: &str| Some(name.to_owned());
        assert_eq!(
            group_names(&app, &workspace),
            [named("notes"), named("app"), named("app")]
        );
        assert_eq!(group_count(&app, &workspace), 2);
    });
}

#[test]
fn a_worktree_workspace_whose_main_checkout_closes_leads_its_own_group() {
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", worktree_script());
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["notes", "app", "feature"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );

        session.set_script(worktree_script().without_workspace("app"));
        announce(&session, "workspace_closed");

        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["feature", "notes"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let named = |name: &str| Some(name.to_owned());
        assert_eq!(
            group_names(&app, &workspace),
            [named("feature"), named("notes")]
        );
        assert_eq!(group_count(&app, &workspace), 2);
    });
}

#[test]
fn the_user_cannot_create_groups_while_user_groups_are_disabled() {
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    let _no_user_groups = FeatureFlag::DisableUserTabGroups.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let session = fake.start_session("default", workspaces(&["api", "web"]));
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let groups_before = group_count(&app, &workspace);

        dispatch(
            &mut app,
            &workspace,
            WorkspaceAction::NewTabGroupFromActiveOrSelectedTabs,
        );
        dispatch(
            &mut app,
            &workspace,
            WorkspaceAction::SelectNewSessionMenuItem(NewSessionMenuItem::CreateNewTabGroup),
        );

        assert_eq!(tab_titles(&app, &workspace), ["api", "web"]);
        assert_eq!(group_count(&app, &workspace), groups_before);
        assert!(
            !session
                .served_methods()
                .contains(&"workspace.create".to_owned())
        );
    });
}

#[cfg(feature = "local_fs")]
mod tools_panel_scope {
    use std::path::{Path, PathBuf};

    use command::blocking::Command;
    use herdr_fake::{PaneTree, TabScript};

    use super::*;
    use crate::code::buffer_location::LocalOrRemotePath;

    fn directory_named(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).expect("create directory");
        dunce::canonicalize(path).expect("canonical directory")
    }

    fn git_init(directory: &Path) {
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .arg(directory)
            .status()
            .expect("run git");
        assert!(status.success(), "git init failed");
    }

    fn two_workspaces(api_cwd: &Path, web_cwd: &Path) -> SessionScript {
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [TabScript::new("server", PaneTree::pane()).cwd(0, api_cwd.to_str().unwrap())],
            )
            .workspace_with_tabs(
                "web",
                [TabScript::new("site", PaneTree::pane()).cwd(0, web_cwd.to_str().unwrap())],
            )
    }

    /// What the tools panel is scoped to in the shown tab.
    fn scope(app: &App, workspace: &ViewHandle<Workspace>) -> Vec<LocalOrRemotePath> {
        workspace.read(app, |workspace, ctx| {
            let pane_group_id = workspace.active_tab_pane_group().id();
            workspace
                .working_directories_model
                .as_ref(ctx)
                .most_recent_directories_for_pane_group(pane_group_id)
                .map(|directories| directories.map(|directory| directory.path).collect())
                .unwrap_or_default()
        })
    }

    fn local(path: &Path) -> Vec<LocalOrRemotePath> {
        vec![LocalOrRemotePath::Local(path.to_path_buf())]
    }

    #[test]
    fn the_tools_panel_follows_the_focused_pane_across_workspaces_and_snapshots() {
        App::test((), |mut app| async move {
            initialize_app_without_herdr(&mut app);
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let api = directory_named(temp_dir.path(), "api");
            let api_src = directory_named(temp_dir.path(), "api-src");
            let web = directory_named(temp_dir.path(), "web");
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces(&api, &web));
            connect_to(&mut app, &fake, "default");
            let workspace = mock_workspace(&mut app);
            assert_eventually!(
                600 => scope(&app, &workspace) == local(&api),
                "the panel was scoped to {:?}", scope(&app, &workspace)
            );

            workspace.update(&mut app, |workspace, ctx| {
                workspace.set_active_tab_index(1, ctx)
            });
            assert_eq!(scope(&app, &workspace), local(&web));

            session.set_script(two_workspaces(&api, &api_src));
            announce(&session, "pane_updated");
            assert_eventually!(
                600 => scope(&app, &workspace) == local(&api_src),
                "the panel was scoped to {:?}", scope(&app, &workspace)
            );
        });
    }

    #[test]
    fn a_directory_inside_a_git_repository_scopes_the_tools_panel_to_that_repository() {
        App::test((), |mut app| async move {
            initialize_app_without_herdr(&mut app);
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let repo = directory_named(temp_dir.path(), "repo");
            git_init(&repo);
            let nested = directory_named(&repo, "src");
            let web = directory_named(temp_dir.path(), "web");
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces(&nested, &web));
            connect_to(&mut app, &fake, "default");
            let workspace = mock_workspace(&mut app);

            assert_eventually!(
                1000 => scope(&app, &workspace) == local(&repo),
                "the panel was scoped to {:?}", scope(&app, &workspace)
            );

            let review = workspace.read(&app, |workspace, ctx| {
                workspace.review_repo_and_session(workspace.active_tab_pane_group(), ctx)
            });
            assert_eq!(review, Some((Some(LocalOrRemotePath::Local(repo)), None)));
        });
    }

    #[test]
    fn the_diff_panel_opened_on_a_herdr_workspace_in_a_git_repository_loads_its_diff() {
        use crate::code_review::diff_state::DiffState;

        App::test((), |mut app| async move {
            initialize_app_without_herdr(&mut app);
            app.add_singleton_model(|_| crate::code_review::GlobalCodeReviewModel);
            app.add_singleton_model(crate::code::global_buffer_model::GlobalBufferModel::new);
            app.add_singleton_model(|_| crate::terminal::local_shell::LocalShellState::NotLoaded);
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let repo = directory_named(temp_dir.path(), "repo");
            git_init(&repo);
            std::fs::write(repo.join("a.txt"), "changed\n").expect("write file");
            let web = directory_named(temp_dir.path(), "web");
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces(&repo, &web));
            connect_to(&mut app, &fake, "default");
            let workspace = mock_workspace(&mut app);
            assert_eventually!(
                1000 => scope(&app, &workspace) == local(&repo),
                "the panel was scoped to {:?}", scope(&app, &workspace)
            );

            workspace.update(&mut app, |workspace, ctx| {
                let pane_group = workspace.active_tab_pane_group().clone();
                workspace.toggle_right_panel(&pane_group, ctx);
            });

            let model = workspace
                .update(&mut app, |workspace, ctx| {
                    workspace
                        .working_directories_model
                        .update(ctx, |model, ctx| {
                            model.get_or_create_diff_state_model(
                                LocalOrRemotePath::Local(repo.clone()),
                                None,
                                ctx,
                            )
                        })
                })
                .expect("a diff state model for the repository");
            let diff_state = |app: &App| model.read(app, |model, ctx| model.get(ctx));
            assert_eventually!(
                3000 => matches!(diff_state(&app), DiffState::Loaded),
                "the diff state was {:?}", diff_state(&app)
            );

            let view_is_loading = |app: &App| {
                workspace.read(app, |workspace, ctx| {
                    let pane_group_id = workspace.active_tab_pane_group().id();
                    workspace
                        .working_directories_model
                        .as_ref(ctx)
                        .get_code_review_view(
                            pane_group_id,
                            &LocalOrRemotePath::Local(repo.clone()),
                        )
                        .map(|view| view.as_ref(ctx).is_showing_loading_state())
                })
            };
            assert_eventually!(
                1000 => view_is_loading(&app) == Some(false),
                "the review view's loading state was {:?}", view_is_loading(&app)
            );
        });
    }

    #[test]
    fn a_herdr_workspace_has_a_tools_panel_to_open() {
        App::test((), |mut app| async move {
            initialize_app_without_herdr(&mut app);
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", workspaces(&["api"]));
            connect_to(&mut app, &fake, "default");
            let workspace = mock_workspace(&mut app);
            assert_eventually!(
                200 => tab_titles(&app, &workspace) == ["api"],
                "tabs were {:?}", tab_titles(&app, &workspace)
            );

            assert!(workspace.read(&app, |workspace, ctx| {
                Workspace::should_enable_file_tree_and_global_search_for_pane_group(
                    workspace.active_tab_pane_group().as_ref(ctx),
                )
            }));
        });
    }
}

mod tab_bar {
    use herdr_client::HerdrTabId;

    use super::*;

    /// `api` (`w1`) has the Herdr tabs `server` and `logs`; `web` (`w2`) has `site`.
    fn two_workspaces() -> SessionScript {
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [
                    TabScript::new("server", PaneTree::pane()),
                    TabScript::new("logs", PaneTree::pane()),
                ],
            )
            .workspace_with_tabs("web", [TabScript::new("site", PaneTree::pane())])
    }

    fn bar_tabs(app: &App, workspace: &ViewHandle<Workspace>) -> Vec<(String, bool)> {
        workspace.read(app, |workspace, ctx| {
            workspace
                .herdr_bar_tabs(ctx)
                .into_iter()
                .map(|tab| (tab.label, tab.is_shown))
                .collect()
        })
    }

    fn shown(labels: &[&str], shown: &str) -> Vec<(String, bool)> {
        labels
            .iter()
            .map(|label| ((*label).to_owned(), *label == shown))
            .collect()
    }

    pub(super) async fn open_workspace_window(
        app: &mut App,
        fake: &FakeHerdr,
    ) -> ViewHandle<Workspace> {
        initialize_app_without_herdr(app);
        connect_to(app, fake, "default");
        let workspace = mock_workspace(app);
        assert_eventually!(
            200 => tab_titles(app, &workspace) == ["api", "web"],
            "tabs were {:?}", tab_titles(app, &workspace)
        );
        workspace
    }

    fn dispatch(app: &mut App, workspace: &ViewHandle<Workspace>, action: WorkspaceAction) {
        app.update(|ctx| {
            workspace.update(ctx, |workspace, ctx| workspace.handle_action(&action, ctx));
        });
    }

    #[test]
    fn the_bar_lists_the_herdr_tabs_of_the_active_workspace_and_follows_the_active_workspace() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );

            workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(1, ctx));
            assert_eq!(bar_tabs(&app, &workspace), shown(&["site"], "site"));

            workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(0, ctx));
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );
        });
    }

    #[test]
    fn selecting_a_bar_tab_shows_it_in_that_workspace_only_and_sends_nothing_to_herdr() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            let served_before = session.served_methods();

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::SelectHerdrTab(HerdrTabId::from("w1:t2")),
            );

            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "logs")
            );
            workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(1, ctx));
            assert_eq!(bar_tabs(&app, &workspace), shown(&["site"], "site"));
            workspace.update(&mut app, |workspace, ctx| workspace.activate_tab(0, ctx));
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "logs")
            );
            assert_eq!(session.served_methods(), served_before);
        });
    }

    #[test]
    fn the_plus_button_creates_a_herdr_tab_in_the_active_workspace_through_herdr() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            dispatch(&mut app, &workspace, WorkspaceAction::CreateHerdrTab);

            assert_eventually!(
                600 => bar_tabs(&app, &workspace).len() == 3,
                "the bar listed {:?}", bar_tabs(&app, &workspace)
            );
            let served: Vec<_> = session
                .served_methods()
                .into_iter()
                .filter(|method| method == "tab.create")
                .collect();
            assert_eq!(served.len(), 1);
        });
    }

    #[test]
    fn a_refused_herdr_tab_creation_leaves_the_bar_alone() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            session.reject_next_creation("server_busy", "try again later");

            dispatch(&mut app, &workspace, WorkspaceAction::CreateHerdrTab);

            assert_eventually!(
                600 => session
                    .served_methods()
                    .iter()
                    .any(|method| method == "tab.create"),
                "Herdr was never asked to create the tab"
            );
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );
        });
    }

    fn served(session: &FakeSession, method: &str) -> usize {
        session
            .served_methods()
            .iter()
            .filter(|served| served.as_str() == method)
            .count()
    }

    /// Types `text` into the bar's rename editor and ends the edit with `event`.
    fn edit_label(
        app: &mut App,
        workspace: &ViewHandle<Workspace>,
        text: &str,
        event: EditorEvent,
    ) {
        app.update(|ctx| {
            workspace.update(ctx, |workspace, ctx| {
                workspace
                    .herdr_tab_bar
                    .rename_editor
                    .update(ctx, |editor, ctx| {
                        editor.clear_buffer_and_reset_undo_stack(ctx);
                        editor.insert_selected_text(text, ctx);
                    });
                workspace.handle_herdr_tab_rename_editor_event(&event, ctx);
            });
        });
    }

    fn is_renaming(app: &App, workspace: &ViewHandle<Workspace>) -> bool {
        workspace.read(app, |workspace, _| {
            workspace.herdr_tab_bar.renaming.is_some()
        })
    }

    #[test]
    fn renaming_a_bar_tab_sends_the_confirmed_label_to_herdr_and_the_bar_follows_herdrs_state() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::RenameHerdrTab(HerdrTabId::from("w1:t2")),
            );
            assert!(is_renaming(&app, &workspace));
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server"),
                "nothing is renamed before the edit is confirmed"
            );
            edit_label(&mut app, &workspace, "  build  ", EditorEvent::Enter);

            assert!(!is_renaming(&app, &workspace));
            assert_eventually!(
                600 => bar_tabs(&app, &workspace) == shown(&["server", "build"], "server"),
                "the bar listed {:?}", bar_tabs(&app, &workspace)
            );
            assert_eq!(served(&session, "tab.rename"), 1);
        });
    }

    #[test]
    fn a_cancelled_blank_or_unchanged_rename_sends_nothing_to_herdr() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            let rename = WorkspaceAction::RenameHerdrTab(HerdrTabId::from("w1:t2"));

            dispatch(&mut app, &workspace, rename.clone());
            edit_label(&mut app, &workspace, "build", EditorEvent::Escape);
            dispatch(&mut app, &workspace, rename.clone());
            edit_label(&mut app, &workspace, "   ", EditorEvent::Enter);
            dispatch(&mut app, &workspace, rename);
            edit_label(&mut app, &workspace, "logs", EditorEvent::Blurred);

            assert!(!is_renaming(&app, &workspace));
            assert_eq!(served(&session, "tab.rename"), 0);
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );
        });
    }

    #[test]
    fn a_refused_rename_leaves_the_bar_alone() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            session.reject_next_rename("tab_not_found", "tab w1:t2 not found");

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::RenameHerdrTab(HerdrTabId::from("w1:t2")),
            );
            edit_label(&mut app, &workspace, "build", EditorEvent::Enter);

            assert_eventually!(
                600 => served(&session, "tab.rename") == 1,
                "Herdr was never asked to rename the tab"
            );
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );
        });
    }

    #[test]
    fn closing_a_bar_tab_goes_through_herdr_and_the_bar_updates_from_herdrs_state() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::CloseHerdrTab(HerdrTabId::from("w1:t1")),
            );

            assert_eventually!(
                600 => bar_tabs(&app, &workspace) == shown(&["logs"], "logs"),
                "the bar listed {:?}", bar_tabs(&app, &workspace)
            );
            assert_eq!(served(&session, "tab.close"), 1);
        });
    }

    #[test]
    fn closing_the_last_herdr_tab_of_a_workspace_leaves_the_tabs_as_herdr_reports_them() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::CloseHerdrTab(HerdrTabId::from("w1:t1")),
            );
            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::CloseHerdrTab(HerdrTabId::from("w1:t2")),
            );

            assert_eventually!(
                600 => tab_titles(&app, &workspace) == ["web"],
                "tabs were {:?}", tab_titles(&app, &workspace)
            );
            assert_eq!(bar_tabs(&app, &workspace), shown(&["site"], "site"));
            assert_eq!(served(&session, "tab.close"), 2);
        });
    }

    #[test]
    fn a_refused_close_leaves_the_bar_alone() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            session.reject_next_close("tab_not_found", "tab w1:t1 not found");

            dispatch(
                &mut app,
                &workspace,
                WorkspaceAction::CloseHerdrTab(HerdrTabId::from("w1:t1")),
            );

            assert_eventually!(
                600 => served(&session, "tab.close") == 1,
                "Herdr was never asked to close the tab"
            );
            assert_eq!(
                bar_tabs(&app, &workspace),
                shown(&["server", "logs"], "server")
            );
        });
    }

    #[test]
    fn the_bar_lists_no_tabs_while_the_agent_panel_covers_the_workspace() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;

            workspace.update(&mut app, |workspace, _| {
                workspace
                    .current_workspace_state
                    .is_agent_management_view_open = true;
            });

            assert!(bar_tabs(&app, &workspace).is_empty());
        });
    }
}

mod pane_rows {
    use herdr_client::PaneId;
    use herdr_fake::SplitDirection;

    use super::tab_bar::open_workspace_window;
    use super::*;

    /// `api` (`w1`) has the Herdr tab `logs` with three panes; `web` (`w2`) has `site` with one.
    fn two_workspaces() -> SessionScript {
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [TabScript::new(
                    "logs",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::split(
                            SplitDirection::Down,
                            0.5,
                            PaneTree::pane(),
                            PaneTree::pane(),
                        ),
                    ),
                )],
            )
            .workspace_with_tabs("web", [TabScript::new("site", PaneTree::pane())])
    }

    fn pane_group_of(app: &App, workspace: &ViewHandle<Workspace>, workspace_id: &str) -> EntityId {
        workspace.read(app, |workspace, ctx| {
            workspace
                .herdr_workspace_tabs(ctx)
                .into_iter()
                .find(|(_, id)| id.as_str() == workspace_id)
                .map(|(pane_group_id, _)| pane_group_id)
                .expect("the workspace has a tab")
        })
    }

    /// The panes of `workspace_id`'s shown Herdr tab with whether each is selected.
    fn rows(
        app: &App,
        workspace: &ViewHandle<Workspace>,
        workspace_id: &str,
    ) -> Vec<(String, bool)> {
        let pane_group_id = pane_group_of(app, workspace, workspace_id);
        workspace.read(app, |workspace, ctx| {
            let index = workspace
                .tab_index_for_pane_group(pane_group_id)
                .expect("the tab exists");
            workspace.tabs[index]
                .pane_group
                .as_ref(ctx)
                .herdr_workspace_view(ctx)
                .expect("a Herdr workspace view")
                .as_ref(ctx)
                .pane_rows(ctx)
                .into_iter()
                .map(|row| (row.pane.as_str().to_owned(), row.is_selected))
                .collect()
        })
    }

    fn select_pane_row(
        app: &mut App,
        workspace: &ViewHandle<Workspace>,
        workspace_id: &str,
        pane: &str,
    ) {
        let pane_group_id = pane_group_of(app, workspace, workspace_id);
        let herdr_tab = workspace.read(app, |workspace, ctx| {
            let index = workspace.tab_index_for_pane_group(pane_group_id).unwrap();
            workspace.tabs[index]
                .pane_group
                .as_ref(ctx)
                .herdr_workspace_view(ctx)
                .unwrap()
                .as_ref(ctx)
                .pane_rows(ctx)
                .first()
                .map(|row| row.herdr_tab.clone())
                .expect("a row")
        });
        dispatch(
            app,
            workspace,
            WorkspaceAction::SelectHerdrPane {
                pane_group_id,
                herdr_tab,
                pane: PaneId::from(pane),
            },
        );
    }

    fn active_tab_title(app: &App, workspace: &ViewHandle<Workspace>) -> String {
        workspace.read(app, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .as_ref(ctx)
                .display_title(ctx)
        })
    }

    /// Whether Herdr was asked to move its focus.
    fn moved_focus(session: &FakeSession) -> bool {
        session
            .served_methods()
            .iter()
            .any(|method| method.ends_with(".focus") || method.ends_with(".focused"))
    }

    #[test]
    fn selecting_a_pane_row_selects_that_pane_of_its_workspace_without_asking_herdr_to_move_focus()
    {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            assert_eq!(
                rows(&app, &workspace, "w1"),
                [
                    ("w1:p1".to_owned(), true),
                    ("w1:p2".to_owned(), false),
                    ("w1:p3".to_owned(), false)
                ]
            );

            select_pane_row(&mut app, &workspace, "w1", "w1:p3");

            assert_eq!(
                rows(&app, &workspace, "w1"),
                [
                    ("w1:p1".to_owned(), false),
                    ("w1:p2".to_owned(), false),
                    ("w1:p3".to_owned(), true)
                ]
            );
            assert!(!moved_focus(&session), "{:?}", session.served_methods());
        });
    }

    #[test]
    fn selecting_a_pane_row_of_another_workspace_shows_that_workspace() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            assert_eq!(active_tab_title(&app, &workspace), "api");

            select_pane_row(&mut app, &workspace, "w2", "w2:p1");

            assert_eq!(active_tab_title(&app, &workspace), "web");
            assert_eq!(rows(&app, &workspace, "w2"), [("w2:p1".to_owned(), true)]);
            assert!(!moved_focus(&session), "{:?}", session.served_methods());
        });
    }

    #[test]
    fn selecting_a_pane_row_closes_the_agent_panel_that_covered_the_workspaces() {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            workspace.update(&mut app, |workspace, _| {
                workspace
                    .current_workspace_state
                    .is_agent_management_view_open = true;
            });

            select_pane_row(&mut app, &workspace, "w1", "w1:p2");

            assert!(workspace.read(&app, |workspace, _| {
                !workspace
                    .current_workspace_state
                    .is_agent_management_view_open
            }));
        });
    }
}

mod vertical_tabs_rows {
    use herdr_fake::{AgentScript, AgentState, SplitDirection};

    use super::tab_bar::open_workspace_window;
    use super::*;
    use crate::workspace::tab_settings::VerticalTabsDisplayGranularity;
    use crate::workspace::view::vertical_tabs::render_tab_group_for_drag_ghost;

    /// `api` (`w1`) has the Herdr tab `logs` with three panes, the second with an agent and the
    /// last without a directory; `web` (`w2`) has `site` with one.
    fn two_workspaces() -> SessionScript {
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [TabScript::new(
                    "logs",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::split(
                            SplitDirection::Down,
                            0.5,
                            PaneTree::pane(),
                            PaneTree::pane(),
                        ),
                    ),
                )
                .pane_label(0, "editor")
                .pane_label(1, "build")
                .cwd(0, "/srv/api")
                .cwd(1, "/srv/api/worker")
                .without_cwd(2)
                .agent(
                    1,
                    AgentScript::new("claude", AgentState::Working).named("builder"),
                )],
            )
            .workspace_with_tabs(
                "web",
                [TabScript::new("site", PaneTree::pane()).pane_label(0, "frontend")],
            )
    }

    fn set_granularity(
        app: &mut App,
        workspace: &ViewHandle<Workspace>,
        granularity: VerticalTabsDisplayGranularity,
    ) {
        dispatch(
            app,
            workspace,
            WorkspaceAction::SetVerticalTabsDisplayGranularity(granularity),
        );
    }

    /// The text of the rows the vertical tabs list for the tab at `tab_index`.
    fn tab_text(app: &App, workspace: &ViewHandle<Workspace>, tab_index: usize) -> String {
        workspace.read(app, |workspace, ctx| {
            render_tab_group_for_drag_ghost(workspace, tab_index, ctx)
                .debug_text_content()
                .unwrap_or_default()
        })
    }

    #[test]
    fn panes_view_lists_the_panes_of_each_workspaces_shown_herdr_tab_and_workspaces_view_does_not()
    {
        App::test((), |mut app| async move {
            let fake = fake_with(HerdrStatus::compatible());
            let _session = fake.start_session("default", two_workspaces());
            let workspace = open_workspace_window(&mut app, &fake).await;
            set_granularity(&mut app, &workspace, VerticalTabsDisplayGranularity::Panes);
            assert_eventually!(
                600 => tab_text(&app, &workspace, 0).contains("builder · claude · working"),
                "the rows were {:?}", tab_text(&app, &workspace, 0)
            );

            let api = tab_text(&app, &workspace, 0);
            for expected in ["api", "editor", "builder", "/srv/api"] {
                assert!(api.contains(expected), "{expected} missing from {api:?}");
            }
            let web = tab_text(&app, &workspace, 1);
            for expected in ["web", "frontend"] {
                assert!(web.contains(expected), "{expected} missing from {web:?}");
            }

            set_granularity(&mut app, &workspace, VerticalTabsDisplayGranularity::Tabs);
            let api = tab_text(&app, &workspace, 0);
            assert!(api.contains("api"), "{api:?}");
            for hidden in ["editor", "build", "w1:p3", "claude"] {
                assert!(!api.contains(hidden), "{hidden} shown in {api:?}");
            }
        });
    }
}

#[test]
fn pane_actions_on_a_herdr_workspace_go_to_herdr_and_never_add_a_warp_pane() {
    use crate::pane_group::{Direction, PaneGroupAction};

    App::test((), |mut app| async move {
        initialize_app_without_herdr(&mut app);
        let fake = fake_with(HerdrStatus::compatible());
        let split = PaneTree::split(
            herdr_fake::SplitDirection::Right,
            0.5,
            PaneTree::pane(),
            PaneTree::pane(),
        );
        let session = fake.start_session(
            "default",
            SessionScript::new().workspace_with_tabs("api", [TabScript::new("1", split)]),
        );
        connect_to(&mut app, &fake, "default");
        let workspace = mock_workspace(&mut app);
        assert_eventually!(
            600 => tab_titles(&app, &workspace) == ["api"],
            "tabs were {:?}", tab_titles(&app, &workspace)
        );
        let pane_group = workspace.read(&app, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        let requests = |method: &str| {
            session
                .served_methods()
                .iter()
                .filter(|served| served.as_str() == method)
                .count()
        };

        pane_group.update(&mut app, |pane_group, ctx| {
            for action in [
                PaneGroupAction::Add(Direction::Left),
                PaneGroupAction::Add(Direction::Up),
                PaneGroupAction::Add(Direction::Right),
                PaneGroupAction::ResizeLeft,
                PaneGroupAction::ToggleMaximizePane,
            ] {
                pane_group.handle_action(&action, ctx);
            }
        });

        assert_eventually!(
            600 => requests("pane.split") == 1 && requests("pane.resize") == 1 && requests("pane.zoom") == 1,
            "Herdr was asked {:?}", session.served_methods()
        );
        pane_group.read(&app, |pane_group, ctx| {
            assert_eq!(pane_group.pane_count(), 1, "a Warp pane was added");
            assert!(!pane_group.is_focused_pane_maximized(ctx));
        });
    });
}
