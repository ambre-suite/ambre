// The grid on the shared kinetic scroll (`TRAME-UI-175`).

use super::*;

/// `TRAME-UI-175`: a drag held past an edge of the cells scrolls towards it.
/// It keeps reaching the nearest edge cell, and away from the edges it
/// scrolls nothing.
#[test]
fn a_drag_held_at_an_edge_scrolls_and_keeps_its_edge_cell() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.scroll_by(10, 3);
    assert_eq!(state.edge_scroll_speed(400.0, 300.0), (0.0, 0.0));
    let (_, down) = state.edge_scroll_speed(400.0, 900.0);
    assert!(down > 0.0, "{down}");
    // Over the header strips the grid scrolls back towards the origin.
    let (left, up) = state.edge_scroll_speed(1.0, 1.0);
    assert!(left < 0.0 && up < 0.0, "{left} {up}");
    // Past the edges the drag still points at a cell: the nearest one.
    assert_eq!(state.cell_at(1.0, 1.0), None);
    assert_eq!(state.drag_cell_at(1.0, 1.0), Some((10, 3)));
    let (row, _) = state.drag_cell_at(400.0, 9_000.0).expect("an edge cell");
    assert!(row > 10);
}
