use herdr_client::{ConnectionFailure, ConnectionStatus, ListedSession, SessionState};
use herdr_fake::{FakeHerdr, SessionScript};
use warpui::{App, SingletonEntity as _};

use super::*;
use crate::test_util::assert_eventually;

fn launch(app: &mut App, fake: &FakeHerdr, session: &str) {
    let cli = HerdrCli::new(fake.binary_path());
    let config_dir = fake.config_dir();
    let session = session.to_owned();
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::launch(cli, Some(config_dir), &session, ctx)
        });
    });
}

fn name(name: &str) -> SessionName {
    SessionName::new(name).unwrap()
}

fn presented_labels(app: &App) -> Vec<String> {
    HerdrSessionModel::handle(app).read(app, |model, _| {
        model
            .presented()
            .workspaces
            .iter()
            .map(|workspace| workspace.label.clone())
            .collect()
    })
}

fn is_connected(app: &App) -> bool {
    HerdrSessionModel::handle(app).read(app, |model, _| {
        model.presented().connection == ConnectionStatus::Connected
    })
}

fn listed(app: &App) -> Vec<(String, SessionState)> {
    HerdrSessionModel::handle(app).read(app, |model, _| {
        model
            .listed_sessions()
            .iter()
            .map(|ListedSession { name, state }| (name.as_str().to_owned(), *state))
            .collect()
    })
}

fn switch(app: &mut App, session: &str) {
    let session = name(session);
    let model = HerdrSessionModel::handle(&*app);
    model.update(app, |model, ctx| model.switch_session(&session, ctx));
}

#[test]
fn launching_lists_every_session_with_its_running_or_stopped_state() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", true), ("api", false), ("web", true)]);
        let _default = fake.start_session("default", SessionScript::new().workspace("main"));

        launch(&mut app, &fake, "default");

        assert_eventually!(
            600 => listed(&app).len() == 3,
            "sessions were {:?}", listed(&app)
        );
        assert_eq!(
            listed(&app),
            [
                ("default".to_owned(), SessionState::Running),
                ("api".to_owned(), SessionState::Stopped),
                ("web".to_owned(), SessionState::Running),
            ]
        );
    });
}

#[test]
fn launching_starts_the_default_session_when_none_is_running() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", false)]);
        fake.script_server_start("default", SessionScript::new().workspace("main"));

        launch(&mut app, &fake, "default");

        assert_eventually!(600 => is_connected(&app), "never connected");
        assert_eq!(presented_labels(&app), ["main"]);
    });
}

#[test]
fn switching_to_a_running_session_presents_that_session() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", true), ("work", true)]);
        let _default = fake.start_session("default", SessionScript::new().workspace("in-default"));
        let _work = fake.start_session("work", SessionScript::new().workspace("in-work"));
        launch(&mut app, &fake, "default");
        assert_eventually!(
            600 => is_connected(&app) && listed(&app).len() == 2,
            "never connected and listed"
        );

        switch(&mut app, "work");

        assert_eventually!(
            600 => presented_labels(&app) == ["in-work"],
            "presented {:?}", presented_labels(&app)
        );
        HerdrSessionModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.current_session(), Some(&name("work")));
        });
    });
}

#[test]
fn switching_to_a_stopped_session_starts_it_and_then_presents_it() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", true), ("work", false)]);
        fake.script_server_start("work", SessionScript::new().workspace("in-work"));
        let _default = fake.start_session("default", SessionScript::new().workspace("in-default"));
        launch(&mut app, &fake, "default");
        assert_eventually!(
            600 => is_connected(&app) && listed(&app).len() == 2,
            "never connected and listed"
        );
        let server_starts = |fake: &FakeHerdr| {
            fake.invocations()
                .iter()
                .filter(|invocation| invocation.args.last().is_some_and(|arg| arg == "server"))
                .count()
        };
        assert_eq!(server_starts(&fake), 0);

        switch(&mut app, "work");

        assert_eventually!(
            600 => presented_labels(&app) == ["in-work"],
            "presented {:?}", presented_labels(&app)
        );
        assert_eq!(server_starts(&fake), 1);
    });
}

#[test]
fn a_session_that_fails_to_start_is_presented_as_a_failed_connection() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", true), ("work", false)]);
        fake.script_command(&["--session", "work", "server"], "", 1);
        let _default = fake.start_session("default", SessionScript::new().workspace("in-default"));
        launch(&mut app, &fake, "default");
        assert_eventually!(
            600 => is_connected(&app) && listed(&app).len() == 2,
            "never connected and listed"
        );

        switch(&mut app, "work");

        assert_eventually!(
            600 => HerdrSessionModel::handle(&app).read(&app, |model, _| {
                matches!(
                    model.presented().connection,
                    ConnectionStatus::Failed(ConnectionFailure::ServerStartFailed { .. })
                )
            }),
            "start failure was never presented"
        );
    });
}

#[test]
fn a_session_herdr_does_not_list_cannot_be_switched_to() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_session_list(&[("default", true)]);
        let _default = fake.start_session("default", SessionScript::new().workspace("in-default"));
        launch(&mut app, &fake, "default");
        assert_eventually!(
            600 => is_connected(&app) && listed(&app).len() == 1,
            "never connected and listed"
        );

        switch(&mut app, "invented");

        HerdrSessionModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.current_session(), Some(&name("default")));
        });
        assert_eq!(presented_labels(&app), ["in-default"]);
    });
}
