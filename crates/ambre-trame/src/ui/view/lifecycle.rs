// ambre-trame/src/ui/view/lifecycle.rs
// TrameView: construction and inline-edit lifecycle (begin/commit/cancel).

use super::*;

impl TrameView {
    #[allow(clippy::too_many_arguments)]
    /// # Panics
    ///
    /// When the browser exposes no `window` or no Performance API. The view
    /// measures every frame against a budget, so neither has a fallback.
    pub fn new(
        document_id: DocumentId,
        to_logic: ambre_bus::conduit::Sender<LogicCommand>,
        from_logic: ambre_bus::conduit::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        // When embedded as a `doc_embed` child (`toile-embeds`), the child view
        // must not grab the shared text pipe on construction. The top-level
        // document owns the caret until the user clicks into the embed. A
        // top-level view passes `false` and focuses immediately, as before.
        embedded: bool,
        anchor: AnchorPublisher,
        perf_pub: PerfPublisher,
        busy_pub: BusyPublisher,
        filter_open: FilterOpenPublisher,
        theme_ctl: ThemeControl,
        #[cfg(target_arch = "wasm32")] formula_ref: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_text: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_action: RwSignal<Option<FormulaAction>>,
        #[cfg(target_arch = "wasm32")] sel_stats: RwSignal<Option<crate::chrome::SelStats>>,
        #[cfg(target_arch = "wasm32")] ctx_scope: RwSignal<crate::ui::grid::CtxScope>,
        #[cfg(target_arch = "wasm32")] zoom_sig: RwSignal<f64>,
    ) -> Self {
        let window = web_sys::window().expect("no window");
        let perf = window.performance().expect("no Performance API");
        let dpr = window.device_pixel_ratio();
        let theme = theme_ctl.get();

        // Focus the pipe up front so the keyboard works without a click. Park it
        // on the active cell so a first keystroke or IME opens there. Skipped for
        // an embedded child, which must not steal the caret from its host board.
        if !embedded {
            text_input.focus();
        }

        Self {
            document_id,
            outbox: Outbox::new(to_logic.clone()),
            busy_pub,
            to_logic,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
            text_input,
            dpr,
            anchor,
            last_anchor: None,
            perf_pub,
            filter_open,
            #[cfg(target_arch = "wasm32")]
            sel_stats,
            #[cfg(target_arch = "wasm32")]
            ctx_scope,
            theme_ctl,
            theme,
            #[cfg(target_arch = "wasm32")]
            formula_ref,
            #[cfg(target_arch = "wasm32")]
            formula_text,
            #[cfg(target_arch = "wasm32")]
            formula_shown: String::new(),
            #[cfg(target_arch = "wasm32")]
            formula_action,
            #[cfg(target_arch = "wasm32")]
            zoom_sig,
            editor: None,
            pending_commit: None,
            editor_text_drag: false,
            selecting_drag: false,
            axis_drag: None,
            resize_drag: None,
            pending_autofit: None,
            pending_column_fit: None,
            fill_drag: None,
            reorder_drag: None,
            hover_cursor: String::new(),
            edit_dirty: false,
            caret_on: true,
            last_blink_ms: 0.0,
            pending_input_mark: None,
            perf,
            text_enabled: true,
            draw_stats: DrawStats::default(),
            initial_viewport_sent: false,
            pending_viewport: ViewportQueue::default(),
            fatal: false,
            needs_full: true,
            viewport_changed: false,
            data_dirty: false,
            prev_top_row: 0,
            prev_left_col: 0,
            scroll: Default::default(),
            drag_point: None,
            cut_pending: false,
            pending_clipboard: None,
            last_clip: None,
        }
    }

    /// Send a lossy `TrameCommand` to the Logic thread: a request that changes
    /// nothing, over a state the screen already draws.
    ///
    /// Returns whether the command reached the queue. A full one drops it and
    /// nothing holds it. The caller MUST read this and either offer the request
    /// again next frame or undo what it drew for it (`BUS-024`).
    /// A caller that ignores it leaves the screen ahead of the document, which
    /// is the state `BUS-018` bans.
    ///
    /// The class carries no document mutation: `send_lossy` refuses one
    /// (`BUS-023`), and the outbox is the path such a command owes
    /// (`send_command_lossless`).
    ///
    /// Runs on the page's main thread, so it must never block.
    pub(crate) fn send_command_lossy(&self, cmd: TrameCommand) -> bool {
        if self.fatal {
            return false;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Trame(cmd),
        };
        match ambre_bus::send_lossy(&self.to_logic, msg) {
            LossyOutcome::Sent => true,
            LossyOutcome::Dropped => {
                tracing::warn!("Trame: UI->Logic channel full, the lossy request was dropped");
                false
            }
            LossyOutcome::Refused => {
                tracing::error!("Trame: a document mutation was offered to the lossy class");
                false
            }
            LossyOutcome::Disconnected => {
                tracing::error!("Trame: UI->Logic channel disconnected");
                false
            }
        }
    }

    /// Send a lossless `TrameCommand` to the Logic thread (edits, cell
    /// selection) through the outbox.
    ///
    /// On a full queue the outbox holds the command for a later frame
    /// (`BUS-011`), and the grid refuses input until the queue drains
    /// (`BUS-016`). `Ok(())` therefore means the command is on its way, not
    /// that it already left.
    ///
    /// `Err` is the command the outbox took nowhere. A caller that mirrors it
    /// into its own grid cache MUST read this and decline (`BUS-018`). A cell
    /// showing an edit the document never received hides the loss.
    pub(crate) fn send_command_lossless(&mut self, cmd: TrameCommand) -> Result<(), SendError> {
        if self.fatal {
            return Err(SendError::Disconnected);
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Trame(cmd),
        };
        match self.outbox.send(msg) {
            SendOutcome::Sent => Ok(()),
            SendOutcome::Held => {
                tracing::warn!("Trame: UI->Logic channel full, command held for a later frame");
                Ok(())
            }
            SendOutcome::Refused => Err(SendError::Refused),
            SendOutcome::Disconnected => Err(SendError::Disconnected),
        }
    }

    /// Offer the held lossless commands to the queue, then publish the busy
    /// level the chrome shows and the input gate reads (`BUS-016`).
    ///
    /// Called from the frame hook.
    pub(crate) fn flush_outbox(&mut self) {
        if self.fatal {
            return;
        }
        if self.outbox.flush() == SendOutcome::Disconnected {
            self.signal_fatal("UI->Logic channel disconnected");
            return;
        }
        if self.busy_pub.publish(self.outbox.is_busy()) && !self.outbox.is_busy() {
            tracing::info!("Trame: the UI->Logic channel cleared, input resumes");
        }
    }

    /// Whether the grid is refusing input because the outbox holds a command.
    pub(crate) fn is_busy(&self) -> bool {
        self.outbox.is_busy()
    }

    /// Enter the inert fatal state: a worker thread is gone, so no command can
    /// be answered. Logged exactly once; the page may surface a fatal screen
    /// (see `ambre-spec-architecture.md`, `ARCH-008`).
    pub(crate) fn signal_fatal(&mut self, reason: &str) {
        if self.fatal {
            return;
        }
        self.fatal = true;
        tracing::error!("Trame: fatal -- {reason}; the UI is now inert");
        // Latch the canvas terminal screen. A Sync-worker death is a SEPARATE
        // wasm instance, so it reaches us only as this queue disconnect; the
        // panic-hook latch never fires for it. Forward it to the host here.
        ambre_canvas::signal_fatal(reason);
    }

    /// Announce the active cell to screen readers via the live region (content
    /// a11y, 2.F): the cell reference plus its value, for example `"B2, 42"`. Called on
    /// every selection change so navigating the grid is audible, like a native
    /// spreadsheet.
    pub(crate) fn announce_active(&self) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let value = self.cell_seed(row, col);
        let desc = if value.is_empty() {
            format!("{}{}, empty", col_label(col), row + 1)
        } else {
            format!("{}{}, {}", col_label(col), row + 1, value)
        };
        self.text_input.announce(&desc);
        self.refresh_selection_stats();
    }

    /// Refresh the status-bar selection aggregates (`trame-status-bar`) after a
    /// selection change. A lone cell shows nothing, so we clear the signal
    /// locally. Any multi-cell selection asks the Logic thread (which holds every
    /// cell, not just the viewport) for the aggregates. The reply is a
    /// [`TrameEvent::SelectionStats`], mirrored into the signal by the launcher's
    /// event loop.
    ///
    /// The request is lossy, and a full queue drops it. The band is then
    /// cleared rather than left showing another selection's totals
    /// (`TRAME-UI-136`): an unanswered band reads as an absence, stale numbers
    /// as a fact.
    pub(crate) fn refresh_selection_stats(&self) {
        if stats_band_clears(self.state.selection.is_multi(), || {
            self.send_command_lossy(TrameCommand::RequestSelectionStats)
        }) {
            #[cfg(target_arch = "wasm32")]
            self.sel_stats.set(None);
        }
    }

    /// The active cell's spreadsheet reference (for example `"B2"`), shown in the
    /// formula bar's name box (`trame-cell-editing`).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn active_cell_ref(&self) -> String {
        format!(
            "{}{}",
            col_label(self.state.active_col()),
            self.state.active_row() + 1
        )
    }

    /// Recompute the active cell's on-screen anchor and publish it for the
    /// floating selection toolbar. The anchor is the centre of the cell's top
    /// edge in CSS px. It is `None` when the cell is scrolled out of the grid
    /// area. Cheap; called every frame so the toolbar tracks pans, but only
    /// forwarded to the chrome on change.
    pub(crate) fn publish_anchor(&mut self) {
        let next = self
            .state
            .cell_pixel_rect(self.state.active_row(), self.state.active_col())
            .map(|(x, y, w, _h)| Anchor {
                x: (x + w / 2.0) / self.dpr,
                y: y / self.dpr,
            });
        if next != self.last_anchor {
            self.last_anchor = next;
            self.anchor.publish(next);
        }
    }

    /// Show the caret solid and restart its blink clock. Called on every edit
    /// interaction (typing, IME, caret motion). That is the one chokepoint where
    /// user input mutates the editor, so it also arms the input->caret latency
    /// probe.
    pub(crate) fn reset_blink(&mut self) {
        self.caret_on = true;
        self.last_blink_ms = self.perf.now();
        self.edit_dirty = true;
        // Arm the latency probe: timestamp now (~keydown), and drop a
        // `performance.mark` so the measure shows in the browser Performance panel.
        self.pending_input_mark = Some(self.perf.now());
        let _ = self.perf.mark("ambre:input");
    }

    /// Seed text for an editor opened on `(row, col)`: the cell's current text,
    /// or empty if not cached.
    pub(crate) fn cell_seed(&self, row: u32, col: u32) -> String {
        self.state
            .cell_cache
            .get(&(row, col))
            .map(|c| c.raw.clone())
            .unwrap_or_default()
    }

    /// Slide the textarea pipe onto a cell (CSS px) so the IME popup anchors
    /// there. The grid works in physical px; the DOM positions in CSS px.
    pub(crate) fn park_input_on(&self, row: u32, col: u32) {
        if let Some((x, y, _, h)) = self.state.cell_pixel_rect(row, col) {
            self.text_input
                .set_caret(x / self.dpr, y / self.dpr, h / self.dpr);
        }
    }

    /// Open an edit session on the active cell, seeded with `seed`. Focuses the
    /// pipe (clearing its buffer) and positions it on the cell.
    pub(crate) fn begin_edit(&mut self, seed: String) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        self.editor = Some(CellEditor::new(row, col, seed.clone()));
        self.editor_text_drag = false;
        self.text_input.clear();
        self.text_input.focus();
        self.park_input_on(row, col);
        self.reset_blink();
        #[cfg(target_arch = "wasm32")]
        self.show_formula_text(seed);
    }

    /// Open an edit session on the active cell WITHOUT touching focus or the
    /// formula bar text. Used when an edit originates from the HTML formula bar
    /// input, which keeps DOM focus and owns the buffer. The in-cell editor is
    /// only the rendering/commit vehicle. Stealing focus to the textarea pipe
    /// (as [`begin_edit`](Self::begin_edit) does) would abort the user's typing.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn begin_edit_external(&mut self) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let seed = self.cell_seed(row, col);
        self.editor = Some(CellEditor::new(row, col, seed));
        self.editor_text_drag = false;
        self.reset_blink();
    }

    /// Commit the current edit into the CRDT (via Logic) and update the local
    /// cache optimistically, then leave edit mode. No-op if not editing.
    pub(crate) fn commit_edit(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let text = editor.text();
        tracing::info!(
            "[UI] commit_edit -> sending SetCellText row={} col={} text='{:.80}'",
            editor.row,
            editor.col,
            text,
        );
        let sent = self.send_command_lossless(TrameCommand::SetCellText {
            row: editor.row,
            col: editor.col,
            text: text.clone(),
        });
        // Close the undo group on cell exit so the whole edit is one undo item
        // (`trame-cell-editing`). It travels only behind an edit that left: a
        // boundary around nothing would seal the previous group early.
        if sent.is_ok()
            && self
                .send_command_lossless(TrameCommand::CommitEdit)
                .is_err()
        {
            // A dropped boundary leaves the group open, so the next edit joins
            // it and one undo takes both. It must not be dropped in silence
            // (`TRAME-UI-142`).
            tracing::error!("Trame: the undo group boundary was refused, the group stays open");
        }
        // The grid cache mirrors the document, so it takes the text only when
        // the command is on its way (`BUS-018`). A refused edit leaves the cell
        // as the document holds it. The user then reads the loss, rather than a
        // value no peer will ever receive.
        if sent.is_ok() {
            self.state.set_cell_text(editor.row, editor.col, text);
        } else {
            tracing::error!("[UI] the edit was refused, the cell keeps its stored value");
        }
        // Content layer must repaint to show the committed text; overlay too, to
        // drop the edit field.
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
        // Announce the committed value to assistive tech.
        self.announce_active();
    }

    /// Commit the current edit's value into *every* selected cell (Ctrl+Enter),
    /// then leave edit mode keeping the selection and active cell put. Each cell
    /// gets the same raw text; the whole fill is one undo group (a single
    /// `CommitEdit` seals it). For a lone single-cell selection this is identical
    /// to [`commit_edit`](Self::commit_edit) but without the cursor advance.
    ///
    /// Note: the raw text is copied verbatim, so a filled *formula* keeps its
    /// references as typed. There is no per-cell relative reindexing; that lives
    /// in the Logic thread's paste/fill path, not the UI.
    pub(crate) fn commit_edit_filled(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let text = editor.text();
        let cells = self.state.selection.selected_cells();
        tracing::info!(
            "[UI] commit_edit_filled -> {} cells, text='{:.80}'",
            cells.len(),
            text
        );
        // One batch command so the whole fill is a single undo item (a stream of
        // SetCellText would be one undo item per cell).
        let batch = cells.iter().map(|&(r, c)| (r, c, text.clone())).collect();
        let sent = self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch });
        // The optimistic update mirrors the batch, so it runs only when the
        // batch is on its way (`BUS-018`).
        if sent.is_ok() {
            for &(row, col) in &cells {
                self.state.set_cell_text(row, col, text.clone());
            }
        } else {
            tracing::error!("[UI] the fill was refused, the cells keep their stored values");
        }
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
        self.announce_active();
    }

    /// Abandon the current edit without writing anything, then leave edit mode.
    pub(crate) fn cancel_edit(&mut self) {
        if self.editor.take().is_some() {
            // Content is untouched; only the overlay field must be cleared. The
            // formula bar reverts to the (unchanged) active cell's stored value,
            // not blank. Blanking it would read as "user typed empty" next frame
            // and could re-open an edit that clears the cell.
            self.edit_dirty = true;
            self.text_input.clear();
            #[cfg(target_arch = "wasm32")]
            self.show_formula_text(
                self.cell_seed(self.state.active_row(), self.state.active_col()),
            );
        }
    }

    /// Write `text` into the formula bar, remembering it as the view's own
    /// (`TRAME-UI-155`).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn show_formula_text(&mut self, text: String) {
        self.formula_shown.clone_from(&text);
        self.formula_text.set(text);
    }
}

/// Whether the status band clears after a selection change, before any reply
/// (`TRAME-UI-136`).
///
/// A single-cell selection sends nothing and clears. A multi-cell one sends a
/// lossy request, and the band clears when that request did not reach the Logic
/// thread. No answer is coming, and the last selection's totals must not stay
/// up.
fn stats_band_clears(is_multi: bool, send: impl FnOnce() -> bool) -> bool {
    if is_multi { !send() } else { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_cell_clears_and_sends_nothing() {
        let mut sent = false;
        assert!(stats_band_clears(false, || {
            sent = true;
            true
        }));
        assert!(!sent, "a single cell must not send a request");
    }

    #[test]
    fn a_dropped_request_clears_the_band() {
        assert!(stats_band_clears(true, || false));
    }

    #[test]
    fn a_sent_request_keeps_the_last_answer() {
        assert!(!stats_band_clears(true, || true));
    }
}
