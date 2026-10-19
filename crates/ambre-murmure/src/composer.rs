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
//!
//! An edit loads a message of the member into the composer, and puts the
//! draft and its history aside until the edit ends (`MURMURE-203`).
//!
//! The thread's region has a composer of its own, whose drafts are kept per
//! thread (`MURMURE-525`). Enter there sends a reply to the root
//! (`MURMURE-523`). A box under its field sends the reply to the channel as
//! well, and unchecks once sent (`MURMURE-531`).

use std::collections::{HashMap, HashSet};

use ambre_bus::{
    ChannelId, LogicCommand, Mention, MessageAt, MessageDraft, MessageId, MurmureCommand, Timestamp,
};
use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
use ambre_canvas::{EditIntent, Palette, TextCache, TextEditor, TextEditorStyle};
use ambre_draw::{Color, DrawSurface, FontContext, Point, Rect};

use crate::commands::PendingEdit;
use crate::paint::{draw, shape};

/// The strip above the field while the composer holds an edit.
pub const EDITING_LINE: &str = "Editing a message. Escape cancels.";

/// The words an empty composer shows.
pub const COMPOSER_HINT: &str = "Write a message";

/// The height of the field when it holds no line yet.
const MIN_LINE_PX: f32 = FONT_BODY + SPACE_4;

/// The family label of the composer's text, the one of the message bodies.
const FAMILY: &str = "Sans";

/// The height of the strip that names who types, above the field.
const TYPING_PX: f32 = FONT_UI + SPACE_4;

/// The words beside the box under a thread's field (`MURMURE-531`).
pub const ALSO_IN_CHANNEL: &str = "Also send to the channel";

/// The height of the row of that box, under the field.
pub(crate) const ALSO_PX: f32 = FONT_UI + SPACE_8;

/// The side of the box.
const BOX_PX: f32 = FONT_UI;

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

/// Whose draft a composer holds: a channel's, or the thread's under a root
/// of the channel. A thread's draft is not its channel's (`MURMURE-525`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DraftOf {
    Channel(ChannelId),
    Thread(ChannelId, MessageId),
}

impl DraftOf {
    /// The channel the draft writes in.
    pub fn channel(self) -> ChannelId {
        match self {
            Self::Channel(channel) | Self::Thread(channel, _) => channel,
        }
    }

    /// The draft whose composer loads `edit`: its thread's for a reply.
    pub fn of_edit(edit: &PendingEdit) -> Self {
        match edit.at {
            MessageAt::Segment(_) => Self::Channel(edit.channel),
            MessageAt::Thread(root) => Self::Thread(edit.channel, root),
        }
    }
}

impl From<ChannelId> for DraftOf {
    fn from(channel: ChannelId) -> Self {
        Self::Channel(channel)
    }
}

/// A message the composer of a channel holds for an edit.
struct Editing {
    at: MessageAt,
    message: MessageId,
    /// The text the message read when the edit began.
    original: String,
    /// Who that text names.
    mentions: Vec<Mention>,
    /// The draft and its history the edit put aside, `None` with no draft.
    kept: Option<(TextEditor, History)>,
}

/// What the composer hands the view to send, once a paint read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// A new message in the channel (`MURMURE-199`), or a reply to the root
    /// of the thread (`MURMURE-523`). A checked box sends the reply to the
    /// channel as well (`MURMURE-531`).
    Message {
        to: DraftOf,
        draft: MessageDraft,
        also_in_channel: bool,
    },
    /// An edit of a message of the member (`MURMURE-203`).
    Edit {
        channel: ChannelId,
        at: MessageAt,
        message: MessageId,
        draft: MessageDraft,
    },
}

impl Sent {
    /// The command to the Logic thread, stamped `now` by the view.
    pub fn command(self, now: Timestamp) -> LogicCommand {
        match self {
            Self::Message {
                to,
                draft,
                also_in_channel,
            } => LogicCommand::Channel {
                channel_id: to.channel(),
                command: match to {
                    DraftOf::Channel(_) => MurmureCommand::SendMessage {
                        draft,
                        now,
                        confirmed: false,
                    },
                    DraftOf::Thread(_, root) => MurmureCommand::Reply {
                        root,
                        draft,
                        also_in_channel,
                        now,
                        confirmed: false,
                    },
                },
            },
            Self::Edit {
                channel,
                at,
                message,
                draft,
            } => LogicCommand::Channel {
                channel_id: channel,
                command: MurmureCommand::EditMessage {
                    at,
                    message,
                    draft,
                    now,
                },
            },
        }
    }
}

/// The mentions of `old` that an edit to `new` leaves whole. The edit
/// changed the chars between the common head and the common tail. A mention
/// before the change keeps its range, one after it moves with the tail, and
/// one the change touches goes.
fn kept_mentions(old: &str, new: &str, mentions: &[Mention]) -> Vec<Mention> {
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    mentions
        .iter()
        .filter_map(|mention| {
            let range = &mention.range;
            if range.end <= head {
                Some(mention.clone())
            } else if range.start >= old_end {
                let start = range.start - old_end + new_end;
                Some(Mention {
                    target: mention.target,
                    range: start..start + range.len(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// The composer of a region, and the draft of every channel or thread it
/// showed.
#[derive(Default)]
pub struct Composer {
    /// The draft of each channel or thread the member wrote in. A sent draft
    /// leaves it.
    drafts: HashMap<DraftOf, TextEditor>,
    /// The history of each draft, which leaves with it.
    histories: HashMap<DraftOf, History>,
    /// The edit each composer holds, in place of its draft.
    editing: HashMap<DraftOf, Editing>,
    /// The keys reach the composer, not the list.
    focused: bool,
    /// An input method composes: its keys are its own.
    composing: bool,
    /// Enter asked to send this draft.
    send: Option<DraftOf>,
    /// A key changed a draft since the view last asked.
    typed: bool,
    /// A press in the field holds a drag.
    dragging: bool,
    /// The height of the band at the last paint, `0` with no channel.
    band: f32,
    /// Where the text of the field started at the last paint.
    origin: (f32, f32),
    /// The threads whose box asks for the reply in the channel too.
    also: HashSet<DraftOf>,
    /// The row of that box at the last paint, as its top and bottom.
    also_row: Option<(f32, f32)>,
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

    /// The draft `of` a channel or a thread as its editor holds it, `None`
    /// when the member wrote nothing there. A key typed since the last paint
    /// is not in it.
    pub fn draft(&self, of: impl Into<DraftOf>) -> Option<String> {
        self.drafts.get(&of.into()).map(TextEditor::text)
    }

    /// Whether the composer `of` a channel or a thread holds nothing: no
    /// edit, no text, and no key or undo waiting for the next paint.
    pub fn blank(&self, of: DraftOf) -> bool {
        let idle = self
            .drafts
            .get(&of)
            .is_none_or(|editor| editor.text().is_empty() && !editor.queued());
        let asked = self
            .histories
            .get(&of)
            .is_some_and(|history| !history.asked.is_empty());
        idle && !asked && !self.composing && !self.editing(of)
    }

    /// The text selected in the draft `of` a channel or a thread.
    pub fn selected(&self, of: DraftOf) -> Option<String> {
        self.drafts.get(&of)?.selected_text()
    }

    /// Whether the box of the thread `of` asks for the reply in the channel
    /// too (`MURMURE-531`).
    pub fn also_in_channel(&self, of: DraftOf) -> bool {
        self.also.contains(&of)
    }

    /// Check the box of the thread `of`, or uncheck it. A channel's draft
    /// has no box.
    pub fn toggle_also(&mut self, of: DraftOf) {
        if matches!(of, DraftOf::Thread(..)) && !self.also.remove(&of) {
            self.also.insert(of);
        }
    }

    /// Whether the height `y` falls on the row of the box at the last paint.
    pub fn on_also(&self, y: f32) -> bool {
        self.also_row
            .is_some_and(|(top, bottom)| (top..bottom).contains(&y))
    }

    /// Whether a key changed a draft since the last call. A move of the
    /// caret or a selection changes none.
    pub fn take_typed(&mut self) -> bool {
        std::mem::take(&mut self.typed)
    }

    /// The editor of the draft `of`, opened empty on first use.
    fn editor(&mut self, of: DraftOf) -> &mut TextEditor {
        self.focused = true;
        self.drafts
            .entry(of)
            .or_insert_with(|| TextEditor::new("", body_style(Color::BLACK)))
    }

    /// The history of the draft `of`, opened empty on first use.
    fn history(&mut self, of: DraftOf) -> &mut History {
        self.histories.entry(of).or_default()
    }

    /// Load the message of `edit` into the composer of its channel, or of
    /// its thread for a reply, with the caret at its end. The draft there and
    /// its history wait for the edit to end. A second edit replaces the first
    /// and keeps the same draft.
    pub fn start_edit(&mut self, edit: PendingEdit) {
        let of = DraftOf::of_edit(&edit);
        let kept = match self.editing.remove(&of) {
            Some(open) => open.kept,
            None => self
                .drafts
                .remove(&of)
                .map(|editor| (editor, self.histories.remove(&of).unwrap_or_default())),
        };
        let editor = TextEditor::new(&edit.text, body_style(Color::BLACK));
        self.drafts.insert(of, editor);
        let history = History {
            last: edit.text.clone(),
            ..History::default()
        };
        self.histories.insert(of, history);
        self.editing.insert(
            of,
            Editing {
                at: edit.at,
                message: edit.message,
                original: edit.text,
                mentions: edit.mentions,
                kept,
            },
        );
        self.focused = true;
    }

    /// Whether the composer `of` a channel or a thread holds an edit.
    pub fn editing(&self, of: impl Into<DraftOf>) -> bool {
        self.editing.contains_key(&of.into())
    }

    /// End the edit `of`: the draft put aside comes back with its history,
    /// and the edited text goes (`MURMURE-204`).
    fn end_edit(&mut self, of: DraftOf) {
        let Some(open) = self.editing.remove(&of) else {
            return;
        };
        self.drafts.remove(&of);
        self.histories.remove(&of);
        if let Some((editor, history)) = open.kept {
            self.drafts.insert(of, editor);
            self.histories.insert(of, history);
        }
    }

    /// Text the pipe committed: a typed key, a paste, a word an input
    /// method chose. A paste is a step of its own, and a space ends a word's.
    pub fn text(&mut self, of: DraftOf, text: &str) {
        self.typed |= !text.is_empty();
        let history = self.history(of);
        if text.chars().count() > 1 {
            history.apart();
        }
        history.edit();
        history.seal_after |= text.ends_with(char::is_whitespace);
        self.editor(of).insert(text);
    }

    /// The preview of an input method, or its final word once `end`.
    pub fn compose(&mut self, of: DraftOf, text: &str, end: bool) {
        self.composing = !end && !text.is_empty();
        self.typed |= !text.is_empty();
        self.history(of).edit();
        let editor = self.editor(of);
        if end {
            editor.commit_compose(text);
        } else {
            editor.set_compose(text);
        }
    }

    /// Cut the selection of the draft `of`, and answer its text.
    pub fn cut(&mut self, of: DraftOf) -> Option<String> {
        let editor = self.drafts.get_mut(&of)?;
        let text = editor.selected_text()?;
        editor.delete_selection();
        self.typed = true;
        self.history(of).apart();
        Some(text)
    }

    /// A key while the composer holds the focus. Enter sends and Shift+Enter
    /// breaks the line (`MURMURE-199`, `MURMURE-200`). `Ctrl+Z` undoes, and
    /// `Ctrl+Shift+Z` or `Ctrl+Y` redoes (`MURMURE-433`). Escape cancels an
    /// edit (`MURMURE-204`), and otherwise gives the keys back to the list.
    /// Answers whether the composer took the key.
    pub fn key(&mut self, of: DraftOf, key: &str, shift: bool, ctrl: bool, alt: bool) -> bool {
        if self.composing {
            return false;
        }
        let word = ctrl || alt;
        let intent = match key {
            "Enter" if shift => EditIntent::Newline,
            "Enter" => {
                self.send = Some(of);
                return true;
            }
            "Escape" if self.editing(of) => {
                self.end_edit(of);
                return true;
            }
            "Escape" => {
                self.blur();
                return true;
            }
            "z" | "Z" | "y" | "Y" if ctrl => {
                let redo = shift || key.eq_ignore_ascii_case("y");
                self.typed = true;
                self.editor(of);
                self.history(of).asked.push(redo);
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
        let history = self.history(of);
        match intent {
            EditIntent::Newline => history.apart(),
            EditIntent::Backspace | EditIntent::Delete => history.edit(),
            _ => history.seal(),
        }
        self.editor(of).push(intent);
        true
    }

    /// A press at `(x, y)` in the band: the caret goes there, and a drag
    /// from it selects.
    pub fn press(&mut self, of: DraftOf, x: f32, y: f32) {
        let (dx, dy) = self.origin;
        self.history(of).seal();
        self.editor(of).point_down(x - dx, y - dy);
        self.dragging = true;
    }

    /// A pointer moved: a drag from the field extends the selection.
    /// Answers whether the composer held a drag.
    pub fn drag_to(&mut self, of: DraftOf, x: f32, y: f32) -> bool {
        if !self.dragging {
            return false;
        }
        let (dx, dy) = self.origin;
        self.editor(of).point_drag(x - dx, y - dy);
        true
    }

    /// The button lifted. Answers whether the composer held a drag.
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.dragging)
    }

    /// A double click in the field selects the word under it.
    pub fn word_at(&mut self, of: DraftOf, x: f32, y: f32) {
        let (dx, dy) = self.origin;
        self.history(of).seal();
        self.editor(of).word_at(x - dx, y - dy);
    }

    /// Apply the edits since the last paint, and size the band for the
    /// draft `shown` in a region of `size`. Answers what Enter sent. An empty
    /// draft sends nothing (`MURMURE-201`). Only the sent draft leaves, so
    /// another channel or thread keeps its own (`MURMURE-202`, `MURMURE-524`).
    /// An edit sends an edit of its message, and one that changed nothing
    /// just ends (`MURMURE-203`).
    pub fn flush(
        &mut self,
        font_cx: &mut FontContext,
        shown: Option<DraftOf>,
        size: (f32, f32),
        palette: &Palette,
    ) -> Option<Sent> {
        let wrap = (size.0 - 2.0 * SPACE_8).max(1.0);
        let style = body_style(palette.cell_text);
        let mut sent = None;
        if let Some(asked) = self.send.take()
            && let Some(editor) = self.drafts.get_mut(&asked)
        {
            editor.set_style(style.clone());
            editor.flush(font_cx, wrap, 1.0);
            let text = editor.text();
            if let Some(open) = self.editing.get(&asked) {
                if !text.trim().is_empty() {
                    if text != open.original {
                        sent = Some(Sent::Edit {
                            channel: asked.channel(),
                            at: open.at,
                            message: open.message,
                            draft: MessageDraft {
                                mentions: kept_mentions(&open.original, &text, &open.mentions),
                                text,
                            },
                        });
                    }
                    self.end_edit(asked);
                }
            } else if !text.trim().is_empty() {
                self.drafts.remove(&asked);
                // The history leaves with the draft: no undo reaches a
                // message sent (`MURMURE-434`).
                self.histories.remove(&asked);
                // The box unchecks once the reply leaves (`MURMURE-531`).
                sent = Some(Sent::Message {
                    to: asked,
                    draft: MessageDraft {
                        text,
                        mentions: Vec::new(),
                    },
                    also_in_channel: self.also.remove(&asked),
                });
            }
        }
        let Some(shown) = shown else {
            self.band = 0.0;
            return sent;
        };
        let tall = match self.drafts.get_mut(&shown) {
            Some(editor) => {
                editor.set_style(style);
                editor.flush(font_cx, wrap, 1.0);
                // Record the edits that landed, then walk the steps asked
                // after them. A step puts the caret at the end of the draft.
                let history = self.histories.entry(shown).or_default();
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
        if matches!(shown, DraftOf::Thread(..)) {
            self.band += ALSO_PX;
        }
        sent
    }

    /// Paint the band of the draft `shown` at the foot of a region of
    /// `size`, over any row that reached it. The line of who types sits above
    /// the field. Answers the caret, its top-left and its height, while the
    /// composer holds the focus.
    #[allow(clippy::too_many_arguments)]
    pub fn paint<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        shown: Option<DraftOf>,
        typing: Option<&str>,
        size: (f32, f32),
        palette: &Palette,
    ) -> Option<(f32, f32, f32)> {
        let shown = shown?;
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
        // A thread's field leaves the row of its box under it.
        self.also_row = None;
        let height = match shown {
            DraftOf::Thread(..) => {
                let row = height - ALSO_PX;
                self.also_row = Some((row, height));
                let side = f64::from(BOX_PX);
                let (x, y) = (
                    f64::from(SPACE_8),
                    f64::from(row + (ALSO_PX - BOX_PX) / 2.0),
                );
                let square = Rect::new(x, y, x + side, y + side);
                if self.also.contains(&shown) {
                    surface.fill_rect(square, palette.selection);
                }
                surface.stroke_rect(square, palette.cell_border, 1.0);
                let words = shape(font_cx, cache, ALSO_IN_CHANNEL, FONT_UI, width, false);
                let label_y = row + (ALSO_PX - words.height()) / 2.0;
                draw(
                    surface,
                    words,
                    SPACE_8 + BOX_PX + SPACE_4,
                    label_y,
                    palette.header_text,
                );
                row
            }
            DraftOf::Channel(_) => height,
        };
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
        let editor = self.drafts.get_mut(&shown);
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

    /// A mention before the change keeps its range, one after it moves with
    /// the text, and one the change touches goes (`MURMURE-203`).
    #[test]
    fn an_edit_keeps_the_mentions_it_leaves_whole() {
        let at = |range: std::ops::Range<usize>| Mention {
            target: ambre_bus::MentionTarget::Channel,
            range,
        };
        // "@a hi @b": "hi" becomes "hello".
        let mentions = [at(0..2), at(6..8)];
        let kept = kept_mentions("@a hi @b", "@a hello @b", &mentions);
        assert_eq!(kept, [at(0..2), at(9..11)]);
        let kept = kept_mentions("@a hi @b", "@x hi @b", &mentions);
        assert_eq!(kept, [at(6..8)], "the first one was edited");
        let same = kept_mentions("@a hi @b", "@a hi @b", &mentions);
        assert_eq!(same, mentions, "no change keeps them all");
        let wide = kept_mentions("é @b", "éé @b", &[at(2..4)]);
        assert_eq!(wide, [at(3..5)], "ranges count chars, not bytes");
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
