//! The channel list of the sidebar (the Murmure layout spec, section 3).
//!
//! A value the view rebuilds from what the Logic thread hands it. That is the
//! channels of the user, the catalogue of each open space, the activity marks
//! and the names of open channels. It sends nothing, so drawing the
//! sidebar opens no channel and moves no read cursor.

use std::collections::HashMap;

use ambre_bus::{
    CatalogueEntry, CategoryView, ChannelActivity, ChannelPlace, MemberChannel, MentionCount,
    UnreadCount,
};
use ambre_core::{CategoryId, ChannelId, SpaceId};

/// The section of the sidebar an entry sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarGroup {
    /// The joined public channels of one category of a space. `None` holds
    /// the channels outside every category.
    Category {
        space: SpaceId,
        category: Option<CategoryId>,
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
        messages: UnreadCount,
        mentions: MentionCount,
    },
}

impl Activity {
    /// The activity a channel answer states.
    pub fn of(answer: &ChannelActivity) -> Self {
        match (answer.unread, answer.mentions) {
            (None, None) if answer.active => Self::Some,
            (None, None) => Self::None,
            (messages, mentions) => {
                let messages = messages.unwrap_or(UnreadCount::ZERO);
                let mentions = mentions.unwrap_or(MentionCount::ZERO);
                if messages.is_zero() && mentions == MentionCount::ZERO {
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
    pub channel: ChannelId,
    /// Absent until a catalogue or the open channel names it.
    pub name: Option<String>,
    pub activity: Activity,
}

/// One section of the sidebar, in the order it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSection {
    pub group: SidebarGroup,
    /// The category name, or the space name for the channels outside every
    /// category. Absent for the other groups, which the view titles itself.
    pub title: Option<String>,
    pub entries: Vec<SidebarEntry>,
}

/// The catalogue of one open space.
#[derive(Debug, Clone, Default)]
struct Catalogue {
    name: Option<String>,
    categories: Vec<CategoryView>,
    channels: Vec<CatalogueEntry>,
}

/// The state the sidebar is drawn from.
#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    members: Vec<MemberChannel>,
    /// The spaces in the order the user's channels first named them.
    spaces: Vec<SpaceId>,
    catalogues: HashMap<SpaceId, Catalogue>,
    activity: HashMap<ChannelId, Activity>,
    names: HashMap<ChannelId, String>,
}

impl Sidebar {
    /// Take the channels of the user, whole. Returns the spaces they name
    /// for the first time, which the view opens to read their catalogue.
    pub fn set_member_channels(&mut self, channels: Vec<MemberChannel>) -> Vec<SpaceId> {
        let mut fresh = Vec::new();
        for channel in &channels {
            if let ChannelPlace::Public(space) | ChannelPlace::Private(space) = channel.place
                && !self.spaces.contains(&space)
            {
                self.spaces.push(space);
                fresh.push(space);
            }
        }
        self.members = channels;
        fresh
    }

    /// Take the catalogue of an open space, whole.
    pub fn set_catalogue(
        &mut self,
        space: SpaceId,
        name: Option<String>,
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
    pub fn set_name(&mut self, channel: ChannelId, name: Option<String>) {
        match name {
            Some(name) => self.names.insert(channel, name),
            None => self.names.remove(&channel),
        };
    }

    /// Drop a channel the user left, or the relay deleted.
    pub fn forget(&mut self, channel: ChannelId) {
        self.members.retain(|m| m.channel_id != channel);
        self.activity.remove(&channel);
        self.names.remove(&channel);
    }

    /// Whether the user belongs to the channel.
    pub fn is_member(&self, channel: ChannelId) -> bool {
        self.members.iter().any(|m| m.channel_id == channel)
    }

    /// The name the sidebar shows for a channel, if it knows one.
    pub fn name_of(&self, channel: ChannelId) -> Option<String> {
        self.names.get(&channel).cloned().or_else(|| {
            self.catalogues
                .values()
                .flat_map(|c| &c.channels)
                .find(|e| e.channel_id == channel)
                .map(|e| e.name.clone())
        })
    }

    /// The open space whose catalogue lists the channel, to join it there.
    pub fn listed_in(&self, channel: ChannelId) -> Option<SpaceId> {
        self.catalogues
            .iter()
            .find(|(_, c)| c.channels.iter().any(|e| e.channel_id == channel))
            .map(|(space, _)| *space)
    }

    fn entry(&self, channel: ChannelId) -> SidebarEntry {
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
            let joined: Vec<ChannelId> = self
                .members
                .iter()
                .filter(|m| m.place == ChannelPlace::Public(space))
                .map(|m| m.channel_id)
                .collect();
            let listed = |id: ChannelId| {
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
                let mut channels: Vec<ChannelId> = joined
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
}

/// The order of a public channel: its rank in the catalogue, ties by
/// identifier. A channel the catalogue does not list yet comes last.
fn rank(entry: Option<&CatalogueEntry>, id: ChannelId) -> (u32, [u8; 16]) {
    (entry.map_or(u32::MAX, |e| e.position), *id.as_bytes())
}

fn push(
    sections: &mut Vec<SidebarSection>,
    group: SidebarGroup,
    title: Option<String>,
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

    fn public(space: SpaceId) -> (ChannelId, MemberChannel) {
        let channel_id = ChannelId::new();
        let place = ChannelPlace::Public(space);
        (channel_id, MemberChannel { channel_id, place })
    }

    fn listed(
        channel_id: ChannelId,
        name: &str,
        category: Option<CategoryId>,
        position: u32,
    ) -> CatalogueEntry {
        CatalogueEntry {
            channel_id,
            name: name.to_owned(),
            topic: None,
            category,
            position,
        }
    }

    fn names(section: &SidebarSection) -> Vec<&str> {
        section
            .entries
            .iter()
            .map(|e| e.name.as_deref().unwrap_or("?"))
            .collect()
    }

    /// The joined public channels of a space are grouped by category, in
    /// the rank of the catalogue. The private channels and the direct
    /// messages follow (`MURMURE-163`).
    #[test]
    fn the_sidebar_groups_public_channels_by_category_then_private_then_direct() {
        let space = SpaceId::new();
        let (general, general_m) = public(space);
        let (sales, sales_m) = public(space);
        let (leads, leads_m) = public(space);
        let private = ChannelId::new();
        let direct = ChannelId::new();
        let mut sidebar = Sidebar::default();
        let fresh = sidebar.set_member_channels(vec![
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
        ]);
        assert_eq!(fresh, vec![space], "the space is opened once");
        let team = CategoryId::new();
        sidebar.set_catalogue(
            space,
            Some("Acme".to_owned()),
            vec![CategoryView {
                id: team,
                name: "Team".to_owned(),
                position: 0,
            }],
            vec![
                listed(leads, "leads", Some(team), 2),
                listed(sales, "sales", Some(team), 1),
                listed(general, "general", None, 0),
            ],
        );
        sidebar.set_name(private, Some("board".to_owned()));

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
        assert_eq!(sections[0].title.as_deref(), Some("Acme"));
        assert_eq!(names(&sections[0]), ["general"]);
        assert_eq!(sections[1].title.as_deref(), Some("Team"));
        assert_eq!(names(&sections[1]), ["sales", "leads"]);
        assert_eq!(names(&sections[2]), ["board"]);
        assert_eq!(sections[3].entries[0].channel, direct);
        assert_eq!(
            sections[3].entries[0].name, None,
            "a closed direct message has no name yet"
        );
        assert!(
            sidebar
                .set_member_channels(sidebar.members.clone())
                .is_empty()
        );
    }

    /// A public channel the user has not joined sits apart, after every
    /// joined one, and leaves that section once joined (`MURMURE-164`).
    #[test]
    fn a_channel_to_join_sits_apart_from_the_joined_ones() {
        let space = SpaceId::new();
        let (general, general_m) = public(space);
        let random = ChannelId::new();
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![general_m]);
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
        assert_eq!(sidebar.listed_in(ChannelId::new()), None);

        sidebar.set_member_channels(vec![
            general_m,
            MemberChannel {
                channel_id: random,
                place: ChannelPlace::Public(space),
            },
        ]);
        let sections = sidebar.sections();
        assert_eq!(sections.len(), 1, "nothing is left to join");
        assert_eq!(names(&sections[0]), ["general", "random"]);
    }

    /// The relay's mark shows before any count, and a count replaces it once
    /// known (`MURMURE-119`, `MURMURE-165`). A channel absent from a later
    /// answer keeps its own.
    #[test]
    fn a_mark_shows_before_the_count_and_the_count_replaces_it() {
        let space = SpaceId::new();
        let (a, a_m) = public(space);
        let (b, b_m) = public(space);
        let mut sidebar = Sidebar::default();
        sidebar.set_member_channels(vec![a_m, b_m]);
        let answer =
            |channel_id, active, unread: Option<u32>, mentions: Option<u32>| ChannelActivity {
                channel_id,
                active,
                unread: unread.map(UnreadCount),
                mentions: mentions.map(MentionCount),
            };
        let of = |sidebar: &Sidebar, id| sidebar.entry(id).activity;
        assert_eq!(of(&sidebar, a), Activity::None);

        sidebar.set_activity(&[answer(a, true, None, None), answer(b, false, None, None)]);
        assert_eq!(of(&sidebar, a), Activity::Some);
        assert_eq!(of(&sidebar, b), Activity::None);

        sidebar.set_activity(&[answer(a, true, Some(3), Some(1))]);
        assert_eq!(
            of(&sidebar, a),
            Activity::Count {
                messages: UnreadCount(3),
                mentions: MentionCount(1)
            }
        );
        sidebar.set_activity(&[answer(b, true, None, None)]);
        assert!(
            matches!(of(&sidebar, a), Activity::Count { .. }),
            "a keeps its count"
        );
        sidebar.set_activity(&[answer(a, false, Some(0), Some(0))]);
        assert_eq!(
            of(&sidebar, a),
            Activity::None,
            "a read channel shows nothing"
        );

        sidebar.forget(b);
        assert!(!sidebar.is_member(b));
        assert_eq!(of(&sidebar, b), Activity::None);
    }
}
