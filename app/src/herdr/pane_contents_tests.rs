use herdr_client::{
    Contest, GridSize, HerdrCli, PaneId, PaneOwnership, PaneSize, SessionName, SessionTarget,
};
use herdr_fake::{
    FakeFrame, FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, StreamKind,
    TerminalStreamScript,
};
use warp_core::ui::appearance::Appearance;
use warpui::{App, EntityId, ModelHandle};

use super::*;
use crate::test_util::assert_eventually;

const PANE: &str = "w1:p1";
const REFUSED: &str = "terminal attach failed: terminal term_ab already has an attached client; retry with --takeover";

fn size() -> PaneSize {
    PaneSize {
        cols: 20,
        rows: 4,
        cell_width_px: 9,
        cell_height_px: 18,
    }
}

fn fake_with_session() -> (FakeHerdr, FakeSession) {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    let session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace_with_tabs("api", [herdr_fake::TabScript::new("1", PaneTree::pane())]),
    );
    (fake, session)
}

fn frame(seq: u64, full: bool, text: &str) -> FakeFrame {
    let bytes = format!("\x1b[2J\x1b[1;1H{text}");
    if full {
        FakeFrame::full(seq, 20, 4, bytes)
    } else {
        FakeFrame::incremental(seq, 20, 4, text)
    }
}

/// The registry of a connected session, with the pane shown by a view that never existed.
async fn show_pane(app: &mut App, fake: &FakeHerdr) -> ModelHandle<HerdrPaneContents> {
    app.add_singleton_model(|_| Appearance::mock());
    let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::connect(HerdrCli::new(fake.binary_path()), target, ctx)
        });
        ctx.add_singleton_model(HerdrPaneContentsRegistry::new);
    });
    let registry = HerdrPaneContentsRegistry::handle(app);
    assert_eventually!(200 => registry.read(app, |registry, _| {
        registry.contents(&PaneId::from(PANE)).is_some()
    }), "the pane never got contents");
    let contents = registry.read(app, |registry, _| {
        registry.contents(&PaneId::from(PANE)).expect("contents")
    });
    app.update(|ctx| {
        registry.update(ctx, |registry, ctx| {
            registry.set_shown(EntityId::new(), vec![PaneId::from(PANE)], ctx);
        });
        contents.read(ctx, |contents, _| contents.stream().set_size(size()));
    });
    contents
}

/// The pane's lines without the blank rows at its bottom, which the grid's text may or may not
/// include.
fn text(app: &App, contents: &ModelHandle<HerdrPaneContents>) -> Vec<String> {
    contents.read(app, |contents, _| {
        let mut lines: Vec<String> = contents
            .terminal()
            .map(|terminal| {
                terminal
                    .text()
                    .lines()
                    .map(|line| line.trim_end().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    })
}

fn ownership(app: &App, contents: &ModelHandle<HerdrPaneContents>) -> Option<PaneOwnership> {
    contents.read(app, |contents, _| contents.ownership().cloned())
}

#[test]
fn a_free_pane_shows_its_frames_as_grid_text() {
    App::test((), |mut app| async move {
        let (mut fake, _session) = fake_with_session();
        fake.script_terminal_stream(
            StreamKind::Control,
            PANE,
            [TerminalStreamScript::new()
                .frame(frame(1, true, "prompt$ ls"))
                .frame(frame(2, false, "\x1b[2;1Hfile.txt"))
                .wait_for_gate("repaint")
                .frame(FakeFrame::full(3, 20, 4, "\x1b[2J\x1b[1;1Hrepainted"))],
        );
        let contents = show_pane(&mut app, &fake).await;

        assert_eventually!(200 => text(&app, &contents).first().is_some_and(|line| line == "prompt$ ls"), "first frame");
        assert_eventually!(200 => text(&app, &contents).get(1).is_some_and(|line| line == "file.txt"), "incremental frame");
        assert_eq!(ownership(&app, &contents), Some(PaneOwnership::Owned));

        fake.open_gate("repaint");
        assert_eventually!(200 => text(&app, &contents) == ["repainted"], "mid-stream full frame");
    });
}

#[test]
fn a_contested_pane_is_watched_at_herdrs_grid_and_marked() {
    App::test((), |mut app| async move {
        let (mut fake, _session) = fake_with_session();
        fake.script_terminal_stream(
            StreamKind::Control,
            PANE,
            [TerminalStreamScript::new().closed(REFUSED)],
        );
        fake.script_terminal_stream(
            StreamKind::Observe,
            PANE,
            [TerminalStreamScript::new().frame(FakeFrame::full(
                1,
                120,
                40,
                "\x1b[2J\x1b[1;1Hother owner",
            ))],
        );
        let contents = show_pane(&mut app, &fake).await;

        assert_eventually!(200 => ownership(&app, &contents)
            == Some(PaneOwnership::Contested(Contest::Refused)), "contested");
        assert_eventually!(200 => text(&app, &contents).first().is_some_and(|line| line == "other owner"), "observed text");
        let observed = fake
            .invocations()
            .into_iter()
            .find(|run| run.args.get(2).map(String::as_str) == Some("observe"))
            .expect("an observe run");
        // Herdr's layout rect for a lone pane in the fake session is its whole 120×40 area.
        assert_eq!(observed.args[4..], ["--cols", "120", "--rows", "40"]);
        let grid = GridSize {
            cols: 120,
            rows: 40,
        };
        assert_eq!(
            contents.read(&app, |contents, _| contents
                .terminal()
                .map(|terminal| (terminal.cols(), terminal.rows()))),
            Some((grid.cols, grid.rows))
        );
    });
}

#[test]
fn an_exited_pane_reports_it() {
    App::test((), |mut app| async move {
        let (mut fake, _session) = fake_with_session();
        fake.script_terminal_stream(
            StreamKind::Control,
            PANE,
            [TerminalStreamScript::new()
                .frame(frame(1, true, "bye"))
                .closed("terminal term_ab exited")],
        );
        let contents = show_pane(&mut app, &fake).await;

        assert_eventually!(200 => ownership(&app, &contents) == Some(PaneOwnership::Exited), "exit");
        assert_eq!(text(&app, &contents)[0], "bye");
    });
}

#[test]
fn a_pane_nobody_shows_is_not_streamed() {
    App::test((), |mut app| async move {
        let (fake, _session) = fake_with_session();
        app.add_singleton_model(|_| Appearance::mock());
        let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
        app.update(|ctx| {
            ctx.add_singleton_model(|ctx| {
                HerdrSessionModel::connect(HerdrCli::new(fake.binary_path()), target, ctx)
            });
            ctx.add_singleton_model(HerdrPaneContentsRegistry::new);
        });
        let registry = HerdrPaneContentsRegistry::handle(&app);
        assert_eventually!(200 => registry.read(&app, |registry, _| {
            registry.contents(&PaneId::from(PANE)).is_some()
        }), "the pane never got contents");
        registry.read(&app, |registry, _| {
            let contents = registry.contents(&PaneId::from(PANE)).expect("contents");
            contents.read(&app, |contents, _| contents.stream().set_size(size()));
        });

        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            fake.invocations()
                .iter()
                .all(|run| run.args.first().map(String::as_str) != Some("terminal"))
        );
    });
}
