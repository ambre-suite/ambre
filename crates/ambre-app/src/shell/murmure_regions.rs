// ambre-app/src/shell/murmure_regions.rs
// Browser-tier tests of the Murmure shell at a set width (the Murmure layout
// spec). A width query reads the viewport of the frame it runs in. So each
// test mounts the shell into an iframe of the width it reads.

use super::app_root::MurmureShell;
use ambre_core::{ChannelId, MessageId, SegmentId};
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
    /// Unmounts the shell on drop.
    _handle: Box<dyn std::any::Any>,
}

impl Frame {
    async fn new(width: u32, pane: MurmurePane) -> Self {
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

        let pane = RwSignal::new(pane);
        let name = RwSignal::new(Some("general".to_owned()));
        let handle = mount_to(inner.body().unwrap(), move || {
            view! {
                <MurmureShell pane=pane channel_name=name>
                    <span class="title">"Ambre"</span>
                </MurmureShell>
            }
        });
        leptos::task::tick().await;
        Self {
            frame,
            inner,
            pane,
            _handle: Box::new(handle),
        }
    }

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
