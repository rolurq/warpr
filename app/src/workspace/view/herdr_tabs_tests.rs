use herdr_client::{HerdrCli, Refusal, SessionName, SessionTarget};
use herdr_fake::{FakeHerdr, HerdrStatus, SessionScript};
use warp_core::features::FeatureFlag;
use warpui::{App, ViewHandle};

use super::*;
use crate::test_util::assert_eventually;
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
            200 => HerdrSessionModel::handle(&app).read(&app, |model, _| {
                matches!(model.presented().connection, ConnectionStatus::Failed(_))
            }),
            "connection never failed"
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
