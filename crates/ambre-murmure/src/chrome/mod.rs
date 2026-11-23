// ambre-murmure/src/chrome/mod.rs
// ============================================================================
// Murmure-SPECIFIC chrome (ambre-spec-architecture.md: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit. It maps the pane onto the regions of the shell. It draws the
// channel header the command bar holds, and the channel list of the sidebar
// (the Murmure layout spec).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
// ============================================================================

use ambre_commands::Locale;
use ambre_core::index::{Len, Pos};
use ambre_core::{Commands, StaticKey};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

use crate::SpaceRanks;
use crate::message::LogicCommand;
use crate::message::{
    ChannelMatches, DirectMessageEvent, MemberView, MessageAt, MurmureCommand, MurmureEvent,
    Presence, SpaceCommand, SpaceEvent, TextStyle,
};
use crate::ui::commands::{
    AskedAll, COMMANDS, KEYS, KeyPlace, MARK_ALL_READ, MessageAction, MessageTarget, NEXT_ACTIVITY,
    PALETTE, PREVIOUS_ACTIVITY, PendingDelete, PendingEdit, Picking, SEARCH, SHOW_SPACE,
    asked_all_title, key_list, message_action, message_actions,
};
use crate::ui::drafts::DraftShelf;
use crate::ui::emoji;
use crate::ui::markers::{Cover, MarkBar};
use crate::ui::mention_list::{MentionEntry, MentionList};
use crate::ui::uploads::PickedFile;
use crate::{
    Activity, DrawnSection, Folding, Folds, HiddenMark, Kept, LiveCalls, MessageRef, MurmurePane,
    PhoneTab, Region, Sidebar, SidebarEntry, SidebarGroup, SidebarSection, SpaceHead,
};
use ambre_bus::MentionState;
use ambre_bus::{Authored, ChannelPlace, Instant, RestCommand, RestEvent};
use ambre_chrome::{
    CommandHandler, FormatBar, FormatControl, KeyRoute, MenuEntry, PaletteEntry, PaletteKey,
    Pressed, ShellAction, ShellRegion, Toasts,
};
use ambre_core::id::{Channel, Id, Space, User};
use ambre_core::{Css, Name, Point, Px, StepDirection, Validated};
use leptos::prelude::*;

pub use crate::ui::notices::{
    CHANNEL_NOT_CHANGED, CHANNEL_NOT_OPENED, DIRECT_MESSAGE_NOT_OPENED, ROOT_LOST, SPACE_NOT_OPENED,
};
use ambre_core::Flag as _;

/// Surface a root the seek could not reach as a toast (the Murmure threads
/// spec). A channel or a space that could not open is told the same way
/// (`MURMURE-549`, `MURMURE-550`). So is a channel change or a direct message
/// that failed (`MURMURE-580`, `MURMURE-581`). The launcher names no Murmure
/// variant. A no-op for every other event.
pub fn notify_event<S>(toasts: &Toasts, ev: &ambre_bus::UiEvent<S, crate::message::Murmure>) {
    if let Some(text) = crate::ui::notices::warning_of(ev) {
        toasts.warning(text);
    }
}

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
pub fn panel_open(pane: MurmurePane) -> ambre_chrome::Openness {
    ambre_chrome::Openness::from_bool(matches!(
        pane,
        MurmurePane::Thread { .. } | MurmurePane::Members { .. }
    ))
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
    name: Signal<Option<Validated<Name>>>,
    /// Whether the current channel shows the members of its space.
    #[prop(into)]
    has_members: Signal<bool>,
    /// The controls the header holds beside the name: the call controls of the page.
    #[prop(optional)]
    children: Option<Children>,
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
            <span class="murmure-channel-name">{move || name.get().map(Validated::into_string).unwrap_or_default()}</span>
            <Show when=move || has_members.get()>
                <button
                    class="murmure-members-toggle"
                    aria-pressed=move || ambre_chrome::Pressed::from_bool(members_open()).aria()
                    on:click=toggle_members
                >
                    "Members"
                </button>
            </Show>
            {children.map(|children| children())}
        </div>
    }
}

/// The member list of the space the current channel lies in (the Murmure
/// members spec). Each row shows the avatar, the display name and the
/// presence mark of one member (`MURMURE-441`, `MURMURE-442`). A member who
/// watches the current channel takes the mark of it (`MURMURE-519`). The filter
/// runs on what the page holds, and sends nothing (`MURMURE-454`). Choosing
/// another member goes through the relay, as in the palette (`MURMURE-455`),
/// and `send` carries what it answers.
#[component]
pub fn MemberList(relay: MurmureRelay, send: Callback<LogicCommand>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let rows = relay.shown_members();
    let watchers = relay.shown_watchers();
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
                    let here = watchers.get();
                    found()
                        .into_iter()
                        .map(|member| {
                            let user = member.user_id;
                            let watching = if here.contains(&user) {
                                Watching::Here
                            } else {
                                Watching::Elsewhere
                            };
                            let choose = (Some(user) != me)
                                .then(|| {
                                    Callback::new(move |()| {
                                        if let Some(command) = relay.choose(PaletteKey::Member(user)) {
                                            send.run(command);
                                        }
                                    })
                                });
                            member_view(member, watching, choose.map(|choose| (choose, "Message")))
                        })
                        .collect_view()
                }}
            </ul>
        </div>
    }
}

/// The invite control of a private channel or a direct message (the Murmure
/// channels spec). It opens a list of the members the relay offers, behind
/// a filter that sends nothing. Choosing one fetches their key for the check
/// of the pin, and closes the list.
#[component]
pub fn InviteControl(relay: MurmureRelay, send: Callback<LogicCommand>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let query = RwSignal::new(String::new());
    let target = relay.invite_target();
    // Another channel closes the list.
    Effect::new(move |_| {
        target.track();
        open.set(ambre_chrome::Openness::Closed);
    });
    let rows = relay.invitees();
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
        <Show when=move || target.get().is_some()>
            <div class="murmure-invite">
                <button
                    class="murmure-invite-toggle"
                    aria-expanded=move || open.get().aria()
                    on:click=move |_| open.update(|o| *o = o.toggled())
                >
                    "Invite"
                </button>
                <Show when=move || open.get() == ambre_chrome::Openness::Open>
                    <div class="murmure-invite-picker">
                        <input
                            class="murmure-invite-filter"
                            type="search"
                            aria-label="Find a member to invite"
                            placeholder="Find a member"
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                        />
                        <ul aria-label="Invite a member">
                            {move || {
                                found()
                                    .into_iter()
                                    .map(|member| {
                                        let user = member.user_id;
                                        let choose = Callback::new(move |()| {
                                            if let Some(command) = relay.invite(user) {
                                                send.run(command);
                                            }
                                            open.set(ambre_chrome::Openness::Closed);
                                            query.set(String::new());
                                        });
                                        member_view(member, Watching::Elsewhere, Some((choose, "Invite")))
                                    })
                                    .collect_view()
                            }}
                        </ul>
                    </div>
                </Show>
            </div>
        </Show>
    }
}

/// Whether a member watches the channel the pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Watching {
    Elsewhere,
    Here,
}

/// One member: the avatar, the name, then the presence mark. The marks
/// differ in shape as well as in colour, and each carries its word for a
/// screen reader (`MURMURE-443`, `MURMURE-520`). A member [`Watching::Here`]
/// watches the current channel. A row the user may act on is a button, titled by the verb
/// of what it does; the user's own row is not.
fn member_view(
    member: MemberView,
    watching: Watching,
    choose: Option<(Callback<()>, &'static str)>,
) -> impl IntoView {
    let (state, word) = match member.presence {
        _ if watching == Watching::Here => ("here", "In this channel"),
        Presence::Online => ("online", "Online"),
        Presence::Offline => ("offline", "Offline"),
    };
    let color = ambre_draw::peer_rgb(&member.user_id);
    let display_name = member.display_name;
    let name = display_name.as_str().to_owned();
    let who = view! {
        <ambre_chrome::Avatar name=display_name color=color />
        <span class="murmure-member-name">{name.clone()}</span>
        <span class="murmure-presence" data-presence=state role="img" aria-label=word></span>
    };
    view! {
        <li class="murmure-member">
            {match choose {
                Some((choose, verb)) => {
                    view! {
                        <button
                            r#type="button"
                            class="murmure-member-open"
                            title=format!("{verb} {name}")
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

/// Whether the browser shows the page, so the user can read the channel
/// the pane holds (`MURMURE-146`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageVisibility {
    Hidden,
    Visible,
}

impl PageVisibility {
    /// [`PageVisibility::Visible`] when `visible` holds.
    pub fn visible_if(visible: bool) -> Self {
        if visible {
            PageVisibility::Visible
        } else {
            PageVisibility::Hidden
        }
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
    opened: RwSignal<HashSet<Id<Channel>>>,
    /// The member list of each open space, as its last event drew it.
    members: RwSignal<HashMap<Id<Space>, Vec<MemberView>>>,
    /// This user, once the session names them. The palette never offers a
    /// direct message with oneself.
    me: RwSignal<Option<Id<User>>>,
    /// The search box of the command bar, closed until the find chord.
    pub search: RwSignal<SearchBox>,
    /// The member whose key was fetched for an invite, and the channel it
    /// goes into. The answer becomes that invite, not a direct message.
    inviting: RwSignal<Option<(Id<User>, Id<Channel>)>>,
    /// The members of each open channel by name, as the Logic thread last
    /// listed them. The invite control offers none of them, and the `@`
    /// list offers them all (`MURMURE-551`).
    #[expect(
        clippy::type_complexity,
        reason = "each `Id<_>` names its kind, and no alias may hide it"
    )]
    channel_members: RwSignal<
        HashMap<Id<Channel>, HashMap<Id<User>, ambre_core::Validated<ambre_core::DisplayName>>>,
    >,
    /// The `@` list a region of the page shows, `None` while none is open
    /// (`MURMURE-555`).
    pub mentions: RwSignal<Option<MentionList>>,
    /// The entry a click picked in the `@` list, until its region takes it.
    picked: RwSignal<Option<(Region, Pos<MentionEntry>)>>,
    /// The format bar a region of the page shows, `None` while nothing is
    /// selected (`MURMURE-432`).
    pub mark_bar: RwSignal<Option<MarkBar>>,
    /// The style a press on the format bar asked for, until its region takes
    /// it.
    pressed_mark: RwSignal<Option<(Region, TextStyle)>>,
    /// The other members who watch each open channel, as the Logic thread
    /// last heard them (`MURMURE-126`). A channel nobody watches has no entry.
    watching: RwSignal<HashMap<Id<Channel>, HashSet<Id<User>>>>,
    /// The message the actions are open on, as the view last found it under
    /// a right-click or a long press. `None` once the menu closes.
    pub target: RwSignal<Option<MessageTarget>>,
    /// The deletion the dialog asks the member to confirm (`MURMURE-512`).
    /// `None` once confirmed or cancelled.
    pub deleting: RwSignal<Option<PendingDelete>>,
    /// The message the member asked to edit, until the channel's region
    /// loads it into the composer (`MURMURE-203`).
    pub editing: RwSignal<Option<PendingEdit>>,
    /// The list of keys shows (`MURMURE-540`).
    pub keys: RwSignal<ambre_chrome::Openness>,
    /// The `@channel` the dialog asks the author about (`MURMURE-557`).
    /// `None` once sent or cancelled.
    pub asking_all: RwSignal<Option<AskedAll>>,
    /// The send the author cancelled, until its region takes the draft back
    /// (`MURMURE-558`).
    returned: RwSignal<Option<AskedAll>>,
    /// The drafts the device keeps. A lost channel takes its own, and a
    /// sign-out takes them all (`MURMURE-215`, `MURMURE-216`).
    drafts: StoredValue<DraftShelf, LocalStorage>,
    /// What the emoji picker is open for, `None` while it is closed.
    pub picking: RwSignal<Option<Picking>>,
    /// The emoji the picker picked for a composer, until its region takes
    /// it (`MURMURE-139`).
    picked_emoji: RwSignal<Option<(Region, ambre_bus::Emoji)>>,
    /// The files dropped on the channel's canvas, until its region takes
    /// them to upload (`MURMURE-132`).
    dropped_files: RwSignal<Vec<PickedFile>>,
    /// The groups of the sidebar the user folded (`MURMURE-720`).
    pub folds: RwSignal<Folds>,
    /// The live calls the page knows, by channel, which the sidebar marks (`MURMURE-689`).
    pub calls: RwSignal<LiveCalls>,
    /// The space the user chose in the switcher, `None` for the first (`MURMURE-712`).
    pub space: RwSignal<Option<Id<Space>>>,
    /// Whether the space switcher lists the spaces (`MURMURE-714`).
    pub switcher: RwSignal<ambre_chrome::Openness>,
    /// The tab of the bar of the phone (`MURMURE-717`).
    pub tab: RwSignal<PhoneTab>,
}

/// The search box: whether it is open, the query typed, and the last answer
/// to that query (the Murmure search spec).
///
/// `Debug` prints the length of the query and counts the channels alone:
/// a member typed the one, and the other carries message texts.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SearchBox {
    /// Whether the box shows in the command bar.
    pub open: ambre_chrome::Openness,
    /// The words typed.
    pub query: String,
    /// What the Logic thread answered for `query`, one entry per open channel.
    pub channels: Vec<ChannelMatches>,
}

impl std::fmt::Debug for SearchBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchBox")
            .field("open", &self.open)
            .field("query", &Len::<ambre_core::Byte>::of(self.query.len()))
            .field("channels", &self.channels.len())
            .finish()
    }
}

/// The heading of the matches of a channel with no name read yet.
const UNNAMED_CHANNEL: &str = "Unnamed channel";

/// The matches of one channel, as the result list shows them.
///
/// `Debug` counts the matches alone: each one carries a message text.
#[derive(Clone, PartialEq, Eq)]
pub struct SearchGroup {
    /// The channel a click on a match opens.
    pub channel_id: Id<Channel>,
    /// The name the sidebar shows, `None` for a channel with no name read
    /// yet.
    pub name: Option<Validated<Name>>,
    /// Each matching message, where a click opens it, and its text, in
    /// channel order.
    pub matches: Vec<(MessageRef, String)>,
    /// Which part of its history was read, as "2 of 14 segments".
    pub coverage: String,
}

impl std::fmt::Debug for SearchGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchGroup")
            .field("channel_id", &self.channel_id)
            .field("name", &self.name)
            .field("matches", &self.matches.len())
            .field("coverage", &self.coverage)
            .finish()
    }
}

/// "1 segment", or "2 segments": the coverage line counts in segments.
fn segments(count: ambre_core::Count<crate::message::Segments>) -> String {
    match count.get() {
        1 => "1 segment".to_owned(),
        n => format!("{n} segments"),
    }
}

/// What the search read, in all: "Searched 2 of 14 segments in 3 channels".
/// A result list states its coverage, matches or not (`MURMURE-149`).
pub fn coverage(channels: &[ChannelMatches]) -> String {
    let covered: usize = channels.iter().map(|c| c.covered.len()).sum();
    let listed = channels.iter().fold(
        ambre_core::Count::<crate::message::Segments>::ZERO,
        |all, c| all.plus(c.listed),
    );
    let open = match channels.len() {
        1 => "1 open channel".to_owned(),
        n => format!("{n} open channels"),
    };
    format!("Searched {covered} of {} in {open}", segments(listed))
}

/// A system notification the view asks the page to raise (the Murmure
/// notifications spec). It names the channel and why, never what was
/// written: the summary it comes from holds no text.
///
/// `Debug` prints the length of the title alone: it names a channel or a
/// member.
#[derive(Clone, PartialEq, Eq)]
pub struct Notice {
    /// The channel a click opens.
    pub channel_id: Id<Channel>,
    /// The message a click rests the list on (`MURMURE-507`). A closed
    /// channel names none, and opens at its latest message.
    pub at: Option<MessageRef>,
    /// What the notification says.
    pub title: String,
}

impl std::fmt::Debug for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notice")
            .field("channel_id", &self.channel_id)
            .field("at", &self.at)
            .field("title", &Len::<ambre_core::Byte>::of(self.title.len()))
            .finish()
    }
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
            search: RwSignal::new(SearchBox::default()),
            inviting: RwSignal::new(None),
            channel_members: RwSignal::new(HashMap::new()),
            mentions: RwSignal::new(None),
            picked: RwSignal::new(None),
            mark_bar: RwSignal::new(None),
            pressed_mark: RwSignal::new(None),
            watching: RwSignal::new(HashMap::new()),
            target: RwSignal::new(None),
            deleting: RwSignal::new(None),
            editing: RwSignal::new(None),
            asking_all: RwSignal::new(None),
            returned: RwSignal::new(None),
            keys: RwSignal::new(ambre_chrome::Openness::Closed),
            drafts: StoredValue::new_local(DraftShelf::default()),
            picking: RwSignal::new(None),
            picked_emoji: RwSignal::new(None),
            dropped_files: RwSignal::new(Vec::new()),
            folds: RwSignal::new(Folds::default()),
            calls: RwSignal::new(LiveCalls::default()),
            space: RwSignal::new(None),
            switcher: RwSignal::new(ambre_chrome::Openness::Closed),
            tab: RwSignal::new(PhoneTab::Channels),
        }
    }

    /// The space the sidebar shows: the one the user chose, else the first (`MURMURE-711`).
    pub fn shown_space(&self) -> Signal<Option<Id<Space>>> {
        let (sidebar, space) = (self.sidebar, self.space);
        Signal::derive(move || sidebar.with(|s| s.shown_space(space.get())))
    }

    /// Shows the space of `rank`, counted from zero in the order of the switcher. Nothing opens,
    /// nothing is joined and no cursor moves (`MURMURE-712`, `MURMURE-713`).
    pub fn show_space(&self, rank: Pos<SpaceRanks>) {
        if let Some(space) = self
            .sidebar
            .with_untracked(|s| s.spaces().get(rank.to_usize()).copied())
        {
            self.space.set(Some(space));
        }
        self.switcher.set(ambre_chrome::Openness::Closed);
    }

    /// Folds the group `group`, or unfolds it. Nothing leaves the page (`MURMURE-720`,
    /// `MURMURE-724`).
    pub fn toggle_fold(&self, group: SidebarGroup) {
        self.folds.update(|f| f.toggle(group));
    }

    /// Shows the view of `tab` in place of the channel list of a phone. The search tab opens the
    /// search box (`MURMURE-717`).
    pub fn choose_tab(&self, tab: PhoneTab) {
        self.tab.set(tab);
        if tab == PhoneTab::Search {
            self.search
                .update(|s| s.open = ambre_chrome::Openness::Open);
        }
    }

    /// The page read `file`, dropped on the channel's canvas.
    pub fn drop_file(&self, file: PickedFile) {
        self.dropped_files.update(|files| files.push(file));
    }

    /// The files dropped since the last call, taken once.
    pub fn take_files(&self) -> Vec<PickedFile> {
        self.dropped_files
            .try_update_untracked(std::mem::take)
            .unwrap_or_default()
    }

    /// Open the emoji picker on the composer of `region` (`MURMURE-139`).
    pub fn ask_emoji(&self, region: Region) {
        self.picking.set(Some(Picking::Composer { region }));
    }

    /// The member picked `emoji`, stamped `now`, and the picker closes. A
    /// pick for a composer waits for its region, and a pick for a message
    /// is the reaction to send (`MURMURE-139`, `MURMURE-140`). `None` with
    /// the picker closed, or for a composer.
    pub fn pick_emoji(
        &self,
        emoji: ambre_bus::Emoji,
        now: Instant<Authored>,
    ) -> Option<LogicCommand> {
        let picking = self.picking.get_untracked()?;
        self.picking.set(None);
        match picking {
            Picking::Composer { region } => {
                self.picked_emoji.set(Some((region, emoji)));
                None
            }
            Picking::Reaction(pending) => Some(pending.command(emoji, now)),
        }
    }

    /// The emoji picked for the composer of `region`, taken once.
    pub fn take_emoji(&self, region: Region) -> Option<ambre_bus::Emoji> {
        let (picked, emoji) = self.picked_emoji.get_untracked()?;
        if picked != region {
            return None;
        }
        self.picked_emoji.set(None);
        Some(emoji)
    }

    /// The drafts the device keeps, which every region of the page shares.
    pub fn drafts(&self) -> DraftShelf {
        self.drafts.get_value()
    }

    /// Delete the drafts of `channels`, which the user lost (`MURMURE-215`).
    fn forget_drafts(&self, channels: &[Id<Channel>]) {
        let shelf = self.drafts();
        for channel in channels {
            shelf.forget_channel(*channel);
        }
    }

    /// The menu of the target: one entry per action it offers, each a
    /// command of the table (`MURMURE-419`). Empty with no target.
    pub fn menu_entries(&self) -> Signal<Vec<MenuEntry>> {
        let (target, me) = (self.target, self.me);
        Signal::derive(move || {
            let me = me.get();
            target.with(|target| {
                let Some(target) = target else {
                    return Vec::new();
                };
                message_actions(target, me)
                    .into_iter()
                    .filter_map(|id| ambre_commands::find(COMMANDS, id))
                    .map(|command| MenuEntry::command(command, &Locale::En))
                    .collect()
            })
        })
    }

    /// What the action `id` does on the target, which the menu closes on.
    /// `None` with no target, or for an id it does not offer.
    pub fn act(&self, id: StaticKey<Commands>) -> Option<MessageAction> {
        let target = self.target.get_untracked()?;
        let action = message_action(id, &target, self.me.get_untracked())?;
        self.target.set(None);
        Some(action)
    }

    /// The command of the deletion the member confirmed, stamped `now`, and
    /// the dialog closed (`MURMURE-512`). `None` with none asked.
    pub fn confirm_delete(&self, now: Instant<Authored>) -> Option<LogicCommand> {
        let pending = self.deleting.get_untracked()?;
        self.deleting.set(None);
        Some(pending.command(now))
    }

    /// The send the author confirmed, stamped `now`, and the dialog closed
    /// (`MURMURE-557`). `None` with none asked.
    pub fn confirm_all(&self, now: Instant<Authored>) -> Option<LogicCommand> {
        let asked = self.asking_all.get_untracked()?;
        self.asking_all.set(None);
        Some(asked.command(now))
    }

    /// The author cancelled the send: the dialog closes, and the draft waits
    /// for its region to take it back (`MURMURE-558`).
    pub fn cancel_all(&self) {
        if let Some(asked) = self.asking_all.get_untracked() {
            self.asking_all.set(None);
            self.returned.set(Some(asked));
        }
    }

    /// The cancelled send, which one region takes once: the thread's region
    /// a reply, the channel's region a message (`MURMURE-558`).
    pub fn take_returned(&self, region: Region) -> Option<AskedAll> {
        let asked = self.returned.get_untracked()?;
        if asked.region() != region {
            return None;
        }
        self.returned.set(None);
        Some(asked)
    }

    /// The edit the member asked for, which one region takes once
    /// (`MURMURE-203`). The thread's region takes a reply, and the channel's
    /// region the rest (`MURMURE-526`). `None` with none asked there.
    pub fn take_edit(&self, region: Region) -> Option<PendingEdit> {
        let pending = self.editing.get_untracked()?;
        if matches!(pending.at, MessageAt::Thread(_)) != (region == Region::Thread) {
            return None;
        }
        self.editing.set(None);
        Some(pending)
    }

    /// The signed-in member, `None` before the session names them.
    pub fn me(&self) -> Option<Id<User>> {
        self.me.get_untracked()
    }

    /// Open the search box, on the find chord (`MURMURE-213`). An open box
    /// keeps its query.
    pub fn open_search(&self) {
        self.search
            .update(|s| s.open = ambre_chrome::Openness::Open);
    }

    /// Close the search box, and forget its query and its answer.
    pub fn close_search(&self) {
        self.search.set(SearchBox::default());
    }

    /// Type `query` in the search box, and return the search it calls for.
    /// A query with no word asks nothing. The last answer shows until the
    /// new one arrives.
    pub fn search_for(&self, query: String) -> Option<LogicCommand> {
        let wanted = ambre_core::Validated::<ambre_core::SearchQuery>::parse(query.clone()).ok();
        let changed = self
            .search
            .try_update(|s| {
                let changed = s.query != query;
                s.query.clone_from(&query);
                if wanted.is_none() {
                    s.channels.clear();
                }
                changed
            })
            .unwrap_or(false);
        wanted
            .filter(|_| changed)
            .map(|query| LogicCommand::SearchMessages { query })
    }

    /// The matches of the last answer, one group per channel with a match, in
    /// the order the answer gives.
    pub fn search_groups(&self) -> Signal<Vec<SearchGroup>> {
        let (search, sidebar) = (self.search, self.sidebar);
        Signal::derive(move || {
            search.with(|s| {
                s.channels
                    .iter()
                    .filter(|c| !c.matches.is_empty())
                    .map(|c| SearchGroup {
                        channel_id: c.channel_id,
                        name: sidebar.with(|side| side.name_of(c.channel_id)),
                        matches: c
                            .matches
                            .iter()
                            .map(|m| {
                                let at = MessageRef {
                                    segment: m.segment,
                                    message: m.message.id,
                                };
                                (at, m.message.text.clone())
                            })
                            .collect(),
                        coverage: format!("{} of {}", c.covered.len(), segments(c.listed)),
                    })
                    .collect()
            })
        })
    }

    /// Choose the match `at` of `channel`: the box closes, and the channel
    /// opens as a click on the sidebar opens it. Its list rests on the
    /// message (`MURMURE-150`).
    pub fn choose_result(&self, channel: Id<Channel>, at: MessageRef) -> Option<LogicCommand> {
        self.close_search();
        self.open_at(channel, Some(at))
    }

    /// Open `channel` as a click on the sidebar opens it, its list resting
    /// on `at` when the channel shows (`MURMURE-507`). No other channel's
    /// cursor moves (`MURMURE-508`).
    pub fn open_at(&self, channel: Id<Channel>, at: Option<MessageRef>) -> Option<LogicCommand> {
        let command = self.select(channel);
        if at.is_some() && self.pane.get_untracked().shown_channel() == Some(channel) {
            self.pane.set(MurmurePane::Channel { channel, at });
        }
        command
    }

    /// The notification an event calls for, if any (`MURMURE-143`). A
    /// message that names the user notifies, and so does any message of a
    /// direct message. The channel the user is reading notifies nothing
    /// (`MURMURE-146`). A hidden page shows none to read.
    pub fn notice<S>(
        &self,
        ev: &ambre_bus::UiEvent<S, crate::message::Murmure>,
        page: PageVisibility,
    ) -> Option<Notice> {
        // The relayed event carries the launcher's sum, which Murmure never reads.
        use ambre_bus::UiEvent;
        let UiEvent::NewMessage {
            channel_id,
            mention,
            message,
        } = *ev
        else {
            return None;
        };
        let at = message.map(|(segment, message)| MessageRef { segment, message });
        let reading = page == PageVisibility::Visible
            && self.pane.get_untracked().shown_channel() == Some(channel_id);
        if reading {
            return None;
        }
        self.sidebar.with_untracked(|s| {
            let direct = s.is_direct(channel_id);
            let name = s.name_of(channel_id);
            let title = match (direct, mention, name) {
                (true, _, Some(name)) => format!("New message from {}", name.as_str()),
                (true, _, None) => "New direct message".to_owned(),
                (false, MentionState::Mentioned, Some(name)) => {
                    format!("You were mentioned in {}", name.as_str())
                }
                (false, MentionState::Mentioned, None) => "You were mentioned".to_owned(),
                (false, MentionState::Unmentioned, _) => return None,
            };
            Some(Notice {
                channel_id,
                at,
                title,
            })
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

    /// The members who watch the shown channel: the others the Logic thread
    /// heard, and this user, who has it on screen. Empty with no channel
    /// shown, so no list shows who reads another channel (`MURMURE-127`).
    pub fn shown_watchers(&self) -> Signal<HashSet<Id<User>>> {
        let (pane, watching, me) = (self.pane, self.watching, self.me);
        Signal::derive(move || {
            let Some(channel) = pane.get().shown_channel() else {
                return HashSet::new();
            };
            let mut users = watching.with(|w| w.get(&channel).cloned().unwrap_or_default());
            users.extend(me.get());
            users
        })
    }

    /// What the view sends once per refresh period: this user's presence on
    /// every space of the sidebar. Each one also turns offline the members
    /// whose presence expired (`MURMURE-445`). `now` is the view's clock.
    ///
    /// The shown channel is ticked too, since a hidden tab draws no frame
    /// (`MURMURE-521`). A visible tab ticks it from its frames as well: the
    /// Logic thread sends one sample per period whoever ticks.
    pub fn refresh_presence(&self, now: Instant<Authored>) -> Vec<LogicCommand> {
        let mut commands: Vec<LogicCommand> = self.sidebar.with_untracked(|s| {
            s.spaces()
                .iter()
                .map(|&space_id| LogicCommand::Space {
                    space_id,
                    command: SpaceCommand::RefreshPresence { now },
                })
                .collect()
        });
        if let Some(channel_id) = self.pane.get_untracked().shown_channel() {
            commands.push(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Tick { now },
            });
        }
        commands
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
    pub fn channel_name(&self) -> Signal<Option<Validated<Name>>> {
        let (sidebar, pane) = (self.sidebar, self.pane);
        Signal::derive(move || {
            let channel = pane.get().shown_channel()?;
            sidebar.with(|s| s.name_of(channel))
        })
    }

    /// Take one event, and return the commands it calls for: a space to
    /// open for its catalogue, the channels to list again after a join.
    pub fn absorb<S>(
        &self,
        ev: &ambre_bus::UiEvent<S, crate::message::Murmure>,
    ) -> Vec<LogicCommand> {
        // The relayed event carries the launcher's sum, which Murmure never reads.
        use ambre_bus::UiEvent;
        match ev {
            UiEvent::Rest {
                evt: RestEvent::SessionReady { user_id, .. },
                ..
            } => {
                self.me.set(Some(*user_id));
                Vec::new()
            }
            // The next person on the device finds no draft (`MURMURE-216`).
            UiEvent::Rest {
                evt: RestEvent::LoggedOut,
                ..
            } => {
                self.me.set(None);
                self.drafts().sign_out();
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
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Members { members },
            } => {
                let listed = members
                    .iter()
                    .map(|member| (member.user_id, member.display_name.clone()))
                    .collect();
                self.channel_members.update(|m| {
                    m.insert(*channel_id, listed);
                });
                Vec::new()
            }
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::ConfirmChannelMention { members, held },
            } => {
                self.asking_all.set(Some(AskedAll {
                    channel: *channel_id,
                    members: *members,
                    held: held.clone(),
                }));
                Vec::new()
            }
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Watchers { watching, .. },
            } => {
                self.watching.update(|w| {
                    if watching.is_empty() {
                        w.remove(channel_id);
                    } else {
                        w.insert(*channel_id, watching.iter().copied().collect());
                    }
                });
                Vec::new()
            }
            // The root a reply on a copy sought is loaded: its thread opens,
            // unless the user went to another channel since (`MURMURE-545`).
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::RootFound { segment, root, .. },
            } => {
                if self.pane.get_untracked().shown_channel() == Some(*channel_id) {
                    self.pane.set(MurmurePane::Thread {
                        channel: *channel_id,
                        root: MessageRef {
                            segment: *segment,
                            message: *root,
                        },
                    });
                }
                Vec::new()
            }
            // A channel that could not open stays in the sidebar, and the next
            // selection opens it anew (`MURMURE-549`).
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::OpenFailed,
            } => {
                self.drop_channels(&[*channel_id]);
                Vec::new()
            }
            // A channel the user lost while it was open (`MURMURE-489`).
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Left,
            } => {
                self.sidebar.update(|s| s.forget(*channel_id));
                self.drop_channels(&[*channel_id]);
                self.forget_drafts(&[*channel_id]);
                Vec::new()
            }
            // Who is in the live call of a channel: the sidebar counts it (`MURMURE-689`).
            UiEvent::Channel {
                channel_id,
                event: MurmureEvent::CallMembers { members, .. },
            } => {
                self.calls.update(|c| c.answer(*channel_id, members));
                Vec::new()
            }
            UiEvent::Space { space_id, event } => self.absorb_space(*space_id, event),
            // An answer to the query the box holds. One to an older query, or
            // to a box closed since, is dropped: typing outruns the answers.
            UiEvent::MessageSearch { query, channels } => {
                self.search.update(|s| {
                    if s.open == ambre_chrome::Openness::Open && s.query == query.as_str() {
                        s.channels.clone_from(channels);
                    }
                });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn absorb_space(&self, space: Id<Space>, event: &SpaceEvent) -> Vec<LogicCommand> {
        match event {
            SpaceEvent::Catalogue {
                name,
                role,
                categories,
                channels,
            } => {
                self.sidebar.update(|s| {
                    s.set_catalogue(space, name.clone(), categories.clone(), channels.clone());
                    s.set_role(space, *role);
                });
                Vec::new()
            }
            // The channel this page created opens at once (`MURMURE-584`).
            // The Logic thread holds it already, and the listing moves it
            // among the joined ones.
            SpaceEvent::ChannelCreated { channel_id } => {
                let mut commands = vec![LogicCommand::ListMemberChannels];
                commands.extend(self.show(*channel_id));
                commands
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
                self.forget_drafts(&[*channel_id]);
                Vec::new()
            }
            // A space that could not open keeps its joined channels, and the
            // next listing opens it anew (`MURMURE-550`).
            SpaceEvent::OpenFailed => {
                self.sidebar.update(|s| s.space_failed(space));
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
                self.forget_drafts(&dropped);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Forget the channels this page opened among `channels`, and leave the
    /// pane that shows one of them.
    fn drop_channels(&self, channels: &[Id<Channel>]) {
        self.opened.update(|o| o.retain(|c| !channels.contains(c)));
        self.channel_members
            .update(|m| m.retain(|c, _| !channels.contains(c)));
        self.watching
            .update(|w| w.retain(|c, _| !channels.contains(c)));
        let shown = self.pane.get_untracked().shown_channel();
        if shown.is_some_and(|c| channels.contains(&c)) {
            self.pane.set(MurmurePane::ChannelList);
        }
    }

    /// Select a channel of the sidebar. A joined one shows in the content,
    /// opened the first time only. One to join is joined first. Nothing is
    /// sent for the channel left behind (`MURMURE-167`).
    pub fn select(&self, channel: Id<Channel>) -> Option<LogicCommand> {
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
                            key: PaletteKey::Channel(entry.channel),
                            label: entry
                                .name
                                .map_or_else(|| words.unnamed.to_owned(), Validated::into_string),
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
                            key: PaletteKey::Member(member.user_id),
                            label: member.display_name.as_str().to_owned(),
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
    /// (`MURMURE-455`). A member who left, or the user themself, opens
    /// nothing.
    pub fn choose(&self, key: PaletteKey) -> Option<LogicCommand> {
        match key {
            PaletteKey::Channel(channel) => self.select(channel),
            PaletteKey::Member(user_id) => {
                let other =
                    self.member_name(user_id).is_some() && self.me.get_untracked() != Some(user_id);
                other.then(|| {
                    self.inviting.set(None);
                    LogicCommand::Rest {
                        id: ambre_bus::next_request(),
                        cmd: RestCommand::FetchMemberKey { user_id },
                    }
                })
            }
        }
    }

    /// The display name of `user` in an open space, if they are a member.
    pub fn member_name(
        &self,
        user: Id<User>,
    ) -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
        self.members.with_untracked(|spaces| {
            spaces
                .values()
                .flatten()
                .find(|member| member.user_id == user)
                .map(|member| member.display_name.clone())
        })
    }

    /// The shown channel, when the user may invite into it: a private
    /// channel or a direct message of theirs. A space member joins a public
    /// channel by themselves.
    pub fn invite_target(&self) -> Signal<Option<Id<Channel>>> {
        let (sidebar, pane) = (self.sidebar, self.pane);
        Signal::derive(move || {
            let channel = pane.get().shown_channel()?;
            let place = sidebar.with(|s| s.place_of(channel))?;
            (!matches!(place, ChannelPlace::Public(_))).then_some(channel)
        })
    }

    /// Who the invite control offers: the members of the space of a private
    /// channel, or of every open space for a direct message. Each comes
    /// once, and neither this user nor a member of the channel does
    /// (`MURMURE-501`).
    pub fn invitees(&self) -> Signal<Vec<MemberView>> {
        let (sidebar, members, me) = (self.sidebar, self.members, self.me);
        let in_channel = self.channel_members;
        let target = self.invite_target();
        Signal::derive(move || {
            let Some(channel) = target.get() else {
                return Vec::new();
            };
            let spaces = sidebar.with(|s| match s.place_of(channel) {
                Some(ChannelPlace::Private(space)) => vec![space],
                _ => s.spaces().to_vec(),
            });
            let me = me.get();
            let mut seen: HashSet<Id<User>> = in_channel.with(|m| {
                m.get(&channel)
                    .map(|listed| listed.keys().copied().collect())
                    .unwrap_or_default()
            });
            seen.extend(me);
            members.with(|m| {
                spaces
                    .iter()
                    .filter_map(|space| m.get(space))
                    .flatten()
                    .filter(|v| seen.insert(v.user_id))
                    .cloned()
                    .collect()
            })
        })
    }

    /// Whether the Logic thread lists `user` among the members of `channel`.
    fn in_channel(&self, channel: Id<Channel>, user: Id<User>) -> bool {
        self.channel_members.with_untracked(|m| {
            m.get(&channel)
                .is_some_and(|listed| listed.contains_key(&user))
        })
    }

    /// The members of `channel` by name, as the Logic thread last listed
    /// them. The `@` list offers them (`MURMURE-551`).
    pub fn members_of(
        &self,
        channel: Id<Channel>,
    ) -> Vec<(Id<User>, ambre_core::Validated<ambre_core::DisplayName>)> {
        self.channel_members.with_untracked(|m| {
            m.get(&channel)
                .map(|listed| listed.iter().map(|(u, n)| (*u, n.clone())).collect())
                .unwrap_or_default()
        })
    }

    /// Show the `@` list a region handed over, or close it. A region closes
    /// only its own list, so the other region's stays (`MURMURE-555`).
    pub fn show_mentions(&self, region: Region, list: Option<MentionList>) {
        let theirs = self
            .mentions
            .with_untracked(|shown| shown.as_ref().is_some_and(|s| s.region != region));
        if list.is_some() || !theirs {
            self.mentions.set(list);
        }
    }

    /// A click on the entry `index` of the `@` list shown: its region takes
    /// the pick at its next frame (`MURMURE-555`).
    pub fn pick_mention(&self, index: Pos<MentionEntry>) {
        if let Some(region) = self
            .mentions
            .with_untracked(|m| m.as_ref().map(|m| m.region))
        {
            self.picked.set(Some((region, index)));
        }
    }

    /// The pick of `region`, taken once.
    pub fn take_pick(&self, region: Region) -> Option<Pos<MentionEntry>> {
        let (picked, index) = self.picked.get_untracked()?;
        if picked != region {
            return None;
        }
        self.picked.set(None);
        Some(index)
    }

    /// Show the format bar a region handed over, or close it. A region
    /// closes only its own bar (`MURMURE-432`).
    pub fn show_bar(&self, region: Region, bar: Option<MarkBar>) {
        let theirs = self
            .mark_bar
            .with_untracked(|shown| shown.as_ref().is_some_and(|s| s.region != region));
        if bar.is_some() || !theirs {
            self.mark_bar.set(bar);
        }
    }

    /// A press on the control of `style` in the format bar shown: its region
    /// takes it at its next frame (`MURMURE-432`).
    pub fn press_mark(&self, style: TextStyle) {
        if let Some(region) = self
            .mark_bar
            .with_untracked(|bar| bar.as_ref().map(|bar| bar.region))
        {
            self.pressed_mark.set(Some((region, style)));
        }
    }

    /// The press on the bar of `region`, taken once.
    pub fn take_press(&self, region: Region) -> Option<TextStyle> {
        let (pressed, style) = self.pressed_mark.get_untracked()?;
        if pressed != region {
            return None;
        }
        self.pressed_mark.set(None);
        Some(style)
    }

    /// Invite `user` into the shown channel. Their key is fetched first, for
    /// the check of the pin, and the invite leaves only once it passes. A
    /// member of the channel is fetched nothing (`MURMURE-501`).
    pub fn invite(&self, user: Id<User>) -> Option<LogicCommand> {
        let channel = self.invite_target().get_untracked()?;
        if self.me.get_untracked() == Some(user)
            || self.member_name(user).is_none()
            || self.in_channel(channel, user)
        {
            return None;
        }
        self.inviting.set(Some((user, channel)));
        Some(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::FetchMemberKey { user_id: user },
        })
    }

    /// The channel a key fetched for `user` invites them into, taken once.
    /// `None` means the key was fetched for a direct message.
    pub fn take_invite(&self, user: Id<User>) -> Option<Id<Channel>> {
        self.inviting
            .try_update(|pending| match *pending {
                Some((invitee, channel)) if invitee == user => {
                    *pending = None;
                    Some(channel)
                }
                _ => None,
            })
            .flatten()
    }

    /// The spaces of the sidebar, each with the role of the user once known
    /// (`MURMURE-583`).
    pub fn space_heads(&self) -> Memo<Vec<SpaceHead>> {
        let sidebar = self.sidebar;
        Memo::new(move |_| sidebar.with(Sidebar::space_heads))
    }

    /// Create a space named `name`, with the user as its owner. The Logic
    /// thread shows it at once, so the sidebar lists it (`MURMURE-583`). A
    /// blank name, or one past the cap, creates nothing.
    pub fn create_space(&self, name: String, now: Instant<Authored>) -> Option<LogicCommand> {
        let name = name_typed(&name)?;
        Some(LogicCommand::CreateSpace {
            space_id: Id::<Space>::mint(),
            name,
            now,
        })
    }

    /// Create a public channel named `name` in `space`, last in its
    /// catalogue. Only an owner or an editor of the space may, and the
    /// channel opens once the relay holds it (`MURMURE-584`).
    pub fn create_channel(
        &self,
        space: Id<Space>,
        name: String,
        now: Instant<Authored>,
    ) -> Option<LogicCommand> {
        let name = name_typed(&name)?;
        let position = self.sidebar.with_untracked(|s| {
            let admin = s
                .space_heads()
                .iter()
                .any(|head| head.space == space && head.is_admin());
            admin.then(|| s.next_position(space))
        })?;
        Some(LogicCommand::Space {
            space_id: space,
            command: SpaceCommand::CreatePublicChannel {
                draft: crate::message::PublicChannelDraft {
                    channel_id: Id::<Channel>::mint(),
                    name,
                    topic: None,
                    category: None,
                    position,
                },
                now,
            },
        })
    }

    /// Invite the user at `email` into `space`. Only its owner may. Their
    /// key is fetched first, for the check of the pin, and the invite to the
    /// space document leaves once it passes (`MURMURE-585`). A blank address
    /// fetches nothing.
    pub fn invite_to_space(&self, space: Id<Space>, email: String) -> Option<LogicCommand> {
        let email = email.trim();
        let owner = self.sidebar.with_untracked(|s| {
            s.space_heads()
                .iter()
                .any(|head| head.space == space && head.is_owner())
        });
        (owner && !email.is_empty()).then(|| LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::FetchInviteeKey {
                document_id: space.document_id(),
                invitee_email: ambre_core::Email::from(email),
            },
        })
    }

    /// Open the next channel with activity after the shown one, or the
    /// previous one (`MURMURE-176`). It sends no read cursor: the channel
    /// opens as a click opens it (`MURMURE-177`).
    pub fn jump(&self, direction: StepDirection) -> Option<LogicCommand> {
        let from = self.pane.get_untracked().shown_channel();
        let to = self
            .sidebar
            .with_untracked(|s| s.next_with_activity(from, direction))?;
        self.select(to)
    }

    fn show(&self, channel: Id<Channel>) -> Option<LogicCommand> {
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
    pub fn choose(&self, key: PaletteKey) {
        if let Some(command) = self.relay.choose(key) {
            self.send.run(command);
        }
    }

    /// Run the command `id`. `false` for an id Murmure did not declare.
    fn run(&self, id: StaticKey<Commands>) -> bool {
        let command = match id {
            NEXT_ACTIVITY => self.relay.jump(StepDirection::Forward),
            PREVIOUS_ACTIVITY => self.relay.jump(StepDirection::Backward),
            // The command of the unread feature, which the relay answers for
            // every channel of the user (`MURMURE-180`).
            MARK_ALL_READ => Some(LogicCommand::MarkAllRead),
            SEARCH => {
                self.relay.open_search();
                None
            }
            KEYS => {
                self.relay.keys.set(ambre_chrome::Openness::Open);
                None
            }
            // The chord of a space of the switcher shows it, and opens nothing (`MURMURE-712`).
            id if SHOW_SPACE.contains(&id) => {
                if let Some(rank) = SHOW_SPACE
                    .iter()
                    .position(|s| *s == id)
                    .and_then(Pos::<SpaceRanks>::from_index)
                {
                    self.relay.show_space(rank);
                }
                None
            }
            _ => return false,
        };
        if let Some(command) = command {
            self.send.run(command);
        }
        true
    }
}

impl CommandHandler for MurmureHandler {
    // Murmure's commands address the user's channels, not the routed
    // document: they leave through `send`, and the shell sends none.
    type Command = Infallible;

    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<Infallible>> {
        if id == PALETTE {
            return vec![ShellAction::OpenPalette];
        }
        // An action of a message acts on the target of the menu. With none,
        // as from the palette, it does nothing (`MURMURE-419`).
        match self.relay.act(id) {
            Some(MessageAction::Open(pane)) => self.relay.pane.set(pane),
            Some(MessageAction::Send(command)) => self.send.run(command),
            Some(MessageAction::Copy(text)) => return vec![ShellAction::CopyText(text)],
            Some(MessageAction::ConfirmDelete(pending)) => self.relay.deleting.set(Some(pending)),
            Some(MessageAction::Edit(pending)) => self.relay.editing.set(Some(pending)),
            Some(MessageAction::React(pending)) => {
                self.relay.picking.set(Some(Picking::Reaction(pending)));
            }
            Some(MessageAction::JoinCall { call, channel }) => {
                return vec![ShellAction::JoinCall { call, channel }];
            }
            None => {
                self.run(id);
            }
        }
        Vec::new()
    }

    fn on_key(
        &self,
        id: StaticKey<Commands>,
        target: ambre_chrome::KeyTarget,
    ) -> KeyRoute<Infallible> {
        if id == PALETTE {
            return KeyRoute::Run(vec![ShellAction::OpenPalette]);
        }
        // A text field keeps Alt+Shift with an arrow, which extends its
        // selection on a Mac. The find chord opens the search from any field,
        // or the browser's find would take it (`MURMURE-213`).
        if target == ambre_chrome::KeyTarget::TextField && id != SEARCH {
            return KeyRoute::Yield;
        }
        self.run(id);
        KeyRoute::Run(Vec::new())
    }

    fn sealed_save(
        &self,
        _passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<Infallible> {
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
    let shown_space = relay.shown_space();
    let sections = Memo::new(move |_| {
        let space = shown_space.get();
        let tab = relay.tab.get();
        let sections: Vec<SidebarSection> = relay
            .sidebar
            .with(|s| s.sections_in(space))
            .into_iter()
            .filter(|section| tab.keeps(section.group))
            .collect();
        let shown = relay.pane.get().shown_channel();
        relay.folds.with(|folds| {
            relay
                .calls
                .with(|calls| crate::fold(sections, folds, Kept { shown, calls }))
        })
    });
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

/// The space switcher at the top of the sidebar, then the controls of the space shown (the
/// Murmure spaces spec, the Murmure layout spec). The switcher names the space shown and lists
/// every space of the user (`MURMURE-711`). A choice shows that space and opens nothing
/// (`MURMURE-712`). A signed-in user creates a space here (`MURMURE-583`). An owner or an
/// editor creates a public channel in the space shown (`MURMURE-584`), and its owner invites
/// another user (`MURMURE-585`). `now` stamps a creation, and `send` carries what the relay
/// answers.
#[component]
pub fn SpaceList(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    now: Callback<(), Instant<Authored>>,
) -> impl IntoView {
    let heads = relay.space_heads();
    let shown = relay.shown_space();
    let new_space = Callback::new(move |name: String| relay.create_space(name, now.run(())));
    let head_shown = move || {
        let shown = shown.get();
        heads.get().into_iter().find(|h| Some(h.space) == shown)
    };
    let shown_name = move || {
        head_shown().map_or_else(
            || "No space yet".to_owned(),
            |h| {
                h.name
                    .map_or_else(|| UNNAMED_SPACE.to_owned(), Validated::into_string)
            },
        )
    };
    let open = relay.switcher;
    view! {
        <section class="murmure-spaces" aria-label="Spaces">
            <button
                r#type="button"
                class="murmure-space-switch"
                aria-label=move || format!("Space: {}. Switch space", shown_name())
                aria-expanded=move || open.get().aria()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <span class="murmure-space-mark" aria-hidden="true">
                    {move || crate::ui::call_videos::initials_of(&shown_name())}
                </span>
                <span class="murmure-space-switch-name">{shown_name}</span>
                <span class="murmure-section-chevron" aria-hidden="true"></span>
            </button>
            {move || {
                (open.get() == ambre_chrome::Openness::Open)
                    .then(|| {
                        let current = shown.get();
                        view! {
                            <ul class="murmure-space-menu">
                                {heads
                                    .get()
                                    .into_iter()
                                    .enumerate()
                                    .filter_map(|(rank, head)| Pos::<SpaceRanks>::from_index(rank).map(|rank| (rank, head)))
                                    .map(|(rank, head)| {
                                        space_choice(relay, rank, head, current)
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })
            }}
            {move || head_shown().map(|head| space_view(relay, send, now, head))}
            <PromptControl words=NEW_SPACE send=send on_submit=new_space />
        </section>
    }
}

/// One space of the switcher: its name, and its chord for the first nine
/// (`ambre-spec-murmure-keyboard.md`).
fn space_choice(
    relay: MurmureRelay,
    rank: Pos<SpaceRanks>,
    head: SpaceHead,
    current: Option<Id<Space>>,
) -> impl IntoView {
    let name = head
        .name
        .map_or_else(|| UNNAMED_SPACE.to_owned(), Validated::into_string);
    // The switcher shows the chord the command table states, so a rebinding moves no view.
    let chord = SHOW_SPACE
        .get(rank.to_usize())
        .and_then(|id| crate::ui::commands::chord_of(*id))
        .map(|chord| view! { <kbd class="murmure-space-chord">{chord.to_string()}</kbd> });
    let here = (Some(head.space) == current).then_some("true");
    view! {
        <li>
            <button
                r#type="button"
                class="murmure-space-choice"
                aria-current=here
                on:click=move |_| relay.show_space(rank)
            >
                <span class="murmure-space-mark" aria-hidden="true">{crate::ui::call_videos::initials_of(&name)}</span>
                <span>{name}</span>
                {chord}
            </button>
        </li>
    }
}

/// The controls of the space shown, as its role allows.
fn space_view(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    now: Callback<(), Instant<Authored>>,
    head: SpaceHead,
) -> impl IntoView {
    let space = head.space;
    let new_channel = head.is_admin().then(|| {
        let on_submit =
            Callback::new(move |name: String| relay.create_channel(space, name, now.run(())));
        view! { <PromptControl words=NEW_CHANNEL send=send on_submit=on_submit /> }
    });
    let invite = head.is_owner().then(|| {
        let on_submit = Callback::new(move |email: String| relay.invite_to_space(space, email));
        view! { <PromptControl words=INVITE_TO_SPACE send=send on_submit=on_submit /> }
    });
    view! {
        <div class="murmure-space">
            {new_channel}
            {invite}
        </div>
    }
}

/// The bar of tabs on the bottom edge of the channel list of a phone (`MURMURE-715`). The
/// stylesheet shows it under the phone width, on the channel list alone (`MURMURE-716`). A tab
/// shows its view, and nothing leaves the page (`MURMURE-717`, `MURMURE-718`).
#[component]
pub fn PhoneTabs(relay: MurmureRelay) -> impl IntoView {
    view! {
        <nav class="murmure-tabs" aria-label="Murmure">
            {PhoneTab::ALL
                .into_iter()
                .map(|tab| {
                    let selected = move || {
                        if relay.tab.get() == tab { "true" } else { "false" }
                    };
                    view! {
                        <button
                            r#type="button"
                            class="murmure-tab"
                            data-tab=tab.word()
                            aria-pressed=selected
                            on:click=move |_| relay.choose_tab(tab)
                        >
                            <span class="murmure-tab-icon" aria-hidden="true"></span>
                            <span class="murmure-tab-label">{tab.label()}</span>
                        </button>
                    }
                })
                .collect_view()}
        </nav>
    }
}

/// The name typed in a field, trimmed. A blank one names nothing, though an
/// empty `Name` parses.
fn name_typed(text: &str) -> Option<Validated<Name>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Validated::<Name>::parse(text).ok()
}

/// The heading of a space whose catalogue names none yet.
const UNNAMED_SPACE: &str = "Unnamed space";

/// The words and the class of one control that asks for a line of text.
#[derive(Clone, Copy)]
struct PromptWords {
    /// The class of the form. The toggle and the field add `-toggle` and
    /// `-field` to it.
    class: StaticKey<ambre_core::CssClasses>,
    /// The words of the toggle.
    toggle: &'static str,
    /// The accessible name of the field.
    label: &'static str,
    /// The words of the submit button.
    submit: &'static str,
}

const NEW_SPACE: PromptWords = PromptWords {
    class: StaticKey::new("murmure-new-space"),
    toggle: "New space",
    label: "Name of the new space",
    submit: "Create",
};

const NEW_CHANNEL: PromptWords = PromptWords {
    class: StaticKey::new("murmure-new-channel"),
    toggle: "New channel",
    label: "Name of the new channel",
    submit: "Create",
};

const INVITE_TO_SPACE: PromptWords = PromptWords {
    class: StaticKey::new("murmure-space-invite"),
    toggle: "Invite",
    label: "Email of the user to invite",
    submit: "Invite",
};

/// A toggle that opens a form of one field. A submit hands the words typed
/// to `on_submit`, and sends the command it answers. A command sent closes
/// the form and clears the field; no command keeps both, so the user can
/// fix the words.
#[component]
fn PromptControl(
    words: PromptWords,
    send: Callback<LogicCommand>,
    on_submit: Callback<String, Option<LogicCommand>>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let text = RwSignal::new(String::new());
    let class = words.class.as_str();
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if let Some(command) = on_submit.run(text.get_untracked()) {
            send.run(command);
            text.set(String::new());
            open.set(ambre_chrome::Openness::Closed);
        }
    };
    view! {
        <button
            r#type="button"
            class=format!("{class}-toggle")
            aria-expanded=move || open.get().aria()
            on:click=move |_| open.update(|o| *o = o.toggled())
        >
            {words.toggle}
        </button>
        <Show when=move || open.get() == ambre_chrome::Openness::Open>
            <form class=class on:submit=submit>
                <input
                    class=format!("{class}-field")
                    r#type="text"
                    aria-label=words.label
                    placeholder=words.label
                    prop:value=move || text.get()
                    on:input=move |ev| text.set(event_target_value(&ev))
                />
                <button r#type="submit">{words.submit}</button>
            </form>
        </Show>
    }
}

/// The search box of the command bar, open on the find chord (`MURMURE-213`).
/// Each change of its words asks the Logic thread, and the answer lists the
/// matches by channel under the coverage line (`MURMURE-149`). Escape closes
/// it, and a match opens its channel. `send` carries what the relay answers.
#[component]
pub fn MessageSearch(relay: MurmureRelay, send: Callback<LogicCommand>) -> impl IntoView {
    let input = NodeRef::<leptos::html::Input>::new();
    // The box takes the caret as it opens, and again on each find chord.
    Effect::new(move |_| {
        if relay
            .search
            .with(|s| s.open == ambre_chrome::Openness::Open)
            && let Some(el) = input.get()
        {
            let _ = el.focus();
        }
    });
    let groups = relay.search_groups();
    let answered = move || relay.search.with(|s| !s.query.trim().is_empty());
    let on_input = move |ev| {
        if let Some(command) = relay.search_for(event_target_value(&ev)) {
            send.run(command);
        }
    };
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Escape" {
            relay.close_search();
        }
    };
    view! {
        <Show when=move || relay.search.with(|s| s.open == ambre_chrome::Openness::Open)>
            <div class="murmure-search" role="search">
                <input
                    node_ref=input
                    class="murmure-search-field"
                    type="search"
                    aria-label="Search messages"
                    placeholder="Search messages"
                    prop:value=move || relay.search.with(|s| s.query.clone())
                    on:input=on_input
                    on:keydown=on_keydown
                />
                <Show when=answered>
                    <div class="murmure-search-results">
                        <p class="murmure-search-coverage">
                            {move || relay.search.with(|s| coverage(&s.channels))}
                        </p>
                        {move || {
                            groups
                                .get()
                                .into_iter()
                                .map(|group| search_group_view(relay, send, group))
                                .collect_view()
                        }}
                    </div>
                </Show>
            </div>
        </Show>
    }
}

/// The dialog that confirms the deletion the actions of a message asked for
/// (`MURMURE-512`). A deletion cannot be undone, so nothing is sent before
/// Delete. Cancel, Escape and the backdrop send nothing. `now` stamps it.
#[component]
pub fn DeleteConfirm(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    now: Callback<(), Instant<Authored>>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    Effect::new(move |_| {
        open.set(ambre_chrome::Openness::from_bool(
            relay.deleting.with(Option::is_some),
        ))
    });
    let cancel = move || relay.deleting.set(None);
    let confirm = move |_| {
        if let Some(command) = relay.confirm_delete(now.run(())) {
            send.run(command);
        }
    };
    view! {
        <ambre_chrome::Dialog
            open=open
            on_close=Callback::new(move |()| cancel())
            class={StaticKey::<ambre_core::CssClasses>::new("murmure-delete")}
            label="Delete message"
        >
            <h2>"Delete this message?"</h2>
            <p>"It is deleted for every member of the channel. This cannot be undone."</p>
            // Cancel comes first, so the focus the dialog opens on is never
            // the irreversible choice.
            <div class="ambre-dialog-actions">
                <button class="murmure-delete-cancel" on:click=move |_| cancel()>"Cancel"</button>
                <button class="ambre-btn-danger murmure-delete-confirm" on:click=confirm>
                    "Delete"
                </button>
            </div>
        </ambre_chrome::Dialog>
    }
}

/// The dialog an `@channel` past the member threshold opens (`MURMURE-557`).
/// Nothing leaves before Send. Cancel, Escape and the backdrop give the
/// draft back to its composer (`MURMURE-558`). `now` stamps the send.
#[component]
pub fn ChannelMentionConfirm(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    now: Callback<(), Instant<Authored>>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    Effect::new(move |_| {
        open.set(ambre_chrome::Openness::from_bool(
            relay.asking_all.with(Option::is_some),
        ))
    });
    let title = move || {
        relay
            .asking_all
            .with(|asked| asked.as_ref().map(|a| asked_all_title(a.members)))
    };
    let confirm = move |_| {
        if let Some(command) = relay.confirm_all(now.run(())) {
            send.run(command);
        }
    };
    view! {
        <ambre_chrome::Dialog
            open=open
            on_close=Callback::new(move |()| relay.cancel_all())
            class={StaticKey::<ambre_core::CssClasses>::new("murmure-all")}
            label="Mention the whole channel"
        >
            <h2>{title}</h2>
            <p>"Each member of the channel is notified of this message."</p>
            // Cancel comes first, so the focus the dialog opens on notifies
            // nobody.
            <div class="ambre-dialog-actions">
                <button class="murmure-all-cancel" on:click=move |_| relay.cancel_all()>
                    "Cancel"
                </button>
                <button class="ambre-btn-primary murmure-all-confirm" on:click=confirm>
                    "Send"
                </button>
            </div>
        </ambre_chrome::Dialog>
    }
}

/// The top-left of the canvas a region paints, in the viewport, `(0, 0)`
/// with no such canvas in the page.
fn canvas_origin(region: Region) -> Point<Px<Css>> {
    let id = match region {
        Region::Thread => crate::ui::view::THREAD_CANVAS,
        Region::Channel => crate::ui::view::CHANNEL_CANVAS,
    };
    leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id.as_str()))
        .map(|canvas| {
            let rect = canvas.get_bounding_client_rect();
            Point::new(
                Px::<Css>::new(rect.left() as f32),
                Px::<Css>::new(rect.top() as f32),
            )
        })
        .unwrap_or_default()
}

/// The `@` list of a composer, hung above its caret (`MURMURE-555`). The
/// pipe keeps the focus, so the composer takes its keys. A press on an
/// entry picks it and leaves the focus where it was.
#[component]
pub fn MentionPanel(relay: MurmureRelay) -> impl IntoView {
    move || {
        relay.mentions.get().map(|list| {
            let at = list.at.on_page(canvas_origin(list.region));
            let (x, y) = (at.x.get(), at.y.get());
            let entries = list
                .entries
                .iter_enumerated()
                .map(|(index, name)| {
                    let name = name.clone();
                    let chosen = (index == list.chosen).to_string();
                    let pick = move |ev: leptos::ev::MouseEvent| {
                        ev.prevent_default();
                        relay.pick_mention(index);
                    };
                    view! {
                        <li role="option" class="murmure-mention" aria-selected=chosen on:mousedown=pick>
                            {name}
                        </li>
                    }
                })
                .collect_view();
            view! {
                <ul
                    class="murmure-mentions"
                    role="listbox"
                    aria-label="Mention a member"
                    style:left=format!("{x}px")
                    style:top=format!("{y}px")
                >
                    {entries}
                </ul>
            }
        })
    }
}

/// The format bar of a composer, over its selection: the shared bar Prose
/// shows, with the Murmure marks alone (`MURMURE-432`, `CHROME-105`). A
/// press toggles its style and leaves the focus where it was.
#[component]
pub fn MarkBarPanel(relay: MurmureRelay) -> impl IntoView {
    move || {
        relay.mark_bar.get().map(|bar| {
            let anchor = bar.at.on_page(canvas_origin(bar.region));
            let controls = bar
                .controls
                .into_iter()
                .map(|(style, cover)| {
                    let label = crate::ui::markers::MARK_BAR
                        .iter()
                        .find(|(shown, _)| *shown == style)
                        .map(|(_, label)| (*label).to_owned())
                        .unwrap_or_default();
                    let pressed = match cover {
                        Cover::Whole => Pressed::On,
                        Cover::Part => Pressed::Mixed,
                        Cover::None => Pressed::Off,
                    };
                    FormatControl {
                        label,
                        pressed,
                        on_press: Callback::new(move |()| relay.press_mark(style.clone())),
                    }
                })
                .collect();
            view! { <FormatBar anchor=anchor controls=controls /> }
        })
    }
}

/// The emoji picker, open on a composer or on a message (`MURMURE-139`,
/// `MURMURE-140`). The field filters the table by name, and Enter picks the
/// first match. Its emoji come from the table the client holds, so it sends
/// no request (`MURMURE-141`). Escape, the backdrop and Cancel pick nothing.
/// `now` stamps a reaction.
#[component]
pub fn EmojiPicker(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    now: Callback<(), Instant<Authored>>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let query = RwSignal::new(String::new());
    Effect::new(move |_| {
        let picking = relay.picking.with(Option::is_some);
        if picking {
            query.set(String::new());
        }
        open.set(ambre_chrome::Openness::from_bool(picking));
    });
    let pick = move |emoji: &ambre_bus::Emoji| {
        if let Some(command) = relay.pick_emoji(emoji.clone(), now.run(())) {
            send.run(command);
        }
    };
    let title = move || match relay.picking.get() {
        Some(Picking::Reaction(_)) => "Add a reaction",
        _ => "Insert an emoji",
    };
    view! {
        <ambre_chrome::Dialog
            open=open
            on_close=Callback::new(move |()| relay.picking.set(None))
            class={StaticKey::<ambre_core::CssClasses>::new("murmure-emoji")}
            label="Emoji"
        >
            <h2>{title}</h2>
            <input
                r#type="search"
                class="murmure-emoji-search"
                aria-label="Search emoji"
                placeholder="Search emoji"
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
                on:keydown=move |ev| {
                    if ev.key() == "Enter" {
                        ev.prevent_default();
                        if let Some(first) = emoji::search(&query.get_untracked()).first() {
                            pick(&first.emoji);
                        }
                    }
                }
            />
            <div class="murmure-emoji-grid" role="group" aria-label="Emoji">
                {move || {
                    emoji::search(&query.get())
                        .into_iter()
                        .map(|e| {
                            let glyph = e.emoji.as_str().to_owned();
                            view! {
                                <button
                                    r#type="button"
                                    class="murmure-emoji-pick"
                                    aria-label=e.name
                                    title=e.name
                                    on:click=move |_| pick(&e.emoji)
                                >
                                    {glyph}
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </div>
            <div class="ambre-dialog-actions">
                <button class="murmure-emoji-cancel" on:click=move |_| relay.picking.set(None)>
                    "Cancel"
                </button>
            </div>
        </ambre_chrome::Dialog>
    }
}

/// The list of the keys Murmure takes, open on the keys command
/// (`MURMURE-540`). Each row names a key, what it does and where. Its keys
/// show at every width (`MURMURE-542`). Escape, the backdrop and Close close
/// it.
#[component]
pub fn KeyList(relay: MurmureRelay) -> impl IntoView {
    let rows = key_list(&Locale::En);
    view! {
        <ambre_chrome::Dialog
            open=relay.keys
            class={StaticKey::<ambre_core::CssClasses>::new("murmure-keys")}
            label="Keyboard shortcuts"
        >
            <h2>"Keyboard shortcuts"</h2>
            <table class="murmure-keys-table">
                <thead>
                    <tr>
                        <th scope="col">"Key"</th>
                        <th scope="col">"Action"</th>
                        <th scope="col">"Where"</th>
                    </tr>
                </thead>
                <tbody>
                    {rows
                        .clone()
                        .into_iter()
                        .map(|row| {
                            view! {
                                <tr>
                                    // Not the chip of a menu, which hides below
                                    // the desktop width: here the keys are what
                                    // the member asked to read.
                                    <td>
                                        <kbd class="murmure-keys-key">{row.keys.to_string()}</kbd>
                                    </td>
                                    <td>{row.action}</td>
                                    <td>{KeyPlace::words(row.place)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                </tbody>
            </table>
            <div class="ambre-dialog-actions">
                <button class="murmure-keys-close" on:click=move |_| relay.keys.set(ambre_chrome::Openness::Closed)>
                    "Close"
                </button>
            </div>
        </ambre_chrome::Dialog>
    }
}

fn search_group_view(
    relay: MurmureRelay,
    send: Callback<LogicCommand>,
    group: SearchGroup,
) -> impl IntoView {
    let channel = group.channel_id;
    view! {
        <section class="murmure-search-group">
            <h3 class="murmure-search-channel">
                {group.name.map_or_else(|| UNNAMED_CHANNEL.to_owned(), Validated::into_string)}
                <span class="murmure-search-segments">{group.coverage}</span>
            </h3>
            <ul>
                {group
                    .matches
                    .into_iter()
                    .map(|(at, text)| {
                        view! {
                            <li>
                                <button
                                    r#type="button"
                                    class="murmure-search-match"
                                    on:click=move |_| {
                                        if let Some(command) = relay.choose_result(channel, at) {
                                            send.run(command);
                                        }
                                    }
                                >
                                    {text}
                                </button>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        </section>
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
    drawn: DrawnSection,
) -> impl IntoView {
    let DrawnSection {
        section,
        folding,
        hidden,
        mark,
    } = drawn;
    let GroupWords {
        word,
        title: fallback,
        unnamed,
        ..
    } = GroupWords::of(section.group);
    let title = section
        .title
        .map_or_else(|| fallback.to_owned(), Validated::into_string);
    let group = section.group;
    // A folded group says how many channels it hides, and marks one with activity
    // (`MURMURE-722`).
    let summary = (folding == Folding::Folded && !hidden.is_zero()).then(|| {
        view! {
            <span class="murmure-section-hidden" aria-label=format!("{} hidden", hidden.get())>
                {(mark == HiddenMark::Active).then(|| view! { <span class="murmure-mark"></span> })}
                {hidden.get()}
            </span>
        }
    });
    view! {
        <section class="murmure-section" data-group=word data-folded=folding.expanded()>
            <h3 class="murmure-section-head">
                <button
                    r#type="button"
                    class="murmure-section-title"
                    aria-expanded=folding.expanded()
                    on:click=move |_| relay.toggle_fold(group)
                >
                    <span class="murmure-section-chevron" aria-hidden="true"></span>
                    <span class="murmure-section-name">{title}</span>
                    {summary}
                </button>
            </h3>
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
                {(mentions.get() > 0).then(|| view! {
                    <span class="murmure-mentions" aria-label="Mentions">{mentions.get()}</span>
                })}
                {(messages.get() > 0).then(|| view! {
                    <span class="murmure-count" aria-label="Unread messages">{messages.get()}</span>
                })}
            }
            .into_any(),
        ),
    };
    // A live call shows a mark and the count of its participants, and no name. The entry of the
    // channel shown adds their initials (`MURMURE-689`, `MURMURE-690`).
    let call = move || {
        let seated = relay.calls.with(|c| c.of(channel).cloned())?;
        let count = seated.count().get();
        let shown = relay.pane.get().shown_channel() == Some(channel);
        let faces = shown.then(|| {
            seated
                .members
                .iter()
                .map(|member| {
                    let name = relay.member_name(*member);
                    let initials = crate::ui::call_videos::initials(name.as_ref());
                    view! { <span class="murmure-call-face" aria-hidden="true">{initials}</span> }
                })
                .collect_view()
        });
        Some(view! {
            <span class="murmure-call-live" aria-label=format!("Call in progress, {count} in it")>
                <span class="murmure-call-live-icon" aria-hidden="true"></span>
                {faces}
                <span class="murmure-call-live-count">{count}</span>
            </span>
        })
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
                    {entry.name.map_or_else(|| unnamed.to_owned(), Validated::into_string)}
                </span>
                {call}
                {badge}
            </button>
        </li>
    }
}
