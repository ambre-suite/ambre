// ambre-trame/src/ui/view/lifecycle.rs
// TrameView: construction and inline-edit lifecycle (begin/commit/cancel).

use super::*;
use ambre_bus::{Instant, PageLoad};
use ambre_canvas::BusyLevel;
use ambre_core::index::{Delta, Pos};

impl TrameView {
    /// A view of `document_id` on a canvas of `canvas`, wired by `wiring`.
    /// The view reads time from the clock of `wiring`, never the browser's.
    pub fn new(
        document_id: Id<Document>,
        canvas: ambre_core::Size<ambre_core::Px<ambre_core::Surface>>,
        wiring: TrameWiring,
    ) -> Self {
        let TrameWiring {
            to_logic,
            from_logic,
            text_input,
            embedding,
            anchor,
            perf_pub,
            busy_pub,
            filter_open,
            theme_ctl,
            clock,
            #[cfg(target_arch = "wasm32")]
            signals,
        } = wiring;
        #[cfg(target_arch = "wasm32")]
        let TrameSignals {
            formula_ref,
            formula_text,
            formula_action,
            sel_stats,
            ctx_scope,
            zoom: zoom_sig,
            active_sheet,
            filter_active,
            find_result,
        } = signals;
        let (canvas_width, canvas_height) = (canvas.width, canvas.height);
        let theme = theme_ctl.get();

        // Focus the pipe up front so the keyboard works without a click. Park it
        // on the active cell so a first keystroke or IME opens there. Skipped for
        // an embedded child, which must not steal the caret from its host board.
        if embedding == ambre_canvas::Embedding::Standalone {
            text_input.name_content(ambre_canvas::ContentKind::Spreadsheet);
            text_input.focus();
        }

        Self {
            document_id,
            outbox: Outbox::new(to_logic),
            busy_pub,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
            text_input,
            anchor,
            last_anchor: None,
            perf_pub,
            filter_open,
            #[cfg(target_arch = "wasm32")]
            sel_stats,
            #[cfg(target_arch = "wasm32")]
            ctx_scope,
            menu_scope: crate::ui::grid::CtxScope::default(),
            theme_ctl,
            theme,
            #[cfg(target_arch = "wasm32")]
            formula_ref,
            #[cfg(target_arch = "wasm32")]
            formula_text,
            formula: FormulaBar::default(),
            #[cfg(target_arch = "wasm32")]
            formula_action,
            #[cfg(target_arch = "wasm32")]
            zoom_sig,
            #[cfg(target_arch = "wasm32")]
            active_sheet,
            #[cfg(target_arch = "wasm32")]
            filter_active,
            #[cfg(target_arch = "wasm32")]
            find_result,
            editor: None,
            next_edit_session: ambre_bus::Token::FIRST,
            pending_commit: None,
            deferred_input: Vec::new(),
            editor_text_drag: false,
            selecting_drag: false,
            axis_drag: None,
            resize_drag: None,
            pending_autofit: None,
            pending_column_fit: None,
            fill_drag: None,
            reorder_drag: None,
            hover_cursor: GridCursor::Unset,
            edit_dirty: false,
            caret_on: true,
            caret_drawn: None,
            last_blink: Instant::<PageLoad>::new(0.0),
            pending_input_mark: None,
            clock,
            text_pass: TextPass::WithText,
            draw_stats: DrawStats::default(),
            initial_viewport_sent: false,
            stats_owed: false,
            pending_viewport: ViewportQueue::default(),
            fatal: false,
            needs_full: true,
            viewport_changed: false,
            data_dirty: false,
            prev_top_row: Pos::<Row>::ZERO,
            prev_left_col: Pos::<Col>::ZERO,
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
    /// The class carries only a command `TrameCommand::traffic` names lossy:
    /// `send_lossy` refuses any other (`BUS-023`), which owes
    /// `send_command_lossless`.
    ///
    /// Runs on the page's main thread, so it must never block.
    pub(crate) fn send_command_lossy(&self, cmd: TrameCommand) -> bool {
        if self.fatal {
            return false;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: cmd,
        };
        match self.outbox.send_lossy(msg) {
            LossyOutcome::Sent => true,
            LossyOutcome::Dropped => {
                tracing::warn!("Trame: UI->Logic channel full, the lossy request was dropped");
                false
            }
            LossyOutcome::Refused => {
                tracing::error!("Trame: a lossless command was offered to the lossy class");
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
    ///
    /// Every `Err` is logged here, so no caller can drop a command in silence
    /// (`TRAME-UI-142`). The fatal latch logged its own cause once.
    pub(crate) fn send_command_lossless(&mut self, cmd: TrameCommand) -> Result<(), SendError> {
        if self.fatal {
            return Err(SendError::Disconnected);
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: cmd,
        };
        match self.outbox.send(msg) {
            SendOutcome::Sent => Ok(()),
            SendOutcome::Held => {
                tracing::warn!("Trame: UI->Logic channel full, command held for a later frame");
                Ok(())
            }
            SendOutcome::Refused => {
                tracing::error!("Trame: the outbox is full, a lossless command was refused");
                Err(SendError::Refused)
            }
            SendOutcome::Disconnected => {
                tracing::error!("Trame: UI->Logic channel disconnected, a command was lost");
                Err(SendError::Disconnected)
            }
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
        if self
            .busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()))
            && !self.outbox.is_busy()
        {
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
        // Latch the canvas terminal screen. The page never waits on a
        // disconnection, which comes late. A panic latches through the hook,
        // and a failed Sync worker through its error event (`ARCH-014`).
        ambre_canvas::signal_fatal(reason);
    }

    /// Announce the active cell to screen readers via the live region (content
    /// a11y, 2.F): the cell reference plus its value, for example `"B2, 42"`. Called on
    /// every selection change so navigating the grid is audible, like a native
    /// spreadsheet.
    pub(crate) fn announce_active(&mut self) {
        let at = self.state.active_cell();
        let value = self.cell_seed(at);
        self.text_input.announce(&cell_announcement(at, &value));
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
    /// as a fact. The refused request stays owed, and the frame offers it again
    /// until it leaves (`TRAME-UI-185`).
    pub(crate) fn refresh_selection_stats(&mut self) {
        let is_multi = self.state.selection.is_multi();
        let mut sent = false;
        if stats_band_clears(is_multi, || {
            sent = self.send_command_lossy(TrameCommand::RequestSelectionStats);
            sent
        }) {
            #[cfg(target_arch = "wasm32")]
            self.sel_stats.set(None);
        }
        self.stats_owed = is_multi && !sent;
    }

    /// The active cell's spreadsheet reference (for example `"B2"`), shown in the
    /// formula bar's name box (`trame-cell-editing`).
    pub(crate) fn active_cell_ref(&self) -> String {
        format!(
            "{}{}",
            crate::columns::col_to_alpha(self.state.active_col()),
            crate::columns::row_label(self.state.active_row())
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
            .cell_pixel_rect(self.state.active_cell())
            .map(|cell| view_point(ambre_draw::Point::new(cell.center().x, cell.y0)));
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
        let now = self.clock.now();
        self.last_blink = now;
        self.edit_dirty = true;
        // Arm the latency probe: timestamp now (~keydown), and drop a
        // `performance.mark` so the measure shows in the browser Performance panel.
        self.pending_input_mark = Some(now);
        performance_mark(PerfMark::Input);
    }

    /// Seed text for an editor opened on `at`: the cell's current text, or
    /// empty if not cached.
    pub(crate) fn cell_seed(&self, at: CellPos) -> String {
        self.state
            .cell_cache
            .get(&at)
            .map(|c| c.raw.as_str().to_owned())
            .unwrap_or_default()
    }

    /// Slide the textarea pipe onto a cell (CSS px) so the IME popup anchors
    /// there. The grid lays out in CSS px too (`CANVAS-032`).
    pub(crate) fn park_input_on(&self, at: CellPos) {
        if let Some(cell) = self.state.cell_pixel_rect(at) {
            let height = ambre_core::Px::<ambre_core::View>::new(cell.height() as f32);
            self.text_input
                .set_caret(view_point(ambre_draw::Point::new(cell.x0, cell.y0)), height);
        }
    }

    /// Open an edit session on the active cell, seeded with `seed`. Focuses the
    /// pipe (clearing its buffer) and positions it on the cell.
    pub(crate) fn begin_edit(&mut self, seed: String) {
        let at = self.state.active_cell();
        if !self.open_session(at, seed.clone()) {
            return;
        }
        self.editor_text_drag = false;
        self.text_input.clear();
        self.text_input.focus();
        self.park_input_on(at);
        self.reset_blink();
        self.show_formula_text(seed);
    }

    /// Open an edit session on the active cell WITHOUT touching focus or the
    /// formula bar text. Used when an edit originates from the HTML formula bar
    /// input, which keeps DOM focus and owns the buffer. The in-cell editor is
    /// only the rendering/commit vehicle. Stealing focus to the textarea pipe
    /// (as [`begin_edit`](Self::begin_edit) does) would abort the user's typing.
    pub(crate) fn begin_edit_external(&mut self) {
        let at = self.state.active_cell();
        let seed = self.cell_seed(at);
        if !self.open_session(at, seed) {
            return;
        }
        self.editor_text_drag = false;
        self.reset_blink();
    }

    /// Open the editor on `at` under a fresh session token, once the
    /// Logic thread has been asked to pin that cell (`TRAME-UI-160`). A session
    /// Logic never heard of could not commit, so a refused request opens none.
    fn open_session(&mut self, at: CellPos, seed: String) -> bool {
        let session = self.next_edit_session;
        self.next_edit_session = session.next();
        let begin = TrameCommand::BeginCellEdit {
            session,
            row: at.row,
            col: at.col,
        };
        if self.send_command_lossless(begin).is_err() {
            tracing::error!("Trame: the outbox is full, the edit session did not open");
            return false;
        }
        self.editor = Some(CellEditor::new(at, session, seed));
        true
    }

    /// Close session `session` with no write, because a peer deleted its cell
    /// (`TRAME-UI-176`). The chrome toasts the loss; this side drops the editor
    /// and announces it. A no-op once the session closed.
    pub(crate) fn drop_lost_edit(&mut self, session: ambre_bus::Token<crate::EditSession>) {
        if self.editor.as_ref().is_none_or(|e| e.session != session) {
            return;
        }
        self.editor = None;
        self.pending_commit = None;
        self.edit_dirty = true;
        self.needs_full = true;
        self.text_input.clear();
        self.text_input
            .announce("A collaborator deleted the cell you were editing. Nothing was written.");
        self.show_formula_text(self.cell_seed(self.state.active_cell()));
    }

    /// Carry session `session` to `at`, where a peer moved its cell. The
    /// editor and the active cell follow, so the cell on screen is the one the
    /// commit reaches (`TRAME-UI-160`).
    pub(crate) fn follow_moved_edit(
        &mut self,
        session: ambre_bus::Token<crate::EditSession>,
        at: CellPos,
    ) {
        if !self.editor.as_mut().is_some_and(|e| e.follow(session, at)) {
            return;
        }
        self.edit_dirty = true;
        self.needs_full = true;
        let mv = self.state.move_to(at, ambre_commands::SelectionMode::Move);
        self.apply_cursor_move(mv);
    }

    /// Commit the current edit into the CRDT (via Logic) and update the local
    /// cache optimistically, then leave edit mode. No-op if not editing.
    pub(crate) fn commit_edit(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let text = editor.text();
        tracing::info!(
            "[UI] commit_edit -> session {} at row={} col={} text='{:.80}'",
            editor.session,
            editor.at.row.get(),
            editor.at.col.get(),
            text,
        );
        let sent = self.send_command_lossless(editor.commit_command(text.clone()));
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
            self.state.set_cell_text(editor.at, text);
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
        let batch = cells.iter().map(|&at| (at, text.clone())).collect();
        let sent = self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch });
        self.end_session(editor.session);
        // The optimistic update mirrors the batch, so it runs only when the
        // batch is on its way (`BUS-018`).
        if sent.is_ok() {
            for &at in &cells {
                self.state.set_cell_text(at, text.clone());
            }
        } else {
            tracing::error!("[UI] the fill was refused, the cells keep their stored values");
        }
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
        self.announce_active();
    }

    /// Release session `session` on the Logic side with no write. A lost
    /// release leaves Logic pinning a cell no editor shows, so a later delete
    /// of it would report a session already closed.
    fn end_session(&mut self, session: ambre_bus::Token<crate::EditSession>) {
        if self
            .send_command_lossless(TrameCommand::EndCellEdit { session })
            .is_err()
        {
            tracing::error!("Trame: the outbox is full, the edit session release was dropped");
        }
    }

    /// Abandon the current edit without writing anything, then leave edit mode.
    pub(crate) fn cancel_edit(&mut self) {
        if let Some(editor) = self.editor.take() {
            self.end_session(editor.session);
            // Content is untouched; only the overlay field must be cleared. The
            // formula bar reverts to the (unchanged) active cell's stored value,
            // not blank. Blanking it would read as "user typed empty" next frame
            // and could re-open an edit that clears the cell.
            self.edit_dirty = true;
            self.text_input.clear();
            self.show_formula_text(self.cell_seed(self.state.active_cell()));
        }
    }

    /// Sync the formula bar with the grid, once per frame (`trame-cell-editing`).
    ///
    /// While the bar has the focus, it owns the text, and the view must not
    /// overwrite what the user typed. A text the view did not write means an
    /// edit is in progress, which opens or drives a session (`TRAME-UI-155`).
    /// Without the focus, the bar mirrors the editor buffer, or the stored
    /// value when no session is open (`TRAME-UI-055`).
    pub(crate) fn sync_formula_bar(&mut self) {
        self.formula.reference = self.active_cell_ref();
        if self.formula.focused == BarFocus::Focused {
            let text = self.formula.text.clone();
            if let Some(ed) = self.editor.as_mut() {
                if ed.text() != text {
                    ed.set_text(&text);
                    self.edit_dirty = true;
                }
            } else {
                let seed = self.cell_seed(self.state.active_cell());
                match self.formula.sync(&seed) {
                    // The first character typed in the bar opens a session on
                    // the active cell, seeded with what was typed. The focus
                    // stays in the bar (`TRAME-UI-092`).
                    super::FormulaBarSync::OpenSession => {
                        self.begin_edit_external();
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_text(&text);
                            self.edit_dirty = true;
                        }
                    }
                    super::FormulaBarSync::Refresh => self.show_formula_text(seed),
                    super::FormulaBarSync::Idle => {}
                }
            }
        } else if let Some(ed) = self.editor.as_ref() {
            let buffer = ed.text();
            if self.formula.text != buffer {
                self.show_formula_text(buffer);
            }
        } else {
            let seed = self.cell_seed(self.state.active_cell());
            if self.formula.text != seed {
                self.show_formula_text(seed);
            }
        }
        // Without the focus, the bar holds only what the view wrote. That text
        // is the base a later keystroke is told apart from.
        if self.formula.focused == BarFocus::Blurred {
            self.formula.shown = self.formula.text.clone();
        }
        let Some(action) = self.formula.action.take() else {
            return;
        };
        match action {
            // A commit from the bar writes the bar's text (`TRAME-UI-169`).
            FormulaAction::Commit(dir) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.set_text(&self.formula.text);
                }
                self.commit_edit();
                match dir {
                    CommitDirection::Down => {
                        self.nav_move(
                            Delta::<Row>::new(1),
                            Delta::<Col>::ZERO,
                            ambre_commands::SelectionMode::Move,
                        );
                    }
                    CommitDirection::Right => {
                        self.nav_move(
                            Delta::<Row>::ZERO,
                            Delta::<Col>::new(1),
                            ambre_commands::SelectionMode::Move,
                        );
                    }
                    CommitDirection::Stay => {}
                }
                // Return the focus to the grid and reset the bar to the cell we
                // landed on. Otherwise the bar still holds the committed value.
                // Say the bar keeps the focus for one frame. The focused branch
                // above then reads that value as typed on the new cell. It
                // opens an edit there and writes the value into that cell too.
                self.text_input.focus();
                self.show_formula_text(self.cell_seed(self.state.active_cell()));
            }
            // Ctrl/Cmd+Enter: fill the value into every selected cell and keep
            // the selection put (no cursor advance).
            FormulaAction::CommitFill => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.set_text(&self.formula.text);
                }
                self.commit_edit_filled();
                self.text_input.focus();
                self.show_formula_text(self.cell_seed(self.state.active_cell()));
            }
            FormulaAction::Cancel => {
                self.cancel_edit();
                self.text_input.focus();
            }
        }
    }

    /// Write `text` into the formula bar, remembering it as the view's own
    /// (`TRAME-UI-155`). The browser signal takes it at once, so the next
    /// frame copies the same text back in.
    pub(crate) fn show_formula_text(&mut self, text: String) {
        self.formula.shown.clone_from(&text);
        #[cfg(target_arch = "wasm32")]
        self.formula_text.set(text.clone());
        self.formula.text = text;
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
