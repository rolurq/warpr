use warp_core::features::FeatureFlag;
use warpui::App;
use warpui::keymap::EditableBinding;
use warpui::platform::menu::MenuItem;

use super::make_new_elements_menu_items;
use crate::util::bindings::CustomAction;
use crate::workspace::WorkspaceAction;

fn new_element_menu_names(app: &mut App) -> Vec<String> {
    app.update(|ctx| {
        // `NewFile` takes its menu title from the registered binding's description.
        ctx.register_editable_bindings([EditableBinding::new(
            "workspace:new_file",
            "New File",
            WorkspaceAction::NewCodeFile,
        )
        .with_custom_action(CustomAction::NewFile)]);

        make_new_elements_menu_items(ctx)
            .into_iter()
            .filter_map(|item| {
                if let MenuItem::Custom(item) = item {
                    Some(item.properties.name)
                } else {
                    None
                }
            })
            .collect()
    })
}

#[test]
fn new_agent_tab_menu_item_is_listed_when_hosted_agent_is_enabled() {
    App::test((), |mut app| async move {
        assert!(
            new_element_menu_names(&mut app)
                .iter()
                .any(|name| name == "New Agent Tab")
        );
    });
}

#[test]
fn new_agent_tab_menu_item_is_hidden_when_hosted_agent_is_disabled() {
    let _disable_hosted_agent = FeatureFlag::DisableHostedAgent.override_enabled(true);

    App::test((), |mut app| async move {
        let names = new_element_menu_names(&mut app);
        assert!(
            !names.iter().any(|name| name == "New Agent Tab"),
            "agent tabs need Warp's hosted agent, so the item must not appear even greyed out: {names:?}"
        );
        assert!(names.iter().any(|name| name == "New Terminal Tab"));
    });
}
