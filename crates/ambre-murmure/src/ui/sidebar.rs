//! The channel list of the sidebar (the Murmure layout spec, section 3).
//!
//! A value the view rebuilds from what the Logic thread hands it. That is the
//! channels of the user, the catalogue of each open space, the activity marks
//! and the names of open channels. It sends nothing, so drawing the
//! sidebar opens no channel and moves no read cursor.

use std::collections::{HashMap, HashSet};

use crate::message::{CatalogPosition, CatalogueEntry, CategoryView};
use ambre_bus::{
    ChannelActivity, ChannelName, ChannelPlace, DirectPeers, MemberChannel, Mentions, Role, Unread,
};
use ambre_core::StepDirection;
use ambre_core::id::{Category, Channel, Id, Space, User};

/// The section of the sidebar an entry sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarGroup {
    /// The joined public channels of one category of a space. `None` holds
    /// the channels outside every category.
    Category {
        space: Id<Space>,
        category: Option<Id<Category>>,
    },
    /// The private channels the user belongs to, in every space.
    Private,
    /// The direct messages.
    Direct,
    /// The public channels of an open space the user has not joined.
    ToJoin,
}

/// What a channel holds past the read cursor of the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Activity {
    /// Nothing new.
    #[default]
    None,
    /// Something new, before any count is known (`MURMURE-119`).
    Some,
    /// The count a loaded document gives.
    Count {
        messages: ambre_core::Count<Unread>,
        mentions: ambre_core::Count<Mentions>,
    },
}

impl Activity {
    /// The activity a channel answer states.
    pub fn of(answer: &ChannelActivity) -> Self {
        match (answer.unread, answer.mentions) {
            (None, None) if answer.active == ambre_bus::Activity::Active => Self::Some,
            (None, None) => Self::None,
            (messages, mentions) => {
                let messages = messages.unwrap_or(ambre_core::Count::<Unread>::ZERO);
                let mentions = mentions.unwrap_or(ambre_core::Count::<Mentions>::ZERO);
                if messages.is_zero() && mentions == ambre_core::Count::<Mentions>::ZERO {
                    Self::None
                } else {
                    Self::Count { messages, mentions }
                }
            }
        }
    }
}

/// One channel of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarEntry {
    pub channel: Id<Channel>,
    /// Absent until a catalogue or the open channel names it.
    pub name: Option<ambre_core::Validated<ambre_core::Name>>,
    pub activity: Activity,
}

/// One section of the sidebar, in the order it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSection {
    pub group: SidebarGroup,
    /// The category name, or the space name for the channels outside every
    /// category. Absent for the other groups, which the view titles itself.
    pub title: Option<ambre_core::Validated<ambre_core::Name>>,
    pub entries: Vec<SidebarEntry>,
}

/// One space of the user, as the head of its controls in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceHead {
    pub space: Id<Space>,
    /// Absent until its catalogue names it, or when the space has no name.
    pub name: Option<ambre_core::Validated<ambre_core::Name>>,
    /// The role of the user on the space document. Absent until the space
    /// opens, so no control shows before the role is known.
    pub role: Option<Role>,
}

impl SpaceHead {
    /// Whether the user may change the catalogue: an owner or an editor of
    /// the space document (the Murmure spaces spec, `MURMURE-032`).
    pub fn is_admin(&self) -> bool {
        matches!(self.role, Some(Role::Owner | Role::Editor))
    }

    /// Whether the user owns the space document, and so may invite.
    pub fn is_owner(&self) -> bool {
        self.role == Some(Role::Owner)
    }
}

/// The catalogue of one open space.
#[derive(Debug, Clone, Default)]
struct Catalogue {
    name: Option<ambre_core::Validated<ambre_core::Name>>,
    categories: Vec<CategoryView>,
    channels: Vec<CatalogueEntry>,
    /// The role of the user on the space document.
    role: Option<Role>,
}

/// The state the sidebar is drawn from.
///
/// `Debug` counts the peers of each direct message alone: each one is the
/// name a member chose, which `DisplayName` prints whole.
#[derive(Clone, Default)]
pub struct Sidebar {
    members: Vec<MemberChannel>,
    /// The spaces in the order the user's channels first named them.
    spaces: Vec<Id<Space>>,
    catalogues: HashMap<Id<Space>, Catalogue>,
    activity: HashMap<Id<Channel>, Activity>,
    /// What each open channel read in its meta, none when it holds no name.
    names: HashMap<Id<Channel>, Option<ambre_core::Validated<ambre_core::Name>>>,
    /// The names the relay's listing carried sealed, for channels not open
    /// (`MURMURE-490`). What meta reads always wins over them.
    hints: HashMap<Id<Channel>, ambre_core::Validated<ambre_core::Name>>,
    /// The other members of each direct message, by display name, which a
    /// direct message nobody named shows (`MURMURE-492`).
    peers: HashMap<Id<Channel>, Vec<ambre_core::Validated<ambre_core::DisplayName>>>,
    /// The spaces whose open failed. The next listing naming one opens it
    /// anew (`MURMURE-550`).
    unopened: Vec<Id<Space>>,
}

impl std::fmt::Debug for Sidebar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let peers: HashMap<&Id<Channel>, usize> = self
            .peers
            .iter()
            .map(|(channel, names)| (channel, names.len()))
            .collect();
        f.debug_struct("Sidebar")
            .field("members", &self.members)
            .field("spaces", &self.spaces)
            .field("catalogues", &self.catalogues)
            .field("activity", &self.activity)
            .field("names", &self.names)
            .field("hints", &self.hints)
            .field("peers", &peers)
            .field("unopened", &self.unopened)
            .finish()
    }
}

impl Sidebar {
    /// Take the channels and the spaces of the user, whole. Returns the
    /// spaces named for the first time, and those whose open failed, which
    /// the view opens to read their catalogue. A space with no channel of
    /// the user is one of them.
    pub fn set_member_channels(
        &mut self,
        channels: Vec<MemberChannel>,
        spaces: &[Id<Space>],
    ) -> Vec<Id<Space>> {
        let named = channels.iter().filter_map(|channel| match channel.place {
            ChannelPlace::Public(space) | ChannelPlace::Private(space) => Some(space),
            ChannelPlace::Direct => None,
        });
        let mut fresh = Vec::new();
        for space in named.chain(spaces.iter().copied()) {
            if !self.spaces.contains(&space) {
                self.spaces.push(space);
                fresh.push(space);
            } else if self.unopened.contains(&space) && !fresh.contains(&space) {
                fresh.push(space);
            }
        }
        self.unopened.retain(|space| !fresh.contains(space));
        self.members = channels;
        fresh
    }

    /// A space that could not open. Its joined channels stay drawn, and the
    /// next listing naming it opens it anew (`MURMURE-550`).
    pub fn space_failed(&mut self, space: Id<Space>) {
        if self.spaces.contains(&space) && !self.unopened.contains(&space) {
            self.unopened.push(space);
        }
    }

    /// Take the catalogue of an open space, whole.
    pub fn set_catalogue(
        &mut self,
        space: Id<Space>,
        name: Option<ambre_core::Validated<ambre_core::Name>>,
        categories: Vec<CategoryView>,
        channels: Vec<CatalogueEntry>,
    ) {
        if !self.spaces.contains(&space) {
            self.spaces.push(space);
        }
        // A catalogue changes without its role: the role stays.
        let role = self.catalogues.get(&space).and_then(|c| c.role);
        self.catalogues.insert(
            space,
            Catalogue {
                name,
                categories,
                channels,
                role,
            },
        );
    }

    /// Take the role of the user on an open space. A space with no catalogue
    /// yet takes it with its catalogue.
    pub fn set_role(&mut self, space: Id<Space>, role: Role) {
        if let Some(catalogue) = self.catalogues.get_mut(&space) {
            catalogue.role = Some(role);
        }
    }

    /// The spaces of the user, in sidebar order, each with its name and the
    /// role of the user once its catalogue is known (`MURMURE-583`).
    pub fn space_heads(&self) -> Vec<SpaceHead> {
        self.spaces
            .iter()
            .map(|&space| {
                let catalogue = self.catalogues.get(&space);
                SpaceHead {
                    space,
                    name: catalogue.and_then(|c| c.name.clone()),
                    role: catalogue.and_then(|c| c.role),
                }
            })
            .collect()
    }

    /// The rank after every channel the catalogue of `space` lists, for a new
    /// public channel to come last.
    pub fn next_position(&self, space: Id<Space>) -> ambre_bus::Rank<CatalogPosition> {
        self.catalogues
            .get(&space)
            .and_then(|c| c.channels.iter().map(|e| e.position).max())
            .map_or(ambre_bus::Rank::FIRST, ambre_bus::Rank::next)
    }

    /// Take an activity answer. A channel absent from it keeps its mark.
    pub fn set_activity(&mut self, answers: &[ChannelActivity]) {
        for answer in answers {
            self.activity
                .insert(answer.channel_id, Activity::of(answer));
        }
    }

    /// Take the name an open channel read.
    pub fn set_name(
        &mut self,
        channel: Id<Channel>,
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    ) {
        self.names.insert(channel, name);
    }

    /// Take the names the relay's listing carried, whole.
    pub fn set_hints(&mut self, hints: &[ChannelName]) {
        self.hints = hints
            .iter()
            .map(|hint| (hint.channel_id, hint.name.clone()))
            .collect();
    }

    /// Take the other members of each direct message, whole.
    pub fn set_peers(&mut self, peers: &[DirectPeers]) {
        self.peers = peers
            .iter()
            .map(|direct| (direct.channel_id, direct.names.clone()))
            .collect();
    }

    /// Drop a channel the user left, or the relay deleted.
    pub fn forget(&mut self, channel: Id<Channel>) {
        self.members.retain(|m| m.channel_id != channel);
        self.activity.remove(&channel);
        self.names.remove(&channel);
        self.hints.remove(&channel);
        self.peers.remove(&channel);
    }

    /// Drop a space the user lost, with its catalogue and every channel of
    /// it, joined or to join. Returns the channels dropped (`MURMURE-488`).
    pub fn forget_space(&mut self, space: Id<Space>) -> Vec<Id<Channel>> {
        self.spaces.retain(|s| *s != space);
        let mut dropped: Vec<Id<Channel>> = self
            .catalogues
            .remove(&space)
            .map(|c| c.channels.iter().map(|e| e.channel_id).collect())
            .unwrap_or_default();
        dropped.extend(
            self.members
                .iter()
                .filter(|m| {
                    matches!(m.place, ChannelPlace::Public(s) | ChannelPlace::Private(s) if s == space)
                })
                .map(|m| m.channel_id),
        );
        dropped.sort_unstable_by_key(|c| *c.as_bytes());
        dropped.dedup();
        for &channel in &dropped {
            self.forget(channel);
        }
        dropped
    }

    /// Whether the user belongs to the channel.
    pub fn is_member(&self, channel: Id<Channel>) -> bool {
        self.members.iter().any(|m| m.channel_id == channel)
    }

    /// Whether a channel of the user is a direct message.
    pub fn is_direct(&self, channel: Id<Channel>) -> bool {
        self.members
            .iter()
            .any(|m| m.channel_id == channel && m.place == ChannelPlace::Direct)
    }

    /// The name the sidebar shows for a channel, if it knows one. That is
    /// what its meta read once open, else the relay's hint, else its
    /// catalogue entry. A channel whose meta reads no name drops its hint.
    /// A direct message with no name shows its other members, composed here
    /// and never stored (`MURMURE-492`).
    pub fn name_of(&self, channel: Id<Channel>) -> Option<ambre_core::Validated<ambre_core::Name>> {
        let read = match self.names.get(&channel) {
            Some(read) => read.clone(),
            None => self.hints.get(&channel).cloned(),
        };
        read.or_else(|| {
            self.catalogues
                .values()
                .flat_map(|c| &c.channels)
                .find(|e| e.channel_id == channel)
                .map(|e| e.name.clone())
        })
        .or_else(|| {
            let peers = self.peers.get(&channel)?;
            (!peers.is_empty()).then(|| {
                let joined = peers
                    .iter()
                    .map(ambre_core::Validated::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                ambre_core::cut_name_to_fit(&joined)
            })
        })
    }

    /// The space whose members a public channel shows: the place the listing
    /// gave a joined one, else the catalogue that lists it. A private channel
    /// or a direct message has members of its own, so it shows none.
    pub fn public_space_of(&self, channel: Id<Channel>) -> Option<Id<Space>> {
        let joined = self.members.iter().find(|m| m.channel_id == channel);
        match joined.map(|m| m.place) {
            Some(ChannelPlace::Public(space)) => Some(space),
            Some(ChannelPlace::Private(_) | ChannelPlace::Direct) => None,
            None => self.listed_in(channel),
        }
    }

    /// Where a channel of the user lies, as the listing gave it.
    pub fn place_of(&self, channel: Id<Channel>) -> Option<ChannelPlace> {
        self.members
            .iter()
            .find(|m| m.channel_id == channel)
            .map(|m| m.place)
    }

    /// The spaces of the user, in the order the sidebar draws them.
    pub fn spaces(&self) -> &[Id<Space>] {
        &self.spaces
    }

    /// The open space whose catalogue lists the channel, to join it there.
    pub fn listed_in(&self, channel: Id<Channel>) -> Option<Id<Space>> {
        self.catalogues
            .iter()
            .find(|(_, c)| c.channels.iter().any(|e| e.channel_id == channel))
            .map(|(space, _)| *space)
    }

    fn entry(&self, channel: Id<Channel>) -> SidebarEntry {
        SidebarEntry {
            channel,
            name: self.name_of(channel),
            activity: self.activity.get(&channel).copied().unwrap_or_default(),
        }
    }

    /// The sections, in the order they are drawn. The joined public channels
    /// of each space come by category, then the private channels and the
    /// direct messages (`MURMURE-163`). The channels to join come apart
    /// (`MURMURE-164`). An empty section is left out.
    pub fn sections(&self) -> Vec<SidebarSection> {
        let mut sections = Vec::new();
        for &space in &self.spaces {
            let catalogue = self.catalogues.get(&space);
            let joined: Vec<Id<Channel>> = self
                .members
                .iter()
                .filter(|m| m.place == ChannelPlace::Public(space))
                .map(|m| m.channel_id)
                .collect();
            let listed = |id: Id<Channel>| {
                catalogue.and_then(|c| c.channels.iter().find(|e| e.channel_id == id))
            };
            let mut categories: Vec<&CategoryView> = catalogue
                .map(|c| c.categories.iter().collect())
                .unwrap_or_default();
            ambre_core::order::sort_by_key_then_id(&mut categories, |c| (c.position, c.id));
            let groups = std::iter::once((None, catalogue.and_then(|c| c.name.clone()))).chain(
                categories
                    .iter()
                    .map(|c| (Some(c.id), Some(c.name.clone()))),
            );
            for (category, title) in groups {
                let mut channels: Vec<Id<Channel>> = joined
                    .iter()
                    .copied()
                    .filter(|&id| listed(id).and_then(|e| e.category) == category)
                    .collect();
                channels.sort_by_key(|&id| rank(listed(id), id));
                push(
                    &mut sections,
                    SidebarGroup::Category { space, category },
                    title,
                    channels.into_iter().map(|id| self.entry(id)).collect(),
                );
            }
        }
        for (group, place) in [
            (SidebarGroup::Private, None),
            (SidebarGroup::Direct, Some(ChannelPlace::Direct)),
        ] {
            let mut entries: Vec<SidebarEntry> = self
                .members
                .iter()
                .filter(|m| match place {
                    None => matches!(m.place, ChannelPlace::Private(_)),
                    Some(place) => m.place == place,
                })
                .map(|m| self.entry(m.channel_id))
                .collect();
            entries.sort_by(|a, b| {
                (a.name.is_none(), &a.name, a.channel.as_bytes()).cmp(&(
                    b.name.is_none(),
                    &b.name,
                    b.channel.as_bytes(),
                ))
            });
            push(&mut sections, group, None, entries);
        }
        let mut to_join = Vec::new();
        for space in &self.spaces {
            if let Some(catalogue) = self.catalogues.get(space) {
                let mut open: Vec<&CatalogueEntry> = catalogue
                    .channels
                    .iter()
                    .filter(|e| !self.is_member(e.channel_id))
                    .collect();
                open.sort_by_key(|e| rank(Some(e), e.channel_id));
                to_join.extend(open.into_iter().map(|e| self.entry(e.channel_id)));
            }
        }
        push(&mut sections, SidebarGroup::ToJoin, None, to_join);
        sections
    }

    /// The space the sidebar shows: the one the user chose while it is still theirs, else the
    /// first (`MURMURE-711`).
    #[must_use]
    pub fn shown_space(&self, chosen: Option<Id<Space>>) -> Option<Id<Space>> {
        chosen
            .filter(|space| self.spaces.contains(space))
            .or_else(|| self.spaces.first().copied())
    }

    /// The sections of the space `space` alone: its categories, its private channels and its
    /// channels to join, with the direct messages of every space (`MURMURE-163`).
    #[must_use]
    pub fn sections_in(&self, space: Option<Id<Space>>) -> Vec<SidebarSection> {
        let mine = |channel: Id<Channel>| match self.place_of(channel) {
            Some(ChannelPlace::Public(s) | ChannelPlace::Private(s)) => Some(s) == space,
            Some(ChannelPlace::Direct) => true,
            None => self.listed_in(channel) == space,
        };
        self.sections()
            .into_iter()
            .filter_map(|mut section| {
                match section.group {
                    SidebarGroup::Category { space: s, .. } if Some(s) != space => return None,
                    SidebarGroup::Category { .. } | SidebarGroup::Direct => {}
                    SidebarGroup::Private | SidebarGroup::ToJoin => {
                        section.entries.retain(|e| mine(e.channel));
                    }
                }
                (!section.entries.is_empty()).then_some(section)
            })
            .collect()
    }

    /// The channel a jump opens: the next one with activity after `from` in
    /// sidebar order, or the previous one, wrapping round (`MURMURE-176`).
    /// A channel to join is never one, so a jump joins nothing. `None` when
    /// no other channel has activity.
    pub fn next_with_activity(
        &self,
        from: Option<Id<Channel>>,
        direction: StepDirection,
    ) -> Option<Id<Channel>> {
        let mut order: Vec<SidebarEntry> = self
            .sections()
            .into_iter()
            .filter(|s| s.group != SidebarGroup::ToJoin)
            .flat_map(|s| s.entries)
            .collect();
        if direction == StepDirection::Backward {
            order.reverse();
        }
        let start = from
            .and_then(|from| order.iter().position(|e| e.channel == from))
            .map_or(0, |at| at + 1);
        let (before, after) = order.split_at(start.min(order.len()));
        after
            .iter()
            .chain(before)
            .find(|e| e.activity != Activity::None && Some(e.channel) != from)
            .map(|e| e.channel)
    }
}

/// Where a public channel stands in the catalogue: at its rank, or after
/// every listed channel while the catalogue does not list it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Placed {
    At(ambre_bus::Rank<CatalogPosition>),
    Unlisted,
}

/// The order of a public channel: where it stands in the catalogue, ties
/// by the bytes of its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChannelOrder {
    placed: Placed,
    channel: Id<Channel>,
}

impl Ord for ChannelOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.placed
            .cmp(&other.placed)
            .then_with(|| self.channel.as_bytes().cmp(other.channel.as_bytes()))
    }
}

impl PartialOrd for ChannelOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The rank of the public channel `channel`, listed by `entry` or not yet.
fn rank(entry: Option<&CatalogueEntry>, channel: Id<Channel>) -> ChannelOrder {
    let placed = entry.map_or(Placed::Unlisted, |e| Placed::At(e.position));
    ChannelOrder { placed, channel }
}

fn push(
    sections: &mut Vec<SidebarSection>,
    group: SidebarGroup,
    title: Option<ambre_core::Validated<ambre_core::Name>>,
    entries: Vec<SidebarEntry>,
) {
    if !entries.is_empty() {
        sections.push(SidebarSection {
            group,
            title,
            entries,
        });
    }
}

/// The rank of a space in the switcher, counted from zero: `Pos<SpaceRanks>` (`MURMURE-712`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpaceRanks {}

impl ambre_core::index::Axis for SpaceRanks {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SpaceRank");
}

/// A rank as the switcher names it: counted from one, as its chord.
impl ambre_core::OrdinalKind for SpaceRanks {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SpaceRankNumber");
}

/// The number the chord of a rank carries: its index plus one.
impl ambre_core::index::Numbered for SpaceRanks {
    type Number = ambre_core::Ordinal<SpaceRanks>;

    fn number_of(pos: ambre_core::index::Pos<SpaceRanks>) -> ambre_core::Ordinal<SpaceRanks> {
        ambre_core::Ordinal::<SpaceRanks>::of_index(pos)
    }
}

/// The switcher shows the number of a rank in its chord.
impl ambre_core::index::Shown for SpaceRanks {}

/// The tab of the bar on the bottom edge of the channel list of a phone (`MURMURE-715`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhoneTab {
    /// Every group of the space shown.
    #[default]
    Channels,
    /// The direct messages alone.
    Direct,
    /// The search over the loaded segments.
    Search,
}

impl PhoneTab {
    /// The three tabs, in the order the bar shows them.
    pub const ALL: [Self; 3] = [Self::Channels, Self::Direct, Self::Search];

    /// The name of the tab, which its button shows and announces (`MURMURE-719`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Channels => "Channels",
            Self::Direct => "Direct messages",
            Self::Search => "Search",
        }
    }

    /// The word of the tab in the markup, which the stylesheet reads for its icon.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Channels => "channels",
            Self::Direct => "direct",
            Self::Search => "search",
        }
    }

    /// The sections the channel list shows under this tab: every one, or the direct messages
    /// alone (`MURMURE-717`). The search tab keeps the list under the search box.
    #[must_use]
    pub fn keeps(self, group: SidebarGroup) -> bool {
        match self {
            Self::Channels | Self::Search => true,
            Self::Direct => group == SidebarGroup::Direct,
        }
    }
}

/// A count of the channels a folded group hides: `Count<Hidden>`.
pub enum Hidden {}

impl ambre_core::NarrowCount for Hidden {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HiddenChannels");
}

/// Whether a group of the sidebar shows every channel, or is folded (`MURMURE-720`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folding {
    /// Every channel of the group shows.
    Open,
    /// The group shows only the channels it must keep in view (`MURMURE-721`).
    Folded,
}

impl Folding {
    /// The value of `aria-expanded` on the head of the group (`MURMURE-725`).
    #[must_use]
    pub const fn expanded(self) -> &'static str {
        match self {
            Self::Open => "true",
            Self::Folded => "false",
        }
    }
}

/// Whether a channel a folded group hides has activity: its head then carries a mark
/// (`MURMURE-722`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HiddenMark {
    /// No hidden channel has activity.
    #[default]
    Quiet,
    /// A hidden channel has activity.
    Active,
}

/// The groups the user folded. The page keeps them until the settings document of the user
/// holds them (`MURMURE-723`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds(HashSet<SidebarGroup>);

impl Folds {
    /// The fold of `group`.
    #[must_use]
    pub fn of(&self, group: SidebarGroup) -> Folding {
        if self.0.contains(&group) {
            Folding::Folded
        } else {
            Folding::Open
        }
    }

    /// Folds `group`, or unfolds it when it is folded: a press on its head (`MURMURE-720`).
    pub fn toggle(&mut self, group: SidebarGroup) {
        if !self.0.remove(&group) {
            self.0.insert(group);
        }
    }
}

/// What a folded group keeps in view: the channel the view shows and the channels with a live
/// call (`MURMURE-721`). A channel that mentions the user stays too, read from its activity.
#[derive(Debug, Clone, Copy)]
pub struct Kept<'k> {
    /// The channel the view shows.
    pub shown: Option<Id<Channel>>,
    /// The live calls the page knows, by channel.
    pub calls: &'k LiveCalls,
}

impl Kept<'_> {
    /// Whether a folded group keeps `entry` in view.
    fn keeps(&self, entry: &SidebarEntry) -> bool {
        let mentions = matches!(
            entry.activity,
            Activity::Count { mentions, .. } if mentions != ambre_core::Count::<Mentions>::ZERO
        );
        Some(entry.channel) == self.shown || self.calls.of(entry.channel).is_some() || mentions
    }
}

/// One group of the sidebar as it is drawn: its entries once folded, and what its head says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawnSection {
    /// The group, its title and the entries it shows.
    pub section: SidebarSection,
    /// Whether the group is folded.
    pub folding: Folding,
    /// How many channels the fold hides.
    pub hidden: ambre_core::Count<Hidden>,
    /// Whether one of them has activity.
    pub mark: HiddenMark,
}

/// The sections as they are drawn under `folds`. A folded group keeps what `kept` names, counts
/// the channels it hides and marks a hidden one with activity (`MURMURE-721`, `MURMURE-722`).
#[must_use]
pub fn fold(sections: Vec<SidebarSection>, folds: &Folds, kept: Kept<'_>) -> Vec<DrawnSection> {
    sections
        .into_iter()
        .map(|mut section| {
            let folding = folds.of(section.group);
            if folding == Folding::Open {
                return DrawnSection {
                    section,
                    folding,
                    hidden: ambre_core::Count::<Hidden>::ZERO,
                    mark: HiddenMark::Quiet,
                };
            }
            let (shown, gone): (Vec<SidebarEntry>, Vec<SidebarEntry>) =
                section.entries.into_iter().partition(|e| kept.keeps(e));
            section.entries = shown;
            let mark = if gone.iter().any(|e| e.activity != Activity::None) {
                HiddenMark::Active
            } else {
                HiddenMark::Quiet
            };
            DrawnSection {
                section,
                folding,
                hidden: ambre_core::Count::<Hidden>::of(gone.len()),
                mark,
            }
        })
        .collect()
}

/// The live call of a channel the sidebar marks, as the relay last answered who is in it
/// (`MURMURE-689`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCall {
    /// The members seated in the call, each once.
    pub members: Vec<Id<User>>,
}

/// A count of the participants of a live call, which the sidebar shows: `Count<Seated>`.
pub enum Seated {}

impl ambre_core::NarrowCount for Seated {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SeatedParticipants");
}

impl LiveCall {
    /// The count the sidebar shows beside the call mark (`MURMURE-689`).
    #[must_use]
    pub fn count(&self) -> ambre_core::Count<Seated> {
        ambre_core::Count::<Seated>::of(self.members.len())
    }
}

/// The live calls the page knows, by channel: the answers of the relay to the asks of who is in
/// each call (`MURMURE-595`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveCalls(HashMap<Id<Channel>, LiveCall>);

impl LiveCalls {
    /// Takes the answer `members` for the call of `channel`. A call with nobody seated, or one
    /// the relay holds no state of, leaves the sidebar.
    pub fn answer(&mut self, channel: Id<Channel>, members: &ambre_bus::CallMembers) {
        match members {
            ambre_bus::CallMembers::Live(seated) if !seated.is_empty() => {
                self.0.insert(
                    channel,
                    LiveCall {
                        members: seated.clone(),
                    },
                );
            }
            ambre_bus::CallMembers::Live(_) | ambre_bus::CallMembers::Gone => {
                self.0.remove(&channel);
            }
        }
    }

    /// The live call of `channel`, if the page knows one.
    #[must_use]
    pub fn of(&self, channel: Id<Channel>) -> Option<&LiveCall> {
        self.0.get(&channel)
    }

    /// Forgets the call of `channel`: the channel left the sidebar.
    pub fn forget(&mut self, channel: Id<Channel>) {
        self.0.remove(&channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug form of the sidebar counts the peers of a direct message,
    /// and names none of them.
    #[test]
    fn the_sidebar_debugs_its_peer_count_alone() {
        let mut sidebar = Sidebar::default();
        let names = vec![
            ambre_core::Validated::<ambre_core::DisplayName>::parse("Secretary").expect("a name"),
            ambre_core::Validated::<ambre_core::DisplayName>::parse("Treasurer").expect("a name"),
        ];
        let channel_id = Id::<Channel>::mint();
        sidebar.set_peers(&[DirectPeers { channel_id, names }]);
        let shown = format!("{sidebar:?}");
        assert!(
            !shown.contains("Secretary") && !shown.contains("Treasurer"),
            "{shown}"
        );
        assert!(shown.contains(&format!("{channel_id:?}: 2")), "{shown}");
    }

    fn public(space: Id<Space>) -> (Id<Channel>, MemberChannel) {
        let channel_id = Id::<Channel>::mint();
        let place = ChannelPlace::Public(space);
        (channel_id, MemberChannel { channel_id, place })
    }

    fn listed(
        channel_id: Id<Channel>,
        name: &str,
        category: Option<Id<Category>>,
        position: u32,
    ) -> CatalogueEntry {
        CatalogueEntry {
            channel_id,
            name: ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
            topic: None,
            category,
            position: ambre_bus::Rank::new(position),
        }
    }

    fn names(section: &SidebarSection) -> Vec<&str> {
        section
            .entries
            .iter()
            .map(|e| {
                e.name
                    .as_ref()
                    .map(ambre_core::Validated::as_str)
                    .unwrap_or("?")
            })
            .collect()
    }

    /// A joined public channel shows the members of the space of its place,
    /// one to join those of the space that lists it. A private channel and a
    /// direct message show none.
    #[test]
    fn a_public_channel_shows_the_members_of_its_space_and_no_other_channel_does() {
        let space = Id::<Space>::mint();
        let (general, general_m) = public(space);
        let private = Id::<Channel>::mint();
        let direct = Id::<Channel>::mint();
        let to_join = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(
            vec![
                general_m,
                MemberChannel {
                    channel_id: private,
                    place: ChannelPlace::Private(space),
                },
                MemberChannel {
                    channel_id: direct,
                    place: ChannelPlace::Direct,
                },
            ],
            &[],
        );
        sidebar.set_catalogue(
            space,
            None,
            vec![],
            vec![listed(to_join, "random", None, 0)],
        );
        assert_eq!(sidebar.public_space_of(general), Some(space));
        assert_eq!(sidebar.public_space_of(to_join), Some(space));
        assert_eq!(sidebar.public_space_of(private), None);
        assert_eq!(sidebar.public_space_of(direct), None);
        assert_eq!(sidebar.public_space_of(Id::<Channel>::mint()), None);
        assert_eq!(sidebar.spaces(), [space]);
    }

    /// The joined public channels of a space are grouped by category, in
    /// the rank of the catalogue. The private channels and the direct
    /// messages follow.
    #[test]
    fn the_sidebar_groups_public_channels_by_category_then_private_then_direct() {
        let space = Id::<Space>::mint();
        let (general, general_m) = public(space);
        let (sales, sales_m) = public(space);
        let (leads, leads_m) = public(space);
        let private = Id::<Channel>::mint();
        let direct = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        let fresh = sidebar.set_member_channels(
            vec![
                leads_m,
                MemberChannel {
                    channel_id: direct,
                    place: ChannelPlace::Direct,
                },
                sales_m,
                MemberChannel {
                    channel_id: private,
                    place: ChannelPlace::Private(space),
                },
                general_m,
            ],
            &[space],
        );
        assert_eq!(fresh, vec![space], "the space is opened once");
        let team = Id::<Category>::mint();
        sidebar.set_catalogue(
            space,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("Acme").unwrap()),
            vec![CategoryView {
                id: team,
                name: ambre_core::Validated::<ambre_core::Name>::parse("Team").unwrap(),
                position: ambre_bus::Rank::new(0),
            }],
            vec![
                listed(leads, "leads", Some(team), 2),
                listed(sales, "sales", Some(team), 1),
                listed(general, "general", None, 0),
            ],
        );
        sidebar.set_name(
            private,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("board").unwrap()),
        );

        let sections = sidebar.sections();
        let groups: Vec<SidebarGroup> = sections.iter().map(|s| s.group).collect();
        assert_eq!(
            groups,
            vec![
                SidebarGroup::Category {
                    space,
                    category: None
                },
                SidebarGroup::Category {
                    space,
                    category: Some(team)
                },
                SidebarGroup::Private,
                SidebarGroup::Direct,
            ]
        );
        assert_eq!(
            sections[0]
                .title
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Acme")
        );
        assert_eq!(names(&sections[0]), ["general"]);
        assert_eq!(
            sections[1]
                .title
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Team")
        );
        assert_eq!(names(&sections[1]), ["sales", "leads"]);
        assert_eq!(names(&sections[2]), ["board"]);
        assert_eq!(sections[3].entries[0].channel, direct);
        assert_eq!(
            sections[3].entries[0].name, None,
            "a closed direct message has no name yet"
        );
        assert!(
            sidebar
                .set_member_channels(sidebar.members.clone(), &[space])
                .is_empty()
        );
    }

    /// A channel the catalogue does not list yet comes after every listed
    /// one, even one at the last rank a position holds.
    #[test]
    fn an_unlisted_channel_comes_after_one_listed_at_the_last_rank() {
        let space = Id::<Space>::mint();
        let last = Id::<Channel>::from_bytes([0xff; 16]);
        let unlisted = Id::<Channel>::from_bytes([0; 16]);
        let place = ChannelPlace::Public(space);
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(
            vec![
                MemberChannel {
                    channel_id: unlisted,
                    place,
                },
                MemberChannel {
                    channel_id: last,
                    place,
                },
            ],
            &[space],
        );
        sidebar.set_catalogue(
            space,
            None,
            Vec::new(),
            vec![listed(last, "last", None, u32::MAX)],
        );
        let order: Vec<Id<Channel>> = sidebar.sections()[0]
            .entries
            .iter()
            .map(|e| e.channel)
            .collect();
        assert_eq!(order, [last, unlisted]);
    }

    /// A public channel the user has not joined sits apart, after every
    /// joined one, and leaves that section once joined (`MURMURE-164`).
    #[test]
    fn a_channel_to_join_sits_apart_from_the_joined_ones() {
        let space = Id::<Space>::mint();
        let (general, general_m) = public(space);
        let random = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![general_m], &[space]);
        sidebar.set_catalogue(
            space,
            None,
            Vec::new(),
            vec![
                listed(random, "random", None, 1),
                listed(general, "general", None, 0),
            ],
        );
        let sections = sidebar.sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(names(&sections[0]), ["general"]);
        assert_eq!(sections[1].group, SidebarGroup::ToJoin);
        assert_eq!(names(&sections[1]), ["random"]);
        assert_eq!(
            sidebar.listed_in(random),
            Some(space),
            "joined in its space"
        );
        assert_eq!(sidebar.listed_in(Id::<Channel>::mint()), None);

        sidebar.set_member_channels(
            vec![
                general_m,
                MemberChannel {
                    channel_id: random,
                    place: ChannelPlace::Public(space),
                },
            ],
            &[space],
        );
        let sections = sidebar.sections();
        assert_eq!(sections.len(), 1, "nothing is left to join");
        assert_eq!(names(&sections[0]), ["general", "random"]);
    }

    /// A space with no channel of the user still opens, and its public
    /// channels show to join. A space named twice opens once
    /// (`MURMURE-486`).
    #[test]
    fn a_space_with_no_channel_of_the_user_shows_its_channels_to_join() {
        let (space, other) = (Id::<Space>::mint(), Id::<Space>::mint());
        let (general, general_m) = public(other);
        let random = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        let fresh = sidebar.set_member_channels(vec![general_m], &[space, other]);
        assert_eq!(fresh, vec![other, space], "each space opens once");
        sidebar.set_catalogue(
            space,
            None,
            Vec::new(),
            vec![listed(random, "random", None, 0)],
        );
        sidebar.set_catalogue(
            other,
            None,
            Vec::new(),
            vec![listed(general, "general", None, 0)],
        );
        let sections = sidebar.sections();
        assert_eq!(names(&sections[0]), ["general"]);
        assert_eq!(sections[1].group, SidebarGroup::ToJoin);
        assert_eq!(names(&sections[1]), ["random"]);
        assert_eq!(sidebar.listed_in(random), Some(space));
        assert!(
            sidebar
                .set_member_channels(vec![general_m], &[space, other])
                .is_empty()
        );
    }

    /// A space that could not open keeps its joined channels drawn, and the
    /// next listing naming it opens it once more. A space never listed, or
    /// failed twice, is asked no more than once (`MURMURE-550`).
    #[test]
    fn a_space_that_failed_opens_anew_on_the_next_listing() {
        let space = Id::<Space>::mint();
        let (general, general_m) = public(space);
        let mut sidebar = Sidebar::default();
        assert_eq!(
            sidebar.set_member_channels(vec![general_m], &[space]),
            [space]
        );
        sidebar.space_failed(space);
        sidebar.space_failed(space);
        sidebar.space_failed(Id::<Space>::mint());
        let drawn = sidebar.sections();
        assert_eq!(drawn[0].entries[0].channel, general, "still drawn");
        assert_eq!(
            sidebar.set_member_channels(vec![general_m], &[space]),
            [space],
            "opened anew, once"
        );
        assert!(
            sidebar
                .set_member_channels(vec![general_m], &[space])
                .is_empty()
        );
        assert_eq!(sidebar.spaces(), [space]);
    }

    /// A space the user lost leaves the sidebar with its catalogue, its
    /// joined channels, public and private, and those to join. Another space
    /// and the direct messages stay (`MURMURE-488`).
    #[test]
    fn a_space_lost_leaves_the_sidebar_with_every_channel_of_it() {
        let (space, other) = (Id::<Space>::mint(), Id::<Space>::mint());
        let (general, general_m) = public(space);
        let (kept, kept_m) = public(other);
        let (random, private, direct) = (
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
        );
        let mut sidebar = Sidebar::default();
        let place = |channel_id, place| MemberChannel { channel_id, place };
        sidebar.set_member_channels(
            vec![
                general_m,
                kept_m,
                place(private, ChannelPlace::Private(space)),
                place(direct, ChannelPlace::Direct),
            ],
            &[space, other],
        );
        sidebar.set_catalogue(
            space,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("Acme").unwrap()),
            Vec::new(),
            vec![
                listed(general, "general", None, 0),
                listed(random, "random", None, 1),
            ],
        );
        sidebar.set_name(
            private,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("board").unwrap()),
        );

        let mut dropped = sidebar.forget_space(space);
        dropped.sort_unstable_by_key(|c| *c.as_bytes());
        let mut expected = vec![general, random, private];
        expected.sort_unstable_by_key(|c| *c.as_bytes());
        assert_eq!(dropped, expected);
        let shown: Vec<Id<Channel>> = sidebar
            .sections()
            .iter()
            .flat_map(|s| s.entries.iter().map(|e| e.channel))
            .collect();
        assert_eq!(shown, vec![kept, direct]);
        assert_eq!(sidebar.listed_in(random), None);
        assert_eq!(sidebar.name_of(private), None);
        assert!(sidebar.forget_space(space).is_empty(), "already gone");
    }

    /// The relay's hint names a channel not open, and a later listing
    /// replaces the hints whole. What meta reads once open wins, even no
    /// name. A forgotten channel keeps no hint (`MURMURE-490`).
    #[test]
    fn a_hint_names_a_channel_until_its_meta_reads_one() {
        let space = Id::<Space>::mint();
        let (board, other) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let hint = |channel_id, name: &str| ChannelName {
            channel_id,
            name: ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
        };
        let mut sidebar = Sidebar::default();
        let place = |channel_id, place| MemberChannel { channel_id, place };
        sidebar.set_member_channels(
            vec![
                place(board, ChannelPlace::Private(space)),
                place(other, ChannelPlace::Direct),
            ],
            &[space],
        );
        sidebar.set_hints(&[hint(board, "board"), hint(other, "trip")]);
        assert_eq!(
            sidebar
                .name_of(board)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("board")
        );
        sidebar.set_hints(&[hint(board, "plans")]);
        assert_eq!(
            sidebar
                .name_of(board)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("plans")
        );
        assert_eq!(sidebar.name_of(other), None, "replaced whole");

        sidebar.set_name(
            board,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("roadmap").unwrap()),
        );
        assert_eq!(
            sidebar
                .name_of(board)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("roadmap")
        );
        sidebar.set_name(board, None);
        assert_eq!(sidebar.name_of(board), None, "meta reads no name");

        sidebar.set_hints(&[hint(other, "trip")]);
        sidebar.forget(other);
        assert_eq!(sidebar.name_of(other), None);
    }

    /// A direct message nobody named shows its other members, in the
    /// relay's order. A name, hinted or read, wins over them, and meta
    /// reading no name brings them back. A forgotten one keeps none
    /// (`MURMURE-492`).
    #[test]
    fn an_unnamed_direct_message_shows_its_other_members() {
        let (direct, lonely) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let mut sidebar = Sidebar::default();
        let place = |channel_id| MemberChannel {
            channel_id,
            place: ChannelPlace::Direct,
        };
        sidebar.set_member_channels(vec![place(direct), place(lonely)], &[]);
        let peers = |channel_id, names: &[&str]| DirectPeers {
            channel_id,
            names: names
                .iter()
                .map(|n| ambre_core::Validated::<ambre_core::DisplayName>::parse(*n).unwrap())
                .collect(),
        };
        sidebar.set_peers(&[peers(direct, &["Bob", "Carol"]), peers(lonely, &[])]);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Bob, Carol")
        );
        assert_eq!(sidebar.name_of(lonely), None, "no other member");

        sidebar.set_hints(&[ChannelName {
            channel_id: direct,
            name: ambre_core::Validated::<ambre_core::Name>::parse("trip").unwrap(),
        }]);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("trip")
        );
        sidebar.set_name(direct, None);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Bob, Carol")
        );
        sidebar.set_name(
            direct,
            Some(ambre_core::Validated::<ambre_core::Name>::parse("plans").unwrap()),
        );
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("plans")
        );

        sidebar.forget(direct);
        assert_eq!(sidebar.name_of(direct), None);
    }

    /// A channel absent from a later listing leaves the sidebar, never
    /// opened or not: the listing is whole (`MURMURE-489`).
    #[test]
    fn a_channel_absent_from_a_listing_leaves_the_sidebar() {
        let space = Id::<Space>::mint();
        let (general, general_m) = public(space);
        let board = MemberChannel {
            channel_id: Id::<Channel>::mint(),
            place: ChannelPlace::Private(space),
        };
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![general_m, board], &[space]);
        assert!(sidebar.is_member(board.channel_id));
        sidebar.set_member_channels(vec![general_m], &[space]);
        assert!(!sidebar.is_member(board.channel_id));
        assert!(sidebar.is_member(general));
        let shown: Vec<SidebarGroup> = sidebar.sections().iter().map(|s| s.group).collect();
        assert!(!shown.contains(&SidebarGroup::Private), "{shown:?}");
    }

    /// The relay's mark shows before any count, and a count replaces it once
    /// known (`MURMURE-119`, `MURMURE-165`). A channel absent from a later
    /// answer keeps its own.
    #[test]
    fn a_mark_shows_before_the_count_and_the_count_replaces_it() {
        let space = Id::<Space>::mint();
        let (a, a_m) = public(space);
        let (b, b_m) = public(space);
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![a_m, b_m], &[space]);
        let answer =
            |channel_id, active, unread: Option<u32>, mentions: Option<u32>| ChannelActivity {
                channel_id,
                active,
                unread: unread.map(ambre_core::Count::<Unread>::new),
                mentions: mentions.map(ambre_core::Count::<Mentions>::new),
            };
        let of = |sidebar: &Sidebar, id| sidebar.entry(id).activity;
        assert_eq!(of(&sidebar, a), Activity::None);

        sidebar.set_activity(&[
            answer(a, ambre_bus::Activity::Active, None, None),
            answer(b, ambre_bus::Activity::Quiet, None, None),
        ]);
        assert_eq!(of(&sidebar, a), Activity::Some);
        assert_eq!(of(&sidebar, b), Activity::None);

        sidebar.set_activity(&[answer(a, ambre_bus::Activity::Active, Some(3), Some(1))]);
        assert_eq!(
            of(&sidebar, a),
            Activity::Count {
                messages: ambre_core::Count::<Unread>::new(3),
                mentions: ambre_core::Count::<Mentions>::new(1)
            }
        );
        sidebar.set_activity(&[answer(b, ambre_bus::Activity::Active, None, None)]);
        assert!(
            matches!(of(&sidebar, a), Activity::Count { .. }),
            "a keeps its count"
        );
        sidebar.set_activity(&[answer(a, ambre_bus::Activity::Quiet, Some(0), Some(0))]);
        assert_eq!(
            of(&sidebar, a),
            Activity::None,
            "a read channel shows nothing"
        );

        sidebar.forget(b);
        assert!(!sidebar.is_member(b));
        assert_eq!(of(&sidebar, b), Activity::None);
    }

    /// A jump opens the next channel with activity in sidebar order, or the
    /// previous one, wrapping round. It skips the quiet ones, the one it
    /// starts from and the channels to join (`MURMURE-176`).
    #[test]
    fn a_jump_opens_the_next_channel_with_activity_in_sidebar_order() {
        let space = Id::<Space>::mint();
        let (a, a_m) = public(space);
        let (b, b_m) = public(space);
        let (c, c_m) = public(space);
        let open = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![c_m, a_m, b_m], &[space]);
        sidebar.set_catalogue(
            space,
            None,
            Vec::new(),
            vec![
                listed(a, "a", None, 0),
                listed(b, "b", None, 1),
                listed(c, "c", None, 2),
                listed(open, "open", None, 3),
            ],
        );
        let active = |channel_id| ChannelActivity {
            channel_id,
            active: ambre_bus::Activity::Active,
            unread: None,
            mentions: None,
        };
        assert_eq!(
            sidebar.next_with_activity(None, StepDirection::Forward),
            None
        );

        sidebar.set_activity(&[active(a), active(c), active(open)]);
        assert_eq!(
            sidebar.next_with_activity(None, StepDirection::Forward),
            Some(a)
        );
        assert_eq!(
            sidebar.next_with_activity(Some(a), StepDirection::Forward),
            Some(c)
        );
        assert_eq!(
            sidebar.next_with_activity(Some(b), StepDirection::Forward),
            Some(c)
        );
        assert_eq!(
            sidebar.next_with_activity(Some(c), StepDirection::Forward),
            Some(a),
            "it wraps"
        );
        assert_eq!(
            sidebar.next_with_activity(Some(a), StepDirection::Backward),
            Some(c)
        );
        assert_eq!(
            sidebar.next_with_activity(Some(c), StepDirection::Backward),
            Some(a)
        );
        assert_eq!(
            sidebar.next_with_activity(None, StepDirection::Backward),
            Some(c)
        );

        sidebar.set_activity(&[ChannelActivity {
            active: ambre_bus::Activity::Quiet,
            ..active(c)
        }]);
        assert_eq!(
            sidebar.next_with_activity(Some(a), StepDirection::Forward),
            None,
            "the channel it starts from is not a jump"
        );
    }

    /// Two spaces of the user, with a public and a private channel in each. One direct message,
    /// and one channel to join in the second space.
    fn two_spaces() -> (Sidebar, [Id<Space>; 2], [Id<Channel>; 6]) {
        let (north, south) = (Id::<Space>::mint(), Id::<Space>::mint());
        let (n_pub, n_pub_m) = public(north);
        let (s_pub, s_pub_m) = public(south);
        let (n_priv, s_priv, direct) = (
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
        );
        let join = Id::<Channel>::mint();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(
            vec![
                n_pub_m,
                s_pub_m,
                MemberChannel {
                    channel_id: n_priv,
                    place: ChannelPlace::Private(north),
                },
                MemberChannel {
                    channel_id: s_priv,
                    place: ChannelPlace::Private(south),
                },
                MemberChannel {
                    channel_id: direct,
                    place: ChannelPlace::Direct,
                },
            ],
            &[north, south],
        );
        sidebar.set_catalogue(
            north,
            None,
            Vec::new(),
            vec![listed(n_pub, "north", None, 0)],
        );
        sidebar.set_catalogue(
            south,
            None,
            Vec::new(),
            vec![
                listed(s_pub, "south", None, 0),
                listed(join, "hiring", None, 1),
            ],
        );
        (
            sidebar,
            [north, south],
            [n_pub, s_pub, n_priv, s_priv, direct, join],
        )
    }

    /// The channels of `sections`, in the order they are drawn.
    fn channels_of(sections: &[SidebarSection]) -> Vec<Id<Channel>> {
        sections
            .iter()
            .flat_map(|s| s.entries.iter().map(|e| e.channel))
            .collect()
    }

    /// `MURMURE-163`, `MURMURE-711`: the sidebar shows the categories, the private channels and
    /// the channels to join of the space shown alone, and every direct message. A space the user
    /// chose shows while it is theirs, else the first.
    #[test]
    fn the_sidebar_shows_one_space_and_every_direct_message() {
        let (sidebar, [north, south], [n_pub, s_pub, n_priv, s_priv, direct, join]) = two_spaces();
        assert_eq!(sidebar.shown_space(None), Some(north));
        assert_eq!(sidebar.shown_space(Some(south)), Some(south));
        assert_eq!(sidebar.shown_space(Some(Id::<Space>::mint())), Some(north));
        assert_eq!(
            channels_of(&sidebar.sections_in(Some(north))),
            vec![n_pub, n_priv, direct]
        );
        assert_eq!(
            channels_of(&sidebar.sections_in(Some(south))),
            vec![s_pub, s_priv, direct, join]
        );
    }

    /// `MURMURE-720`: a press on the head of a group folds it, and a second press unfolds it.
    /// Each group folds apart.
    #[test]
    fn a_press_on_the_head_folds_the_group_and_a_second_unfolds_it() {
        let mut folds = Folds::default();
        assert_eq!(folds.of(SidebarGroup::Direct), Folding::Open);
        folds.toggle(SidebarGroup::Direct);
        assert_eq!(folds.of(SidebarGroup::Direct), Folding::Folded);
        assert_eq!(folds.of(SidebarGroup::Private), Folding::Open);
        folds.toggle(SidebarGroup::Direct);
        assert_eq!(folds.of(SidebarGroup::Direct), Folding::Open);
        assert_eq!(Folding::Folded.expanded(), "false");
        assert_eq!(Folding::Open.expanded(), "true");
    }

    /// `MURMURE-721`, `MURMURE-722`: a folded group keeps the channel shown, a channel with a
    /// live call and a channel that mentions the user. Its head counts the channels it hides,
    /// and marks one with activity.
    #[test]
    fn a_folded_group_keeps_what_the_user_must_not_lose_and_counts_the_rest() {
        let space = Id::<Space>::mint();
        let ids: Vec<(Id<Channel>, MemberChannel)> = (0..5).map(|_| public(space)).collect();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(ids.iter().map(|(_, m)| *m).collect(), &[space]);
        let channels: Vec<Id<Channel>> = ids.iter().map(|(c, _)| *c).collect();
        sidebar.set_catalogue(
            space,
            None,
            Vec::new(),
            channels
                .iter()
                .enumerate()
                .map(|(i, c)| listed(*c, &format!("c{i}"), None, u32::try_from(i).unwrap()))
                .collect(),
        );
        let activity = |channel: Id<Channel>, unread: u32, mentions: u32| ChannelActivity {
            channel_id: channel,
            active: ambre_bus::Activity::Active,
            unread: Some(ambre_core::Count::<Unread>::new(unread)),
            mentions: Some(ambre_core::Count::<Mentions>::new(mentions)),
        };
        sidebar.set_activity(&[activity(channels[2], 3, 1), activity(channels[4], 5, 0)]);
        let mut calls = LiveCalls::default();
        calls.answer(
            channels[1],
            &ambre_bus::CallMembers::Live(vec![Id::<User>::mint()]),
        );
        let mut folds = Folds::default();
        let group = SidebarGroup::Category {
            space,
            category: None,
        };
        folds.toggle(group);
        let kept = Kept {
            shown: Some(channels[0]),
            calls: &calls,
        };
        let drawn = fold(sidebar.sections_in(Some(space)), &folds, kept);
        let section = &drawn[0];
        assert_eq!(section.folding, Folding::Folded);
        let shown: Vec<Id<Channel>> = section.section.entries.iter().map(|e| e.channel).collect();
        assert_eq!(
            shown,
            channels[..3],
            "the shown channel, the call and the mention"
        );
        assert_eq!(section.hidden, ambre_core::Count::<Hidden>::new(2));
        assert_eq!(
            section.mark,
            HiddenMark::Active,
            "a hidden channel has unread messages"
        );
        folds.toggle(group);
        let open = fold(sidebar.sections_in(Some(space)), &folds, kept);
        assert_eq!(open[0].section.entries.len(), 5);
        assert_eq!(open[0].hidden, ambre_core::Count::<Hidden>::ZERO);
    }

    /// `MURMURE-689`: a live call counts its participants, and a call with nobody seated, or
    /// one the relay holds no state of, leaves the sidebar.
    #[test]
    fn a_live_call_shows_its_count_and_leaves_once_gone() {
        let channel = Id::<Channel>::mint();
        let mut calls = LiveCalls::default();
        let seated = vec![Id::<User>::mint(), Id::<User>::mint()];
        calls.answer(channel, &ambre_bus::CallMembers::Live(seated));
        assert_eq!(
            calls.of(channel).map(LiveCall::count),
            Some(ambre_core::Count::<Seated>::new(2))
        );
        calls.answer(channel, &ambre_bus::CallMembers::Live(Vec::new()));
        assert_eq!(calls.of(channel), None);
        calls.answer(
            channel,
            &ambre_bus::CallMembers::Live(vec![Id::<User>::mint()]),
        );
        calls.answer(channel, &ambre_bus::CallMembers::Gone);
        assert_eq!(calls.of(channel), None);
    }

    /// `MURMURE-717`: the tab of direct messages shows the direct messages alone, and the other
    /// tabs every group. Each tab names itself.
    #[test]
    fn a_tab_shows_its_view_in_place_of_the_list() {
        assert!(PhoneTab::Direct.keeps(SidebarGroup::Direct));
        assert!(!PhoneTab::Direct.keeps(SidebarGroup::Private));
        assert!(PhoneTab::Channels.keeps(SidebarGroup::ToJoin));
        let labels: Vec<&str> = PhoneTab::ALL.iter().map(|t| t.label()).collect();
        assert_eq!(labels, ["Channels", "Direct messages", "Search"]);
    }
}
