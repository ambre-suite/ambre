// ambre-murmure/src/logic/runtime/unopened.rs
// ============================================================================
// The channels of the user this session never opened. Each is counted from
// the light subscriptions of its `meta` and its segments, as a closed channel
// is. Its first message then notifies (the Murmure notifications spec).
// ============================================================================

use super::*;
use ambre_bus::{ChannelDocumentKind, ChannelMeta, ChannelName, DocumentActivity};

/// A name of a channel, as `meta` holds it or as the seal beside `meta` holds it.
type HeldName = Option<ambre_core::Validated<ambre_core::Name>>;

/// The segments of a channel, each with the last number the activity
/// answer gave, if any.
///
/// Written out at its uses, the type fails `clippy::type_complexity`.
type Segments = Vec<(Id<Document>, Option<Seq<Relay>>)>;

/// What the Logic thread knows of the channels it may watch unopened.
#[derive(Debug, Default)]
pub(crate) struct ListedChannels {
    /// The `meta` of each channel of the latest listing whose key opened,
    /// with its last number then.
    metas: HashMap<Id<Channel>, ChannelMeta>,
    /// Whether a listing answered in this session.
    listed: bool,
    /// The segments of each channel the latest activity answer named, each
    /// with its last number then, or `None` before any answer.
    segments: Option<HashMap<Id<Channel>, Segments>>,
    /// The channels watched unopened, so a listing that drops one ends it.
    watched: HashSet<Id<Channel>>,
    /// The name sealed beside the `meta` of each channel of the latest
    /// listing, `None` for a channel listed with no name that opened. It is a
    /// hint: a seal that failed, or two renames racing, leave an older one.
    hints: HashMap<Id<Channel>, HeldName>,
}

impl ListedChannels {
    /// Forget a channel the user no longer reads, deleted or left, so no
    /// later answer watches it again.
    pub(crate) fn forget(&mut self, channel_id: Id<Channel>) {
        self.metas.remove(&channel_id);
        self.watched.remove(&channel_id);
        self.hints.remove(&channel_id);
    }
}

impl<D> Runtime<D> {
    /// The relay listed the user's channels, with the `meta` of each whose
    /// key opened. A channel the listing drops is no longer the user's, and
    /// its watch ends. A channel joined since the last listing has segments
    /// no activity answer named yet, so the activity is asked again.
    pub fn channels_listed(&mut self, metas: &[ChannelMeta], names: &[ChannelName]) {
        let fresh: HashMap<Id<Channel>, ChannelMeta> =
            metas.iter().map(|m| (m.channel_id, *m)).collect();
        let listed = &mut self.listed_channels;
        let joined = listed.listed && fresh.keys().any(|c| !listed.metas.contains_key(c));
        let gone: Vec<Id<Channel>> = listed
            .watched
            .iter()
            .filter(|c| !fresh.contains_key(c))
            .copied()
            .collect();
        listed.listed = true;
        listed.hints = fresh
            .keys()
            .map(|channel_id| {
                let hint = names
                    .iter()
                    .find(|n| n.channel_id == *channel_id)
                    .map(|n| n.name.clone());
                (*channel_id, hint)
            })
            .collect();
        listed.metas = fresh;
        for channel_id in gone {
            self.listed_channels.watched.remove(&channel_id);
            self.end_light_channel(channel_id);
        }
        if joined && self.listed_channels.segments.is_some() {
            self.ask(SyncCommand::FetchActivity);
        }
        self.watch_unheld();
        let open: Vec<Id<Channel>> = self.channels.keys().copied().collect();
        for channel_id in open {
            self.reseal_stale_name(channel_id);
        }
    }

    /// Seal the name `meta` reads beside it again when the hint of the latest
    /// listing differs (`MURMURE-682`). A failed seal, two renames racing, or
    /// an older seal the relay served leave a stale hint. An open channel
    /// reads the true name. A channel no listing named, or one not open, seals
    /// nothing.
    pub(crate) fn reseal_stale_name(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.get(&channel_id) else {
            return;
        };
        let (meta, name) = (channel.meta_id(), channel.name());
        let Some(hint) = self.listed_channels.hints.get_mut(&channel_id) else {
            return;
        };
        if *hint == name {
            return;
        }
        // The hint takes the name sealed now, so a later open seals it once.
        hint.clone_from(&name);
        self.ask(SyncCommand::NameChannel { meta, name });
    }

    /// Note the name this client just sealed beside the `meta` of a listed
    /// channel, so an open does not seal it twice.
    pub(crate) fn note_sealed_name(&mut self, channel_id: Id<Channel>, name: &HeldName) {
        if let Some(hint) = self.listed_channels.hints.get_mut(&channel_id) {
            hint.clone_from(name);
        }
    }

    /// The relay's activity answer named the segments of each channel.
    pub(crate) fn segments_listed(&mut self, documents: &[DocumentActivity]) {
        let mut segments: HashMap<Id<Channel>, Segments> = HashMap::new();
        for document in documents
            .iter()
            .filter(|d| d.kind == ChannelDocumentKind::Segment)
        {
            segments
                .entry(document.channel_id)
                .or_default()
                .push((document.document_id, document.last));
        }
        self.listed_channels.segments = Some(segments);
        self.watch_unheld();
    }

    /// Watch each listed channel this session neither holds, opens nor
    /// counts. Each segment counts after the last number the answer gave,
    /// and its `meta` after the listing's, so nothing landing since is
    /// missed (`RESID-022`). A thread is left to its own signal, which
    /// counts its mentions alone. A channel no answer names a segment of
    /// waits.
    fn watch_unheld(&mut self) {
        // A call that started before the watch sent its cue already, so the listing names the
        // live call of each channel (`MURMURE-691`).
        let calls: Vec<(Id<Channel>, Id<Call>)> = self
            .listed_channels
            .metas
            .values()
            .filter_map(|m| Some((m.channel_id, m.call?)))
            .collect();
        let Some(segments) = &self.listed_channels.segments else {
            return;
        };
        let unheld: Vec<(ChannelMeta, Segments)> = self
            .listed_channels
            .metas
            .iter()
            .filter(|(c, _)| {
                !self.channels.contains_key(c)
                    && !self.opening.contains_key(c)
                    && !self.closed_channels.contains_key(c)
            })
            .filter_map(|(c, m)| segments.get(c).map(|s| (*m, s.clone())))
            .collect();
        for (listed, documents) in unheld {
            let ChannelMeta {
                channel_id,
                meta,
                last,
                ..
            } = listed;
            let held = documents
                .iter()
                .map(|(document_id, _)| *document_id)
                .chain([meta.document_id()])
                .collect();
            self.closed_channels
                .insert(channel_id, channels::ClosedChannel::watched(meta, held));
            self.listed_channels.watched.insert(channel_id);
            self.ask(SyncCommand::WatchChannel {
                channel_id,
                meta,
                meta_last: last,
                documents,
            });
        }
        self.channel_calls(&calls);
    }
}
