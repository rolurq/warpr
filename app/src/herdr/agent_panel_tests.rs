use herdr_client::{AgentState, HerdrCli, SessionName, SessionTarget, WorkspaceId};
use herdr_fake::{
    AgentScript, FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, SplitDirection,
    TabScript,
};
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

use super::*;
use crate::app_state::{HerdrWorkspaceContents, HerdrWorkspaceSnapshot};
use crate::herdr::HerdrWorkspaceView;
use crate::test_util::assert_eventually;

fn two_panes() -> PaneTree {
    PaneTree::split(
        SplitDirection::Right,
        0.5,
        PaneTree::pane(),
        PaneTree::pane(),
    )
}

/// How the fake scripts an agent that is in `state` as the client presents it.
fn scripted(state: AgentState) -> herdr_fake::AgentState {
    match state {
        AgentState::Idle => herdr_fake::AgentState::Idle,
        AgentState::Working => herdr_fake::AgentState::Working,
        AgentState::Blocked => herdr_fake::AgentState::Blocked,
        AgentState::Done => herdr_fake::AgentState::Done,
        AgentState::Unknown => herdr_fake::AgentState::Unknown,
    }
}

/// Workspace `api` (`w1`) has the Herdr tabs `server` (pane `w1:p1`, a named agent) and `logs`
/// (panes `w1:p2` and `w1:p3`, an unnamed agent in the second).
fn fleet(builder: AgentState) -> SessionScript {
    SessionScript::new().workspace_with_tabs(
        "api",
        [
            TabScript::new("server", PaneTree::pane()).agent(
                0,
                AgentScript::new("claude", scripted(builder)).named("builder"),
            ),
            TabScript::new("logs", two_panes())
                .agent(1, AgentScript::new("codex", scripted(AgentState::Idle))),
        ],
    )
}

fn compatible_fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
}

fn connect(app: &mut App, fake: &FakeHerdr) {
    app.add_singleton_model(|_| Appearance::mock());
    let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::connect(HerdrCli::new(fake.binary_path()), target, ctx)
        });
    });
}

fn open_panel(app: &mut App, fake: &FakeHerdr) -> ViewHandle<HerdrAgentPanelView> {
    connect(app, fake);
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, HerdrAgentPanelView::new);
    view
}

fn rows(app: &App) -> Vec<AgentRow> {
    app.read(HerdrAgentPanelView::rows)
}

fn row(pane: &str, title: &str, state: AgentState, detail: &str) -> AgentRow {
    AgentRow {
        pane: pane.into(),
        title: title.to_owned(),
        state,
        detail: detail.to_owned(),
    }
}

fn state_of(app: &App, pane: &str) -> Option<AgentState> {
    rows(app)
        .into_iter()
        .find(|row| row.pane.as_str() == pane)
        .map(|row| row.state)
}

fn wait_for_agent_subscriptions(session: &FakeSession) {
    let start = instant::Instant::now();
    while session.agent_status_subscriptions().is_empty() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the agents never subscribed"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn every_agent_is_listed_with_its_name_kind_state_and_location() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = fake.start_session("default", fleet(AgentState::Blocked));
        let _panel = open_panel(&mut app, &fake);

        let expected = [
            row(
                "w1:p1",
                "builder",
                AgentState::Blocked,
                "claude · api › server › w1:p1",
            ),
            row(
                "w1:p3",
                "codex",
                AgentState::Idle,
                "codex · api › logs › w1:p3",
            ),
        ];

        // The rows exist as soon as the agents are listed, but name their workspace and tab by
        // id until the session's own presentation, which arrives separately, has the labels.
        assert_eventually!(
            600 => rows(&app) == expected,
            "rows were {:?}", rows(&app)
        );
    });
}

#[test]
fn an_agent_state_change_appears_in_the_panel_without_a_refresh() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = fake.start_session("default", fleet(AgentState::Working));
        let _panel = open_panel(&mut app, &fake);
        assert_eventually!(
            600 => state_of(&app, "w1:p1") == Some(AgentState::Working),
            "rows were {:?}", rows(&app)
        );
        wait_for_agent_subscriptions(&session);

        session.set_script(fleet(AgentState::Blocked));
        session.emit_agent_status_changed("w1:p1", "claude", scripted(AgentState::Blocked));

        assert_eventually!(
            600 => state_of(&app, "w1:p1") == Some(AgentState::Blocked),
            "rows were {:?}", rows(&app)
        );
    });
}

#[test]
fn an_agent_that_appears_or_goes_away_is_listed_or_dropped() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = fake.start_session("default", fleet(AgentState::Working));
        let _panel = open_panel(&mut app, &fake);
        assert_eventually!(600 => rows(&app).len() == 2, "rows were {:?}", rows(&app));

        session.set_script(fleet(AgentState::Working).agent_in(
            "api",
            "logs",
            0,
            AgentScript::new("pi", scripted(AgentState::Done)),
        ));
        session.emit_event("pane_agent_detected", serde_json::json!({}));
        assert_eventually!(
            600 => state_of(&app, "w1:p2") == Some(AgentState::Done),
            "rows were {:?}", rows(&app)
        );

        session.set_script(fleet(AgentState::Working));
        session.emit_event("pane_agent_detected", serde_json::json!({}));
        assert_eventually!(
            600 => state_of(&app, "w1:p2").is_none(),
            "rows were {:?}", rows(&app)
        );
    });
}

fn open_workspace_view(app: &mut App) -> ViewHandle<HerdrWorkspaceView> {
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

fn workspace_text(app: &App, view: &ViewHandle<HerdrWorkspaceView>) -> String {
    view.read(app, |view, ctx| {
        view.render(ctx).debug_text_content().unwrap_or_default()
    })
}

#[test]
fn selecting_an_agent_shows_its_herdr_tab_and_pane_without_moving_herdrs_focus() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = fake.start_session("default", fleet(AgentState::Working));
        let panel = open_panel(&mut app, &fake);
        let workspace_view = open_workspace_view(&mut app);
        assert_eventually!(600 => rows(&app).len() == 2, "rows were {:?}", rows(&app));
        wait_for_agent_subscriptions(&session);
        assert_eventually!(
            600 => workspace_text(&app, &workspace_view).starts_with("server logs +\nw1:p1"),
            "text was {:?}", workspace_text(&app, &workspace_view)
        );

        app.update(|ctx| {
            panel.update(ctx, |panel, ctx| {
                panel.handle_action(&HerdrAgentPanelAction::Jump("w1:p3".into()), ctx);
            });
        });

        assert_eq!(
            workspace_text(&app, &workspace_view),
            "server logs +\nw1:p2 w1:p3"
        );
        let moved_focus = session
            .served_methods()
            .iter()
            .any(|method| method.ends_with(".focus") || method.ends_with(".focused"));
        assert!(!moved_focus, "{:?}", session.served_methods());
        let herdr_active = HerdrSessionModel::handle(&app).read(&app, |model, _| {
            model.presented().workspaces[0].active_herdr_tab.clone()
        });
        assert_eq!(herdr_active, Some("w1:t1".into()));
    });
}

#[test]
fn an_agent_the_session_does_not_present_cannot_be_jumped_to() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = fake.start_session("default", fleet(AgentState::Working));
        let _panel = open_panel(&mut app, &fake);
        assert_eventually!(600 => rows(&app).len() == 2, "rows were {:?}", rows(&app));

        let target = HerdrSessionModel::handle(&app)
            .read(&app, |model, _| model.agent_jump_target(&"w9:p1".into()));

        assert_eq!(target, None);
    });
}
