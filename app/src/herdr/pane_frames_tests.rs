use herdr_client::CellRect;
use warpui::geometry::vector::vec2f;

use super::placement;

fn rect(x: u16, y: u16, width: u16, height: u16) -> CellRect {
    CellRect {
        x,
        y,
        width,
        height,
    }
}

const AREA: CellRect = CellRect {
    x: 0,
    y: 0,
    width: 120,
    height: 40,
};

#[test]
fn a_pane_covering_the_area_fills_the_given_size() {
    let frame = placement(AREA, AREA, vec2f(1000., 500.));

    assert_eq!(frame.origin(), vec2f(0., 0.));
    assert_eq!(frame.size(), vec2f(1000., 500.));
}

#[test]
fn frames_keep_the_proportions_of_herdrs_cells() {
    let left = placement(AREA, rect(0, 0, 36, 40), vec2f(1200., 400.));
    let right = placement(AREA, rect(36, 0, 84, 40), vec2f(1200., 400.));

    assert_eq!((left.origin().x(), left.size().x()), (0., 360.));
    assert_eq!((right.origin().x(), right.size().x()), (360., 840.));
}

#[test]
fn neighbouring_frames_share_an_edge_even_when_cells_do_not_divide_the_size() {
    let size = vec2f(1001., 333.);
    let left = placement(AREA, rect(0, 0, 36, 20), size);
    let right = placement(AREA, rect(36, 0, 84, 20), size);
    let below = placement(AREA, rect(0, 20, 36, 20), size);

    assert_eq!(left.upper_right().x(), right.origin().x());
    assert_eq!(left.lower_left().y(), below.origin().y());
    assert_eq!(right.upper_right().x(), size.x());
    assert_eq!(below.lower_left().y(), size.y());
}

#[test]
fn an_area_that_does_not_start_at_the_origin_is_measured_from_its_own_corner() {
    let area = rect(10, 5, 100, 20);

    let frame = placement(area, rect(60, 15, 50, 10), vec2f(1000., 200.));

    assert_eq!(frame.origin(), vec2f(500., 100.));
    assert_eq!(frame.size(), vec2f(500., 100.));
}

#[test]
fn an_empty_area_places_nothing() {
    let frame = placement(rect(0, 0, 0, 0), rect(0, 0, 10, 10), vec2f(1000., 500.));

    assert_eq!(frame.size(), vec2f(0., 0.));
}
