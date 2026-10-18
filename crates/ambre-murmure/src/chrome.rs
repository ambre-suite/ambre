// ambre-murmure/src/chrome.rs
// ============================================================================
// Murmure-SPECIFIC chrome (ambre-spec-architecture.md: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit. It maps the pane onto the regions of the shell. It draws the
// channel header the command bar holds, and the channel list of the sidebar
// (the Murmure layout spec).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
// ============================================================================

use std::collections::{HashMap, HashSet};

use crate::commands::{MARK_ALL_READ, NEXT_ACTIVITY, PALETTE, PREVIOUS_ACTIVITY};
use crate::{Activity, MurmurePane, Sidebar, SidebarEntry, SidebarGroup, SidebarSection};
use ambre_bus::AppCommand;
use ambre_bus::{
    DirectMessageEvent, LogicCommand, MemberView, MurmureEvent, Presence, RequestId, RestCommand,
    RestEvent, SpaceCommand, SpaceEvent, Timestamp, UiEvent,
};
use ambre_chrome::{CommandHandler, KeyRoute, PaletteEntry, ShellAction, ShellRegion};
use ambre_commands::CommandId;
use ambre_core::{ChannelId, SpaceId, UserId};
use leptos::prelude::*;

/// The region of the shell a pane is in. The channel list is the sidebar,
/// a channel the content, and a thread or the member list the panel.
pub fn focus_of(pane: MurmurePane) -> ShellRegion {
    match pane {
        MurmurePane::ChannelList => ShellRegion::Sidebar,
        MurmurePane::Channel { .. } => ShellRegion::Content,
        MurmurePane::Thread { .. } | MurmurePane::Members { .. } => ShellRegion::Panel,
    }
}

/// Whether the panel is open: only while the user is in a thread or in the
/// member list.
pub fn panel_open(pane: MurmurePane) -> bool {
    matches!(
        pane,
        MurmurePane::Thread { .. } | MurmurePane::Members { .. }
    )
}

/// The accessible name of the panel: what it holds.
pub fn panel_label(pane: MurmurePane) -> &'static str {
    match pane {
        MurmurePane::Members { .. } => "Members",
        _ => "Thread",
    }
}

/// The header of the current channel, in the command bar of the shell
/// (`MURMURE-428`). Its back control goes to the pane before, and touches
/// nothing but the pane (`MURMURE-168`, `MURMURE-169`). A public channel of
/// a space shows a control that opens the member list of that space beside it.
#[component]
pub fn ChannelHeader(
    /// The pane the user is in.
    pane: RwSignal<MurmurePane>,
    /// The name of the current channel, once its engine has read it.
    #[prop(into)]
    name: Signal<Option<String>>,
    /// Whether the current channel shows the members of its space.
    #[prop(into)]
    has_members: Signal<bool>,
) -> impl IntoView {
    let members_open = move || matches!(pane.get(), MurmurePane::Members { .. });
    let toggle_members = move |_| {
        pane.update(|p| {
            *p = match *p {
                MurmurePane::Members { channel } => MurmurePane::channel(channel),
                other => match other.shown_channel() {
                    Some(channel) => MurmurePane::Members { channel },
                    None => other,
                },
            }
        })
    };
    view! {
        <div class="murmure-channel-header">
            <Show when=move || pane.get() != MurmurePane::ChannelList>
                <button
                    class="icon murmure-back"
                    aria-label="Back"
                    on:click=move |_| pane.update(|p| *p = p.back())
                >
                    "\u{2039}"
                </button>
            </Show>
            <span class="murmure-channel-name">{move || name.get().unwrap_or_default()}</span>
            <Show when=move || has_members.get()>
                <button
                    class="murmure-members-toggle"
                    aria-pressed=move || ambre_chrome::aria_bool(members_open())
                    on:click=toggle_members
                >
                    "Members"
                </button>
            </Show>
        </div>
    }
}

/// The member list of the space the current channel lies in (the Murmure
/// members spec). Each row shows the avatar, the display name and the
/// presence mark of one member (`MURMURE-441`, `MURMURE-442`). The filter
/// runs on what the page holds, and sends nothing (`MURMURE-454`). Choosing
/// another member goes through the relay, as in the palette (`MURMURE-455`),
/// and `send` carries what it answers.
#[component]
pub fn MemberList(relay: MurmureRelay, send: Callback<LogicCommand>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let rows = relay.shown_members();
    let found = move || {
        rows.with(|members| {
            query.with(|q| {
                crate::find_members(members, q)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
    };
    view! {
        <div class="murmure-members">
            <input
                class="murmure-members-filter"
                type="search"
                aria-label="Find a member"
                placeholder="Find a member"
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
            />
            <ul aria-label="Members">
                {move || {
                    let me = relay.me.get();
                    found()
                        .into_iter()
                        .map(|member| {
                            let user = member.user_id;
                            let choose = (Some(user) != me)
                                .then(|| {
                                    Callback::new(move |()| {
                                        if let Some(command) = relay.choose(*user.as_bytes()) {
                                            send.run(command);
                                        }
                                    })
                                });
                            member_view(member, choose)
                        })
                        .collect_view()
                }}
            </ul>
        </div>
    }
}

/// One member: the avatar, the name, then the presence mark. The two marks
/// differ in shape as well as in colour, and each carries its word for a
/// screen reader (`MURMURE-443`). Another member's row is a button that
/// writes to them; the user's own row is not.
fn member_view(member: MemberView, choose: Option<Callback<()>>) -> impl IntoView {
    let (state, word) = match member.presence {
        Presence::Online => ("online", "Online"),
        Presence::Offline => ("offline", "Offline"),
    };
    let color = ambre_draw::peer_css(&member.user_id);
    let name = member.display_name;
    let who = view! {
        <ambre_chrome::Avatar name=name.clone() color=color />
        <span class="murmure-member-name">{name.clone()}</span>
        <span class="murmure-presence" data-presence=state role="img" aria-label=word></span>
    };
    view! {
        <li class="murmure-member">
            {match choose {
                Some(choose) => {
                    view! {
                        <button
                            r#type="button"
                            class="murmure-member-open"
                            title=format!("Message {name}")
                            on:click=move |_| choose.run(())
                        >
                            {who}
                        </button>
                    }
                        .into_any()
                }
                None => who.into_any(),
            }}
        </li>
    }
}

/// The view state of a Murmure page, fed by the events of the Logic thread
/// (`ARCH-018`). The launcher hands it every event, and sends the commands
/// it answers. It names no variant itself.
#[derive(Clone, Copy)]
pub struct MurmureRelay {
    /// What the sidebar is drawn from.
    pub sidebar: RwSignal<Sidebar>,
    /// The pane the user is in.
    pub pane: RwSignal<MurmurePane>,
    /// The channels this page asked the Logic thread to open.
    opened: RwSignal<HashSet<ChannelId>>,
    /// The member list of each open space, as its last event drew it.
    members: RwSignal<HashMap<SpaceId, Vec<MemberView>>>,
    /// This user, once the session names them. The palette never offers a
    /// direct message with oneself.
    me: RwSignal<Option<UserId>>,
}

/// A system notification the view asks the page to raise (the Murmure
/// notifications spec). It names the channel and why, never what was
/// written: the summary it comes from holds no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The channel a click opens.
    pub channel_id: ChannelId,
    /// What the notification says.
    pub title: String,
}

impl Default for MurmureRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl MurmureRelay {
    /// An empty sidebar on the channel list.
    pub fn new() -> Self {
        Self {
            sidebar: RwSignal::new(Sidebar::default()),
            pane: RwSignal::new(MurmurePane::default()),
            opened: RwSignal::new(HashSet::new()),
            members: RwSignal::new(HashMap::new()),
            me: RwSignal::new(None),
        }
    }

    /// The notification an event calls for, if any (`MURMURE-143`). A
    /// message that names the user notifies, and so does any message of a
    /// direct message. The channel the user is reading notifies nothing
    /// (`MURMURE-146`). A page that is not `visible` shows none to read.
    pub fn notice(&self, ev: &UiEvent, visible: bool) -> Option<Notice> {
        let UiEvent::NewMessage {
            channel_id,
            mention,
        } = *ev
        else {
            return None;
        };
        let reading = visible && self.pane.get_untracked().shown_channel() == Some(channel_id);
        if reading {
            return None;
        }
        self.sidebar.with_untracked(|s| {
            let direct = s.is_direct(channel_id);
            let name = s.name_of(channel_id);
            let title = match (direct, mention, name) {
                (true, _, Some(name)) => format!("New message from {name}"),
                (true, _, None) => "New direct message".to_owned(),
                (false, true, Some(name)) => format!("You were mentioned in {name}"),
                (false, true, None) => "You were mentioned".to_owned(),
                (false, false, _) => return None,
            };
            Some(Notice { channel_id, title })
        })
    }

    /// Whether the channel the pane shows is a public channel of a space,
    /// whose members the member list shows.
    pub fn has_members(&self) -> Signal<bool> {
        let (sidebar, pane) = (self.sidebar, self.pane);
        Signal::derive(move || {
            pane.get()
                .shown_channel()
                .is_some_and(|channel| sidebar.with(|s| s.public_space_of(channel).is_some()))
        })
    }

    /// The member list of the space of the shown public channel, empty until
    /// its first event.
    pub fn shown_members(&self) -> Signal<Vec<MemberView>> {
        let (sidebar, pane, members) = (self.sidebar, self.pane, self.members);
        Signal::derive(move || {
            let space = pane
                .get()
                .shown_channel()
                .and_then(|channel| sidebar.with(|s| s.public_space_of(channel)));
            space
                .and_then(|space| members.with(|m| m.get(&space).cloned()))
                .unwrap_or_default()
        })
    }

    /// What the view sends once per refresh period: this user's presence on
    /// every space of the sidebar. Each one also turns offline the members
    /// whose presence expired (`MURMURE-445`). `now` is the view's clock.
    pub fn refresh_presence(&self, now: Timestamp) -> Vec<LogicCommand> {
        self.sidebar.with_untracked(|s| {
            s.spaces()
                .iter()
                .map(|&space_id| LogicCommand::Space {
                    space_id,
                    command: SpaceCommand::RefreshPresence { now },
                })
                .collect()
        })
    }

    /// What a Murmure page asks once its session is ready: the channels of
    /// its user and their activity. Neither opens a channel.
    pub fn boot() -> [LogicCommand; 2] {
        [
            LogicCommand::ListMemberChannels,
            LogicCommand::FetchActivity,
        ]
    }

    /// The name of the channel the pane shows, once known.
    pub fn channel_name(&self) -> Signal<Option<String>> {
        let (sidebar, pane) = (self.sidebar, self.pane);
        Signal::derive(move || {
            let channel = pane.get().shown_channel()?;
            sidebar.with(|s| s.name_of(channel))
        })
    }

    /// Take one event, and return the commands it calls for: a space to
    /// open for its catalogue, the channels to list again after a join.
    pub fn absorb(&self, ev: &UiEvent) -> Vec<LogicCommand> {
        match ev {
            UiEvent::Rest {
                evt: RestEvent::SessionReady { user_id, .. },
                ..
            } => {
                self.me.set(Some(*user_id));
                Vec::new()
            }
            // The direct message a member was chosen for, created or found.
            // The Logic thread opened it, so it shows and opens nothing more.
            // The listing puts it in the sidebar.
            UiEvent::DirectMessage {
                event: DirectMessageEvent::Opened { channel_id },
                ..
            } => {
                self.opened.update(|o| {
                    o.insert(*channel_id);
                });
                self.pane.set(MurmurePane::channel(*channel_id));
                vec![LogicCommand::ListMemberChannels]
            }
            UiEvent::MemberChannels {
                channels,
                spaces,
                names,
                peers,
            } => self
                .sidebar
                .try_update(|s| {
                    s.set_hints(names);
                    s.set_peers(peers);
                    s.set_member_channels(channels.clone(), spaces)
                })
                .unwrap_or_default()
                .into_iter()
                .map(|space_id| LogicCommand::OpenSpace { space_id })
                .collect(),
            UiEvent::ChannelActivity { channels } => {
                self.sidebar.update(|s| s.set_activity(channels));
                Vec::new()
            }
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Named { name },
            } => {
                self.sidebar
                    .update(|s| s.set_name(*channel_id, name.clone()));
                Vec::new()
            }
            // A channel the user lost while it was open (`MURMURE-489`).
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Left,
            } => {
                self.sidebar.update(|s| s.forget(*channel_id));
                self.drop_channels(&[*channel_id]);
                Vec::new()
            }
            UiEvent::Space { space_id, event } => self.absorb_space(*space_id, event),
            _ => Vec::new(),
        }
    }

    fn absorb_space(&self, space: ambre_core::SpaceId, event: &SpaceEvent) -> Vec<LogicCommand> {
        match event {
            SpaceEvent::Catalogue {
                name,
                categories,
                channels,
            } => {
                self.sidebar.update(|s| {
                    s.set_catalogue(space, name.clone(), categories.clone(), channels.clone());
                });
                Vec::new()
            }
            SpaceEvent::Members { members } => {
                self.members.update(|m| {
                    m.insert(space, members.clone());
                });
                Vec::new()
            }
            // The join this page asked for: the channel shows at once, and
            // the listing moves it among the joined ones.
            SpaceEvent::Joined { channel_id } => {
                let mut commands = vec![LogicCommand::ListMemberChannels];
                commands.extend(self.show(*channel_id));
                commands
            }
            SpaceEvent::ChannelLeft { channel_id } | SpaceEvent::ChannelDeleted { channel_id } => {
                self.sidebar.update(|s| s.forget(*channel_id));
                self.drop_channels(&[*channel_id]);
                Vec::new()
            }
            // The space leaves with every channel of it (`MURMURE-488`).
            SpaceEvent::Left => {
                let dropped = self
                    .sidebar
                    .try_update(|s| s.forget_space(space))
                    .unwrap_or_default();
                self.members.update(|m| {
                    m.remove(&space);
                });
                self.drop_channels(&dropped);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Forget the channels this page opened among `channels`, and leave the
    /// pane that shows one of them.
    fn drop_channels(&self, channels: &[ChannelId]) {
        self.opened.update(|o| o.retain(|c| !channels.contains(c)));
        let shown = self.pane.get_untracked().shown_channel();
        if shown.is_some_and(|c| channels.contains(&c)) {
            self.pane.set(MurmurePane::ChannelList);
        }
    }

    /// Select a channel of the sidebar. A joined one shows in the content,
    /// opened the first time only. One to join is joined first. Nothing is
    /// sent for the channel left behind (`MURMURE-167`).
    pub fn select(&self, channel: ChannelId) -> Option<LogicCommand> {
        let to_join = self
            .sidebar
            .with_untracked(|s| (!s.is_member(channel)).then(|| s.listed_in(channel)));
        match to_join {
            Some(space) => space.map(|space_id| LogicCommand::Space {
                space_id,
                command: SpaceCommand::JoinChannel {
                    channel_id: channel,
                },
            }),
            None => self.show(channel),
        }
    }

    /// What the palette lists before the commands: the channels in sidebar
    /// order, each named as the sidebar names it (`MURMURE-172`). Then come
    /// the other members of the space of the shown channel (`MURMURE-452`).
    pub fn palette_entries(&self) -> Signal<Vec<PaletteEntry>> {
        let (sidebar, members, me) = (self.sidebar, self.shown_members(), self.me);
        Signal::derive(move || {
            let mut entries: Vec<PaletteEntry> = sidebar.with(|s| {
                s.sections()
                    .into_iter()
                    .flat_map(|section| {
                        let words = GroupWords::of(section.group);
                        section.entries.into_iter().map(move |entry| PaletteEntry {
                            key: *entry.channel.as_bytes(),
                            label: entry.name.unwrap_or_else(|| words.unnamed.to_owned()),
                            hint: Some(words.hint),
                        })
                    })
                    .collect()
            });
            let me = me.get();
            members.with(|members| {
                entries.extend(
                    members
                        .iter()
                        .filter(|member| Some(member.user_id) != me)
                        .map(|member| PaletteEntry {
                            key: *member.user_id.as_bytes(),
                            label: member.display_name.clone(),
                            hint: Some("Member"),
                        }),
                );
            });
            entries
        })
    }

    /// Choose the palette entry of `key`. A channel opens as a click on the
    /// sidebar opens it (`MURMURE-173`). A member's key is fetched for the
    /// check of the pin, which opens the direct message of the two
    /// (`MURMURE-455`).
    pub fn choose(&self, key: [u8; 16]) -> Option<LogicCommand> {
        let user_id = UserId::from_bytes(key);
        if self.member_name(user_id).is_some() && self.me.get_untracked() != Some(user_id) {
            return Some(LogicCommand::Rest {
                id: RequestId::next(),
                cmd: RestCommand::FetchMemberKey { user_id },
            });
        }
        self.select(ChannelId::from_bytes(key))
    }

    /// The display name of `user` in an open space, if they are a member.
    pub fn member_name(&self, user: UserId) -> Option<String> {
        self.members.with_untracked(|spaces| {
            spaces
                .values()
                .flatten()
                .find(|member| member.user_id == user)
                .map(|member| member.display_name.clone())
        })
    }

    /// Open the next channel with activity after the shown one, or the
    /// previous one (`MURMURE-176`). It sends no read cursor: the channel
    /// opens as a click opens it (`MURMURE-177`).
    pub fn jump(&self, forward: bool) -> Option<LogicCommand> {
        let from = self.pane.get_untracked().shown_channel();
        let to = self
            .sidebar
            .with_untracked(|s| s.next_with_activity(from, forward))?;
        self.select(to)
    }

    fn show(&self, channel: ChannelId) -> Option<LogicCommand> {
        self.pane.set(MurmurePane::channel(channel));
        let fresh = self
            .opened
            .try_update(|o| o.insert(channel))
            .unwrap_or(false);
        fresh.then_some(LogicCommand::OpenChannel {
            channel_id: channel,
        })
    }
}

/// Murmure's half of the command path (`ARCH-018`). Its commands address the
/// user's channels, not the routed document, so it sends them through `send`.
/// It asks the shell for the palette alone.
#[derive(Clone, Copy)]
pub struct MurmureHandler {
    pub relay: MurmureRelay,
    pub send: Callback<LogicCommand>,
}

impl MurmureHandler {
    /// Open the channel of the palette entry `key`, the palette closing
    /// behind it (`MURMURE-173`).
    pub fn choose(&self, key: [u8; 16]) {
        if let Some(command) = self.relay.choose(key) {
            self.send.run(command);
        }
    }

    /// Run the command `id`. `false` for an id Murmure did not declare.
    fn run(&self, id: CommandId) -> bool {
        let command = match id {
            NEXT_ACTIVITY => self.relay.jump(true),
            PREVIOUS_ACTIVITY => self.relay.jump(false),
            // The command of the unread feature, which the relay answers for
            // every channel of the user (`MURMURE-180`).
            MARK_ALL_READ => Some(LogicCommand::MarkAllRead),
            _ => return false,
        };
        if let Some(command) = command {
            self.send.run(command);
        }
        true
    }
}

impl CommandHandler for MurmureHandler {
    type Command = AppCommand;

    fn on_command(&self, id: CommandId) -> Vec<ShellAction<AppCommand>> {
        if id == PALETTE {
            return vec![ShellAction::OpenPalette];
        }
        self.run(id);
        Vec::new()
    }

    fn on_key(&self, id: CommandId, in_text_field: bool) -> KeyRoute<AppCommand> {
        if id == PALETTE {
            return KeyRoute::Run(vec![ShellAction::OpenPalette]);
        }
        // A text field keeps Alt+Shift with an arrow, which extends its
        // selection on a Mac.
        if in_text_field {
            return KeyRoute::Yield;
        }
        self.run(id);
        KeyRoute::Run(Vec::new())
    }

    fn sealed_save(&self, _passphrase: String) -> Option<AppCommand> {
        // Murmure declares no sealed save.
        None
    }
}

/// The channel list of the sidebar, one section per group (`MURMURE-163`,
/// `MURMURE-164`). An entry shows the mark of its channel, or its counts
/// once known (`MURMURE-165`). Selecting one goes through the relay, and
/// `send` carries what the relay answers.
#[component]
pub fn ChannelList(relay: MurmureRelay, send: Callback<LogicCommand>) -> impl IntoView {
    let sections = Memo::new(move |_| relay.sidebar.with(Sidebar::sections));
    view! {
        <nav class="murmure-channel-list" aria-label="Channels">
            {move || {
                sections
                    .get()
                    .into_iter()
                    .map(|section| section_view(relay, send, section))
                    .collect_view()
            }}
        </nav>
    }
}

/// What the chrome calls a group of the sidebar, and an entry with no name.
#[derive(Clone, Copy)]
struct GroupWords {
    /// The word of the group in the markup.
    word: &'static str,
    /// The title of a section with none of its own.
    title: &'static str,
    /// The hint of an entry in the palette.
    hint: &'static str,
    /// The label of an entry nothing named yet.
    unnamed: &'static str,
}

impl GroupWords {
    fn of(group: SidebarGroup) -> Self {
        let (word, title, hint) = match group {
            SidebarGroup::Category { .. } => ("category", "Channels", "Channel"),
            SidebarGroup::Private => ("private", "Private channels", "Private channel"),
            SidebarGroup::Direct => ("direct", "Direct messages", "Direct message"),
            SidebarGroup::ToJoin => ("to-join", "Channels to join", "To join"),
        };
        let unnamed = match group {
            SidebarGroup::Direct => "Direct message",
            _ => "Private channel",
        };
        Self {
            word,
            title,
            hint,
            unnamed,
        }
    }
}

fn section_view(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    section: SidebarSection,
) -> impl IntoView {
    let GroupWords {
        word,
        title: fallback,
        unnamed,
        ..
    } = GroupWords::of(section.group);
    let title = section.title.unwrap_or_else(|| fallback.to_owned());
    view! {
        <section class="murmure-section" data-group=word>
            <h3 class="murmure-section-title">{title}</h3>
            <ul>
                {section
                    .entries
                    .into_iter()
                    .map(|entry| entry_view(relay, send, entry, unnamed))
                    .collect_view()}
            </ul>
        </section>
    }
}

fn entry_view(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    entry: SidebarEntry,
    unnamed: &'static str,
) -> impl IntoView {
    let channel = entry.channel;
    let current = move || (relay.pane.get().shown_channel() == Some(channel)).then_some("page");
    let badge = match entry.activity {
        Activity::None => None,
        Activity::Some => {
            Some(view! { <span class="murmure-mark" aria-label="New messages"></span> }.into_any())
        }
        Activity::Count { messages, mentions } => Some(
            view! {
                {(mentions.0 > 0).then(|| view! {
                    <span class="murmure-mentions" aria-label="Mentions">{mentions.0}</span>
                })}
                {(messages.0 > 0).then(|| view! {
                    <span class="murmure-count" aria-label="Unread messages">{messages.0}</span>
                })}
            }
            .into_any(),
        ),
    };
    view! {
        <li>
            <button
                class="murmure-entry"
                aria-current=current
                on:click=move |_| {
                    if let Some(command) = relay.select(channel) {
                        send.run(command);
                    }
                }
            >
                <span class="murmure-entry-name">
                    {entry.name.unwrap_or_else(|| unnamed.to_owned())}
                </span>
                {badge}
            </button>
        </li>
    }
}
