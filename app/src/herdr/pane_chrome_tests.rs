use std::path::PathBuf;

use herdr_client::{CellRect, PaneId, PaneLayout, PresentedPane};

use super::*;

fn pane(id: &str, name: Option<&str>, cwd: Option<&str>) -> PresentedPane {
    PresentedPane {
        id: PaneId::from(id),
        rect: CellRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        },
        focused: false,
        cwd: cwd.map(PathBuf::from),
        name: name.map(str::to_owned),
    }
}

fn layout(panes: Vec<PresentedPane>, zoomed: bool) -> PaneLayout {
    let focused_pane = panes[0].id.clone();
    PaneLayout {
        area: panes[0].rect,
        panes,
        zoomed,
        focused_pane,
    }
}

#[test]
fn a_pane_is_titled_with_the_name_herdr_gives_it_even_when_it_has_a_directory() {
    let pane = pane("w1:p1", Some("build"), Some("/srv/api"));

    assert_eq!(pane_title(&pane), "build");
}

#[test]
fn a_pane_without_a_name_is_titled_with_the_folder_of_its_working_directory() {
    assert_eq!(
        pane_title(&pane("w1:p1", None, Some("/srv/api/src"))),
        "src"
    );
    assert_eq!(pane_title(&pane("w1:p1", None, Some("/srv/api/"))), "api");
}

#[test]
fn a_pane_in_the_root_directory_is_titled_with_the_root() {
    assert_eq!(pane_title(&pane("w1:p1", None, Some("/"))), "/");
}

#[test]
fn a_pane_with_neither_a_name_nor_a_directory_is_titled_with_its_id() {
    assert_eq!(pane_title(&pane("w1:p7", None, None)), "w1:p7");
}

#[test]
fn the_only_pane_of_a_herdr_tab_cannot_be_closed_but_each_pane_of_a_split_can() {
    let alone = layout(vec![pane("w1:p1", None, None)], false);
    let split = layout(
        vec![pane("w1:p1", None, None), pane("w1:p2", None, None)],
        false,
    );

    assert!(!can_close_panes(&alone));
    assert!(can_close_panes(&split));
}

#[test]
fn a_zoomed_pane_can_be_closed_because_the_tab_still_holds_the_panes_it_hides() {
    let zoomed = layout(
        vec![pane("w1:p1", None, None), pane("w1:p2", None, None)],
        true,
    );

    assert_eq!(zoomed.frames().len(), 1);
    assert!(can_close_panes(&zoomed));
}

#[test]
fn a_pane_is_dimmed_only_when_the_setting_is_on_it_shares_the_view_and_it_is_not_selected() {
    assert!(dims_pane(true, 2, false));
    assert!(!dims_pane(true, 2, true), "the selected pane is not dimmed");
    assert!(!dims_pane(false, 2, false), "the setting is off");
    assert!(!dims_pane(true, 1, false), "a lone pane is not inactive");
}
