use herdr_fake::{FakeHerdr, HerdrStatus, SessionScript};
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::test_util::assert_eventually;
use crate::workspace::view::tests::initialize_app;

fn open_picker(app: &mut App, fake: &FakeHerdr) -> ViewHandle<HerdrSessionPicker> {
    let cli = herdr_client::HerdrCli::new(fake.binary_path());
    let config_dir = fake.config_dir();
    app.update(|ctx| {
        ctx.add_singleton_model(|ctx| {
            HerdrSessionModel::launch(cli, Some(config_dir), "default", ctx)
        });
    });
    let (_, picker) = app.add_window(WindowStyle::NotStealFocus, HerdrSessionPicker::new);
    picker
}

/// What the picker would do for the entry labelled `label`, if it has one.
fn action_for(
    app: &mut App,
    picker: &ViewHandle<HerdrSessionPicker>,
    label: &str,
) -> Option<HerdrSessionPickerAction> {
    let dropdown = picker.read(app, |picker, _| picker.dropdown.clone());
    dropdown.update(app, |dropdown, ctx| {
        dropdown.set_selected_by_name(label, ctx);
        dropdown.selected_action()
    })
}

#[test]
fn the_picker_marks_each_session_running_or_stopped() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mut fake = FakeHerdr::new();
        fake.script_status(&HerdrStatus::compatible());
        fake.script_session_list(&[("default", true), ("work", false)]);
        let _default = fake.start_session("default", SessionScript::new().workspace("main"));
        let picker = open_picker(&mut app, &fake);

        assert_eventually!(
            600 => action_for(&mut app, &picker, "work (stopped)").is_some(),
            "the stopped session was never offered"
        );

        assert_eq!(
            action_for(&mut app, &picker, "default (running)"),
            Some(HerdrSessionPickerAction::Switch(
                SessionName::new("default").unwrap()
            ))
        );
        assert_eq!(
            action_for(&mut app, &picker, "work (stopped)"),
            Some(HerdrSessionPickerAction::Switch(
                SessionName::new("work").unwrap()
            ))
        );
        assert_eq!(action_for(&mut app, &picker, "work (running)"), None);
    });
}

#[test]
fn choosing_a_session_in_the_picker_presents_it() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mut fake = FakeHerdr::new();
        fake.script_status(&HerdrStatus::compatible());
        fake.script_session_list(&[("default", true), ("work", true)]);
        let _default = fake.start_session("default", SessionScript::new().workspace("in-default"));
        let _work = fake.start_session("work", SessionScript::new().workspace("in-work"));
        let picker = open_picker(&mut app, &fake);
        assert_eventually!(
            600 => action_for(&mut app, &picker, "work (running)").is_some(),
            "the listing never arrived"
        );

        let choice = HerdrSessionPickerAction::Switch(SessionName::new("work").unwrap());
        picker.update(&mut app, |picker, ctx| picker.handle_action(&choice, ctx));

        assert_eventually!(
            600 => HerdrSessionModel::handle(&app).read(&app, |model, _| {
                model.presented().workspaces.first().map(|w| w.label.as_str()) == Some("in-work")
            }),
            "the chosen session was never presented"
        );
    });
}
