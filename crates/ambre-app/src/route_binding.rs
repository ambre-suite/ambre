//! The SPA route binding, polled each frame on the main thread.
//!
//! The `!Send` canvas handle lives on this thread, so the binding stays out
//! of the Leptos tree. The client URL is the single source of truth:
//! `leptos_router` pushState navigations and browser back/forward both land
//! here. On a change the binding closes the previous document, opens the
//! next, and points the persistent canvas at it. It then gates the rAF
//! render: paint the document, or freeze it behind the landing overlay. It
//! publishes the active document, so every command trigger targets it.

use ambre_bus::DocType;
use ambre_core::id::{Document, Id};
use ambre_logic::app::LogicCommand;
use leptos::prelude::*;

use crate::host::DocSwitcher;
use crate::outbox::ShellOutbox;
use crate::route::{self, Route};
use crate::{ChromeStatus, Slot};

/// The route binding: what a route change drives, and the route last bound.
pub(crate) struct RouteBinding {
    /// The canvas document-swap handle, filled once the canvas boots.
    switcher: Slot<DocSwitcher>,
    to_logic: ShellOutbox,
    raf: ambre_canvas::RafControl,
    current_doc: RwSignal<Option<Id<Document>>>,
    chrome_status: RwSignal<ChromeStatus>,
    prev: Option<Route>,
}

impl RouteBinding {
    /// A binding that has bound no route yet, so the first poll binds one.
    pub(crate) fn new(
        switcher: Slot<DocSwitcher>,
        to_logic: ShellOutbox,
        raf: ambre_canvas::RafControl,
        current_doc: RwSignal<Option<Id<Document>>>,
        chrome_status: RwSignal<ChromeStatus>,
    ) -> Self {
        Self {
            switcher,
            to_logic,
            raf,
            current_doc,
            chrome_status,
            prev: None,
        }
    }

    /// Read the client URL, bind a changed route, and answer the routed
    /// document, or `None` on the landing route.
    pub(crate) fn poll(&mut self) -> Option<(DocType, Id<Document>)> {
        let cur_route = route::current();
        let cur_doc = cur_route.doc();
        if let Some(step) = route::step(self.prev.as_ref(), &cur_route) {
            // A Murmure link pins its inviter for every Murmure open of the
            // session, ahead of the spaces the page asks for (`SYNC-236`).
            if let Some(inviter) = route::current_fragment()
                .and_then(|fragment| route::murmure_inviter(&cur_route, &fragment))
            {
                self.to_logic
                    .send(LogicCommand::PinMurmureInviter { inviter });
            }
            if let Some(document_id) = step.close {
                self.to_logic
                    .send(LogicCommand::CloseDocument { document_id });
            }
            if let Some((doc_type, document_id)) = step.open {
                // An invite link pins its inviter before the open it drives,
                // so the Sync worker checks that open against it (`SYNC-209`).
                if let Some(inviter) = route::current_inviter() {
                    self.to_logic.send(LogicCommand::PinInviter {
                        document_id,
                        inviter,
                    });
                }
                self.to_logic.send(LogicCommand::OpenDocument {
                    document_id,
                    doc_type,
                });
            }
            if step.close.is_some() || step.open.is_some() {
                self.current_doc.set(step.show);
                // Arm the loading overlay when entering a document; clear it on
                // the landing route. The switchboard clears `hydrating` on the
                // matching `DocumentReady`.
                self.chrome_status.update(|s| {
                    s.hydrating = cur_doc.is_some();
                    if cur_doc.is_none() {
                        s.connected = ambre_core::LinkState::Disconnected;
                    }
                });
            }
            if let Some(document_id) = step.show
                && let Some(sw) = self.switcher.borrow().as_ref()
            {
                sw.set_document(document_id);
            }
            self.raf.set_cover(route::cover(&cur_route));
            self.prev = Some(cur_route);
        }
        cur_doc
    }
}
