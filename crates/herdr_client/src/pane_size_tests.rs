use super::*;

#[test]
fn fit_floors_to_whole_cells_and_reports_the_pixel_size_of_a_cell() {
    let fitted = PaneSize::fit(899.0, 540.0, 9.0, 18.0);

    assert_eq!(
        fitted,
        PaneSize {
            cols: 99,
            rows: 30,
            cell_width_px: 9,
            cell_height_px: 18
        }
    );
}

#[test]
fn fit_derives_columns_from_the_rounded_cell_size_it_reports() {
    let fitted = PaneSize::fit(900.0, 540.0, 8.6, 17.6);

    assert_eq!((fitted.cell_width_px, fitted.cell_height_px), (9, 18));
    assert_eq!((fitted.cols, fitted.rows), (100, 30));
}

#[test]
fn fit_never_asks_for_zero_cells() {
    let fitted = PaneSize::fit(3.0, 0.0, 9.0, 18.0);

    assert_eq!((fitted.cols, fitted.rows), (1, 1));
}
