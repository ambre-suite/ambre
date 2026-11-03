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

use ambre_core::Tally;
use ambre_core::index::{Delta, IdxVec, Len, Pos, Span};
use std::collections::{HashMap, HashSet};

use ambre_bus::{
    Attachment, Authored, Instant, LogicCommand, Mention, MessageAt, MessageDraft, MurmureCommand,
    ReplyEcho, StyleMark, TextStyle,
};
use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
use ambre_canvas::{EditIntent, Palette, TextCache, TextEditor, TextEditorStyle, TextSpan};
use ambre_commands::{Key, KeyBinding, KeyChar, Modifiers};
use ambre_core::id::{Channel, Id, Message, User};
use ambre_core::{
    Byte, Char, HistoryStep, byte_span_to_chars, byte_to_char, char_len, char_span_to_bytes,
};
use ambre_core::{Doc, Px, Scale, Size, View};
use ambre_draw::{Color, Consumed, DrawSurface, FontContext, Point, Rect};

use crate::Region;
use crate::commands::PendingEdit;
use crate::drafts::{DraftShelf, Loss, ShelfLosses};
use crate::look;
use crate::markers::{self, DraftEdit, MarkBar};
use crate::mention_list::{MentionEntry, MentionList, Offer, offers, query_at};
use crate::paint::{RowBounds, draw, shape, surface_px, text_height, text_width, view_rect};
use crate::read_aloud::{self, Say};
use crate::uploads::{UploadFile, Uploads};
use crate::view::WindowWidth;

/// The strip above the field while the composer holds an edit.
pub const EDITING_LINE: &str = "Editing a message. Escape cancels.";

/// The words an empty composer shows.
pub const COMPOSER_HINT: &str = "Write a message";

/// The height of the field when it holds no line yet.
const MIN_LINE_HEIGHT: Px<View> = Px::<View>::new(FONT_BODY.get() + SPACE_4.get());

/// The family of the composer's text, the one of the message bodies.
const FAMILY: ambre_core::FontFamily = ambre_core::FontFamily::Sans;

/// The height of the strip that names who types, above the field.
const TYPING_HEIGHT: Px<View> = Px::<View>::new(FONT_UI.get() + SPACE_4.get());

/// The words beside the box under a thread's field (`MURMURE-531`).
pub const ALSO_IN_CHANNEL: &str = "Also send to the channel";

/// The chord that turns the box, named after its words at the desktop width
/// (`MURMURE-544`). The box draws it as the binding spells itself.
pub const ALSO_CHORD: KeyBinding = KeyBinding::new(Modifiers::ALT, Key::Enter);

/// The height of the row of that box, under the field.
pub(crate) const ALSO_HEIGHT: Px<View> = Px::<View>::new(FONT_UI.get() + SPACE_8.get());

/// The height of the row of files over the field, while the draft has one.
pub(crate) const FILES_HEIGHT: Px<View> = Px::<View>::new(FONT_UI.get() + SPACE_8.get());

/// What ends the chip of a file a press takes off the draft.
const REMOVE_MARK: &str = "\u{d7}";

/// What a file still uploading says after its name.
pub const UPLOADING: &str = "uploading";

/// The side of the box.
const BOX_SIZE: Px<View> = FONT_UI;

/// The caret the composer drew: its top-left and its height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretBox {
    pub at: ambre_core::Point<Px<View>>,
    pub tall: Px<View>,
}

/// How far an input method has come with its word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composition {
    /// The word is a preview the method may still change.
    Preview,
    /// The method ended, and the word is final.
    Final,
}

/// What a screen reader hears when a key turns the box (`MURMURE-535`).
pub fn also_said(echo: ReplyEcho) -> String {
    let state = if echo == ReplyEcho::AlsoInChannel {
        "checked"
    } else {
        "unchecked"
    };
    format!("{ALSO_IN_CHANNEL}, {state}")
}

/// The line that names the members typing, `None` when nobody types. Past
/// two names it counts no one, so the line stays one short line. A member
/// with no name known reads as "Someone".
pub fn typing_line(names: &[Option<ambre_core::DisplayName>]) -> Option<String> {
    let names: Vec<&str> = names
        .iter()
        .map(|name| {
            name.as_ref()
                .map_or("Someone", ambre_core::DisplayName::as_str)
        })
        .collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("{one} is typing")),
        [one, two] => Some(format!("{one} and {two} are typing")),
        _ => Some("Several people are typing".to_owned()),
    }
}

/// The style of a draft: the face and the size of a message body.
fn body_style(color: Color) -> TextEditorStyle {
    TextEditorStyle {
        weight: ambre_core::Bounded::<ambre_core::FontWeight>::REGULAR,
        italic: ambre_core::Slant::Upright,
        family: FAMILY,
        base_size: FONT_BODY.unzoomed(Scale::<Doc, View>::ONE),
        color,
        underline: ambre_core::Underline::Plain,
        strikethrough: ambre_core::Strikethrough::Plain,
    }
}

/// The span the style `mark` draws over `text`, its chars turned to bytes,
/// in the look the list draws it in too (`MURMURE-564`).
fn span_of(text: &str, mark: &StyleMark, palette: &Palette) -> TextSpan {
    let shape = look::shape(&mark.style, char_span_to_bytes(text, mark.range));
    TextSpan {
        range: shape.range,
        weight: shape.weight,
        italic: shape.italic,
        family: shape.family,
        color: look::ink(&mark.style, palette),
        underline: shape.underline,
        background: look::tint(&mark.style, palette),
        indent: ambre_core::Px::<ambre_core::Doc>::new(f32::from(shape.indent.get())),
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
#[derive(Default)]
struct History {
    undo: Vec<String>,
    redo: Vec<String>,
    /// The draft as the last paint left it.
    last: String,
    /// The way of the step still open, `None` once sealed.
    open: Option<Change>,
    /// What the next paint does with the step once it records the edit.
    seal_after: SealAfter,
    /// Whether an edit waits for the next paint.
    pending: Pending,
    /// The steps asked since the last paint.
    asked: Vec<HistoryStep>,
}

ambre_core::define_flag! {
    /// Whether an edit of a draft waits for the next paint to record it.
    enum Pending {
        /// Every edit is recorded.
        Idle,
        /// An edit waits for the next paint.
        Waiting,
    }

    /// What the next paint does with the open step of a history once it
    /// records the edit.
    enum SealAfter {
        /// The step stays open, and the next edit of its way joins it.
        Keep,
        /// The step closes, and the next edit starts one of its own.
        Seal,
    }
}

impl History {
    /// An edit landed, which the next paint records.
    fn edit(&mut self) {
        self.pending = Pending::Waiting;
    }

    /// The next edit starts a step of its own. An edit not yet painted still
    /// joins the step it lands in, and closes it.
    fn seal(&mut self) {
        match self.pending {
            Pending::Waiting => self.seal_after = SealAfter::Seal,
            Pending::Idle => self.open = None,
        }
    }

    /// The edit about to land is a step of its own: a paste, a cut, a line
    /// break.
    fn apart(&mut self) {
        self.open = None;
        self.seal_after = SealAfter::Seal;
        self.pending = Pending::Waiting;
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
        if std::mem::take(&mut self.seal_after) == SealAfter::Seal {
            self.open = None;
        }
        self.pending = Pending::Idle;
    }

    /// Walk one step back, or forward for a redo, and answer the text the
    /// draft takes. `None` with nothing to walk.
    fn step(&mut self, step: HistoryStep) -> Option<String> {
        let (from, to) = match step {
            HistoryStep::Redo => (&mut self.redo, &mut self.undo),
            HistoryStep::Undo => (&mut self.undo, &mut self.redo),
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
    Channel(Id<Channel>),
    Thread(Id<Channel>, Id<Message>),
}

impl DraftOf {
    /// The channel the draft writes in.
    pub fn channel(self) -> Id<Channel> {
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

impl From<Id<Channel>> for DraftOf {
    fn from(channel: Id<Channel>) -> Self {
        Self::Channel(channel)
    }
}

/// A message the composer of a channel holds for an edit.
struct Editing {
    at: MessageAt,
    message: Id<Message>,
    /// The text the message read when the edit began, and its styles.
    original: String,
    original_styles: Vec<StyleMark>,
    /// The draft, its history and its mentions the edit put aside, `None`
    /// with no draft.
    kept: Option<(TextEditor, History, Option<Marks>)>,
}

/// The mentions and the styles of a draft, and the text whose chars their
/// ranges count.
#[derive(Clone, Default)]
struct Marks {
    text: String,
    mentions: Vec<Mention>,
    styles: Vec<StyleMark>,
}

impl Marks {
    /// The mentions `text` leaves whole, once the ranges follow it
    /// (`MURMURE-553`). The styles follow it too.
    fn follow(&mut self, text: &str) -> &[Mention] {
        if self.text != text {
            let edit = DraftEdit {
                before: &self.text,
                after: text,
            };
            self.mentions = kept_mentions(edit, &self.mentions);
            self.styles = markers::kept_styles(edit, &self.styles);
            text.clone_into(&mut self.text);
        }
        &self.mentions
    }

    /// The draft `text` sends, its marks followed to it.
    fn draft(&mut self, text: String) -> MessageDraft {
        let mentions = self.follow(&text).to_vec();
        MessageDraft {
            text,
            mentions,
            styles: crate::style::normalized(&self.styles),
        }
    }
}

/// The `@` list open in a draft (`MURMURE-551`).
#[derive(Clone)]
struct Asking {
    of: DraftOf,
    /// The byte of its `@`, and what follows it up to the caret.
    at: Pos<Byte>,
    query: String,
    /// The entries of the members the view last offered.
    offers: IdxVec<MentionEntry, Offer>,
    chosen: Pos<MentionEntry>,
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
        /// The files the draft carries, every one uploaded (`MURMURE-132`).
        attachments: Vec<Attachment>,
        also_in_channel: ReplyEcho,
    },
    /// An edit of a message of the member (`MURMURE-203`).
    Edit {
        channel: Id<Channel>,
        at: MessageAt,
        message: Id<Message>,
        draft: MessageDraft,
    },
}

impl Sent {
    /// The command to the Logic thread, stamped `now` by the view.
    pub fn command(self, now: Instant<Authored>) -> LogicCommand {
        match self {
            Self::Message {
                to,
                draft,
                attachments,
                also_in_channel,
            } => LogicCommand::Channel {
                channel_id: to.channel(),
                command: match to {
                    DraftOf::Channel(_) => MurmureCommand::SendMessage {
                        draft,
                        attachments,
                        now,
                        confirmed: ambre_bus::Confirmation::Unconfirmed,
                    },
                    DraftOf::Thread(_, root) => MurmureCommand::Reply {
                        root,
                        draft,
                        also_in_channel,
                        now,
                        confirmed: ambre_bus::Confirmation::Unconfirmed,
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

/// The mentions of the text before `edit` that the edit leaves whole. The
/// edit changed the chars between the common head and the common tail. A
/// mention before the change keeps its range, one after it moves with the
/// tail, and one the change touches goes.
fn kept_mentions(edit: DraftEdit<'_>, mentions: &[Mention]) -> Vec<Mention> {
    let markers::Changed {
        head,
        before_end,
        after_end,
    } = markers::change(edit);
    mentions
        .iter()
        .filter_map(|mention| {
            let range = mention.range;
            if range.end <= head {
                Some(mention.clone())
            } else if range.start >= before_end {
                let start = range.start + (after_end - before_end);
                Some(Mention {
                    target: mention.target,
                    range: Span::<Char>::from_len(start, range.len()),
                })
            } else {
                None
            }
        })
        .collect()
}

ambre_core::define_flag! {
    /// What the keys of a region reach.
    enum Focus {
        /// The list: the arrows move between its messages.
        List,
        /// The composer: the keys write the draft.
        Composer,
    }

    /// Whether an input method composes in the composer.
    enum Composing {
        /// No input method composes: the composer takes its keys.
        Idle,
        /// An input method shows a preview: its keys are its own.
        Preview,
    }

    /// Whether a key changed a draft since the view last asked.
    enum Typing {
        /// No key changed one: a caret move or a selection alone.
        Clean,
        /// A key changed one.
        Typed,
    }

    /// Whether a press in the field of the composer holds a drag.
    enum Drag {
        /// No press holds one.
        Idle,
        /// A press holds one: a move extends the selection.
        Selecting,
    }
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
    /// What the keys reach: the composer, or the list.
    focus: Focus,
    /// Whether an input method composes: its keys are its own.
    composing: Composing,
    /// Enter asked to send this draft.
    send: Option<DraftOf>,
    /// Whether a key changed a draft since the view last asked.
    typed: Typing,
    /// Whether a press in the field holds a drag.
    drag: Drag,
    /// The height of the band at the last paint, `0` with no channel.
    band: Px<View>,
    /// Where the text of the field started at the last paint.
    origin: ambre_core::Point<Px<View>>,
    /// The threads whose box asks for the reply in the channel too.
    also: HashSet<DraftOf>,
    /// The row of that box at the last paint, as its top and bottom.
    also_row: Option<RowBounds>,
    /// The width of the window: as wide as a desktop, the box names its
    /// chord (`MURMURE-544`).
    width: WindowWidth,
    /// The mentions each draft carries (`MURMURE-552`).
    marks: HashMap<DraftOf, Marks>,
    /// The `@` list, while a query is open.
    asking: Option<Asking>,
    /// The `@` Escape closed the list of: it stays closed while its query
    /// lasts (`MURMURE-554`).
    dismissed: Option<(DraftOf, Pos<Byte>)>,
    /// The entry picked, which the next paint writes into the draft.
    picked: Option<(DraftOf, Pos<MentionEntry>)>,
    /// A key that can close a marker was typed in this draft: the next paint
    /// looks before the caret (`MURMURE-197`).
    closing: Option<DraftOf>,
    /// A line break was typed in this draft: the next paint closes the block
    /// marker of the line it ended (`MURMURE-563`).
    breaking: Option<DraftOf>,
    /// The styles chords asked for, which the next paint puts on the
    /// selection or takes off it (`MURMURE-431`).
    toggles: Vec<(DraftOf, TextStyle)>,
    /// What the last key of the frame asks the next paint to say
    /// (`MURMURE-571` to `MURMURE-574`).
    saying: Option<(DraftOf, Say)>,
    /// What the last paint said, until the view takes it.
    said: Option<String>,
    /// The drafts sent since the shelf last read the composer, whose rows
    /// go (`MURMURE-212`).
    sent: Vec<DraftOf>,
    /// How many losses of the shelf the composer applied.
    losses_seen: Tally<ShelfLosses>,
    /// The files each draft carries, and those still uploading.
    uploads: Uploads,
    /// Each file chip of the last paint that a press takes off the draft, as
    /// its rectangle and its place in the draft's files.
    file_chips: Vec<(Rect, Pos<UploadFile>)>,
}

impl Composer {
    /// The files each draft carries, and those still uploading.
    pub fn uploads(&self) -> &Uploads {
        &self.uploads
    }

    /// The same, for the view to hand it an upload or its answer.
    pub fn uploads_mut(&mut self) -> &mut Uploads {
        &mut self.uploads
    }

    /// The height of the band at the last paint, `0` when no channel showed.
    pub fn band(&self) -> Px<View> {
        self.band
    }

    /// The file whose chip the last paint drew under `(x, y)`, by its place
    /// in the files of the draft shown.
    pub fn file_at(&self, at: ambre_core::Point<Px<View>>) -> Option<Pos<UploadFile>> {
        let at = Point::new(at.x.get().into(), at.y.get().into());
        self.file_chips
            .iter()
            .find(|(chip, _)| chip.contains(at))
            .map(|&(_, index)| index)
    }

    /// Whether the draft `of` shows a row of files.
    fn has_files(&self, of: DraftOf) -> bool {
        !self.uploads.ready(of).is_empty() || !self.uploads.uploading(of).is_empty()
    }

    /// Whether the keys reach the composer.
    pub fn focused(&self) -> bool {
        self.focus == Focus::Composer
    }

    /// The window is `width` wide: the box names its chord on a desktop
    /// alone. `true` when the chord shows or hides.
    pub fn show_chords(&mut self, width: WindowWidth) -> bool {
        std::mem::replace(&mut self.width, width) != width
    }

    /// The keys go back to the list.
    pub fn blur(&mut self) {
        self.focus = Focus::List;
        self.drag = Drag::Idle;
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
        idle && !asked && self.composing == Composing::Idle && !self.editing(of)
    }

    /// The text selected in the draft `of` a channel or a thread.
    pub fn selected(&self, of: DraftOf) -> Option<String> {
        self.drafts.get(&of)?.selected_text()
    }

    /// Whether the box of the thread `of` asks for the reply in the channel
    /// too (`MURMURE-531`).
    pub fn also_in_channel(&self, of: DraftOf) -> ReplyEcho {
        if self.also.contains(&of) {
            ReplyEcho::AlsoInChannel
        } else {
            ReplyEcho::ThreadOnly
        }
    }

    /// Check the box of the thread `of`, or uncheck it. A channel's draft
    /// has no box.
    pub fn toggle_also(&mut self, of: DraftOf) {
        if matches!(of, DraftOf::Thread(..)) && !self.also.remove(&of) {
            self.also.insert(of);
        }
    }

    /// Whether the height `y` falls on the row of the box at the last paint.
    pub fn on_also(&self, y: Px<View>) -> bool {
        self.also_row.is_some_and(|row| row.holds(y))
    }

    /// Whether a key changed a draft since the last call. A move of the
    /// caret or a selection changes none.
    pub fn take_typed(&mut self) -> bool {
        std::mem::take(&mut self.typed) == Typing::Typed
    }

    /// What a screen reader hears of the last paint, once
    /// (`MURMURE-571` to `MURMURE-574`).
    pub fn take_said(&mut self) -> Option<String> {
        self.said.take()
    }

    /// The editor of the draft `of`, opened empty on first use.
    fn editor(&mut self, of: DraftOf) -> &mut TextEditor {
        self.focus = Focus::Composer;
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
            None => self.drafts.remove(&of).map(|editor| {
                let history = self.histories.remove(&of).unwrap_or_default();
                (editor, history, self.marks.remove(&of))
            }),
        };
        let editor = TextEditor::new(&edit.text, body_style(Color::BLACK));
        self.drafts.insert(of, editor);
        let history = History {
            last: edit.text.clone(),
            ..History::default()
        };
        self.histories.insert(of, history);
        let marks = Marks {
            text: edit.text.clone(),
            mentions: edit.mentions,
            styles: edit.styles.clone(),
        };
        self.marks.insert(of, marks);
        self.editing.insert(
            of,
            Editing {
                at: edit.at,
                message: edit.message,
                original: edit.text,
                original_styles: edit.styles,
                kept,
            },
        );
        self.focus = Focus::Composer;
    }

    /// Give back a draft its author did not send, with its mentions and the
    /// box of its thread, and the caret at its end. A composer that holds a
    /// draft typed since keeps it (`MURMURE-558`). Answers whether it took it.
    pub fn restore(&mut self, of: DraftOf, draft: MessageDraft, echo: ReplyEcho) -> bool {
        if !self.blank(of) {
            return false;
        }
        self.load(of, draft, echo);
        self.focus = Focus::Composer;
        true
    }

    /// Hold `draft` as the draft `of`, the caret at its end.
    fn load(&mut self, of: DraftOf, draft: MessageDraft, echo: ReplyEcho) {
        let editor = TextEditor::new(&draft.text, body_style(Color::BLACK));
        self.drafts.insert(of, editor);
        let history = History {
            last: draft.text.clone(),
            ..History::default()
        };
        self.histories.insert(of, history);
        let marks = Marks {
            text: draft.text,
            mentions: draft.mentions,
            styles: draft.styles,
        };
        self.marks.insert(of, marks);
        if (echo == ReplyEcho::AlsoInChannel) && matches!(of, DraftOf::Thread(..)) {
            self.also.insert(of);
        }
    }

    /// Read the shelf before a paint of the draft `shown`. Drop what the
    /// shelf lost since the last read. Then take the draft it keeps for
    /// `shown` into a blank composer, the keys left where they are
    /// (`MURMURE-210`).
    pub fn unshelve(&mut self, shown: Option<DraftOf>, shelf: &DraftShelf) {
        let (losses, seen) = shelf.losses_since(self.losses_seen);
        self.losses_seen = seen;
        for loss in losses {
            let lost = |of: &DraftOf| match loss {
                Loss::Channel(channel) => of.channel() == channel,
                Loss::All => true,
            };
            self.drafts.retain(|of, _| !lost(of));
            self.histories.retain(|of, _| !lost(of));
            self.editing.retain(|of, _| !lost(of));
            self.marks.retain(|of, _| !lost(of));
            self.also.retain(|of| !lost(of));
            self.sent.retain(|of| !lost(of));
        }
        if let Some(of) = shown
            && self.blank(of)
            && let Some(draft) = shelf.get(of)
        {
            self.load(of, draft, ReplyEcho::ThreadOnly);
        }
    }

    /// Hand the shelf the drafts after a paint. That is each one held, and
    /// the one an edit put aside in place of the edit. The ones sent go
    /// (`MURMURE-212`). A draft this composer never held stays as the shelf
    /// keeps it, so another region's is safe.
    pub fn shelve(&mut self, shelf: &DraftShelf) {
        for of in std::mem::take(&mut self.sent) {
            if !self.drafts.contains_key(&of) {
                shelf.put(of, None);
            }
        }
        for (of, editor) in &self.drafts {
            let kept = match self.editing.get(of) {
                Some(open) => open
                    .kept
                    .as_ref()
                    .map(|(editor, _, marks)| (editor.text(), marks.clone().unwrap_or_default())),
                None => Some((
                    editor.text(),
                    self.marks.get(of).cloned().unwrap_or_default(),
                )),
            };
            let draft = kept
                .filter(|(text, _)| !text.is_empty())
                .map(|(text, mut marks)| marks.draft(text));
            shelf.put(*of, draft);
        }
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
        self.marks.remove(&of);
        if let Some((editor, history, marks)) = open.kept {
            self.drafts.insert(of, editor);
            self.histories.insert(of, history);
            if let Some(marks) = marks {
                self.marks.insert(of, marks);
            }
        }
    }

    /// Text the pipe committed: a typed key, a paste, a word an input
    /// method chose. A paste is a step of its own, and a space ends a word's.
    pub fn text(&mut self, of: DraftOf, text: &str) {
        if !text.is_empty() {
            self.typed = Typing::Typed;
        }
        // The screen reader echoes a typed key itself.
        self.saying = None;
        let history = self.history(of);
        if text.chars().count() > 1 {
            history.apart();
        }
        history.edit();
        if text.ends_with(char::is_whitespace) {
            history.seal_after = SealAfter::Seal;
        }
        // A paste is text, whatever its last char (`MURMURE-197`).
        if markers::closes(text) {
            self.closing = Some(of);
        }
        self.editor(of).insert(text);
    }

    /// The preview of an input method, or its final word once it ends.
    pub fn compose(&mut self, of: DraftOf, text: &str, stage: Composition) {
        let end = stage == Composition::Final;
        self.composing = if end || text.is_empty() {
            Composing::Idle
        } else {
            Composing::Preview
        };
        if !text.is_empty() {
            self.typed = Typing::Typed;
        }
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
        self.typed = Typing::Typed;
        self.saying = Some((of, Say::Removed));
        self.history(of).apart();
        Some(text)
    }

    /// A key while the composer holds the focus. Enter sends and Shift+Enter
    /// breaks the line (`MURMURE-199`, `MURMURE-200`). Alt+Enter in a thread
    /// checks the box or unchecks it (`MURMURE-534`). `Ctrl+Z` undoes, and
    /// `Ctrl+Shift+Z` or `Ctrl+Y` redoes (`MURMURE-433`). Escape cancels an
    /// edit (`MURMURE-204`), and otherwise gives the keys back to the list.
    ///
    /// An open `@` list takes its own keys first (`MURMURE-554`). Answers
    /// whether the composer took the key. `Ctrl` here is the primary
    /// accelerator, the command key on macOS (`UI-THREAD-028`).
    pub fn key(&mut self, of: DraftOf, key: Key, mods: Modifiers) -> Consumed {
        if self.composing == Composing::Preview {
            return Consumed::No;
        }
        if mods == Modifiers::NONE && self.list_key(of, key).is_yes() {
            return Consumed::Yes;
        }
        let (primary, alt, shift) = (mods.primary(), mods.alt(), mods.shift());
        // A chord puts its style on the selection once the keys before it
        // landed (`MURMURE-431`).
        if mods == Modifiers::PRIMARY
            && let Some(style) = markers::chord_style(key)
        {
            self.toggle(of, style);
            return Consumed::Yes;
        }
        let word = ambre_canvas::Stride::word_if(primary || alt);
        let intent = match key {
            Key::Enter if shift => {
                // A line break closes the block marker of its line.
                self.breaking = Some(of);
                EditIntent::Newline
            }
            Key::Enter if alt && matches!(of, DraftOf::Thread(..)) => {
                self.toggle_also(of);
                return Consumed::Yes;
            }
            Key::Enter => {
                self.send = Some(of);
                return Consumed::Yes;
            }
            Key::Escape if self.editing(of) => {
                self.end_edit(of);
                return Consumed::Yes;
            }
            Key::Escape => {
                self.blur();
                return Consumed::Yes;
            }
            Key::Char(KeyChar::Z | KeyChar::Y) if primary => {
                let step = if shift || key == Key::Char(KeyChar::Y) {
                    HistoryStep::Redo
                } else {
                    HistoryStep::Undo
                };
                self.typed = Typing::Typed;
                self.editor(of);
                self.history(of).asked.push(step);
                self.saying = Some((of, Say::Draft));
                return Consumed::Yes;
            }
            Key::Char(KeyChar::A) if primary => EditIntent::SelectAll,
            Key::Backspace => EditIntent::Backspace,
            Key::Delete => EditIntent::Delete,
            Key::ArrowLeft => EditIntent::Left {
                select: ambre_canvas::Extend::select_if(shift),
                word,
            },
            Key::ArrowRight => EditIntent::Right {
                select: ambre_canvas::Extend::select_if(shift),
                word,
            },
            Key::ArrowUp => EditIntent::Up {
                select: ambre_canvas::Extend::select_if(shift),
            },
            Key::ArrowDown => EditIntent::Down {
                select: ambre_canvas::Extend::select_if(shift),
            },
            Key::Home => EditIntent::LineStart {
                select: ambre_canvas::Extend::select_if(shift),
            },
            Key::End => EditIntent::LineEnd {
                select: ambre_canvas::Extend::select_if(shift),
            },
            _ => return Consumed::No,
        };
        let edits = matches!(
            intent,
            EditIntent::Newline | EditIntent::Backspace | EditIntent::Delete
        );
        if edits {
            self.typed = Typing::Typed;
        }
        self.saying = match intent {
            EditIntent::Newline => None,
            EditIntent::Backspace | EditIntent::Delete => Some((of, Say::Removed)),
            EditIntent::Left {
                word: ambre_canvas::Stride::Word,
                ..
            }
            | EditIntent::Right {
                word: ambre_canvas::Stride::Word,
                ..
            } => Some((of, Say::Word)),
            EditIntent::Left { .. } | EditIntent::Right { .. } | EditIntent::SelectAll => {
                Some((of, Say::Char))
            }
            _ => Some((of, Say::Line)),
        };
        // A caret move ends the step, and a line break is one of its own.
        let history = self.history(of);
        match intent {
            EditIntent::Newline => history.apart(),
            EditIntent::Backspace | EditIntent::Delete => history.edit(),
            _ => history.seal(),
        }
        self.editor(of).push(intent);
        Consumed::Yes
    }

    /// A key while the `@` list of the draft `of` shows an entry. Up and Down
    /// move the choice round the list, Enter and Tab pick it, and Escape
    /// closes the list until its query ends (`MURMURE-554`).
    fn list_key(&mut self, of: DraftOf, key: Key) -> Consumed {
        let Some(asking) = self
            .asking
            .as_mut()
            .filter(|asking| asking.of == of && !asking.offers.is_empty())
        else {
            return Consumed::No;
        };
        let len = asking.offers.len();
        match key {
            // The list is not empty, so a wrapped entry always exists.
            Key::ArrowDown => {
                asking.chosen = asking
                    .chosen
                    .wrapped(Delta::new(1), len)
                    .unwrap_or(Pos::ZERO);
            }
            Key::ArrowUp => {
                asking.chosen = asking
                    .chosen
                    .wrapped(Delta::new(-1), len)
                    .unwrap_or(Pos::ZERO);
            }
            Key::Enter | Key::Tab => {
                let chosen = asking.chosen;
                self.pick(of, chosen);
            }
            Key::Escape => {
                self.dismissed = Some((of, asking.at));
                self.asking = None;
            }
            _ => return Consumed::No,
        }
        Consumed::Yes
    }

    /// Pick the entry `index` of the `@` list of the draft `of`: the next
    /// paint writes it over the query (`MURMURE-552`). A pick is a step of
    /// its own in the history.
    pub fn pick(&mut self, of: DraftOf, index: Pos<MentionEntry>) {
        let shows = self
            .asking
            .as_ref()
            .is_some_and(|asking| asking.of == of && asking.offers.get(index).is_some());
        if shows {
            self.picked = Some((of, index));
            self.typed = Typing::Typed;
            self.history(of).apart();
        }
    }

    /// Hand the `@` list the members of its channel, and keep the choice on
    /// the list. `me` is never offered (`MURMURE-551`).
    pub fn offer(&mut self, members: &[(Id<User>, ambre_core::DisplayName)], me: Option<Id<User>>) {
        if let Some(asking) = &mut self.asking {
            asking.offers = IdxVec::<MentionEntry, _>::from(offers(&asking.query, members, me));
            asking.chosen = asking.chosen.within(asking.offers.len());
        }
    }

    /// The channel whose members the open `@` list wants, `None` while no
    /// list is open.
    pub fn asks_members_of(&self) -> Option<Id<Channel>> {
        self.asking.as_ref().map(|asking| asking.of.channel())
    }

    /// What the page draws of the `@` list, from the caret `at` of the last
    /// paint. `None` while no list is open or no entry matches
    /// (`MURMURE-555`).
    pub fn mention_list(
        &self,
        at: ambre_core::Point<Px<View>>,
        region: Region,
    ) -> Option<MentionList> {
        let asking = self.asking.as_ref().filter(|a| !a.offers.is_empty())?;
        Some(MentionList {
            entries: asking.offers.iter().map(Offer::written).collect(),
            chosen: asking.chosen,
            at,
            region,
        })
    }

    /// Put `style` on the selection of the draft `of` at the next paint, or
    /// take it off. A chord asks for it, or a press on the format bar
    /// (`MURMURE-431`, `MURMURE-432`).
    pub fn toggle(&mut self, of: DraftOf, style: TextStyle) {
        self.typed = Typing::Typed;
        self.editor(of);
        self.toggles.push((of, style));
    }

    /// What the page draws of the format bar of the draft `of`, from the
    /// caret `at` of the last paint. `None` while nothing is selected
    /// (`MURMURE-432`).
    pub fn mark_bar(
        &self,
        of: DraftOf,
        at: ambre_core::Point<Px<View>>,
        region: Region,
    ) -> Option<MarkBar> {
        let editor = self.drafts.get(&of)?;
        let bytes = editor.selection()?;
        let text = editor.text();
        text.get(..bytes.start.to_usize())?;
        text.get(..bytes.end.to_usize())?;
        let range = byte_span_to_chars(&text, bytes);
        let styles = self
            .marks
            .get(&of)
            .map(|m| m.styles.as_slice())
            .unwrap_or_default();
        let controls = markers::MARK_BAR
            .iter()
            .map(|(style, _)| (style.clone(), markers::cover(styles, style, range)))
            .collect();
        Some(MarkBar {
            controls,
            at,
            region,
        })
    }

    /// The mentions the draft `of` carries now, for a test.
    #[cfg(test)]
    pub fn mentions(&mut self, of: DraftOf) -> Vec<Mention> {
        let text = self
            .drafts
            .get(&of)
            .map(TextEditor::text)
            .unwrap_or_default();
        self.marks
            .get_mut(&of)
            .map(|marks| marks.follow(&text).to_vec())
            .unwrap_or_default()
    }

    /// Write the entry picked over its query, with a space after it, and
    /// mark the name (`MURMURE-552`). Runs once the edits before it landed,
    /// so the query is read where it now stands.
    fn write_pick(&mut self, editor: &mut TextEditor, font_cx: &mut FontContext, wrap: Px<View>) {
        let Some((of, index)) = self.picked.take() else {
            return;
        };
        let text = editor.text();
        let Some(asking) = self.asking.take().filter(|asking| asking.of == of) else {
            return;
        };
        let Some(offer) = asking.offers.get(index) else {
            return;
        };
        let Some((at, query)) = editor.caret().and_then(|caret| query_at(&text, caret)) else {
            return;
        };
        let written = offer.written();
        editor.push(EditIntent::Replace {
            start: at,
            end: at + Len::<Byte>::of(1 + query.len()),
            text: format!("{written} "),
        });
        editor.flush(
            font_cx,
            wrap,
            ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
        );
        let start = byte_to_char(&text, at);
        let marks = self.marks.entry(of).or_insert_with(|| Marks {
            text: text.clone(),
            ..Marks::default()
        });
        marks.follow(&text);
        marks.follow(&editor.text());
        marks.mentions.push(Mention {
            target: offer.target,
            range: Span::<Char>::from_len(start, char_len(&written)),
        });
        marks.mentions.sort_by_key(|mention| mention.range.start);
    }

    /// Turn the marker the caret of the draft `of` just closed into its
    /// mark, and take its chars out (`MURMURE-197`). A marker typed in code
    /// stays text (`MURMURE-198`). A line break closes the block marker of
    /// the line it ended (`MURMURE-563`). The change is a step of its own,
    /// so an undo gives the markers back.
    fn close_marker(
        &mut self,
        of: DraftOf,
        editor: &mut TextEditor,
        font_cx: &mut FontContext,
        wrap: Px<View>,
    ) {
        let closing = self.closing.take() == Some(of);
        let breaking = self.breaking.take() == Some(of);
        if !closing && !breaking {
            return;
        }
        let Some(caret) = editor.caret() else {
            return;
        };
        self.close_with(of, editor, font_cx, wrap, |text, code| {
            let by_break = breaking
                .then(|| markers::closed_by_break(text, caret, code))
                .flatten();
            by_break.or_else(|| {
                closing
                    .then(|| markers::closed(text, caret, code))
                    .flatten()
            })
        });
    }

    /// Turn the marker `found` reads in the draft `of` into its mark, given
    /// the text and the chars code covers, and take its chars out.
    fn close_with(
        &mut self,
        of: DraftOf,
        editor: &mut TextEditor,
        font_cx: &mut FontContext,
        wrap: Px<View>,
        found: impl FnOnce(&str, &[Span<Char>]) -> Option<markers::Closed>,
    ) {
        let text = editor.text();
        let marks = self.marks.entry(of).or_insert_with(|| Marks {
            text: text.clone(),
            ..Marks::default()
        });
        marks.follow(&text);
        let code: Vec<_> = marks
            .styles
            .iter()
            .filter(|mark| matches!(mark.style, TextStyle::Code | TextStyle::CodeBlock))
            .map(|mark| mark.range)
            .collect();
        let Some(closed) = found(&text, &code) else {
            return;
        };
        let markers::Unmarked { between, after } = closed.apply(&text);
        let caret = editor.caret();
        editor.push(EditIntent::Replace {
            start: closed.span.start,
            end: closed.span.end,
            text: text[closed.inner.to_usize_range()].to_owned(),
        });
        // Text typed past the marker in the same frame keeps the caret after
        // it: an empty replace puts the caret back.
        if let Some(caret) = caret.filter(|caret| *caret > closed.span.end) {
            let at = caret - closed.span.len().saturating_sub(closed.inner.len());
            editor.push(EditIntent::Replace {
                start: at,
                end: at,
                text: String::new(),
            });
        }
        editor.flush(
            font_cx,
            wrap,
            ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
        );
        if editor.text() != after {
            return;
        }
        // The close marker goes, then the open one, so a mention between
        // them stays whole.
        marks.follow(&between);
        marks.follow(&after);
        marks.styles.push(StyleMark {
            range: closed.range(&text),
            style: closed.style,
        });
        marks.styles = crate::style::normalized(&marks.styles);
        let history = self.history(of);
        history.seal();
        history.record(&after);
    }

    /// Put each style a chord asked for in the draft `of` on its selection,
    /// or take it off (`MURMURE-431`). With no selection, a chord changes
    /// nothing.
    fn toggle_styles(&mut self, of: DraftOf, editor: &TextEditor) {
        let toggles = std::mem::take(&mut self.toggles);
        let Some(bytes) = editor.selection() else {
            return;
        };
        let text = editor.text();
        let range = byte_span_to_chars(&text, bytes);
        let marks = self.marks.entry(of).or_insert_with(|| Marks {
            text: text.clone(),
            ..Marks::default()
        });
        marks.follow(&text);
        for (_, style) in toggles.into_iter().filter(|(to, _)| *to == of) {
            marks.styles = markers::toggled(&marks.styles, &style, range);
        }
    }

    /// The styles the draft `of` carries now, for a test.
    #[cfg(test)]
    pub fn styles(&self, of: impl Into<DraftOf>) -> Vec<StyleMark> {
        let of = of.into();
        let text = self
            .drafts
            .get(&of)
            .map(TextEditor::text)
            .unwrap_or_default();
        self.marks
            .get(&of)
            .map(|marks| marks.clone().draft(text).styles)
            .unwrap_or_default()
    }

    /// The spans the field of the draft `of` draws now, for a test.
    #[cfg(test)]
    pub fn spans(&self, of: impl Into<DraftOf>) -> Vec<TextSpan> {
        self.drafts
            .get(&of.into())
            .map(|editor| editor.spans().to_vec())
            .unwrap_or_default()
    }

    /// Open the `@` list where the caret of the draft `of` ends a query, or
    /// close it. A query Escape closed stays closed until it ends.
    fn ask(&mut self, of: DraftOf, editor: &TextEditor) {
        let text = editor.text();
        let query = editor.caret().and_then(|caret| query_at(&text, caret));
        let Some((at, query)) = query.filter(|_| self.focused()) else {
            self.asking = None;
            self.dismissed = None;
            return;
        };
        if self.dismissed != Some((of, at)) {
            self.dismissed = None;
        }
        if self.dismissed.is_some() {
            self.asking = None;
            return;
        }
        match &mut self.asking {
            Some(asking) if asking.of == of && asking.at == at => {
                query.clone_into(&mut asking.query)
            }
            _ => {
                self.asking = Some(Asking {
                    of,
                    at,
                    query: query.to_owned(),
                    offers: IdxVec::<MentionEntry, _>::default(),
                    chosen: Pos::<MentionEntry>::ZERO,
                });
            }
        }
    }

    /// A press at `(x, y)` in the band: the caret goes there, and a drag
    /// from it selects.
    pub fn press(&mut self, of: DraftOf, at: ambre_core::Point<Px<View>>) {
        let at = self.in_field(at);
        // The field that takes the focus says its draft (`MURMURE-574`).
        if !self.focused() {
            self.saying = Some((of, Say::Draft));
        }
        self.history(of).seal();
        self.editor(of).point_down(at);
        self.drag = Drag::Selecting;
    }

    /// A pointer moved: a drag from the field extends the selection.
    /// Answers whether the composer held a drag.
    pub fn drag_to(&mut self, of: DraftOf, at: ambre_core::Point<Px<View>>) -> Consumed {
        if self.drag == Drag::Idle {
            return Consumed::No;
        }
        let at = self.in_field(at);
        self.editor(of).point_drag(at);
        Consumed::Yes
    }

    /// The button lifted. Answers whether the composer held a drag.
    pub fn release(&mut self) -> Consumed {
        Consumed::yes_if(std::mem::take(&mut self.drag) == Drag::Selecting)
    }

    /// A double click in the field selects the word under it.
    pub fn word_at(&mut self, of: DraftOf, at: ambre_core::Point<Px<View>>) {
        let at = self.in_field(at);
        self.history(of).seal();
        self.editor(of).word_at(at);
    }

    /// The point `at` of the view in the field's own layout, which the list
    /// draws at no zoom.
    fn in_field(&self, at: ambre_core::Point<Px<View>>) -> ambre_core::Point<Px<Doc>> {
        let from = at - self.origin;
        ambre_core::Point::new(
            from.dx.unzoomed(Scale::<Doc, View>::ONE),
            from.dy.unzoomed(Scale::<Doc, View>::ONE),
        )
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
        size: Size<Px<View>>,
        palette: &Palette,
    ) -> Option<Sent> {
        let wrap = (size.width - SPACE_8 * 2.0).max(Px::<View>::new(1.0));
        let style = body_style(palette.cell_text);
        let mut sent = None;
        if let Some(asked) = self.send.take()
            && let Some(mut editor) = self.drafts.remove(&asked)
        {
            editor.set_style(style.clone());
            editor.flush(
                font_cx,
                wrap,
                ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
            );
            // A marker or a chord typed before Enter lands before the draft
            // leaves, never on the draft that comes back.
            self.close_marker(asked, &mut editor, font_cx, wrap);
            // The send ends the last line, so it closes its block marker.
            self.close_with(asked, &mut editor, font_cx, wrap, markers::closed_at_send);
            self.toggle_styles(asked, &editor);
            let text = editor.text();
            self.drafts.insert(asked, editor);
            // The mentions the text left whole leave with it (`MURMURE-552`),
            // and the styles with them (`MURMURE-562`).
            let draft = self.marks.entry(asked).or_default().draft(text.clone());
            if let Some(open) = self.editing.get(&asked) {
                if !text.trim().is_empty() {
                    let styled = draft.styles != crate::style::normalized(&open.original_styles);
                    if text != open.original || styled {
                        sent = Some(Sent::Edit {
                            channel: asked.channel(),
                            at: open.at,
                            message: open.message,
                            draft,
                        });
                    }
                    self.end_edit(asked);
                }
            } else if self.uploads.busy(asked) {
                // A file still uploads: the draft holds until it lands or
                // fails, so no message names a file the relay lacks
                // (`MURMURE-132`).
            } else if !text.trim().is_empty() || !self.uploads.ready(asked).is_empty() {
                self.drafts.remove(&asked);
                // The history leaves with the draft: no undo reaches a
                // message sent (`MURMURE-434`).
                self.histories.remove(&asked);
                self.marks.remove(&asked);
                self.sent.push(asked);
                // The box unchecks once the reply leaves (`MURMURE-531`).
                sent = Some(Sent::Message {
                    to: asked,
                    draft,
                    attachments: self.uploads.take(asked),
                    also_in_channel: if self.also.remove(&asked) {
                        ReplyEcho::AlsoInChannel
                    } else {
                        ReplyEcho::ThreadOnly
                    },
                });
            }
        }
        let Some(shown) = shown else {
            self.band = Px::<View>::ZERO;
            self.asking = None;
            return sent;
        };
        let tall = match self.drafts.remove(&shown) {
            Some(mut editor) => {
                let before = editor.text();
                editor.set_style(style);
                editor.flush(
                    font_cx,
                    wrap,
                    ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
                );
                // A pick lands once the keys before it did.
                self.write_pick(&mut editor, font_cx, wrap);
                // Record the edits that landed, then walk the steps asked
                // after them. A step puts the caret at the end of the draft.
                let history = self.histories.entry(shown).or_default();
                history.record(&editor.text());
                let mut walked = false;
                for step in std::mem::take(&mut history.asked) {
                    if let Some(text) = history.step(step) {
                        editor.set_text(&text);
                        walked = true;
                    }
                }
                if walked {
                    editor.flush(
                        font_cx,
                        wrap,
                        ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
                    );
                }
                if let Some(marks) = self.marks.get_mut(&shown) {
                    marks.follow(&editor.text());
                }
                if walked {
                    self.closing = None;
                    self.breaking = None;
                }
                self.close_marker(shown, &mut editor, font_cx, wrap);
                self.toggle_styles(shown, &editor);
                self.ask(shown, &editor);
                // The field draws each style as it is typed (`MURMURE-564`).
                let text = editor.text();
                let spans = self.marks.get_mut(&shown).map_or_else(Vec::new, |marks| {
                    marks.follow(&text);
                    let styles = crate::style::normalized(&marks.styles);
                    styles
                        .iter()
                        .map(|mark| span_of(&text, mark, palette))
                        .collect()
                });
                editor.set_spans(font_cx, spans);
                if let Some((_, say)) = self.saying.take().filter(|(of, _)| *of == shown) {
                    let at = read_aloud::CaretAt {
                        caret: editor.caret(),
                        selection: editor.selection(),
                        drawn: editor.caret_line(),
                    };
                    let edit = DraftEdit {
                        before: &before,
                        after: &text,
                    };
                    self.said = read_aloud::said(say, edit, at);
                }
                let tall = editor.layout_height().unwrap_or(Px::<View>::ZERO);
                self.drafts.insert(shown, editor);
                tall
            }
            None => {
                self.asking = None;
                Px::<View>::ZERO
            }
        };
        let most = (size.height / 2.0 - SPACE_4 * 2.0).max(MIN_LINE_HEIGHT);
        self.band = TYPING_HEIGHT + tall.clamp(MIN_LINE_HEIGHT, most) + SPACE_4 * 2.0;
        if matches!(shown, DraftOf::Thread(..)) {
            self.band += ALSO_HEIGHT;
        }
        if self.has_files(shown) {
            self.band += FILES_HEIGHT;
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
        size: Size<Px<View>>,
        palette: &Palette,
    ) -> Option<CaretBox> {
        let shown = shown?;
        let (width, height) = (size.width, size.height);
        let rect = view_rect;
        let strip = height - self.band;
        surface.fill_rect(
            rect(Px::<View>::ZERO, strip, width, height),
            palette.content_bg,
        );
        if let Some(line) = typing {
            let shaped = shape(
                font_cx,
                cache,
                line,
                FONT_UI,
                width - SPACE_8 * 2.0,
                ambre_canvas::Weight::Regular,
            );
            let at = ambre_core::Point::new(SPACE_8, strip + SPACE_2);
            draw(surface, shaped, at, palette.header_text);
        }
        let mut top = strip + TYPING_HEIGHT;
        // The files of the draft sit in a row over the field.
        self.file_chips.clear();
        if self.has_files(shown) {
            self.paint_files(surface, font_cx, cache, shown, top, width, palette);
            top += FILES_HEIGHT;
        }
        // A thread's field leaves the row of its box under it.
        self.also_row = None;
        let height = match shown {
            DraftOf::Thread(..) => {
                let row = height - ALSO_HEIGHT;
                self.also_row = Some(RowBounds {
                    top: row,
                    bottom: height,
                });
                let side = surface_px(BOX_SIZE).get();
                let (x, y) = (
                    surface_px(SPACE_8).get(),
                    surface_px(row + (ALSO_HEIGHT - BOX_SIZE) / 2.0).get(),
                );
                let square = Rect::new(x, y, x + side, y + side);
                if self.also.contains(&shown) {
                    surface.fill_rect(square, palette.selection);
                }
                surface.stroke_rect(
                    square,
                    palette.cell_border,
                    ambre_core::Px::<ambre_core::Surface>::new(1.0),
                );
                let words = shape(
                    font_cx,
                    cache,
                    ALSO_IN_CHANNEL,
                    FONT_UI,
                    width,
                    ambre_canvas::Weight::Regular,
                );
                let label_y = row + (ALSO_HEIGHT - text_height(words)) / 2.0;
                let label_x = SPACE_8 + BOX_SIZE + SPACE_4;
                let chord_x = label_x + text_width(words) + SPACE_8;
                draw(
                    surface,
                    words,
                    ambre_core::Point::new(label_x, label_y),
                    palette.header_text,
                );
                // Below the desktop width, no chord shows (`CHROME-057`).
                if self.width == WindowWidth::Desktop {
                    let chord = shape(
                        font_cx,
                        cache,
                        &ALSO_CHORD.to_string(),
                        FONT_UI,
                        width,
                        ambre_canvas::Weight::Regular,
                    );
                    draw(
                        surface,
                        chord,
                        ambre_core::Point::new(chord_x, label_y),
                        palette.cell_border,
                    );
                }
                row
            }
            DraftOf::Channel(_) => height,
        };
        let field = rect(SPACE_4, top + SPACE_2, width - SPACE_4, height - SPACE_2);
        surface.fill_rect(field, palette.header_bg);
        let edge = if self.focused() {
            palette.selection
        } else {
            palette.cell_border
        };
        surface.stroke_rect(field, edge, ambre_core::Px::<ambre_core::Surface>::new(1.0));
        let inner = RowBounds {
            top: top + SPACE_4,
            bottom: height - SPACE_4,
        };
        self.origin = ambre_core::Point::new(SPACE_8, inner.top);
        let editor = self.drafts.get_mut(&shown);
        let empty = editor.as_ref().is_none_or(|e| e.text().is_empty())
            && self.composing == Composing::Idle;
        if empty {
            let hint = shape(
                font_cx,
                cache,
                COMPOSER_HINT,
                FONT_BODY,
                width,
                ambre_canvas::Weight::Regular,
            );
            draw(
                surface,
                hint,
                ambre_core::Point::new(SPACE_8, inner.top),
                palette.header_text,
            );
        }
        let editor = editor?;
        // A draft taller than the field shows its last lines.
        let tall = editor.layout_height().unwrap_or(Px::<View>::ZERO);
        let shift = (inner.bottom - inner.top - tall).min(Px::<View>::ZERO);
        self.origin = ambre_core::Point::new(SPACE_8, inner.top + shift);
        let origin = Point::new(
            surface_px(SPACE_8).get(),
            surface_px(inner.top + shift).get(),
        );
        surface.push_clip(rect(SPACE_4, inner.top, width - SPACE_4, inner.bottom));
        editor.render(
            surface,
            origin,
            palette.selection,
            ambre_canvas::Caret::shown_if(self.focus == Focus::Composer),
        );
        // A quote's rule and a list's dots, in the indent of their lines.
        if let (Some(layout), Some(marks)) = (editor.layout(), self.marks.get(&shown)) {
            let text = editor.text();
            for mark in crate::style::normalized(&marks.styles) {
                let bytes = char_span_to_bytes(&text, mark.range);
                for at in look::margin(layout, &text, &mark.style, bytes) {
                    let at = Rect::new(
                        at.x0 + origin.x,
                        at.y0 + origin.y,
                        at.x1 + origin.x,
                        at.y1 + origin.y,
                    );
                    surface.fill_rect(at, look::margin_ink(palette));
                }
            }
        }
        surface.pop_clip();
        let caret = editor.caret_local().filter(|_| self.focused())?;
        // The surface takes the caret in the same logical pixels; only the
        // float narrows.
        let (x, y, tall) = (
            (origin.x + caret.x0) as f32,
            (origin.y + caret.y0) as f32,
            caret.height() as f32,
        );
        Some(CaretBox {
            at: ambre_core::Point::new(Px::<View>::new(x), Px::<View>::new(y)),
            tall: Px::<View>::new(tall),
        })
    }

    /// Paint the files of the draft `shown` as a row of chips from `top`.
    /// The uploaded files come first, a press takes one off, and those that
    /// still upload follow (`MURMURE-132`). A chip past the width is left out.
    #[allow(clippy::too_many_arguments)]
    fn paint_files<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        shown: DraftOf,
        top: Px<View>,
        width: Px<View>,
        palette: &Palette,
    ) {
        let ready = self.uploads.ready(shown).iter().map(|file| {
            let size = crate::uploads::size_label(file.size);
            (format!("{} ({size}) {REMOVE_MARK}", file.name), true)
        });
        let uploading = self
            .uploads
            .uploading(shown)
            .into_iter()
            .map(|name| (format!("{name}, {UPLOADING}"), false));
        let chips: Vec<(String, bool)> = ready.chain(uploading).collect();
        let mut x = SPACE_8;
        for (index, (label, removable)) in chips.into_iter().enumerate() {
            let words = shape(
                font_cx,
                cache,
                &label,
                FONT_UI,
                width,
                ambre_canvas::Weight::Regular,
            );
            let chip_w = text_width(words) + SPACE_4 * 2.0;
            if x + chip_w > width - SPACE_4 {
                break;
            }
            let (y0, y1) = (top + SPACE_2, top + FILES_HEIGHT - SPACE_2);
            let chip = view_rect(x, y0, x + chip_w, y1);
            surface.fill_rect(chip, palette.header_bg);
            surface.stroke_rect(
                chip,
                palette.cell_border,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            let ink = if removable {
                palette.cell_text
            } else {
                palette.header_text
            };
            let label_y = y0 + (y1 - y0 - text_height(words)) / 2.0;
            draw(
                surface,
                words,
                ambre_core::Point::new(x + SPACE_4, label_y),
                ink,
            );
            if removable {
                self.file_chips
                    .push((chip, Pos::<UploadFile>::from_usize(index)));
            }
            x += chip_w + SPACE_4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One or two names read in full, more read as a count of no one, and
    /// nobody typing shows no line (`MURMURE-517`).
    #[test]
    fn the_typing_line_names_one_or_two_and_counts_no_more() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|n| ambre_core::DisplayName::new(n).ok())
                .collect::<Vec<_>>()
        };
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
            range: Span::<Char>::from_usize_range(range),
        };
        // "@a hi @b": "hi" becomes "hello".
        let mentions = [at(0..2), at(6..8)];
        let edit = |before, after| DraftEdit { before, after };
        let kept = kept_mentions(edit("@a hi @b", "@a hello @b"), &mentions);
        assert_eq!(kept, [at(0..2), at(9..11)]);
        let kept = kept_mentions(edit("@a hi @b", "@x hi @b"), &mentions);
        assert_eq!(kept, [at(6..8)], "the first one was edited");
        let same = kept_mentions(edit("@a hi @b", "@a hi @b"), &mentions);
        assert_eq!(same, mentions, "no change keeps them all");
        let wide = kept_mentions(edit("é @b", "éé @b"), &[at(2..4)]);
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
        assert_eq!(history.step(HistoryStep::Undo).as_deref(), Some("hi!"));
        assert_eq!(history.step(HistoryStep::Undo).as_deref(), Some(""));
        assert_eq!(
            history.step(HistoryStep::Undo),
            None,
            "nothing left to walk"
        );
        assert_eq!(history.step(HistoryStep::Redo).as_deref(), Some("hi!"));
    }
}
