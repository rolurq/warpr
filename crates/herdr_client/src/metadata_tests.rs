use std::sync::Arc;
use std::thread;
use std::time::Duration;

use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, SessionScript};
use instant::Instant;
use parking_lot::Mutex;
use serde_json::json;

use crate::{
    HerdrCli, LiveSession, MetadataToken, PresentedSession, ReconnectPolicy, SessionName,
    SessionTarget, WorkspaceId, present_session,
};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
}

fn target(fake: &FakeHerdr) -> SessionTarget {
    SessionTarget::new(
        &fake.config_dir(),
        SessionName::new("default").expect("valid name"),
    )
}

fn token(name: &str, value: &str) -> MetadataToken {
    MetadataToken {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

fn tokens_of(presented: &PresentedSession, workspace: &str) -> Vec<MetadataToken> {
    presented
        .workspace(&WorkspaceId::from(workspace))
        .unwrap_or_else(|| panic!("no workspace {workspace}"))
        .tokens
        .clone()
}

fn label_of(presented: &PresentedSession, workspace: &str) -> String {
    presented
        .workspace(&WorkspaceId::from(workspace))
        .unwrap_or_else(|| panic!("no workspace {workspace}"))
        .label
        .clone()
}

struct Observed {
    _live: LiveSession,
    latest: Arc<Mutex<PresentedSession>>,
}

fn observe(fake: &FakeHerdr) -> Observed {
    let latest = Arc::new(Mutex::new(PresentedSession::connecting()));
    let live = LiveSession::start(
        HerdrCli::new(fake.binary_path()),
        target(fake),
        ReconnectPolicy {
            initial_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(20),
            disconnected_after: 3,
        },
        {
            let latest = Arc::clone(&latest);
            move |presented| *latest.lock() = presented
        },
    );
    Observed {
        _live: live,
        latest,
    }
}

impl Observed {
    fn wait_for(&self, expectation: &str, matches: impl Fn(&PresentedSession) -> bool) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while !matches(&self.latest.lock()) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {expectation}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_for_tokens(&self, workspace: &str, expected: &[MetadataToken]) {
        self.wait_for(
            &format!("tokens {expected:?} on {workspace}"),
            |presented| {
                presented
                    .workspace(&WorkspaceId::from(workspace))
                    .map(|w| &w.tokens[..])
                    == Some(expected)
            },
        );
    }
}

fn start_session(fake: &FakeHerdr, script: SessionScript) -> FakeSession {
    fake.start_session("default", script)
}

fn announce_metadata(session: &FakeSession) {
    session.emit_event(
        "workspace_metadata_updated",
        json!({ "type": "workspace_metadata_updated" }),
    );
}

#[test]
fn a_workspace_presents_the_tokens_herdr_reports_sorted_by_name() {
    let fake = fake();
    let _session = start_session(
        &fake,
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main*")
            .workspace_token("api", "ci", "passing")
            .workspace("docs"),
    );

    let presented = present_session(&HerdrCli::new(fake.binary_path()), &target(&fake));

    assert_eq!(
        tokens_of(&presented, "w1"),
        [token("ci", "passing"), token("vcs", "main*")]
    );
    assert_eq!(tokens_of(&presented, "w2"), []);
}

#[test]
fn a_token_set_again_presents_its_latest_value_only() {
    let fake = fake();
    let _session = start_session(
        &fake,
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main")
            .workspace_token("api", "vcs", "main*"),
    );

    let presented = present_session(&HerdrCli::new(fake.binary_path()), &target(&fake));

    assert_eq!(tokens_of(&presented, "w1"), [token("vcs", "main*")]);
}

#[test]
fn a_changed_token_appears_when_herdr_announces_it() {
    let fake = fake();
    let session = start_session(
        &fake,
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main"),
    );
    let observed = observe(&fake);
    observed.wait_for_tokens("w1", &[token("vcs", "main")]);

    session.set_script(
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main*")
            .workspace_token("api", "ci", "running"),
    );
    announce_metadata(&session);

    observed.wait_for_tokens("w1", &[token("ci", "running"), token("vcs", "main*")]);
}

#[test]
fn an_expired_token_disappears_when_herdr_announces_the_expiry() {
    let fake = fake();
    let session = start_session(
        &fake,
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main*")
            .workspace_token("api", "ci", "running"),
    );
    let observed = observe(&fake);
    observed.wait_for_tokens("w1", &[token("ci", "running"), token("vcs", "main*")]);

    session.set_script(
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main*"),
    );
    announce_metadata(&session);
    observed.wait_for_tokens("w1", &[token("vcs", "main*")]);

    session.set_script(SessionScript::new().workspace("api"));
    announce_metadata(&session);
    observed.wait_for_tokens("w1", &[]);
}

#[test]
fn a_token_in_an_event_payload_is_not_presented_before_a_snapshot_confirms_it() {
    let fake = fake();
    let session = start_session(&fake, SessionScript::new().workspace("api"));
    let observed = observe(&fake);
    observed.wait_for_tokens("w1", &[]);

    // Herdr's snapshot is authoritative and carries no token, so the payload alone must not
    // leave one presented.
    session.emit_event(
        "workspace_metadata_updated",
        json!({ "workspace": { "workspace_id": "w1", "tokens": { "vcs": "stale" } } }),
    );
    session.set_script(
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "ci", "passing"),
    );
    announce_metadata(&session);

    observed.wait_for_tokens("w1", &[token("ci", "passing")]);
}

#[test]
fn a_renamed_workspace_shows_its_new_title() {
    let fake = fake();
    let session = start_session(&fake, SessionScript::new().workspace("api"));
    let observed = observe(&fake);
    observed.wait_for("the first title", |p| {
        p.workspace(&WorkspaceId::from("w1")).is_some()
    });

    session.set_script(SessionScript::new().workspace("api-v2"));
    session.emit_event(
        "workspace_renamed",
        json!({ "type": "workspace_renamed", "workspace_id": "w1", "label": "api-v2" }),
    );

    observed.wait_for("the new title", |p| label_of(p, "w1") == "api-v2");
}

#[test]
fn tokens_changed_while_the_connection_was_down_converge_after_it_returns() {
    let fake = fake();
    let session = start_session(
        &fake,
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main")
            .workspace_token("api", "ci", "running"),
    );
    let observed = observe(&fake);
    observed.wait_for_tokens("w1", &[token("ci", "running"), token("vcs", "main")]);

    session.go_offline();
    observed.wait_for("the lost connection", |p| {
        p.connection != crate::ConnectionStatus::Connected
    });
    assert_eq!(
        tokens_of(&observed.latest.lock(), "w1"),
        [token("ci", "running"), token("vcs", "main")],
        "tokens stay in place while disconnected"
    );

    session.set_script(
        SessionScript::new()
            .workspace("api")
            .workspace_token("api", "vcs", "main*"),
    );
    session.go_online();

    observed.wait_for_tokens("w1", &[token("vcs", "main*")]);
}
