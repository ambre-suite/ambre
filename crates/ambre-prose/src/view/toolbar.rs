//! The format bar's state, published to the chrome
//! (`features/ambre-spec-prose-toolbar.md section 3`).
//!
//! The bar is a DOM element, and this view is what knows where the selection
//! is. It publishes a [`ToolbarState`] and the element renders it. The marks
//! the controls report arrive from the Logic thread with their selection
//! (`PROSE-UI-057`). Nothing here remembers a mark across a selection change
//! (`PROSE-UI-097`).

use ambre_core::index::Span;
use ambre_core::{Commands, Count, StaticKey};
use ambre_core::{Doc, Point, Px, View};

use crate::events::{ActiveMarks, MarkAction, ProseCommand, TriState, Words};
use crate::layout::run::RunChar;
use crate::mark::MarkRoot;
use crate::ui::ProseUiState;
use crate::view::caret::Spot;

/// What the format bar shows, rebuilt on every frame from the selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarState {
    /// The top-centre of the selection's first line, on the canvas. `None`
    /// hides the bar (`PROSE-UI-053`).
    pub anchor: Option<Point<Px<View>>>,
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

/// The field of the format bar a press opened: the address of a link, or
/// the first reply of a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarPrompt {
    Link,
    Comment,
}

impl BarPrompt {
    /// The field a command id opens, or `None` for a command that writes at
    /// once.
    #[must_use]
    pub fn of(id: StaticKey<Commands>) -> Option<Self> {
        if id == crate::commands::LINK {
            Some(Self::Link)
        } else if id == crate::commands::COMMENT {
            Some(Self::Comment)
        } else {
            None
        }
    }
}

/// The state one key of the bar holds in a set of marks.
#[must_use]
pub fn state_of(marks: &ActiveMarks, key: MarkRoot) -> TriState {
    match key {
        MarkRoot::Bold => marks.bold,
        MarkRoot::Italic => marks.italic,
        MarkRoot::Underline => marks.underline,
        MarkRoot::Strike => marks.strikethrough,
        MarkRoot::Code => marks.code,
        MarkRoot::Superscript => marks.superscript,
        MarkRoot::Subscript => marks.subscript,
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

    /// The words of the selected text, counted as the word count counts
    /// them, which the subtitle of the context panel shows (`CHROME-241`).
    /// `None` with no selection, or for one whose ends the flow of the
    /// layout in force does not hold, such as a table cell.
    #[must_use]
    pub fn selected_words(&self) -> Option<Count<Words>> {
        let ((first, from), (last, to)) = self.selection()?;
        let start = self.flow_index(&crate::events::tree_id(first))?;
        let end = self.flow_index(&crate::events::tree_id(last))?;
        let words = start
            .through(end)
            .into_iter()
            .filter_map(|at| self.shaped.block(at))
            .map(|layout| {
                let block = crate::events::block_id(layout.block_id);
                let text = self.shown_text(block).unwrap_or_default();
                let skip = if block == first { from.to_usize() } else { 0 };
                let chars = text.chars().skip(skip);
                let selected: String = if block == last {
                    chars.take(to.to_usize().saturating_sub(skip)).collect()
                } else {
                    chars.collect()
                };
                crate::count::words_in(&selected)
            })
            .fold(Count::<Words>::ZERO, Count::<Words>::plus);
        Some(words)
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
    /// blocks, so one undo takes the gesture back (`PROSE-UI-192`). An insert
    /// of the creation bar resolves at the caret, with no selection.
    #[must_use]
    pub fn format(&self, id: StaticKey<Commands>) -> Option<ProseCommand> {
        if let Some(item) = crate::view::slash::inserted(id) {
            // A command of the creation bar lands after the block the caret
            // sits in, as the slash menu's item does (`CHROME-180`).
            return Some(item.resolve(self.cursor.as_ref()?.block_id));
        }
        let (anchor, focus) = self.selection()?;
        if id == crate::commands::CLEAR_FORMATTING {
            return Some(ProseCommand::ClearSelectionMarks { anchor, focus });
        }
        let key = crate::commands::mark_key(id)?;
        let on = self
            .toolbar()
            .active
            .is_some_and(|m| state_of(&m, key) == TriState::On);
        Some(ProseCommand::FormatSelection {
            anchor,
            focus,
            key: key.key(),
            action: if on {
                MarkAction::Remove
            } else {
                MarkAction::Apply
            },
        })
    }

    /// The selection the context panel shows, which a pick copies into its
    /// write (`PROSE-UI-281`). `None` with no text selected.
    #[must_use]
    pub fn picked_selection(&self) -> Option<crate::view::panel::PickedSelection> {
        let (anchor, focus) = self.selection()?;
        Some(crate::view::panel::PickedSelection { anchor, focus })
    }

    /// The one command a control of the context panel asks for, over the
    /// selection the write captured at its pick (`PROSE-UI-098`,
    /// `PROSE-UI-281`). Every write over a selection is one command, so one
    /// undo takes it back (`CHROME-173`). A default font needs no selection.
    ///
    /// The view reads none of its own state, so a caret that moved since
    /// the pick changes nothing.
    ///
    /// `None` when the pick held no selection for a write that needs one,
    /// or when the registry has no spelling for the value.
    #[must_use]
    pub fn text_write(&self, write: crate::view::panel::TextWrite) -> Option<ProseCommand> {
        use crate::view::panel::{PickedSelection, TextValue};
        let crate::view::panel::TextWrite { value, selection } = write;
        if let TextValue::DefaultFont(family) = value {
            return Some(ProseCommand::SetDefaultFont { family });
        }
        let PickedSelection { anchor, focus } = selection?;
        let mark = |root: MarkRoot, raw: Option<String>| {
            let value = match raw {
                Some(raw) => Some(crate::mark::validate_value(&root.key(), Some(&raw)).ok()?),
                None => None,
            };
            Some(ProseCommand::MarkSelection {
                anchor,
                focus,
                key: root.key(),
                value,
            })
        };
        let colour = crate::mark::canonical_color;
        match value {
            TextValue::Font(family) => {
                mark(MarkRoot::FontFamily, family.map(|f| f.label().to_owned()))
            }
            TextValue::Size(size) => {
                mark(MarkRoot::FontSize, size.map(crate::mark::canonical_number))
            }
            TextValue::Colour(rgb) => mark(MarkRoot::Ink, rgb.map(colour)),
            TextValue::Highlight(rgb) => mark(MarkRoot::Highlight, rgb.map(colour)),
            // An empty address takes the link off.
            TextValue::Link(url) => mark(
                MarkRoot::Link,
                url.map(|u| u.as_str().trim().to_owned())
                    .filter(|u| !u.is_empty()),
            ),
            TextValue::Paragraph(style) => Some(ProseCommand::SetSelectionParagraph {
                anchor,
                focus,
                style,
            }),
            TextValue::Kind(kind) => Some(ProseCommand::SetSelectionKind {
                anchor,
                focus,
                kind,
            }),
            TextValue::Indent(step) => Some(ProseCommand::IndentSelection {
                anchor,
                focus,
                step,
            }),
            TextValue::DefaultFont(..) => None,
        }
    }

    /// The top-centre of the selection's first line, on the canvas.
    fn selection_top(
        &self,
        ((start, start_off), (end, end_off)): (Spot, Spot),
    ) -> Option<Point<Px<View>>> {
        let run = self.run_of(start)?;
        let from = self.to_laid(start, start_off);
        let to = if start == end {
            self.to_laid(end, end_off)
        } else {
            run.len().as_end()
        };
        let first = run
            .range_boxes(Span::<RunChar>::new(from, to.max(from)))
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
        let top = Px::<Doc>::new(y0);
        let (page, piece) = self.fragment_at(start, top)?;
        let centre = Point::new(Px::<Doc>::new((x0 + x1) / 2.0), top);
        Some(self.to_screen(start, page, piece, centre))
    }
}
