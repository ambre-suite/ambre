// ambre-trame/src/render/tests.rs
// Renderer unit tests (moved verbatim out of render.rs).

use super::*;
use crate::test_support::at;
use ambre_canvas::{TextCache, Theme};
use ambre_core::Rgb;
use ambre_core::index::{Delta, Len, Pos};
use ambre_core::{Doc, Px, Surface};
use ambre_draw::FontContext;
use ambre_draw::testing::{DrawOp, RecordingSurface};

/// A shared empty column-width map for uniform-layout fixtures.
fn empty_widths() -> &'static std::collections::HashMap<Pos<Col>, ambre_core::Px<ambre_core::Doc>> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<std::collections::HashMap<Pos<Col>, ambre_core::Px<ambre_core::Doc>>> =
        OnceLock::new();
    EMPTY.get_or_init(std::collections::HashMap::new)
}

/// A shared empty row-height map for uniform-layout fixtures.
fn empty_heights() -> &'static std::collections::HashMap<Pos<Row>, ambre_core::Px<ambre_core::Doc>>
{
    use std::sync::OnceLock;
    static EMPTY: OnceLock<std::collections::HashMap<Pos<Row>, ambre_core::Px<ambre_core::Doc>>> =
        OnceLock::new();
    EMPTY.get_or_init(std::collections::HashMap::new)
}

/// 800x600 surface, uniform 80x20 cells, 50x24 headers, origin at (0,0).
fn layout() -> GridLayout<'static> {
    GridLayout {
        hidden_rows: &[],
        header_width: Px::<Surface>::new(50.0),
        header_height: Px::<Surface>::new(24.0),
        top: Px::<Surface>::ZERO,
        col_width: ambre_core::Px::<ambre_core::Doc>::new(80.0),
        row_height: ambre_core::Px::<ambre_core::Doc>::new(20.0),
        col_widths: empty_widths(),
        row_heights: empty_heights(),
        origin_col: Pos::<Col>::new(0),
        origin_row: Pos::<Row>::new(0),
        width: Px::<Surface>::new(800.0),
        height: Px::<Surface>::new(600.0),
        total_cols: Len::<Col>::new(1000),
        total_rows: Len::<Row>::new(100_000),
        zoom: ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(1.0),
    }
}

fn harness() -> (RecordingSurface, FontContext, TextCache) {
    (
        RecordingSurface::new(),
        FontContext::new(),
        TextCache::new(),
    )
}

#[test]
fn cell_rect_maps_index_to_pixels() {
    let l = layout();
    let r = l.cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)));
    assert_eq!((r.x0, r.y0), (50.0, 24.0));
    let r = l.cell_rect(CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(1)));
    assert_eq!((r.x0, r.y0), (50.0 + 80.0, 24.0 + 2.0 * 20.0));
}

#[test]
fn variable_metrics_shift_cells_and_hit_test() {
    let mut l = layout();
    // Column 1 widened to 200, row 0 heightened to 50 (absolute keys).
    let cols = std::collections::HashMap::from([(
        Pos::<Col>::new(1),
        ambre_core::Px::<ambre_core::Doc>::new(200.0),
    )]);
    let rows = std::collections::HashMap::from([(
        Pos::<Row>::new(0),
        ambre_core::Px::<ambre_core::Doc>::new(50.0),
    )]);
    l.col_widths = &cols;
    l.row_heights = &rows;

    // col_w / row_h report the per-axis values, default elsewhere.
    assert_eq!(l.col_w(Pos::<Col>::new(1)), Px::<Surface>::new(200.0));
    assert_eq!(l.col_w(Pos::<Col>::new(2)), Px::<Surface>::new(80.0));
    assert_eq!(l.row_h(Pos::<Row>::new(0)), Px::<Surface>::new(50.0));
    assert_eq!(l.row_h(Pos::<Row>::new(1)), Px::<Surface>::new(20.0));

    // cell_rect: col 2 starts after the widened col 1; row 1 starts below the
    // tall row 0.
    let r = l.cell_rect(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(2)));
    assert_eq!(r.x0, 50.0 + 80.0 + 200.0); // header + col0(80) + col1(200)
    assert_eq!(r.y0, 24.0 + 50.0); // header + row0(50)
    assert_eq!(r.x1 - r.x0, 80.0);
    assert_eq!(r.y1 - r.y0, 20.0);

    // col_at / row_at invert the mapping through the variable bands.
    let px = Px::<Surface>::new;
    assert_eq!(l.col_at(px(50.0 + 80.0 + 10.0)), Some(Pos::<Col>::new(1))); // inside the wide col 1
    assert_eq!(
        l.col_at(px(50.0 + 80.0 + 200.0 + 5.0)),
        Some(Pos::<Col>::new(2))
    );
    assert_eq!(l.row_at(px(24.0 + 25.0)), Some(Pos::<Row>::new(0))); // inside the tall row 0
    assert_eq!(l.row_at(px(24.0 + 50.0 + 5.0)), Some(Pos::<Row>::new(1)));
}

/// A coordinate that is not finite lies in no band. A NaN once walked every
/// column of the grid before it answered none.
#[test]
fn a_coordinate_that_is_not_finite_hits_no_line() {
    let l = layout();
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(l.col_at(Px::<Surface>::new(bad)), None);
        assert_eq!(l.row_at(Px::<Surface>::new(bad)), None);
    }
}

#[test]
fn cell_rect_collapses_over_hidden_rows() {
    let mut l = layout();
    // Rows 2 and 3 are filtered out (sorted slice).
    let hidden = [Pos::<Row>::new(2), Pos::<Row>::new(3)];
    l.hidden_rows = &hidden;
    assert!(
        l.is_hidden(Pos::<Row>::new(2))
            && l.is_hidden(Pos::<Row>::new(3))
            && !l.is_hidden(Pos::<Row>::new(4))
    );
    // Row 4 draws on the 3rd slot (rows 0,1 above; 2,3 collapsed).
    assert_eq!(l.visible_offset(Pos::<Row>::new(4)), Delta::<Row>::new(2));
    let r = l.cell_rect(CellPos::new(Pos::<Row>::new(4), Pos::<Col>::new(0)));
    assert_eq!(r.y0, 24.0 + 2.0 * 20.0);
    // Slot 2 maps back to absolute row 4, not row 2.
    assert_eq!(l.row_at_slot(Len::<Row>::new(2)), Some(Pos::<Row>::new(4)));
}

#[test]
fn cell_rect_accounts_for_scroll_origin() {
    let mut l = layout();
    l.origin_col = Pos::<Col>::new(3);
    l.origin_row = Pos::<Row>::new(5);
    // The origin cell sits at the content top-left.
    let r = l.cell_rect(CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(3)));
    assert_eq!((r.x0, r.y0), (50.0, 24.0));
}

#[test]
fn draw_cells_on_fills_each_and_counts_nonempty_text() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    let cells = vec![
        (at(0, 0), CellContent::plain("hi")),
        (at(1, 1), CellContent::plain("")),
    ];
    let drawn = draw_cells_on(
        &mut s,
        &l,
        palette,
        &mut fc,
        &mut tc,
        TextPass::WithText,
        cells,
        |_| true,
    );

    // Both cells get a content_bg fill...
    assert_eq!(
        s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.content_bg))
            .get(),
        2,
    );
    // ...but only the non-empty one is counted.
    assert_eq!(drawn, ambre_core::Count::<ambre_canvas::CellsDrawn>::new(1));
    assert!(s.clips_balanced());
}

#[test]
fn spill_width_stops_at_first_non_empty_neighbour() {
    use super::cells::spill_width;
    let l = layout(); // col_width 80, 1000 cols

    // All neighbours empty: capped at `need`, not at the column boundary.
    let px = Px::<Surface>::new;
    let all_empty = |_: CellPos| true;
    assert_eq!(
        spill_width(
            &l,
            CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            Delta::<Col>::new(1),
            px(100.0),
            &all_empty
        ),
        px(100.0)
    );

    // Column 3 is occupied: spilling right from col 0 covers cols 1+2 (160px)
    // then stops, even though `need` asks for more.
    let occupied_3 = |at: CellPos| at.col != Pos::<Col>::new(3);
    assert_eq!(
        spill_width(
            &l,
            CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            Delta::<Col>::new(1),
            px(1000.0),
            &occupied_3
        ),
        px(160.0)
    );

    // Spilling left from col 5 with col 3 occupied covers cols 4 only (80px).
    assert_eq!(
        spill_width(
            &l,
            CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(5)),
            Delta::<Col>::new(-1),
            px(1000.0),
            &occupied_3
        ),
        px(80.0)
    );

    // An immediately non-empty neighbour yields no spill (text clips to cell).
    assert_eq!(
        spill_width(&l, at(0, 0), Delta::<Col>::new(1), px(1000.0), &|_| false),
        Px::<Surface>::ZERO
    );

    // The grid edge bounds the leftward walk at column 0.
    assert_eq!(
        spill_width(
            &l,
            CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(1)),
            Delta::<Col>::new(-1),
            px(1000.0),
            &all_empty
        ),
        px(80.0)
    );
}

#[test]
fn draw_grid_lines_on_strokes_with_grid_color() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, _fc, _tc) = harness();

    draw_grid_lines_on(&mut s, &l, palette.grid_line, l.content_area());

    assert!(
        s.count(|op| matches!(op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line))
            .get()
            > 0
    );
}

#[test]
fn col_headers_draw_bg_separator_and_active_tint() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    draw_col_headers_on(
        &mut s,
        &l,
        palette,
        Some(Pos::<Col>::new(0)),
        None,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    // The full-width header strip.
    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        ))
        .get()
            >= 1
    );
    // The active-column tint.
    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_active
        ))
        .get()
            >= 1
    );
    // The bottom separator plus one per-column hairline: strictly more than the
    // single bottom line, proving the inter-column separators are drawn.
    assert!(
        s.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.header_border
        ))
        .get()
            >= 2
    );
    assert!(s.clips_balanced());
}

#[test]
fn col_headers_draw_hover_tint() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    // Hover column 1 while column 0 is active: both tints must be present.
    draw_col_headers_on(
        &mut s,
        &l,
        palette,
        Some(Pos::<Col>::new(0)),
        Some(Pos::<Col>::new(1)),
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_hover
        ))
        .get()
            >= 1
    );
}

#[test]
fn col_headers_skip_hover_tint_on_active_column() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    // Hover lands on the active column -> the active tint wins, no hover fill.
    draw_col_headers_on(
        &mut s,
        &l,
        palette,
        Some(Pos::<Col>::new(0)),
        Some(Pos::<Col>::new(0)),
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    assert_eq!(
        s.count(
            |op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.header_hover)
        )
        .get(),
        0,
    );
}

#[test]
fn row_headers_draw_hover_tint() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    // Hover a non-active row -> hover tint present.
    draw_row_headers_on(
        &mut s,
        &l,
        palette,
        None,
        Some(Pos::<Row>::new(1)),
        CornerHover::Plain,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_hover
        ))
        .get()
            >= 1
    );
}

#[test]
fn row_headers_draw_corner_hover_tint() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    // Hovering the top-left corner tints it with header_hover.
    draw_row_headers_on(
        &mut s,
        &l,
        palette,
        None,
        None,
        CornerHover::Hovered,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_hover
        ))
        .get()
            >= 1
    );
}

#[test]
fn row_headers_draw_per_row_separators() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    draw_row_headers_on(
        &mut s,
        &l,
        palette,
        None,
        None,
        CornerHover::Plain,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    // The right separator plus one per-row hairline: strictly more than the
    // single right line, proving the inter-row separators are drawn.
    assert!(
        s.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.header_border
        ))
        .get()
            >= 2
    );
    assert!(s.clips_balanced());
}

#[test]
fn row_headers_skip_tint_when_inactive() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    // No active row -> no header_active fill, just bg + separator.
    draw_row_headers_on(
        &mut s,
        &l,
        palette,
        None,
        None,
        CornerHover::Plain,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    assert_eq!(
        s.count(
            |op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.header_active)
        )
        .get(),
        0,
    );
    assert!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        ))
        .get()
            >= 1
    );
    assert!(s.clips_balanced());
}

#[test]
fn row_headers_draw_active_tint() {
    let l = layout();
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();

    draw_row_headers_on(
        &mut s,
        &l,
        palette,
        Some(Pos::<Row>::new(2)),
        None,
        CornerHover::Plain,
        &mut fc,
        &mut tc,
        TextPass::WithText,
    );

    // One tint, over the band of row 2 alone.
    let top = l.row_top(Pos::<Row>::new(2)).get();
    assert_eq!(
        s.count(|op| matches!(
            op, DrawOp::FillRect { color, rect, .. }
                if *color == palette.header_active && rect.y0 == top && rect.y1 == top + 20.0
        ))
        .get(),
        1,
    );
    assert!(s.clips_balanced());
}

#[test]
fn row_label_is_the_one_based_number() {
    assert_eq!(row_label(Pos::<Row>::new(0)), "1");
    assert_eq!(row_label(Pos::<Row>::new(9)), "10");
    assert_eq!(row_label(Pos::<Row>::new(u32::MAX)), "4294967296");
}

#[test]
fn col_label_matches_spreadsheet_convention() {
    assert_eq!(col_label(Pos::<Col>::new(0)), "A");
    assert_eq!(col_label(Pos::<Col>::new(25)), "Z");
    assert_eq!(col_label(Pos::<Col>::new(26)), "AA");
    assert_eq!(col_label(Pos::<Col>::new(701)), "ZZ");
    assert_eq!(col_label(Pos::<Col>::new(702)), "AAA");
}

/// Draw one cell at `(row, col)` with every neighbour empty, and return the ops.
fn draw_one(l: &GridLayout<'_>, row: u32, col: u32, content: CellContent<'_>) -> Vec<DrawOp> {
    let palette = Theme::Light.palette();
    let (mut s, mut fc, mut tc) = harness();
    let drawn = draw_cells_on(
        &mut s,
        l,
        palette,
        &mut fc,
        &mut tc,
        TextPass::WithText,
        [(at(row, col), content)],
        |_| true,
    );
    assert_eq!(
        drawn,
        ambre_core::Count::<ambre_canvas::CellsDrawn>::new(1),
        "the cell has text, so it is drawn"
    );
    assert!(s.clips_balanced());
    s.ops().to_vec()
}

/// The one clip rect pushed by a draw. Panics if the text pushed none, so a
/// missing font fails loudly instead of passing over an empty stack.
fn the_clip(ops: &[DrawOp]) -> Rect {
    let clips: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::PushClip { rect } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(clips.len(), 1, "one clip, so the text was shaped: {ops:?}");
    clips[0]
}

const LONG: &str = "a line of text much wider than its own column";

#[test]
fn a_spill_runs_in_the_direction_of_the_alignment() {
    // `TRAME-UI-096`: left text spills right, right text spills left, centred
    // text spills both ways.
    let l = layout();
    let rect = l.cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(3)));
    let with = |h_align| {
        let mut c = CellContent::plain(LONG);
        c.h_align = h_align;
        the_clip(&draw_one(&l, 0, 3, c))
    };
    let left = with(ambre_bus::HAlign::Left);
    assert_eq!(left.x0, rect.x0);
    assert!(left.x1 > rect.x1);
    let right = with(ambre_bus::HAlign::Right);
    assert!(right.x0 < rect.x0);
    assert_eq!(right.x1, rect.x1);
    let center = with(ambre_bus::HAlign::Center);
    assert!(center.x0 < rect.x0 && center.x1 > rect.x1);
    for clip in [left, right, center] {
        assert_eq!(
            (clip.y0, clip.y1),
            (rect.y0, rect.y1),
            "a spill is horizontal"
        );
    }
}

#[test]
fn a_wrapped_cell_is_cut_at_its_own_box() {
    // `TRAME-UI-097`: wrap takes the cell out of the spill and clips it.
    let l = layout();
    let mut c = CellContent::plain(LONG);
    c.wrap = ambre_core::WordBreak::Anywhere;
    let ops = draw_one(&l, 0, 3, c);
    assert_eq!(
        the_clip(&ops),
        l.cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(3)))
    );
}

#[test]
fn text_taller_than_its_row_is_cut_at_the_row() {
    // `TRAME-UI-098`: newlines push the text below the row, and the clip is the
    // cell's own rect, so nothing paints over the row below.
    let l = layout();
    let ops = draw_one(&l, 0, 0, CellContent::plain("a\nb\nc\nd"));
    assert_eq!(
        the_clip(&ops),
        l.cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
    );
}

#[test]
fn a_vertical_alignment_never_lifts_text_above_the_top() {
    // `TRAME-UI-099`: text taller than the box sits at the top for every align.
    use super::content::{TextFit, cell_v_offset};
    use ambre_bus::VAlign;
    let fit = |box_h: f64, text_h: f64| TextFit {
        box_height: Px::<Surface>::new(box_h),
        text_height: Px::<Surface>::new(text_h),
    };
    for v in [VAlign::Top, VAlign::Middle, VAlign::Bottom] {
        assert_eq!(
            cell_v_offset(v, fit(20.0, 50.0)),
            Px::<Surface>::ZERO,
            "{v:?}"
        );
        assert!(
            cell_v_offset(v, fit(20.0, 10.0)) >= Px::<Surface>::ZERO,
            "{v:?}"
        );
    }
    assert_eq!(
        cell_v_offset(VAlign::Bottom, fit(20.0, 10.0)),
        Px::<Surface>::new(10.0)
    );
}

#[test]
fn glyphs_scale_with_the_zoom() {
    // `TRAME-UI-100`: the baseline drop from the cell top is a font metric, so
    // it is four times larger at a zoom of four.
    let drop = |zoom: f64| {
        let mut l = layout();
        l.zoom = ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(zoom);
        let top = l
            .cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
            .y0;
        let mut c = CellContent::plain("Ag");
        c.v_align = ambre_bus::VAlign::Top;
        let ops = draw_one(&l, 0, 0, c);
        let y = ops
            .iter()
            .find_map(|op| match op {
                DrawOp::Glyphs { transform, .. } => Some(transform.translation().y),
                _ => None,
            })
            .expect("a glyph run, so a font was found");
        y - top
    };
    let (one, four) = (drop(1.0), drop(4.0));
    assert!(one > 0.0);
    // Line metrics round to whole pixels, so the ratio is near four, not exact.
    let ratio = four / one;
    assert!((3.5..4.5).contains(&ratio), "zoom 1: {one}, zoom 4: {four}");
}

#[test]
fn a_fill_and_a_border_stop_at_the_cell_edge() {
    // `TRAME-UI-139`: the text spills, the fill does not. It takes the cell
    // rect and is emitted before the clip opens.
    let l = layout();
    let rect = l.cell_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(3)));
    let mut c = CellContent::plain(LONG);
    c.bg = Some(Rgb::from_bits(0x00ff_0000));
    let ops = draw_one(&l, 0, 3, c);
    assert!(the_clip(&ops).x1 > rect.x1, "the text spills");
    let fill = ops
        .iter()
        .position(|op| matches!(op, DrawOp::FillRect { .. }));
    let clip = ops
        .iter()
        .position(|op| matches!(op, DrawOp::PushClip { .. }));
    assert!(fill < clip);
    for op in &ops {
        if let DrawOp::FillRect { rect: r, .. } = op {
            assert_eq!(*r, rect);
        }
    }
    let edge = Some(ambre_bus::CellBorder {
        style: ambre_bus::BorderStyle::Thick,
        color: None,
    });
    c.borders = ambre_bus::Borders {
        top: edge,
        right: edge,
        bottom: edge,
        left: edge,
    };
    let mut s = RecordingSurface::new();
    draw_cell_borders_on(&mut s, [(rect, c)], Theme::Light.palette().cell_border);
    let lines = s.count(|op| {
        let DrawOp::StrokeLine { line, .. } = *op else {
            return false;
        };
        let inside = |p: ambre_draw::Point| {
            (rect.x0..=rect.x1).contains(&p.x) && (rect.y0..=rect.y1).contains(&p.y)
        };
        assert!(inside(line.p0) && inside(line.p1), "{op:?}");
        true
    });
    assert_eq!(lines.get(), 4);
}

#[test]
fn a_cell_size_in_points_lays_out_as_prose_lays_it_out() {
    // The bus carries points. Prose lays twelve points out at sixteen pixels
    // through `FontSize::to_px`, and a cell must not lay out twelve pixels.
    let twelve = ambre_core::FontSize::from_whole_points(12);
    assert_eq!(cell_font_size(Some(twelve)), twelve.to_px());
    assert_eq!(cell_font_size(Some(twelve)), Px::<Doc>::new(16.0));
    // A cell with no size keeps the body default, which is its points laid out.
    assert_eq!(cell_font_size(None), BODY_SIZE);
    assert_eq!(BODY_FONT.to_px(), BODY_SIZE);
}
