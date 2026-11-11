// ambre-trame/src/logic/history.rs
// Trame's side of the one undo history (`TRAME-288`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// The history itself is `ambre_crdt::history::UndoHistory`: each local commit
// records its own inverse, and no undo manager runs. This file seals a step
// per command and runs Trame's tail on what a step wrote.

use ambre_core::HistoryStep;
use ambre_core::crdt::CommitOrigin;

use super::remote::LocalNote;
use super::*;

/// The path an undo takes, for the test that holds the history against the
/// undo manager (`TRAME-286`).
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryPath {
    /// Each edit records its own inverse.
    Recorded,
    /// The reference model: the undo manager holds every edit.
    Managed,
}

impl TrameState {
    /// Whether the test runs the reference model.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn managed(&self) -> bool {
        self.history_path == HistoryPath::Managed
    }

    /// How many steps an undo can take back, for the tests that count them.
    #[cfg(test)]
    pub(super) fn undo_depth(&self) -> usize {
        self.history.depth().to_usize()
    }

    /// Whether an undo has a step to take back.
    pub(super) fn can_undo(&self) -> ambre_core::Availability {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return ambre_core::Availability::from_bool(self.oracle.can_undo());
        }
        self.history.can_undo()
    }

    /// Whether a redo has a step to do again.
    pub(super) fn can_redo(&self) -> ambre_core::Availability {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return ambre_core::Availability::from_bool(self.oracle.can_redo());
        }
        self.history.can_redo()
    }

    /// End the step of the last command and open one for the command that
    /// starts: every commit it makes joins one step (`TRAME-017`).
    pub(super) fn seal_history(&mut self) {
        self.history.seal();
        self.drop_parked_past_the_history();
        #[cfg(all(test, not(target_arch = "wasm32")))]
        {
            self.oracle.group_end();
            let _ = self.oracle.group_start();
        }
    }

    /// Forget every step, for a new identity: a step names this peer's work.
    pub(super) fn clear_history(&mut self) {
        self.history.clear();
        self.engine.bytecode.clear_parked();
    }

    /// Drop the formulas the engine parked once no undo step is left: only
    /// an undo writes a removed line back, so none can be reused. A step the
    /// history ended itself, over a container, is gone the same way
    /// (`TRAME-290`, `TRAME-300`).
    fn drop_parked_past_the_history(&mut self) {
        if self.engine.bytecode.has_parked()
            && self.history.can_undo() == ambre_core::Availability::Disabled
        {
            self.engine.bytecode.clear_parked();
        }
    }

    /// Take one step of the history back or forward, then run the tail on
    /// what it wrote (`after_history`). An empty side reports nothing: the
    /// availability the chrome holds is still true (`UNDO-009`).
    pub(super) fn step_history(&mut self, step: HistoryStep) -> TrameOutcome {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return self.apply_history(step);
        }
        if self.history.can(step) == ambre_core::Availability::Disabled {
            return TrameOutcome::display_only(Vec::new());
        }
        let order = self.sheet_order_ids();
        drop(self.take_imported());
        self.engine.changed.clear();
        self.note_local(LocalNote::Noted);
        // The side holds a step, checked above, so the write moves it.
        let _ = self.history.write(&self.doc, step);
        self.doc
            .set_next_commit_origin(CommitOrigin::Recorded.as_str());
        self.doc.commit();
        self.note_local(LocalNote::Ignored);
        let written = self.take_imported();
        let outcome = match step {
            HistoryStep::Undo => self.after_undo(&written, &order),
            HistoryStep::Redo => self.after_redo(&written, &order),
        };
        self.refresh_undo_state(outcome)
    }

    /// Report the availability the history holds once the step moved, in
    /// place of the one the tail read before.
    fn refresh_undo_state(&self, mut outcome: TrameOutcome) -> TrameOutcome {
        if let Some(last) = outcome.display.last_mut()
            && matches!(last, TrameEvent::UndoState { .. })
        {
            *last = self.undo_state_event();
        }
        outcome
    }
}
