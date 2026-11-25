// ambre-app/src/shell/murmure_regions.rs
// Browser-tier tests of the Murmure shell at a set width (the Murmure layout
// spec). A width query reads the viewport of the frame it runs in. So each
// test mounts the shell into an iframe of the width it reads.

use super::app_root::MurmureShell;
use ambre_bus::{
    Authored, ChannelActivity, ChannelName, ChannelPlace, DirectPeers, Instant, MemberChannel,
    Mentions, Unread,
};
use ambre_core::id::{Category, Channel, Id, Message, Segment, Space, User};
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::message::{
    CatalogueEntry, CategoryView, ChannelMatches, MemberView, MessageView, Presence, SearchMatch,
    SpaceCommand, SpaceEvent,
};
use ambre_murmure::message::{LogicCommand, UiEvent};
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
            relay.sidebar.update(|s| {
                s.set_name(
                    channel,
                    Some(ambre_core::Validated::<ambre_core::Name>::parse("general").unwrap()),
                )
            });
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
        // No invite was made, so the toolbar shows no link.
        let invite_link = RwSignal::new(None);
        let handle = mount_to(inner.body().unwrap(), move || {
            view! {
                <MurmureShell relay=relay send=send invite_link=invite_link>
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
    space: Id<Space>,
    general: Id<Channel>,
    sales: Id<Channel>,
    random: Id<Channel>,
    board: Id<Channel>,
    direct: Id<Channel>,
}

impl Acme {
    fn new() -> Self {
        Self {
            space: Id::<Space>::mint(),
            general: Id::<Channel>::mint(),
            sales: Id::<Channel>::mint(),
            random: Id::<Channel>::mint(),
            board: Id::<Channel>::mint(),
            direct: Id::<Channel>::mint(),
        }
    }

    fn members(&self, also: &[Id<Channel>]) -> UiEvent {
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
        let team = Id::<Category>::mint();
        let entry = |channel_id, name: &str, category, position: u32| CatalogueEntry {
            channel_id,
            name: ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
            topic: None,
            category,
            position: ambre_bus::Rank::new(position),
        };
        UiEvent::Space {
            space_id: self.space,
            event: SpaceEvent::Catalogue {
                name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Acme").unwrap()),
                role: ambre_bus::Role::Viewer,
                categories: vec![CategoryView {
                    id: team,
                    name: ambre_core::Validated::<ambre_core::Name>::parse("Team").unwrap(),
                    position: ambre_bus::Rank::FIRST,
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

fn activity(
    channel_id: Id<Channel>,
    active: ambre_bus::Activity,
    counts: Option<(u32, u32)>,
) -> UiEvent {
    UiEvent::ChannelActivity {
        channels: vec![ChannelActivity {
            channel_id,
            active,
            unread: counts.map(|(m, _)| ambre_core::Count::<Unread>::new(m)),
            mentions: counts.map(|(_, n)| ambre_core::Count::<Mentions>::new(n)),
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
    MurmurePane::channel(Id::<Channel>::mint())
}

fn thread() -> MurmurePane {
    MurmurePane::Thread {
        channel: Id::<Channel>::mint(),
        root: MessageRef {
            segment: Id::<Segment>::mint(),
            message: Id::<Message>::mint(),
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

    let channel = Id::<Channel>::mint();
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
        panic!("a thread pane")
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

/// The call controls of the header (`MURMURE-603`, `MURMURE-605`, `MURMURE-602`). A shown
/// channel offers the call button. A call offers mute and hang up in its place. Each is a
/// button with an accessible name that takes the focus, and mute carries its pressed state. A refusal
/// of the microphone reads as an alert beside the call button.
#[wasm_bindgen_test]
async fn the_call_controls_name_themselves_and_take_the_focus() {
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone, MicrophoneRefusal};
    let frame = Frame::new(1025, channel()).await;
    let phase = crate::call::phase();
    let named = |selector: &str| {
        let button: web_sys::HtmlElement = frame
            .query(&format!(".murmure-channel-header {selector}"))
            .unwrap_or_else(|| panic!("{selector} is in the header"))
            .dyn_into()
            .unwrap();
        assert_eq!(button.tag_name(), "BUTTON", "{selector}");
        button.focus().unwrap();
        assert_eq!(
            frame.inner.active_element().as_ref(),
            Some(button.unchecked_ref::<web_sys::Element>()),
            "{selector} takes the focus"
        );
        button.get_attribute("aria-label").unwrap_or_default()
    };
    assert_eq!(named(".murmure-call-start"), "Start a call");
    assert!(frame.query(".murmure-call-mute").is_none());

    phase.set(CallPhase::InCall(Microphone::Live));
    leptos::task::tick().await;
    assert!(
        frame.query(".murmure-call-start").is_none(),
        "no call button in a call"
    );
    assert_eq!(named(".murmure-call-mute"), "Mute");
    assert_eq!(named(".murmure-call-hang-up"), "Hang up");
    let pressed = || {
        frame
            .query(".murmure-call-mute")
            .and_then(|b| b.get_attribute("aria-pressed"))
    };
    assert_eq!(pressed().as_deref(), Some("false"));
    // A press of mute with no track still flips the state the header reads.
    frame.click(".murmure-call-mute").await;
    assert_eq!(pressed().as_deref(), Some("true"));
    frame.click(".murmure-call-mute").await;
    assert_eq!(pressed().as_deref(), Some("false"));

    phase.set(CallPhase::Refused(MicrophoneRefusal::Blocked));
    leptos::task::tick().await;
    let refusal = frame
        .query(".murmure-channel-header .murmure-call-refusal")
        .expect("the refusal shows");
    assert_eq!(refusal.get_attribute("role").as_deref(), Some("alert"));
    assert_eq!(
        refusal.text_content().as_deref(),
        Some(MicrophoneRefusal::Blocked.notice())
    );
    assert_eq!(named(".murmure-call-start"), "Start a call");
    phase.set(CallPhase::Idle);
}

/// The ring alert of a call (`MURMURE-641`, `MURMURE-646`). A ring cue of a public channel
/// rings nothing. A ring cue of the direct message shows the alert. Its title reads as an
/// alert. Accept and decline are buttons with an accessible name, and each takes the focus. A
/// press of decline ends the ring, and the alert goes.
#[wasm_bindgen_test]
async fn the_ring_alert_names_the_caller_and_offers_accept_and_decline() {
    use ambre_murmure::ui::call_ring::Ring;
    let acme = Acme::new();
    let frame = acme.frame().await;
    let (to_logic, logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
        ambre_bus::conduit::ChannelSlots,
    >::of::<4>());
    crate::call_ring::install(crate::outbox::ShellOutbox::new(to_logic), frame.relay);
    let ring = crate::call_ring::ring();
    let cue = ambre_bus::CallCue {
        call: Id::<ambre_core::id::Call>::mint(),
        kind: ambre_bus::CueKind::Ring,
        at: Instant::<Authored>::new(js_sys::Date::now() as u64),
    };
    crate::call_ring::heard(&frame.relay, acme.general, cue);
    leptos::task::tick().await;
    assert!(
        frame.query(".murmure-ring").is_none(),
        "a channel rings nothing"
    );
    crate::call_ring::heard(&frame.relay, acme.direct, cue);
    leptos::task::tick().await;
    let title = frame
        .query(".murmure-ring .murmure-ring-title")
        .expect("the alert shows");
    assert_eq!(title.get_attribute("role").as_deref(), Some("alert"));
    assert_eq!(title.text_content().as_deref(), Some("Incoming call"));
    for (selector, name) in [
        (".murmure-ring-accept", "Accept the call"),
        (".murmure-ring-decline", "Decline the call"),
    ] {
        let button: web_sys::HtmlElement = frame
            .query(&format!(".murmure-ring {selector}"))
            .unwrap_or_else(|| panic!("{selector} is in the alert"))
            .dyn_into()
            .unwrap();
        assert_eq!(button.tag_name(), "BUTTON", "{selector}");
        assert_eq!(button.get_attribute("aria-label").as_deref(), Some(name));
        button.focus().unwrap();
        assert_eq!(
            frame.inner.active_element().as_ref(),
            Some(button.unchecked_ref::<web_sys::Element>()),
            "{selector} takes the focus"
        );
    }
    frame.click(".murmure-ring-decline").await;
    assert_eq!(ring.get_untracked(), Ring::Quiet, "decline ends the ring");
    assert!(frame.query(".murmure-ring").is_none(), "the alert goes");
    let declined = logic.try_iter().any(|command| {
        matches!(
            command,
            ambre_bus::LogicCommand::Channel {
                channel_id,
                command: ambre_murmure::message::MurmureCommand::DeclineCall { call, .. },
            } if channel_id == acme.direct && call == cue.call
        )
    });
    assert!(
        declined,
        "decline writes the decline of the call into its direct message"
    );
}

/// The notice of a ring that ended with no answer (`MURMURE-645`). It is a status beside the
/// call button, and it names a decline apart from no answer.
#[wasm_bindgen_test]
async fn the_header_says_why_a_call_rang_with_no_answer() {
    use ambre_murmure::ui::call_ring::Unanswered;
    let frame = Frame::new(1025, channel()).await;
    let unanswered = crate::call_ring::unanswered();
    assert!(frame.query(".murmure-call-unanswered").is_none());
    for why in [Unanswered::Declined, Unanswered::NoAnswer] {
        unanswered.set(Some(why));
        leptos::task::tick().await;
        let notice = frame
            .query(".murmure-channel-header .murmure-call-unanswered")
            .expect("the notice shows");
        assert_eq!(notice.get_attribute("role").as_deref(), Some("status"));
        assert_eq!(notice.text_content().as_deref(), Some(why.notice()));
        assert!(
            frame.query(".murmure-call-start").is_some(),
            "the call button stays"
        );
    }
    crate::call_ring::clear_notice();
    leptos::task::tick().await;
    assert!(frame.query(".murmure-call-unanswered").is_none());
}

/// The camera button of a call (`MURMURE-624`, `MURMURE-623`). A call offers it beside mute. It
/// is a button with an accessible name that takes the focus, and carries its pressed state. A
/// refused camera reads as an alert, and the call keeps mute and hang up.
#[wasm_bindgen_test]
async fn the_camera_button_names_itself_and_carries_its_pressed_state() {
    use ambre_murmure::ui::call_controls::{CallPhase, Camera, CameraRefusal, Microphone};
    let frame = Frame::new(1025, channel()).await;
    let (phase, camera) = (crate::call::phase(), crate::call::camera());
    assert!(
        frame.query(".murmure-call-camera").is_none(),
        "no camera button out of a call"
    );
    phase.set(CallPhase::InCall(Microphone::Live));
    leptos::task::tick().await;
    let button: web_sys::HtmlElement = frame
        .query(".murmure-channel-header .murmure-call-camera")
        .expect("the camera button is in the header")
        .dyn_into()
        .unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>()),
        "the camera button takes the focus"
    );
    let read = |attribute: &str| button.get_attribute(attribute).unwrap_or_default();
    assert_eq!(read("aria-label"), "Turn the camera on");
    assert_eq!(read("aria-pressed"), "false");
    camera.set(Camera::On);
    leptos::task::tick().await;
    assert_eq!(read("aria-label"), "Turn the camera off");
    assert_eq!(read("aria-pressed"), "true");

    camera.set(Camera::Refused(CameraRefusal::Blocked));
    leptos::task::tick().await;
    let refusal = frame
        .query(".murmure-channel-header .murmure-call-camera-refusal")
        .expect("the refusal of the camera shows");
    assert_eq!(refusal.get_attribute("role").as_deref(), Some("alert"));
    assert_eq!(
        refusal.text_content().as_deref(),
        Some(CameraRefusal::Blocked.notice())
    );
    assert_eq!(read("aria-pressed"), "false");
    assert!(
        frame.query(".murmure-call-mute").is_some(),
        "the call goes on"
    );
    camera.set(Camera::Off);
    phase.set(CallPhase::Idle);
}

/// The participants button of a call (`MURMURE-668`, `MURMURE-669`). A call offers
/// it in the header. It is a button with an accessible name that takes the focus, and reports
/// whether its list is open. The list names the member and each other participant. Each other
/// participant carries a pin button with an accessible name and its pressed state, which pins
/// it and unpins it.
#[wasm_bindgen_test]
async fn the_participants_button_names_itself_and_lists_each_participant() {
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone};
    let frame = Frame::new(1025, channel()).await;
    let phase = crate::call::phase();
    let (participants, roster) = (
        crate::call_volumes::participants(),
        crate::call_stage::roster(),
    );
    assert!(
        frame.query(".murmure-call-roster").is_none(),
        "no participants button out of a call"
    );
    phase.set(CallPhase::InCall(Microphone::Live));
    participants.set(vec![Id::<User>::mint(), Id::<User>::mint()]);
    leptos::task::tick().await;
    let button: web_sys::HtmlElement = frame
        .query(".murmure-channel-header .murmure-call-roster")
        .expect("the participants button is in the header")
        .dyn_into()
        .unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>()),
        "the participants button takes the focus"
    );
    let read = |attribute: &str| button.get_attribute(attribute).unwrap_or_default();
    assert_eq!(read("aria-label"), "Participants");
    assert_eq!(read("aria-expanded"), "false");
    assert!(frame.query(".murmure-call-roster-list").is_none());
    button.click();
    leptos::task::tick().await;
    assert_eq!(read("aria-expanded"), "true");
    assert_eq!(
        frame.texts(".murmure-call-roster-list .murmure-call-roster-name"),
        ["You", "A participant", "A participant"]
    );
    button.click();
    leptos::task::tick().await;
    assert!(frame.query(".murmure-call-roster-list").is_none());
    // Each other participant carries a pin button, which pins it and a second press unpins.
    button.click();
    leptos::task::tick().await;
    let pins = frame
        .inner
        .query_selector_all(".murmure-call-roster-list .murmure-call-pin")
        .unwrap();
    assert_eq!(pins.length(), 2, "one pin button per other participant");
    let pin: web_sys::HtmlElement = pins.get(0).unwrap().dyn_into().unwrap();
    assert_eq!(pin.tag_name(), "BUTTON");
    pin.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(pin.unchecked_ref::<web_sys::Element>()),
        "a pin button takes the focus"
    );
    let pin_read = |attribute: &str| pin.get_attribute(attribute).unwrap_or_default();
    assert_eq!(pin_read("aria-label"), "Pin A participant");
    assert_eq!(pin_read("aria-pressed"), "false");
    let first = participants.get_untracked()[0];
    pin.click();
    leptos::task::tick().await;
    assert_eq!(crate::call_stage::pinned().get_untracked(), Some(first));
    let pin: web_sys::HtmlElement = frame
        .query(".murmure-call-roster-list .murmure-call-pin")
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(
        pin.get_attribute("aria-label").unwrap_or_default(),
        "Unpin A participant"
    );
    assert_eq!(
        pin.get_attribute("aria-pressed").unwrap_or_default(),
        "true"
    );
    pin.click();
    leptos::task::tick().await;
    assert_eq!(crate::call_stage::pinned().get_untracked(), None);
    roster.set(ambre_chrome::Openness::Closed);
    participants.set(Vec::new());
    phase.set(CallPhase::Idle);
}

/// The page buttons of a call (`MURMURE-669`, `MURMURE-654`). A band of one page offers none.
/// Past five other participants, the header offers the two, each a button with an accessible
/// name that takes the focus. Each is disabled at the end it turns to, and a press turns the
/// page.
#[wasm_bindgen_test]
async fn the_page_buttons_name_themselves_and_turn_the_page() {
    use ambre_core::index::Pos;
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone};
    use ambre_murmure::ui::call_videos::GridPages;
    let frame = Frame::new(1025, channel()).await;
    let (phase, participants) = (crate::call::phase(), crate::call_volumes::participants());
    let page = crate::call_stage::page();
    phase.set(CallPhase::InCall(Microphone::Live));
    participants.set((0..5).map(|_| Id::<User>::mint()).collect());
    leptos::task::tick().await;
    assert!(
        frame.query(".murmure-call-page-next").is_none(),
        "five participants take one page of the band"
    );
    participants.set((0..12).map(|_| Id::<User>::mint()).collect());
    leptos::task::tick().await;
    let named = |selector: &str| -> web_sys::HtmlButtonElement {
        frame
            .query(&format!(".murmure-channel-header {selector}"))
            .expect("a page button in the header")
            .dyn_into()
            .unwrap()
    };
    let (previous, next) = (
        named(".murmure-call-page-previous"),
        named(".murmure-call-page-next"),
    );
    assert_eq!(
        next.get_attribute("aria-label").unwrap_or_default(),
        "Next page of the call"
    );
    assert_eq!(
        previous.get_attribute("aria-label").unwrap_or_default(),
        "Previous page of the call"
    );
    assert!(previous.disabled(), "no page before the first");
    next.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(next.unchecked_ref::<web_sys::Element>()),
        "a page button takes the focus"
    );
    next.click();
    leptos::task::tick().await;
    assert_eq!(page.get_untracked(), Pos::<GridPages>::new(1));
    assert!(!previous.disabled());
    assert_eq!(frame.texts(".murmure-call-page-number"), ["2 / 3"]);
    next.click();
    leptos::task::tick().await;
    assert!(next.disabled(), "no page after the last");
    previous.click();
    previous.click();
    leptos::task::tick().await;
    assert_eq!(page.get_untracked(), Pos::<GridPages>::new(0));
    participants.set(Vec::new());
    phase.set(CallPhase::Idle);
}

/// The fill button of a call (`MURMURE-659`, `MURMURE-657`). A call offers it in the header. It
/// is a button with an accessible name that takes the focus and carries its pressed state, and
/// the keyboard presses it. A press widens the call, and a second brings back the band.
#[wasm_bindgen_test]
async fn the_fill_button_names_itself_and_widens_the_call() {
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone};
    use ambre_murmure::ui::call_videos::Fill;
    let frame = Frame::new(1025, channel()).await;
    let (phase, fill) = (crate::call::phase(), crate::call_stage::fill());
    assert!(
        frame.query(".murmure-call-fill").is_none(),
        "no fill button out of a call"
    );
    phase.set(CallPhase::InCall(Microphone::Live));
    leptos::task::tick().await;
    let button: web_sys::HtmlElement = frame
        .query(".murmure-channel-header .murmure-call-fill")
        .expect("the fill button is in the header")
        .dyn_into()
        .unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>()),
        "the fill button takes the focus"
    );
    let read = |attribute: &str| button.get_attribute(attribute).unwrap_or_default();
    assert_eq!(read("aria-label"), "Fill the channel region with the call");
    assert_eq!(read("aria-pressed"), "false");
    // A native button: Enter and Space press it, as a click does.
    button.click();
    leptos::task::tick().await;
    assert_eq!(fill.get_untracked(), Fill::Region);
    assert_eq!(read("aria-label"), "Shrink the call to a band");
    assert_eq!(read("aria-pressed"), "true");
    button.click();
    leptos::task::tick().await;
    assert_eq!(fill.get_untracked(), Fill::Band);
    assert_eq!(read("aria-pressed"), "false");
    phase.set(CallPhase::Idle);
}

/// The share button of the header (`MURMURE-633`, `MURMURE-634`, `MURMURE-630`). It is a button
/// with an accessible name that takes the focus and carries its pressed state. A refusal is an
/// alert that says why, and a take-over is a status that says another member shares. Mute and
/// hang up stay through both.
#[wasm_bindgen_test]
async fn the_share_button_names_itself_and_carries_its_pressed_state() {
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone, Share, ShareRefusal};
    let frame = Frame::new(1025, channel()).await;
    let (phase, share) = (crate::call::phase(), crate::call::share());
    assert!(
        frame.query(".murmure-call-share").is_none(),
        "no share button out of a call"
    );
    phase.set(CallPhase::InCall(Microphone::Live));
    leptos::task::tick().await;
    let button: web_sys::HtmlElement = frame
        .query(".murmure-channel-header .murmure-call-share")
        .expect("the share button is in the header")
        .dyn_into()
        .unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>()),
        "the share button takes the focus"
    );
    let read = |attribute: &str| button.get_attribute(attribute).unwrap_or_default();
    assert_eq!(read("aria-label"), "Share the screen");
    assert_eq!(read("aria-pressed"), "false");
    share.set(Share::On);
    leptos::task::tick().await;
    assert_eq!(read("aria-label"), "Stop sharing the screen");
    assert_eq!(read("aria-pressed"), "true");
    assert!(frame.query(".murmure-call-share-notice").is_none());

    share.set(Share::Refused(ShareRefusal::Refused));
    leptos::task::tick().await;
    let notice = frame
        .query(".murmure-channel-header .murmure-call-share-notice")
        .expect("the refusal of the screen shows");
    assert_eq!(notice.get_attribute("role").as_deref(), Some("alert"));
    assert_eq!(
        notice.text_content().as_deref(),
        Some(ShareRefusal::Refused.notice())
    );
    assert_eq!(read("aria-pressed"), "false");

    share.set(Share::TakenOver);
    leptos::task::tick().await;
    let notice = frame
        .query(".murmure-channel-header .murmure-call-share-notice")
        .expect("the take-over shows");
    assert_eq!(notice.get_attribute("role").as_deref(), Some("status"));
    assert_eq!(
        notice.text_content().as_deref(),
        Some("Another member shares the screen now")
    );
    assert!(
        frame.query(".murmure-call-mute").is_some()
            && frame.query(".murmure-call-hang-up").is_some(),
        "the call goes on"
    );
    share.set(Share::Off);
    phase.set(CallPhase::Idle);
}

/// The device panel of the header (`MURMURE-611`). The devices control is a button with an
/// accessible name that takes the focus, and reads whether the panel is open. Each choice of
/// the panel is a select that its visible label names, and it takes the focus.
#[wasm_bindgen_test]
async fn the_device_controls_name_themselves_and_take_the_focus() {
    let frame = Frame::new(1025, channel()).await;
    let button: web_sys::HtmlElement = frame
        .query(".murmure-channel-header .murmure-call-devices")
        .expect("the devices control is in the header")
        .dyn_into()
        .unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    assert_eq!(
        button.get_attribute("aria-label").as_deref(),
        Some("Call devices")
    );
    assert_eq!(
        button.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>())
    );
    frame.click(".murmure-call-devices").await;
    assert_eq!(
        button.get_attribute("aria-expanded").as_deref(),
        Some("true")
    );
    let select: web_sys::HtmlElement = frame
        .query(".murmure-call-panel .murmure-call-microphone")
        .expect("the choice of microphone shows")
        .dyn_into()
        .unwrap();
    assert_eq!(select.tag_name(), "SELECT");
    // The label that wraps the select names it.
    let named = select
        .closest("label")
        .unwrap()
        .and_then(|label| label.text_content())
        .unwrap_or_default();
    assert!(named.starts_with("Microphone"), "{named}");
    select.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(select.unchecked_ref::<web_sys::Element>())
    );
    frame.click(".murmure-call-devices").await;
    assert!(frame.query(".murmure-call-panel").is_none());
}

/// The channel each command opens, `None` for any other command.
fn opened(commands: &[LogicCommand]) -> Vec<Option<Id<Channel>>> {
    commands
        .iter()
        .map(|c| match c {
            LogicCommand::OpenChannel { channel_id } => Some(*channel_id),
            _ => None,
        })
        .collect()
}

/// The sidebar draws the joined public channels of the space by category,
/// then the private channels and the direct messages. The
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
            name: ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
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
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
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

    frame
        .absorb(activity(acme.sales, ambre_bus::Activity::Active, None))
        .await;
    assert_eq!(badges(&frame), (true, None, None), "the mark, no count yet");

    frame
        .absorb(activity(
            acme.sales,
            ambre_bus::Activity::Active,
            Some((3, 1)),
        ))
        .await;
    assert_eq!(
        badges(&frame),
        (false, Some("3".to_owned()), Some("1".to_owned()))
    );

    frame
        .absorb(activity(acme.general, ambre_bus::Activity::Active, None))
        .await;
    assert_eq!(
        badges(&frame),
        (false, Some("3".to_owned()), Some("1".to_owned())),
        "a channel absent from an answer keeps its counts"
    );

    frame
        .absorb(activity(
            acme.sales,
            ambre_bus::Activity::Quiet,
            Some((0, 0)),
        ))
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
fn alice_and_bob(acme: &Acme) -> (Id<User>, UiEvent) {
    let alice = Id::<User>::mint();
    let member = |user_id, name: &str, presence| MemberView {
        user_id,
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
        presence,
    };
    let event = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Members {
            members: vec![
                member(alice, "Alice Martin", Presence::Online),
                member(Id::<User>::mint(), "Bob", Presence::Offline),
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
        id: Some(ambre_bus::next_request()),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice Martin")
                .unwrap(),
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
        id: Some(ambre_bus::next_request()),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice Martin")
                .unwrap(),
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
    let (alice, bob, carol) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let member = |user_id, name: &str| MemberView {
        user_id,
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
        presence: Presence::Offline,
    };
    let session = UiEvent::Rest {
        id: Some(ambre_bus::next_request()),
        evt: ambre_bus::RestEvent::SessionReady {
            user_id: alice,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
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
    let watchers = ambre_murmure::ui::testing::watchers;
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
        panic!("alice_and_bob lists members")
    };
    let carol = Id::<User>::mint();
    let mut rows = rows.clone();
    rows.push(MemberView {
        user_id: carol,
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
        presence: Presence::Online,
    });
    let members = UiEvent::Space {
        space_id: acme.space,
        event: SpaceEvent::Members { members: rows },
    };
    assert!(frame.absorb(members).await.is_empty());
    let watching = ambre_murmure::ui::testing::watchers(acme.general, vec![carol]);
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
    let now = Instant::<Authored>::new(1_700_000_000_000);
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
        matches!(asked[..], [LogicCommand::SearchMessages { ref query }] if query.as_str() == "budget"),
        "{asked:?}"
    );
    let segment = Id::<Segment>::mint();
    let message = MessageView {
        id: Id::<Message>::mint(),
        author: Id::<User>::mint(),
        ts: Instant::<Authored>::new(1),
        text: "the budget for May".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        notice: None,
        edited: ambre_murmure::message::Revision::Original,
        deleted: ambre_murmure::message::Deletion::Live,
        thread: None,
        also_in_thread: None,
        call: None,
    };
    let answer = UiEvent::MessageSearch {
        query: ambre_core::Validated::<ambre_core::SearchQuery>::parse("budget".to_owned())
            .expect("a word"),
        channels: vec![ChannelMatches {
            channel_id: acme.general,
            covered: vec![segment],
            listed: ambre_core::Count::<ambre_murmure::message::Segments>::new(1),
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
        frame.relay.keys.set(ambre_chrome::Openness::Open);
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
    use ambre_chrome::commands::KeyBinding;
    use ambre_murmure::message::TextStyle;
    use ambre_prose::commands::{BOLD, CODE, COMMANDS, ITALIC};

    for (key, style) in ambre_murmure::ui::markers::MARK_CHORDS {
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
    use ambre_murmure::message::TextStyle;
    use ambre_prose::commands::{BOLD, CODE, COMMANDS, FORMAT_BAR, ITALIC};

    let prose: Vec<_> = FORMAT_BAR.iter().map(|(id, _)| *id).collect();
    let mut at = Vec::new();
    for (style, label) in ambre_murmure::ui::markers::MARK_BAR {
        let id = match style {
            TextStyle::Bold => BOLD,
            TextStyle::Italic => ITALIC,
            TextStyle::Code => CODE,
            TextStyle::Link(_) | TextStyle::CodeBlock | TextStyle::Quote | TextStyle::List => {
                panic!("{style:?} has no control")
            }
        };
        let command = COMMANDS.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            (command.title)(&ambre_chrome::commands::Locale::En).as_str(),
            label
        );
        at.push(prose.iter().position(|shown| *shown == id).unwrap());
    }
    assert!(at.is_sorted(), "the controls keep Prose's order: {at:?}");
}

/// The attribute `name` of the element `selector` names.
fn attribute(frame: &Frame, selector: &str, name: &str) -> Option<String> {
    frame.query(selector).and_then(|el| el.get_attribute(name))
}

/// A press on the head of a group folds it, and a second unfolds it (`MURMURE-720`). The head
/// is a button that takes the focus and carries its expanded state (`MURMURE-725`). Folded, the
/// group keeps the channel shown and counts what it hides (`MURMURE-721`, `MURMURE-722`). A fold
/// sends nothing to the Logic thread (`MURMURE-724`).
#[wasm_bindgen_test]
async fn a_folded_group_keeps_the_channel_shown_and_counts_the_rest() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    frame.relay.pane.set(MurmurePane::channel(acme.board));
    frame
        .absorb(activity(acme.direct, ambre_bus::Activity::Active, None))
        .await;
    let head = ".murmure-section[data-group=\"private\"] .murmure-section-title";
    let button: web_sys::HtmlElement = frame.query(head).unwrap().dyn_into().unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.focus().unwrap();
    assert_eq!(
        frame.inner.active_element().as_ref(),
        Some(button.unchecked_ref::<web_sys::Element>())
    );
    assert_eq!(
        attribute(&frame, head, "aria-expanded").as_deref(),
        Some("true")
    );
    let before = frame.sent.get_untracked().len();
    frame.click(head).await;
    assert_eq!(
        attribute(&frame, head, "aria-expanded").as_deref(),
        Some("false")
    );
    assert_eq!(
        frame.texts(".murmure-section[data-group=\"private\"] .murmure-entry-name"),
        ["Private channel"],
        "the channel shown stays"
    );
    let direct = ".murmure-section[data-group=\"direct\"] .murmure-section-title";
    frame.click(direct).await;
    assert!(
        frame
            .texts(".murmure-section[data-group=\"direct\"] .murmure-entry-name")
            .is_empty(),
        "a quiet direct message folds away"
    );
    assert!(
        frame
            .query(".murmure-section[data-group=\"direct\"] .murmure-section-hidden .murmure-mark")
            .is_some(),
        "the head marks the hidden activity"
    );
    assert_eq!(
        frame.texts(".murmure-section[data-group=\"direct\"] .murmure-section-hidden"),
        ["1"]
    );
    frame.click(direct).await;
    assert_eq!(
        frame.texts(".murmure-section[data-group=\"direct\"] .murmure-entry-name"),
        ["Direct message"]
    );
    assert_eq!(
        frame.sent.get_untracked().len(),
        before,
        "a fold sends nothing"
    );
}

/// The top of the sidebar holds the space switcher, which names the space shown (`MURMURE-711`).
/// It is a button that carries its expanded state (`MURMURE-714`). A choice shows that space
/// and sends nothing (`MURMURE-712`, `MURMURE-713`).
#[wasm_bindgen_test]
async fn the_space_switcher_names_the_space_and_lists_each_one() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let switch = ".murmure-space-switch .ambre-select-trigger";
    assert!(
        frame.texts(switch).concat().contains("Acme"),
        "{:?}",
        frame.texts(switch)
    );
    assert_eq!(
        attribute(&frame, switch, "aria-expanded").as_deref(),
        Some("false")
    );
    frame.click(switch).await;
    assert_eq!(
        attribute(&frame, switch, "aria-expanded").as_deref(),
        Some("true")
    );
    assert_eq!(
        frame.texts(".murmure-space-switch .ambre-select-option > span:first-of-type"),
        ["Acme"]
    );
    // The chord shows as the command table states it, hidden from a screen reader.
    assert_eq!(
        frame.texts(".murmure-space-switch .ambre-select-hint"),
        ["Alt+Shift+1"]
    );
    assert_eq!(
        attribute(
            &frame,
            ".murmure-space-switch .ambre-select-hint",
            "aria-hidden"
        )
        .as_deref(),
        Some("true")
    );
    let before = frame.sent.get_untracked().len();
    frame
        .click(".murmure-space-switch .ambre-select-option")
        .await;
    assert_eq!(frame.relay.space.get_untracked(), Some(acme.space));
    assert_eq!(
        attribute(&frame, switch, "aria-expanded").as_deref(),
        Some("false"),
        "the list closes"
    );
    assert_eq!(
        frame.sent.get_untracked().len(),
        before,
        "a switch sends nothing"
    );
}

/// Under the phone width, the channel list carries a bar of tabs on its bottom edge
/// (`MURMURE-715`). Each tab is a button with its name and its selected state (`MURMURE-719`). The tab
/// of direct messages shows them alone and sends nothing (`MURMURE-717`, `MURMURE-718`). A
/// channel shows no bar (`MURMURE-716`), and a desktop none either.
#[wasm_bindgen_test]
async fn the_bar_of_tabs_shows_on_the_phone_list_alone() {
    let acme = Acme::new();
    let frame = Frame::with(390, MurmureRelay::new()).await;
    frame.absorb(acme.members(&[])).await;
    frame.absorb(acme.catalogue()).await;
    leptos::task::tick().await;
    assert!(
        frame.width_of(".murmure-tabs") > 0.0,
        "the list carries the bar"
    );
    assert_eq!(
        frame.texts(".murmure-tabs [role=\"radio\"]"),
        ["Channels", "Direct messages", "Search"]
    );
    let tab = ".murmure-tabs [aria-label=\"Direct messages\"]";
    assert_eq!(
        attribute(&frame, tab, "aria-checked").as_deref(),
        Some("false")
    );
    let before = frame.sent.get_untracked().len();
    frame.click(tab).await;
    assert_eq!(
        attribute(&frame, tab, "aria-checked").as_deref(),
        Some("true")
    );
    assert_eq!(frame.texts(".murmure-entry-name"), ["Direct message"]);
    assert_eq!(
        frame.sent.get_untracked().len(),
        before,
        "a tab sends nothing"
    );
    frame.relay.pane.set(MurmurePane::channel(acme.general));
    leptos::task::tick().await;
    assert_eq!(
        frame.width_of(".murmure-tabs"),
        0.0,
        "a channel keeps its height"
    );
    let wide = acme.frame().await;
    assert_eq!(
        wide.width_of(".murmure-tabs"),
        0.0,
        "a desktop shows no bar"
    );
}

/// A sidebar entry whose channel holds a live call shows a call mark and the count of its
/// participants, and no name (`MURMURE-689`). The entry of the channel shown adds their initials
/// (`MURMURE-690`). An answer of no state takes the mark away.
#[wasm_bindgen_test]
async fn a_live_call_shows_its_mark_its_count_and_its_faces_on_the_channel_shown() {
    let acme = Acme::new();
    let frame = acme.frame().await;
    let call = Id::<ambre_core::id::Call>::mint();
    let seated = vec![Id::<User>::mint(), Id::<User>::mint()];
    let answer = |members| UiEvent::Channel {
        channel_id: acme.sales,
        event: ambre_murmure::message::MurmureEvent::CallMembers { call, members },
    };
    frame
        .absorb(answer(ambre_bus::CallMembers::Live(seated)))
        .await;
    let sales = frame.entry("sales");
    let mark = sales
        .query_selector(".murmure-call-live")
        .unwrap()
        .expect("the call mark");
    assert_eq!(
        mark.get_attribute("aria-label").as_deref(),
        Some("Call in progress, 2 in it")
    );
    assert_eq!(
        mark.query_selector(".murmure-call-live-count")
            .unwrap()
            .and_then(|c| c.text_content())
            .as_deref(),
        Some("2")
    );
    assert_eq!(
        mark.query_selector_all(".murmure-call-face")
            .unwrap()
            .length(),
        0
    );
    frame.select("sales").await;
    let mark = frame
        .entry("sales")
        .query_selector(".murmure-call-live")
        .unwrap()
        .expect("the call mark");
    assert_eq!(
        mark.query_selector_all(".murmure-call-face")
            .unwrap()
            .length(),
        2
    );
    frame.absorb(answer(ambre_bus::CallMembers::Gone)).await;
    assert!(
        frame
            .entry("sales")
            .query_selector(".murmure-call-live")
            .unwrap()
            .is_none(),
        "a call the relay holds no state of leaves"
    );
}

/// While the page is in a call and shows another channel, it shows the call away from its
/// channel (`MURMURE-696`). Each control names itself, and mute carries its pressed state
/// (`MURMURE-693`). The way back shows the channel of the call, where the float leaves
/// (`MURMURE-700`), and nothing is sent (`MURMURE-702`). Under the phone width, the pill keeps
/// mute and hang up alone (`MURMURE-697`). The float reads a phase of its own here, since other
/// tests move the phase of the page while this one waits.
#[wasm_bindgen_test]
async fn the_call_floats_away_from_its_channel_and_brings_it_back() {
    use super::call_bar::{CallBar, CallFloat};
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone};
    let acme = Acme::new();
    for width in [1025, 390] {
        let frame = Frame::with(width, MurmureRelay::new()).await;
        frame.absorb(acme.members(&[])).await;
        frame.absorb(acme.catalogue()).await;
        frame.relay.pane.set(MurmurePane::channel(acme.board));
        let root = frame.inner.create_element("div").unwrap();
        root.set_class_name("float-root");
        frame.inner.body().unwrap().append_child(&root).unwrap();
        let phase = RwSignal::new(CallPhase::InCall(Microphone::Live));
        let (pane, relay) = (frame.relay.pane, frame.relay);
        crate::call::where_from().set(Some((acme.sales, crate::host::now())));
        let handle = mount_to(root.clone().unchecked_into(), move || {
            view! { <CallFloat pane=pane phase=Signal::from(phase) relay=relay /> }
        });
        // The header of another channel offers no call control: the float holds them
        // (`MURMURE-728`).
        let bar_root = frame.inner.create_element("div").unwrap();
        frame.inner.body().unwrap().append_child(&bar_root).unwrap();
        let bar = mount_to(bar_root.clone().unchecked_into(), move || {
            view! { <CallBar pane=pane phase=Signal::from(phase) relay=relay /> }
        });
        leptos::task::tick().await;
        let in_bar = |selector: &str| bar_root.query_selector(selector).unwrap().is_some();
        assert!(!in_bar(".murmure-call-mute") && !in_bar(".murmure-call-hang-up"));
        let within = |selector: &str| root.query_selector(selector).unwrap();
        let float = within(".murmure-call-float").expect("the call floats");
        assert_eq!(
            float.get_attribute("aria-label").as_deref(),
            Some("Call in progress")
        );
        assert_eq!(
            within(".murmure-float-channel")
                .and_then(|c| c.text_content())
                .as_deref(),
            Some("sales")
        );
        let label = |selector: &str| within(selector).and_then(|b| b.get_attribute("aria-label"));
        assert_eq!(label(".murmure-call-mute").as_deref(), Some("Mute"));
        assert_eq!(label(".murmure-call-hang-up").as_deref(), Some("Hang up"));
        let shown = |selector: &str| {
            within(selector).is_some_and(|el| el.get_bounding_client_rect().width() > 0.0)
        };
        assert!(shown(".murmure-call-mute") && shown(".murmure-call-hang-up"));
        assert_eq!(shown(".murmure-call-camera"), width > 599, "{width}");
        assert_eq!(shown(".murmure-float-back"), width > 599, "{width}");
        let before = frame.sent.get_untracked().len();
        let grip: web_sys::HtmlElement = within(".murmure-float-grip").unwrap().unchecked_into();
        grip.click();
        leptos::task::tick().await;
        assert_eq!(
            frame.relay.pane.get_untracked().shown_channel(),
            Some(acme.sales)
        );
        assert!(
            within(".murmure-call-float").is_none(),
            "its channel in view"
        );
        assert!(
            in_bar(".murmure-call-mute") && in_bar(".murmure-call-hang-up"),
            "the header of the channel of the call holds them again"
        );
        assert_eq!(
            frame.sent.get_untracked().len(),
            before,
            "the way back sends nothing"
        );
        crate::call::where_from().set(None);
        drop(bar);
        drop(handle);
    }
}

/// Waits for two frames of the window: the bubble measures itself in the frame after a change.
async fn two_frames() {
    for _ in 0..2 {
        let frame = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .expect("a window")
                .request_animation_frame(&resolve);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(frame).await;
    }
}

/// `MURMURE-698`: the bubble publishes the box it covers, and the box of its mini video once a
/// press on its faces opens it. The faces are a button with its name and its pressed state
/// (`MURMURE-693`). Over a picture, the mini box is the topmost element at its centre and paints
/// nothing, so the canvas below shows through (`MURMURE-699`). The way back clears
/// the cover. Under the phone width, the pill publishes no mini video.
#[wasm_bindgen_test]
async fn the_bubble_publishes_what_it_covers_and_the_box_of_its_mini_video() {
    use super::call_bar::CallFloat;
    use crate::call_stage::{MiniShows, cover, mini, mini_shows};
    use ambre_murmure::ui::call_controls::{CallPhase, Microphone};
    use ambre_murmure::ui::call_float::{Cover, Mini};
    let acme = Acme::new();
    for width in [1025, 390] {
        let frame = Frame::with(width, MurmureRelay::new()).await;
        frame.absorb(acme.members(&[])).await;
        frame.absorb(acme.catalogue()).await;
        frame.relay.pane.set(MurmurePane::channel(acme.board));
        let root = frame.inner.create_element("div").unwrap();
        frame.inner.body().unwrap().append_child(&root).unwrap();
        let phase = RwSignal::new(CallPhase::InCall(Microphone::Live));
        let (pane, relay) = (frame.relay.pane, frame.relay);
        crate::call::where_from().set(Some((acme.sales, crate::host::now())));
        mini().set(Mini::Folded);
        mini_shows().set(MiniShows::Faces);
        let handle = mount_to(root.clone().unchecked_into(), move || {
            view! { <CallFloat pane=pane phase=Signal::from(phase) relay=relay /> }
        });
        leptos::task::tick().await;
        two_frames().await;
        let within = |selector: &str| root.query_selector(selector).unwrap();
        let rect_of = |selector: &str| {
            let r = within(selector).expect(selector).get_bounding_client_rect();
            ambre_draw::Rect::new(r.left(), r.top(), r.right(), r.bottom())
        };
        assert_eq!(
            cover(),
            Cover {
                bubble: Some(rect_of(".murmure-call-float")),
                mini: None,
            },
            "{width}: the folded bubble covers its own box alone"
        );
        let faces: web_sys::HtmlElement = within(".murmure-float-faces").unwrap().unchecked_into();
        assert_eq!(faces.tag_name(), "BUTTON");
        assert_eq!(
            faces.get_attribute("aria-label").as_deref(),
            Some("Show the video of the call")
        );
        assert_eq!(
            faces.get_attribute("aria-pressed").as_deref(),
            Some("false")
        );
        faces.focus().unwrap();
        assert!(frame.inner.active_element().is_some_and(|e| e == *faces));
        faces.click();
        leptos::task::tick().await;
        two_frames().await;
        assert_eq!(faces.get_attribute("aria-pressed").as_deref(), Some("true"));
        assert_eq!(
            faces.get_attribute("aria-label").as_deref(),
            Some("Hide the video of the call")
        );
        let bubble = rect_of(".murmure-call-float");
        if width > 599 {
            let mini_box = rect_of(".murmure-float-mini");
            assert_eq!(
                cover(),
                Cover {
                    bubble: Some(bubble),
                    mini: Some(mini_box),
                }
            );
            assert_eq!(bubble.intersect(mini_box), mini_box, "inside the bubble");
            // A picture arrives: the box paints nothing, and no control covers it.
            mini_shows().set(MiniShows::Video);
            leptos::task::tick().await;
            let float = within(".murmure-call-float").unwrap();
            assert_eq!(float.get_attribute("data-mini").as_deref(), Some("video"));
            let view = frame.inner.default_view().unwrap();
            let background = |el: &web_sys::Element| {
                view.get_computed_style(el)
                    .unwrap()
                    .unwrap()
                    .get_property_value("background-color")
                    .unwrap()
            };
            let mini_el = within(".murmure-float-mini").unwrap();
            assert_eq!(background(&float), "rgba(0, 0, 0, 0)");
            assert_eq!(background(&mini_el), "rgba(0, 0, 0, 0)");
            assert!(within(".murmure-float-mini-none").is_none());
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a coordinate of the frame fits an f32"
            )]
            let at = |x: f64, y: f64| frame.inner.element_from_point(x as f32, y as f32);
            let c = mini_box.center();
            for (x, y) in [
                (c.x, c.y),
                (mini_box.x0 + 2.0, mini_box.y0 + 2.0),
                (mini_box.x1 - 2.0, mini_box.y1 - 2.0),
            ] {
                let top = at(x, y);
                assert_eq!(
                    top.as_ref(),
                    Some(&mini_el),
                    "the mini box is topmost at ({x}, {y}), not {:?}",
                    top.as_ref().map(web_sys::Element::class_name)
                );
            }
        } else {
            assert_eq!(
                cover(),
                Cover {
                    bubble: Some(bubble),
                    mini: None,
                },
                "the pill shows no video"
            );
        }
        let grip: web_sys::HtmlElement = within(".murmure-float-grip").unwrap().unchecked_into();
        grip.click();
        leptos::task::tick().await;
        two_frames().await;
        assert_eq!(
            cover(),
            Cover::NONE,
            "{width}: the channel of the call in view"
        );
        crate::call::where_from().set(None);
        mini().set(Mini::Folded);
        mini_shows().set(MiniShows::Faces);
        drop(handle);
    }
}
