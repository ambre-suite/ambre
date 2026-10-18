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

use ambre_bus::{
    ChannelActivity, ChannelName, ChannelPlace, LogicCommand, MemberChannel, MurmureEvent, UiEvent,
};
use ambre_chrome::{CommandHandler, KeyRoute, PaletteRow, ShellAction};
use ambre_commands::Locale;
use ambre_core::{ChannelId, SpaceId};
use ambre_murmure::MurmurePane;
use ambre_murmure::chrome::{MurmureHandler, MurmureRelay};
use ambre_murmure::commands::{COMMANDS, MARK_ALL_READ, NEXT_ACTIVITY, PALETTE, PREVIOUS_ACTIVITY};
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
        peers: Vec::new(),
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
        peers: Vec::new(),
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

/// A relay with two private channels and a direct message, each named by
/// the listing: `[budget, zeta, Budget review]` in sidebar order.
fn three_named() -> (Owner, MurmureRelay, [ChannelId; 3]) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let space = SpaceId::new();
    let ids = [ChannelId::new(), ChannelId::new(), ChannelId::new()];
    let [budget, zeta, direct] = ids;
    let member = |channel_id, place| MemberChannel { channel_id, place };
    let name = |channel_id, name: &str| ChannelName {
        channel_id,
        name: name.to_owned(),
    };
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![
            member(direct, ChannelPlace::Direct),
            member(zeta, ChannelPlace::Private(space)),
            member(budget, ChannelPlace::Private(space)),
        ],
        spaces: Vec::new(),
        names: vec![
            name(budget, "budget"),
            name(zeta, "zeta"),
            name(direct, "Budget review"),
        ],
        peers: Vec::new(),
    });
    (owner, relay, ids)
}

/// The palette lists the channels and direct messages in sidebar order,
/// before every command, and the query keeps those whose name matches
/// (`MURMURE-172`).
#[wasm_bindgen_test]
fn the_palette_lists_the_channels_first_in_sidebar_order() {
    let (_owner, relay, [budget, zeta, direct]) = three_named();
    let entries = relay.palette_entries().get_untracked();
    let keys: Vec<[u8; 16]> = entries.iter().map(|e| e.key).collect();
    assert_eq!(
        keys,
        [budget, zeta, direct].map(|c| *c.as_bytes()),
        "{entries:?}"
    );
    assert_eq!(entries[2].hint, Some("Direct message"));

    let shown = |query| -> Vec<String> {
        ambre_chrome::palette_rows(&entries, COMMANDS, query, &Locale::En)
            .into_iter()
            .map(|row| match row {
                PaletteRow::Entry(e) => e.label.clone(),
                PaletteRow::Command(c) => c.id.0.to_owned(),
            })
            .collect()
    };
    assert_eq!(shown("bud"), ["budget", "Budget review"]);
    let all = shown("");
    assert_eq!(all.len(), entries.len() + COMMANDS.len());
    assert_eq!(all[..3], ["budget", "zeta", "Budget review"]);
}

/// Choosing a channel in the palette opens it, as a click on the sidebar
/// does (`MURMURE-173`).
#[wasm_bindgen_test]
fn choosing_a_channel_in_the_palette_opens_it() {
    let (_owner, relay, [_, zeta, _]) = three_named();
    let opened = relay.choose(*zeta.as_bytes());
    assert!(
        matches!(opened, Some(LogicCommand::OpenChannel { channel_id }) if channel_id == zeta),
        "{opened:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(zeta));
}

/// The commands a handler sent through its sink.
fn handler(relay: MurmureRelay) -> (MurmureHandler, RwSignal<Vec<LogicCommand>>) {
    let sent = RwSignal::new(Vec::new());
    let send = Callback::new(move |command| sent.update(|s| s.push(command)));
    (MurmureHandler { relay, send }, sent)
}

/// The jump chords open the next or the previous channel with activity in
/// sidebar order (`MURMURE-176`). They send its opening alone, and no read
/// cursor moves (`MURMURE-177`).
#[wasm_bindgen_test]
fn a_jump_opens_the_next_active_channel_and_moves_no_read_cursor() {
    let (_owner, relay, [budget, zeta, direct]) = three_named();
    let active = |channel_id| ChannelActivity {
        channel_id,
        active: true,
        unread: None,
        mentions: None,
    };
    relay.absorb(&UiEvent::ChannelActivity {
        channels: vec![active(budget), active(direct)],
    });
    let (handler, sent) = handler(relay);

    assert!(matches!(
        handler.on_key(NEXT_ACTIVITY, false),
        KeyRoute::Run(actions) if actions.is_empty()
    ));
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(budget));
    handler.on_key(NEXT_ACTIVITY, false);
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(direct));
    handler.on_command(PREVIOUS_ACTIVITY);
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(budget));
    assert_ne!(relay.pane.get_untracked().shown_channel(), Some(zeta));

    let sent = sent.get_untracked();
    assert_eq!(sent.len(), 2, "each channel opens once: {sent:?}");
    assert!(
        sent.iter()
            .all(|c| matches!(c, LogicCommand::OpenChannel { .. })),
        "a jump sends no read cursor: {sent:?}"
    );
}

/// The mark-all-read chord runs the mark-all-read command of the unread
/// feature (`MURMURE-180`). A text field keeps the chord, and the palette
/// chord opens the palette.
#[wasm_bindgen_test]
fn the_mark_all_read_chord_runs_the_command_of_the_unread_feature() {
    let (_owner, relay, _) = three_named();
    let (handler, sent) = handler(relay);
    assert!(matches!(
        handler.on_key(MARK_ALL_READ, true),
        KeyRoute::Yield
    ));
    assert!(sent.get_untracked().is_empty());

    handler.on_key(MARK_ALL_READ, false);
    assert!(matches!(
        sent.get_untracked()[..],
        [LogicCommand::MarkAllRead]
    ));
    assert!(matches!(
        handler.on_key(PALETTE, true),
        KeyRoute::Run(actions) if matches!(actions[..], [ShellAction::OpenPalette])
    ));
}
