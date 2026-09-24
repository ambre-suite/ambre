//! What a key, a press or a run of typed text does to the view, and which
//! command it sends (`features/ambre-spec-prose-text-editing.md section 3`,
//! `features/ambre-spec-prose-selection.md section 2`).
//!
//! The host translates its own events into [`ProseInput`] and hands a send
//! function in. Every mutation leaves through that function (`PROSE-UI-004`);
//! the view writes its draft and its caret, and nothing else.

use crate::events::{BlockInit, ProseCommand, TextEdit};
use crate::ui::{ProseUiState, SendResult, chars};
use crate::view::caret::{Direction, Spot};

/// Two presses closer than this in time, and in place, count as one gesture.
pub const MULTI_PRESS_MS: f64 = 500.0;

/// How far a second press may land from the first and still be in place, in
/// canvas pixels.
const MULTI_PRESS_PX: f32 = 4.0;

/// How many inputs the view holds while it waits on a structural command. A
/// wait lasts one round trip, so the bound is never reached by a hand.
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
}

impl ProseKey {
    /// The key a browser key name stands for, `ctrl` holding the command key.
    /// `None` for a key the view leaves to the host.
    #[must_use]
    pub fn from_name(name: &str, ctrl: bool) -> Option<Self> {
        Some(match name {
            "ArrowLeft" => Self::Left,
            "ArrowRight" => Self::Right,
            "ArrowUp" => Self::Up,
            "ArrowDown" => Self::Down,
            "Home" => Self::Home,
            "End" => Self::End,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            "Backspace" => Self::Backspace,
            "Delete" => Self::Delete,
            "Enter" => Self::Enter,
            "Escape" => Self::Escape,
            "a" | "A" if ctrl => Self::SelectAll,
            _ => return None,
        })
    }
}

/// One input, as the host hands it over. Points are canvas pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum ProseInput {
    Key {
        key: ProseKey,
        shift: bool,
        ctrl: bool,
    },
    /// Committed text: typing, dictation, a plain paste.
    Text(String),
    Press {
        x: f32,
        y: f32,
        shift: bool,
        /// The platform's modifier is held: `Ctrl`, or `Cmd` on a Mac. A press
        /// with it on a link follows the link (`PROSE-UI-071`).
        follow: bool,
        now_ms: f64,
    },
    Drag {
        x: f32,
        y: f32,
    },
    Release,
}

/// A command sent, as the view keeps track of it.
enum Sent {
    /// An insertion or a deletion inside one block, in the text the view
    /// shows.
    Text(TextEdit),
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
            } => Self::Text(TextEdit {
                block_id: *block_id,
                at: *pos,
                delta: i64::from(chars(text)),
            }),
            ProseCommand::DeleteText { block_id, range } => {
                let (start, end) = (range.0.min(range.1), range.0.max(range.1));
                Self::Text(TextEdit {
                    block_id: *block_id,
                    at: start,
                    delta: -i64::from(end - start),
                })
            }
            ProseCommand::SplitBlock { .. }
            | ProseCommand::DeleteRange { .. }
            | ProseCommand::InsertBlock { .. } => Self::Barrier,
            _ => return None,
        })
    }
}

/// The last press, to count a second and a third one in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Press {
    x: f32,
    y: f32,
    at_ms: f64,
    count: u8,
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
    ) -> bool {
        if self.waiting() {
            if self.queued.len() < QUEUED_INPUTS {
                self.queued.push_back(input.clone());
            } else {
                tracing::warn!("Prose: too much input held, one is dropped");
            }
            return true;
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
            self.syncs.push_back(false);
        }
        let mut barrier = false;
        for note in sent {
            match note {
                Sent::Text(edit) => self.inflight.push_back(edit),
                Sent::Barrier => barrier = true,
            }
        }
        if barrier && send(ProseCommand::Sync { seen: self.seen }).accepted() {
            self.sent_seen = self.seen;
            self.syncs.push_back(true);
        }
        consumed
    }

    /// Whether a structural command is still unanswered.
    fn waiting(&self) -> bool {
        self.syncs.iter().any(|holds| *holds)
    }

    /// Act on the input held while a structural command was unanswered, in
    /// order, until another one holds the rest. Answers whether any was
    /// consumed.
    ///
    /// The Logic thread keeps each change until this view says it applied
    /// it, so the view says so here, once per batch of events taken.
    pub fn replay(&mut self, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> bool {
        if !self.waiting()
            && self.seen > self.sent_seen
            && send(ProseCommand::Sync { seen: self.seen }).accepted()
        {
            self.sent_seen = self.seen;
            self.syncs.push_back(false);
        }
        let mut consumed = false;
        while !self.waiting() {
            let Some(input) = self.queued.pop_front() else {
                break;
            };
            consumed |= self.handle_input(&input, send);
        }
        consumed
    }

    fn act(
        &mut self,
        input: &ProseInput,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> bool {
        match input {
            ProseInput::Text(text) => self.type_text(text, send),
            ProseInput::Key { key, shift, ctrl } => self.key(*key, *shift, *ctrl, send),
            ProseInput::Press {
                x,
                y,
                shift,
                follow,
                now_ms,
            } => {
                let target = follow
                    .then(|| self.link_at(*x, *y).map(|link| link.url.clone()))
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
                        true
                    }
                    None => self.press(*x, *y, *shift, *now_ms, send),
                }
            }
            ProseInput::Drag { x, y } => {
                self.update_drag(*x, *y);
                self.dragging.is_some()
            }
            ProseInput::Release => self.dragging.take().is_some(),
        }
    }

    /// Type a run of text at the caret (`PROSE-UI-136`, `PROSE-UI-176`).
    ///
    /// The selection goes first. The draft then takes the text only when the
    /// insertion reached the channel: a dropped command never happened.
    fn type_text(&mut self, text: &str, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> bool {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() || self.cursor.is_none() {
            return false;
        }
        if !self.take_selection(send) {
            return true;
        }
        let Some(cursor) = self.cursor else {
            return true;
        };
        let command = ProseCommand::InsertText {
            block_id: cursor.block_id,
            pos: cursor.offset,
            text: text.clone(),
        };
        if send(command).accepted() {
            self.draft.push_str(&text, &cursor.block_id, cursor.offset);
            self.advance_cursor(chars(&text));
        }
        true
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
                range: (start.1, end.1),
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
        self.draft.clear();
        self.set_caret(start);
        self.reveal_caret();
        true
    }

    fn key(
        &mut self,
        key: ProseKey,
        shift: bool,
        ctrl: bool,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> bool {
        if self.cursor.is_none() {
            return false;
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
            ProseKey::Backspace => return self.delete(false, send),
            ProseKey::Delete => return self.delete(true, send),
            ProseKey::Enter => return self.split(send),
            ProseKey::Escape => {
                self.escape();
                return true;
            }
            ProseKey::SelectAll => {
                self.select_all();
                return true;
            }
        };
        self.move_cursor(direction, shift);
        true
    }

    /// Backspace, or Delete when `forward`: the selection, else the grapheme
    /// beside the caret, else the join with the paragraph beside it.
    fn delete(&mut self, forward: bool, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> bool {
        if self.selection().is_some() {
            self.take_selection(send);
            return true;
        }
        let Some(cursor) = self.cursor else {
            return false;
        };
        let here: Spot = (cursor.block_id, cursor.offset);
        let beside = self.grapheme_beside(here, forward);
        let (command, caret) = match beside {
            Some(other) if forward => (
                ProseCommand::DeleteText {
                    block_id: here.0,
                    range: (here.1, other),
                },
                here,
            ),
            Some(other) => (
                ProseCommand::DeleteText {
                    block_id: here.0,
                    range: (other, here.1),
                },
                (here.0, other),
            ),
            None => {
                let Some(next) = self.neighbour(here.0, forward) else {
                    return true;
                };
                let edge = (next, if forward { 0 } else { self.block_len(next) });
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
            self.draft.clear();
            self.set_caret(caret);
            self.reveal_caret();
        }
        true
    }

    /// The grapheme boundary beside a place, in document offsets, reading the
    /// draft where the place is inside it. `None` at the edge of the block.
    fn grapheme_beside(&self, spot: Spot, forward: bool) -> Option<u32> {
        use unicode_segmentation::UnicodeSegmentation;
        if let Some((block, at, text)) = self.draft.shown()
            && block == spot.0
            && spot.1 >= at
            && spot.1 <= at.saturating_add(chars(&text))
        {
            let rel = spot.1 - at;
            let boundaries: Vec<u32> = text
                .grapheme_indices(true)
                .map(|(byte, _)| chars(text.get(..byte).unwrap_or("")))
                .chain(std::iter::once(chars(&text)))
                .collect();
            let found = if forward {
                boundaries.iter().find(|b| **b > rel)
            } else {
                boundaries.iter().rev().find(|b| **b < rel)
            };
            if let Some(b) = found {
                return Some(at + b);
            }
        }
        let run = self.run_of(spot.0)?;
        let laid = self.to_laid(spot.0, spot.1);
        let moved = if forward {
            run.grapheme_after(laid)
        } else {
            run.grapheme_before(laid)
        }?;
        Some(self.to_doc(spot.0, moved))
    }

    /// Enter: the selection goes, then the paragraph parts at the caret. The
    /// caret moves when the split confirms, onto the block it minted
    /// (`PROSE-UI-141`).
    fn split(&mut self, send: &mut dyn FnMut(ProseCommand) -> SendResult) -> bool {
        if !self.take_selection(send) {
            return true;
        }
        let Some(cursor) = self.cursor else {
            return true;
        };
        let command = ProseCommand::SplitBlock {
            block_id: cursor.block_id,
            pos: cursor.offset,
        };
        if send(command).accepted() {
            self.draft.clear();
        }
        true
    }

    /// A press on the canvas: a caret, a word, a paragraph or an extension,
    /// by how many presses landed in place and by Shift.
    ///
    /// It shapes nothing and scrolls nothing: what it resolved is on screen
    /// already (`PROSE-UI-157`). A layout with no page resolves nowhere and
    /// the caret stays (`PROSE-UI-156`).
    fn press(
        &mut self,
        x: f32,
        y: f32,
        shift: bool,
        now_ms: f64,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> bool {
        let layout = &self.shaped.layout;
        if layout.page_count == 0 {
            return false;
        }
        if layout.blocks.is_empty() {
            // Nothing to hit: the block comes through the command path, and
            // the caret goes to it once a layout carries it (`PROSE-UI-072`).
            let command = ProseCommand::InsertBlock {
                parent_id: None,
                after_id: None,
                init: BlockInit::Paragraph,
                style_id: None,
            };
            if send(command).accepted() {
                self.awaiting_block = true;
            }
            return true;
        }
        let Some(spot) = self.place_at(x, y) else {
            return false;
        };
        let count = match self.last_press {
            Some(last)
                if now_ms - last.at_ms <= MULTI_PRESS_MS
                    && (x - last.x).abs() <= MULTI_PRESS_PX
                    && (y - last.y).abs() <= MULTI_PRESS_PX =>
            {
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some(Press {
            x,
            y,
            at_ms: now_ms,
            count,
        });
        match count {
            1 if shift => {
                let anchor = self
                    .cursor
                    .as_ref()
                    .map_or(spot, |c| c.anchor.unwrap_or((c.block_id, c.offset)));
                self.start_drag(anchor);
                self.update_drag(x, y);
            }
            1 => {
                // A plain press collapses the selection (`PROSE-UI-154`).
                self.set_caret(spot);
                self.start_drag(spot);
            }
            2 => self.select_around(spot, false),
            _ => self.select_around(spot, true),
        }
        true
    }

    /// A layout was taken: a caret a press asked a block for goes to the
    /// first one now (`PROSE-UI-072`).
    pub(crate) fn settle_awaited_block(&mut self) {
        if !self.awaiting_block {
            return;
        }
        let first = self
            .shaped
            .layout
            .blocks
            .iter()
            .find(|block| self.shaped.blocks.contains_key(&block.block_id))
            .map(|block| crate::events::block_id(block.block_id));
        if let Some(block) = first {
            self.awaiting_block = false;
            self.set_caret((block, 0));
        }
    }
}
