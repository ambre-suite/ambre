//! The Murmure runtime: every open channel, space and direct message request
//! of one session, and what the relay and the view answer for each
//! (`specs/5-Murmure/features/ambre-spec-murmure-engine.md`).
//!
//! A channel spans several documents, while the engine interface of the Logic
//! runtime takes one. So Murmure keeps its own runtime state here, in its
//! crate, and the Logic runtime routes to it from its dispatch alone
//! (`ARCH-121`).
//!
//! No handler sends on a queue (`BUS-010`). Each one appends what it owes the
//! other threads to its outcome, in the order it owes them. The dispatcher
//! fans the outcome out once the handler returns ([`Runtime::take_outcome`]).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use ambre_bus::{
    DocType, Relay, Request, RestCommand, RestEvent, Role, Seq, SnapshotVerdict, SyncCommand,
};
use ambre_core::id::{Call, Channel, Document, Id, Message, Meta, Segment, Snapshot, Space, User};
use ambre_crdt::ledger::SeqLedger;

use crate::Order;
use crate::message::Murmure;
pub use channels::{HeldCommand, ReadCursor};

mod calls;
mod channels;
mod direct;
mod erasing;
mod removals;
mod segments;
mod spaces;
mod threads;
mod unopened;

/// The Logic to UI envelope, over the document sum `D` of the Logic runtime.
/// The runtime builds the variants of its channel plane alone.
pub type UiEvent<D> = ambre_bus::UiEvent<D, Murmure>;

/// How many commands a document holds while it waits for its session
/// identity (`ambre-spec-budgets.md` section 6). Past the cap a new command
/// is dropped and logged, so the ones held keep the order they came in.
const HELD_EDITS_LIMIT: usize = 256;

/// One write of a channel document to the local store, which the Main thread
/// performs (`RESID-004`, `RESID-010`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreWrite {
    /// The whole document, written as the channel's view closes.
    Snapshot(ambre_core::Payload<ambre_core::SnapshotBytes>),
    /// Not a write: the channel's view closed, so the copy may leave the
    /// store once the relay holds every change (`RESID-008`).
    Hidden,
    /// Not a write: the channel reopens, so the store reads the copy back.
    ReadBack,
    /// Not a write: the copy leaves the store whole. A channel whose log
    /// roots outside its space left it, so no later open finds it
    /// (`SYNC-233`).
    Erase,
}

/// One message the runtime owes another thread, in the order it owes it.
#[derive(Debug)]
pub enum Effect<D> {
    /// An event for the view.
    Ui(UiEvent<D>),
    /// A command for the Sync worker.
    Sync(SyncCommand),
    /// A write for the local store.
    Store(Id<Document>, StoreWrite),
    /// The image `media_ref`, stored under `document_id`, fetched and decoded
    /// for the preview the view of `channel_id` paints. The Logic runtime
    /// holds the decoders (`ambre-spec-murmure-attachments.md`).
    Preview {
        channel_id: Id<Channel>,
        document_id: Id<Document>,
        media_ref: ambre_core::Validated<ambre_bus::ContentHash>,
    },
}

/// The Murmure state of one session, held by the Logic thread.
///
/// `D` is the document sum of the Logic runtime. The runtime never builds one
/// of its variants: it names the sum only to build the envelope.
pub struct Runtime<D> {
    /// What the handlers owe the other threads since the last
    /// [`Runtime::take_outcome`]. A `Cell`, so a handler that reads the state
    /// alone still appends to it.
    outcome: Cell<Vec<Effect<D>>>,
    /// The signed-in user, from the relay's `SessionReady`.
    my_user_id: Option<Id<User>>,
    /// Open Murmure channels, keyed by `channel_id`. A channel is several
    /// documents, and a remote delta finds its channel by the document it
    /// names (the Murmure engine spec). Removed on a close and on logout.
    channels: HashMap<Id<Channel>, crate::ChannelState>,
    /// Channels on their way to open (the Murmure engine spec). `None` while
    /// the relay has not listed the documents yet. Moved to `channels` once
    /// every live document is in, and removed on a failure, a close or
    /// logout.
    opening: HashMap<Id<Channel>, Option<crate::OpeningChannel>>,
    /// The relay's number of each delta a channel document took in, so its
    /// snapshot names the last delta it folds in (`SRV-083`). Keyed by
    /// document, dropped with its channel.
    channel_ledgers: HashMap<Id<Document>, SeqLedger>,
    /// This user's read cursor on each channel document (the Murmure unread
    /// spec). The relay's activity answer sets it, and this client's own
    /// marks move it. Cleared on logout.
    read_cursors: HashMap<Id<Document>, channels::ReadCursor>,
    /// Channels whose view closed: their documents left memory, and Sync
    /// counts them from their summaries alone (`RESID-004`). Removed on a
    /// reopen, a delete, a leave or logout.
    closed_channels: HashMap<Id<Channel>, channels::ClosedChannel>,
    /// The channel of each segment and thread the relay's activity answer
    /// listed, so a thread's signal finds its channel (the Murmure threads
    /// spec). Cleared on logout.
    document_channels: HashMap<Id<Document>, Id<Channel>>,
    /// The threads this client does not hold, counted from their summaries
    /// as light subscriptions. Removed when the thread opens, or with its
    /// channel.
    watched_threads: HashMap<Id<Document>, threads::WatchedThread>,
    /// The signalled threads whose channel the next activity answer names.
    unplaced_threads: HashSet<Id<Document>>,
    /// The channels of the user this session may watch unopened (the
    /// Murmure notifications spec). Cleared on logout.
    listed_channels: unopened::ListedChannels,
    /// The role this client holds on the `meta` of each open or opening
    /// channel, from the ready of that `meta` (`MURMURE-729`). A channel
    /// whose `meta` it cannot write keeps its listed hint. Dropped with the
    /// channel, and cleared on logout.
    meta_roles: HashMap<Id<Document>, Role>,
    /// The close each open channel asked of the Sync worker, with the time
    /// of the send that filled its segment (the Murmure segments spec).
    /// Removed on the answer, and with the channel.
    closing_segments: HashMap<Id<Channel>, ambre_bus::Instant<ambre_bus::Authored>>,
    /// The successor each channel's close names, kept across a failed close
    /// so the next ask registers the one already created. Removed once the
    /// relay answers, and with the channel.
    close_successors: HashMap<Id<Channel>, Id<Segment>>,
    /// The edits and deletes a segment's lock refused, owed again as
    /// amendments. They leave at the channel's next tick, which stamps them.
    /// Removed with the channel.
    owed_replays: HashMap<Id<Channel>, Vec<crate::Replay>>,
    /// Who watches each open channel and who types in it, with this client's
    /// own samples (the Murmure typing and presence spec). Taken on the
    /// first tick, removed with the channel and on logout.
    channel_watchers: HashMap<Id<Channel>, crate::Watchers>,
    /// The asks of who is in the calls of each open channel (the Murmure
    /// calls spec).
    call_asks: HashMap<Id<Channel>, calls::CallAsks>,
    /// The live call of each channel this thread holds no state of (`MURMURE-691`). A sealed
    /// cue or the ask of the live calls names it. Removed at its end, and on logout.
    unheld_calls: HashMap<Id<Channel>, Id<Call>>,
    /// The declines of a ring whose channel was not open, each held until the
    /// channel opens (the Murmure calls spec, the ring).
    declines: HashMap<Id<Channel>, Vec<crate::message::MurmureCommand>>,
    /// The asks of a join for the `meta` of a channel still opening, each answered once the
    /// open ends (`CALL-149`).
    meta_asks: Vec<(Id<Call>, Id<Channel>)>,
    /// The members of each open or opening channel, as Sync lists them for
    /// its `meta` (`SYNC-150`). The `@` list offers them, and a mention of
    /// anyone else notifies no one (the Murmure mentions spec). Removed with
    /// the channel and on logout.
    channel_members: HashMap<Id<Channel>, Vec<ambre_bus::MemberInfo>>,
    /// Open Murmure spaces, keyed by `space_id`: one document each (the
    /// Murmure spaces spec). Removed on a close and on logout.
    spaces: HashMap<Id<Space>, spaces::OpenSpace>,
    /// Spaces on their way to open, with the payloads and the member rows
    /// that arrived before their document was ready.
    space_opening: HashMap<Id<Space>, spaces::Opening>,
    /// Public channels the relay is creating, until it answers.
    creating: HashMap<Id<Channel>, spaces::Creating>,
    /// Self-joins the relay has not answered yet, with their space.
    joining: HashMap<Id<Channel>, Id<Space>>,
    /// Channel deletes the relay has not answered yet, with their space.
    deleting: HashMap<Id<Channel>, Id<Space>>,
    /// Channel leaves the relay has not answered yet, with their space.
    leaving: HashMap<Id<Channel>, Id<Space>>,
    /// The removals from a channel that wait for it to open, so each leaves
    /// with the documents its sealed `meta` names (`SYNC-320`). Removed once
    /// the open ends, and on logout.
    removals: HashMap<Id<Channel>, Vec<removals::Removal>>,
    /// The channels opened for a removal alone, which close again once it
    /// leaves. An open the view asks for takes the channel off.
    opened_for_removal: HashSet<Id<Channel>>,
    /// Leaves of a space held while its owner role passes on (the Murmure
    /// spaces spec). Each is keyed by the role change this runtime asked
    /// for, and holds the leave the view asked for.
    handing_off: HashMap<Seq<Request>, (Seq<Request>, Id<Document>)>,
    /// The erases of a deleted message's files the relay has not answered,
    /// each with the document its file is stored under and its address.
    erasing: HashMap<Seq<Request>, (Id<Document>, ambre_core::Validated<ambre_core::ContentHash>)>,
    /// The erases that did not reach the relay, asked again on the next
    /// connection (`ambre-spec-murmure-attachments.md`).
    erase_owed: Vec<(Id<Document>, ambre_core::Validated<ambre_core::ContentHash>)>,
    /// Direct message requests the relay has not answered yet, keyed by the
    /// channel id each minted, with the channel it would create.
    direct: HashMap<Id<Channel>, crate::ChannelState>,
    /// The whole content of a document this client created, sent as its first
    /// update once the Sync worker holds its session.
    unsent: HashMap<Id<Document>, ambre_core::Payload<ambre_core::DeltaBytes>>,
    /// The session identity Sync issued for a channel or a space document
    /// that no open channel or space holds yet, adopted once one does
    /// (`AUTHOR-002`).
    murmure_identities: HashMap<Id<Document>, (ambre_core::PeerId, ambre_core::PeerEntry)>,
    /// The channel and space commands that write a document whose session
    /// identity has not landed, in the order they came (`AUTHOR-026`).
    murmure_waiting: HashMap<Id<Document>, Vec<channels::HeldCommand>>,
    /// The roots whose own thread the relay linked before the thread went
    /// up, by that thread. Each root names its thread once the thread's
    /// first update left (the Murmure threads spec).
    unnamed_roots: HashMap<Id<Document>, threads::UnnamedRoot>,
}

impl<D> Default for Runtime<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> Runtime<D> {
    /// A runtime with nothing open and no user signed in.
    #[must_use]
    pub fn new() -> Self {
        Self {
            outcome: Cell::new(Vec::new()),
            my_user_id: None,
            channels: HashMap::new(),
            opening: HashMap::new(),
            channel_ledgers: HashMap::new(),
            read_cursors: HashMap::new(),
            closed_channels: HashMap::new(),
            document_channels: HashMap::new(),
            watched_threads: HashMap::new(),
            unplaced_threads: HashSet::new(),
            listed_channels: unopened::ListedChannels::default(),
            meta_roles: HashMap::new(),
            closing_segments: HashMap::new(),
            close_successors: HashMap::new(),
            owed_replays: HashMap::new(),
            channel_watchers: HashMap::new(),
            call_asks: HashMap::new(),
            unheld_calls: HashMap::new(),
            declines: HashMap::new(),
            meta_asks: Vec::new(),
            channel_members: HashMap::new(),
            spaces: HashMap::new(),
            space_opening: HashMap::new(),
            creating: HashMap::new(),
            joining: HashMap::new(),
            deleting: HashMap::new(),
            leaving: HashMap::new(),
            removals: HashMap::new(),
            opened_for_removal: HashSet::new(),
            handing_off: HashMap::new(),
            erasing: HashMap::new(),
            erase_owed: Vec::new(),
            direct: HashMap::new(),
            unsent: HashMap::new(),
            murmure_identities: HashMap::new(),
            murmure_waiting: HashMap::new(),
            unnamed_roots: HashMap::new(),
        }
    }

    /// What the handlers owe the other threads since the last call, in the
    /// order they owe it. The dispatcher fans it out after each handler.
    pub fn take_outcome(&mut self) -> Vec<Effect<D>> {
        self.settle_meta_asks();
        self.outcome.take()
    }

    /// The relay signed `user_id` in: each open space learns who this
    /// client is, and the erases that waited for a session leave again.
    pub fn session_ready(&mut self, user_id: Id<User>) {
        self.my_user_id = Some(user_id);
        self.spaces_know_me(user_id);
        self.retry_erases();
    }

    /// The user logs out: every channel, space and request of the session
    /// goes, so no later answer writes against it.
    pub fn logout(&mut self) {
        self.channels.clear();
        self.channel_ledgers.clear();
        self.opening.clear();
        self.meta_roles.clear();
        self.read_cursors.clear();
        self.closed_channels.clear();
        self.document_channels.clear();
        self.watched_threads.clear();
        self.unplaced_threads.clear();
        self.listed_channels = unopened::ListedChannels::default();
        self.closing_segments.clear();
        self.close_successors.clear();
        self.owed_replays.clear();
        self.channel_watchers.clear();
        self.channel_members.clear();
        self.spaces.clear();
        self.space_opening.clear();
        self.creating.clear();
        self.joining.clear();
        // A request the relay has not answered belongs to this user. No
        // later session asks it again or acts on its answer (the Murmure
        // engine spec).
        self.deleting.clear();
        self.leaving.clear();
        self.removals.clear();
        self.opened_for_removal.clear();
        self.erasing.clear();
        self.erase_owed.clear();
        self.call_asks.clear();
        self.unheld_calls.clear();
        self.declines.clear();
        self.handing_off.clear();
        self.direct.clear();
        self.unsent.clear();
        self.murmure_identities.clear();
        self.murmure_waiting.clear();
        self.unnamed_roots.clear();
    }

    /// Owe the view `event`.
    fn emit(&self, event: UiEvent<D>) {
        self.owe(Effect::Ui(event));
    }

    /// Owe the Sync worker `command`.
    fn ask(&self, command: SyncCommand) {
        self.owe(Effect::Sync(command));
    }

    /// Owe the local store `write` of `document_id`.
    fn store(&self, document_id: Id<Document>, write: StoreWrite) {
        self.owe(Effect::Store(document_id, write));
    }

    /// Append `effect` to the outcome. The vector moves out and back, so no
    /// borrow of the cell is ever live. Kept out of line, so each handler
    /// that owes an effect pays one call and no copy of the push.
    #[inline(never)]
    fn owe(&self, effect: Effect<D>) {
        let mut outcome = self.outcome.take();
        outcome.push(effect);
        self.outcome.set(outcome);
    }

    /// The relay refused to open `channel_id`: a channel still opening ends,
    /// and the view learns why. A channel no longer opening is left alone.
    pub fn opening_failed(&mut self, channel_id: Id<Channel>, failure: &ambre_bus::ApiFailure) {
        if self.opening.remove(&channel_id).is_some() {
            self.end_light_channel(channel_id);
            self.channel_open_failed(channel_id, failure);
        }
    }

    /// The key of `channel_id` was sealed again for its space: the space's
    /// catalogue takes it, and the open channel learns its key rotated.
    pub fn key_resealed(
        &mut self,
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        key: &ambre_bus::SealedChannelKey,
    ) {
        self.channel_resealed(space_id, channel_id, key);
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.key_rotated();
        }
    }

    /// The relay refused the role change a held leave waited on. Returns the
    /// leave the view asked for, which fails with it, or `None` for a request
    /// that held no leave.
    pub fn hand_off_refused(&mut self, id: Seq<Request>) -> Option<Seq<Request>> {
        self.handing_off.remove(&id).map(|(leave, _)| leave)
    }

    /// Hand an invite on as the dispatcher would: one that announces its
    /// member first waits for it, any other leaves for the relay.
    fn forward_invite(&mut self, id: Seq<Request>, cmd: RestCommand) {
        if !self.announce_before_invite(id, &cmd) {
            self.ask(SyncCommand::Rest { id, cmd });
        }
    }
}

/// What the host tests of the Logic runtime read and plant in the state.
#[cfg(feature = "testing")]
impl<D> Runtime<D> {
    /// Name the signed-in user, with none of the effects of a session that
    /// begins.
    pub fn set_user(&mut self, user_id: Id<User>) {
        self.my_user_id = Some(user_id);
    }

    /// The open channels.
    #[must_use]
    pub fn channels(&self) -> &HashMap<Id<Channel>, crate::ChannelState> {
        &self.channels
    }

    /// The open channels, to plant one or to write one.
    pub fn channels_mut(&mut self) -> &mut HashMap<Id<Channel>, crate::ChannelState> {
        &mut self.channels
    }

    /// The channels on their way to open.
    #[must_use]
    pub fn opening(&self) -> &HashMap<Id<Channel>, Option<crate::OpeningChannel>> {
        &self.opening
    }

    /// This user's read cursor on each channel document.
    #[must_use]
    pub fn read_cursors(&self) -> &HashMap<Id<Document>, ReadCursor> {
        &self.read_cursors
    }

    /// The read cursors, to plant one.
    pub fn read_cursors_mut(&mut self) -> &mut HashMap<Id<Document>, ReadCursor> {
        &mut self.read_cursors
    }

    /// The channels whose view closed while their documents stay counted.
    pub fn closed_channels(&self) -> impl Iterator<Item = Id<Channel>> + '_ {
        self.closed_channels.keys().copied()
    }

    /// The threads this client counts without holding them.
    pub fn watched_threads(&self) -> impl Iterator<Item = Id<Document>> + '_ {
        self.watched_threads.keys().copied()
    }

    /// The successor each channel's close names.
    #[must_use]
    pub fn close_successors(&self) -> &HashMap<Id<Channel>, Id<Segment>> {
        &self.close_successors
    }

    /// The commands that wait on the identity of each document.
    #[must_use]
    pub fn murmure_waiting(&self) -> &HashMap<Id<Document>, Vec<HeldCommand>> {
        &self.murmure_waiting
    }

    /// The state of the open space `space_id`.
    #[must_use]
    pub fn space(&self, space_id: Id<Space>) -> Option<&crate::SpaceState> {
        self.spaces.get(&space_id).map(|space| &space.state)
    }

    /// The open spaces.
    pub fn spaces(&self) -> impl Iterator<Item = (Id<Space>, &crate::SpaceState)> + '_ {
        self.spaces.iter().map(|(id, space)| (*id, &space.state))
    }

    /// Plant a delete of `channel_id` the relay has not answered.
    pub fn plant_delete(&mut self, channel_id: Id<Channel>, space_id: Id<Space>) {
        self.deleting.insert(channel_id, space_id);
    }

    /// The members Sync listed for `channel_id`.
    #[must_use]
    pub fn channel_members(&self, channel_id: Id<Channel>) -> Option<&[ambre_bus::MemberInfo]> {
        self.channel_members.get(&channel_id).map(Vec::as_slice)
    }

    /// Plant the members Sync listed for `channel_id`.
    pub fn plant_members(&mut self, channel_id: Id<Channel>, members: Vec<ambre_bus::MemberInfo>) {
        self.channel_members.insert(channel_id, members);
    }

    /// The direct message requests the relay has not answered.
    #[must_use]
    pub fn direct_requests(&self) -> &HashMap<Id<Channel>, crate::ChannelState> {
        &self.direct
    }

    /// Forget the whole content of `document_id`, as if it left.
    pub fn forget_unsent(&mut self, document_id: Id<Document>) {
        self.unsent.remove(&document_id);
    }

    /// The whole content of each document this client created, waiting to
    /// leave as its first update.
    #[must_use]
    pub fn unsent(&self) -> &HashMap<Id<Document>, ambre_core::Payload<ambre_core::DeltaBytes>> {
        &self.unsent
    }
}
