use herdr_client::{
    ConnectionStatus, HerdrCli, HerdrTabId, SessionName, SessionTarget, WorkspaceId,
};
use herdr_fake::{
    FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, SplitDirection, TabScript,
};
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

use super::*;
use crate::app_state::HerdrWorkspaceSnapshot;
use crate::test_util::assert_eventually;

fn split_layout() -> PaneTree {
    PaneTree::split(
        SplitDirection::Right,
        0.3,
        PaneTree::pane(),
        PaneTree::split(
            SplitDirection::Down,
            0.5,
            PaneTree::pane(),
            PaneTree::pane(),
        ),
    )
}

fn compatible_fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
}

fn api_session(fake: &FakeHerdr) -> FakeSession {
    fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane()),
                TabScript::new("logs", split_layout()),
                TabScript::new("zoomed", split_layout())
                    .focused_pane(2)
                    .zoomed(),
            ],
        ),
    )
}

/// Opens a view of the fake session's workspace `w1` once the session has been presented.
fn open_workspace_view(app: &mut App, fake: &FakeHerdr) -> ViewHandle<HerdrWorkspaceView> {
    app.add_singleton_model(|_| Appearance::mock());
    let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::connect(HerdrCli::new(fake.binary_path()), target, ctx)
        });
    });
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        HerdrWorkspaceView::new(
            HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                workspace_id: WorkspaceId::from("w1"),
                label: "api".to_owned(),
            }),
            ctx,
        )
    });
    view
}

fn rendered_text(app: &App, view: &ViewHandle<HerdrWorkspaceView>) -> String {
    view.read(app, |view, ctx| {
        view.render(ctx).debug_text_content().unwrap_or_default()
    })
}

fn select(app: &mut App, view: &ViewHandle<HerdrWorkspaceView>, tab_id: &str) {
    app.update(|ctx| {
        view.update(ctx, |view, ctx| {
            view.handle_action(
                &HerdrWorkspaceViewAction::SelectTab(HerdrTabId::from(tab_id)),
                ctx,
            );
        });
    });
}

fn is_connected(app: &App) -> bool {
    HerdrSessionModel::handle(app).read(app, |model, _| {
        model.presented().connection == ConnectionStatus::Connected
    })
}

#[test]
fn the_view_shows_a_strip_of_the_workspaces_herdr_tabs_in_herdr_order() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        let text = rendered_text(&app, &view);

        assert!(
            text.starts_with("server logs zoomed\n"),
            "rendered text was {text:?}"
        );
    });
}

#[test]
fn the_tab_herdr_has_focused_shows_its_pane_frames_until_another_is_picked() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        assert_eq!(rendered_text(&app, &view), "server logs zoomed\nw1:p1");
    });
}

#[test]
fn picking_a_herdr_tab_shows_its_panes_in_herdrs_layout() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        select(&mut app, &view, "w1:t2");

        assert_eq!(
            rendered_text(&app, &view),
            "server logs zoomed\nw1:p2 w1:p3 w1:p4"
        );
    });
}

#[test]
fn a_zoomed_herdr_tab_shows_only_its_zoomed_pane() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        select(&mut app, &view, "w1:t3");

        assert_eq!(rendered_text(&app, &view), "server logs zoomed\nw1:p7");
    });
}

#[test]
fn picking_a_herdr_tab_sends_nothing_to_herdr() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        let served_before = session.served_methods();

        select(&mut app, &view, "w1:t2");

        assert_eq!(session.served_methods(), served_before);
        let herdr_active = HerdrSessionModel::handle(&app).read(&app, |model, _| {
            model.presented().workspaces[0].active_herdr_tab.clone()
        });
        assert_eq!(herdr_active, Some(HerdrTabId::from("w1:t1")));
    });
}

#[test]
fn herdr_tabs_and_panes_created_in_another_client_appear_without_a_refresh() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        session.set_script(
            SessionScript::new().workspace_with_tabs(
                "api",
                [
                    TabScript::new("server", PaneTree::pane()),
                    TabScript::new("logs", split_layout()),
                    TabScript::new("zoomed", split_layout())
                        .focused_pane(2)
                        .zoomed(),
                    TabScript::new("build", PaneTree::pane()),
                ],
            ),
        );
        session.emit_event("tab_created", serde_json::json!({}));

        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("server logs zoomed build\n"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );
    });
}

#[test]
fn a_layout_change_made_in_another_client_redraws_the_pane_frames() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        assert_eq!(rendered_text(&app, &view), "server logs zoomed\nw1:p1");

        session.set_script(
            SessionScript::new().workspace_with_tabs(
                "api",
                [
                    TabScript::new("server", split_layout()),
                    TabScript::new("logs", split_layout()),
                    TabScript::new("zoomed", split_layout())
                        .focused_pane(2)
                        .zoomed(),
                ],
            ),
        );
        session.emit_event("layout_updated", serde_json::json!({}));

        assert_eventually!(
            600 => rendered_text(&app, &view) == "server logs zoomed\nw1:p1 w1:p2 w1:p3",
            "rendered text was {:?}", rendered_text(&app, &view)
        );
    });
}

#[test]
fn a_lost_connection_is_shown_above_the_workspace_until_it_returns() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        session.go_offline();

        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("Lost the connection to Herdr, reconnecting"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );
        assert!(rendered_text(&app, &view).contains("server logs zoomed\nw1:p1"));

        session.go_online();

        assert_eventually!(
            600 => rendered_text(&app, &view) == "server logs zoomed\nw1:p1",
            "rendered text was {:?}", rendered_text(&app, &view)
        );
    });
}
