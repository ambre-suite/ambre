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
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{DocumentSummary, MemberInfo, RestCommand, RestEvent};
use ambre_chrome::ToastKind;
use ambre_core::id::{Channel, Document, Id, User};
use ambre_core::{
    Credential, DisplayName, OidcState, Opaque, Payload, PlaintextContainer, ProviderUrls,
    Validated,
};
use ambre_logic::app::{LogicCommand, UiEvent};

use crate::ChromeStatus;
use crate::tofu::{self, TofuPrompt};

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

/// What a page boots, as far as the relay reads it: a Murmure page asks its
/// channels and their activity once signed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    /// A document application, or the launcher's home.
    Document,
    /// The Murmure page.
    Murmure,
}

/// Whether the relay asked the document list of this page yet.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Listing {
    #[default]
    Unasked,
    Asked,
}

/// What the relay reads of the shell beside the event: the routed document,
/// and the header status as the shell holds it now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown {
    /// The currently routed document, or `None` on the launcher's home.
    pub(crate) current: Option<Id<Document>>,
    /// The header status before the event.
    pub(crate) status: ChromeStatus,
}

/// What the relay needs of the Murmure members, to file a fetched key under
/// what it is for.
pub(crate) trait Roster {
    /// The display name of a member this page knows, for the prompt.
    fn member_name(&self, user: Id<User>) -> Option<Validated<DisplayName>>;
    /// The channel `user` is being invited into, taken so that it answers one
    /// fetched key alone. `None` when no invitation waits for that member.
    fn take_invite(&self, user: Id<User>) -> Option<Id<Channel>>;
}

/// A notice the relay raises, with its severity.
pub(crate) struct Notice {
    /// The severity, which picks the toast's accent.
    pub(crate) kind: ToastKind,
    /// The words the user reads.
    pub(crate) text: String,
}

impl Notice {
    fn new(kind: ToastKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// One decision of the relay on a service event. The wiring applies each one
/// in the order the relay answers them.
pub(crate) enum Effect<'e> {
    /// Raise a toast.
    Notice(Notice),
    /// Send a command to the Logic thread through the launcher's outbox.
    Send(LogicCommand),
    /// Send the commands a Murmure page boots with.
    BootMurmure,
    /// The session is ready: show the user and file the session on the
    /// launcher's stores.
    SignedIn {
        user_id: Id<User>,
        display_name: &'e Validated<DisplayName>,
    },
    /// The worker ended the session: once the drafts are written, the page
    /// boots home afresh (`CHROME-108`).
    LeaveSession,
    /// Send the browser to the provider's address (`CHROME-095`).
    FollowProvider {
        url: &'e Opaque<ProviderUrls>,
        state: &'e Credential<OidcState>,
    },
    /// Replace the landing's document list.
    ShowDocuments(&'e [DocumentSummary]),
    /// Drop one card from the landing's document list.
    DropDocument(Id<Document>),
    /// Leave the document the view is on for the launcher's home.
    GoHome,
    /// Replace the share dialog's member list.
    ShowMembers(&'e [MemberInfo]),
    /// Compare a fetched key with the pinned one (`CHROME-050`).
    Classify(TofuPrompt),
    /// Replace the header status.
    Status(ChromeStatus),
}

/// The relay's own state across frames: the page it runs on, and whether it
/// asked the document list yet.
pub(crate) struct Relay {
    page: Page,
    listing: Listing,
}

impl Relay {
    /// The relay of a page that boots `page`.
    pub(crate) fn new(page: Page) -> Self {
        Self {
            page,
            listing: Listing::Unasked,
        }
    }

    /// What a service event calls for, in the order the wiring applies it.
    ///
    /// A content event calls for nothing here: it goes on to the view. The
    /// answer borrows from `ev`, so the wiring applies it before the event
    /// moves on.
    pub(crate) fn service<'e>(
        &mut self,
        ev: &'e UiEvent,
        shown: Shown,
        roster: &dyn Roster,
    ) -> Vec<Effect<'e>> {
        let mut effects = Vec::new();
        match ev {
            // Non-fatal sync warnings surface as a transient warning toast.
            // They never reach the fatal path. The session keeps running and
            // the CRDT reconverges: a dropped/quarantined update is re-fetched
            // via RequestMissing once its parent snapshot lands.
            UiEvent::SyncWarning { code, .. } => effects.push(Effect::Notice(Notice::new(
                ToastKind::Warning,
                sync_warning_message(*code),
            ))),
            // The relay moved to a newer wire version: syncing stays off
            // until the page reloads onto the new client (`SYNC-137`).
            UiEvent::UpdateRequired => effects.push(Effect::Notice(Notice::new(
                ToastKind::Warning,
                "A new version of Ambre is available. Reload the page to keep syncing.",
            ))),
            UiEvent::Rest { evt, .. } => self.rest(evt, shown, roster, &mut effects),
            // Chrome status indicator (`-chrome.md` 2.G): mirror the active
            // document's WebSocket link into the header label. Ignored for a
            // non-active document (a stale push during a fast doc switch).
            UiEvent::ConnectionStatus {
                document_id,
                connected,
            } if shown.current == Some(*document_id) => {
                effects.push(Effect::Status(ChromeStatus {
                    connected: *connected,
                    ..shown.status
                }));
            }
            // Initial hydration finished: dismiss the loading overlay. Guarded
            // on the active document so a late ready for a doc we already left
            // does not clear the new doc's overlay.
            UiEvent::DocumentReady { document_id, .. } if shown.current == Some(*document_id) => {
                effects.push(Effect::Status(ChromeStatus {
                    hydrating: false,
                    ..shown.status
                }));
            }
            // The open failed: lower the loading overlay and say why the
            // canvas stays empty (`CHROME-104`).
            UiEvent::DocumentOpenFailed { document_id, .. } => {
                let mut status = shown.status;
                if let Some(notice) = status.open_failed(shown.current, *document_id) {
                    effects.push(Effect::Status(status));
                    effects.push(Effect::Notice(Notice::new(ToastKind::Warning, notice)));
                }
            }
            // A document was shared with us: refresh the landing list so it
            // appears without a manual reload, and notify the user.
            UiEvent::DocumentShared { .. } => {
                effects.push(Effect::Notice(Notice::new(
                    ToastKind::Info,
                    "A document was shared with you",
                )));
                effects.push(Effect::Send(list_documents()));
            }
            // Relayed service failures (for example unknown invitee) surface as a
            // warning toast so the share action gives feedback.
            UiEvent::ApiError { message, .. } => effects.push(Effect::Notice(Notice::new(
                ToastKind::Warning,
                message.clone(),
            ))),
            _ => {}
        }
        effects
    }

    /// What a service answer of the Sync worker calls for.
    fn rest<'e>(
        &mut self,
        evt: &'e RestEvent,
        shown: Shown,
        roster: &dyn Roster,
        effects: &mut Vec<Effect<'e>>,
    ) {
        match evt {
            // Session bootstrap (dev `?user=` or OIDC): show the user, and
            // request the document list once for the landing overlay.
            RestEvent::SessionReady {
                user_id,
                display_name,
                ..
            } => {
                effects.push(Effect::SignedIn {
                    user_id: *user_id,
                    display_name,
                });
                if self.listing == Listing::Unasked {
                    self.listing = Listing::Asked;
                    effects.push(Effect::Send(list_documents()));
                    // A Murmure page lists its channels and their activity.
                    if self.page == Page::Murmure {
                        effects.push(Effect::BootMurmure);
                    }
                }
            }
            // The worker ended the session and asked the relay to revoke
            // it (`CHROME-107`). The relays deleted the drafts
            // (`ambre-spec-murmure-composer.md`). Once that write has landed, the page boots
            // home afresh and holds nothing of the session (`CHROME-108`).
            RestEvent::LoggedOut => effects.push(Effect::LeaveSession),
            // The provider's address: send the browser there (`CHROME-095`).
            RestEvent::ProviderAddress { url, state } => {
                effects.push(Effect::FollowProvider { url, state });
            }
            // Signed in with the provider: the Sync worker holds the tokens,
            // and the keys are still to open or to set up (`CHROME-096`).
            RestEvent::ProviderSignedIn { keys_stored, .. } => {
                tracing::info!(
                    "Provider sign-in done (keys stored: {})",
                    keys_stored.as_bool()
                );
            }
            RestEvent::DocumentList { documents } => effects.push(Effect::ShowDocuments(documents)),
            // A document was renamed or deleted (possibly from the launcher):
            // re-list so the card reflects the new name / disappears. Delete is
            // also removed optimistically by the card; re-listing reconciles.
            RestEvent::DocumentRenamed { .. } | RestEvent::DocumentDeleted { .. } => {
                effects.push(Effect::Send(list_documents()));
            }
            // The user left a document (`SYNC-141`), and the worker has
            // closed it. A view still on it goes back to the launcher, and
            // the list drops its card.
            RestEvent::DocumentLeft { document_id } => {
                effects.push(Effect::DropDocument(*document_id));
                if shown.current == Some(*document_id) {
                    effects.push(Effect::GoHome);
                }
            }
            // One message for two causes, and that is the contract rather
            // than laziness. The tag cannot tell a wrong passphrase from a
            // modified file, and a message that named one would be a guess
            // the user trusts (`DOCFILE-016`).
            RestEvent::DocFileOpenFailed => effects.push(Effect::Notice(Notice::new(
                ToastKind::Error,
                "That file did not open. The passphrase may be wrong, \
                 or the file may be damaged.",
            ))),
            // Share dialog: the current member list landed -> render it.
            RestEvent::DocumentMembers { members, .. } => {
                effects.push(Effect::ShowMembers(members));
            }
            // An invite succeeded: confirm and refresh the member list.
            RestEvent::InviteSent {
                document_id,
                display_name,
                ..
            } => {
                effects.push(Effect::Notice(Notice::new(
                    ToastKind::Success,
                    format!("Invited {}", display_name.as_str()),
                )));
                effects.push(Effect::Send(list_members(*document_id)));
            }
            // TOFU phase 1 answer: classify the fetched key against the
            // pinned one. Trusted (same key) -> dispatch phase 2 straight away;
            // New (no pin) or Changed (divergence) -> raise the modal prompt.
            RestEvent::InviteeKey {
                document_id,
                user_id,
                display_name,
                email,
                exchange_pub_key,
                fingerprint,
            } => effects.push(Effect::Classify(TofuPrompt {
                purpose: tofu::Purpose::Invite {
                    document_id: *document_id,
                    email: email.clone(),
                },
                user_id: *user_id,
                display_name: Some(display_name.clone()),
                exchange_pub_key: *exchange_pub_key,
                fingerprint: fingerprint.clone(),
                contact: tofu::KeyContact::New,
            })),
            // A member chosen in the Murmure palette, or for an invite into
            // the shown channel. Their key passes the same check before a
            // direct message or the invite wraps a key for them.
            RestEvent::MemberKey {
                user_id,
                exchange_pub_key,
                fingerprint,
            } => {
                let display_name = roster.member_name(*user_id);
                let purpose = match roster.take_invite(*user_id) {
                    Some(channel_id) => tofu::Purpose::Channel { channel_id },
                    None => tofu::Purpose::Direct {
                        channel_id: Id::<Channel>::mint(),
                    },
                };
                effects.push(Effect::Classify(TofuPrompt {
                    purpose,
                    user_id: *user_id,
                    display_name,
                    exchange_pub_key: *exchange_pub_key,
                    fingerprint: fingerprint.clone(),
                    contact: tofu::KeyContact::New,
                }));
            }
            // Revocation done (key rotation complete): confirm and refresh.
            RestEvent::RevocationComplete { document_id } => {
                effects.push(Effect::Notice(Notice::new(
                    ToastKind::Success,
                    "Member revoked",
                )));
                effects.push(Effect::Send(list_members(*document_id)));
            }
            // A role change took effect: refresh the member list.
            RestEvent::MemberRoleUpdated { document_id, .. } => {
                effects.push(Effect::Send(list_members(*document_id)));
            }
            _ => {}
        }
    }
}

/// The request for the user's document list.
fn list_documents() -> LogicCommand {
    LogicCommand::Rest {
        id: ambre_bus::next_request(),
        cmd: RestCommand::ListDocuments,
    }
}

/// The request for the member list of `document_id`.
fn list_members(document_id: Id<Document>) -> LogicCommand {
    LogicCommand::Rest {
        id: ambre_bus::next_request(),
        cmd: RestCommand::ListDocumentMembers { document_id },
    }
}

/// Map a `SyncWarningCode` (the closed `ambre-bus` contract) to a user-facing
/// notice. All are non-fatal, and all but an unreadable document correct
/// themselves, so the wording reassures rather than alarms. Exhaustive by
/// design: a new variant is a compile error here until it gets a message.
fn sync_warning_message(code: SyncWarningCode) -> &'static str {
    match code {
        SyncWarningCode::QuarantineOverflow => {
            "Syncing is catching up - some changes are taking longer to arrive."
        }
        SyncWarningCode::DecryptFailed => "A remote change could not be read and was skipped.",
        SyncWarningCode::DocumentUnreadable => {
            "Some changes were made by a different version of Ambre and cannot be shown here."
        }
        SyncWarningCode::LocalStoreFull => {
            "This device is out of storage. Editing is paused until your changes are saved."
        }
        SyncWarningCode::UpdateRefused => "A change could not be sent: its document is not open.",
        SyncWarningCode::EditRefused => {
            "The server refused a change. Reopen the document before editing it again."
        }
    }
}

/// Where an event goes once the relay has read it.
pub(crate) enum Destination {
    /// The canvas view's stream: content, and everything else.
    View(UiEvent),
    /// A sealed file that came back open (`DOCFILE-014`). What it holds is a
    /// whole plaintext container, so the ordinary open path takes it from
    /// here, and the view reads nothing of it.
    OpenFile(Payload<PlaintextContainer>),
}

/// Where `ev` goes. The event is taken by value, so the bytes of an opened
/// file move into the container with no copy.
pub(crate) fn destination(ev: UiEvent) -> Destination {
    match ev {
        UiEvent::Rest {
            evt: RestEvent::DocFileOpened { plaintext },
            ..
        } => Destination::OpenFile(Payload::<PlaintextContainer>::new(
            plaintext.into_plaintext(),
        )),
        ev => Destination::View(ev),
    }
}

/// The relay's roster on a Murmure page is the Murmure chrome's own.
#[cfg(target_arch = "wasm32")]
impl Roster for ambre_murmure::chrome::MurmureRelay {
    fn member_name(&self, user: Id<User>) -> Option<Validated<DisplayName>> {
        ambre_murmure::chrome::MurmureRelay::member_name(self, user)
    }

    fn take_invite(&self, user: Id<User>) -> Option<Id<Channel>> {
        ambre_murmure::chrome::MurmureRelay::take_invite(self, user)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHILD_BACKLOG_LIMIT, ChildFanOut, Destination, Effect, Page, Relay, Roster, Shown,
        destination,
    };
    use crate::ChromeStatus;
    use crate::tofu::{self, KeyContact, Next, Purpose, TofuPrompt};
    use ambre_bus::conduit;
    use ambre_bus::sync_warning::SyncWarningCode;
    use ambre_bus::{Confidential, DocType, Plaintext, RestCommand, RestEvent, Role};
    use ambre_chrome::ToastKind;
    use ambre_core::id::{Channel, Document, Id, User};
    use ambre_core::{
        Credential, DisplayName, Email, ExchangePublicKey, FixedBytes, KeyFingerprints, LinkState,
        OidcState, Opaque, ProviderUrls, Validated,
    };
    use ambre_logic::app::{LogicCommand, UiEvent};
    use std::cell::Cell;
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

    /// A roster with one member, who may be waiting for an invitation.
    struct Members {
        name: Option<Validated<DisplayName>>,
        invite: Cell<Option<(Id<User>, Id<Channel>)>>,
    }

    impl Roster for Members {
        fn member_name(&self, _: Id<User>) -> Option<Validated<DisplayName>> {
            self.name.clone()
        }

        fn take_invite(&self, user: Id<User>) -> Option<Id<Channel>> {
            match self.invite.get() {
                Some((invitee, channel)) if invitee == user => {
                    self.invite.set(None);
                    Some(channel)
                }
                _ => None,
            }
        }
    }

    /// A roster that knows nobody.
    fn nobody() -> Members {
        Members {
            name: None,
            invite: Cell::new(None),
        }
    }

    fn name(text: &str) -> Validated<DisplayName> {
        Validated::<DisplayName>::parse(text).expect("a display name")
    }

    fn key(byte: u8) -> FixedBytes<ExchangePublicKey> {
        FixedBytes::from_array([byte; 32])
    }

    fn rest(evt: RestEvent) -> UiEvent {
        UiEvent::Rest { id: None, evt }
    }

    /// The shell on `current`, its header still loading and online.
    fn on(current: Option<Id<Document>>) -> Shown {
        Shown {
            current,
            status: ChromeStatus {
                hydrating: true,
                connected: LinkState::Connected,
            },
        }
    }

    fn member_key(user_id: Id<User>, byte: u8) -> UiEvent {
        rest(RestEvent::MemberKey {
            user_id,
            exchange_pub_key: key(byte),
            fingerprint: Opaque::<KeyFingerprints>::new("ABCD EFGH"),
        })
    }

    /// The one prompt the relay asks to classify, or a failure naming what
    /// it answered instead.
    fn classified(effects: Vec<Effect<'_>>) -> TofuPrompt {
        match <[Effect<'_>; 1]>::try_from(effects) {
            Ok([Effect::Classify(prompt)]) => prompt,
            _ => panic!("the key was not handed to the classification alone"),
        }
    }

    /// The notice of `effect`, as its severity and its words.
    fn notice<'a>(effect: &'a Effect<'_>) -> Option<(Tone, &'a str)> {
        match effect {
            Effect::Notice(notice) => Some((tone(notice.kind), notice.text.as_str())),
            _ => None,
        }
    }

    /// The severity of a notice, in a form a failed assertion prints.
    #[derive(Debug, PartialEq, Eq)]
    enum Tone {
        Info,
        Success,
        Warning,
        Error,
    }

    fn tone(kind: ToastKind) -> Tone {
        match kind {
            ToastKind::Info => Tone::Info,
            ToastKind::Success => Tone::Success,
            ToastKind::Warning => Tone::Warning,
            ToastKind::Error => Tone::Error,
        }
    }

    /// Whether `effect` asks the list of the user's documents.
    fn asks_documents(effect: &Effect<'_>) -> bool {
        matches!(
            effect,
            Effect::Send(LogicCommand::Rest {
                cmd: RestCommand::ListDocuments,
                ..
            })
        )
    }

    /// The document whose member list `effect` asks, if it asks one.
    fn asks_members(effect: &Effect<'_>) -> Option<Id<Document>> {
        match effect {
            Effect::Send(LogicCommand::Rest {
                cmd: RestCommand::ListDocumentMembers { document_id },
                ..
            }) => Some(*document_id),
            _ => None,
        }
    }

    /// The Murmure members spec: a key fetched for a member the user invites
    /// into the shown channel goes to that channel. The invitation answers that
    /// one key, so the next key of the member opens a direct message.
    #[test]
    fn a_murmure_member_key_goes_to_the_invitation_then_to_a_direct_message() {
        let user = Id::<User>::mint();
        let channel = Id::<Channel>::mint();
        let roster = Members {
            name: Some(name("Bob")),
            invite: Cell::new(Some((user, channel))),
        };
        let mut relay = Relay::new(Page::Murmure);

        let ev = member_key(user, 3);
        let prompt = classified(relay.service(&ev, on(None), &roster));
        assert_eq!(
            prompt.purpose,
            Purpose::Channel {
                channel_id: channel
            }
        );
        assert_eq!(prompt.user_id, user);
        assert_eq!(prompt.display_name, Some(name("Bob")));
        assert_eq!(prompt.exchange_pub_key, key(3));
        assert_eq!(prompt.fingerprint.as_str(), "ABCD EFGH");
        assert_eq!(prompt.contact, KeyContact::New);

        let ev = member_key(user, 3);
        let again = classified(relay.service(&ev, on(None), &roster));
        assert!(
            matches!(again.purpose, Purpose::Direct { .. }),
            "one invitation answered two keys"
        );
    }

    /// The Murmure members spec: a key with no invitation waiting opens a
    /// direct message of its own, under a fresh channel each time.
    #[test]
    fn a_murmure_member_key_with_no_invitation_opens_a_direct_message() {
        let user = Id::<User>::mint();
        let mut relay = Relay::new(Page::Murmure);

        let first = member_key(user, 1);
        let second = member_key(user, 1);
        let first = classified(relay.service(&first, on(None), &nobody()));
        let second = classified(relay.service(&second, on(None), &nobody()));

        match (first.purpose, second.purpose) {
            (Purpose::Direct { channel_id: a }, Purpose::Direct { channel_id: b }) => {
                assert_ne!(a, b, "two requests shared one channel");
            }
            _ => panic!("a key with no invitation did not open a direct message"),
        }
        assert_eq!(first.display_name, None, "a stranger got a name");
    }

    /// An invitee's key is classified for its document. With no pin it is a
    /// first contact, and with another pin it is a change. Both ask the user.
    /// The pinned key reads the browser clock, so the wasm tier holds that
    /// path (`ambre_app::tofu::tests`).
    #[test]
    fn an_invitee_key_asks_for_a_first_contact_and_for_a_change() {
        let document_id = Id::<Document>::mint();
        let user_id = Id::<User>::mint();
        let ev = rest(RestEvent::InviteeKey {
            document_id,
            user_id,
            display_name: name("Ada"),
            email: Email::from("ada@test.local"),
            exchange_pub_key: key(7),
            fingerprint: Opaque::<KeyFingerprints>::new("WXYZ"),
        });
        let prompt = classified(Relay::new(Page::Document).service(&ev, on(None), &nobody()));
        assert_eq!(
            prompt.purpose,
            Purpose::Invite {
                document_id,
                email: Email::from("ada@test.local"),
            }
        );
        assert_eq!(prompt.display_name, Some(name("Ada")));

        match tofu::classify(prompt.clone(), None, Role::Editor) {
            Next::Ask(asked) => assert_eq!(asked.contact, KeyContact::New),
            Next::Proceed(sent) => panic!("a key with no pin left with no prompt: {sent:?}"),
        }
        match tofu::classify(prompt, Some(key(8)), Role::Editor) {
            Next::Ask(asked) => assert_eq!(asked.contact, KeyContact::Changed),
            Next::Proceed(sent) => panic!("a changed key left with no prompt: {sent:?}"),
        }
    }

    /// A Murmure page asks its channels once signed in, after the document
    /// list. A second session event shows the user again and asks nothing.
    #[test]
    fn a_murmure_page_boots_once_signed_in() {
        let user_id = Id::<User>::mint();
        let ev = rest(RestEvent::SessionReady {
            user_id,
            display_name: name("Eve"),
            email: Email::from("eve@test.local"),
        });
        let mut relay = Relay::new(Page::Murmure);

        let first = relay.service(&ev, on(None), &nobody());
        assert_eq!(first.len(), 3);
        match first.first() {
            Some(Effect::SignedIn {
                user_id: shown,
                display_name,
            }) => {
                assert_eq!(*shown, user_id);
                assert_eq!(display_name.as_str(), "Eve");
            }
            _ => panic!("the session did not show its user first"),
        }
        assert!(first.get(1).is_some_and(asks_documents));
        assert!(matches!(first.get(2), Some(Effect::BootMurmure)));

        let again = relay.service(&ev, on(None), &nobody());
        assert!(
            matches!(again.as_slice(), [Effect::SignedIn { .. }]),
            "a second session event asked again"
        );

        let document_page = Relay::new(Page::Document).service(&ev, on(None), &nobody());
        assert!(
            !document_page
                .iter()
                .any(|effect| matches!(effect, Effect::BootMurmure)),
            "a document page booted Murmure"
        );
    }

    /// The header follows the routed document alone. A link or a ready event
    /// of another document changes nothing. A failed open of the routed one
    /// lowers the loading screen and says so.
    #[test]
    fn the_header_follows_the_routed_document_alone() {
        let routed = Id::<Document>::mint();
        let other = Id::<Document>::mint();
        let mut relay = Relay::new(Page::Document);
        let status = |effects: Vec<Effect<'_>>| match effects.first() {
            Some(Effect::Status(status)) => Some(*status),
            _ => None,
        };

        let offline = |document_id| UiEvent::ConnectionStatus {
            document_id,
            connected: LinkState::Disconnected,
        };
        let ready = |document_id| UiEvent::DocumentReady {
            document_id,
            role: Role::Editor,
        };
        let failed = |document_id| UiEvent::DocumentOpenFailed {
            document_id,
            message: String::new(),
        };

        let ev = offline(routed);
        assert_eq!(
            status(relay.service(&ev, on(Some(routed)), &nobody())),
            Some(ChromeStatus {
                hydrating: true,
                connected: LinkState::Disconnected,
            })
        );
        let ev = ready(routed);
        assert_eq!(
            status(relay.service(&ev, on(Some(routed)), &nobody())),
            Some(ChromeStatus {
                hydrating: false,
                connected: LinkState::Connected,
            })
        );
        for ev in [offline(other), ready(other), failed(other)] {
            assert!(
                relay.service(&ev, on(Some(routed)), &nobody()).is_empty(),
                "another document moved the header"
            );
        }

        let ev = failed(routed);
        let effects = relay.service(&ev, on(Some(routed)), &nobody());
        assert_eq!(effects.len(), 2);
        assert!(effects.first().is_some_and(|e| matches!(
            e,
            Effect::Status(ChromeStatus {
                hydrating: false,
                ..
            })
        )));
        assert_eq!(
            effects.get(1).and_then(notice),
            Some((Tone::Warning, "The document could not open."))
        );
    }

    /// A document the user left drops its card, and a view still
    /// on it goes home. A view on another document stays.
    #[test]
    fn a_left_document_drops_its_card_and_sends_its_view_home() {
        let left = Id::<Document>::mint();
        let ev = rest(RestEvent::DocumentLeft { document_id: left });
        let mut relay = Relay::new(Page::Document);

        let on_it = relay.service(&ev, on(Some(left)), &nobody());
        assert!(matches!(
            on_it.as_slice(),
            [Effect::DropDocument(id), Effect::GoHome] if *id == left
        ));
        let elsewhere = relay.service(&ev, on(Some(Id::<Document>::mint())), &nobody());
        assert!(matches!(elsewhere.as_slice(), [Effect::DropDocument(_)]));
    }

    /// Each change of membership confirms what it can, then asks the member
    /// list of its document again.
    #[test]
    fn a_membership_change_asks_its_member_list_again() {
        let doc = Id::<Document>::mint();
        let mut relay = Relay::new(Page::Document);
        let cases = [
            (
                rest(RestEvent::InviteSent {
                    document_id: doc,
                    user_id: Id::<User>::mint(),
                    display_name: name("Bob"),
                }),
                Some((Tone::Success, "Invited Bob")),
            ),
            (
                rest(RestEvent::RevocationComplete { document_id: doc }),
                Some((Tone::Success, "Member revoked")),
            ),
            (
                rest(RestEvent::MemberRoleUpdated {
                    document_id: doc,
                    user_id: Id::<User>::mint(),
                    role: Role::Viewer,
                }),
                None,
            ),
        ];
        for (ev, said) in cases {
            let effects = relay.service(&ev, on(None), &nobody());
            assert_eq!(effects.iter().find_map(notice), said);
            assert_eq!(
                effects.last().and_then(asks_members),
                Some(doc),
                "a membership change left its list stale"
            );
        }
    }

    /// The landing reads the lists the relay hands it, and a sharing, a rename
    /// or a deletion asks the document list again.
    #[test]
    fn the_landing_takes_each_list_and_a_change_asks_it_again() {
        let mut relay = Relay::new(Page::Document);

        let ev = rest(RestEvent::DocumentList {
            documents: Vec::new(),
        });
        assert!(matches!(
            relay.service(&ev, on(None), &nobody()).as_slice(),
            [Effect::ShowDocuments(documents)] if documents.is_empty()
        ));
        let ev = rest(RestEvent::DocumentMembers {
            document_id: Id::<Document>::mint(),
            members: Vec::new(),
        });
        assert!(matches!(
            relay.service(&ev, on(None), &nobody()).as_slice(),
            [Effect::ShowMembers(members)] if members.is_empty()
        ));

        let shared = UiEvent::DocumentShared {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Trame,
        };
        let effects = relay.service(&shared, on(None), &nobody());
        assert_eq!(
            effects.first().and_then(notice),
            Some((Tone::Info, "A document was shared with you"))
        );
        assert!(effects.last().is_some_and(asks_documents));

        let renamed = rest(RestEvent::DocumentDeleted {
            document_id: Id::<Document>::mint(),
        });
        assert!(
            relay
                .service(&renamed, on(None), &nobody())
                .first()
                .is_some_and(asks_documents)
        );
    }

    /// The session's exits and the warnings each reach the wiring as their
    /// own decision, and a provider's address keeps its state.
    #[test]
    fn the_session_exits_and_the_warnings_each_reach_the_wiring() {
        let mut relay = Relay::new(Page::Document);

        let ev = rest(RestEvent::LoggedOut);
        assert!(matches!(
            relay.service(&ev, on(None), &nobody()).as_slice(),
            [Effect::LeaveSession]
        ));

        let ev = rest(RestEvent::ProviderAddress {
            url: Opaque::<ProviderUrls>::new("https://id.test.local/auth"),
            state: Credential::<OidcState>::new("state".to_owned()),
        });
        let RestEvent::ProviderAddress { state: sent, .. } = (match &ev {
            UiEvent::Rest { evt, .. } => evt,
            _ => panic!("the event is a service answer"),
        }) else {
            panic!("the event is a provider address");
        };
        match relay.service(&ev, on(None), &nobody()).as_slice() {
            [Effect::FollowProvider { url, state }] => {
                assert_eq!(url.as_str(), "https://id.test.local/auth");
                assert!(std::ptr::eq(*state, sent), "the state was copied");
            }
            _ => panic!("the provider's address was not followed"),
        }

        let warned = UiEvent::SyncWarning {
            document_id: Id::<Document>::mint(),
            code: SyncWarningCode::DecryptFailed,
            message: String::new(),
        };
        assert_eq!(
            relay
                .service(&warned, on(None), &nobody())
                .first()
                .and_then(notice),
            Some((
                Tone::Warning,
                "A remote change could not be read and was skipped."
            ))
        );
        let refused = rest(RestEvent::DocFileOpenFailed);
        assert_eq!(
            relay
                .service(&refused, on(None), &nobody())
                .first()
                .and_then(notice)
                .map(|(kind, _)| kind),
            Some(Tone::Error)
        );
        assert!(
            relay
                .service(&UiEvent::UpdateRequired, on(None), &nobody())
                .first()
                .is_some_and(|effect| notice(effect).is_some())
        );
    }

    /// A sealed file that came back open goes to the open path,
    /// and its bytes never reach the view. Every other event goes on to it.
    #[test]
    fn an_opened_file_goes_to_the_open_path_and_not_to_the_view() {
        let opened = rest(RestEvent::DocFileOpened {
            plaintext: Confidential::<Plaintext>::new(b"AMBRE".to_vec()),
        });
        match destination(opened) {
            Destination::OpenFile(container) => assert_eq!(container.as_bytes(), b"AMBRE"),
            Destination::View(_) => panic!("an opened file reached the view"),
        }

        let routed = Id::<Document>::mint();
        let content = UiEvent::DocumentReady {
            document_id: routed,
            role: Role::Editor,
        };
        match destination(content) {
            Destination::View(UiEvent::DocumentReady { document_id, .. }) => {
                assert_eq!(document_id, routed);
            }
            _ => panic!("an event of the view did not reach it"),
        }
    }
}
