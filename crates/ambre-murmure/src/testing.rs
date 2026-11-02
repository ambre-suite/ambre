//! A conversation fed to a view, for the browser tests of a host. The host
//! names no Murmure event, so the event is built here.

use std::rc::Rc;

use ambre_bus::conduit::{Sender, local};
use ambre_bus::{Authored, Instant, MessageView, MurmureEvent, UiEvent};
use ambre_canvas::{TextInput, ThemeControl};
use ambre_core::id::{Channel, Id, Message, Segment, User};

use crate::MurmurePane;
use crate::drafts::DraftShelf;
use crate::view::{MurmureView, PageReads};
use ambre_core::PageLoad;

/// The Logic thread's word that `watching` watch `channel_id`, and no one
/// types in it.
pub fn watchers(channel_id: Id<Channel>, watching: Vec<Id<User>>) -> UiEvent {
    UiEvent::Channel {
        channel_id,
        event: MurmureEvent::Watchers {
            watching,
            typing: Vec::new(),
            in_threads: Vec::new(),
        },
    }
}

/// A channel view of `size` whose messages read `texts`, each by "Ada" at
/// "t", and the queue that fed it. The messages land on the next update.
pub fn channel_of(
    texts: &[&str],
    size: ambre_core::Size<ambre_core::Px<ambre_core::CanvasCss>>,
) -> (MurmureView, Sender<UiEvent>) {
    let channel_id = Id::<Channel>::mint();
    let reads = PageReads {
        pane: Rc::new(move || MurmurePane::Channel {
            channel: channel_id,
            at: None,
        }),
        name_of: Rc::new(|_| ambre_core::DisplayName::new("Ada").ok()),
        time_of: Rc::new(|_| "t".to_owned()),
        now: Rc::new(|| Instant::<PageLoad>::new(0.0)),
        wall_clock: Rc::new(|| Instant::<Authored>::new(0)),
        motion: Rc::new(|| crate::pictures::Motion::Full),
        announce: Rc::new(|_| {}),
        act_on: Rc::new(|_| {}),
        take_edit: Rc::new(|_| None),
        back: Rc::new(|| {}),
        me: Rc::new(|| None),
        pipe: TextInput::detached(),
        members_of: Rc::new(|_| Vec::new()),
        show_mentions: Rc::new(|_, _| {}),
        take_pick: Rc::new(|_| None),
        show_bar: Rc::new(|_, _| {}),
        take_press: Rc::new(|_| None),
        take_returned: Rc::new(|_| None),
        open_link: Rc::new(|_| {}),
        drafts: DraftShelf::default(),
        ask_emoji: Rc::new(|_| {}),
        take_emoji: Rc::new(|_| None),
        take_files: Rc::new(Vec::new),
        save_file: Rc::new(|_, _| {}),
        warn: Rc::new(|_| {}),
    };
    let (events, from_logic) = local(ambre_bus::conduit::ChannelCapacity::new(8));
    let view = MurmureView::new(Some(from_logic), None, reads, size, ThemeControl::default());
    let author = Id::<User>::mint();
    let messages = texts
        .iter()
        .enumerate()
        .map(|(i, text)| MessageView {
            id: Id::<Message>::mint(),
            author,
            ts: Instant::<Authored>::new(i as u64),
            text: (*text).to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            notice: None,
            edited: ambre_bus::Revision::Original,
            deleted: ambre_bus::Deletion::Live,
            thread: None,
            also_in_thread: None,
        })
        .collect();
    let segment = Id::<Segment>::mint();
    let event = MurmureEvent::Messages { segment, messages };
    let _ = events.try_send(UiEvent::Channel { channel_id, event });
    (view, events)
}
