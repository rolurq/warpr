use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use herdr_client::{
    ConnectionStatus, HerdrCli, HerdrTabId, PaneId, SessionName, SessionTarget, WorkspaceId,
};
use herdr_fake::{
    FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, SplitDirection, StreamKind,
    TabScript, TerminalStreamScript,
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

/// A Herdr whose panes can all be controlled, and print nothing until they are.
fn compatible_fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    for number in 1..=8 {
        fake.script_terminal_stream(
            StreamKind::Control,
            &format!("w1:p{number}"),
            [TerminalStreamScript::new()],
        );
    }
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

fn bar_labels(app: &App, view: &ViewHandle<HerdrWorkspaceView>) -> Vec<String> {
    view.read(app, |view, ctx| {
        view.bar_tabs(ctx)
            .into_iter()
            .map(|tab| tab.label)
            .collect()
    })
}

#[test]
fn the_view_lists_the_workspaces_herdr_tabs_in_herdr_order_for_the_tab_bar_and_shows_none_itself() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        assert_eq!(bar_labels(&app, &view), ["server", "logs", "zoomed"]);
        let text = rendered_text(&app, &view);
        assert!(
            !text.contains("server") && !text.contains('+'),
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

        assert_eq!(rendered_text(&app, &view), "w1:p1");
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

        assert_eq!(rendered_text(&app, &view), "w1:p2 w1:p3 w1:p4");
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

        assert_eq!(rendered_text(&app, &view), "w1:p7");
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
fn picking_a_pane_moves_only_this_views_selection() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");
        let served_before = session.served_methods();
        let runs_before = fake.invocations().len();

        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.handle_action(
                    &HerdrWorkspaceViewAction::SelectPane(PaneId::from("w1:p3")),
                    ctx,
                );
            });
        });

        assert_eq!(session.served_methods(), served_before);
        assert_eq!(fake.invocations().len(), runs_before);
        let selected = view.read(&app, |view, _| view.selected_pane.clone());
        assert_eq!(selected, Some(PaneId::from("w1:p3")));
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
            600 => bar_labels(&app, &view) == ["server", "logs", "zoomed", "build"],
            "the bar listed {:?}", bar_labels(&app, &view)
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
        assert_eq!(rendered_text(&app, &view), "w1:p1");

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
            600 => rendered_text(&app, &view) == "w1:p1 w1:p2 w1:p3",
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
        assert!(rendered_text(&app, &view).ends_with("w1:p1"));

        session.go_online();

        assert_eventually!(
            600 => rendered_text(&app, &view) == "w1:p1",
            "rendered text was {:?}", rendered_text(&app, &view)
        );
    });
}

fn api_with_tokens(tokens: &[(&str, &str)]) -> SessionScript {
    let script = SessionScript::new().workspace("api");
    tokens.iter().fold(script, |script, (name, value)| {
        script.workspace_token("api", name, value)
    })
}

#[test]
fn the_metadata_tokens_herdr_shows_appear_above_the_panes_and_follow_their_changes() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = fake.start_session("default", api_with_tokens(&[("vcs", "main")]));
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("vcs: main\nw1:p1"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );

        session.set_script(api_with_tokens(&[("vcs", "main*"), ("ci", "running")]));
        session.emit_event("workspace_metadata_updated", serde_json::json!({}));
        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("ci: running  vcs: main*\nw1:p1"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );

        session.set_script(api_with_tokens(&[("vcs", "main*")]));
        session.emit_event("workspace_metadata_updated", serde_json::json!({}));
        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("vcs: main*\nw1:p1"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );

        session.set_script(api_with_tokens(&[]));
        session.emit_event("workspace_metadata_updated", serde_json::json!({}));
        assert_eventually!(
            600 => rendered_text(&app, &view) == "w1:p1",
            "rendered text was {:?}", rendered_text(&app, &view)
        );
    });
}

#[test]
fn a_workspace_renamed_in_another_client_shows_its_new_title() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = fake.start_session("default", SessionScript::new().workspace("api"));
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        session.set_script(SessionScript::new().workspace("api-v2"));
        session.emit_event("workspace_renamed", serde_json::json!({}));

        assert_eventually!(
            600 => view.read(&app, |view, ctx| {
                view.pane_configuration().as_ref(ctx).title() == "api-v2"
            }),
            "the view kept its old title"
        );
    });
}

#[test]
fn a_shown_pane_is_controlled_at_the_cell_size_of_its_frame_and_hidden_panes_are_not() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let _view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        assert_eventually!(
            600 => fake.control_input("w1:p1").len() == 1,
            "the pane was never given a size"
        );

        let runs: Vec<_> = fake
            .invocations()
            .into_iter()
            .filter(|run| run.args.get(2).map(String::as_str) == Some("control"))
            .collect();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].args[3], "w1:p1");
        let resize = &fake.control_input("w1:p1")[0];
        assert_eq!(resize["type"], "terminal.resize");
        assert_eq!(resize["cols"].to_string(), runs[0].args[5]);
        assert_eq!(resize["rows"].to_string(), runs[0].args[7]);
        assert!(resize["cell_width_px"].as_u64().is_some_and(|px| px > 0));
        assert!(resize["cell_height_px"].as_u64().is_some_and(|px| px > 0));
    });
}

fn create_tab(app: &mut App, view: &ViewHandle<HerdrWorkspaceView>) {
    app.update(|ctx| {
        view.update(ctx, |view, ctx| {
            view.handle_action(&HerdrWorkspaceViewAction::CreateTab, ctx);
        });
    });
}

#[test]
fn creating_a_herdr_tab_in_this_workspace_shows_it_through_herdrs_state() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");

        create_tab(&mut app, &view);

        assert_eventually!(
            600 => rendered_text(&app, &view) == "w1:p8",
            "rendered text was {:?}", rendered_text(&app, &view)
        );
        let shown: Vec<_> = view.read(&app, |view, ctx| {
            view.bar_tabs(ctx)
                .into_iter()
                .filter(|tab| tab.is_shown)
                .map(|tab| tab.label)
                .collect()
        });
        assert_eq!(shown, ["4"]);
        assert_eq!(
            session
                .served_methods()
                .iter()
                .filter(|method| method.as_str() == "tab.create")
                .count(),
            1
        );
    });
}

#[test]
fn a_refused_tab_creation_shows_the_error_and_leaves_the_selection_alone() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");
        session.reject_next_creation("server_busy", "try again later");

        create_tab(&mut app, &view);

        assert_eventually!(
            600 => rendered_text(&app, &view).starts_with("Could not create a tab:"),
            "rendered text was {:?}", rendered_text(&app, &view)
        );
        let text = rendered_text(&app, &view);
        assert!(text.contains("try again later"), "{text:?}");
        assert!(text.ends_with("w1:p2 w1:p3 w1:p4"), "{text:?}");
    });
}

/// Workspace `api` (`w1`) with a different directory in each pane and one pane without any, and
/// workspace `web` (`w2`) with a single pane.
fn session_with_directories(fake: &FakeHerdr) -> FakeSession {
    fake.start_session("default", scripted_directories("/srv/logs/a"))
}

fn scripted_directories(first_logs_pane_cwd: &str) -> SessionScript {
    SessionScript::new()
        .workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane()).cwd(0, "/srv/api"),
                TabScript::new("logs", split_layout())
                    .cwd(0, first_logs_pane_cwd)
                    .cwd(1, "/srv/logs/b")
                    .without_cwd(2),
                TabScript::new("zoomed", split_layout())
                    .cwd(0, "/srv/zoomed/a")
                    .cwd(1, "/srv/zoomed/b")
                    .cwd(2, "/srv/zoomed/c")
                    .focused_pane(2)
                    .zoomed(),
            ],
        )
        .workspace_with_tabs(
            "web",
            [TabScript::new("site", PaneTree::pane()).cwd(0, "/srv/web")],
        )
}

fn directory(app: &App, view: &ViewHandle<HerdrWorkspaceView>) -> Option<PathBuf> {
    view.read(app, |view, ctx| {
        view.focused_pane_directory(ctx).map(Path::to_path_buf)
    })
}

fn select_pane(app: &mut App, view: &ViewHandle<HerdrWorkspaceView>, pane: &str) {
    app.update(|ctx| {
        view.update(ctx, |view, ctx| {
            view.handle_action(
                &HerdrWorkspaceViewAction::SelectPane(PaneId::from(pane)),
                ctx,
            );
        });
    });
}

#[test]
fn the_directory_follows_the_selected_pane_and_the_shown_herdr_tab() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        assert_eq!(directory(&app, &view), Some(PathBuf::from("/srv/api")));

        select(&mut app, &view, "w1:t2");
        assert_eq!(directory(&app, &view), Some(PathBuf::from("/srv/logs/a")));

        select_pane(&mut app, &view, "w1:p3");
        assert_eq!(directory(&app, &view), Some(PathBuf::from("/srv/logs/b")));

        select(&mut app, &view, "w1:t3");
        assert_eq!(directory(&app, &view), Some(PathBuf::from("/srv/zoomed/c")));
    });
}

#[test]
fn a_selected_pane_without_a_cwd_has_no_directory() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");

        select_pane(&mut app, &view, "w1:p4");

        assert_eq!(directory(&app, &view), None);
    });
}

#[test]
fn a_cwd_change_reported_by_herdr_changes_the_directory() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");
        assert_eq!(directory(&app, &view), Some(PathBuf::from("/srv/logs/a")));

        session.set_script(scripted_directories("/srv/logs/a/src"));
        session.emit_event("pane_updated", serde_json::json!({}));

        assert_eventually!(
            600 => directory(&app, &view) == Some(PathBuf::from("/srv/logs/a/src")),
            "the directory stayed {:?}", directory(&app, &view)
        );
    });
}

#[test]
fn each_workspace_view_has_the_directory_of_its_own_workspace() {
    App::test((), |mut app| async move {
        let mut fake = compatible_fake();
        fake.script_terminal_stream(StreamKind::Control, "w2:p1", [TerminalStreamScript::new()]);
        let _session = session_with_directories(&fake);
        let api = open_workspace_view(&mut app, &fake);
        let (_, web) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            HerdrWorkspaceView::new(
                HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                    workspace_id: WorkspaceId::from("w2"),
                    label: "web".to_owned(),
                }),
                ctx,
            )
        });
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");

        assert_eq!(directory(&app, &api), Some(PathBuf::from("/srv/api")));
        assert_eq!(directory(&app, &web), Some(PathBuf::from("/srv/web")));
    });
}

fn count_tab_changes(app: &mut App, view: &ViewHandle<HerdrWorkspaceView>) -> Rc<Cell<usize>> {
    let changes = Rc::new(Cell::new(0));
    app.update(|ctx| {
        let changes = changes.clone();
        ctx.subscribe_to_view(view, move |_, event, _| {
            if matches!(event, PaneEvent::HerdrTabChanged) {
                changes.set(changes.get() + 1);
            }
        });
    });
    changes
}

#[test]
fn the_view_tells_its_pane_group_when_the_shown_herdr_tab_changes_so_the_bar_redraws() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = api_session(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        let changes = count_tab_changes(&mut app, &view);

        select(&mut app, &view, "w1:t2");
        assert_eq!(changes.get(), 1);

        select_pane(&mut app, &view, "w1:p3");
        assert_eq!(changes.get(), 1, "picking a pane leaves the bar as it is");

        create_tab(&mut app, &view);
        assert_eventually!(600 => changes.get() == 2, "changes {}", changes.get());
    });
}

fn count_sidebar_changes(app: &mut App, view: &ViewHandle<HerdrWorkspaceView>) -> Rc<Cell<usize>> {
    let changes = Rc::new(Cell::new(0));
    let counter = changes.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(view, move |_, event, _| {
            if matches!(event, PaneEvent::SidebarDetailsChanged) {
                counter.set(counter.get() + 1);
            }
        });
    });
    changes
}

#[test]
fn the_sidebar_is_told_when_the_selection_or_a_snapshot_changes_the_directory() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        let changes = count_sidebar_changes(&mut app, &view);

        select(&mut app, &view, "w1:t2");
        assert_eq!(changes.get(), 1, "another Herdr tab, another directory");

        select_pane(&mut app, &view, "w1:p2");
        assert_eq!(changes.get(), 1, "same directory, nothing to redraw");

        select_pane(&mut app, &view, "w1:p3");
        assert_eq!(changes.get(), 2, "another pane, another directory");

        select_pane(&mut app, &view, "w1:p2");
        assert_eq!(changes.get(), 3);

        session.set_script(scripted_directories("/srv/logs/a/src"));
        session.emit_event("pane_updated", serde_json::json!({}));
        assert_eventually!(600 => changes.get() == 4, "changes {}", changes.get());
    });
}

/// Counts the times the view tells its pane group that the directory it presents changed.
fn count_directory_changes(
    app: &mut App,
    view: &ViewHandle<HerdrWorkspaceView>,
) -> Rc<Cell<usize>> {
    let changes = Rc::new(Cell::new(0));
    app.update(|ctx| {
        let changes = changes.clone();
        ctx.subscribe_to_view(view, move |_, event, _| {
            if matches!(event, PaneEvent::RepoChanged) {
                changes.set(changes.get() + 1);
            }
        });
    });
    changes
}

#[test]
fn the_view_reports_a_new_directory_when_the_selection_moves_to_another_one() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let _session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        let changes = count_directory_changes(&mut app, &view);

        select(&mut app, &view, "w1:t2");
        assert_eq!(changes.get(), 1, "another Herdr tab, another directory");

        select_pane(&mut app, &view, "w1:p3");
        assert_eq!(changes.get(), 2, "another pane, another directory");

        select_pane(&mut app, &view, "w1:p3");
        assert_eq!(changes.get(), 2, "the same pane again is no change");
    });
}

#[test]
fn the_view_reports_a_directory_change_that_a_later_herdr_snapshot_brings() {
    App::test((), |mut app| async move {
        let fake = compatible_fake();
        let session = session_with_directories(&fake);
        let view = open_workspace_view(&mut app, &fake);
        assert_eventually!(200 => is_connected(&app), "Herdr session never connected");
        select(&mut app, &view, "w1:t2");
        let changes = count_directory_changes(&mut app, &view);

        session.set_script(scripted_directories("/srv/logs/a/src"));
        session.emit_event("pane_updated", serde_json::json!({}));

        assert_eventually!(600 => changes.get() == 1, "{} changes reported", changes.get());
    });
}
