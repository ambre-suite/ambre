// ambre-trame/src/render/tests.rs
// Renderer unit tests (moved verbatim out of render.rs).

    use super::*;
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use ambre_draw::FontContext;

    /// 800x600 surface, uniform 80x20 cells, 50x24 headers, origin at (0,0).
    fn layout() -> GridLayout<'static> {
        GridLayout {
            hidden_rows: &[],
            header_width: 50.0,
            header_height: 24.0,
            top: 0.0,
            col_width: 80.0,
            row_height: 20.0,
            origin_col: 0,
            origin_row: 0,
            width: 800.0,
            height: 600.0,
            total_cols: 1000,
            total_rows: 100_000,
        }
    }

    fn harness() -> (RecordingSurface, FontContext, TextCache) {
        (RecordingSurface::new(), FontContext::new(), TextCache::new(1.0))
    }

    #[test]
    fn cell_rect_maps_index_to_pixels() {
        let l = layout();
        let r = l.cell_rect(0, 0);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
        let r = l.cell_rect(2, 1);
        assert_eq!((r.x0, r.y0), (50.0 + 80.0, 24.0 + 2.0 * 20.0));
    }

    #[test]
    fn cell_rect_collapses_over_hidden_rows() {
        let mut l = layout();
        // Rows 2 and 3 are filtered out (sorted slice).
        l.hidden_rows = &[2, 3];
        assert!(l.is_hidden(2) && l.is_hidden(3) && !l.is_hidden(4));
        // Row 4 draws on the 3rd slot (rows 0,1 above; 2,3 collapsed).
        assert_eq!(l.visible_offset(4), 2);
        let r = l.cell_rect(4, 0);
        assert_eq!(r.y0, 24.0 + 2.0 * 20.0);
        // Slot 2 maps back to absolute row 4, not row 2.
        assert_eq!(l.row_at_slot(2), Some(4));
    }

    #[test]
    fn cell_rect_accounts_for_scroll_origin() {
        let mut l = layout();
        l.origin_col = 3;
        l.origin_row = 5;
        // The origin cell sits at the content top-left.
        let r = l.cell_rect(5, 3);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
    }

    #[test]
    fn draw_cells_on_fills_each_and_counts_nonempty_text() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        let cells = vec![
            (0, 0, CellContent::plain("hi", false, false)),
            (1, 1, CellContent::plain("", false, false)),
        ];
        let drawn = draw_cells_on(&mut s, &l, palette, &mut fc, &mut tc, true, cells, |_, _| true);

        // Both cells get a content_bg fill...
        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.content_bg)),
            2,
        );
        // ...but only the non-empty one is tallied.
        assert_eq!(drawn, 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn spill_width_stops_at_first_non_empty_neighbour() {
        use super::cells::spill_width;
        let l = layout(); // col_width 80, 1000 cols

        // All neighbours empty: capped at `need`, not at the column boundary.
        let all_empty = |_, _| true;
        assert_eq!(spill_width(&l, 0, 0, 1, 100.0, &all_empty), 100.0);

        // Column 3 is occupied: spilling right from col 0 covers cols 1+2 (160px)
        // then stops, even though `need` asks for more.
        let occupied_3 = |_r, c| c != 3;
        assert_eq!(spill_width(&l, 0, 0, 1, 1000.0, &occupied_3), 160.0);

        // Spilling left from col 5 with col 3 occupied covers cols 4 only (80px).
        assert_eq!(spill_width(&l, 0, 5, -1, 1000.0, &occupied_3), 80.0);

        // An immediately non-empty neighbour yields no spill (text clips to cell).
        assert_eq!(spill_width(&l, 0, 0, 1, 1000.0, &|_, _| false), 0.0);

        // The grid edge bounds the leftward walk at column 0.
        assert_eq!(spill_width(&l, 0, 1, -1, 1000.0, &all_empty), 80.0);
    }

    #[test]
    fn draw_grid_lines_on_strokes_with_grid_color() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, _fc, _tc) = harness();

        draw_grid_lines_on(&mut s, &l, palette.grid_line, l.content_area());

        assert!(
            s.count(|op| matches!(op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line)) > 0
        );
    }

    #[test]
    fn col_headers_draw_bg_separator_and_active_tint() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        draw_col_headers_on(&mut s, &l, palette, Some(0), &mut fc, &mut tc, true);

        // The full-width header strip.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        // The active-column tint.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_active
        )) >= 1);
        // The bottom separator.
        assert!(s.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.header_border
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn row_headers_skip_tint_when_inactive() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        // No active row -> no header_active fill, just bg + separator.
        draw_row_headers_on(&mut s, &l, palette, None, &mut fc, &mut tc, true);

        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.header_active)),
            0,
        );
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn col_label_matches_spreadsheet_convention() {
        assert_eq!(col_label(0), "A");
        assert_eq!(col_label(25), "Z");
        assert_eq!(col_label(26), "AA");
        assert_eq!(col_label(701), "ZZ");
        assert_eq!(col_label(702), "AAA");
    }
