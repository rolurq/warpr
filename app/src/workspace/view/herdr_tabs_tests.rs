use std::time::Duration;

use herdr_client::{HerdrCli, ReconnectPolicy, Refusal, SessionName, SessionTarget};
use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, SessionScript};
use serde_json::json;
use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, TypedActionView as _, ViewHandle};

use super::*;
use crate::tab::NewSessionMenuItem;
use crate::test_util::assert_eventually;
use crate::workspace::WorkspaceAction;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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

/// A window as warpr runs it: Herdr tabs on, Warp's hosted agent off.
fn agent_panel_flags() -> impl Sized {
    (
        FeatureFlag::HerdrWorkspaceTabs.override_enabled(true),
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
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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

#[cfg(feature = "local_tty")]
#[test]
fn adding_a_tab_with_a_shell_creates_a_herdr_workspace_instead_of_a_local_tab() {
    use std::path::PathBuf;

    use crate::server::telemetry::AddTabWithShellSource;
    use crate::terminal::available_shells::AvailableShell;
    use crate::terminal::shell::ShellType;

    let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _herdr = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
    let _herdr = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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
fn the_user_cannot_create_groups_in_a_herdr_window() {
    let _herdr = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
    let _groups = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
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

    use herdr_fake::{PaneTree, TabScript};

    use super::*;
    use crate::code::buffer_location::LocalOrRemotePath;

    fn directory_named(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).expect("create directory");
        dunce::canonicalize(path).expect("canonical directory")
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
        let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
        App::test((), |mut app| async move {
            initialize_app(&mut app);
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
        let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let repo = directory_named(temp_dir.path(), "repo");
            let status = std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&repo)
                .status()
                .expect("run git");
            assert!(status.success(), "git init failed");
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
    fn a_herdr_workspace_has_a_tools_panel_to_open() {
        let _flag = FeatureFlag::HerdrWorkspaceTabs.override_enabled(true);
        App::test((), |mut app| async move {
            initialize_app(&mut app);
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
