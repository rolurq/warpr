use herdr_fake::{FakeHerdr, PaneTree, SessionScript, SplitDirection, TabScript};

use crate::{
    CellRect, ConnectionStatus, HerdrTabId, PaneId, PresentedHerdrTab, PresentedSession,
    SessionName, SessionTarget, present_session,
};

fn present(fake: &FakeHerdr) -> PresentedSession {
    let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
    let presented = present_session(&target);
    assert_eq!(presented.connection, ConnectionStatus::Connected);
    presented
}

fn tab(label: &str) -> TabScript {
    TabScript::new(label, PaneTree::pane())
}

fn rect(x: u16, y: u16, width: u16, height: u16) -> CellRect {
    CellRect {
        x,
        y,
        width,
        height,
    }
}

fn tab_labels(tabs: &[PresentedHerdrTab]) -> Vec<&str> {
    tabs.iter().map(|tab| tab.label.as_str()).collect()
}

fn frames_of(tab: &PresentedHerdrTab) -> Vec<(String, CellRect)> {
    let layout = tab.layout.as_ref().expect("tab has a layout");
    layout
        .frames()
        .into_iter()
        .map(|frame| (frame.id.as_str().to_owned(), frame.rect))
        .collect()
}

fn assert_frames(tab: &PresentedHerdrTab, expected: &[(&str, CellRect)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|(id, rect)| ((*id).to_owned(), *rect))
        .collect();
    assert_eq!(frames_of(tab), expected);
}

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

#[test]
fn each_workspace_presents_its_herdr_tabs_in_herdr_order_with_herdr_labels() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace_with_tabs("api", [tab("server"), tab("logs"), tab("scratch")])
            .workspace_with_tabs("web", [tab("build"), tab("serve")]),
    );

    let presented = present(&fake);

    assert_eq!(
        tab_labels(&presented.workspaces[0].herdr_tabs),
        ["server", "logs", "scratch"]
    );
    assert_eq!(
        tab_labels(&presented.workspaces[1].herdr_tabs),
        ["build", "serve"]
    );
}

#[test]
fn a_workspace_without_scripted_tabs_presents_herdrs_single_tab_with_one_pane() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session("default", SessionScript::new().workspace("api"));

    let presented = present(&fake);

    let [only_tab] = presented.workspaces[0].herdr_tabs.as_slice() else {
        panic!("expected one Herdr tab");
    };
    assert_eq!(only_tab.label, "1");
    assert_frames(only_tab, &[("w1:p1", rect(0, 0, 120, 40))]);
}

#[test]
fn the_herdr_tab_herdr_has_focused_is_presented_as_active_in_every_workspace() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace_with_tabs("api", [tab("server"), tab("logs")])
            .workspace_with_tabs("web", [tab("build"), tab("serve")])
            .active_tab("api", "logs")
            .active_tab("web", "serve")
            .focused("api"),
    );

    let presented = present(&fake);

    let active_label = |index: usize| {
        let workspace = &presented.workspaces[index];
        workspace
            .active_herdr_tab
            .as_ref()
            .and_then(|id| workspace.herdr_tabs.iter().find(|tab| &tab.id == id))
            .map(|tab| tab.label.as_str())
    };
    assert_eq!(active_label(0), Some("logs"));
    assert_eq!(active_label(1), Some("serve"));
}

#[test]
fn a_workspaces_first_herdr_tab_is_active_unless_herdr_says_otherwise() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs("api", [tab("server"), tab("logs")]),
    );

    let presented = present(&fake);

    let workspace = &presented.workspaces[0];
    assert_eq!(
        workspace.selected_tab(None).map(|tab| tab.label.as_str()),
        Some("server")
    );
    assert_eq!(workspace.active_herdr_tab, Some(HerdrTabId::from("w1:t1")));
}

#[test]
fn pane_frames_carry_the_rects_herdr_computed_for_the_tab() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new("split", split_layout()),
                TabScript::new(
                    "beside",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::pane(),
                    ),
                ),
            ],
        ),
    );

    let presented = present(&fake);

    let [split, beside] = presented.workspaces[0].herdr_tabs.as_slice() else {
        panic!("expected two Herdr tabs");
    };
    assert_frames(
        split,
        &[
            ("w1:p1", rect(0, 0, 36, 40)),
            ("w1:p2", rect(36, 0, 84, 20)),
            ("w1:p3", rect(36, 20, 84, 20)),
        ],
    );
    assert_frames(
        beside,
        &[
            ("w1:p4", rect(0, 0, 60, 40)),
            ("w1:p5", rect(60, 0, 60, 40)),
        ],
    );
}

#[test]
fn a_zoomed_tab_presents_only_its_focused_pane_filling_the_layout_area() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new("zoomed", split_layout())
                    .focused_pane(2)
                    .zoomed(),
                TabScript::new("plain", split_layout()).focused_pane(2),
            ],
        ),
    );

    let presented = present(&fake);

    let [zoomed, plain] = presented.workspaces[0].herdr_tabs.as_slice() else {
        panic!("expected two Herdr tabs");
    };
    assert_frames(zoomed, &[("w1:p3", rect(0, 0, 120, 40))]);
    assert_eq!(frames_of(plain).len(), 3);
}

#[test]
fn zoom_is_presented_per_herdr_tab() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new("plain", split_layout()),
                TabScript::new("zoomed", split_layout()).zoomed(),
            ],
        ),
    );

    let presented = present(&fake);

    let layouts: Vec<_> = presented.workspaces[0]
        .herdr_tabs
        .iter()
        .map(|tab| tab.layout.as_ref().expect("layout").zoomed)
        .collect();
    assert_eq!(layouts, [false, true]);
}

#[test]
fn picking_a_herdr_tab_shows_it_without_asking_herdr_to_focus_it() {
    let fake = FakeHerdr::new();
    let session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs("api", [tab("server"), tab("logs")]),
    );
    let logs = HerdrTabId::from("w1:t2");

    let presented = present(&fake);
    let shown = presented.workspaces[0]
        .selected_tab(Some(&logs))
        .map(|tab| tab.label.clone());
    let presented_again = present(&fake);

    assert_eq!(shown.as_deref(), Some("logs"));
    assert_eq!(
        presented_again.workspaces[0].active_herdr_tab,
        Some(HerdrTabId::from("w1:t1"))
    );
    assert_eq!(
        session.served_methods(),
        ["session.snapshot", "session.snapshot"]
    );
}

#[test]
fn a_pick_of_a_herdr_tab_that_no_longer_exists_falls_back_to_herdrs_active_tab() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace_with_tabs("api", [tab("server"), tab("logs")])
            .active_tab("api", "logs"),
    );
    let closed = HerdrTabId::from("w1:t9");

    let presented = present(&fake);

    assert_eq!(
        presented.workspaces[0]
            .selected_tab(Some(&closed))
            .map(|tab| tab.label.as_str()),
        Some("logs")
    );
}

#[test]
fn a_pane_id_is_the_one_herdr_reports() {
    let fake = FakeHerdr::new();
    let _session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs("api", [TabScript::new("t", split_layout())]),
    );

    let presented = present(&fake);

    let layout = presented.workspaces[0].herdr_tabs[0]
        .layout
        .as_ref()
        .expect("layout");
    assert_eq!(layout.focused_pane, PaneId::from("w1:p1"));
    assert_eq!(layout.area, rect(0, 0, 120, 40));
}
