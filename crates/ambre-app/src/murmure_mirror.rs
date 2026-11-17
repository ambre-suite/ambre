//! The Murmure chrome, fed from the Logic thread's events.
//!
//! The Murmure chrome fills its pane and its sidebar from each event, and
//! answers the commands they call for. It also decides which message raises
//! a system notification. On a Murmure page, an interval refreshes this
//! user's presence on each space.

use std::cell::Cell;
use std::rc::Rc;

use ambre_bus::{Authored, Embed, Instant};
use ambre_core::id::{Channel, Id};
use ambre_logic::app::{AppCommand, UiEvent};
use ambre_murmure::MessageRef;
use ambre_murmure::chrome::MurmureRelay;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::notify;
use crate::outbox::ShellOutbox;

/// The Murmure chrome, the outbox its commands leave by, and the state of
/// its system notifications.
pub(crate) struct MurmureMirror {
    chrome: MurmureRelay,
    to_logic: ShellOutbox,
    /// Whether the page asked for the notification permission. The
    /// permission is asked once per page.
    notice_asked: Rc<Cell<notify::Question>>,
    /// A click on a notification opens its channel at its message (the
    /// Murmure notifications spec).
    open_noticed: Rc<dyn Fn(Id<Channel>, Option<MessageRef>)>,
}

impl MurmureMirror {
    /// A mirror that has asked no permission yet.
    pub(crate) fn new(chrome: MurmureRelay, to_logic: ShellOutbox) -> Self {
        let open_noticed: Rc<dyn Fn(Id<Channel>, Option<MessageRef>)> = {
            let tx = to_logic.clone();
            Rc::new(move |channel, at| {
                if let Some(command) = chrome.open_at(channel, at) {
                    tx.send(command.map(AppCommand::embed));
                }
            })
        };
        Self {
            chrome,
            to_logic,
            notice_asked: Rc::new(Cell::new(notify::Question::Unasked)),
            open_noticed,
        }
    }

    /// Let a call of a direct message ring this page. Only a Murmure page shows the ring
    /// alert, so only a Murmure page rings (the Murmure calls spec, the ring).
    pub(crate) fn ring_here(&self) {
        crate::call_ring::install(self.to_logic.clone(), self.chrome);
    }

    /// Refresh this user's presence on each space once per refresh period
    /// (the Murmure members spec). The tick reaches the shown channel too
    /// (the Murmure typing spec). An interval, not the frame loop: a hidden
    /// tab pauses its frames, yet holds its spaces open. The interval lives
    /// as long as the page.
    pub(crate) fn keep_presence(&self) {
        let chrome = self.chrome;
        let presence_tx = self.to_logic.clone();
        let refresh = Closure::<dyn FnMut()>::new(move || {
            let now = Instant::<Authored>::new(js_sys::Date::now() as u64);
            // The tick is lossy and the next period offers it again. The
            // presence refresh of a space is not, so the outbox holds it.
            for command in chrome.refresh_presence(now) {
                presence_tx.offer(command.map(AppCommand::embed));
            }
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                refresh.as_ref().unchecked_ref(),
                ambre_murmure::REFRESH_PERIOD.to_timer_i32(),
            );
        }
        refresh.forget();
    }

    /// Fill the Murmure pane and sidebar from one event, send the commands
    /// it calls for, and raise its system notification, if any.
    pub(crate) fn mirror(&self, ev: &UiEvent) {
        for command in self.chrome.absorb(ev) {
            self.to_logic.send(command.map(AppCommand::embed));
        }
        // A call message of a direct message rings the page (the Murmure calls spec, the
        // ring).
        if let UiEvent::CallCue { channel_id, cue } = ev {
            crate::call_ring::heard(&self.chrome, *channel_id, *cue);
        }
        if let Some(notice) = self.chrome.notice(
            ev,
            ambre_murmure::chrome::PageVisibility::visible_if(notify::page_visible()),
        ) {
            notify::deliver(notice, &self.notice_asked, Rc::clone(&self.open_noticed));
        }
    }
}
