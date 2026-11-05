//! What a key, a press or a run of typed text does to the view, and which
//! command it sends (`features/ambre-spec-prose-text-editing.md section 3`,
//! `features/ambre-spec-prose-selection.md section 2`).
//!
//! The host translates its own events into [`ProseInput`] and hands a send
//! function in. Every mutation leaves through that function (`PROSE-UI-004`);
//! the view writes its draft and its caret, and nothing else.

use crate::message::DocChar;
use ambre_bus::PasteSource;
use ambre_commands::{ClipboardWrite, Key, KeyChar, ModifierKeys, SelectionMode};
use ambre_core::Flags;
use ambre_core::StepDirection;
use ambre_core::index::{Delta, Pos, Span};
use ambre_core::{Point, Px, View};
use ambre_draw::Consumed;

use crate::events::{BlockInit, CursorPosition, ProseCommand, TextEdit};
use crate::layout::run::Affinity;
use crate::ui::{BlockWait, ProseUiState, SendResult, SyncHold};
use crate::view::caret::{Direction, SelectUnit, Spot};
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

/// Two presses closer than this in time, and in place, count as one gesture.
pub const MULTI_PRESS_WINDOW: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(500.0);

/// How far a second press may land from the first and still be in place.
const MULTI_PRESS: Px<View> = Px::<View>::new(4.0);

/// How many inputs the view holds while it waits on a structural command. A
/// wait lasts one round trip, so the bound is never reached by a hand. Text
/// is held past it, joined to the text before it (`PROSE-UI-198`).
const QUEUED_INPUTS: usize = 256;

/// A key the view acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProseKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Enter,
    Escape,
    SelectAll,
    /// `Ctrl+Z`; with `Shift` it redoes (`UNDO-010`, `UNDO-011`).
    Undo,
    /// `Ctrl+Y`, the second redo chord.
    Redo,
}

impl ProseKey {
    /// The key a pressed key stands for under the chord's modifiers.
    /// `None` for a key the view leaves to the host.
    #[must_use]
    pub fn from_key(key: Key, mods: Flags<ModifierKeys>) -> Option<Self> {
        let ctrl = mods.contains(ModifierKeys::PRIMARY);
        Some(match key {
            Key::ArrowLeft => Self::Left,
            Key::ArrowRight => Self::Right,
            Key::ArrowUp => Self::Up,
            Key::ArrowDown => Self::Down,
            Key::Home => Self::Home,
            Key::End => Self::End,
            Key::PageUp => Self::PageUp,
            Key::PageDown => Self::PageDown,
            Key::Backspace => Self::Backspace,
            Key::Delete => Self::Delete,
            Key::Enter => Self::Enter,
            Key::Escape => Self::Escape,
            Key::Char(KeyChar::A) if ctrl => Self::SelectAll,
            Key::Char(KeyChar::Z) if ctrl => Self::Undo,
            Key::Char(KeyChar::Y) if ctrl => Self::Redo,
            _ => return None,
        })
    }
}

/// One input, as the host hands it over. Points are pixels of the view, from
/// the canvas's corner.
///
/// The text an input carries is the user's, so `Debug` prints its length
/// alone.
#[derive(Clone, PartialEq)]
pub enum ProseInput {
    Key {
        key: ProseKey,
        mods: Flags<ModifierKeys>,
    },
    /// Committed text: typing, dictation, a plain paste.
    Text(String),
    Press {
        at: Point<Px<View>>,
        /// The modifiers held. A press with the primary one on a link follows
        /// the link (`PROSE-UI-071`); one with Shift extends the selection.
        mods: Flags<ModifierKeys>,
        now: Instant<PageLoad>,
    },
    Drag {
        at: Point<Px<View>>,
    },
    Release,
    /// Text dropped into the page at a point: one insertion there
    /// (`PROSE-UI-204`).
    Drop {
        at: Point<Px<View>>,
        text: String,
    },
    /// An item of the slash menu picked with the pointer, by its place in
    /// the filtered list.
    SlashPick(Pos<crate::view::slash::SlashChoice>),
    /// The input method's text in progress, replacing the previous one. It
    /// is drawn and never sent (`PROSE-UI-142`).
    Compose(String),
    /// The input method committed its text: one insertion at the caret
    /// (`PROSE-UI-029`).
    ComposeEnd(String),
    /// Copy the selection, and delete it after for a cut
    /// (`PROSE-UI-044`).
    Copy(ClipboardWrite),
    /// Paste over the selection or at the caret: this application's own
    /// bytes when there are any, the plain text otherwise (`PROSE-UI-042`).
    Paste {
        text: String,
        html: Option<String>,
        rich: Option<ambre_core::Payload<ambre_core::ClipboardBytes>>,
    },
}

impl std::fmt::Debug for ProseInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::Redacted;
        match self {
            Self::Key { key, mods } => f
                .debug_struct("Key")
                .field("key", key)
                .field("mods", mods)
                .finish(),
            Self::Text(text) => f.debug_tuple("Text").field(&Redacted::new(text)).finish(),
            Self::Press { at, mods, now } => f
                .debug_struct("Press")
                .field("at", at)
                .field("mods", mods)
                .field("now", now)
                .finish(),
            Self::Drag { at } => f.debug_struct("Drag").field("at", at).finish(),
            Self::Release => f.write_str("Release"),
            Self::Drop { at, text } => f
                .debug_struct("Drop")
                .field("at", at)
                .field("text", &Redacted::new(text))
                .finish(),
            Self::SlashPick(choice) => f.debug_tuple("SlashPick").field(choice).finish(),
            Self::Compose(text) => f
                .debug_tuple("Compose")
                .field(&Redacted::new(text))
                .finish(),
            Self::ComposeEnd(text) => f
                .debug_tuple("ComposeEnd")
                .field(&Redacted::new(text))
                .finish(),
            Self::Copy(write) => f.debug_tuple("Copy").field(write).finish(),
            Self::Paste { text, html, rich } => f
                .debug_struct("Paste")
                .field("text", &Redacted::new(text))
                .field("html", &html.as_ref().map(Redacted::new))
                .field("rich", rich)
                .finish(),
        }
    }
}

/// A command sent, as the view keeps track of it.
enum Sent {
    /// An insertion or a deletion inside one block, in the text the view
    /// shows, with the text an insertion carries.
    Text(TextEdit, Option<String>),
    /// A command that changes the blocks.
    Barrier,
}

impl Sent {
    fn of(command: &ProseCommand) -> Option<Self> {
        Some(match command {
            ProseCommand::InsertText {
                block_id,
                pos,
                text,
            } => Self::Text(
                TextEdit {
                    block_id: *block_id,
                    at: *pos,
                    delta: Delta::<DocChar>::from_len(
                        ambre_core::char_len(text).as_axis::<DocChar>(),
                    ),
                },
                Some(text.clone()),
            ),
            ProseCommand::DeleteText { block_id, range } => {
                let (start, end) = (range.start.min(range.end), range.start.max(range.end));
                Self::Text(
                    TextEdit {
                        block_id: *block_id,
                        at: start,
                        delta: Delta::<DocChar>::from_len(end.len_from(start)).reversed(),
                    },
                    None,
                )
            }
            ProseCommand::SplitBlock { .. }
            | ProseCommand::DeleteRange { .. }
            | ProseCommand::InsertBlock { .. }
            | ProseCommand::InsertTable { .. }
            | ProseCommand::InsertToc { .. }
            | ProseCommand::Paste { .. } => Self::Barrier,
            _ => return None,
        })
    }
}

/// The last press, to count a second and a third one in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Press {
    at: Point<Px<View>>,
    pressed_at: Instant<PageLoad>,
    clicks: Clicks,
}

/// Where a press stands in a run of presses in one place. A fourth press
/// starts the run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clicks {
    /// A press alone: it places the caret.
    Single,
    /// The second press: it selects a word.
    Double,
    /// The third press: it selects a paragraph.
    Triple,
}

impl Clicks {
    /// The press after this one in the same run.
    fn next(self) -> Self {
        match self {
            Self::Single => Self::Double,
            Self::Double => Self::Triple,
            Self::Triple => Self::Single,
        }
    }
}

impl ProseUiState {
    /// Act on one input. Answers whether the view consumed it; a key it does
    /// not act on is left to the host.
    ///
    /// After a structural command the view no longer knows where its
    /// positions fall, so it holds the input until the Logic thread answers
    /// (`features/ambre-spec-prose-text-editing.md section 3`). The host
    /// calls [`replay`](Self::replay) once events have been taken.
    pub fn handle_input(
        &mut self,
        input: &ProseInput,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        if self.waiting() {
            self.hold(input);
            return Consumed::Yes;
        }
        let (seen, mut sent_seen) = (self.seen, self.sent_seen);
        let mut synced = false;
        let mut sent: Vec<Sent> = Vec::new();
        let consumed = {
            let mut tracked = |command: ProseCommand| {
                let note = Sent::of(&command);
                // The Logic thread hears what this view has applied before
                // the edit that counts on it.
                if note.is_some() && seen > sent_seen {
                    if !send(ProseCommand::Sync { seen }).accepted() {
                        return SendResult::Dropped;
                    }
                    sent_seen = seen;
                    synced = true;
                }
                let result = send(command);
                if result.accepted() {
                    sent.extend(note);
                }
                result
            };
            self.act(input, &mut tracked)
        };
        self.sent_seen = sent_seen;
        if synced {
            self.syncs.push_back(SyncHold::Passes);
        }
        let mut barrier = false;
        for note in sent {
            match note {
                Sent::Text(edit, text) => {
                    // The number the Logic thread will acknowledge it by: one
                    // past the last taken, and past each command in flight.
                    let seq = self
                        .inflight
                        .iter()
                        .fold(self.popped.next(), |seq, _| seq.next());
                    match text {
                        Some(text) => self.draft.push_str(seq, &text, &edit.block_id, edit.at),
                        None => {
                            let len = edit.delta.magnitude();
                            self.draft
                                .push_delete(seq, &edit.block_id, edit.at, edit.at + len);
                        }
                    }
                    self.inflight.push_back(edit);
                }
                Sent::Barrier => barrier = true,
            }
        }
        if barrier && send(ProseCommand::Sync { seen: self.seen }).accepted() {
            self.sent_seen = self.seen;
            self.syncs.push_back(SyncHold::Holds);
        }
        consumed
    }

    /// Hold an input until the structural command is answered.
    ///
    /// A preview replaces the preview held before it, and text joins the text
    /// held before it. Past the bound a key or a pointer input is dropped;
    /// text never is, since the user cannot tell it went (`PROSE-UI-198`).
    fn hold(&mut self, input: &ProseInput) {
        match (self.queued.back_mut(), input) {
            (Some(ProseInput::Compose(held)), ProseInput::Compose(text)) => {
                held.clone_from(text);
                return;
            }
            // The end of a composition takes the place of its last preview.
            (Some(held @ ProseInput::Compose(_)), ProseInput::ComposeEnd(_)) => {
                held.clone_from(input);
                return;
            }
            (
                Some(ProseInput::Text(held) | ProseInput::ComposeEnd(held)),
                ProseInput::Text(text),
            ) => {
                held.push_str(text);
                return;
            }
            _ => {}
        }
        let carries_text = matches!(
            input,
            ProseInput::Text(_) | ProseInput::Compose(_) | ProseInput::ComposeEnd(_)
        );
        if carries_text || self.queued.len() < QUEUED_INPUTS {
            self.queued.push_back(input.clone());
        } else {
            tracing::warn!("Prose: too much input held, a key or a pointer input is dropped");
        }
    }

    /// Whether a structural command is still unanswered.
    fn waiting(&self) -> bool {
        self.syncs.contains(&SyncHold::Holds)
    }

    /// Act on the input held while a structural command was unanswered, in
    /// order, until another one holds the rest. Answers whether any was
    /// consumed.
    ///
    /// The Logic thread keeps each change until this view says it applied
    /// it, so the view says so here, once per batch of events taken.
    pub fn replay(&mut self, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> Consumed {
        if !self.waiting()
            && self.seen > self.sent_seen
            && send(ProseCommand::Sync { seen: self.seen }).accepted()
        {
            self.sent_seen = self.seen;
            self.syncs.push_back(SyncHold::Passes);
        }
        let mut consumed = Consumed::No;
        while !self.waiting() {
            let Some(input) = self.queued.pop_front() else {
                break;
            };
            if self.handle_input(&input, send).is_yes() {
                consumed = Consumed::Yes;
            }
        }
        consumed
    }

    fn act(
        &mut self,
        input: &ProseInput,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        // A press or a key stops the motion a flick left (`PROSE-UI-218`).
        if matches!(input, ProseInput::Press { .. } | ProseInput::Key { .. }) {
            self.stop_fling();
        }
        match input {
            ProseInput::Text(text) => self.type_into_menu(text, send),
            ProseInput::Key { key, mods } => {
                if let Some(consumed) = self.menu_key(*key, *mods, send) {
                    return consumed;
                }
                self.key(*key, *mods, send)
            }
            ProseInput::SlashPick(index) => self.pick(*index, send),
            ProseInput::Compose(text) => {
                // The preview replaces the selection, as typing does
                // (`PROSE-UI-197`).
                if !text.is_empty() && !self.take_selection(send) {
                    return Consumed::Yes;
                }
                self.composing.clone_from(text);
                Consumed::Yes
            }
            ProseInput::ComposeEnd(text) => {
                self.composing.clear();
                // The end of a composition is the view's whether or not the
                // run landed.
                let _ = self.type_into_menu(text, send);
                Consumed::Yes
            }
            ProseInput::Press { at, mods, now } => {
                let target = mods
                    .contains(ModifierKeys::PRIMARY)
                    .then(|| self.link_at(*at).map(|link| link.url.clone()))
                    .flatten();
                match target {
                    // The target goes to the host, and the caret and the
                    // selection stay as they are (`PROSE-UI-170`). A target
                    // that could run code in the page's stead is held back
                    // (`PROSE-UI-186`).
                    Some(url) => {
                        if crate::link::openable(&url) {
                            self.followed = Some(url);
                        }
                        Consumed::Yes
                    }
                    None => {
                        // A press on the canvas is outside the menu.
                        self.slash = None;
                        self.press(*at, SelectionMode::from_mods(*mods), *now, send)
                    }
                }
            }
            ProseInput::Drag { at } => {
                self.update_drag(*at);
                Consumed::yes_if(self.dragging.is_some())
            }
            ProseInput::Release => Consumed::yes_if(self.end_drag()),
            ProseInput::Drop { at, text } => self.drop_text(*at, text, send),
            ProseInput::Copy(write) => self.copy(*write, send),
            ProseInput::Paste { text, html, rich } => {
                self.paste(text, html.as_deref(), rich.as_ref(), send)
            }
        }
    }

    /// The payload of the last copy, once, for the host to mirror to the
    /// system clipboard (`PROSE-UI-041`).
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.pending_clip.take()
    }

    /// The paste that what the host read stands for. The payload kept from
    /// the last copy serves while the host found this tab's token on the
    /// clipboard; otherwise what the host read serves (`PROSE-UI-043`). The
    /// choice is the one every view makes (`UI-THREAD-053`).
    #[must_use]
    pub fn paste_input(&self, read: ambre_bus::ClipboardBlob, from: PasteSource) -> ProseInput {
        let source = ambre_bus::ClipboardBlob::source(self.last_clip.as_ref(), &read, from);
        if source.form == ambre_core::PasteForm::Plain {
            // The menu path reads every half, so the view drops them itself.
            return ProseInput::Paste {
                text: source.text.clone(),
                html: None,
                rich: None,
            };
        }
        ProseInput::Paste {
            text: source.text.clone(),
            html: source.html.clone(),
            rich: source.ambre.clone(),
        }
    }

    /// Ask the Logic thread for the payload of the whole selection. A cut
    /// deletes it once the copy is sent, and not when the copy was refused
    /// (`PROSE-UI-044`, `PROSE-UI-110`).
    fn copy(
        &mut self,
        write: ClipboardWrite,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let Some((anchor, focus)) = self.selection() else {
            return Consumed::No;
        };
        if !send(ProseCommand::CopyRange { anchor, focus }).accepted() {
            return Consumed::Yes;
        }
        if write == ClipboardWrite::Cut {
            self.take_selection(send);
        }
        Consumed::Yes
    }

    /// Paste over the selection, or at the caret. The content leaves as a
    /// command, and the view keeps no model of it (`PROSE-UI-013`).
    fn paste(
        &mut self,
        text: &str,
        html: Option<&str>,
        rich: Option<&ambre_core::Payload<ambre_core::ClipboardBytes>>,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        if rich.is_none() && html.is_none() && !text.contains('\n') {
            return self.type_into_menu(text, send);
        }
        if !self.take_selection(send) {
            return Consumed::Yes;
        }
        let Some(cursor) = self.cursor else {
            return Consumed::No;
        };
        let at = (cursor.block_id, cursor.offset);
        send(ProseCommand::Paste {
            at,
            text: text.replace("\r\n", "\n"),
            html: html.map(str::to_owned),
            payload: rich.cloned(),
        });
        Consumed::Yes
    }

    /// Text dropped at a point: the caret goes there, collapsed, and the text
    /// takes the paste path (`PROSE-UI-204`). One line is one insertion,
    /// drawn from the draft; several lines are one paste. A drop off the
    /// text changes nothing.
    fn drop_text(
        &mut self,
        at: Point<Px<View>>,
        text: &str,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let Some((block_id, offset)) = self.place_at(at) else {
            return Consumed::No;
        };
        self.slash = None;
        self.end_drag();
        self.cursor = Some(CursorPosition {
            block_id,
            offset,
            anchor: None,
        });
        self.caret_affinity = Affinity::Downstream;
        self.x_goal = None;
        let text = text.replace("\r\n", "\n");
        if text.contains('\n') {
            return self.paste(&text, None, None, send);
        }
        // The drop placed the caret, so it is the view's whether or not the
        // insertion landed.
        let _ = self.type_text(&text, send);
        Consumed::Yes
    }

    /// Type a run of text, and open, narrow or close the slash menu with it.
    ///
    /// The text always takes the ordinary path (`PROSE-UI-059`): the menu
    /// only watches it, and opening one writes nothing else (`PROSE-UI-092`).
    fn type_into_menu(
        &mut self,
        text: &str,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if self.slash.is_some() && text.chars().any(char::is_whitespace) {
            self.slash = None;
        }
        let at = self.cursor.map(|c| (c.block_id, c.offset));
        let opens = self.slash.is_none()
            && self.selection().is_none()
            && text
                .strip_prefix('/')
                .is_some_and(|rest| !rest.chars().any(char::is_whitespace))
            && at.is_some_and(|spot| self.opens_menu(spot));
        let consumed = self.type_text(&text, send);
        let typed = self.cursor.map(|c| (c.block_id, c.offset)) != at;
        if !typed {
            return consumed;
        }
        if opens && let Some(origin) = at {
            self.slash = Some(crate::view::slash::SlashMenu {
                origin,
                query: text.chars().skip(1).collect(),
                selected: Pos::<crate::view::slash::SlashChoice>::ZERO,
            });
        } else if let Some(menu) = &mut self.slash {
            menu.query.push_str(&text);
            menu.selected = Pos::<crate::view::slash::SlashChoice>::ZERO;
        }
        consumed
    }

    /// A key while the slash menu is open. `None` leaves the key to the
    /// editor, once the menu has closed if the key closes it.
    fn menu_key(
        &mut self,
        key: ProseKey,
        mods: Flags<ModifierKeys>,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Option<Consumed> {
        let menu = self.slash.as_mut()?;
        match key {
            ProseKey::Up | ProseKey::Down
                if !mods.contains(ModifierKeys::SHIFT) && !mods.contains(ModifierKeys::PRIMARY) =>
            {
                self.step_menu(if key == ProseKey::Down {
                    StepDirection::Forward
                } else {
                    StepDirection::Backward
                });
                Some(Consumed::Yes)
            }
            ProseKey::Enter if !crate::view::slash::filter(&menu.query).is_empty() => {
                let selected = menu.selected;
                Some(self.pick(selected, send))
            }
            ProseKey::Escape => {
                self.slash = None;
                Some(Consumed::Yes)
            }
            // The Backspace over the slash closes the menu; one over the
            // query narrows it less. Both delete like any Backspace.
            ProseKey::Backspace
                if !mods.contains(ModifierKeys::PRIMARY) && menu.query.pop().is_some() =>
            {
                menu.selected = Pos::<crate::view::slash::SlashChoice>::ZERO;
                None
            }
            _ => {
                self.slash = None;
                None
            }
        }
    }

    /// Confirm an item. The slash and the query leave the origin block, then
    /// the item's one command is sent (`PROSE-UI-060`, `PROSE-UI-093`). The
    /// menu closes (`PROSE-UI-175`).
    fn pick(
        &mut self,
        index: Pos<crate::view::slash::SlashChoice>,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let Some(menu) = self.slash.take() else {
            return Consumed::No;
        };
        let Some(item) = crate::view::slash::filter(&menu.query).get(index).copied() else {
            self.slash = Some(menu);
            return Consumed::Yes;
        };
        let erase = ProseCommand::DeleteText {
            block_id: menu.origin.0,
            range: menu.span(),
        };
        if !send(erase).accepted() {
            return Consumed::Yes;
        }
        self.set_caret(menu.origin);
        send(item.command.resolve(menu.origin.0));
        Consumed::Yes
    }

    /// Type a run of text at the caret (`PROSE-UI-136`, `PROSE-UI-176`).
    ///
    /// The selection goes first. The draft then takes the text only when the
    /// insertion reached the channel: a dropped command never happened.
    fn type_text(
        &mut self,
        text: &str,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() || self.cursor.is_none() {
            return Consumed::No;
        }
        if !self.take_selection(send) {
            return Consumed::Yes;
        }
        let Some(cursor) = self.cursor else {
            return Consumed::Yes;
        };
        let command = ProseCommand::InsertText {
            block_id: cursor.block_id,
            pos: cursor.offset,
            text: text.clone(),
        };
        if send(command).accepted() {
            self.advance_cursor(ambre_core::char_len(&text).as_axis::<DocChar>());
        }
        Consumed::Yes
    }

    /// Delete the active selection, and collapse the caret to where it
    /// started. The one primitive every editing path calls, and it sends the
    /// deletion alone: what follows is the caller's.
    ///
    /// Answers whether the caller may go on: true with no selection, or once
    /// the deletion reached the channel.
    pub(crate) fn take_selection(
        &mut self,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> bool {
        let Some((start, end)) = self.selection() else {
            return true;
        };
        let command = if start.0 == end.0 {
            ProseCommand::DeleteText {
                block_id: start.0,
                range: Span::<DocChar>::new(start.1, end.1),
            }
        } else {
            ProseCommand::DeleteRange {
                anchor: start,
                focus: end,
            }
        };
        if !send(command).accepted() {
            return false;
        }
        self.set_caret(start);
        self.reveal_caret();
        true
    }

    fn key(
        &mut self,
        key: ProseKey,
        mods: Flags<ModifierKeys>,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let ctrl = mods.contains(ModifierKeys::PRIMARY);
        // A history step needs no caret: the engine places it after
        // (`UNDO-010`, `UNDO-011`). The key is the view's whether or not the
        // command was taken, so the pipe never runs its own undo.
        if matches!(key, ProseKey::Undo | ProseKey::Redo) {
            let redo = key == ProseKey::Redo || mods.contains(ModifierKeys::SHIFT);
            send(if redo {
                ProseCommand::Redo
            } else {
                ProseCommand::Undo
            });
            return Consumed::Yes;
        }
        if self.cursor.is_none() {
            return Consumed::No;
        }
        let direction = match key {
            ProseKey::Left if ctrl => Direction::WordLeft,
            ProseKey::Left => Direction::Left,
            ProseKey::Right if ctrl => Direction::WordRight,
            ProseKey::Right => Direction::Right,
            ProseKey::Up => Direction::Up,
            ProseKey::Down => Direction::Down,
            ProseKey::Home if ctrl => Direction::DocStart,
            ProseKey::Home => Direction::LineStart,
            ProseKey::End if ctrl => Direction::DocEnd,
            ProseKey::End => Direction::LineEnd,
            ProseKey::PageUp => Direction::PageUp,
            ProseKey::PageDown => Direction::PageDown,
            ProseKey::Backspace => return self.delete(StepDirection::Backward, send),
            ProseKey::Delete => return self.delete(StepDirection::Forward, send),
            ProseKey::Enter => return self.split(send),
            ProseKey::Escape => {
                self.escape();
                return Consumed::Yes;
            }
            ProseKey::SelectAll => {
                self.select_all();
                return Consumed::Yes;
            }
            // Taken above, before the caret is asked for.
            ProseKey::Undo | ProseKey::Redo => return Consumed::Yes,
        };
        self.move_cursor(direction, SelectionMode::from_mods(mods));
        Consumed::Yes
    }

    /// Backspace, or Delete forward: the selection, else the grapheme beside
    /// the caret, else the join with the paragraph beside it.
    fn delete(
        &mut self,
        direction: StepDirection,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let forward = direction == StepDirection::Forward;
        if self.selection().is_some() {
            self.take_selection(send);
            return Consumed::Yes;
        }
        let Some(cursor) = self.cursor else {
            return Consumed::No;
        };
        let here: Spot = (cursor.block_id, cursor.offset);
        let beside = self.grapheme_beside(here, direction);
        let (command, caret) = match beside {
            Some(other) if forward => (
                ProseCommand::DeleteText {
                    block_id: here.0,
                    range: Span::<DocChar>::new(here.1, other),
                },
                here,
            ),
            Some(other) => (
                ProseCommand::DeleteText {
                    block_id: here.0,
                    range: Span::<DocChar>::new(other, here.1),
                },
                (here.0, other),
            ),
            None => {
                let Some(next) = self.neighbour(here.0, direction) else {
                    return Consumed::Yes;
                };
                let edge = (
                    next,
                    if forward {
                        Pos::<DocChar>::ZERO
                    } else {
                        self.block_len(next).as_end()
                    },
                );
                let (first, second) = if forward { (here, edge) } else { (edge, here) };
                (
                    ProseCommand::DeleteRange {
                        anchor: first,
                        focus: second,
                    },
                    first,
                )
            }
        };
        if send(command).accepted() {
            self.set_caret(caret);
            self.reveal_caret();
        }
        Consumed::Yes
    }

    /// The grapheme boundary beside a place, in document offsets, reading the
    /// text the view shows. `None` at the edge of the block.
    fn grapheme_beside(&self, spot: Spot, direction: StepDirection) -> Option<Pos<DocChar>> {
        use unicode_segmentation::UnicodeSegmentation;
        let text = self.shown_text(spot.0)?;
        let boundaries = text
            .grapheme_indices(true)
            .map(|(byte, _)| {
                ambre_core::char_len(text.get(..byte).unwrap_or(""))
                    .as_axis::<DocChar>()
                    .as_end()
            })
            .chain(std::iter::once(
                ambre_core::char_len(&text).as_axis::<DocChar>().as_end(),
            ));
        match direction {
            StepDirection::Forward => boundaries.into_iter().find(|b| *b > spot.1),
            StepDirection::Backward => boundaries.into_iter().rfind(|b| *b < spot.1),
        }
    }

    /// Enter: the selection goes, then the paragraph parts at the caret. The
    /// caret moves when the split confirms, onto the block it minted
    /// (`PROSE-UI-141`).
    fn split(&mut self, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> Consumed {
        if !self.take_selection(send) {
            return Consumed::Yes;
        }
        let Some(cursor) = self.cursor else {
            return Consumed::Yes;
        };
        let command = ProseCommand::SplitBlock {
            block_id: cursor.block_id,
            pos: cursor.offset,
        };
        if send(command).accepted() {
            // Drawn parted until the document answers (`PROSE-UI-203`).
            self.splitting = Some(crate::view::provisional::Splitting {
                block: cursor.block_id,
                at: cursor.offset,
                new_block: None,
            });
        }
        Consumed::Yes
    }

    /// A press on the canvas: a caret, a word, a paragraph or an extension,
    /// by how many presses landed in place and by Shift.
    ///
    /// It shapes nothing and scrolls nothing: what it resolved is on screen
    /// already (`PROSE-UI-157`). A layout with no page resolves nowhere and
    /// the caret stays (`PROSE-UI-156`).
    fn press(
        &mut self,
        at: Point<Px<View>>,
        mode: SelectionMode,
        now: Instant<PageLoad>,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let layout = &self.shaped.layout;
        if layout.page_count.is_zero() {
            return Consumed::No;
        }
        if self.shaped.block_count().is_zero() {
            // Nothing to hit: the block comes through the command path, and
            // the caret goes to it once a layout carries it (`PROSE-UI-072`).
            let command = ProseCommand::InsertBlock {
                parent_id: None,
                after_id: None,
                init: BlockInit::Paragraph,
                style_id: None,
            };
            if send(command).accepted() {
                self.awaiting_block = BlockWait::Awaiting;
            }
            return Consumed::Yes;
        }
        let Some(spot) = self.place_at(at) else {
            return Consumed::No;
        };
        let clicks = match self.last_press {
            Some(last)
                if now - last.pressed_at <= MULTI_PRESS_WINDOW
                    && (at.x - last.at.x).abs() <= MULTI_PRESS
                    && (at.y - last.at.y).abs() <= MULTI_PRESS =>
            {
                last.clicks.next()
            }
            _ => Clicks::Single,
        };
        self.last_press = Some(Press {
            at,
            pressed_at: now,
            clicks,
        });
        // A caret in a header or a footer shows on the page it was placed
        // on, since every page draws the band (`PROSE-UI-033`).
        if self.is_band(spot.0) {
            self.band_page = self.page_under(at);
        }
        let anchor = self
            .cursor
            .as_ref()
            .map_or(spot, |c| c.anchor.unwrap_or((c.block_id, c.offset)));
        match clicks {
            Clicks::Single if mode == SelectionMode::Extend && self.joins(anchor, spot) => {
                self.start_drag(anchor);
                self.update_drag(at);
            }
            Clicks::Single => {
                // A plain press collapses the selection (`PROSE-UI-154`).
                self.set_caret(spot);
                self.start_drag(spot);
            }
            Clicks::Double => self.select_around(spot, SelectUnit::Word),
            Clicks::Triple => self.select_around(spot, SelectUnit::Paragraph),
        }
        Consumed::Yes
    }

    /// A layout was taken: a caret a press asked a block for goes to the
    /// first one now (`PROSE-UI-072`).
    pub(crate) fn settle_awaited_block(&mut self) {
        if self.awaiting_block == BlockWait::Idle {
            return;
        }
        let first = self
            .shaped
            .window_blocks()
            .find(|(_, block)| self.shaped.blocks.contains_key(&block.block_id))
            .map(|(_, block)| crate::events::block_id(block.block_id));
        if let Some(block) = first {
            self.awaiting_block = BlockWait::Idle;
            self.set_caret((block, Pos::<DocChar>::ZERO));
        }
    }
}
