// ambre-toile/src/ui/status.rs
// ============================================================================
// Pure status-bar model + formatting for the selection status band
// (`toile-status-bar`). Kept in the always-compiled `ui` module (like `ui::find`)
// so the count/bounds formatting is host-unit-tested. The DOM `StatusBar`
// component (chrome/status.rs) is a thin wasm-only view over these helpers.
//
// The band's left side reports the selection tally and its combined world
// bounds (x / y / w / h). The right side hosts the shared zoom + fps chips
// (`ambre-chrome`). An empty or single-element selection shows only the count
// (no meaningful multi-element bounds), matching Trame's "lone cell shows
// nothing" rule (`trame-status-bar`).
// ============================================================================

use ambre_bus::app::toile::ElementCount;
use ambre_draw::Rect;

/// Selection aggregates rendered by the status bar, mirrored from
/// [`ToileEvent::SelectionStats`](ambre_bus::app::toile::ToileEvent::SelectionStats).
/// `count` is the number of selected elements; `bounds` is their union AABB in
/// world units, `None` when the selection is empty (nothing to frame).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SelStats {
    pub count: ElementCount,
    pub bounds: Option<Rect>,
}

impl SelStats {
    /// Build the aggregates from a Logic-computed reply: the element tally and
    /// the optional union rectangle (world space).
    pub fn from_reply(count: ElementCount, bounds: Option<Rect>) -> Self {
        Self { count, bounds }
    }
}

/// Format a world coordinate / length for the band: no decimals when whole,
/// otherwise one decimal (world units are coarse; sub-pixel noise is not useful).
pub fn fmt_world(v: ambre_core::BoardLength) -> String {
    let v = v.get();
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// The selection-count caption, for example `"1 element"` / `"3 elements"`. `None` on
/// an empty selection so the band shows only the zoom chip.
pub fn count_label(stats: &SelStats) -> Option<String> {
    match stats.count.get() {
        0 => None,
        1 => Some("1 element".to_owned()),
        n => Some(format!("{n} elements")),
    }
}

/// The combined-bounds caption (`x / y / w / h` in world units), shown only for
/// a multi-element selection where a union rectangle is meaningful. A lone or
/// empty selection returns `None`.
pub fn bounds_label(stats: &SelStats) -> Option<String> {
    if stats.count.get() < 2 {
        return None;
    }
    let board = ambre_core::BoardLength::new;
    stats.bounds.map(|b| {
        format!(
            "x {}  y {}  w {}  h {}",
            fmt_world(board(b.x0)),
            fmt_world(board(b.y0)),
            fmt_world(board(b.width())),
            fmt_world(board(b.height())),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::ElementCount;
    use ambre_draw::Rect;

    #[test]
    fn empty_selection_shows_nothing() {
        let s = SelStats::default();
        assert_eq!(count_label(&s), None);
        assert_eq!(bounds_label(&s), None);
    }

    #[test]
    fn single_element_shows_count_only() {
        // A lone element: count is shown, but no combined bounds.
        let s = SelStats::from_reply(ElementCount::new(1), Some(Rect::new(0.0, 0.0, 10.0, 20.0)));
        assert_eq!(count_label(&s).as_deref(), Some("1 element"));
        assert_eq!(bounds_label(&s), None);
    }

    #[test]
    fn multi_element_shows_count_and_bounds() {
        // Union rect (10,20)->(50,80): x=10 y=20 w=40 h=60.
        let s = SelStats::from_reply(
            ElementCount::new(3),
            Some(Rect::new(10.0, 20.0, 50.0, 80.0)),
        );
        assert_eq!(count_label(&s).as_deref(), Some("3 elements"));
        assert_eq!(bounds_label(&s).as_deref(), Some("x 10  y 20  w 40  h 60"),);
    }

    #[test]
    fn fractional_world_values_get_one_decimal() {
        let s = SelStats::from_reply(ElementCount::new(2), Some(Rect::new(1.25, 0.0, 4.0, 3.5)));
        // x=1.25 -> "1.2" (one decimal), w=2.75 -> "2.8", h=3.5 -> "3.5".
        assert_eq!(
            bounds_label(&s).as_deref(),
            Some("x 1.2  y 0  w 2.8  h 3.5"),
        );
    }
}
