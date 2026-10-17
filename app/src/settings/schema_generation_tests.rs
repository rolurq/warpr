use std::collections::HashSet;

use serde_json::json;
use settings::SettingsMode;
use settings::schema::SettingSchemaEntry;

use super::{
    setting_surface_names, settings_schema_json, strip_empty_enum_entries, strip_numeric_metadata,
};

#[test]
fn strips_numeric_metadata_recursively() {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "count": {
                "type": "integer",
                "minimum": 0,
                "maximum": 255,
                "format": "uint8"
            }
        }
    });

    strip_numeric_metadata(&mut schema);

    assert_eq!(
        schema,
        json!({
            "type": "object",
            "properties": {
                "count": {
                    "type": "integer"
                }
            }
        })
    );
}

#[test]
fn strips_empty_enum_entries() {
    let mut schema = json!({
        "oneOf": [
            {
                "enum": [],
                "type": "string"
            },
            {
                "const": "kept"
            }
        ]
    });

    strip_empty_enum_entries(&mut schema);

    assert_eq!(
        schema,
        json!({
            "oneOf": [
                {
                    "const": "kept"
                }
            ]
        })
    );
}

#[test]
fn generates_a_settings_schema() {
    let schema = settings_schema_json(|_| false).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema).unwrap();

    assert_eq!(schema["title"], "Warp Settings");
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"].is_object());
}

#[test]
fn the_vertical_tabs_layout_setting_is_only_in_the_schema_while_the_layout_choice_is_on() {
    let layout_setting = |schema: &str| {
        let schema: serde_json::Value = serde_json::from_str(schema).unwrap();
        schema["properties"]["appearance"]["properties"]["vertical_tabs"]["properties"]
            .get("enabled")
            .cloned()
    };

    let without_choice = settings_schema_json(|_| false).unwrap();
    let with_choice =
        settings_schema_json(|flag| flag == warp_core::features::FeatureFlag::TabLayoutChoice)
            .unwrap();

    assert_eq!(layout_setting(&without_choice), None);
    assert!(layout_setting(&with_choice).is_some());
}

#[test]
fn surface_annotation_matches_setting_schema_entry_metadata() {
    for entry in inventory::iter::<SettingSchemaEntry> {
        let surfaces = (entry.surfaces_fn)();
        let annotation = setting_surface_names(surfaces);
        let annotation_names: HashSet<&str> = annotation
            .iter()
            .filter_map(|value| value.as_str())
            .collect();

        assert_eq!(
            annotation_names.contains("gui"),
            surfaces.includes(SettingsMode::Gui),
            "GUI surface mismatch for {}",
            entry.storage_key
        );
        assert_eq!(
            annotation_names.contains("tui"),
            surfaces.includes(SettingsMode::Tui),
            "TUI surface mismatch for {}",
            entry.storage_key
        );
        assert_eq!(
            annotation_names.len(),
            usize::from(surfaces.includes(SettingsMode::Gui))
                + usize::from(surfaces.includes(SettingsMode::Tui)),
            "unexpected surface annotation for {}",
            entry.storage_key
        );
    }
}
