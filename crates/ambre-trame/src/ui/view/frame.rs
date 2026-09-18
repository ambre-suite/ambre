// ambre-trame/src/ui/view/frame.rs
// TrameView: document binding, event dispatch, per-frame update and paint.

use super::*;

impl TrameView {
    /// Swap the active document in place (SPA route change). Drop every
    /// document-scoped field back to its `new()` value, preserving the shared,
    /// route-independent plumbing (channels, text pipe, anchor, theme) and the
    /// current canvas size. Then force a full repaint and a fresh initial
    /// viewport fetch for the new document. No-op if already current.
    pub fn set_document(&mut self, document_id: DocumentId) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.state = TrameUiState::new(self.state.canvas_width, self.state.canvas_height);
        self.editor = None;
        self.edit_dirty = false;
        self.initial_viewport_sent = false;
        self.pending_viewport = None;
        self.needs_full = true;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.prev_top_row = 0;
        self.prev_left_col = 0;
        self.cut_pending = false;
        // Retract any floating-toolbar anchor from the previous document.
        self.last_anchor = None;
        self.anchor.publish(None);
    }

    /// Canvas-side reaction to a single [`TrameEvent`] (`trame-engine`, the
    /// update from the Logic thread). Exhaustive on purpose: a new variant must
    /// fail the build here rather than be dropped silently. Variants that only
    /// affect the DOM chrome (toasts, formatting toolbar, undo/redo enablement)
    /// are explicit no-ops. They are handled in the per-frame relay
    /// (`ambre-app`), not here.
    pub(crate) fn dispatch_trame(&mut self, te: TrameEvent) {
        match te {
            // Cell content batch: merge into the cache and flag data dirty. If a
            // scroll happens the same frame the strip path reads the refreshed
            // cache; otherwise plan_repaint promotes data_dirty to a Full repaint.
            TrameEvent::ViewportData { .. } => {
                self.state.on_viewport_data(te);
                self.data_dirty = true;
                // A batch may carry a newly-created merge whose span now covers
                // the active cell (for example the drag-end corner after a merge). Re-home
                // the cursor on the master so typing targets the top-left, not a
                // covered cell (which would reject the write as a conflict). Skip
                // while dragging/editing so an in-progress gesture is not disturbed.
                if !self.selecting_drag
                    && self.editor.is_none()
                    && self.state.snap_active_to_master()
                {
                    self.send_command_lossy(self.set_active_cmd());
                    self.needs_full = true;
                }
            }
            // The active sheet may have changed (switch, or delete fallback
            // decided by the Logic thread); adopt it so edits target the right
            // sheet. The DOM `SheetTabs` strip mirrors the same event for the UI;
            // a separate DataReset re-fetches the viewport.
            TrameEvent::SheetList { active, .. } => {
                self.state.active_sheet = active;
                self.needs_full = true;
            }
            // A rejected lifecycle op (for example deleting the last sheet). The chrome
            // toasts it; here we log it and announce it for a11y.
            TrameEvent::Error { message } => {
                tracing::warn!("[UI] Trame rejected: {message}");
                self.text_input.announce(&message);
            }
            // Both schema notices are the chrome's to toast. Announcing them here
            // too puts them on the a11y channel, which is the only surface a
            // screen-reader user has for either (`SCHEMA-003`, `SCHEMA-008`).
            TrameEvent::DocumentRefused { reason } => {
                tracing::warn!("[UI] Trame refused a document: {reason}");
                self.text_input.announce(&reason);
            }
            TrameEvent::UndecodableRecords => {
                tracing::warn!("[UI] Trame met a register it cannot decode");
                self.text_input
                    .announce("Some formatting in this workbook could not be read.");
            }
            // Shared filter (`trame-shared-filter`): track the filtered column for the funnel
            // badge. The accompanying DataReset re-fetches the collapsed rows.
            TrameEvent::FilterApplied { col, .. } => {
                self.state.filtered_col = Some(col);
                self.needs_full = true;
            }
            TrameEvent::FilterCleared => {
                self.state.filtered_col = None;
                self.needs_full = true;
            }
            // Find & Replace (`trame-find-replace`): highlight the match set and jump the
            // active cell onto the focused match. That reuses the navigation path, so an
            // off-screen match scrolls into view. An empty set clears the highlight
            // without moving the cursor.
            TrameEvent::FindResult { matches, active } => {
                self.state.find_matches = matches.clone();
                self.needs_full = true;
                if let Some(i) = active
                    && let Some(&(row, col)) = matches.get(i)
                {
                    self.nav_goto(row, col, false);
                }
            }
            // Wholesale content change: a local snapshot restore (DataReset) or a
            // conditional-formatting rule change (CondFormatsUpdated,
            // `trame-cell-styles`). Both
            // invalidate the cache -- re-fetch the visible region, then full-repaint
            // with the result.
            TrameEvent::DataReset | TrameEvent::CondFormatsUpdated { .. } => {
                self.needs_full = true;
                let cmd = self.state.scroll_viewport_cmd();
                self.send_command_lossy(cmd);
            }
            // Logic asks the canvas to adopt a selection it cannot derive locally.
            // An unmerge is the case: the freed cells stay selected, though the
            // span-defining merge is gone. Active cell collapses to the top-left
            // corner; the anchor sits at the bottom-right so the rectangle renders
            // whole. Mirror the active cell back to Logic and the input pipe.
            TrameEvent::SelectRange { r1, c1, r2, c2 } => {
                self.state.select_rect(r1, c1, r2, c2);
                self.needs_full = true;
                self.viewport_changed = true;
                if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
                    tracing::error!("Trame: the outbox is full, SelectRange dropped");
                }
                self.park_input_on(r1, c1);
                self.announce_active();
            }
            // Interop export (`trame-import-export`): the Logic thread
            // serialized the file bytes; the UI Thread owns the DOM, so the
            // download is triggered here. No canvas state changes.
            TrameEvent::ExportComplete { filename, data } => {
                trigger_download(&filename, &data);
            }
            // Authoritative copy reply (`trame-clipboard`): the Logic thread
            // serialized the whole tracked selection from the document (no viewport
            // limit). Build the transport blob: TSV for cross-app `text/plain`,
            // postcard of the typed payload for the private Ambre type. Stash it in
            // memory for a full-fidelity same-tab paste. Mark it pending so the
            // host mirrors it to the OS clipboard next frame.
            TrameEvent::Clipboard(payload) => {
                let blob = ambre_bus::ClipboardBlob {
                    text: payload.to_tsv(),
                    ambre: postcard::to_allocvec(&payload).ok(),
                };
                self.last_clip = Some(blob.clone());
                self.pending_clipboard = Some(blob);
            }
            // Interop import finished (`trame-import-export`): the accompanying
            // DataReset already re-fetches the populated region, so this is purely
            // informational here (the chrome relay may toast it).
            TrameEvent::ImportComplete { .. } => {}
            // Chrome-only variants: surfaced as toasts / toolbar state / undo-redo
            // enablement by the per-frame relay (`ambre-app`). No canvas state
            // changes -- the cache is refreshed by the DataReset that accompanies
            // any of these that mutate content.
            TrameEvent::StyleApplied { .. }
            | TrameEvent::MergeRejected { .. }
            | TrameEvent::MergeConflictDetected { .. }
            | TrameEvent::UndoState { .. }
            | TrameEvent::SortComplete
            | TrameEvent::ColumnValues { .. }
            | TrameEvent::SelectionStats { .. }
            | TrameEvent::ReplaceDone { .. }
            | TrameEvent::PasteRejected { .. }
            | TrameEvent::FilterViewSaved { .. }
            | TrameEvent::ValidationError { .. }
            | TrameEvent::ValidationWarning { .. }
            | TrameEvent::ProtectionDenied { .. }
            | TrameEvent::ProtectionViolationWarning { .. } => {}
        }
    }

    pub fn update(&mut self) {
        // Offer the held commands to the channel first: the Logic thread drained
        // while the frame was out. The grid stays busy for as long as one
        // command waits (`BUS-011`, `BUS-016`).
        self.flush_outbox();
        // Drain every queued UiEvent without blocking. A `Disconnected` error
        // means the Logic thread is gone (panic=abort or shutdown). There is
        // no recovery, so flip to the fatal state instead of silently looping.
        loop {
            match self.from_logic.try_recv() {
                // Single entry point for every Trame logic message: an exhaustive
                // dispatcher (`dispatch_trame`) decides the canvas-side reaction.
                // The chrome side (toasts / toolbar / command store) consumes the
                // same event in the per-frame relay; the two never overlap.
                Ok(UiEvent::Viewport {
                    event: AppEvent::Trame(te),
                    ..
                }) => self.dispatch_trame(te),
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                // Other UiEvent variants (session, CRUD, channels...) are not yet
                // consumed by the Trame view; ignore them for now.
                Ok(_) => {}
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }

        // Apply the latest grid zoom selected in the status-bar menu
        // (`trame-status-bar`). `set_zoom` is a no-op when unchanged; on a change it
        // queues the viewport re-request flushed just below.
        #[cfg(target_arch = "wasm32")]
        {
            let z = self.zoom_sig.get_untracked();
            self.set_zoom(z);
        }

        // Advance any touchpad momentum coast before the viewport flush, so the
        // ScrollViewport it produces ships this same frame.
        self.step_momentum();

        // Flush the coalesced viewport request: at most one ScrollViewport per
        // frame, regardless of how many arrow keys fired since the last frame.
        if let Some(cmd) = self.pending_viewport.take() {
            self.send_command_lossy(cmd);
        }

        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            let cmd = self.state.scroll_viewport_cmd();
            self.send_command_lossy(cmd);
        }

        // Pick up a theme switch (chrome toggled it): repaint everything with
        // the new palette.
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.needs_full = true;
        }

        // Keep the floating selection toolbar glued to the active cell as the
        // viewport pans (published on change only).
        self.publish_anchor();

        // Caret blink: while editing, flip the phase every half-period and dirty
        // the overlay so it redraws. Costs one overlay redraw per ~530 ms; idle
        // (non-editing) frames never touch it.
        if self.editor.is_some() {
            let now = self.perf.now();
            if now - self.last_blink_ms >= BLINK_MS {
                self.caret_on = !self.caret_on;
                self.last_blink_ms = now;
                self.edit_dirty = true;
            }
        }

        // While a clipboard marquee is active, dirty the overlay each frame so
        // the dashes crawl (`trame-clipboard`). Cleared on Escape / next copy.
        if self.state.clipboard_anchor.is_some() {
            self.edit_dirty = true;
        }

        // Sync the HTML formula bar with the grid (`trame-cell-editing`).
        #[cfg(target_arch = "wasm32")]
        {
            self.formula_ref.set(self.active_cell_ref());

            // While the formula input is focused it OWNS the text. The user is
            // typing there, so the view must not overwrite `formula_text` (that
            // would fight the caret). A divergence from the active cell's value
            // means an edit is in progress -- open / drive an edit session from
            // the input's content. While the input is NOT focused, the formula
            // bar is read-only: mirror the in-cell editor buffer (live), or the
            // active cell's stored value when idle.
            if formula_input_focused() {
                let text = self.formula_text.get_untracked();
                if let Some(ed) = self.editor.as_mut() {
                    if ed.text() != text {
                        ed.set_text(&text);
                        self.edit_dirty = true;
                    }
                } else {
                    let seed = self.cell_seed(self.state.active_row(), self.state.active_col());
                    if text != seed {
                        // First typed character in the formula bar: open an edit
                        // session on the active cell, seeded with what was typed.
                        self.begin_edit_external();
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_text(&text);
                            self.edit_dirty = true;
                        }
                    }
                }
            } else if let Some(ed) = self.editor.as_ref() {
                let buffer = ed.text();
                if self.formula_text.get_untracked() != buffer {
                    self.formula_text.set(buffer);
                }
            } else {
                let seed = self.cell_seed(self.state.active_row(), self.state.active_col());
                if self.formula_text.get_untracked() != seed {
                    self.formula_text.set(seed);
                }
            }

            // Handle formula bar commit / cancel actions from the DOM input.
            if let Some(action) = self.formula_action.get_untracked() {
                match action {
                    FormulaAction::Commit(dir) => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_text(&self.formula_text.get_untracked());
                        }
                        self.commit_edit();
                        match dir {
                            CommitDirection::Down => {
                                self.nav_move(1, 0, false);
                            }
                            CommitDirection::Right => {
                                self.nav_move(0, 1, false);
                            }
                            CommitDirection::Stay => {}
                        }
                        // Return focus to the grid and reset the formula text to
                        // the cell we landed on. Without this reset `formula_text`
                        // still holds the just-committed value. If the input keeps
                        // DOM focus for even one frame, the focused branch above
                        // would see `text != seed` on the NEW cell. It would then
                        // re-open an edit there, writing the value into the cell
                        // below too.
                        self.text_input.focus();
                        self.formula_text
                            .set(self.cell_seed(self.state.active_row(), self.state.active_col()));
                    }
                    // Ctrl/Cmd+Enter: fill the value into every selected cell and
                    // keep the selection put (no cursor advance).
                    FormulaAction::CommitFill => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_text(&self.formula_text.get_untracked());
                        }
                        self.commit_edit_filled();
                        self.text_input.focus();
                        self.formula_text
                            .set(self.cell_seed(self.state.active_row(), self.state.active_col()));
                    }
                    FormulaAction::Cancel => {
                        self.cancel_edit();
                        self.text_input.focus();
                    }
                }
                self.formula_action.set(None);
            }
        }
    }

    pub fn plan_repaint(&mut self, _now_ms: f64, hud_tick: bool) -> RepaintPlan {
        // Formatting lives per-cell in the persistent cell layer, so a cursor
        // move needs no special repaint -- the cells keep their runs as they
        // shift. Only a CellFormat event (handled in `update`) dirties content.
        let any_dirty = self.needs_full
            || self.viewport_changed
            || self.data_dirty
            || self.edit_dirty
            || hud_tick;

        let (content, content_strip): (ContentRepaint, Option<Rect>) = if self.needs_full {
            (ContentRepaint::Full, None)
        } else if self.viewport_changed {
            let dr = self.state.top_row as i64 - self.prev_top_row as i64;
            let dc = self.state.left_col as i64 - self.prev_left_col as i64;
            if dr == 0 && dc == 0 {
                // Cursor moved within the viewport without scrolling. If data
                // also arrived the visible content need a refresh; otherwise the
                // content offscreen is still valid (the border is in the overlay).
                if self.data_dirty {
                    (ContentRepaint::Full, None)
                } else {
                    (ContentRepaint::Reuse, None)
                }
            } else if dr != 0 && dc != 0 {
                // Diagonal scroll would need an L-shaped strip; go full.
                (ContentRepaint::Full, None)
            } else if !self.state.col_widths.is_empty() || !self.state.row_heights.is_empty() {
                // The blit-scroll shift below assumes uniform metrics: moving `dr`
                // rows shifts the content by `dr * ROW_HEIGHT`. With any resized
                // axis (`trame-resize`) that pixel distance is wrong, so repaint fully
                // rather than tear. Resized axes are rare; the cost is bounded.
                (ContentRepaint::Full, None)
            } else {
                // One whole-cell step is the base metric scaled by the grid zoom.
                let dy_px = (-(dr as f64) * ROW_HEIGHT * self.state.zoom).round() as i32;
                let dx_px = (-(dc as f64) * COL_WIDTH * self.state.zoom).round() as i32;
                let w = self.state.canvas_width;
                // Bottom edge of the cell area (now the full canvas: the sheet-tab
                // strip moved to DOM chrome, so it no longer reserves a band here).
                let h = self.state.grid_bottom();
                // Top of the scrollable cell area: below the formula bar band
                // (`trame-cell-editing`) and the column-header strip.
                let hh = self.state.header_h();
                // If the scroll exceeds the viewport, nothing useful to shift.
                let abs_dx = dx_px.unsigned_abs();
                let abs_dy = dy_px.unsigned_abs();
                if abs_dx as f64 >= w || abs_dy as f64 >= h {
                    (ContentRepaint::Full, None)
                } else {
                    // Strip = the area of the destination offscreen exposed by
                    // the shift, restricted to the cell area.
                    let hw = self.state.header_w();
                    let strip = if dy_px > 0 {
                        Rect::new(hw, hh, w, hh + dy_px as f64)
                    } else if dy_px < 0 {
                        Rect::new(hw, h + dy_px as f64, w, h)
                    } else if dx_px > 0 {
                        Rect::new(hw, hh, hw + dx_px as f64, h)
                    } else {
                        Rect::new(w + dx_px as f64, hh, w, h)
                    };
                    (ContentRepaint::Scroll { dx_px, dy_px }, Some(strip))
                }
            }
        } else if self.data_dirty {
            // Data arrived without scroll/cursor movement: repaint so the freshly
            // cached text shows up.
            (ContentRepaint::Full, None)
        } else {
            (ContentRepaint::Reuse, None)
        };

        let overlay = if any_dirty {
            OverlayRepaint::Redraw
        } else {
            OverlayRepaint::Reuse
        };

        // Consume the per-frame flags.
        self.needs_full = false;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.edit_dirty = false;
        self.prev_top_row = self.state.top_row;
        self.prev_left_col = self.state.left_col;

        // When the content layer is reused the renderer won't run; zero its HUD
        // sub-phase timings so they read 0 instead of stale values.
        if matches!(content, ContentRepaint::Reuse) {
            self.draw_stats.bg_ms = 0.0;
            self.draw_stats.cells_ms = 0.0;
            self.draw_stats.cells_drawn = 0;
        }

        RepaintPlan {
            content,
            content_strip,
            overlay,
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
        // Resolve a pending auto-fit now that the font context is in hand
        // (`trame-resize`): measure the column/row's cached cells and emit a resize.
        if let Some(hit) = self.pending_autofit.take() {
            self.resolve_autofit(hit, font_cx, text_cache);
        }
        renderer::draw_cells(
            surface,
            &self.state,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.text_enabled,
            strip,
            &PerfClock(&self.perf),
            &mut self.draw_stats,
        );
    }

    pub fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        renderer::draw_overlay(
            surface,
            &self.state,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.text_enabled,
            &PerfClock(&self.perf),
            &mut self.draw_stats,
        );

        // Publish the per-frame perf snapshot to the DOM status bar (the HUD no
        // longer paints on the wgpu overlay). Merge the driver timings (`stats`)
        // with the view's sub-phase breakdown and the glyph-cache misses.
        self.perf_pub.publish(PerfSnapshot {
            fps: stats.fps,
            frame_ms: stats.frame_ms,
            drain_ms: stats.drain_ms,
            draw_ms: stats.draw_ms,
            gpu_ms: stats.gpu_ms,
            bg_ms: self.draw_stats.bg_ms,
            cells_ms: self.draw_stats.cells_ms,
            hdrs_ms: self.draw_stats.hdrs_ms,
            cells_drawn: self.draw_stats.cells_drawn,
            input_lat_ms: self.draw_stats.input_latency_ms,
            glyph_misses: text_cache.frame_misses as u32,
        });

        // The in-cell edit field sits above everything (after the active border).
        // draw_edit flushes the editor's queued intents (a FontContext is live
        // here), so the buffer is final afterwards. That is why a deferred commit
        // is resolved below, not in the key handler.
        if let Some(editor) = &mut self.editor {
            renderer::draw_edit(
                surface,
                &self.state,
                self.theme.palette(),
                editor,
                self.caret_on,
                font_cx,
            );

            // The caret was just drawn: close the input->caret latency probe.
            // Measured at CPU-paint completion (one GPU submit, ~0.5-1 ms, still
            // ahead). The `performance.measure` lands in the browser Performance
            // panel; the duration also feeds the HUD (`inlat`). A 10 KB paste
            // travels the same Text path, so it is covered too.
            if let Some(t0) = self.pending_input_mark.take() {
                self.draw_stats.input_latency_ms = (self.perf.now() - t0) as f32;
                let _ = self
                    .perf
                    .measure_with_start_mark("ambre:input-to-caret", "ambre:input");
            }
        }

        // Resolve a deferred cell-edit commit now that the flush applied every
        // queued edit (grid Enter / Tab / Up / Down / click-outside / Ctrl+Enter).
        if let Some(pc) = self.pending_commit.take() {
            match pc {
                PendingCommit::Fill => self.commit_edit_filled(),
                PendingCommit::Stay => self.commit_edit(),
                PendingCommit::Nav(dr, dc) => {
                    self.commit_edit();
                    self.nav_move(dr, dc, false);
                }
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    /// Set the grid zoom factor (`1.0` = 100%); scales cells, header strips and
    /// the cell font together (`trame-status-bar`). A no-op when unchanged. On a
    /// real change the visible range shifts, so we request the new viewport and
    /// force a full repaint. Pure local view state -- never synced.
    pub fn set_zoom(&mut self, zoom: f64) -> bool {
        if !self.state.set_zoom(zoom) {
            return false;
        }
        self.needs_full = true;
        self.viewport_changed = true;
        self.pending_viewport = Some(self.state.scroll_viewport_cmd());
        true
    }

    /// The current grid zoom factor (`1.0` = 100%), for the status-bar readout.
    pub fn zoom(&self) -> f64 {
        self.state.zoom
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // The outbox holds a command the Logic thread has not taken, so the grid
        // stops accepting input until the queue drains (`BUS-016`). Consuming
        // the event is what makes the refusal visible: a key that fell through
        // to the browser would scroll the page under a frozen grid.
        if self.is_busy() {
            return true;
        }
        // Wheel / two-finger touchpad scroll pans the viewport in any mode and
        // never disturbs the cursor or an open editor.
        if let InputEvent::Wheel {
            dx,
            dy,
            delta_mode,
            ctrl_key,
        } = ev
        {
            return self.handle_wheel(*dx, *dy, *delta_mode, *ctrl_key);
        }
        // An open edit session owns text, IME and the editing keys.
        if self.editor.is_some() {
            return self.handle_input_editing(ev);
        }
        self.handle_input_nav(ev)
    }
}
