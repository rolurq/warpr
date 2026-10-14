use herdr_client::{SessionName, SessionState};
use warpui::elements::{ChildView, MainAxisSize};
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::HerdrSessionModel;
use crate::view_components::dropdown::DropdownStyle;
use crate::view_components::{Dropdown, DropdownEvent, DropdownItem};

/// Lists every session Herdr knows, marked running or stopped, and switches the presented session
/// to the one picked. Offers nothing else: sessions are neither created, stopped nor deleted here.
pub struct HerdrSessionPicker {
    dropdown: ViewHandle<Dropdown<HerdrSessionPickerAction>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HerdrSessionPickerAction {
    Switch(SessionName),
}

impl HerdrSessionPicker {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_style(DropdownStyle::Naked, ctx);
            dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
            dropdown
        });
        // Sessions can be started or stopped outside warpr, so the listing is read again whenever
        // the picker is opened.
        ctx.subscribe_to_view(&dropdown, |_, _, event, ctx| match event {
            DropdownEvent::ToggleExpanded => {
                HerdrSessionModel::handle(ctx)
                    .update(ctx, |model, ctx| model.refresh_sessions(ctx));
            }
            DropdownEvent::Close => {}
        });
        ctx.subscribe_to_model(&HerdrSessionModel::handle(ctx), |me, _, _, ctx| {
            me.sync_items(ctx);
        });

        let mut picker = Self { dropdown };
        picker.sync_items(ctx);
        picker
    }

    /// The picker's wording for a session: its name and whether its server is up.
    fn label(name: &SessionName, state: Option<SessionState>) -> String {
        match state {
            Some(SessionState::Running) => format!("{} (running)", name.as_str()),
            Some(SessionState::Stopped) => format!("{} (stopped)", name.as_str()),
            None => name.as_str().to_owned(),
        }
    }

    fn sync_items(&mut self, ctx: &mut ViewContext<Self>) {
        let model = HerdrSessionModel::as_ref(ctx);
        let current = model.current_session().cloned();
        let mut items: Vec<_> = model
            .listed_sessions()
            .iter()
            .map(|session| {
                DropdownItem::new(
                    Self::label(&session.name, Some(session.state)),
                    HerdrSessionPickerAction::Switch(session.name.clone()),
                )
            })
            .collect();
        // Until Herdr's listing arrives the session being connected to still has to be named.
        if let Some(current) = &current
            && !model
                .listed_sessions()
                .iter()
                .any(|session| &session.name == current)
        {
            items.push(DropdownItem::new(
                Self::label(current, None),
                HerdrSessionPickerAction::Switch(current.clone()),
            ));
        }
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            if let Some(current) = current {
                dropdown.set_selected_by_action(HerdrSessionPickerAction::Switch(current), ctx);
            } else {
                dropdown.set_selected_to_none(ctx);
            }
        });
    }
}

impl Entity for HerdrSessionPicker {
    type Event = ();
}

impl View for HerdrSessionPicker {
    fn ui_name() -> &'static str {
        "HerdrSessionPicker"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.dropdown).finish()
    }
}

impl TypedActionView for HerdrSessionPicker {
    type Action = HerdrSessionPickerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HerdrSessionPickerAction::Switch(name) => {
                HerdrSessionModel::handle(ctx)
                    .update(ctx, |model, ctx| model.switch_session(name, ctx));
            }
        }
    }
}

#[cfg(all(test, unix))]
#[path = "session_picker_tests.rs"]
mod tests;
