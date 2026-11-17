//! A conversation fed to a view, for the browser tests of a host. The host
//! names no Murmure event, so the event is built here.

use std::rc::Rc;

use crate::message::UiEvent;
use crate::message::{MessageView, MurmureEvent};
use ambre_bus::conduit::{Sender, local};
use ambre_bus::{Authored, Instant};
use ambre_canvas::{TextInput, ThemeControl};
use ambre_core::id::{Channel, Id, Message, Segment, User};

use crate::MurmurePane;
use crate::ui::drafts::DraftShelf;
use crate::ui::view::{MurmureView, PageReads};
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
        name_of: Rc::new(|_| ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").ok()),
        time_of: Rc::new(|_| "t".to_owned()),
        now: Rc::new(|| Instant::<PageLoad>::new(0.0)),
        wall_clock: Rc::new(|| Instant::<Authored>::new(0)),
        motion: Rc::new(|| crate::ui::pictures::Motion::Full),
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
        call: Rc::new(|| None),
        press_call: Rc::new(|_| {}),
    };
    let (events, from_logic) =
        local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
    let view = MurmureView::new(
        Some(ambre_bus::Feed::direct(from_logic)),
        None,
        ambre_canvas::BusyPublisher::new(),
        reads,
        size,
        ThemeControl::default(),
    );
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
            edited: crate::message::Revision::Original,
            deleted: crate::message::Deletion::Live,
            thread: None,
            also_in_thread: None,
            call: None,
        })
        .collect();
    let segment = Id::<Segment>::mint();
    let event = MurmureEvent::Messages { segment, messages };
    let _ = events.try_send(UiEvent::Channel { channel_id, event });
    (view, events)
}
