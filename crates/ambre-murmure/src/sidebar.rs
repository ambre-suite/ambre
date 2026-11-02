//! The channel list of the sidebar (the Murmure layout spec, section 3).
//!
//! A value the view rebuilds from what the Logic thread hands it. That is the
//! channels of the user, the catalogue of each open space, the activity marks
//! and the names of open channels. It sends nothing, so drawing the
//! sidebar opens no channel and moves no read cursor.

use std::collections::HashMap;

use ambre_bus::{
    CatalogPosition, CatalogueEntry, CategoryView, ChannelActivity, ChannelName, ChannelPlace,
    DirectPeers, MemberChannel, Mentions, Unread,
};
use ambre_core::StepDirection;
use ambre_core::id::{Category, Channel, Id, Space};

/// The section of the sidebar an entry sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        messages: ambre_core::Tally<Unread>,
        mentions: ambre_core::Tally<Mentions>,
    },
}

impl Activity {
    /// The activity a channel answer states.
    pub fn of(answer: &ChannelActivity) -> Self {
        match (answer.unread, answer.mentions) {
            (None, None) if answer.active == ambre_bus::Activity::Active => Self::Some,
            (None, None) => Self::None,
            (messages, mentions) => {
                let messages = messages.unwrap_or(ambre_core::Tally::<Unread>::ZERO);
                let mentions = mentions.unwrap_or(ambre_core::Tally::<Mentions>::ZERO);
                if messages.is_zero() && mentions == ambre_core::Tally::<Mentions>::ZERO {
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
    pub name: Option<ambre_core::Name>,
    pub activity: Activity,
}

/// One section of the sidebar, in the order it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSection {
    pub group: SidebarGroup,
    /// The category name, or the space name for the channels outside every
    /// category. Absent for the other groups, which the view titles itself.
    pub title: Option<ambre_core::Name>,
    pub entries: Vec<SidebarEntry>,
}

/// The catalogue of one open space.
#[derive(Debug, Clone, Default)]
struct Catalogue {
    name: Option<ambre_core::Name>,
    categories: Vec<CategoryView>,
    channels: Vec<CatalogueEntry>,
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
    names: HashMap<Id<Channel>, Option<ambre_core::Name>>,
    /// The names the relay's listing carried sealed, for channels not open
    /// (`MURMURE-490`). What meta reads always wins over them.
    hints: HashMap<Id<Channel>, ambre_core::Name>,
    /// The other members of each direct message, by display name, which a
    /// direct message nobody named shows (`MURMURE-492`).
    peers: HashMap<Id<Channel>, Vec<ambre_core::DisplayName>>,
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
        name: Option<ambre_core::Name>,
        categories: Vec<CategoryView>,
        channels: Vec<CatalogueEntry>,
    ) {
        if !self.spaces.contains(&space) {
            self.spaces.push(space);
        }
        self.catalogues.insert(
            space,
            Catalogue {
                name,
                categories,
                channels,
            },
        );
    }

    /// Take an activity answer. A channel absent from it keeps its mark.
    pub fn set_activity(&mut self, answers: &[ChannelActivity]) {
        for answer in answers {
            self.activity
                .insert(answer.channel_id, Activity::of(answer));
        }
    }

    /// Take the name an open channel read.
    pub fn set_name(&mut self, channel: Id<Channel>, name: Option<ambre_core::Name>) {
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
    pub fn name_of(&self, channel: Id<Channel>) -> Option<ambre_core::Name> {
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
                    .map(ambre_core::DisplayName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                ambre_core::Name::cut_to_fit(&joined)
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
            categories.sort_by_key(|c| (c.position, *c.id.as_bytes()));
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
    At(CatalogPosition),
    Unlisted,
}

/// The order of a public channel: where it stands in the catalogue, ties
/// by the bytes of its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rank {
    placed: Placed,
    channel: Id<Channel>,
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.placed
            .cmp(&other.placed)
            .then_with(|| self.channel.as_bytes().cmp(other.channel.as_bytes()))
    }
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The rank of the public channel `channel`, listed by `entry` or not yet.
fn rank(entry: Option<&CatalogueEntry>, channel: Id<Channel>) -> Rank {
    let placed = entry.map_or(Placed::Unlisted, |e| Placed::At(e.position));
    Rank { placed, channel }
}

fn push(
    sections: &mut Vec<SidebarSection>,
    group: SidebarGroup,
    title: Option<ambre_core::Name>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug form of the sidebar counts the peers of a direct message,
    /// and names none of them.
    #[test]
    fn the_sidebar_debugs_its_peer_count_alone() {
        let mut sidebar = Sidebar::default();
        let names = vec![
            ambre_core::DisplayName::new("Secretary").expect("a name"),
            ambre_core::DisplayName::new("Treasurer").expect("a name"),
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
            name: ambre_core::Name::new(name).unwrap(),
            topic: None,
            category,
            position: CatalogPosition::new(position),
        }
    }

    fn names(section: &SidebarSection) -> Vec<&str> {
        section
            .entries
            .iter()
            .map(|e| e.name.as_ref().map(ambre_core::Name::as_str).unwrap_or("?"))
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
    /// messages follow (`MURMURE-163`).
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
            Some(ambre_core::Name::new("Acme").unwrap()),
            vec![CategoryView {
                id: team,
                name: ambre_core::Name::new("Team").unwrap(),
                position: ambre_bus::CatalogPosition::new(0),
            }],
            vec![
                listed(leads, "leads", Some(team), 2),
                listed(sales, "sales", Some(team), 1),
                listed(general, "general", None, 0),
            ],
        );
        sidebar.set_name(private, Some(ambre_core::Name::new("board").unwrap()));

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
            sections[0].title.as_ref().map(ambre_core::Name::as_str),
            Some("Acme")
        );
        assert_eq!(names(&sections[0]), ["general"]);
        assert_eq!(
            sections[1].title.as_ref().map(ambre_core::Name::as_str),
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
            Some(ambre_core::Name::new("Acme").unwrap()),
            Vec::new(),
            vec![
                listed(general, "general", None, 0),
                listed(random, "random", None, 1),
            ],
        );
        sidebar.set_name(private, Some(ambre_core::Name::new("board").unwrap()));

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
            name: ambre_core::Name::new(name).unwrap(),
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
                .map(ambre_core::Name::as_str),
            Some("board")
        );
        sidebar.set_hints(&[hint(board, "plans")]);
        assert_eq!(
            sidebar
                .name_of(board)
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some("plans")
        );
        assert_eq!(sidebar.name_of(other), None, "replaced whole");

        sidebar.set_name(board, Some(ambre_core::Name::new("roadmap").unwrap()));
        assert_eq!(
            sidebar
                .name_of(board)
                .as_ref()
                .map(ambre_core::Name::as_str),
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
                .map(|n| ambre_core::DisplayName::new(n).unwrap())
                .collect(),
        };
        sidebar.set_peers(&[peers(direct, &["Bob", "Carol"]), peers(lonely, &[])]);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some("Bob, Carol")
        );
        assert_eq!(sidebar.name_of(lonely), None, "no other member");

        sidebar.set_hints(&[ChannelName {
            channel_id: direct,
            name: ambre_core::Name::new("trip").unwrap(),
        }]);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some("trip")
        );
        sidebar.set_name(direct, None);
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some("Bob, Carol")
        );
        sidebar.set_name(direct, Some(ambre_core::Name::new("plans").unwrap()));
        assert_eq!(
            sidebar
                .name_of(direct)
                .as_ref()
                .map(ambre_core::Name::as_str),
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
                unread: unread.map(ambre_core::Tally::<Unread>::new),
                mentions: mentions.map(ambre_core::Tally::<Mentions>::new),
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
                messages: ambre_core::Tally::<Unread>::new(3),
                mentions: ambre_core::Tally::<Mentions>::new(1)
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
}
