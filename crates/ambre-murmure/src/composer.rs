//! The composer of a channel (the Murmure composer spec).
//!
//! A band at the foot of the channel's region, where the member writes. It
//! drives the inline editor every application shares, fed by the page's input
//! pipe (`MURMURE-430`). Each channel keeps its own draft while the member
//! moves between channels (`MURMURE-195`). The band draws the field alone: no
//! toolbar, no button (`MURMURE-196`).
//!
//! The editor applies its edits at paint, where the font context lives. So
//! Enter only asks to send, and the next paint reads the draft once every key
//! typed before it has landed.
//!
//! A strip above the field names the members typing in the channel. It is
//! always there, so the list never moves when someone starts or stops.
//!
//! Each draft keeps its own history, read at paint like its edits: `Ctrl+Z`
//! and `Ctrl+Shift+Z` walk it (`MURMURE-433`). A send drops it, so no undo
//! reaches a message sent (`MURMURE-434`).

use std::collections::HashMap;

use ambre_bus::ChannelId;
use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
use ambre_canvas::{EditIntent, Palette, TextCache, TextEditor, TextEditorStyle};
use ambre_draw::{Color, DrawSurface, FontContext, Point, Rect};

use crate::paint::{draw, shape};

/// The words an empty composer shows.
pub const COMPOSER_HINT: &str = "Write a message";

/// The height of the field when it holds no line yet.
const MIN_LINE_PX: f32 = FONT_BODY + SPACE_4;

/// The family label of the composer's text, the one of the message bodies.
const FAMILY: &str = "Sans";

/// The height of the strip that names who types, above the field.
const TYPING_PX: f32 = FONT_UI + SPACE_4;

/// The line that names the members typing, `None` when nobody types. Past
/// two names it counts no one, so the line stays one short line.
pub fn typing_line(names: &[String]) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(format!("{one} is typing")),
        [one, two] => Some(format!("{one} and {two} are typing")),
        _ => Some("Several people are typing".to_owned()),
    }
}

/// The style of a draft: the face and the size of a message body.
fn body_style(color: Color) -> TextEditorStyle {
    TextEditorStyle {
        weight: 400,
        italic: false,
        family: FAMILY.to_owned(),
        base_size: FONT_BODY,
        color,
        underline: false,
        strikethrough: false,
    }
}

/// Which way a step of the history moved the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Grow,
    Shrink,
}

/// The history of one draft: the texts an undo goes back to, and those a redo
/// goes forward to. A step gathers the edits of one way. A caret move, a line
/// break, a paste or a space seals it, so one undo takes back a word.
#[derive(Debug, Default)]
struct History {
    undo: Vec<String>,
    redo: Vec<String>,
    /// The draft as the last paint left it.
    last: String,
    /// The way of the step still open, `None` once sealed.
    open: Option<Change>,
    /// Seal the step once the next paint records the edit that asked it.
    seal_after: bool,
    /// An edit waits for the next paint.
    pending: bool,
    /// The steps asked since the last paint, `true` for a redo.
    asked: Vec<bool>,
}

impl History {
    /// An edit landed, which the next paint records.
    fn edit(&mut self) {
        self.pending = true;
    }

    /// The next edit starts a step of its own. An edit not yet painted still
    /// joins the step it lands in, and closes it.
    fn seal(&mut self) {
        if self.pending {
            self.seal_after = true;
        } else {
            self.open = None;
        }
    }

    /// The edit about to land is a step of its own: a paste, a cut, a line
    /// break.
    fn apart(&mut self) {
        self.open = None;
        self.seal_after = true;
        self.pending = true;
    }

    /// The draft reads `text` after a paint. A change opens a step, or joins
    /// the open one when it goes the same way. Any change drops the redo.
    fn record(&mut self, text: &str) {
        if text != self.last {
            let change = if text.len() > self.last.len() {
                Change::Grow
            } else {
                Change::Shrink
            };
            let before = std::mem::replace(&mut self.last, text.to_owned());
            if self.open != Some(change) {
                self.undo.push(before);
            }
            self.redo.clear();
            self.open = Some(change);
        }
        if std::mem::take(&mut self.seal_after) {
            self.open = None;
        }
        self.pending = false;
    }

    /// Walk one step back, or forward for a `redo`, and answer the text the
    /// draft takes. `None` with nothing to walk.
    fn step(&mut self, redo: bool) -> Option<String> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let text = from.pop()?;
        to.push(std::mem::replace(&mut self.last, text.clone()));
        self.open = None;
        Some(text)
    }
}

/// The composer of the channel region, and the draft of every channel.
#[derive(Default)]
pub struct Composer {
    /// The draft of each channel the member wrote in. A sent draft leaves it.
    drafts: HashMap<ChannelId, TextEditor>,
    /// The history of each draft, which leaves with it.
    histories: HashMap<ChannelId, History>,
    /// The keys reach the composer, not the list.
    focused: bool,
    /// An input method composes: its keys are its own.
    composing: bool,
    /// Enter asked to send the draft of this channel.
    send: Option<ChannelId>,
    /// A key changed a draft since the view last asked.
    typed: bool,
    /// A press in the field holds a drag.
    dragging: bool,
    /// The height of the band at the last paint, `0` with no channel.
    band: f32,
    /// Where the text of the field started at the last paint.
    origin: (f32, f32),
}

impl Composer {
    /// The height of the band at the last paint, `0` when no channel showed.
    pub fn band(&self) -> f32 {
        self.band
    }

    /// Whether the keys reach the composer.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The keys go back to the list.
    pub fn blur(&mut self) {
        self.focused = false;
        self.dragging = false;
    }

    /// The draft of `channel` as its editor holds it, `None` when the member
    /// wrote nothing there. A key typed since the last paint is not in it.
    pub fn draft(&self, channel: ChannelId) -> Option<String> {
        self.drafts.get(&channel).map(TextEditor::text)
    }

    /// The text selected in the draft of `channel`.
    pub fn selected(&self, channel: ChannelId) -> Option<String> {
        self.drafts.get(&channel)?.selected_text()
    }

    /// Whether a key changed a draft since the last call. A move of the
    /// caret or a selection changes none.
    pub fn take_typed(&mut self) -> bool {
        std::mem::take(&mut self.typed)
    }

    /// The editor of the draft of `channel`, opened empty on first use.
    fn editor(&mut self, channel: ChannelId) -> &mut TextEditor {
        self.focused = true;
        self.drafts
            .entry(channel)
            .or_insert_with(|| TextEditor::new("", body_style(Color::BLACK)))
    }

    /// The history of the draft of `channel`, opened empty on first use.
    fn history(&mut self, channel: ChannelId) -> &mut History {
        self.histories.entry(channel).or_default()
    }

    /// Text the pipe committed: a typed key, a paste, a word an input
    /// method chose. A paste is a step of its own, and a space ends a word's.
    pub fn text(&mut self, channel: ChannelId, text: &str) {
        self.typed |= !text.is_empty();
        let history = self.history(channel);
        if text.chars().count() > 1 {
            history.apart();
        }
        history.edit();
        history.seal_after |= text.ends_with(char::is_whitespace);
        self.editor(channel).insert(text);
    }

    /// The preview of an input method, or its final word once `end`.
    pub fn compose(&mut self, channel: ChannelId, text: &str, end: bool) {
        self.composing = !end && !text.is_empty();
        self.typed |= !text.is_empty();
        self.history(channel).edit();
        let editor = self.editor(channel);
        if end {
            editor.commit_compose(text);
        } else {
            editor.set_compose(text);
        }
    }

    /// Cut the selection of the draft of `channel`, and answer its text.
    pub fn cut(&mut self, channel: ChannelId) -> Option<String> {
        let editor = self.drafts.get_mut(&channel)?;
        let text = editor.selected_text()?;
        editor.delete_selection();
        self.typed = true;
        self.history(channel).apart();
        Some(text)
    }

    /// A key while the composer holds the focus. Enter sends and Shift+Enter
    /// breaks the line (`MURMURE-199`, `MURMURE-200`). `Ctrl+Z` undoes, and
    /// `Ctrl+Shift+Z` or `Ctrl+Y` redoes (`MURMURE-433`). Escape gives the keys
    /// back to the list. Answers whether the composer took the key.
    pub fn key(
        &mut self,
        channel: ChannelId,
        key: &str,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        if self.composing {
            return false;
        }
        let word = ctrl || alt;
        let intent = match key {
            "Enter" if shift => EditIntent::Newline,
            "Enter" => {
                self.send = Some(channel);
                return true;
            }
            "Escape" => {
                self.blur();
                return true;
            }
            "z" | "Z" | "y" | "Y" if ctrl => {
                let redo = shift || key.eq_ignore_ascii_case("y");
                self.typed = true;
                self.editor(channel);
                self.history(channel).asked.push(redo);
                return true;
            }
            "a" | "A" if ctrl => EditIntent::SelectAll,
            "Backspace" => EditIntent::Backspace,
            "Delete" => EditIntent::Delete,
            "ArrowLeft" => EditIntent::Left {
                select: shift,
                word,
            },
            "ArrowRight" => EditIntent::Right {
                select: shift,
                word,
            },
            "ArrowUp" => EditIntent::Up { select: shift },
            "ArrowDown" => EditIntent::Down { select: shift },
            "Home" => EditIntent::LineStart { select: shift },
            "End" => EditIntent::LineEnd { select: shift },
            _ => return false,
        };
        let edits = matches!(
            intent,
            EditIntent::Newline | EditIntent::Backspace | EditIntent::Delete
        );
        self.typed |= edits;
        // A caret move ends the step, and a line break is one of its own.
        let history = self.history(channel);
        match intent {
            EditIntent::Newline => history.apart(),
            EditIntent::Backspace | EditIntent::Delete => history.edit(),
            _ => history.seal(),
        }
        self.editor(channel).push(intent);
        true
    }

    /// A press at `(x, y)` in the band: the caret goes there, and a drag
    /// from it selects.
    pub fn press(&mut self, channel: ChannelId, x: f32, y: f32) {
        let (dx, dy) = self.origin;
        self.history(channel).seal();
        self.editor(channel).point_down(x - dx, y - dy);
        self.dragging = true;
    }

    /// A pointer moved: a drag from the field extends the selection.
    /// Answers whether the composer held a drag.
    pub fn drag_to(&mut self, channel: ChannelId, x: f32, y: f32) -> bool {
        if !self.dragging {
            return false;
        }
        let (dx, dy) = self.origin;
        self.editor(channel).point_drag(x - dx, y - dy);
        true
    }

    /// The button lifted. Answers whether the composer held a drag.
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.dragging)
    }

    /// A double click in the field selects the word under it.
    pub fn word_at(&mut self, channel: ChannelId, x: f32, y: f32) {
        let (dx, dy) = self.origin;
        self.history(channel).seal();
        self.editor(channel).word_at(x - dx, y - dy);
    }

    /// Apply the edits since the last paint, and size the band for the draft
    /// of `channel` in a region of `size`. Answers the draft Enter sent, and
    /// its channel. An empty draft sends nothing (`MURMURE-201`). Only the
    /// sent draft leaves, so another channel keeps its own (`MURMURE-202`).
    pub fn flush(
        &mut self,
        font_cx: &mut FontContext,
        channel: Option<ChannelId>,
        size: (f32, f32),
        palette: &Palette,
    ) -> Option<(ChannelId, String)> {
        let wrap = (size.0 - 2.0 * SPACE_8).max(1.0);
        let style = body_style(palette.cell_text);
        let mut sent = None;
        if let Some(asked) = self.send.take()
            && let Some(editor) = self.drafts.get_mut(&asked)
        {
            editor.set_style(style.clone());
            editor.flush(font_cx, wrap, 1.0);
            let text = editor.text();
            if !text.trim().is_empty() {
                self.drafts.remove(&asked);
                // The history leaves with the draft: no undo reaches a
                // message sent (`MURMURE-434`).
                self.histories.remove(&asked);
                sent = Some((asked, text));
            }
        }
        let Some(channel) = channel else {
            self.band = 0.0;
            return sent;
        };
        let tall = match self.drafts.get_mut(&channel) {
            Some(editor) => {
                editor.set_style(style);
                editor.flush(font_cx, wrap, 1.0);
                // Record the edits that landed, then walk the steps asked
                // after them. A step puts the caret at the end of the draft.
                let history = self.histories.entry(channel).or_default();
                history.record(&editor.text());
                let mut walked = false;
                for redo in std::mem::take(&mut history.asked) {
                    if let Some(text) = history.step(redo) {
                        editor.set_text(&text);
                        walked = true;
                    }
                }
                if walked {
                    editor.flush(font_cx, wrap, 1.0);
                }
                editor.layout_height().unwrap_or(0.0)
            }
            None => 0.0,
        };
        let most = (size.1 / 2.0 - 2.0 * SPACE_4).max(MIN_LINE_PX);
        self.band = TYPING_PX + tall.clamp(MIN_LINE_PX, most) + 2.0 * SPACE_4;
        sent
    }

    /// Paint the band of `channel` at the foot of a region of `size`, over
    /// any row that reached it. The line of who types sits above the field.
    /// Answers the caret, its top-left and its height, while the composer
    /// holds the focus.
    #[allow(clippy::too_many_arguments)]
    pub fn paint<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        channel: Option<ChannelId>,
        typing: Option<&str>,
        size: (f32, f32),
        palette: &Palette,
    ) -> Option<(f32, f32, f32)> {
        let channel = channel?;
        let (width, height) = size;
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
            Rect::new(x0.into(), y0.into(), x1.into(), y1.into())
        };
        let strip = height - self.band;
        surface.fill_rect(rect(0.0, strip, width, height), palette.content_bg);
        if let Some(line) = typing {
            let shaped = shape(font_cx, cache, line, FONT_UI, width - 2.0 * SPACE_8, false);
            draw(
                surface,
                shaped,
                SPACE_8,
                strip + SPACE_2,
                palette.header_text,
            );
        }
        let top = strip + TYPING_PX;
        let field = rect(SPACE_4, top + SPACE_2, width - SPACE_4, height - SPACE_2);
        surface.fill_rect(field, palette.header_bg);
        let edge = if self.focused {
            palette.selection
        } else {
            palette.cell_border
        };
        surface.stroke_rect(field, edge, 1.0);
        let inner = (top + SPACE_4, height - SPACE_4);
        self.origin = (SPACE_8, inner.0);
        let editor = self.drafts.get_mut(&channel);
        let empty = editor.as_ref().is_none_or(|e| e.text().is_empty()) && !self.composing;
        if empty {
            let hint = shape(font_cx, cache, COMPOSER_HINT, FONT_BODY, width, false);
            draw(surface, hint, SPACE_8, inner.0, palette.header_text);
        }
        let editor = editor?;
        // A draft taller than the field shows its last lines.
        let tall = editor.layout_height().unwrap_or(0.0);
        let shift = (inner.1 - inner.0 - tall).min(0.0);
        self.origin = (SPACE_8, inner.0 + shift);
        let origin = Point::new(SPACE_8.into(), (inner.0 + shift).into());
        surface.push_clip(rect(SPACE_4, inner.0, width - SPACE_4, inner.1));
        editor.render(surface, origin, palette.selection, self.focused);
        surface.pop_clip();
        let caret = editor.caret_local().filter(|_| self.focused)?;
        Some((
            (origin.x + caret.x0) as f32,
            (origin.y + caret.y0) as f32,
            caret.height() as f32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One or two names read in full, more read as a count of no one, and
    /// nobody typing shows no line (`MURMURE-517`).
    #[test]
    fn the_typing_line_names_one_or_two_and_counts_no_more() {
        let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(typing_line(&[]), None);
        assert_eq!(
            typing_line(&names(&["Ada"])).as_deref(),
            Some("Ada is typing")
        );
        let two = typing_line(&names(&["Ada", "Bob"]));
        assert_eq!(two.as_deref(), Some("Ada and Bob are typing"));
        let three = typing_line(&names(&["Ada", "Bob", "Cy"]));
        assert_eq!(three.as_deref(), Some("Several people are typing"));
    }

    /// A seal with no edit waiting starts the next step, and does not close
    /// it on its first key. A seal with an edit waiting closes the step that
    /// edit joins (`MURMURE-433`).
    #[test]
    fn a_seal_closes_the_waiting_edit_and_never_the_next_one() {
        let mut history = History::default();
        history.seal();
        for text in ["h", "hi"] {
            history.edit();
            history.record(text);
        }
        history.edit();
        history.seal();
        history.record("hi!");
        history.edit();
        history.record("hi!?");
        assert_eq!(history.step(false).as_deref(), Some("hi!"));
        assert_eq!(history.step(false).as_deref(), Some(""));
        assert_eq!(history.step(false), None, "nothing left to walk");
        assert_eq!(history.step(true).as_deref(), Some("hi!"));
    }
}
