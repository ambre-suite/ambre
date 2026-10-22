//! The format bar's state, published to the chrome
//! (`features/ambre-spec-prose-toolbar.md section 3`).
//!
//! The bar is a DOM element, and this view is what knows where the selection
//! is. It publishes a [`ToolbarState`] and the element renders it. The marks
//! the controls report arrive from the Logic thread with their selection
//! (`PROSE-UI-057`). Nothing here remembers a mark across a selection change
//! (`PROSE-UI-097`).

use ambre_commands::CommandId;

use crate::events::{ActiveMarks, MarkAction, ProseCommand, TriState};
use crate::layout::run::RunSpan;
use crate::ui::ProseUiState;
use crate::view::caret::Spot;

/// What the format bar shows, rebuilt on every frame from the selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarState {
    /// The top-centre of the selection's first line, on the canvas. `None`
    /// hides the bar (`PROSE-UI-053`).
    pub anchor: Option<(f32, f32)>,
    /// What each control reports. `None` until the Logic thread answered for
    /// this very selection.
    pub active: Option<ActiveMarks>,
}

impl ToolbarState {
    #[must_use]
    pub fn visible(&self) -> bool {
        self.anchor.is_some()
    }
}

/// The state one key of the bar holds in a set of marks.
#[must_use]
pub fn state_of(marks: &ActiveMarks, key: &str) -> TriState {
    match key {
        "b" => marks.bold,
        "i" => marks.italic,
        "u" => marks.underline,
        "s" => marks.strikethrough,
        "c" => marks.code,
        _ => TriState::Off,
    }
}

impl ProseUiState {
    /// The bar as it stands: shown over a non-empty selection alone
    /// (`PROSE-UI-053`, `PROSE-UI-100`). It sits at the selection's place on
    /// the canvas as it scrolls and zooms (`PROSE-UI-055`).
    #[must_use]
    pub fn toolbar(&self) -> ToolbarState {
        let Some(selection) = self.selection() else {
            return ToolbarState::default();
        };
        let active = self
            .marks
            .as_ref()
            .filter(|(read, _)| *read == selection)
            .map(|(_, marks)| marks.clone());
        ToolbarState {
            anchor: self.selection_top(selection),
            active,
        }
    }

    /// The read the bar still needs: the selection changed, or the layout
    /// did, since the last one asked. Raising the bar writes nothing and
    /// leaves the selection as it was (`PROSE-UI-099`).
    ///
    /// With no selection, what was read for the last one is forgotten, so
    /// the next one starts from nothing (`PROSE-UI-097`).
    pub fn marks_query(&mut self) -> Option<ProseCommand> {
        let Some(selection) = self.selection() else {
            self.marks = None;
            self.marks_asked = None;
            return None;
        };
        if self.marks_asked == Some((selection, self.layout_version)) {
            return None;
        }
        let ((a, a_off), (f, f_off)) = selection;
        Some(ProseCommand::ReadMarks {
            anchor: (a, a_off),
            focus: (f, f_off),
        })
    }

    /// The read [`marks_query`](Self::marks_query) answered reached the
    /// channel: it is not asked again for this selection and this layout.
    pub fn marks_sent(&mut self) {
        self.marks_asked = self.selection().map(|s| (s, self.layout_version));
    }

    /// Take the marks the Logic thread read, for the selection it names.
    pub(crate) fn take_marks(&mut self, anchor: Spot, focus: Spot, marks: &ActiveMarks) {
        self.marks = Some(((anchor, focus), marks.clone()));
    }

    /// Apply or remove one control's formatting over the whole selection
    /// (`PROSE-UI-102`).
    ///
    /// A control reporting on removes the mark. Off and mixed apply it
    /// (`PROSE-UI-103`), and so does a state not read yet. Only a command the
    /// plane declares leaves (`PROSE-UI-098`): one, whatever the number of
    /// blocks, so one undo takes the gesture back (`PROSE-UI-192`).
    #[must_use]
    pub fn format(&self, id: CommandId) -> Option<ProseCommand> {
        let key = crate::commands::mark_key(id)?;
        let (anchor, focus) = self.selection()?;
        let on = self
            .toolbar()
            .active
            .is_some_and(|m| state_of(&m, key) == TriState::On);
        Some(ProseCommand::FormatSelection {
            anchor,
            focus,
            key: key.to_owned(),
            action: if on {
                MarkAction::Remove
            } else {
                MarkAction::Apply
            },
        })
    }

    /// The top-centre of the selection's first line, on the canvas.
    fn selection_top(
        &self,
        ((start, start_off), (end, end_off)): (Spot, Spot),
    ) -> Option<(f32, f32)> {
        let run = self.run_of(start)?;
        let from = self.to_laid(start, start_off);
        let to = if start == end {
            self.to_laid(end, end_off)
        } else {
            run.len().as_end()
        };
        let first = run
            .range_boxes(RunSpan::new(from, to.max(from)))
            .into_iter()
            .next();
        let (x0, x1, y0) = match first {
            Some(part) => (part.x0 as f32, part.x1 as f32, part.y0 as f32),
            // The selection starts at the end of its block: the caret's box.
            None => {
                let caret = run.caret_box_at(from, crate::layout::run::Affinity::Downstream);
                (caret.x0 as f32, caret.x0 as f32, caret.y0 as f32)
            }
        };
        let (page, piece) = self.fragment_at(start, y0)?;
        Some(self.to_screen(start, page, piece, (x0 + x1) / 2.0, y0))
    }
}
