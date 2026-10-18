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

use std::collections::HashSet;

use crate::{Activity, MurmurePane, Sidebar, SidebarEntry, SidebarGroup, SidebarSection};
use ambre_bus::{LogicCommand, MurmureEvent, SpaceCommand, SpaceEvent, UiEvent};
use ambre_chrome::ShellRegion;
use ambre_core::ChannelId;
use leptos::prelude::*;

/// The region of the shell a pane is in. The channel list is the sidebar,
/// a channel the content, and a thread the panel.
pub fn focus_of(pane: MurmurePane) -> ShellRegion {
    match pane {
        MurmurePane::ChannelList => ShellRegion::Sidebar,
        MurmurePane::Channel { .. } => ShellRegion::Content,
        MurmurePane::Thread { .. } => ShellRegion::Panel,
    }
}

/// Whether the thread panel is open: only while the user is in a thread.
pub fn thread_open(pane: MurmurePane) -> bool {
    matches!(pane, MurmurePane::Thread { .. })
}

/// The header of the current channel, in the command bar of the shell
/// (`MURMURE-428`). Its back control goes to the pane before, and touches
/// nothing but the pane (`MURMURE-168`, `MURMURE-169`).
#[component]
pub fn ChannelHeader(
    /// The pane the user is in.
    pane: RwSignal<MurmurePane>,
    /// The name of the current channel, once its engine has read it.
    #[prop(into)]
    name: Signal<Option<String>>,
) -> impl IntoView {
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
        </div>
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
        }
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

fn section_view(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    section: SidebarSection,
) -> impl IntoView {
    let (word, fallback) = match section.group {
        SidebarGroup::Category { .. } => ("category", "Channels"),
        SidebarGroup::Private => ("private", "Private channels"),
        SidebarGroup::Direct => ("direct", "Direct messages"),
        SidebarGroup::ToJoin => ("to-join", "Channels to join"),
    };
    let title = section.title.unwrap_or_else(|| fallback.to_owned());
    let unnamed = match section.group {
        SidebarGroup::Direct => "Direct message",
        _ => "Private channel",
    };
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
