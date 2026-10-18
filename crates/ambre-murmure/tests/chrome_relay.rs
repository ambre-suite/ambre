//! The Murmure chrome state, driven by the events Logic sends
//! (`features/ambre-spec-murmure-layout.md`).
//!
//! The chrome is wasm-only Leptos, so these tests run under
//! `wasm-pack test --headless --chrome crates/ambre-murmure --test chrome_relay`
//! (`just test-wasm`). On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::{ChannelPlace, LogicCommand, MemberChannel, MurmureEvent, UiEvent};
use ambre_core::ChannelId;
use ambre_murmure::MurmurePane;
use ambre_murmure::chrome::MurmureRelay;
use leptos::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// A relay that holds one direct channel, shown in the pane.
fn shown_direct() -> (Owner, MurmureRelay, ChannelId) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let direct = ChannelId::new();
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: direct,
            place: ChannelPlace::Direct,
        }],
        spaces: Vec::new(),
        names: Vec::new(),
    });
    let opened = relay.select(direct);
    assert!(
        matches!(opened, Some(LogicCommand::OpenChannel { channel_id }) if channel_id == direct),
        "{opened:?}"
    );
    (owner, relay, direct)
}

/// A channel the user lost while it was open leaves the sidebar, and the
/// pane showing it goes back to the channel list. A later invite opens it
/// again (`MURMURE-489`).
#[wasm_bindgen_test]
fn a_channel_lost_while_open_leaves_the_sidebar_and_the_pane() {
    let (_owner, relay, direct) = shown_direct();
    let lost = UiEvent::Channel {
        channel_id: direct,
        event: MurmureEvent::Left,
    };
    assert!(relay.absorb(&lost).is_empty());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::ChannelList);
    assert!(!relay.sidebar.with_untracked(|s| s.is_member(direct)));

    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: direct,
            place: ChannelPlace::Direct,
        }],
        spaces: Vec::new(),
        names: Vec::new(),
    });
    assert!(
        matches!(relay.select(direct), Some(LogicCommand::OpenChannel { .. })),
        "the channel opens again"
    );
}

/// The name an open channel learns is the one its header reads
/// (`MURMURE-428`).
#[wasm_bindgen_test]
fn a_channel_named_shows_its_name_in_the_header() {
    let (_owner, relay, direct) = shown_direct();
    let named = UiEvent::Channel {
        channel_id: direct,
        event: MurmureEvent::Named {
            name: Some("Ada".to_owned()),
        },
    };
    assert!(relay.absorb(&named).is_empty());
    assert_eq!(relay.channel_name().get_untracked().as_deref(), Some("Ada"));
}
