// The grid on the shared kinetic scroll (`TRAME-UI-175`).

use super::*;
use ambre_core::{Px, Surface};

/// The canvas point at `(x, y)`.
fn view_point(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::View>::new(x as f32),
        ambre_core::Px::<ambre_core::View>::new(y as f32),
    )
}

/// `TRAME-UI-175`: a drag held past an edge of the cells scrolls towards it.
/// It keeps reaching the nearest edge cell, and away from the edges it
/// scrolls nothing.
#[test]
fn a_drag_held_at_an_edge_scrolls_and_keeps_its_edge_cell() {
    let mut state = TrameUiState::new(Px::<Surface>::new(800.0), Px::<Surface>::new(600.0));
    state.scroll_by(RowDelta::new(10), ColDelta::new(3));
    let (sx, sy) = state.edge_scroll_speed(pointer(400.0, 300.0));
    assert_eq!((sx.get().get(), sy.get().get()), (0.0, 0.0));
    let down = state.edge_scroll_speed(pointer(400.0, 900.0)).1.get().get();
    assert!(down > 0.0, "{down}");
    // Over the header strips the grid scrolls back towards the origin.
    let (left, up) = state.edge_scroll_speed(pointer(1.0, 1.0));
    let (left, up) = (left.get().get(), up.get().get());
    assert!(left < 0.0 && up < 0.0, "{left} {up}");
    // Past the edges the drag still points at a cell: the nearest one.
    assert_eq!(state.cell_at(pointer(1.0, 1.0)), None);
    assert_eq!(state.drag_cell_at(pointer(1.0, 1.0)), Some(at(10, 3)));
    let CellPos { row, col: _ } = state
        .drag_cell_at(pointer(400.0, 9_000.0))
        .expect("an edge cell");
    assert!(row > Row::new(10));
}

/// `UI-THREAD-001`: a canvas point reaches a cell through the shared
/// hit-test, under a scroll and a zoom. The header strips hit no cell.
#[test]
fn a_canvas_point_reaches_its_cell_through_the_shared_hit_test() {
    use ambre_draw::HitTest;
    let mut state = TrameUiState::new(Px::<Surface>::new(800.0), Px::<Surface>::new(600.0));
    state.scroll_by(RowDelta::new(10), ColDelta::new(3));
    assert!(state.set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(2.0)));
    for (row, col) in [(10, 3), (12, 4)] {
        let cell = state
            .cell_pixel_rect(CellPos::new(Row::new(row), Col::new(col)))
            .expect("visible");
        let centre = cell.center();
        assert_eq!(
            state.hit_test(view_point(centre.x, centre.y)),
            Some(at(row, col))
        );
    }
    assert_eq!(state.hit_test(view_point(1.0, 1.0)), None);
}
