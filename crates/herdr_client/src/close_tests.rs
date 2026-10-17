use herdr_fake::{
    FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, SplitDirection, TabScript,
};

use super::*;
use crate::{
    ConnectionStatus, HerdrCli, PresentedSession, SessionName, WorkspaceId, present_session,
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

    fn start(&self) -> FakeSession {
        let split = PaneTree::split(
            SplitDirection::Right,
            0.5,
            PaneTree::pane(),
            PaneTree::pane(),
        );
        self.fake.start_session(
            "main",
            SessionScript::new().workspace_with_tabs(
                "api",
                [
                    TabScript::new("one", split),
                    TabScript::new("two", PaneTree::pane()),
                ],
            ),
        )
    }

    fn presented(&self) -> PresentedSession {
        present_session(&HerdrCli::new(self.fake.binary_path()), &self.target)
    }

    fn pane_ids(&self, tab: &str) -> Vec<String> {
        let session = self.presented();
        let workspace = session.workspace(&WorkspaceId::from("w1"));
        workspace
            .into_iter()
            .flat_map(|workspace| &workspace.herdr_tabs)
            .filter(|candidate| candidate.label == tab)
            .filter_map(|tab| tab.layout.as_ref())
            .flat_map(|layout| layout.frames())
            .map(|pane| pane.id.as_str().to_owned())
            .collect()
    }
}

#[test]
fn a_closed_pane_leaves_its_herdr_tab_and_the_other_panes_keep_their_ids() {
    let harness = Harness::new();
    let _session = harness.start();

    close_pane(&harness.target, &PaneId::from("w1:p1")).expect("closed");

    assert_eq!(harness.presented().connection, ConnectionStatus::Connected);
    assert_eq!(harness.pane_ids("one"), ["w1:p2"]);
    assert_eq!(harness.pane_ids("two"), ["w1:p3"]);
}

#[test]
fn a_rejected_close_reports_herdrs_error_and_closes_nothing() {
    let harness = Harness::new();
    let session = harness.start();
    session.reject_next_close("pane_busy", "try again later");

    let failure = close_pane(&harness.target, &PaneId::from("w1:p1")).expect_err("rejected");

    assert!(
        matches!(&failure, ConnectionFailure::Rejected { message, .. } if message.contains("try again later")),
        "{failure:?}"
    );
    assert_eq!(harness.pane_ids("one"), ["w1:p1", "w1:p2"]);
}

#[test]
fn closing_a_pane_herdr_does_not_have_is_rejected() {
    let harness = Harness::new();
    let _session = harness.start();

    let failure = close_pane(&harness.target, &PaneId::from("w1:p9")).expect_err("rejected");

    assert!(
        matches!(&failure, ConnectionFailure::Rejected { message, .. } if message.contains("w1:p9")),
        "{failure:?}"
    );
}

#[test]
fn closing_a_pane_in_an_unreachable_session_fails_without_a_connection() {
    let harness = Harness::new();

    let failure = close_pane(&harness.target, &PaneId::from("w1:p1")).expect_err("unreachable");

    assert!(
        !matches!(failure, ConnectionFailure::Rejected { .. }),
        "{failure:?}"
    );
}
