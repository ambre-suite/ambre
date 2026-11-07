// ambre-trame/src/ui/view/frame.rs
// TrameView: document binding, event dispatch, per-frame update and paint.

use super::*;
use ambre_core::index::Pos;
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

impl TrameView {
    /// Swap the active document in place (SPA route change). Drop every
    /// document-scoped field back to its `new()` value, preserving the shared,
    /// route-independent plumbing (queues, text pipe, anchor, theme) and the
    /// current canvas size. Then force a full repaint and a fresh initial
    /// viewport fetch for the new document. No-op if already current.
    pub fn set_document(&mut self, document_id: Id<Document>) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.state = TrameUiState::new(self.state.canvas_width, self.state.canvas_height);
        self.editor = None;
        self.edit_dirty = false;
        self.initial_viewport_sent = false;
        self.stats_owed = false;
        self.pending_viewport = ViewportQueue::default();
        self.needs_full = true;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.prev_top_row = Pos::<Row>::ZERO;
        self.prev_left_col = Pos::<Col>::ZERO;
        self.cut_pending = false;
        // Retract any floating-toolbar anchor from the previous document.
        self.last_anchor = None;
        self.anchor.publish(None);
    }

    /// Canvas-side reaction to a single [`TrameEvent`] (`trame-engine`, the
    /// update from the Logic thread). Exhaustive on purpose: a new variant must
    /// fail the build here rather than be dropped silently. Variants that only
    /// affect the DOM chrome (toasts, formatting toolbar, undo/redo enablement)
    /// are explicit no-ops. They are handled in the switchboard
    /// (`ambre-app`), not here.
    pub(crate) fn dispatch_trame(&mut self, te: TrameEvent) {
        match te {
            // The whole column an auto-fit asked for (`TRAME-UI-128`): measured
            // by the next paint, which owns the font context.
            TrameEvent::ColumnContent { col, cells } => {
                self.pending_column_fit = Some((col, cells));
                self.edit_dirty = true;
            }
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
                    // A selection is lossless: the Logic thread resolves range
                    // commands against the cell it tracks. A dropped re-home
                    // would leave it pointing at a covered cell.
                    if let Err(SendError::Refused) =
                        self.send_command_lossless(self.set_active_cmd())
                    {
                        tracing::error!("Trame: the outbox is full, the merge re-home was dropped");
                    }
                    self.needs_full = true;
                }
            }
            // The active sheet may have changed (switch, or delete fallback
            // decided by the Logic thread); adopt it so edits target the right
            // sheet. The tab strip reads it from here (`UI-THREAD-003`). A
            // separate DataReset re-fetches the viewport.
            TrameEvent::SheetList { active, .. } => {
                #[cfg(target_arch = "wasm32")]
                self.active_sheet.set(active.clone());
                self.state.active_sheet = active;
                self.needs_full = true;
            }
            // A rejected lifecycle op (for example deleting the last sheet). The chrome
            // toasts it; here we log it and announce it for a11y.
            TrameEvent::Error { message } => {
                tracing::warn!("[UI] Trame rejected: {message}");
                self.text_input.announce(&message);
            }
            // A peer moved or deleted the cell an open session writes to
            // (`TRAME-UI-160`, `TRAME-UI-176`).
            TrameEvent::EditTargetMoved { session, row, col } => {
                self.follow_moved_edit(session, CellPos::new(row, col));
            }
            TrameEvent::EditTargetGone { session } => self.drop_lost_edit(session),
            // Both schema notices are the chrome's to toast. Announcing them here
            // too puts them on the a11y path, which is the only surface a
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
            // badge, and publish the flag the context menu reads. The
            // accompanying DataReset re-fetches the collapsed rows.
            TrameEvent::FilterApplied { col, .. } => {
                self.state.filtered_col = Some(col);
                #[cfg(target_arch = "wasm32")]
                self.filter_active.set(crate::chrome::SheetFilter::Filtered);
                self.needs_full = true;
            }
            TrameEvent::FilterCleared => {
                self.state.filtered_col = None;
                #[cfg(target_arch = "wasm32")]
                self.filter_active
                    .set(crate::chrome::SheetFilter::Unfiltered);
                self.needs_full = true;
            }
            // Find & Replace (`trame-find-replace`): highlight the match set and jump the
            // active cell onto the focused match. That reuses the navigation path, so an
            // off-screen match scrolls into view. An empty set clears the highlight
            // without moving the cursor. The panel's counter is published here.
            TrameEvent::FindResult { matches, active } => {
                #[cfg(target_arch = "wasm32")]
                self.find_result.set(Some(crate::chrome::FindPosition::new(
                    active,
                    Len::<ambre_bus::app::find::Match>::of(matches.len()),
                )));
                self.state.find_matches = matches.clone();
                self.needs_full = true;
                if let Some(i) = active
                    && let Some(&at) = i.to_index().and_then(|i| matches.get(i))
                {
                    self.nav_goto(at, ambre_commands::SelectionMode::Move);
                }
            }
            // Wholesale content change: a local snapshot restore (DataReset) or a
            // conditional-formatting rule change (CondFormatsUpdated,
            // `trame-cell-styles`). Both
            // invalidate the cache -- re-fetch the visible region, then full-repaint
            // with the result.
            // A change the Logic thread names cell by cell: re-fetch the part
            // of the window it covers, and nothing else (`TRAME-UI-184`).
            TrameEvent::CellsChanged { version, range } => {
                if let Some(cmd) = self.state.range_viewport_cmd(range, version) {
                    self.pending_viewport.coalesce_cells(cmd);
                }
            }
            TrameEvent::DataReset | TrameEvent::CondFormatsUpdated { .. } => {
                self.needs_full = true;
                // Queued rather than sent here, so the one flush below carries
                // it and re-offers it when the queue is full (`BUS-024`). A
                // dropped re-fetch would leave the cache showing the content
                // this event just invalidated.
                self.pending_viewport
                    .coalesce(self.state.scroll_viewport_cmd());
            }
            // Logic asks the canvas to adopt a selection it cannot derive locally.
            // An unmerge is the case: the freed cells stay selected, though the
            // span-defining merge is gone. Active cell collapses to the top-left
            // corner; the anchor sits at the bottom-right so the rectangle renders
            // whole. Mirror the active cell back to Logic and the input pipe.
            TrameEvent::SelectRange { rect } => {
                self.state.select_rect(rect);
                self.needs_full = true;
                self.viewport_changed = true;
                if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
                    tracing::error!("Trame: the outbox is full, SelectRange dropped");
                }
                self.park_input_on(rect.top_left);
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
                    html: None,
                    ambre: postcard::to_allocvec(&payload)
                        .ok()
                        .map(ambre_core::Payload::<ambre_core::ClipboardBytes>::new),
                    form: ambre_core::PasteForm::Rich,
                };
                self.last_clip = Some(blob.clone());
                self.pending_clipboard = Some(blob);
            }
            // Interop import finished (`trame-import-export`): the accompanying
            // DataReset already re-fetches the populated region, so this is purely
            // informational here (the chrome relay may toast it).
            TrameEvent::ImportComplete { .. } => {}
            // Chrome-only variants: surfaced as toasts / toolbar state / undo-redo
            // enablement by the switchboard (`ambre-app`). No canvas state
            // changes -- the cache is refreshed by the DataReset that accompanies
            // any of these that mutate content.
            TrameEvent::StyleApplied { .. }
            | TrameEvent::MergeRejected { .. }
            | TrameEvent::MergeConflictDetected { .. }
            | TrameEvent::UndoState { .. }
            | TrameEvent::SortComplete
            | TrameEvent::SortRange { .. }
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
        // Offer the held commands to the queue first: the Logic thread drained
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
                // same event in the switchboard; the two never overlap.
                Ok(UiEvent::Viewport { event, .. }) => self.dispatch_trame(event),
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                // Other UiEvent variants (session, CRUD, channels...) are not yet
                // consumed by the Trame view; ignore them for now.
                Ok(_) => {}
                Err(ambre_bus::conduit::TryRecvError::Empty) => break,
                Err(ambre_bus::conduit::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }

        // Apply the latest grid zoom selected in the status-bar menu
        // (`trame-status-bar`). `set_zoom` is a no-op when unchanged; on a change it
        // queues the viewport re-request flushed just below. The signal is
        // shared with Toile, whose range is wider. The grid writes its own
        // clamp back, so the chip reads the zoom the grid shows.
        #[cfg(target_arch = "wasm32")]
        {
            let asked = self.zoom_sig.get_untracked();
            self.set_zoom(asked);
            let [lo, .., hi] = TrameUiState::ZOOM_PRESETS;
            let shown = self.state.zoom.clamped(lo, hi);
            if shown != asked {
                self.zoom_sig.set(shown);
            }
        }

        // Advance any touchpad momentum coast before the viewport flush, so the
        // ScrollViewport it produces ships this same frame.
        self.step_momentum();
        self.step_edge_scroll();

        // The first fetch of the document joins the coalescing queue, so it
        // takes the same re-offer as every other one. Sent and forgotten, a full
        // queue on the opening frame would leave the grid empty for good.
        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            self.pending_viewport
                .coalesce(self.state.scroll_viewport_cmd());
        }

        // Flush the coalesced viewport request: at most one ScrollViewport per
        // frame, regardless of how many arrow keys fired since the last frame
        // (`BUS-012`). A full queue drops it, and the request goes back in the
        // queue for the next frame (`BUS-024`).
        if let Some(pending) = self.pending_viewport.take()
            && !self.send_command_lossy(pending.command().clone())
        {
            self.pending_viewport.re_offer(pending);
        }

        // A selection-statistics request the queue refused is offered again
        // until it leaves, so the band fills with no new selection change
        // (`TRAME-UI-185`, `BUS-024`).
        if self.stats_owed && self.send_command_lossy(TrameCommand::RequestSelectionStats) {
            self.stats_owed = false;
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
            let now = self.clock.now();
            if now - self.last_blink >= BLINK_PERIOD {
                self.caret_on = !self.caret_on;
                self.last_blink = now;
                self.edit_dirty = true;
            }
        }

        // While a clipboard marquee is active, dirty the overlay each frame so
        // the dashes crawl (`trame-clipboard`). Cleared on Escape / next copy.
        if self.state.clipboard_anchor.is_some() {
            self.edit_dirty = true;
        }

        // Sync the HTML formula bar with the grid (`trame-cell-editing`). The
        // browser copies the bar in, the sync runs on plain state, and the
        // result goes back out.
        #[cfg(target_arch = "wasm32")]
        {
            self.formula.text = self.formula_text.get_untracked();
            self.formula.action = self.formula_action.get_untracked();
            self.formula.focused = BarFocus::from_bool(formula_input_focused());
        }
        self.sync_formula_bar();
        #[cfg(target_arch = "wasm32")]
        {
            if self.formula_ref.get_untracked() != self.formula.reference {
                self.formula_ref.set(self.formula.reference.clone());
            }
            if self.formula_text.get_untracked() != self.formula.text {
                self.formula_text.set(self.formula.text.clone());
            }
            if self.formula.action.is_none() && self.formula_action.get_untracked().is_some() {
                self.formula_action.set(None);
            }
        }
    }

    pub fn plan_repaint(
        &mut self,
        _now: Instant<PageLoad>,
        hud_tick: ambre_canvas::HudTick,
    ) -> RepaintPlan {
        // Formatting lives per-cell in the persistent cell layer, so a cursor
        // move needs no special repaint -- the cells keep their runs as they
        // shift. Only a CellFormat event (handled in `update`) dirties content.
        let any_dirty = self.needs_full
            || self.viewport_changed
            || self.data_dirty
            || self.edit_dirty
            || hud_tick == ambre_canvas::HudTick::Due;

        let (content, content_strip): (ContentRepaint, Option<Rect>) = if self.needs_full {
            (ContentRepaint::Full, None)
        } else if self.viewport_changed {
            let dr = (self.state.top_row - self.prev_top_row).get();
            let dc = (self.state.left_col - self.prev_left_col).get();
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
                let dy_px = (-(dr as f64) * f64::from(ROW_HEIGHT.get()) * self.state.zoom.get())
                    .round() as i32;
                let dx_px = (-(dc as f64) * f64::from(COL_WIDTH.get()) * self.state.zoom.get())
                    .round() as i32;
                let w = self.state.canvas_width.get();
                // Bottom edge of the cell area (now the full canvas: the sheet-tab
                // strip moved to DOM chrome, so it no longer reserves a band here).
                let h = self.state.grid_bottom().get();
                // Top of the scrollable cell area: below the formula bar band
                // (`trame-cell-editing`) and the column-header strip.
                let hh = self.state.header_h().get();
                // If the scroll exceeds the viewport, nothing useful to shift.
                let abs_dx = dx_px.unsigned_abs();
                let abs_dy = dy_px.unsigned_abs();
                if abs_dx as f64 >= w || abs_dy as f64 >= h {
                    (ContentRepaint::Full, None)
                } else {
                    // Strip = the area of the destination offscreen exposed by
                    // the shift, restricted to the cell area.
                    let hw = self.state.header_w().get();
                    let strip = if dy_px > 0 {
                        Rect::new(hw, hh, w, hh + dy_px as f64)
                    } else if dy_px < 0 {
                        Rect::new(hw, h + dy_px as f64, w, h)
                    } else if dx_px > 0 {
                        Rect::new(hw, hh, hw + dx_px as f64, h)
                    } else {
                        Rect::new(w + dx_px as f64, hh, w, h)
                    };
                    (
                        ContentRepaint::Scroll {
                            dx: ambre_core::Px::<ambre_core::View>::new(dx_px as f32),
                            dy: ambre_core::Px::<ambre_core::View>::new(dy_px as f32),
                        },
                        Some(strip),
                    )
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
            LayerRepaint::Redraw
        } else {
            LayerRepaint::Reuse
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
            self.draw_stats.background = TimeSpan::<FracMs>::ZERO;
            self.draw_stats.cells = TimeSpan::<FracMs>::ZERO;
            self.draw_stats.cells_drawn = ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO;
        }

        RepaintPlan {
            content,
            content_strip,
            overlay,
            backdrop: self.theme.palette().content_bg,
        }
    }

    pub fn paint_content<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
        // Resolve a pending auto-fit now that the font context is in hand
        // (`trame-resize`): measure the column/row's cached cells and emit a resize.
        if let Some(hit) = self.pending_autofit.take() {
            self.resolve_autofit(hit, font_cx, text_cache);
        }
        if let Some((col, cells)) = self.pending_column_fit.take() {
            let width = super::interact::fit_column_width(&cells, font_cx, text_cache);
            self.send_resize_col(col, width);
        }
        renderer::draw_cells(
            surface,
            &self.state,
            self.theme.palette(),
            TextPen {
                font_cx,
                cache: text_cache,
                pass: self.text_pass,
            },
            strip,
            &*self.clock,
            &mut self.draw_stats,
        );
    }

    pub fn paint_overlay<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        renderer::draw_overlay(
            surface,
            &self.state,
            self.theme.palette(),
            TextPen {
                font_cx,
                cache: text_cache,
                pass: self.text_pass,
            },
            &*self.clock,
            &mut self.draw_stats,
        );

        // Publish the per-frame perf snapshot to the DOM status bar (the HUD no
        // longer paints on the wgpu overlay). Merge the driver timings (`stats`)
        // with the view's sub-phase breakdown and the glyph-cache misses.
        self.perf_pub.publish(PerfSnapshot {
            fps: stats.fps,
            frame: stats.frame,
            drain: stats.drain,
            draw: stats.draw,
            gpu: stats.gpu,
            background: self.draw_stats.background,
            cells: self.draw_stats.cells,
            headers: self.draw_stats.headers,
            cells_drawn: self.draw_stats.cells_drawn,
            input_latency: self.draw_stats.input_latency,
            glyph_misses: ambre_core::Count::<ambre_canvas::GlyphMisses>::of(
                text_cache.frame_misses,
            ),
        });

        // The in-cell edit field sits above everything (after the active border).
        // draw_edit flushes the editor's queued intents (a FontContext is live
        // here), so the buffer is final afterwards. That is why a deferred commit
        // is resolved below, not in the key handler.
        self.caret_drawn = None;
        if let Some(editor) = &mut self.editor {
            let caret = renderer::draw_edit(
                surface,
                &self.state,
                self.theme.palette(),
                editor,
                self.caret_on,
                font_cx,
            );
            // The pipe leaves the cell's corner for the caret inside it, so
            // the IME popup follows the text being typed (`CHROME-007`).
            if let Some(caret) = caret {
                let height =
                    ambre_core::Px::<ambre_core::View>::new(caret.height().max(1.0) as f32);
                self.text_input
                    .set_caret(view_point(caret.origin()), height);
            }
            self.caret_drawn = caret;

            // The caret was just drawn: close the input->caret latency probe.
            // Measured at CPU-paint completion (one GPU submit, ~0.5-1 ms, still
            // ahead). The `performance.measure` lands in the browser Performance
            // panel; the duration also feeds the HUD (`inlat`). A 10 KB paste
            // travels the same Text path, so it is covered too.
            if let Some(t0) = self.pending_input_mark.take() {
                self.draw_stats.input_latency = self.clock.now() - t0;
                performance_measure(PerfMeasure::InputToCaret);
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
                    self.nav_move(dr, dc, ambre_commands::SelectionMode::Move);
                }
            }
        }
        // What was typed behind the commit now reaches the cell the commit
        // moved to. A committing chord among it defers the rest again.
        if self.pending_commit.is_none() {
            for ev in std::mem::take(&mut self.deferred_input) {
                let _ = self.handle_input(&ev);
            }
        }
    }

    /// The canvas now lays out at `size` logical pixels (`CANVAS-044`).
    pub fn resize(&mut self, size: ambre_core::Size<ambre_core::Px<ambre_core::CanvasCss>>) {
        self.state.resize(
            ambre_core::Px::<ambre_core::Surface>::new(f64::from(size.width.get())),
            ambre_core::Px::<ambre_core::Surface>::new(f64::from(size.height.get())),
        );
        self.needs_full = true;
    }

    /// Set the grid zoom factor (`1.0` = 100%); scales cells, header strips and
    /// the cell font together (`trame-status-bar`). A no-op when unchanged. On a
    /// real change the visible range shifts, so we request the new viewport and
    /// force a full repaint. Pure local view state -- never synced.
    pub fn set_zoom(&mut self, zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>) -> bool {
        if !self.state.set_zoom(zoom) {
            return false;
        }
        self.needs_full = true;
        self.viewport_changed = true;
        self.pending_viewport
            .coalesce(self.state.scroll_viewport_cmd());
        true
    }

    /// The current grid zoom factor (`1.0` = 100%), for the status-bar readout.
    pub fn zoom(&self) -> ambre_core::Scale<ambre_core::Doc, ambre_core::View> {
        self.state.zoom
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> ambre_draw::Consumed {
        // The outbox holds a command the Logic thread has not taken, so the grid
        // stops accepting input until the queue drains (`BUS-016`). Consuming
        // the event is what makes the refusal visible: a key that fell through
        // to the browser would scroll the page under a frozen grid.
        if self.is_busy() {
            return ambre_draw::Consumed::Yes;
        }
        // Wheel / two-finger touchpad scroll pans the viewport in any mode and
        // never disturbs the cursor or an open editor.
        if let InputEvent::Wheel { delta, mods } = ev {
            return self.handle_wheel(*delta, *mods);
        }
        // A commit waits for the paint flush. A text or an editing key typed
        // before it belongs to the next cell, never to the buffer being
        // committed (`TRAME-UI-183`).
        if self.pending_commit.is_some() && waits_for_commit(ev) {
            self.deferred_input.push(ev.clone());
            return ambre_draw::Consumed::Yes;
        }
        // An open edit session owns text, IME and the editing keys.
        if self.editor.is_some() {
            return self.handle_input_editing(ev);
        }
        self.handle_input_nav(ev)
    }
}

/// Whether `ev` waits behind a pending commit. Three keys pass. A printable
/// key leaves its text to the `Text` event. A chord with the primary modifier
/// goes to the chrome keymap. The browser serves `Shift+Delete` as a cut.
fn waits_for_commit(ev: &InputEvent) -> bool {
    use ambre_commands::{Key, ModifierKeys};
    match ev {
        InputEvent::Text { .. }
        | InputEvent::CompositionUpdate { .. }
        | InputEvent::CompositionEnd { .. } => true,
        InputEvent::Key { key, mods } => {
            let passes = mods.contains(ModifierKeys::PRIMARY)
                || matches!(key, Key::Char(_) | Key::Space)
                || (*key == Key::Delete && mods.contains(ModifierKeys::SHIFT));
            !passes
        }
        _ => false,
    }
}
