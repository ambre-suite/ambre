// ambre-app/src/switchboard.rs
// ============================================================================
// The decisions of the switchboard (`ARCH-072`). The switchboard closure in
// `lib.rs` reads the queues, hands each event to this module, and applies
// what this module answers. This module names no DOM, no `web_sys` and no
// signal. So it compiles on the host, and the native tier runs its tests. It
// reads no clock either: the wiring passes each instant, so a test sets it.
//
// The view fan-out holds the senders of the main view and of the embedded
// views. Those are `ambre_bus::conduit` queues, which run on the host as
// well. The decision to defer an event depends on what the queue takes, so
// the fan-out keeps the senders rather than answering a list of sends.
// ============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use ambre_bus::conduit::{Sender, TrySendError};
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{DocumentSummary, MemberInfo, RestCommand, RestEvent, Role};
use ambre_canvas::{PerfPublisher, PerfSnapshot};
use ambre_chrome::ToastKind;
use ambre_core::id::{Channel, Document, Id, User};
use ambre_core::{
    Authored, Credential, DisplayName, ExchangePublicKey, FixedBytes, FracMs, Instant, InviterPin,
    OidcState, Opaque, PageLoad, Payload, PlaintextContainer, ProviderUrls, TimeSpan, Validated,
};
use ambre_logic::app::{LogicCommand, UiEvent};

use crate::ChromeStatus;
use crate::tofu::{self, TofuPrompt};
use ambre_core::Flag as _;

/// How many events may wait for one view target before the oldest are dropped.
///
/// Each view is pumped once per frame, so a backlog only forms on a burst
/// larger than the queue itself. Past this the client is saturated for good and
/// no amount of memory fixes it. The real answer is the busy signal `BUS-016`
/// prescribes, which lands with its own Story. Until then the bound is what
/// keeps a stalled view from growing without limit, and it says so out loud.
// A `u32` widens into a `usize` on every target the workspace builds.
pub(crate) const VIEW_BACKLOG_LIMIT: usize = crate::CHANNEL_CAPACITY.get() as usize;

/// The views a deferred event waits for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Target {
    /// The view of the routed document, on the top-level queue.
    Main,
    /// Every live view of one nested document.
    Child(Id<Document>),
}

/// What one view queue did with an event it was offered.
enum Offer {
    /// The queue took it.
    Taken,
    /// The queue was full, and hands it back.
    Full(UiEvent),
    /// The view is gone, and so is its queue.
    Gone,
}

/// Whether a view still reads its queue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reader {
    /// The view is there, whether or not it read lately.
    Live,
    /// The view dropped its receiver.
    Gone,
}

/// One view's queue, what it could not take yet, and the documents whose
/// events it lost.
///
/// Each view keeps a backlog of its own (`BUS-044`). A backlog shared by two
/// views of one document offers a waiting event to both again. The view that
/// took it the first time would then apply it twice.
struct ViewQueue {
    tx: Sender<UiEvent>,
    /// What the queue refused, oldest first (`BUS-038`).
    backlog: VecDeque<UiEvent>,
    /// The documents an event dropped past the bound belonged to, since the
    /// backlog last drained (`BUS-041`).
    lost: Vec<Id<Document>>,
}

impl ViewQueue {
    fn new(tx: Sender<UiEvent>) -> Self {
        Self {
            tx,
            backlog: VecDeque::new(),
            lost: Vec::new(),
        }
    }

    fn offer(&self, ev: UiEvent) -> Offer {
        match self.tx.try_send(ev) {
            Ok(()) => Offer::Taken,
            Err(TrySendError::Full(ev)) => Offer::Full(ev),
            Err(TrySendError::Disconnected(_)) => Offer::Gone,
        }
    }

    /// Hand `ev` to the view, behind what waits already (`BUS-038`). Reports
    /// whether the view is still there.
    fn send(&mut self, target: Target, ev: UiEvent) -> Reader {
        if !self.backlog.is_empty() {
            self.defer(target, ev);
            return Reader::Live;
        }
        match self.offer(ev) {
            Offer::Taken => Reader::Live,
            Offer::Full(ev) => {
                self.defer(target, ev);
                Reader::Live
            }
            Offer::Gone => Reader::Gone,
        }
    }

    /// Re-offer what waits, oldest first, stopping at the first event the queue
    /// still refuses so the order is kept (`BUS-038`). Reports whether the view
    /// is still there.
    fn flush(&mut self) -> Reader {
        while let Some(ev) = self.backlog.pop_front() {
            match self.offer(ev) {
                Offer::Taken => {}
                Offer::Full(ev) => {
                    self.backlog.push_front(ev);
                    return Reader::Live;
                }
                Offer::Gone => return Reader::Gone,
            }
        }
        Reader::Live
    }

    /// The documents this view lost an event of, once its backlog is empty
    /// again (`BUS-041`). The view reads its queue by then, so the state Logic
    /// answers with reaches it.
    fn caught_up(&mut self) -> Vec<Id<Document>> {
        if self.backlog.is_empty() {
            std::mem::take(&mut self.lost)
        } else {
            Vec::new()
        }
    }

    /// Queue `ev`, oldest first, and say so when the bound forces a drop
    /// (`BUS-039`). The document of the dropped event is kept, so the view can
    /// ask for its whole state once it reads again (`BUS-041`).
    fn defer(&mut self, target: Target, ev: UiEvent) {
        self.backlog.push_back(ev);
        if self.backlog.len() <= VIEW_BACKLOG_LIMIT {
            return;
        }
        let dropped = self.backlog.pop_front();
        let document = match target {
            Target::Main => dropped.and_then(|ev| ev.routed_document_id()),
            Target::Child(id) => Some(id),
        };
        if let Some(id) = document
            && !self.lost.contains(&id)
        {
            self.lost.push(id);
        }
        match target {
            Target::Main => {
                tracing::error!("main view: backlog full, an event was dropped");
            }
            Target::Child(id) => {
                tracing::error!("embedded document {id}: backlog full, an event was dropped");
            }
        }
    }
}

/// The view fan-out: the one writer on every view queue. It holds the main
/// view's queue, every live view of a nested document, and what each view
/// could not take yet (`BUS-038`, `BUS-039`).
///
/// The child factory registers a view, and the switchboard routes the events.
/// The two share one fan-out behind an `Rc`, on the page's main thread alone.
pub(crate) struct ViewFanOut {
    /// The queue of the routed document's view.
    main: RefCell<ViewQueue>,
    /// The queues of every live child view, by document. A list rather than a
    /// slot -- two embeds of one document are two views (the Toile embeds spec).
    routes: RefCell<HashMap<Id<Document>, Vec<ViewQueue>>>,
}

impl ViewFanOut {
    /// The fan-out over the main view's queue, with no child yet.
    pub(crate) fn new(main: Sender<UiEvent>) -> Self {
        Self {
            main: RefCell::new(ViewQueue::new(main)),
            routes: RefCell::default(),
        }
    }

    /// Add the queue of one more view of `id`.
    pub(crate) fn register(&self, id: Id<Document>, tx: Sender<UiEvent>) {
        self.routes
            .borrow_mut()
            .entry(id)
            .or_default()
            .push(ViewQueue::new(tx));
    }

    /// Fan a nested child's event out to its own view queues, or give the
    /// event back when no child holds a route for its document.
    ///
    /// A child's events must not drive the parent's formula bar, zoom chip or
    /// status band. Only a document that is a live child is diverted. The
    /// top-level document's events, and untagged service events, come back
    /// unchanged.
    pub(crate) fn divert(&self, ev: UiEvent) -> Option<UiEvent> {
        let Some(id) = ev.routed_document_id() else {
            return Some(ev);
        };
        let mut routes = self.routes.borrow_mut();
        let Some(views) = routes.get_mut(&id) else {
            return Some(ev);
        };
        // A gone view gives its slot back. The entry stays even with no view
        // left in it, and that emptiness is what holds the Toile embeds spec,
        // section 3. A child view is dropped the frame its embed leaves the
        // culled set, while its session stays open for the whole close
        // debounce. An event arriving in that window used to find no route and
        // fall through to the board's own stream and chrome mirrors. A Toile
        // nested in a Toile emits exactly the event the parent board would then
        // apply to itself. An empty entry absorbs it instead, and the next view
        // of that document pushes its queue back into the same slot.
        //
        // What is left is a few dozen bytes per document embedded at least once
        // in this session, which nothing reclaims. Bounded by the boards a user
        // opens, and the trade the other way is a delta applied to the wrong
        // document.
        views.retain_mut(|view| view.send(Target::Child(id), ev.clone()) == Reader::Live);
        None
    }

    /// Hand a top-level event to the main view, deferring it behind what its
    /// queue refused before (`BUS-038`, `BUS-039`).
    ///
    /// The old path dropped it: `let _ = view_tx.try_send(ev)` threw away
    /// whatever a full queue refused, with no trace. A burst larger than the
    /// queue left the view behind its own document.
    pub(crate) fn deliver(&self, ev: UiEvent) {
        let mut main = self.main.borrow_mut();
        if main.send(Target::Main, ev) == Reader::Gone {
            // No main view reads any more: nothing waits for it.
            main.backlog.clear();
        }
    }

    /// Re-offer every deferred event, oldest first, and answer the documents
    /// whose views lost an event and read again (`BUS-041`). The switchboard
    /// asks the Logic thread for the whole state of each.
    ///
    /// Each view stops at the first event its queue still refuses, so the
    /// order is kept (`BUS-038`).
    #[must_use = "a view that lost an event stays behind until its document is resent"]
    pub(crate) fn flush(&self) -> Vec<Id<Document>> {
        let mut lost = Vec::new();
        {
            let mut main = self.main.borrow_mut();
            if main.flush() == Reader::Live {
                lost.extend(main.caught_up());
            } else {
                main.backlog.clear();
                main.lost.clear();
            }
        }
        for views in self.routes.borrow_mut().values_mut() {
            views.retain_mut(|view| view.flush() == Reader::Live);
            for view in views.iter_mut() {
                lost.extend(view.caught_up());
            }
        }
        // Two views of one document lost it: one resend serves both.
        let mut resend: Vec<Id<Document>> = Vec::with_capacity(lost.len());
        for id in lost {
            if !resend.contains(&id) {
                resend.push(id);
            }
        }
        resend
    }

    /// How many events wait for `target`, summed over its views.
    #[cfg(test)]
    fn waiting(&self, target: Target) -> usize {
        match target {
            Target::Main => self.main.borrow().backlog.len(),
            Target::Child(id) => self
                .routes
                .borrow()
                .get(&id)
                .map_or(0, |views| views.iter().map(|v| v.backlog.len()).sum()),
        }
    }

    /// Whether no view waits on anything.
    #[cfg(test)]
    fn idle(&self) -> bool {
        self.main.borrow().backlog.is_empty()
            && self
                .routes
                .borrow()
                .values()
                .flatten()
                .all(|view| view.backlog.is_empty())
    }
}

/// What a page boots, as far as the switchboard reads it: a Murmure page asks its
/// channels and their activity once signed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    /// A document application, or the launcher's home.
    Document,
    /// The Murmure page.
    Murmure,
}

/// Whether the switchboard asked the document list of this page yet.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Listing {
    #[default]
    Unasked,
    Asked,
}

/// What the switchboard reads of the shell beside the event: the routed document,
/// and the header status as the shell holds it now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown {
    /// The currently routed document, or `None` on the launcher's home.
    pub(crate) current: Option<Id<Document>>,
    /// The header status before the event.
    pub(crate) status: ChromeStatus,
}

/// What the switchboard needs of the Murmure members, to file a fetched key under
/// what it is for.
pub(crate) trait Roster {
    /// The display name of a member this page knows, for the prompt.
    fn member_name(&self, user: Id<User>) -> Option<Validated<DisplayName>>;
    /// The channel `user` is being invited into, taken so that it answers one
    /// fetched key alone. `None` when no invitation waits for that member.
    fn take_invite(&self, user: Id<User>) -> Option<Id<Channel>>;
}

/// A notice the switchboard raises, with its severity.
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

/// One decision of the switchboard on a service event. The wiring applies each one
/// in the order the switchboard answers them.
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
    /// Put the link of the document the view is on, pinned to this inviter,
    /// on the clipboard (`SYNC-208`).
    CopyInviteLink(FixedBytes<InviterPin>),
    /// Compare a fetched key with the pinned one (`CHROME-050`).
    Classify(TofuPrompt),
    /// Replace the header status.
    Status(ChromeStatus),
}

/// The switchboard's own state across frames: the page it runs on, and whether it
/// asked the document list yet.
pub(crate) struct Switchboard {
    page: Page,
    listing: Listing,
}

impl Switchboard {
    /// The switchboard of a page that boots `page`.
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
            UiEvent::ApiError { failure, .. } => effects.push(Effect::Notice(Notice::new(
                ToastKind::Warning,
                failure.to_string(),
            ))),
            // A revocation left members whose key does not read: they lost
            // the document, and the user who revoked reads who (`SYNC-043`).
            UiEvent::KeyNotShared { members, .. } => {
                let names: Vec<&str> = members.iter().map(Validated::as_str).collect();
                effects.push(Effect::Notice(Notice::new(
                    ToastKind::Warning,
                    format!(
                        "The new key could not be shared with {}: their key is unreadable",
                        names.join(", ")
                    ),
                )));
            }
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
            // The invite link pins the inviter, so the invitee's first open
            // checks the log against it (`SYNC-208`).
            RestEvent::InviteSent {
                document_id,
                display_name,
                inviter,
                ..
            } => {
                effects.push(Effect::CopyInviteLink(*inviter));
                effects.push(Effect::Notice(Notice::new(
                    ToastKind::Success,
                    format!(
                        "Invited {}. Their invite link is on the clipboard: send it to them.",
                        display_name.as_str()
                    ),
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
        SyncWarningCode::SignatureRefused => {
            "A remote change carried no valid signature and was skipped."
        }
        SyncWarningCode::KeyNotHeld => {
            "A remote change uses a key this device does not hold yet and was skipped."
        }
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
        SyncWarningCode::LogEntryWithheld => {
            "The server may be hiding a change of members from this device. Do not share \
             anything new here until a member confirms who has access."
        }
    }
}

/// Where an event goes once the switchboard has read it.
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

/// Whether the document holds the user's input back: the view's queue, or
/// the launcher's outbox.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum Load {
    /// Every command left.
    #[default]
    Current,
    /// A command waits for room on the queue.
    Behind,
}

/// The busy notice: one when the document falls behind, one when it catches
/// up. The view restates its level every frame, so this reads the edge, and
/// the user meets one notice per episode rather than one per frame.
#[derive(Default)]
pub(crate) struct BusyNotice {
    last: Load,
}

impl BusyNotice {
    /// The notice the load of this frame calls for, if it changed.
    pub(crate) fn step(&mut self, load: Load) -> Option<Notice> {
        if load == self.last {
            return None;
        }
        self.last = load;
        Some(match load {
            Load::Behind => Notice::new(
                ToastKind::Warning,
                "The document is catching up. Your edit is queued.",
            ),
            Load::Current => Notice::new(ToastKind::Info, "The document is up to date."),
        })
    }
}

/// What a fetched key leads to once the wiring has read its pin
/// (`CHROME-050`, `CHROME-051`). A trusted key sends its request at once,
/// dated `now`. A new or a changed key asks the user first.
///
/// The wiring reads the pin off the thread and the clock after that read, so
/// the request carries the instant it leaves. `role` is the one an
/// invitation grants.
pub(crate) fn pin_read(
    prompt: TofuPrompt,
    pinned: Option<FixedBytes<ExchangePublicKey>>,
    role: Role,
    now: Instant<Authored>,
) -> tofu::Next {
    tofu::classify(prompt, pinned, role, now)
}

/// The shortest span between two perf read-outs of the status bar. The view
/// publishes a snapshot every frame, and a text read-out for a developer
/// needs no more than two a second. The HUD of the frame keeps the same
/// period (`ambre-spec-frame.md`).
pub(crate) const PERF_PERIOD: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(500.0);

/// The cadence of the perf read-out: one snapshot per [`PERF_PERIOD`] at
/// most, on the page clock.
#[derive(Default)]
pub(crate) struct PerfCadence {
    last: Instant<PageLoad>,
}

impl PerfCadence {
    /// The snapshot to show at `now`, or `None` while the last one is fresh.
    ///
    /// The cadence starts at the page load, so the first snapshot shows one
    /// period after it.
    pub(crate) fn step(
        &mut self,
        now: Instant<PageLoad>,
        perf: &PerfPublisher,
    ) -> Option<PerfSnapshot> {
        if now - self.last < PERF_PERIOD {
            return None;
        }
        self.last = now;
        Some(perf.read())
    }
}

/// The switchboard's roster on a Murmure page is the Murmure chrome's own.
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
        BusyNotice, Destination, Effect, Load, Notice, Page, PerfCadence, Roster, Shown,
        Switchboard, Target, VIEW_BACKLOG_LIMIT, ViewFanOut, destination, pin_read,
    };
    use crate::ChromeStatus;
    use crate::tofu::{KeyContact, Next, Purpose, TofuPrompt};
    use ambre_bus::conduit;
    use ambre_bus::sync_warning::SyncWarningCode;
    use ambre_bus::{Authored, DirectMember, Instant};
    use ambre_bus::{Confidential, DocType, Plaintext, RestCommand, RestEvent, Role};
    use ambre_canvas::{PerfPublisher, PerfSnapshot};
    use ambre_chrome::ToastKind;
    use ambre_core::id::{Channel, Document, Id, User};
    use ambre_core::{
        Count, Credential, DisplayName, Email, ExchangePublicKey, FixedBytes, KeyFingerprints,
        LinkState, Ms, OidcState, Opaque, PageLoad, ProviderUrls, TimeSpan, Validated,
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

    /// A fan-out whose main view is gone, for the tests that read the children.
    fn children() -> ViewFanOut {
        ViewFanOut::new(queue::<1>().0)
    }

    /// The document each event a view queue holds names, oldest first. A test
    /// mints one document per event, so the list reads the order back.
    fn drain(rx: &conduit::Receiver<UiEvent>) -> Vec<Id<Document>> {
        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                UiEvent::ConnectionStatus { document_id, .. } => seen.push(document_id),
                _ => panic!("a foreign event reached the main view"),
            }
        }
        seen
    }

    /// Counts the error events while `f` runs.
    fn errors_during(f: impl FnOnce()) -> usize {
        let errors = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::registry(),
            Errors(Arc::clone(&errors)),
        );
        tracing::subscriber::with_default(subscriber, f);
        errors.load(Ordering::Relaxed)
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
        let fan_out = children();
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
            fan_out.waiting(Target::Child(doc)),
            2,
            "a refused event was dropped"
        );

        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            seen.push(connected(ev));
            assert!(fan_out.flush().is_empty(), "nothing was lost");
        }
        assert_eq!(
            seen,
            [
                LinkState::Connected,
                LinkState::Disconnected,
                LinkState::Connected
            ]
        );
        assert!(fan_out.idle(), "a drained backlog lingers");
    }

    /// The Toile embeds spec, section 5: a backlog past its bound drops its oldest event, and
    /// says so. Nothing is dropped in silence.
    #[test]
    fn an_overflowing_backlog_says_what_it_dropped() {
        let fan_out = children();
        let doc = Id::<Document>::mint();
        let (tx, _rx) = queue::<1>();
        fan_out.register(doc, tx);
        let errors = errors_during(|| {
            // One taken by the queue, the bound waiting, and two past it.
            for _ in 0..VIEW_BACKLOG_LIMIT + 3 {
                assert!(fan_out.divert(status(doc, LinkState::Connected)).is_none());
            }
        });
        assert_eq!(fan_out.waiting(Target::Child(doc)), VIEW_BACKLOG_LIMIT);
        assert_eq!(errors, 2, "an event was dropped in silence");
    }

    /// `BUS-038`: what the main view's full queue refuses waits, and a later
    /// event never overtakes it.
    #[test]
    fn a_refused_main_view_event_waits_and_arrives_in_order() {
        let (tx, rx) = queue::<1>();
        let fan_out = ViewFanOut::new(tx);
        let sent: Vec<Id<Document>> = (0..3).map(|_| Id::<Document>::mint()).collect();

        for &doc in &sent {
            fan_out.deliver(status(doc, LinkState::Connected));
        }
        assert_eq!(
            fan_out.waiting(Target::Main),
            2,
            "a refused event was dropped"
        );

        let mut seen = drain(&rx);
        while !fan_out.idle() {
            assert!(fan_out.flush().is_empty(), "nothing was lost");
            seen.extend(drain(&rx));
        }
        assert_eq!(seen, sent);
    }

    /// `BUS-039`: past its bound the main view's backlog drops its oldest
    /// event, and says so once per event. Nothing is dropped in silence.
    #[test]
    fn an_overflowing_main_view_backlog_says_what_it_dropped() {
        let (tx, rx) = queue::<1>();
        let fan_out = ViewFanOut::new(tx);
        let sent: Vec<Id<Document>> = (0..VIEW_BACKLOG_LIMIT + 3)
            .map(|_| Id::<Document>::mint())
            .collect();

        // One taken by the queue, the bound waiting, and two past it.
        let errors = errors_during(|| {
            for &doc in &sent {
                fan_out.deliver(status(doc, LinkState::Connected));
            }
        });
        assert_eq!(errors, 2, "an event was dropped in silence");

        let mut seen = drain(&rx);
        while !fan_out.idle() {
            let _resent = fan_out.flush();
            seen.extend(drain(&rx));
        }
        // The queue kept the first; the backlog lost the two oldest it held.
        let mut kept = vec![sent[0]];
        kept.extend_from_slice(&sent[3..]);
        assert_eq!(seen, kept);
    }

    /// `BUS-038`: the next frame's flush hands the main view what waited,
    /// before an event that frame delivers.
    #[test]
    fn the_main_view_backlog_drains_on_the_next_frame() {
        let (tx, rx) = queue::<3>();
        let fan_out = ViewFanOut::new(tx);
        let first: Vec<Id<Document>> = (0..5).map(|_| Id::<Document>::mint()).collect();

        // Frame 1: a burst of five on a queue of three, then the view reads.
        for &doc in &first {
            fan_out.deliver(status(doc, LinkState::Connected));
        }
        assert_eq!(drain(&rx), first[..3]);

        // Frame 2: the flush runs before the fresh stream, as `lib.rs` orders it.
        assert!(fan_out.flush().is_empty(), "nothing was lost");
        assert!(fan_out.idle(), "a drained backlog lingers");
        let fresh = Id::<Document>::mint();
        fan_out.deliver(status(fresh, LinkState::Connected));
        assert_eq!(drain(&rx), [first[3], first[4], fresh]);
    }

    /// `BUS-040`: each view waits on its own backlog. A full main view leaves a
    /// child its events, and a full child leaves the main view its own.
    #[test]
    fn a_full_view_holds_back_no_other_view() {
        let (main_tx, main_rx) = queue::<1>();
        let fan_out = ViewFanOut::new(main_tx);
        let child = Id::<Document>::mint();
        let (child_tx, child_rx) = queue::<1>();
        fan_out.register(child, child_tx);

        // Fill the main view, and leave one event waiting for it.
        let top: Vec<Id<Document>> = (0..2).map(|_| Id::<Document>::mint()).collect();
        for &doc in &top {
            fan_out.deliver(status(doc, LinkState::Connected));
        }
        assert!(
            fan_out
                .divert(status(child, LinkState::Connected))
                .is_none()
        );
        assert_eq!(
            child_rx.try_recv().map(connected),
            Ok(LinkState::Connected),
            "the main view's backlog held the child back"
        );

        // Fill the child, and leave one event waiting for it.
        for link in [LinkState::Connected, LinkState::Disconnected] {
            assert!(fan_out.divert(status(child, link)).is_none());
        }
        assert_eq!(drain(&main_rx), top[..1]);
        assert!(fan_out.flush().is_empty(), "nothing was lost");
        assert_eq!(
            drain(&main_rx),
            top[1..],
            "the child's backlog held the main view back"
        );
        assert_eq!(
            fan_out.waiting(Target::Child(child)),
            1,
            "the child's refused event was dropped"
        );
    }

    /// The Toile embeds spec, section 3: a nested document's event stays with its children, even
    /// once its last view is gone. The route outlives the view, so the event is
    /// absorbed instead of falling through to the board's own stream.
    #[test]
    fn a_childs_event_never_reaches_the_board() {
        let fan_out = children();
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
        let fan_out = children();
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

    /// `BUS-044`: the two embeds of one document are two views with a backlog
    /// each. The view that took an event the first time is not offered it
    /// again when the other one reads.
    #[test]
    fn two_views_of_one_document_never_take_an_event_twice() {
        let fan_out = children();
        let doc = Id::<Document>::mint();
        let (wide, wide_rx) = queue::<4>();
        let (narrow, narrow_rx) = queue::<1>();
        fan_out.register(doc, wide);
        fan_out.register(doc, narrow);

        for link in [LinkState::Connected, LinkState::Disconnected] {
            assert!(fan_out.divert(status(doc, link)).is_none());
        }
        assert_eq!(fan_out.waiting(Target::Child(doc)), 1);

        let mut narrow_seen = Vec::new();
        while let Ok(ev) = narrow_rx.try_recv() {
            narrow_seen.push(connected(ev));
            assert!(fan_out.flush().is_empty(), "nothing was lost");
        }
        let wide_seen: Vec<LinkState> = std::iter::from_fn(|| wide_rx.try_recv().ok())
            .map(connected)
            .collect();
        let both = [LinkState::Connected, LinkState::Disconnected];
        assert_eq!(wide_seen, both, "the wide view took an event twice");
        assert_eq!(narrow_seen, both);
    }

    /// `BUS-041`: a main view that lost an event has its document resent once
    /// it reads again, and once only. A view that still lags asks nothing yet.
    #[test]
    fn a_main_view_that_lost_an_event_has_its_document_resent_once_it_reads() {
        let (tx, rx) = queue::<1>();
        let fan_out = ViewFanOut::new(tx);
        let doc = Id::<Document>::mint();
        let errors = errors_during(|| {
            for _ in 0..VIEW_BACKLOG_LIMIT + 2 {
                fan_out.deliver(status(doc, LinkState::Connected));
            }
        });
        assert_eq!(errors, 1, "one event past the bound");
        assert!(
            fan_out.flush().is_empty(),
            "a view that reads nothing was resent its document"
        );

        let mut resent = Vec::new();
        while rx.try_recv().is_ok() {
            resent.extend(fan_out.flush());
        }
        assert!(fan_out.idle());
        assert_eq!(resent, [doc]);
        assert!(fan_out.flush().is_empty(), "the document was resent twice");
    }

    /// `BUS-041`: an embedded view that lost an event has its document resent
    /// once it reads again.
    #[test]
    fn an_embedded_view_that_lost_an_event_has_its_document_resent_once_it_reads() {
        let fan_out = children();
        let doc = Id::<Document>::mint();
        let (tx, rx) = queue::<1>();
        fan_out.register(doc, tx);
        let errors = errors_during(|| {
            for _ in 0..VIEW_BACKLOG_LIMIT + 2 {
                assert!(fan_out.divert(status(doc, LinkState::Connected)).is_none());
            }
        });
        assert_eq!(errors, 1, "one event past the bound");

        let mut resent = Vec::new();
        while rx.try_recv().is_ok() {
            resent.extend(fan_out.flush());
        }
        assert_eq!(resent, [doc]);
    }

    /// `BUS-041`: a view gone before it read again asks for nothing.
    #[test]
    fn a_view_gone_after_a_loss_asks_for_nothing() {
        let fan_out = children();
        let doc = Id::<Document>::mint();
        let (tx, rx) = queue::<1>();
        fan_out.register(doc, tx);
        errors_during(|| {
            for _ in 0..VIEW_BACKLOG_LIMIT + 2 {
                assert!(fan_out.divert(status(doc, LinkState::Connected)).is_none());
            }
        });
        drop(rx);
        assert!(fan_out.flush().is_empty());
        assert!(fan_out.idle(), "a gone view kept its backlog");
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

    /// The one prompt the switchboard asks to classify, or a failure naming what
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
        let mut switchboard = Switchboard::new(Page::Murmure);

        let ev = member_key(user, 3);
        let prompt = classified(switchboard.service(&ev, on(None), &roster));
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
        let again = classified(switchboard.service(&ev, on(None), &roster));
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
        let mut switchboard = Switchboard::new(Page::Murmure);

        let first = member_key(user, 1);
        let second = member_key(user, 1);
        let first = classified(switchboard.service(&first, on(None), &nobody()));
        let second = classified(switchboard.service(&second, on(None), &nobody()));

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
        let prompt = classified(Switchboard::new(Page::Document).service(&ev, on(None), &nobody()));
        assert_eq!(
            prompt.purpose,
            Purpose::Invite {
                document_id,
                email: Email::from("ada@test.local"),
            }
        );
        assert_eq!(prompt.display_name, Some(name("Ada")));

        match pin_read(prompt.clone(), None, Role::Editor, AT) {
            Next::Ask(asked) => assert_eq!(asked.contact, KeyContact::New),
            Next::Proceed(sent) => panic!("a key with no pin left with no prompt: {sent:?}"),
        }
        match pin_read(prompt, Some(key(8)), Role::Editor, AT) {
            Next::Ask(asked) => assert_eq!(asked.contact, KeyContact::Changed),
            Next::Proceed(sent) => panic!("a changed key left with no prompt: {sent:?}"),
        }
    }

    /// The instant the clock of these tests reads when a request leaves.
    const AT: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

    /// The pinned key of an invitee sends the invitation with no prompt. It
    /// grants the role asked, carries the key the user saw, and leaves at the
    /// instant the wiring read (`CHROME-047`, `CHROME-051`).
    #[test]
    fn a_pinned_invitee_key_invites_at_the_instant_read() {
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
        let prompt = classified(Switchboard::new(Page::Document).service(&ev, on(None), &nobody()));
        let Next::Proceed(LogicCommand::Rest {
            cmd:
                RestCommand::InviteToDocument {
                    document_id: sent_to,
                    invitee,
                    role,
                    expect_exchange_pub_key,
                    now,
                    ..
                },
            ..
        }) = pin_read(prompt, Some(key(7)), Role::Viewer, AT)
        else {
            panic!("a pinned key did not invite");
        };
        assert_eq!(sent_to, document_id);
        assert_eq!(invitee, user_id);
        assert_eq!(role, Role::Viewer);
        assert_eq!(expect_exchange_pub_key, key(7));
        assert_eq!(now, AT, "the invitation is dated by another clock");
    }

    /// The pinned key of a Murmure member opens the direct message of the two
    /// at the instant read. A later clock dates the same request later, so the
    /// instant comes from the wiring and from no clock of its own.
    #[test]
    fn a_pinned_member_key_opens_the_direct_message_at_the_instant_read() {
        let user_id = Id::<User>::mint();
        let prompt = classified(Switchboard::new(Page::Murmure).service(
            &member_key(user_id, 5),
            on(None),
            &nobody(),
        ));
        let later = AT.plus(TimeSpan::<Ms>::new(60_000));
        for at in [AT, later] {
            let Next::Proceed(LogicCommand::OpenDirectMessage { draft, now }) =
                pin_read(prompt.clone(), Some(key(5)), Role::Editor, at)
            else {
                panic!("a pinned member key did not open the direct message");
            };
            assert_eq!(now, at);
            assert_eq!(
                draft.members,
                vec![DirectMember {
                    user_id,
                    exchange_pub_key: key(5),
                }]
            );
        }
    }

    /// The perf read-out shows one snapshot per period at most, on the page
    /// clock the wiring reads. The first one waits a period from the page
    /// load, and a frame inside the period shows nothing.
    #[test]
    fn the_perf_read_out_shows_one_snapshot_per_period() {
        let perf = PerfPublisher::new();
        let mut cadence = PerfCadence::default();
        let at = |ms: f64| Instant::<PageLoad>::new(ms);
        let shown = |snapshot: Option<PerfSnapshot>| snapshot.map(|s| s.cells_drawn);

        perf.publish(drawn(1));
        assert_eq!(
            shown(cadence.step(at(16.0), &perf)),
            None,
            "shown before a period"
        );
        assert_eq!(shown(cadence.step(at(500.0), &perf)), Some(Count::new(1)));
        perf.publish(drawn(2));
        assert_eq!(
            shown(cadence.step(at(999.0), &perf)),
            None,
            "shown twice in a period"
        );
        assert_eq!(shown(cadence.step(at(1_000.0), &perf)), Some(Count::new(2)));
        // A frame late by a whole period shows once, and the period restarts there.
        assert_eq!(shown(cadence.step(at(2_700.0), &perf)), Some(Count::new(2)));
        assert_eq!(shown(cadence.step(at(3_100.0), &perf)), None);
        assert_eq!(shown(cadence.step(at(3_200.0), &perf)), Some(Count::new(2)));
    }

    /// A snapshot that counts `cells` drawn cells, so two read apart.
    fn drawn(cells: u32) -> PerfSnapshot {
        PerfSnapshot {
            cells_drawn: Count::new(cells),
            ..PerfSnapshot::default()
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
        let mut switchboard = Switchboard::new(Page::Murmure);

        let first = switchboard.service(&ev, on(None), &nobody());
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

        let again = switchboard.service(&ev, on(None), &nobody());
        assert!(
            matches!(again.as_slice(), [Effect::SignedIn { .. }]),
            "a second session event asked again"
        );

        let document_page = Switchboard::new(Page::Document).service(&ev, on(None), &nobody());
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
        let mut switchboard = Switchboard::new(Page::Document);
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
            cause: ambre_bus::OpenFailure::Transient,
        };

        let ev = offline(routed);
        assert_eq!(
            status(switchboard.service(&ev, on(Some(routed)), &nobody())),
            Some(ChromeStatus {
                hydrating: true,
                connected: LinkState::Disconnected,
            })
        );
        let ev = ready(routed);
        assert_eq!(
            status(switchboard.service(&ev, on(Some(routed)), &nobody())),
            Some(ChromeStatus {
                hydrating: false,
                connected: LinkState::Connected,
            })
        );
        for ev in [offline(other), ready(other), failed(other)] {
            assert!(
                switchboard
                    .service(&ev, on(Some(routed)), &nobody())
                    .is_empty(),
                "another document moved the header"
            );
        }

        let ev = failed(routed);
        let effects = switchboard.service(&ev, on(Some(routed)), &nobody());
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
        let mut switchboard = Switchboard::new(Page::Document);

        let on_it = switchboard.service(&ev, on(Some(left)), &nobody());
        assert!(matches!(
            on_it.as_slice(),
            [Effect::DropDocument(id), Effect::GoHome] if *id == left
        ));
        let elsewhere = switchboard.service(&ev, on(Some(Id::<Document>::mint())), &nobody());
        assert!(matches!(elsewhere.as_slice(), [Effect::DropDocument(_)]));
    }

    /// Holds `SYNC-208` at the switchboard: a landed invite puts the link
    /// pinned to the inviter's own key on the clipboard, and says so.
    #[test]
    fn a_landed_invite_copies_the_link_pinned_to_its_inviter() {
        let pin = FixedBytes::<ambre_core::InviterPin>::from_array([3; 32]);
        let mut switchboard = Switchboard::new(Page::Document);
        let ev = rest(RestEvent::InviteSent {
            document_id: Id::<Document>::mint(),
            user_id: Id::<User>::mint(),
            display_name: name("Bob"),
            inviter: pin,
        });
        let effects = switchboard.service(&ev, on(None), &nobody());
        assert!(matches!(
            effects.first(),
            Some(Effect::CopyInviteLink(copied)) if *copied == pin
        ));
    }

    /// Each change of membership confirms what it can, then asks the member
    /// list of its document again.
    #[test]
    fn a_membership_change_asks_its_member_list_again() {
        let doc = Id::<Document>::mint();
        let mut switchboard = Switchboard::new(Page::Document);
        let cases = [
            (
                rest(RestEvent::InviteSent {
                    document_id: doc,
                    user_id: Id::<User>::mint(),
                    display_name: name("Bob"),
                    inviter: FixedBytes::from_array([3; 32]),
                }),
                Some((
                    Tone::Success,
                    "Invited Bob. Their invite link is on the clipboard: send it to them.",
                )),
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
            let effects = switchboard.service(&ev, on(None), &nobody());
            assert_eq!(effects.iter().find_map(notice), said);
            assert_eq!(
                effects.last().and_then(asks_members),
                Some(doc),
                "a membership change left its list stale"
            );
        }
    }

    /// The landing reads the lists the switchboard hands it, and a sharing, a rename
    /// or a deletion asks the document list again.
    #[test]
    fn the_landing_takes_each_list_and_a_change_asks_it_again() {
        let mut switchboard = Switchboard::new(Page::Document);

        let ev = rest(RestEvent::DocumentList {
            documents: Vec::new(),
        });
        assert!(matches!(
            switchboard.service(&ev, on(None), &nobody()).as_slice(),
            [Effect::ShowDocuments(documents)] if documents.is_empty()
        ));
        let ev = rest(RestEvent::DocumentMembers {
            document_id: Id::<Document>::mint(),
            members: Vec::new(),
        });
        assert!(matches!(
            switchboard.service(&ev, on(None), &nobody()).as_slice(),
            [Effect::ShowMembers(members)] if members.is_empty()
        ));

        let shared = UiEvent::DocumentShared {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Trame,
        };
        let effects = switchboard.service(&shared, on(None), &nobody());
        assert_eq!(
            effects.first().and_then(notice),
            Some((Tone::Info, "A document was shared with you"))
        );
        assert!(effects.last().is_some_and(asks_documents));

        let renamed = rest(RestEvent::DocumentDeleted {
            document_id: Id::<Document>::mint(),
        });
        assert!(
            switchboard
                .service(&renamed, on(None), &nobody())
                .first()
                .is_some_and(asks_documents)
        );
    }

    /// The session's exits and the warnings each reach the wiring as their
    /// own decision, and a provider's address keeps its state.
    #[test]
    fn the_session_exits_and_the_warnings_each_reach_the_wiring() {
        let mut switchboard = Switchboard::new(Page::Document);

        let ev = rest(RestEvent::LoggedOut);
        assert!(matches!(
            switchboard.service(&ev, on(None), &nobody()).as_slice(),
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
        match switchboard.service(&ev, on(None), &nobody()).as_slice() {
            [Effect::FollowProvider { url, state }] => {
                assert_eq!(url.as_str(), "https://id.test.local/auth");
                assert!(std::ptr::eq(*state, sent), "the state was copied");
            }
            _ => panic!("the provider's address was not followed"),
        }

        let warned = UiEvent::SyncWarning {
            document_id: Id::<Document>::mint(),
            code: SyncWarningCode::DecryptFailed,
        };
        assert_eq!(
            switchboard
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
            switchboard
                .service(&refused, on(None), &nobody())
                .first()
                .and_then(notice)
                .map(|(kind, _)| kind),
            Some(Tone::Error)
        );
        assert!(
            switchboard
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

    /// A document that falls behind says so once, and says once that it
    /// caught up. A frame that restates the same level raises nothing.
    #[test]
    fn the_busy_notice_reads_the_edge_alone() {
        let mut busy = BusyNotice::default();
        let said = |notice: Option<Notice>| notice.map(|n| (tone(n.kind), n.text));

        assert_eq!(
            said(busy.step(Load::Current)),
            None,
            "a calm start raised a notice"
        );
        assert_eq!(
            said(busy.step(Load::Behind)),
            Some((
                Tone::Warning,
                "The document is catching up. Your edit is queued.".to_owned()
            ))
        );
        assert_eq!(
            said(busy.step(Load::Behind)),
            None,
            "one episode raised two notices"
        );
        assert_eq!(
            said(busy.step(Load::Current)),
            Some((Tone::Info, "The document is up to date.".to_owned()))
        );
        assert_eq!(said(busy.step(Load::Current)), None);
    }
}
