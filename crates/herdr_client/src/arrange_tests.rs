use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, TabScript};

use super::*;
use crate::{
    CellRect, ConnectionStatus, HerdrCli, PaneLayout, SessionName, WorkspaceId, present_session,
};

struct Harness {
    fake: FakeHerdr,
    target: SessionTarget,
}

impl Harness {
    fn new() -> Self {
        let mut fake = FakeHerdr::new();
        fake.script_status(&HerdrStatus::compatible());
        let target = SessionTarget::new(&fake.config_dir(), SessionName::new("main").unwrap());
        Self { fake, target }
    }

    /// Tab `one` holds `w1:p1` alone, tab `two` holds `w1:p2` on the left of `w1:p3`.
    fn start(&self) -> FakeSession {
        let split = PaneTree::split(
            herdr_fake::SplitDirection::Right,
            0.5,
            PaneTree::pane(),
            PaneTree::pane(),
        );
        self.fake.start_session(
            "main",
            SessionScript::new().workspace_with_tabs(
                "api",
                [
                    TabScript::new("one", PaneTree::pane()),
                    TabScript::new("two", split),
                ],
            ),
        )
    }

    fn layout(&self, tab: &str) -> PaneLayout {
        let session = present_session(&HerdrCli::new(self.fake.binary_path()), &self.target);
        assert_eq!(session.connection, ConnectionStatus::Connected);
        session
            .workspace(&WorkspaceId::from("w1"))
            .into_iter()
            .flat_map(|workspace| &workspace.herdr_tabs)
            .find(|candidate| candidate.label == tab)
            .and_then(|tab| tab.layout.clone())
            .unwrap_or_else(|| panic!("no layout for tab {tab}"))
    }

    fn rects(&self, tab: &str) -> Vec<(String, CellRect)> {
        self.layout(tab)
            .frames()
            .into_iter()
            .map(|pane| (pane.id.as_str().to_owned(), pane.rect))
            .collect()
    }
}

fn rect(x: u16, y: u16, width: u16, height: u16) -> CellRect {
    CellRect {
        x,
        y,
        width,
        height,
    }
}

fn owned(rects: &[(&str, CellRect)]) -> Vec<(String, CellRect)> {
    rects
        .iter()
        .map(|(id, rect)| ((*id).to_owned(), *rect))
        .collect()
}

#[test]
fn splitting_right_puts_a_new_pane_right_of_the_pane_and_leaves_herdrs_focus_on_it() {
    let harness = Harness::new();
    let _session = harness.start();

    split_pane(
        &harness.target,
        &PaneId::from("w1:p1"),
        SplitDirection::Right,
    )
    .expect("split");

    assert_eq!(
        harness.rects("one"),
        owned(&[
            ("w1:p1", rect(0, 0, 60, 40)),
            ("w1:p4", rect(60, 0, 60, 40))
        ]),
        "the new pane takes the next number of the workspace"
    );
    assert_eq!(harness.layout("one").focused_pane, PaneId::from("w1:p1"));
    assert_eq!(
        harness.rects("two"),
        owned(&[
            ("w1:p2", rect(0, 0, 60, 40)),
            ("w1:p3", rect(60, 0, 60, 40))
        ]),
    );
}

#[test]
fn splitting_down_puts_the_new_pane_below_and_keeps_the_other_panes_in_place() {
    let harness = Harness::new();
    let _session = harness.start();

    split_pane(
        &harness.target,
        &PaneId::from("w1:p3"),
        SplitDirection::Down,
    )
    .expect("split");

    assert_eq!(
        harness.rects("two"),
        owned(&[
            ("w1:p2", rect(0, 0, 60, 40)),
            ("w1:p3", rect(60, 0, 60, 20)),
            ("w1:p4", rect(60, 20, 60, 20)),
        ]),
    );
}

#[test]
fn zooming_a_pane_focuses_it_and_shows_it_alone_until_the_tab_is_unzoomed() {
    let harness = Harness::new();
    let _session = harness.start();

    set_pane_zoom(&harness.target, &PaneId::from("w1:p3"), true).expect("zoomed");

    let layout = harness.layout("two");
    assert!(layout.zoomed);
    assert_eq!(layout.focused_pane, PaneId::from("w1:p3"));
    assert_eq!(
        harness.rects("two"),
        owned(&[("w1:p3", rect(0, 0, 120, 40))])
    );

    set_pane_zoom(&harness.target, &PaneId::from("w1:p3"), false).expect("unzoomed");

    let layout = harness.layout("two");
    assert!(!layout.zoomed);
    assert_eq!(layout.focused_pane, PaneId::from("w1:p3"));
    assert_eq!(harness.rects("two").len(), 2);
}

#[test]
fn zooming_a_tabs_only_pane_leaves_it_unzoomed() {
    let harness = Harness::new();
    let _session = harness.start();

    set_pane_zoom(&harness.target, &PaneId::from("w1:p1"), true).expect("answered");

    assert!(!harness.layout("one").zoomed);
}

#[test]
fn resizing_moves_the_divider_one_step_in_the_direction_whichever_side_the_pane_is_on() {
    let harness = Harness::new();
    let _session = harness.start();

    resize_pane(
        &harness.target,
        &PaneId::from("w1:p3"),
        PaneDirection::Right,
    )
    .expect("resized");

    assert_eq!(
        harness.rects("two"),
        owned(&[
            ("w1:p2", rect(0, 0, 66, 40)),
            ("w1:p3", rect(66, 0, 54, 40))
        ]),
    );

    resize_pane(&harness.target, &PaneId::from("w1:p2"), PaneDirection::Left).expect("resized");

    assert_eq!(
        harness.rects("two"),
        owned(&[
            ("w1:p2", rect(0, 0, 60, 40)),
            ("w1:p3", rect(60, 0, 60, 40))
        ]),
    );
}

#[test]
fn a_rejected_layout_change_reports_herdrs_error_and_changes_nothing() {
    let harness = Harness::new();
    let session = harness.start();
    let pane = PaneId::from("w1:p2");

    for name in ["split", "zoom", "resize"] {
        session.reject_next_layout_change("pane_busy", "try again later");

        let result = match name {
            "split" => split_pane(&harness.target, &pane, SplitDirection::Right),
            "zoom" => set_pane_zoom(&harness.target, &pane, true),
            _ => resize_pane(&harness.target, &pane, PaneDirection::Right),
        };
        let failure = result.expect_err(name);

        assert!(
            matches!(&failure, ConnectionFailure::Rejected { message, .. } if message.contains("try again later")),
            "{name}: {failure:?}"
        );
        let layout = harness.layout("two");
        assert!(!layout.zoomed, "{name}");
        assert_eq!(
            harness.rects("two"),
            owned(&[
                ("w1:p2", rect(0, 0, 60, 40)),
                ("w1:p3", rect(60, 0, 60, 40))
            ]),
            "{name}"
        );
    }
}

#[test]
fn changing_the_layout_around_a_pane_herdr_does_not_have_is_rejected() {
    let harness = Harness::new();
    let _session = harness.start();
    let missing = PaneId::from("w1:p9");

    for failure in [
        split_pane(&harness.target, &missing, SplitDirection::Down),
        set_pane_zoom(&harness.target, &missing, true),
        resize_pane(&harness.target, &missing, PaneDirection::Up),
    ] {
        let failure = failure.expect_err("rejected");
        assert!(
            matches!(&failure, ConnectionFailure::Rejected { message, .. } if message.contains("w1:p9")),
            "{failure:?}"
        );
    }
}
