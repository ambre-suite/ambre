// ambre-toile/src/chrome/relay.rs
// ============================================================================
// What Toile's chrome learns from its events, and what its panels ask of the
// board (`ARCH-018`).
//
// The launcher hands every event from the Logic thread here and names no
// variant. The status band and the presenter panel read the signals this
// fills. The canvas view gets the same events on its own queue, for the
// highlight, the camera glide and the overlay pass. It publishes what it keeps
// of them, the find results and the present mode (`UI-THREAD-003`).
//
// The other way, a panel's intent becomes a `ToileCommand` here. The launcher
// only sends it to the routed document.
// ============================================================================

use ambre_bus::app::toile::{
    Broadcast, ElementId, SlidePosition, StatsRequestKind, ToileCommand, ToileEvent,
};
use ambre_bus::{AppCommand, AppEvent, UiEvent, UserId};
use leptos::prelude::*;

use super::find::FindRequest;
use crate::ui::presenter::PresenterIntent;
use crate::ui::status::SelStats;

/// What one of Toile's panels asks of the board.
pub enum ToileIntent {
    /// Search the whole board, off-screen matches included (`toile-find`).
    Find(FindRequest),
    /// Drive the presentation from the presenter panel (`toile-presentation`).
    Present(PresenterIntent),
}

/// The chrome signals Toile's events fill. Each is `Copy`, so the relay is too.
#[derive(Clone, Copy)]
pub struct ToileRelay {
    /// The selection's count and bounds, for the status band. The find
    /// results and the present mode are the view's to publish: it keeps both
    /// (`UI-THREAD-003`).
    pub sel_stats: RwSignal<Option<SelStats>>,
    /// The slide on screen and the deck's length.
    pub presenter_slide: RwSignal<SlidePosition>,
    /// The speaker notes of the slide on screen.
    pub presenter_notes: RwSignal<String>,
    /// The frame those notes belong to.
    pub presenter_frame: RwSignal<Option<ElementId>>,
    /// Whether this client broadcasts its playback position.
    pub broadcasting: RwSignal<Broadcast>,
    /// The presenter this client follows, if any.
    pub following: RwSignal<Option<UserId>>,
    /// A peer presenting now, whom the user may follow.
    pub presenter_peer: RwSignal<Option<UserId>>,
}

impl ToileRelay {
    /// The command `intent` becomes. `None` for a note edit with no slide on
    /// screen, which has no frame to land on.
    pub fn command(&self, intent: ToileIntent) -> Option<AppCommand> {
        let command = match intent {
            ToileIntent::Find(q) => ToileCommand::Find {
                query: q.query,
                from: q.from,
                direction: q.direction,
            },
            ToileIntent::Present(intent) => match intent {
                PresenterIntent::Prev => ToileCommand::PrevSlide,
                PresenterIntent::Next => ToileCommand::NextSlide,
                PresenterIntent::Exit => ToileCommand::ExitPresent,
                // A note edit lands on the frame the last slide event named.
                PresenterIntent::EditNotes { base, delta } => ToileCommand::EditNotes {
                    frame: self.presenter_frame.get_untracked()?,
                    base,
                    delta,
                },
                PresenterIntent::ToggleBroadcast(b) => ToileCommand::SetPresentBroadcast(b),
                PresenterIntent::Follow(who) => ToileCommand::FollowPresenter(who),
            },
        };
        Some(AppCommand::Toile(command))
    }

    /// Fill the chrome from `ev`. Any event not Toile's is a no-op.
    pub fn absorb(&self, ev: &UiEvent) {
        let UiEvent::Viewport {
            event: AppEvent::Toile(te),
            ..
        } = ev
        else {
            return;
        };
        match te {
            // A fit or a find focus also answers `SelectionStats`, and its
            // bounds belong to that request. Only the band's own request
            // updates the band (`TOILE-UI-108`).
            ToileEvent::SelectionStats(s) if s.kind == StatsRequestKind::Band => {
                self.sel_stats
                    .set(Some(SelStats::from_reply(s.count, s.bounds)));
            }
            ToileEvent::SlideChanged(c) => {
                self.presenter_slide.set(c.position);
                self.presenter_notes.set(c.notes.clone());
                self.presenter_frame.set(c.frame.clone());
            }
            // A peer or a history step wrote into the note on screen
            // (`TOILE-UI-316`). The slide event alone seeds the editor, so
            // without this the writing would stay invisible until the slide
            // changed.
            ToileEvent::NotesChanged { frame, notes }
                if self.presenter_frame.get_untracked().as_ref() == Some(frame) =>
            {
                self.presenter_notes.set(notes.clone());
            }
            ToileEvent::PresentBroadcastChanged(b) => self.broadcasting.set(*b),
            ToileEvent::FollowingChanged(who) => self.following.set(*who),
            // The event is the whole live set (`PRESENCE-009`), so a presenter
            // who left it stops being offered to follow.
            ToileEvent::PeerPresence(peers) => self.presenter_peer.set(
                peers
                    .iter()
                    .find(|p| p.presence.present_slide.is_some())
                    .map(|p| p.user_id),
            ),
            _ => {}
        }
    }
}
