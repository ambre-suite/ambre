// ambre-app/src/shell/murmure_regions.rs
// Browser-tier tests of the Murmure shell at a set width (the Murmure layout
// spec). A width query reads the viewport of the frame it runs in. So each
// test mounts the shell into an iframe of the width it reads.

use super::app_root::MurmureShell;
use ambre_bus::{
    CatalogueEntry, CategoryView, ChannelActivity, ChannelName, ChannelPlace, DirectPeers,
    LogicCommand, MemberChannel, MentionCount, SpaceCommand, SpaceEvent, UiEvent, UnreadCount,
};
use ambre_core::{CategoryId, ChannelId, MessageId, SegmentId, SpaceId};
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
                .update(|s| s.set_name(channel, Some("general".to_owned())));
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
        let sheet = inner.create_element("style").unwrap();
        sheet.set_text_content(Some(STYLES));
        inner.head().unwrap().append_child(&sheet).unwrap();
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
            name: name.to_owned(),
            topic: None,
            category,
            position,
        };
        UiEvent::Space {
            space_id: self.space,
            event: SpaceEvent::Catalogue {
                name: Some("Acme".to_owned()),
                categories: vec![CategoryView {
                    id: team,
                    name: "Team".to_owned(),
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
            name: name.to_owned(),
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
            names: vec!["Bob".into(), "Carol".into()],
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
