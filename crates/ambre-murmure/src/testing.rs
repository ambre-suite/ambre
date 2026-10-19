//! A conversation fed to a view, for the browser tests of a host. The host
//! names no Murmure event, so the event is built here.

use std::rc::Rc;

use ambre_bus::conduit::{Sender, local};
use ambre_bus::{
    ChannelId, MessageId, MessageView, MurmureEvent, SegmentId, Timestamp, UiEvent, UserId,
};
use ambre_canvas::ThemeControl;

use crate::MurmurePane;
use crate::view::{MurmureView, PageReads};

/// A channel view of `size` whose messages read `texts`, each by "Ada" at
/// "t", and the queue that fed it. The messages land on the next update.
pub fn channel_of(texts: &[&str], size: (f64, f64)) -> (MurmureView, Sender<UiEvent>) {
    let channel_id = ChannelId::new();
    let reads = PageReads {
        pane: Rc::new(move || MurmurePane::Channel {
            channel: channel_id,
            at: None,
        }),
        name_of: Rc::new(|_| Some("Ada".to_owned())),
        time_of: Rc::new(|_| "t".to_owned()),
        now_ms: Rc::new(|| 0.0),
        announce: Rc::new(|_| {}),
        act_on: Rc::new(|_| {}),
    };
    let (events, from_logic) = local(8);
    let view = MurmureView::new(Some(from_logic), None, reads, size, ThemeControl::default());
    let author = UserId::new();
    let messages = texts
        .iter()
        .enumerate()
        .map(|(i, text)| MessageView {
            id: MessageId::new(),
            author,
            ts: Timestamp(i as u64),
            text: (*text).to_owned(),
            mentions: Vec::new(),
            reactions: Vec::new(),
            notice: None,
            edited: false,
            deleted: false,
            thread: None,
            also_in_thread: None,
        })
        .collect();
    let segment = SegmentId::new();
    let event = MurmureEvent::Messages { segment, messages };
    let _ = events.try_send(UiEvent::Channel { channel_id, event });
    (view, events)
}
