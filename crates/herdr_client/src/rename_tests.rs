use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, TabScript};

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
        self.fake.start_session(
            "main",
            SessionScript::new().workspace_with_tabs(
                "api",
                [
                    TabScript::new("one", PaneTree::pane()),
                    TabScript::new("two", PaneTree::pane()),
                ],
            ),
        )
    }

    fn presented(&self) -> PresentedSession {
        present_session(&HerdrCli::new(self.fake.binary_path()), &self.target)
    }

    fn tab_labels(&self) -> Vec<String> {
        self.presented()
            .workspace(&WorkspaceId::from("w1"))
            .expect("workspace is presented")
            .herdr_tabs
            .iter()
            .map(|tab| tab.label.clone())
            .collect()
    }
}

#[test]
fn a_renamed_tab_is_presented_under_its_new_label_and_the_others_keep_theirs() {
    let harness = Harness::new();
    let _session = harness.start();

    rename(
        &harness.target,
        &RenameTarget::Tab(HerdrTabId::from("w1:t2")),
        "build",
    )
    .expect("renamed");

    assert_eq!(harness.presented().connection, ConnectionStatus::Connected);
    assert_eq!(harness.tab_labels(), ["one", "build"]);
}

#[test]
fn a_rejected_rename_reports_herdrs_error_and_renames_nothing() {
    let harness = Harness::new();
    let session = harness.start();
    session.reject_next_rename("tab_busy", "try again later");

    let failure = rename(
        &harness.target,
        &RenameTarget::Tab(HerdrTabId::from("w1:t2")),
        "build",
    )
    .expect_err("rejected");

    assert!(
        matches!(&failure, ConnectionFailure::Rejected { message, .. } if message.contains("try again later")),
        "{failure:?}"
    );
    assert_eq!(harness.tab_labels(), ["one", "two"]);
}

#[test]
fn renaming_a_tab_herdr_does_not_have_is_rejected() {
    let harness = Harness::new();
    let _session = harness.start();

    let failure = rename(
        &harness.target,
        &RenameTarget::Tab(HerdrTabId::from("w1:t9")),
        "build",
    )
    .expect_err("rejected");

    assert!(
        matches!(&failure, ConnectionFailure::Rejected { code, message } if code == "tab_not_found" && message.contains("w1:t9")),
        "{failure:?}"
    );
    assert_eq!(harness.tab_labels(), ["one", "two"]);
}

#[test]
fn renaming_in_an_unreachable_session_fails_without_a_connection() {
    let harness = Harness::new();

    let failure = rename(
        &harness.target,
        &RenameTarget::Tab(HerdrTabId::from("w1:t1")),
        "build",
    )
    .expect_err("unreachable");

    assert!(
        !matches!(failure, ConnectionFailure::Rejected { .. }),
        "{failure:?}"
    );
}
