// ambre-app/src/relay.rs
// ============================================================================
// The decisions of the per-frame relay (`ambre-spec-architecture.md`
// section 5). The relay closure in `lib.rs` reads the queues, hands each
// event to this module, and applies what this module answers. This module
// names no DOM, no `web_sys` and no signal. So it compiles on the host, and
// the native tier runs its tests.
//
// The child fan-out holds the senders of the embedded views. Those are
// `ambre_bus::conduit` queues, which run on the host as well. The decision
// to defer an event depends on what the queue takes, so the fan-out keeps
// the senders rather than answering a list of sends.
// ============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use ambre_bus::conduit::{Sender, TrySendError};
use ambre_core::id::{Document, Id};
use ambre_logic::app::UiEvent;

/// How many events may wait for one child before the oldest are dropped.
///
/// A child is pumped once per frame, so a backlog only forms on a burst larger
/// than the queue itself. Past this the client is saturated for good and no
/// amount of memory fixes it. The real answer is the busy signal `BUS-016`
/// prescribes, which lands with its own Story. Until then the bound is what
/// keeps a stalled child from growing without limit, and it says so out loud.
// A `u32` widens into a `usize` on every target the workspace builds.
pub(crate) const CHILD_BACKLOG_LIMIT: usize = crate::CHANNEL_CAPACITY.get() as usize;

/// The child fan-out: every live view of a nested document, by document, and
/// what each child's queue could not take yet.
///
/// The child factory registers a view, and the relay routes the events. The
/// two share one fan-out behind an `Rc`, on the page's main thread alone.
#[derive(Default)]
pub(crate) struct ChildFanOut {
    /// The senders of every live view, by document. A list rather than a
    /// slot -- two embeds of one document are two views (the Toile embeds spec).
    routes: RefCell<HashMap<Id<Document>, Vec<Sender<UiEvent>>>>,
    /// What a child's queue could not take yet, oldest first, by document
    /// (the Toile embeds spec).
    backlog: RefCell<HashMap<Id<Document>, VecDeque<UiEvent>>>,
}

impl ChildFanOut {
    /// Add the queue of one more view of `id`.
    pub(crate) fn register(&self, id: Id<Document>, tx: Sender<UiEvent>) {
        self.routes.borrow_mut().entry(id).or_default().push(tx);
    }

    /// Fan a nested child's event out to its own view queues, or give the
    /// event back when no child holds a route for its document.
    ///
    /// A child's events must not drive the parent's formula bar, zoom chip or
    /// status band. Only a document that is a live child is diverted. The
    /// top-level document's events, and untagged service events, come back
    /// unchanged.
    pub(crate) fn divert(&self, ev: UiEvent) -> Option<UiEvent> {
        match self.child_of(&ev) {
            Some(id) => {
                self.send(id, ev);
                None
            }
            None => Some(ev),
        }
    }

    /// Re-offer every deferred event, oldest first, stopping at the first one a
    /// queue still refuses so the order is kept (the Toile embeds spec).
    pub(crate) fn flush(&self) {
        let ids: Vec<Id<Document>> = self.backlog.borrow().keys().copied().collect();
        for id in ids {
            loop {
                let Some(ev) = self
                    .backlog
                    .borrow()
                    .get(&id)
                    .and_then(|q| q.front().cloned())
                else {
                    break;
                };
                if !self.offer(id, &ev) {
                    break;
                }
                self.backlog
                    .borrow_mut()
                    .get_mut(&id)
                    .map(VecDeque::pop_front);
            }
            if self
                .backlog
                .borrow()
                .get(&id)
                .is_some_and(VecDeque::is_empty)
            {
                self.backlog.borrow_mut().remove(&id);
            }
        }
    }

    /// The nested document `ev` belongs to, when it is one the fan-out holds a
    /// route for. Such an event goes to the child's queues and never to the
    /// board's own stream or chrome (the Toile embeds spec). A route left with no view
    /// still counts.
    fn child_of(&self, ev: &UiEvent) -> Option<Id<Document>> {
        ev.routed_document_id()
            .filter(|id| self.routes.borrow().contains_key(id))
    }

    /// Hand `ev` to every live view of `id`, deferring what a full queue
    /// refuses (the Toile embeds spec).
    ///
    /// The old path dropped it: a `Full` fell through the disconnected test and
    /// then past the `continue`. The event reached neither the child nor the
    /// board. It is a CRDT delta, so the child's view then sat behind its own
    /// document until something else redrew it from scratch.
    ///
    /// A document with anything already waiting takes the new event straight to
    /// the back of its queue. A child must not see a later delta before an
    /// earlier one.
    fn send(&self, id: Id<Document>, ev: UiEvent) {
        if self
            .backlog
            .borrow()
            .get(&id)
            .is_some_and(|q| !q.is_empty())
        {
            self.defer(id, ev);
            return;
        }
        if !self.offer(id, &ev) {
            self.defer(id, ev);
        }
    }

    /// Offer `ev` to each live view of `id`, dropping the senders whose view is
    /// gone. Reports whether every remaining one took it; `false` means at least
    /// one queue was full and the event must be re-offered.
    fn offer(&self, id: Id<Document>, ev: &UiEvent) -> bool {
        let mut routes = self.routes.borrow_mut();
        let Some(senders) = routes.get_mut(&id) else {
            // No view left at all: the document is no longer shown, so the
            // event has nowhere to go and nothing to be behind.
            return true;
        };
        let mut all_took_it = true;
        senders.retain(|tx| match tx.try_send(ev.clone()) {
            Ok(()) => true,
            // The child view was dropped; reclaim its slot.
            Err(TrySendError::Disconnected(_)) => false,
            Err(TrySendError::Full(_)) => {
                all_took_it = false;
                true
            }
        });
        // The entry stays even with no sender left in it, and that emptiness is
        // what holds the Toile embeds spec, section 3. A child view is dropped the frame its embed
        // leaves the culled set, while its session stays open for the whole
        // close debounce. An event arriving in that window used to find no route
        // and fall through to the board's own stream and chrome mirrors. A Toile
        // nested in a Toile emits exactly the event the parent board would then
        // apply to itself. An empty entry absorbs it instead, and the next view
        // of that document pushes its sender back into the same slot.
        //
        // What is left is a few dozen bytes per document embedded at least once
        // in this session, which nothing reclaims. Bounded by the boards a user
        // opens, and the trade the other way is a delta applied to the wrong
        // document.
        all_took_it
    }

    /// Queue `ev` for `id`, oldest first, and say so when the bound forces a
    /// drop.
    fn defer(&self, id: Id<Document>, ev: UiEvent) {
        let mut backlog = self.backlog.borrow_mut();
        let queue = backlog.entry(id).or_default();
        queue.push_back(ev);
        if queue.len() > CHILD_BACKLOG_LIMIT {
            queue.pop_front();
            tracing::error!("embedded document {id}: backlog full, an event was dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CHILD_BACKLOG_LIMIT, ChildFanOut};
    use crate::ChromeStatus;
    use ambre_bus::conduit;
    use ambre_core::LinkState;
    use ambre_core::id::{Document, Id};
    use ambre_logic::app::UiEvent;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An event a child's stream carries. Its `connected` flag tells two
    /// events apart, so the order they arrive in can be read back.
    fn status(document_id: Id<Document>, connected: LinkState) -> UiEvent {
        UiEvent::ConnectionStatus {
            document_id,
            connected,
        }
    }

    fn connected(ev: UiEvent) -> LinkState {
        match ev {
            UiEvent::ConnectionStatus { connected, .. } => connected,
            _ => panic!("a foreign event reached the child"),
        }
    }

    /// A queue of `N` slots, so a test fills it with a few events.
    fn queue<const N: u32>() -> (conduit::Sender<UiEvent>, conduit::Receiver<UiEvent>) {
        conduit::local::<UiEvent>(ambre_core::Bounded::<conduit::ChannelSlots>::of::<N>())
    }

    /// Counts the error events emitted while it is the default subscriber.
    struct Errors(Arc<AtomicUsize>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Errors {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() == tracing::Level::ERROR {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// The Toile embeds spec, section 5: what a full queue refuses waits, and is re-offered in
    /// order and ahead of the fresh stream. A later event never overtakes it.
    #[test]
    fn a_refused_event_waits_and_arrives_in_order() {
        let fan_out = ChildFanOut::default();
        let doc = Id::<Document>::mint();
        let (tx, rx) = queue::<1>();
        fan_out.register(doc, tx);

        for link in [
            LinkState::Connected,
            LinkState::Disconnected,
            LinkState::Connected,
        ] {
            assert!(fan_out.divert(status(doc, link)).is_none());
        }
        assert_eq!(
            fan_out.backlog.borrow()[&doc].len(),
            2,
            "a refused event was dropped"
        );

        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            seen.push(connected(ev));
            fan_out.flush();
        }
        assert_eq!(
            seen,
            [
                LinkState::Connected,
                LinkState::Disconnected,
                LinkState::Connected
            ]
        );
        assert!(
            fan_out.backlog.borrow().is_empty(),
            "a drained backlog lingers"
        );
    }

    /// The Toile embeds spec, section 5: a backlog past its bound drops its oldest event, and
    /// says so. Nothing is dropped in silence.
    #[test]
    fn an_overflowing_backlog_says_what_it_dropped() {
        let fan_out = ChildFanOut::default();
        let doc = Id::<Document>::mint();
        let (tx, _rx) = queue::<1>();
        fan_out.register(doc, tx);
        let errors = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::registry(),
            Errors(Arc::clone(&errors)),
        );
        tracing::subscriber::with_default(subscriber, || {
            // One taken by the queue, the bound waiting, and two past it.
            for _ in 0..CHILD_BACKLOG_LIMIT + 3 {
                assert!(fan_out.divert(status(doc, LinkState::Connected)).is_none());
            }
        });
        assert_eq!(fan_out.backlog.borrow()[&doc].len(), CHILD_BACKLOG_LIMIT);
        assert_eq!(
            errors.load(Ordering::Relaxed),
            2,
            "an event was dropped in silence"
        );
    }

    /// The Toile embeds spec, section 3: a nested document's event stays with its children, even
    /// once its last view is gone. The route outlives the view, so the event is
    /// absorbed instead of falling through to the board's own stream.
    #[test]
    fn a_childs_event_never_reaches_the_board() {
        let fan_out = ChildFanOut::default();
        let child = Id::<Document>::mint();
        let board = Id::<Document>::mint();
        let (tx, rx) = queue::<4>();
        fan_out.register(child, tx);
        assert!(
            fan_out
                .divert(status(board, LinkState::Connected))
                .is_some(),
            "the board's own event was diverted"
        );
        assert!(
            fan_out
                .divert(status(child, LinkState::Connected))
                .is_none()
        );
        assert_eq!(rx.try_recv().map(connected), Ok(LinkState::Connected));

        // The embed leaves the frame: its view, and with it the receiver, drop.
        drop(rx);
        assert!(
            fan_out
                .divert(status(child, LinkState::Connected))
                .is_none()
        );
        assert!(
            fan_out.routes.borrow()[&child].is_empty(),
            "the dead view kept its slot"
        );
        assert!(
            fan_out
                .divert(status(child, LinkState::Disconnected))
                .is_none(),
            "an event of a document with no view fell through to the board"
        );
    }

    /// The Toile embeds spec, section 3: two embeds of one document are two views, and both see
    /// each event of that document.
    #[test]
    fn two_views_of_one_document_each_take_its_event() {
        let fan_out = ChildFanOut::default();
        let doc = Id::<Document>::mint();
        let (first, first_rx) = queue::<4>();
        let (second, second_rx) = queue::<4>();
        fan_out.register(doc, first);
        fan_out.register(doc, second);

        assert!(fan_out.divert(status(doc, LinkState::Connected)).is_none());

        assert_eq!(first_rx.try_recv().map(connected), Ok(LinkState::Connected));
        assert_eq!(
            second_rx.try_recv().map(connected),
            Ok(LinkState::Connected),
            "the second registration evicted the first"
        );
    }

    /// `CHROME-104`: a failed open of the active document lowers its loading
    /// screen and raises a notice. A document already left keeps both down.
    #[test]
    fn a_failed_open_ends_the_loading_screen() {
        let active = Id::<Document>::mint();
        let loading = ChromeStatus {
            hydrating: true,
            connected: LinkState::Connected,
        };

        let mut left = loading;
        assert_eq!(left.open_failed(Some(active), Id::<Document>::mint()), None);
        assert!(
            left.hydrating,
            "another document's failure lowered the screen"
        );

        let mut failed = loading;
        assert!(failed.open_failed(Some(active), active).is_some());
        assert!(!failed.hydrating, "the loading screen stayed up");
    }
}
