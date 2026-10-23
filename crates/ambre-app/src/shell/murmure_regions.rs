// ambre-app/src/shell/murmure_regions.rs
// Browser-tier tests of the Murmure shell at a set width (the Murmure layout
// spec). A width query reads the viewport of the frame it runs in. So each
// test mounts the shell into an iframe of the width it reads.

use super::app_root::MurmureShell;
use ambre_bus::{
    CatalogueEntry, CategoryView, ChannelActivity, ChannelMatches, ChannelName, ChannelPlace,
    DirectPeers, LogicCommand, MemberChannel, MemberView, MentionCount, MessageView, Presence,
    SearchMatch, SpaceCommand, SpaceEvent, Timestamp, UiEvent, UnreadCount,
};
use ambre_core::{CategoryId, ChannelId, MessageId, SegmentId, SpaceId, UserId};
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::{MessageRef, MurmurePane};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

/// The shell stylesheet, as the page loads it.
const STYLES: &str = include_str!("../../styles.css");

/// The frame's height.
const HEIGHT: u32 = 400;

/// The width of each body region on screen, zero for one hidden or absent.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Regions {
    sidebar: f64,
    content: f64,
    panel: f64,
}

impl Regions {
    /// Which regions are on screen, in their order.
    fn shown(self) -> (bool, bool, bool) {
        (self.sidebar > 0.0, self.content > 0.0, self.panel > 0.0)
    }
}

/// A Murmure shell mounted in a frame of `width` pixels.
struct Frame {
    frame: web_sys::HtmlIFrameElement,
    inner: web_sys::Document,
    pane: RwSignal<MurmurePane>,
    relay: MurmureRelay,
    /// What the channel list sent the Logic thread, in order.
    sent: RwSignal<Vec<LogicCommand>>,
    /// Unmounts the shell on drop.
    _handle: Box<dyn std::any::Any>,
}

impl Frame {
    /// The shell on `pane`, its channel named `general`.
    async fn new(width: u32, pane: MurmurePane) -> Self {
        let relay = MurmureRelay::new();
        relay.pane.set(pane);
        if let Some(channel) = pane.shown_channel() {
            relay
                .sidebar
                .update(|s| s.set_name(channel, Some(ambre_core::Name::new("general").unwrap())));
        }
        Self::with(width, relay).await
    }

    async fn with(width: u32, relay: MurmureRelay) -> Self {
        let frame: web_sys::HtmlIFrameElement = document()
            .create_element("iframe")
            .unwrap()
            .dyn_into()
            .unwrap();
        let style = format!("width:{width}px;height:{HEIGHT}px;border:0");
        frame.set_attribute("style", &style).unwrap();
        document().body().unwrap().append_child(&frame).unwrap();
        let inner = frame.content_document().unwrap();
        // The colour tokens, then the shell sheet, as the page loads them.
        for css in [ambre_canvas::theme::THEME_CSS, STYLES] {
            let sheet = inner.create_element("style").unwrap();
            sheet.set_text_content(Some(css));
            inner.head().unwrap().append_child(&sheet).unwrap();
        }
        let fill = inner.create_element("style").unwrap();
        fill.set_text_content(Some("html,body{margin:0;height:100%}"));
        inner.head().unwrap().append_child(&fill).unwrap();

        let sent = RwSignal::new(Vec::new());
        let send = Callback::new(move |command| sent.update(|s| s.push(command)));
        let handle = mount_to(inner.body().unwrap(), move || {
            view! {
                <MurmureShell relay=relay send=send>
                    <span class="title">"Ambre"</span>
                </MurmureShell>
            }
        });
        leptos::task::tick().await;
        Self {
            frame,
            inner,
            pane: relay.pane,
            relay,
            sent,
            _handle: Box::new(handle),
        }
    }

    /// Hand the relay one event, as the per-frame loop does, and return the
    /// commands it answers.
    async fn absorb(&self, ev: UiEvent) -> Vec<LogicCommand> {
        let commands = self.relay.absorb(&ev);
        leptos::task::tick().await;
        commands
    }

    /// The text of every element `selector` names, in order.
    fn texts(&self, selector: &str) -> Vec<String> {
        let list = self.inner.query_selector_all(selector).unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.text_content())
            .collect()
    }

    /// The sidebar entry of the channel named `name`.
    fn entry(&self, name: &str) -> web_sys::HtmlElement {
        let list = self.inner.query_selector_all(".murmure-entry").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
            .find(|el| {
                el.query_selector(".murmure-entry-name")
                    .unwrap()
                    .and_then(|n| n.text_content())
                    .as_deref()
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("{name} is in the sidebar"))
    }

    async fn select(&self, name: &str) {
        self.entry(name).click();
        leptos::task::tick().await;
    }
}

/// A space of one category, `general` outside it and `sales` in it, with
/// `random` to join; a private `board` and a direct message. The user
/// belongs to all but `random`.
struct Acme {
    space: SpaceId,
    general: ChannelId,
    sales: ChannelId,
    random: ChannelId,
    board: ChannelId,
    direct: ChannelId,
}

impl Acme {
    fn new() -> Self {
        Self {
            space: SpaceId::new(),
            general: ChannelId::new(),
            sales: ChannelId::new(),
            random: ChannelId::new(),
            board: ChannelId::new(),
            direct: ChannelId::new(),
        }
    }

    fn members(&self, also: &[ChannelId]) -> UiEvent {
        let public = ChannelPlace::Public(self.space);
        let mut channels = vec![
            MemberChannel {
                channel_id: self.direct,
                place: ChannelPlace::Direct,
            },
            MemberChannel {
                channel_id: self.sales,
                place: public,
            },
            MemberChannel {
                channel_id: self.board,
                place: ChannelPlace::Private(self.space),
            },
            MemberChannel {
                channel_id: self.general,
                place: public,
            },
        ];
        channels.extend(also.iter().map(|&channel_id| MemberChannel {
            channel_id,
            place: public,
        }));
        UiEvent::MemberChannels {
            channels,
            spaces: vec![self.space],
            names: Vec::new(),
            peers: Vec::new(),
        }
    }

    fn catalogue(&self) -> UiEvent {
        let team = CategoryId::new();
        let entry = |channel_id, name: &str, category, position| CatalogueEntry {
            channel_id,
            name: ambre_core::Name::new(name).unwrap(),
            topic: None,
            category,
            position,
        };
        UiEvent::Space {
            space_id: self.space,
            event: SpaceEvent::Catalogue {
                name: Some(ambre_core::Name::new("Acme").unwrap()),
                categories: vec![CategoryView {
                    id: team,
                    name: ambre_core::Name::new("Team").unwrap(),
                    position: 0,
                }],
                channels: vec![
                    entry(self.random, "random", None, 1),
                    entry(self.sales, "sales", Some(team), 0),
                    entry(self.general, "general", None, 0),
                ],
            },
        }
    }

    /// The sidebar of a desktop, filled as at start.
    async fn frame(&self) -> Frame {
        let frame = Frame::with(1025, MurmureRelay::new()).await;
        let opened = frame.absorb(self.members(&[])).await;
        assert!(
            matches!(opened[..], [LogicCommand::OpenSpace { space_id }] if space_id == self.space),
            "the space opens once, for its catalogue: {opened:?}"
        );
        assert!(frame.absorb(self.catalogue()).await.is_empty());
        frame
    }
}

fn activity(channel_id: ChannelId, active: bool, counts: Option<(u32, u32)>) -> UiEvent {
    UiEvent::ChannelActivity {
        channels: vec![ChannelActivity {
            channel_id,
            active,
            unread: counts.map(|(m, _)| UnreadCount(m)),
            mentions: counts.map(|(_, n)| MentionCount(n)),
        }],
    }
}

impl Frame {
    /// The element `selector` names, if it is in the frame.
    fn query(&self, selector: &str) -> Option<web_sys::Element> {
        self.inner.query_selector(selector).unwrap()
    }

    fn width_of(&self, selector: &str) -> f64 {
        self.query(selector)
            .map_or(0.0, |el| el.get_bounding_client_rect().width())
    }

    async fn regions(&self) -> Regions {
        leptos::task::tick().await;
        let _ = self.frame.offset_width();
        Regions {
            sidebar: self.width_of(".ambre-sidebar"),
            content: self.width_of(".ambre-content"),
            panel: self.width_of(".ambre-panel"),
        }
    }

    async fn click(&self, selector: &str) {
        let el: web_sys::HtmlElement = self
            .query(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"))
            .dyn_into()
            .unwrap();
        el.click();
        leptos::task::tick().await;
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.frame.remove();
    }
}

fn channel() -> MurmurePane {
    MurmurePane::channel(ChannelId::new())
}

fn thread() -> MurmurePane {
    MurmurePane::Thread {
        channel: ChannelId::new(),
        root: MessageRef {
            segment: SegmentId::new(),
            message: MessageId::new(),
        },
    }
}

/// Over the desktop width, the sidebar, the channel and an open thread sit
/// side by side (`MURMURE-158`). The widest tablet still shows two.
#[wasm_bindgen_test]
async fn a_desktop_shows_the_sidebar_the_channel_and_the_thread_side_by_side() {
    let frame = Frame::new(1025, thread()).await;
    assert_eq!(frame.regions().await.shown(), (true, true, true));
    let frame = Frame::new(1024, thread()).await;
    assert_eq!(frame.regions().await.shown(), (true, false, true));
}

/// The channel and the thread are two canvases, one in each cell, and each
/// is as wide as its own cell (`MURMURE-421`). The member list in the panel
/// holds no canvas.
#[wasm_bindgen_test]
async fn the_channel_and_the_thread_are_each_a_canvas_of_their_cell() {
    let frame = Frame::new(1100, thread()).await;
    let regions = frame.regions().await;
    let canvases = frame.inner.query_selector_all("canvas").unwrap();
    assert_eq!(canvases.length(), 2);
    let panel = format!(".ambre-panel #{}", crate::host::THREAD_CANVAS);
    assert_eq!(
        frame.width_of(".ambre-content > #ambre-canvas"),
        regions.content
    );
    assert_eq!(
        frame.width_of(&panel),
        regions.panel - 1.0,
        "less its border"
    );
    assert_ne!(regions.content, regions.panel);
    // The thread takes its own keys, so a screen reader hands it the arrows
    // (`MURMURE-510`).
    let thread = frame.query(&panel).expect("the thread's canvas");
    assert_eq!(thread.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(thread.get_attribute("role").as_deref(), Some("application"));
    assert_eq!(
        thread.get_attribute("aria-label").as_deref(),
        Some("Thread")
    );

    let channel = ChannelId::new();
    frame.pane.set(MurmurePane::Members { channel });
    leptos::task::tick().await;
    assert!(frame.query(&panel).is_none());
    assert_eq!(
        frame.inner.query_selector_all("canvas").unwrap().length(),
        1
    );
}

/// Between the phone and the desktop widths, an open thread replaces the
/// channel, and the back control shows the channel again (`MURMURE-159`).
#[wasm_bindgen_test]
async fn a_tablet_thread_replaces_the_channel_with_a_way_back() {
    for width in [600, 1024] {
        let frame = Frame::new(width, thread()).await;
        assert_eq!(
            frame.regions().await.shown(),
            (true, false, true),
            "{width} px"
        );
        frame.click(".murmure-back").await;
        assert_eq!(
            frame.regions().await.shown(),
            (true, true, false),
            "{width} px"
        );
    }
}

/// Under the phone width, Murmure shows one view at a time: the channel
/// list, a channel or a thread (`MURMURE-160`).
#[wasm_bindgen_test]
async fn a_phone_shows_one_view_at_a_time() {
    for width in [360, 599] {
        for (pane, shown) in [
            (MurmurePane::ChannelList, (true, false, false)),
            (channel(), (false, true, false)),
            (thread(), (false, false, true)),
        ] {
            let frame = Frame::new(width, pane).await;
            let regions = frame.regions().await;
            assert_eq!(regions.shown(), shown, "{width} px, {pane:?}");
            let on_screen = [regions.sidebar, regions.content, regions.panel];
            assert!(
                on_screen.contains(&f64::from(width)),
                "the one view fills the {width} px window"
            );
        }
    }
}

/// A region collapses at a narrow width, never disappears (`MURMURE-161`).
/// The folded sidebar is a strip that unfolds again, and a phone reaches the
/// channel list from the channel through the back control.
#[wasm_bindgen_test]
async fn a_region_collapses_and_never_disappears() {
    let frame = Frame::new(800, channel()).await;
    let open = frame.regions().await.sidebar;
    frame.click(".ambre-sidebar-fold").await;
    let strip = frame.regions().await.sidebar;
    assert!(strip > 0.0 && strip < open, "the folded sidebar is a strip");
    assert!(frame.query(".ambre-sidebar-fold").is_some());
    frame.click(".ambre-sidebar-fold").await;
    assert_eq!(frame.regions().await.sidebar, open);

    let frame = Frame::new(360, channel()).await;
    assert!(
        frame.query(".ambre-sidebar").is_some(),
        "the list stays mounted"
    );
    frame.click(".murmure-back").await;
    assert_eq!(frame.regions().await.shown(), (true, false, false));
}

/// The layout reads the three widths of the adaptive layout: each region
/// flips exactly at 600 and at 1025 pixels (`MURMURE-162`).
#[wasm_bindgen_test]
async fn the_layout_reads_the_three_widths() {
    for (width, shown) in [
        (599, (false, true, false)),
        (600, (true, true, false)),
        (1024, (true, true, false)),
        (1025, (true, true, false)),
    ] {
        let frame = Frame::new(width, channel()).await;
        assert_eq!(
            frame.regions().await.shown(),
            shown,
            "{width} px, a channel"
        );
    }
    for (width, shown) in [
        (599, (false, false, true)),
        (600, (true, false, true)),
        (1024, (true, false, true)),
        (1025, (true, true, true)),
    ] {
        let frame = Frame::new(width, thread()).await;
        assert_eq!(frame.regions().await.shown(), shown, "{width} px, a thread");
    }
}

/// Back from a thread shows its channel at the root message, then the list
/// (`MURMURE-168`). The back control writes the pane alone: the shell holds
/// no command sender, so going back moves no read cursor (`MURMURE-169`).
#[wasm_bindgen_test]
async fn back_shows_the_channel_at_the_root_then_the_list() {
    let pane = thread();
    let MurmurePane::Thread { channel, root } = pane else {
        unreachable!()
    };
    let frame = Frame::new(360, pane).await;
    frame.click(".murmure-back").await;
    assert_eq!(
        frame.pane.get_untracked(),
        MurmurePane::Channel {
            channel,
            at: Some(root)
        }
    );
    frame.click(".murmure-back").await;
    assert_eq!(frame.pane.get_untracked(), MurmurePane::ChannelList);
    assert!(
        frame.query(".murmure-back").is_none(),
        "the list is the first pane"
    );
}

/// The command bar holds the channel header, and no status bar row exists
/// (`MURMURE-428`, `MURMURE-429`).
#[wasm_bindgen_test]
async fn the_command_bar_holds_the_channel_header_and_no_status_bar_shows() {
    let frame = Frame::new(1025, channel()).await;
    let name = frame
        .query(".ambre-commandbar > .murmure-channel-header .murmure-channel-name")
        .expect("the channel header is in the command bar");
    assert_eq!(name.text_content().as_deref(), Some("general"));
    assert!(frame.query(".ambre-statusbar").is_none());
    assert!(frame.query(".ambre-formulabar").is_none());
    assert!(frame.query(".ambre-tabstrip").is_none());
}

/// The channel each command opens, `None` for any other command.
fn opened(commands: &[LogicCommand]) -> Vec<Option<ChannelId>> {
    commands
        .iter()
        .map(|c| match c {
            LogicCommand::OpenChannel { channel_id } => Some(*channel_id),
            _ => None,
        })
        .collect()
}

/// The sidebar draws the joined public channels of the space by category,
/// then the private channels and the direct messages (`MURMURE-163`). The
/// channel to join is in a section apart (`MURMURE-164`).
#[wasm_bindgen_test]
async fn the_sidebar_draws_its_sections_in_order() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    assert_eq!(
        frame.texts(".murmure-section-title"),
        [
            "Acme",
            "Team",
            "Private channels",
            "Direct messages",
            "Channels to join"
        ]
    );
    assert_eq!(
        frame.texts(".murmure-entry-name"),
        [
            "general",
            "sales",
            "Private channel",
            "Direct message",
            "random"
        ],
        "a private channel or a direct message the relay names nothing for"
    );
    let groups = frame.inner.query_selector_all(".murmure-section").unwrap();
    let last = groups.item(groups.length() - 1).unwrap();
    let last: web_sys::Element = last.dyn_into().unwrap();
    assert_eq!(last.get_attribute("data-group").as_deref(), Some("to-join"));
}

/// A private channel and a named direct message show their names before
/// either opens, as the relay's listing carried them (`MURMURE-490`).
#[wasm_bindgen_test]
async fn a_named_channel_not_open_shows_its_name() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let mut listing = acme.members(&[]);
    if let UiEvent::MemberChannels { names, .. } = &mut listing {
        let named = |channel_id, name: &str| ChannelName {
            channel_id,
            name: ambre_core::Name::new(name).unwrap(),
        };
        *names = vec![named(acme.board, "board"), named(acme.direct, "trip")];
    }
    assert!(frame.absorb(listing).await.is_empty());
    assert_eq!(
        frame.texts(".murmure-entry-name"),
        ["general", "sales", "board", "trip", "random"]
    );
}

/// A direct message nobody named shows its other members, closed and once
/// open, and the header names it the same way (`MURMURE-492`).
#[wasm_bindgen_test]
async fn an_unnamed_direct_message_shows_its_other_members() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let mut listing = acme.members(&[]);
    if let UiEvent::MemberChannels { peers, .. } = &mut listing {
        *peers = vec![DirectPeers {
            channel_id: acme.direct,
            names: vec![
                ambre_core::DisplayName::new("Bob").unwrap(),
                ambre_core::DisplayName::new("Carol").unwrap(),
            ],
        }];
    }
    assert!(frame.absorb(listing).await.is_empty());
    assert_eq!(
        frame.texts(".murmure-entry-name"),
        [
            "general",
            "sales",
            "Private channel",
            "Bob, Carol",
            "random"
        ]
    );
    frame.select("Bob, Carol").await;
    assert_eq!(frame.texts(".murmure-channel-name"), ["Bob, Carol"]);
}

/// A channel with activity shows the relay's mark before any count, then
/// its counts once known, and a read channel shows nothing (`MURMURE-119`,
/// `MURMURE-165`).
#[wasm_bindgen_test]
async fn a_channel_shows_its_mark_then_its_counts() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let badges = |frame: &Frame| {
        let entry = frame.entry("sales");
        let mark = entry.query_selector(".murmure-mark").unwrap().is_some();
        let text = |selector| {
            entry
                .query_selector(selector)
                .unwrap()
                .and_then(|el| el.text_content())
        };
        (mark, text(".murmure-count"), text(".murmure-mentions"))
    };
    assert_eq!(badges(&frame), (false, None, None));

    frame.absorb(activity(acme.sales, true, None)).await;
    assert_eq!(badges(&frame), (true, None, None), "the mark, no count yet");

    frame.absorb(activity(acme.sales, true, Some((3, 1)))).await;
    assert_eq!(
        badges(&frame),
        (false, Some("3".to_owned()), Some("1".to_owned()))
    );

    frame.absorb(activity(acme.general, true, None)).await;
    assert_eq!(
        badges(&frame),
        (false, Some("3".to_owned()), Some("1".to_owned())),
        "a channel absent from an answer keeps its counts"
    );

    frame
        .absorb(activity(acme.sales, false, Some((0, 0))))
        .await;
    assert_eq!(
        badges(&frame),
        (false, None, None),
        "read, it shows nothing"
    );
}

/// Selecting a channel shows it in the content and marks it current. It
/// opens the first time only, and nothing is sent for the channel left
/// behind, so its read cursor stays (`MURMURE-167`).
#[wasm_bindgen_test]
async fn selecting_a_channel_sends_nothing_for_the_one_left_behind() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let current =
        |frame: &Frame| frame.texts(".murmure-entry[aria-current=\"page\"] .murmure-entry-name");

    frame.select("general").await;
    assert_eq!(
        frame.pane.get_untracked(),
        MurmurePane::channel(acme.general)
    );
    assert_eq!(current(&frame), ["general"]);
    assert_eq!(frame.texts(".murmure-channel-name"), ["general"]);

    frame.select("sales").await;
    assert_eq!(current(&frame), ["sales"]);
    frame.select("general").await;
    assert_eq!(
        opened(&frame.sent.get_untracked()),
        [Some(acme.general), Some(acme.sales)],
        "each channel opens once, and nothing else is sent"
    );
}

/// Selecting a channel to join joins it in its space. The join shows the
/// channel, opens it, and lists the channels again, so it moves among the
/// joined ones.
#[wasm_bindgen_test]
async fn a_channel_to_join_is_joined_then_shown() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    frame.select("random").await;
    let sent = frame.sent.get_untracked();
    assert!(
        matches!(
            sent[..],
            [LogicCommand::Space {
                space_id,
                command: SpaceCommand::JoinChannel { channel_id },
            }] if space_id == acme.space && channel_id == acme.random
        ),
        "{sent:?}"
    );
    assert_eq!(frame.pane.get_untracked(), MurmurePane::ChannelList);

    let joined = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Joined {
            channel_id: acme.random,
        },
    };
    let answer = frame.absorb(joined).await;
    assert!(matches!(answer[0], LogicCommand::ListMemberChannels));
    assert_eq!(opened(&answer[1..]), [Some(acme.random)]);
    assert_eq!(
        frame.pane.get_untracked(),
        MurmurePane::channel(acme.random)
    );

    assert!(frame.absorb(acme.members(&[acme.random])).await.is_empty());
    assert!(
        !frame
            .texts(".murmure-section-title")
            .contains(&"Channels to join".to_owned()),
        "nothing is left to join"
    );
}

/// A space the user joined no channel of opens for its catalogue, and its
/// channels show to join (`MURMURE-486`).
#[wasm_bindgen_test]
async fn a_space_with_no_channel_of_the_user_shows_its_channels_to_join() {
    let acme = Acme::new();
    let frame = Frame::with(1025, MurmureRelay::new()).await;
    let alone = UiEvent::MemberChannels {
        channels: Vec::new(),
        spaces: vec![acme.space],
        names: Vec::new(),
        peers: Vec::new(),
    };
    let opened = frame.absorb(alone.clone()).await;
    assert!(
        matches!(opened[..], [LogicCommand::OpenSpace { space_id }] if space_id == acme.space),
        "{opened:?}"
    );
    assert!(frame.absorb(acme.catalogue()).await.is_empty());
    assert_eq!(frame.texts(".murmure-section-title"), ["Channels to join"]);
    let mut names = frame.texts(".murmure-entry-name");
    names.sort();
    assert_eq!(names, ["general", "random", "sales"]);
    assert!(frame.absorb(alone).await.is_empty(), "it opens once");
}

/// A public channel the user left moves among those to join, and a pane
/// showing it goes back to the channel list.
#[wasm_bindgen_test]
async fn a_channel_left_leaves_the_sidebar_and_the_pane() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    frame.select("sales").await;
    let left = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::ChannelLeft {
            channel_id: acme.sales,
        },
    };
    assert!(frame.absorb(left).await.is_empty());
    assert_eq!(frame.pane.get_untracked(), MurmurePane::ChannelList);
    assert_eq!(
        frame.texts(".murmure-section[data-group=\"to-join\"] .murmure-entry-name"),
        ["sales", "random"]
    );
}

/// A space the user lost leaves the sidebar with every channel of it, and a
/// pane showing one goes back to the channel list. A later invite shows it
/// again (`MURMURE-488`).
#[wasm_bindgen_test]
async fn a_space_lost_leaves_the_sidebar_with_its_channels() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    frame.select("sales").await;
    let lost = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Left,
    };
    assert!(frame.absorb(lost).await.is_empty());
    assert_eq!(frame.pane.get_untracked(), MurmurePane::ChannelList);
    assert_eq!(frame.texts(".murmure-section-title"), ["Direct messages"]);

    let again = frame.absorb(acme.members(&[])).await;
    assert!(
        matches!(again[..], [LogicCommand::OpenSpace { space_id }] if space_id == acme.space),
        "{again:?}"
    );
    assert!(frame.absorb(acme.catalogue()).await.is_empty());
    frame.select("sales").await;
    let sent = frame.sent.get_untracked();
    assert!(
        matches!(
            sent[..],
            [LogicCommand::OpenChannel { channel_id: a }, LogicCommand::OpenChannel { channel_id: b }]
                if a == acme.sales && b == acme.sales
        ),
        "the channel opens again: {sent:?}"
    );
}

/// Alice online and Bob offline, as the Logic thread lists the members of
/// the space.
fn alice_and_bob(acme: &Acme) -> (UserId, UiEvent) {
    let alice = UserId::new();
    let member = |user_id, name: &str, presence| MemberView {
        user_id,
        display_name: ambre_core::DisplayName::new(name).unwrap(),
        presence,
    };
    let event = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Members {
            members: vec![
                member(alice, "Alice Martin", Presence::Online),
                member(UserId::new(), "Bob", Presence::Offline),
            ],
        },
    };
    (alice, event)
}

impl Frame {
    /// The computed value of `property` on the element `selector` names.
    fn style_of(&self, selector: &str, property: &str) -> String {
        let el = self
            .query(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"));
        self.frame
            .content_window()
            .unwrap()
            .get_computed_style(&el)
            .unwrap()
            .unwrap()
            .get_property_value(property)
            .unwrap()
    }

    async fn filter(&self, text: &str) {
        self.type_into(".murmure-members-filter", text).await;
    }

    /// Type `text` into the field `selector` names, as one input event.
    async fn type_into(&self, selector: &str, text: &str) {
        let input: web_sys::HtmlInputElement = self
            .query(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"))
            .dyn_into()
            .unwrap();
        input.set_value(text);
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
    }
}

/// The header of a channel in a space opens the member list of the space
/// in the panel. Each member shows by name and initials on their identity
/// colour (`MURMURE-441`, `PROFILE-002`), with a mark for their presence
/// (`MURMURE-442`). The filter narrows the list and sends nothing.
#[wasm_bindgen_test]
async fn the_member_list_shows_each_member_by_name_initials_and_presence() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (alice, members) = alice_and_bob(&acme);
    assert!(frame.absorb(members).await.is_empty());
    frame.select("sales").await;
    assert!(frame.query(".murmure-members").is_none(), "closed at first");
    frame.click(".murmure-members-toggle").await;
    assert_eq!(
        frame.pane.get_untracked(),
        MurmurePane::Members {
            channel: acme.sales
        }
    );
    assert!(
        frame.regions().await.panel > 0.0,
        "the panel holds the list"
    );
    let panel = frame.query(".ambre-panel").unwrap();
    assert_eq!(
        panel.get_attribute("aria-label").as_deref(),
        Some("Members")
    );

    assert_eq!(frame.texts(".murmure-member-name"), ["Alice Martin", "Bob"]);
    assert_eq!(frame.texts(".murmure-member .ambre-avatar"), ["AM", "BO"]);
    let avatar = frame.query(".murmure-member .ambre-avatar").unwrap();
    assert_eq!(
        avatar.get_attribute("title").as_deref(),
        Some("Alice Martin")
    );
    let [r, g, b, _] = ambre_draw::peer_color(&alice).to_rgba8().to_u8_array();
    assert_eq!(
        frame.style_of(".murmure-member .ambre-avatar", "background-color"),
        format!("rgb({r}, {g}, {b})"),
        "the avatar takes the identity colour"
    );
    let marks = frame.inner.query_selector_all(".murmure-presence").unwrap();
    let mark = |i| -> (Option<String>, Option<String>) {
        let el: web_sys::Element = marks.item(i).unwrap().dyn_into().unwrap();
        (
            el.get_attribute("data-presence"),
            el.get_attribute("aria-label"),
        )
    };
    assert_eq!(mark(0), (Some("online".into()), Some("Online".into())));
    assert_eq!(mark(1), (Some("offline".into()), Some("Offline".into())));

    let sent = frame.sent.get_untracked().len();
    frame.filter("bo").await;
    assert_eq!(frame.texts(".murmure-member-name"), ["Bob"]);
    frame.filter("").await;
    assert_eq!(frame.texts(".murmure-member-name").len(), 2);
    assert_eq!(
        frame.sent.get_untracked().len(),
        sent,
        "the filter sends nothing"
    );

    frame.click(".murmure-members-toggle").await;
    assert_eq!(frame.pane.get_untracked(), MurmurePane::channel(acme.sales));
    assert_eq!(frame.regions().await.panel, 0.0, "the list closes");
}

/// Another member's row writes to them: a click fetches their key for the
/// check of the pin (the Murmure members spec). The user's own row is no
/// button.
#[wasm_bindgen_test]
async fn a_click_on_another_member_fetches_their_key() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (alice, members) = alice_and_bob(&acme);
    let session = UiEvent::Rest {
        id: ambre_bus::RequestId::next(),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::DisplayName::new("Alice Martin").unwrap(),
            email: ambre_core::Email::from("alice@test.local"),
        },
    };
    assert!(frame.absorb(session).await.is_empty());
    assert!(frame.absorb(members).await.is_empty());
    frame.select("sales").await;
    frame.click(".murmure-members-toggle").await;
    assert_eq!(
        frame.texts(".murmure-member-open .murmure-member-name"),
        ["Bob"]
    );

    frame.click(".murmure-member-open").await;
    let sent = frame.sent.get_untracked();
    assert!(
        matches!(
            sent.last(),
            Some(LogicCommand::Rest { cmd: ambre_bus::RestCommand::FetchMemberKey { user_id }, .. })
                if *user_id != alice
        ),
        "{sent:?}"
    );
}

/// A private channel shows an invite control, and a public one none. Its
/// list offers the other members of the space, and its filter sends nothing.
/// A choice fetches the key for the check of the pin, then closes the list.
#[wasm_bindgen_test]
async fn the_invite_control_offers_the_other_members_and_fetches_a_key() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (alice, members) = alice_and_bob(&acme);
    let session = UiEvent::Rest {
        id: ambre_bus::RequestId::next(),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::DisplayName::new("Alice Martin").unwrap(),
            email: ambre_core::Email::from("alice@test.local"),
        },
    };
    assert!(frame.absorb(session).await.is_empty());
    assert!(frame.absorb(members).await.is_empty());
    frame.select("general").await;
    assert!(frame.query(".murmure-invite-toggle").is_none());

    frame.select("Private channel").await;
    assert!(frame.query(".murmure-invite-picker").is_none(), "closed");
    frame.click(".murmure-invite-toggle").await;
    let names = ".murmure-invite-picker .murmure-member-name";
    assert_eq!(frame.texts(names), ["Bob"]);
    let sent = frame.sent.get_untracked().len();
    frame.type_into(".murmure-invite-filter", "zz").await;
    assert!(frame.texts(names).is_empty());
    frame.type_into(".murmure-invite-filter", "").await;
    assert_eq!(
        frame.sent.get_untracked().len(),
        sent,
        "the filter sends nothing"
    );

    frame
        .click(".murmure-invite-picker .murmure-member-open")
        .await;
    let sent = frame.sent.get_untracked();
    let Some(LogicCommand::Rest {
        cmd: ambre_bus::RestCommand::FetchMemberKey { user_id: bob },
        ..
    }) = sent.last().cloned()
    else {
        panic!("no key fetched: {sent:?}");
    };
    assert_ne!(bob, alice);
    assert!(frame.query(".murmure-invite-picker").is_none(), "it closes");
    assert_eq!(frame.relay.take_invite(bob), Some(acme.board));
}

/// Online is a filled dot and offline a hollow ring: the two marks differ
/// in shape as well as in colour (`MURMURE-443`).
#[wasm_bindgen_test]
async fn the_presence_marks_differ_in_shape_as_well_as_in_colour() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    assert!(frame.absorb(alice_and_bob(&acme).1).await.is_empty());
    frame.select("general").await;
    frame.click(".murmure-members-toggle").await;
    let online = ".murmure-presence[data-presence=\"online\"]";
    let offline = ".murmure-presence[data-presence=\"offline\"]";
    assert_eq!(frame.style_of(online, "border-top-width"), "0px");
    assert_ne!(
        frame.style_of(online, "background-color"),
        "rgba(0, 0, 0, 0)",
        "online is filled"
    );
    assert_eq!(frame.style_of(offline, "border-top-width"), "2px");
    assert_eq!(
        frame.style_of(offline, "background-color"),
        "rgba(0, 0, 0, 0)",
        "offline is hollow"
    );
    assert_ne!(
        frame.style_of(online, "background-color"),
        frame.style_of(offline, "border-top-color"),
        "and the colours differ"
    );
}

/// The list opened from a channel marks who watches that channel, this user
/// included, whatever their mark in the space. Another channel's list marks
/// none of them, and the mark leaves once the member stops watching
/// (`MURMURE-519`).
#[wasm_bindgen_test]
async fn the_member_list_marks_who_watches_the_channel() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (alice, bob, carol) = (UserId::new(), UserId::new(), UserId::new());
    let member = |user_id, name: &str| MemberView {
        user_id,
        display_name: ambre_core::DisplayName::new(name).unwrap(),
        presence: Presence::Offline,
    };
    let session = UiEvent::Rest {
        id: ambre_bus::RequestId::next(),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::DisplayName::new("Alice").unwrap(),
            email: ambre_core::Email::from("alice@test.local"),
        },
    };
    assert!(frame.absorb(session).await.is_empty());
    let members = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Members {
            members: vec![
                member(alice, "Alice"),
                member(bob, "Bob"),
                member(carol, "Carol"),
            ],
        },
    };
    assert!(frame.absorb(members).await.is_empty());
    let watchers = ambre_murmure::testing::watchers;
    let marks = |frame: &Frame| -> Vec<Option<String>> {
        let list = frame.inner.query_selector_all(".murmure-presence").unwrap();
        (0..list.length())
            .map(|i| {
                let el: web_sys::Element = list.item(i).unwrap().dyn_into().unwrap();
                el.get_attribute("data-presence")
            })
            .collect()
    };
    let (here, offline) = (Some("here".to_owned()), Some("offline".to_owned()));

    assert!(
        frame
            .absorb(watchers(acme.sales, vec![bob]))
            .await
            .is_empty()
    );
    frame.select("sales").await;
    frame.click(".murmure-members-toggle").await;
    assert_eq!(
        frame.texts(".murmure-member-name"),
        ["Alice", "Bob", "Carol"]
    );
    assert_eq!(marks(&frame), [here.clone(), here.clone(), offline.clone()]);
    let label = frame
        .query(".murmure-presence[data-presence=\"here\"]")
        .and_then(|el| el.get_attribute("aria-label"));
    assert_eq!(label.as_deref(), Some("In this channel"));

    frame.select("general").await;
    frame.click(".murmure-members-toggle").await;
    assert_eq!(
        marks(&frame),
        [here.clone(), offline.clone(), offline.clone()],
        "the list of another channel marks this user alone"
    );

    frame.select("sales").await;
    frame.click(".murmure-members-toggle").await;
    assert_eq!(marks(&frame), [here.clone(), here.clone(), offline.clone()]);
    assert!(
        frame
            .absorb(watchers(acme.sales, Vec::new()))
            .await
            .is_empty()
    );
    assert_eq!(marks(&frame), [here, offline.clone(), offline]);
}

/// A member in the channel is a dot inside a ring, unlike the filled dot of
/// online and the hollow ring of offline (`MURMURE-520`).
#[wasm_bindgen_test]
async fn the_mark_of_the_channel_differs_in_shape_from_the_others() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (_, members) = alice_and_bob(&acme);
    let UiEvent::Space {
        event: SpaceEvent::Members { members: rows },
        ..
    } = &members
    else {
        unreachable!("alice_and_bob lists members")
    };
    let carol = UserId::new();
    let mut rows = rows.clone();
    rows.push(MemberView {
        user_id: carol,
        display_name: ambre_core::DisplayName::new("Carol").unwrap(),
        presence: Presence::Online,
    });
    let members = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Members { members: rows },
    };
    assert!(frame.absorb(members).await.is_empty());
    let watching = ambre_murmure::testing::watchers(acme.general, vec![carol]);
    assert!(frame.absorb(watching).await.is_empty());
    frame.select("general").await;
    frame.click(".murmure-members-toggle").await;
    let here = ".murmure-presence[data-presence=\"here\"]";
    let online = ".murmure-presence[data-presence=\"online\"]";
    let offline = ".murmure-presence[data-presence=\"offline\"]";
    assert_eq!(frame.style_of(here, "border-top-width"), "2px", "a ring");
    assert_ne!(
        frame.style_of(here, "background-color"),
        "rgba(0, 0, 0, 0)",
        "around a dot"
    );
    assert_ne!(frame.style_of(here, "box-shadow"), "none");
    assert_eq!(frame.style_of(online, "border-top-width"), "0px");
    assert_eq!(
        frame.style_of(offline, "background-color"),
        "rgba(0, 0, 0, 0)"
    );
    assert_ne!(
        frame.style_of(here, "background-color"),
        frame.style_of(online, "background-color"),
        "and its colour is its own"
    );
}

/// A private channel and a direct message have members of their own, so
/// their header offers no member list of the space.
#[wasm_bindgen_test]
async fn a_private_channel_or_a_direct_message_offers_no_member_list() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    for name in ["Direct message", "Private channel"] {
        frame.select(name).await;
        assert!(frame.query(".murmure-members-toggle").is_none(), "{name}");
    }
    frame.select("general").await;
    assert!(frame.query(".murmure-members-toggle").is_some());
}

/// Each refresh asks the Logic thread to send this user's presence on every
/// space of the sidebar (`MURMURE-445`). A space lost takes its member list
/// with it.
#[wasm_bindgen_test]
async fn a_refresh_reaches_every_space_and_a_space_lost_drops_its_members() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let now = Timestamp(1_700_000_000_000);
    let refresh = frame.relay.refresh_presence(now);
    assert!(
        matches!(
            refresh[..],
            [LogicCommand::Space {
                space_id,
                command: SpaceCommand::RefreshPresence { now: at },
            }] if space_id == acme.space && at == now
        ),
        "{refresh:?}"
    );
    assert!(frame.absorb(alice_and_bob(&acme).1).await.is_empty());
    frame.select("general").await;
    assert_eq!(frame.relay.shown_members().get_untracked().len(), 2);
    let lost = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Left,
    };
    assert!(frame.absorb(lost).await.is_empty());
    assert!(frame.relay.refresh_presence(now).is_empty());
    assert!(frame.absorb(acme.members(&[])).await.len() == 1);
    assert!(frame.absorb(acme.catalogue()).await.is_empty());
    frame.select("general").await;
    assert!(
        frame.relay.shown_members().get_untracked().is_empty(),
        "the old list is gone"
    );
}

impl Frame {
    /// The field of the search box, once open.
    fn search_field(&self) -> web_sys::HtmlInputElement {
        self.query(".murmure-search-field")
            .expect("the search box is open")
            .dyn_into()
            .unwrap()
    }

    async fn type_search(&self, text: &str) {
        let input = self.search_field();
        input.set_value(text);
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
    }
}

/// The search box opens in the command bar and takes the caret
/// (`MURMURE-213`). Its words ask the Logic thread, and the answer lists
/// each match under its channel and the coverage (`MURMURE-149`). A match
/// opens its channel, and Escape closes the box.
#[wasm_bindgen_test]
async fn the_search_box_asks_lists_and_opens_a_match() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    assert!(frame.query(".murmure-search").is_none(), "closed at first");
    frame.relay.open_search();
    leptos::task::tick().await;
    let field = frame.search_field();
    assert!(
        frame.query(".ambre-commandbar .murmure-search").is_some(),
        "in the command bar"
    );
    assert_eq!(
        frame.inner.active_element(),
        Some(field.clone().into()),
        "the box takes the caret"
    );

    frame.type_search("budget").await;
    let asked = frame.sent.get_untracked();
    assert!(
        matches!(asked[..], [LogicCommand::SearchMessages { ref query }] if query == "budget"),
        "{asked:?}"
    );
    let segment = SegmentId::new();
    let message = MessageView {
        id: MessageId::new(),
        author: UserId::new(),
        ts: Timestamp(1),
        text: "the budget for May".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        notice: None,
        edited: false,
        deleted: false,
        thread: None,
        also_in_thread: None,
    };
    let answer = UiEvent::MessageSearch {
        query: "budget".to_owned(),
        channels: vec![ChannelMatches {
            channel_id: acme.general,
            covered: vec![segment],
            listed: 1,
            matches: vec![SearchMatch { segment, message }],
        }],
    };
    assert!(frame.absorb(answer).await.is_empty());
    assert_eq!(
        frame.texts(".murmure-search-coverage"),
        ["Searched 1 of 1 segment in 1 open channel"]
    );
    assert_eq!(
        frame.texts(".murmure-search-channel"),
        ["general1 of 1 segment"]
    );
    assert_eq!(frame.texts(".murmure-search-match"), ["the budget for May"]);

    frame.click(".murmure-search-match").await;
    assert!(
        frame.query(".murmure-search").is_none(),
        "a match closes it"
    );
    assert_eq!(
        frame.pane.get_untracked().shown_channel(),
        Some(acme.general)
    );
    assert!(matches!(
        frame.sent.get_untracked().last(),
        Some(LogicCommand::OpenChannel { channel_id }) if *channel_id == acme.general
    ));

    frame.relay.open_search();
    leptos::task::tick().await;
    let escape = web_sys::KeyboardEventInit::new();
    escape.set_key("Escape");
    let key = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape);
    frame.search_field().dispatch_event(&key.unwrap()).unwrap();
    leptos::task::tick().await;
    assert!(frame.query(".murmure-search").is_none(), "Escape closes it");
}

/// The list of keys shows its keys on a phone as on a desktop. The chip of
/// a menu hides there, but the member opened this list to read them
/// (`MURMURE-542`).
#[wasm_bindgen_test]
async fn the_list_of_keys_shows_its_keys_at_every_width() {
    for width in [400, 1025] {
        let frame = Frame::new(width, channel()).await;
        assert_eq!(
            frame.width_of(".murmure-keys-key"),
            0.0,
            "closed at {width}"
        );
        frame.relay.keys.set(true);
        leptos::task::tick().await;
        assert!(
            frame.width_of(".murmure-keys-key") > 0.0,
            "shown at {width}"
        );
        assert_eq!(frame.texts(".murmure-keys-key")[0], "Ctrl+K");
    }
}

/// The host tells the canvas it is on a desktop exactly where the stylesheet
/// shows a chord. The box of a thread names Alt+Enter only there (`MURMURE-544`).
#[wasm_bindgen_test]
async fn the_canvas_reads_the_desktop_width_as_the_stylesheet_does() {
    for (width, wide) in [(1024, false), (1025, true)] {
        let frame = Frame::new(width, channel()).await;
        let chip = frame.inner.create_element("kbd").unwrap();
        chip.set_class_name("ambre-kbd");
        chip.set_text_content(Some("Alt+Enter"));
        frame.inner.body().unwrap().append_child(&chip).unwrap();
        let window = frame.frame.content_window().unwrap();
        assert_eq!(crate::host::desktop_in(&window), wide, "at {width}");
        assert_eq!(frame.width_of(".ambre-kbd") > 0.0, wide, "chip at {width}");
    }
}

/// Each style of the composer takes the chord Prose binds to the same mark,
/// so a hand learns one chord per mark (`MURMURE-431`).
#[wasm_bindgen_test]
fn a_style_takes_the_chord_prose_binds_to_its_mark() {
    use ambre_bus::TextStyle;
    use ambre_chrome::commands::KeyBinding;
    use ambre_prose::commands::{BOLD, CODE, COMMANDS, ITALIC};

    for (key, style) in ambre_murmure::markers::MARK_CHORDS {
        let id = match style {
            TextStyle::Bold => BOLD,
            TextStyle::Italic => ITALIC,
            TextStyle::Code => CODE,
            TextStyle::Link(_) | TextStyle::CodeBlock | TextStyle::Quote | TextStyle::List => {
                panic!("{style:?} takes no chord")
            }
        };
        let command = COMMANDS.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            command.keybindings,
            &[KeyBinding::primary(key)],
            "{style:?} takes another chord than Prose"
        );
    }
}

/// The format bar of the composer shows the Prose controls of the Murmure
/// marks alone, in Prose's order. Each takes the title Prose gives it
/// (`MURMURE-432`).
#[wasm_bindgen_test]
fn the_format_bar_shows_the_prose_controls_of_the_murmure_marks() {
    use ambre_bus::TextStyle;
    use ambre_prose::commands::{BOLD, CODE, COMMANDS, FORMAT_BAR, ITALIC};

    let prose: Vec<_> = FORMAT_BAR.iter().map(|(id, _)| *id).collect();
    let mut at = Vec::new();
    for (style, label) in ambre_murmure::markers::MARK_BAR {
        let id = match style {
            TextStyle::Bold => BOLD,
            TextStyle::Italic => ITALIC,
            TextStyle::Code => CODE,
            TextStyle::Link(_) | TextStyle::CodeBlock | TextStyle::Quote | TextStyle::List => {
                panic!("{style:?} has no control")
            }
        };
        let command = COMMANDS.iter().find(|c| c.id == id).unwrap();
        assert_eq!((command.title)(&ambre_chrome::commands::Locale::En), label);
        at.push(prose.iter().position(|shown| *shown == id).unwrap());
    }
    assert!(at.is_sorted(), "the controls keep Prose's order: {at:?}");
}
