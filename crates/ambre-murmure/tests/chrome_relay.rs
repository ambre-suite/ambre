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
    CatalogueEntry, ChannelActivity, ChannelMatches, ChannelName, ChannelPlace, DirectMessageEvent,
    LogicCommand, MemberChannel, MemberInfo, MemberStanding, MemberView, MessageAt, MessageView,
    MurmureCommand, MurmureEvent, Presence, RequestId, RestCommand, RestEvent, Role, SearchMatch,
    SpaceEvent, Timestamp, UiEvent,
};
use ambre_chrome::{CommandHandler, KeyRoute, MenuEntry, PaletteRow, ShellAction};
use ambre_commands::{KeyBinding, Locale};
use ambre_core::{ChannelId, MessageId, SegmentId, SpaceId, UserId};
use ambre_murmure::chrome::{DeleteConfirm, MurmureHandler, MurmureRelay, SearchBox, coverage};
use ambre_murmure::commands::{
    COMMANDS, COPY_TEXT, DELETE, MARK_ALL_READ, MARK_UNREAD, MessageTarget, NEXT_ACTIVITY, PALETTE,
    PREVIOUS_ACTIVITY, REPLY, SEARCH,
};
use ambre_murmure::{MessageRef, MurmurePane};
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

/// A relay showing `general`, a public channel of a space whose members are
/// this user, Bob and Carol. Returns the space's members in that order.
fn a_space_with_members() -> (Owner, MurmureRelay, ChannelId, [UserId; 3]) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let space = SpaceId::new();
    let general = ChannelId::new();
    let users = [UserId::new(), UserId::new(), UserId::new()];
    relay.absorb(&UiEvent::Rest {
        id: RequestId::next(),
        evt: RestEvent::SessionReady {
            user_id: users[0],
            display_name: "Alice".to_owned(),
            email: "alice@test.local".to_owned(),
        },
    });
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: general,
            place: ChannelPlace::Public(space),
        }],
        spaces: vec![space],
        names: Vec::new(),
        peers: Vec::new(),
    });
    relay.absorb(&UiEvent::Space {
        space_id: space,
        event: SpaceEvent::Catalogue {
            name: Some("Acme".to_owned()),
            categories: Vec::new(),
            channels: vec![CatalogueEntry {
                channel_id: general,
                name: "general".to_owned(),
                topic: None,
                category: None,
                position: 0,
            }],
        },
    });
    let member = |user_id, name: &str| MemberView {
        user_id,
        display_name: name.to_owned(),
        presence: Presence::Offline,
    };
    relay.absorb(&UiEvent::Space {
        space_id: space,
        event: SpaceEvent::Members {
            members: vec![
                member(users[0], "Alice"),
                member(users[1], "Bob"),
                member(users[2], "Carol"),
            ],
        },
    });
    relay.select(general);
    (owner, relay, general, users)
}

/// After the channels, the palette lists the other members of the space of
/// the shown channel, and the query keeps those whose name matches. This
/// user is not among them (`MURMURE-452`).
#[wasm_bindgen_test]
fn the_palette_lists_the_other_members_of_the_space_after_the_channels() {
    let (_owner, relay, general, [_, bob, carol]) = a_space_with_members();
    let entries = relay.palette_entries().get_untracked();
    let keys: Vec<[u8; 16]> = entries.iter().map(|e| e.key).collect();
    assert_eq!(
        keys,
        [*general.as_bytes(), *bob.as_bytes(), *carol.as_bytes()],
        "{entries:?}"
    );
    assert_eq!(entries[1].hint, Some("Member"));
    let shown: Vec<String> = ambre_chrome::palette_rows(&entries, COMMANDS, "bo", &Locale::En)
        .into_iter()
        .map(|row| match row {
            PaletteRow::Entry(e) => e.label.clone(),
            PaletteRow::Command(c) => c.id.0.to_owned(),
        })
        .collect();
    assert_eq!(shown, ["Bob"]);
}

/// Choosing a member fetches their key and opens nothing yet: the key is
/// checked against the pin first. The direct message the relay answers
/// then shows, and opens nothing the Logic thread did not (`MURMURE-455`).
#[wasm_bindgen_test]
fn choosing_a_member_fetches_their_key_then_shows_the_direct_message() {
    let (_owner, relay, general, [_, bob, _]) = a_space_with_members();
    let fetched = relay.choose(*bob.as_bytes());
    assert!(
        matches!(
            fetched,
            Some(LogicCommand::Rest { cmd: RestCommand::FetchMemberKey { user_id }, .. })
                if user_id == bob
        ),
        "{fetched:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(general));
    assert_eq!(relay.member_name(bob).as_deref(), Some("Bob"));

    let direct = ChannelId::new();
    let after = relay.absorb(&UiEvent::DirectMessage {
        requested: ChannelId::new(),
        event: DirectMessageEvent::Opened { channel_id: direct },
    });
    assert!(
        matches!(after[..], [LogicCommand::ListMemberChannels]),
        "{after:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(direct));
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: direct,
            place: ChannelPlace::Direct,
        }],
        spaces: Vec::new(),
        names: Vec::new(),
        peers: Vec::new(),
    });
    assert!(relay.select(direct).is_none(), "it is open already");
}

/// A private channel and a direct message offer to invite the other members:
/// those of the channel's space, or of every open space, each once. A public
/// channel offers none. The key fetched for an invite is taken once, for the
/// channel shown when it was asked (the Murmure channels spec).
#[wasm_bindgen_test]
fn an_invite_fetches_the_key_for_the_shown_channel_and_is_taken_once() {
    let (_owner, relay, general, [alice, bob, carol]) = a_space_with_members();
    assert_eq!(relay.invite_target().get_untracked(), None, "a public one");
    assert!(relay.invite(bob).is_none());

    let space = relay
        .sidebar
        .with_untracked(|s| s.public_space_of(general))
        .unwrap();
    let other = SpaceId::new();
    let (board, direct, dave) = (ChannelId::new(), ChannelId::new(), UserId::new());
    let member = |channel_id, place| MemberChannel { channel_id, place };
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![
            member(general, ChannelPlace::Public(space)),
            member(board, ChannelPlace::Private(space)),
            member(direct, ChannelPlace::Direct),
        ],
        spaces: vec![space, other],
        names: Vec::new(),
        peers: Vec::new(),
    });
    let view = |user_id, name: &str| MemberView {
        user_id,
        display_name: name.to_owned(),
        presence: Presence::Offline,
    };
    relay.absorb(&UiEvent::Space {
        space_id: other,
        event: SpaceEvent::Members {
            members: vec![view(bob, "Bob"), view(dave, "Dave")],
        },
    });
    let offered = |channel| -> Vec<UserId> {
        relay.select(channel);
        assert_eq!(relay.invite_target().get_untracked(), Some(channel));
        relay
            .invitees()
            .get_untracked()
            .iter()
            .map(|m| m.user_id)
            .collect()
    };
    assert_eq!(offered(board), [bob, carol], "the space of the channel");
    assert_eq!(
        offered(direct),
        [bob, carol, dave],
        "every space, each once"
    );

    assert!(relay.invite(alice).is_none(), "never oneself");
    let fetched = relay.invite(carol);
    assert!(
        matches!(
            fetched,
            Some(LogicCommand::Rest { cmd: RestCommand::FetchMemberKey { user_id }, .. })
                if user_id == carol
        ),
        "{fetched:?}"
    );
    relay.select(board);
    assert_eq!(relay.take_invite(bob), None, "another member's key");
    assert_eq!(relay.take_invite(carol), Some(direct), "the channel asked");
    assert_eq!(relay.take_invite(carol), None, "taken once");

    assert!(relay.invite(bob).is_some());
    assert!(relay.choose(*bob.as_bytes()).is_some());
    assert_eq!(relay.take_invite(bob), None, "a direct message cancels it");
}

/// The invite control offers no member the open channel lists, and fetches
/// no key for one (`MURMURE-501`). A channel left forgets its list.
#[wasm_bindgen_test]
fn the_invite_control_offers_no_member_of_the_channel() {
    let (_owner, relay, general, [_alice, bob, carol]) = a_space_with_members();
    let space = relay
        .sidebar
        .with_untracked(|s| s.public_space_of(general))
        .unwrap();
    let board = ChannelId::new();
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: board,
            place: ChannelPlace::Private(space),
        }],
        spaces: vec![space],
        names: Vec::new(),
        peers: Vec::new(),
    });
    relay.select(board);
    let offered = || -> Vec<UserId> {
        relay
            .invitees()
            .get_untracked()
            .iter()
            .map(|m| m.user_id)
            .collect()
    };
    assert_eq!(offered(), [bob, carol], "no list yet: every member");

    let info = |user_id| MemberInfo {
        user_id,
        display_name: String::new(),
        email: String::new(),
        role: Role::Editor,
        standing: MemberStanding::Verified,
    };
    relay.absorb(&UiEvent::Channel {
        channel_id: board,
        event: MurmureEvent::Members {
            members: vec![info(bob)],
        },
    });
    assert_eq!(offered(), [carol], "bob holds the channel");
    assert!(relay.invite(bob).is_none(), "no key for a member");
    assert!(relay.invite(carol).is_some());

    relay.absorb(&UiEvent::Channel {
        channel_id: board,
        event: MurmureEvent::Left,
    });
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: board,
            place: ChannelPlace::Private(space),
        }],
        spaces: vec![space],
        names: Vec::new(),
        peers: Vec::new(),
    });
    relay.select(board);
    assert_eq!(offered(), [bob, carol], "the list left with the channel");
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

/// The labels of the menu of the target, in order.
fn menu_labels(relay: MurmureRelay) -> Vec<String> {
    relay
        .menu_entries()
        .get_untracked()
        .iter()
        .map(|entry| match entry {
            MenuEntry::Item(item) => item.label().to_owned(),
            MenuEntry::Separator => "-".to_owned(),
        })
        .collect()
}

/// The menu offers the actions of the message the view named, each a
/// command of the table. Reply opens its thread, mark unread sends the
/// cursor back, and copy asks the shell for the clipboard. Each closes the
/// menu, and an action with no target does nothing (`MURMURE-419`).
#[wasm_bindgen_test]
fn the_menu_of_a_message_runs_its_actions_on_it() {
    let (_owner, relay, direct) = shown_direct();
    let (handler, sent) = handler(relay);
    assert!(menu_labels(relay).is_empty(), "no target, no menu");
    assert!(handler.on_command(REPLY).is_empty());
    assert!(sent.get_untracked().is_empty(), "no target, nothing sent");

    let segment = SegmentId::new();
    let message = MessageId::new();
    let target = MessageTarget {
        channel: direct,
        at: MessageAt::Segment(segment),
        message,
        author: UserId::new(),
        text: "hello".to_owned(),
        notice: false,
        deleted: false,
        pressed: None,
        canvas: "ambre-canvas",
    };
    relay.target.set(Some(target.clone()));
    assert_eq!(
        menu_labels(relay),
        ["Reply in thread", "Mark unread", "Copy text"]
    );

    assert!(handler.on_command(REPLY).is_empty());
    let root = MessageRef { segment, message };
    assert_eq!(
        relay.pane.get_untracked(),
        MurmurePane::Thread {
            channel: direct,
            root
        }
    );
    assert_eq!(relay.target.get_untracked(), None, "the menu closed");

    relay.target.set(Some(target.clone()));
    handler.on_command(MARK_UNREAD);
    assert!(matches!(
        sent.get_untracked()[..],
        [LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::MarkUnread { segment: s, message: m },
        }] if channel_id == direct && s == segment && m == message
    ));

    relay.target.set(Some(target));
    assert!(matches!(
        &handler.on_command(COPY_TEXT)[..],
        [ShellAction::CopyText(text)] if text == "hello"
    ));
    assert_eq!(sent.get_untracked().len(), 1, "copy sends nothing");
}

/// The member's own message offers its deletion, last. The action sends
/// nothing and opens the dialog. Cancel and a close of the dialog send
/// nothing, and Delete sends the deletion, stamped by the view
/// (`MURMURE-512`).
#[wasm_bindgen_test]
async fn a_deletion_is_sent_only_once_the_dialog_confirms_it() {
    use wasm_bindgen::JsCast;

    let (_owner, relay, direct) = shown_direct();
    let me = UserId::new();
    relay.absorb(&UiEvent::Rest {
        id: RequestId::next(),
        evt: RestEvent::SessionReady {
            user_id: me,
            display_name: "Alice".to_owned(),
            email: "alice@test.local".to_owned(),
        },
    });
    let (handler, sent) = handler(relay);
    let segment = SegmentId::new();
    let message = MessageId::new();
    let target = MessageTarget {
        channel: direct,
        at: MessageAt::Segment(segment),
        message,
        author: me,
        text: "oops".to_owned(),
        notice: false,
        deleted: false,
        pressed: None,
        canvas: "ambre-canvas",
    };
    let other = MessageTarget {
        author: UserId::new(),
        ..target.clone()
    };
    relay.target.set(Some(other));
    assert!(!menu_labels(relay).contains(&"Delete message".to_owned()));
    relay.target.set(Some(target.clone()));
    assert_eq!(
        menu_labels(relay).last().map(String::as_str),
        Some("Delete message")
    );

    let send = handler.send;
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let now = Timestamp(1_700_000_000_000);
    let _handle = leptos::mount::mount_to(mount, move || {
        view! { <DeleteConfirm relay=relay send=send now=Callback::new(move |()| now) /> }
    });
    let dialog: web_sys::HtmlDialogElement = root
        .query_selector("dialog")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let click = |class: &str| {
        let selector = format!(".{class}");
        let button = root.query_selector(&selector).unwrap().unwrap();
        button.dyn_into::<web_sys::HtmlElement>().unwrap().click();
    };

    assert!(handler.on_command(DELETE).is_empty());
    leptos::task::tick().await;
    assert!(dialog.open(), "the action asks first");
    assert_eq!(relay.target.get_untracked(), None, "the menu closed");
    assert!(
        sent.get_untracked().is_empty(),
        "nothing sent before Delete"
    );
    click("murmure-delete-cancel");
    leptos::task::tick().await;
    assert!(!dialog.open());
    assert_eq!(relay.deleting.get_untracked(), None);
    assert!(sent.get_untracked().is_empty(), "a cancel sends nothing");

    relay.target.set(Some(target.clone()));
    handler.on_command(DELETE);
    leptos::task::tick().await;
    assert!(dialog.open());
    // Escape closes the dialog natively, as this close does. Its event is
    // queued as a task, after the close returns.
    let closed = js_sys::Promise::new(&mut |resolve, _| {
        dialog
            .add_event_listener_with_callback("close", &resolve)
            .unwrap();
    });
    dialog.close();
    wasm_bindgen_futures::JsFuture::from(closed).await.unwrap();
    assert_eq!(relay.deleting.get_untracked(), None, "a close cancels");
    assert!(sent.get_untracked().is_empty(), "a close sends nothing");

    relay.target.set(Some(target));
    handler.on_command(DELETE);
    leptos::task::tick().await;
    click("murmure-delete-confirm");
    leptos::task::tick().await;
    assert!(!dialog.open());
    let at = MessageAt::Segment(segment);
    assert!(matches!(
        sent.get_untracked()[..],
        [LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::DeleteMessage { at: a, message: m, now: t },
        }] if channel_id == direct && a == at && m == message && t == now
    ));
    root.remove();
}

/// What a search of `query` found: `budget` holds one match in the one
/// segment of two it read, and `zeta` holds none.
fn found(query: &str, budget: ChannelId, zeta: ChannelId) -> UiEvent {
    let segment = SegmentId::new();
    let message = MessageView {
        id: MessageId::new(),
        author: UserId::new(),
        ts: Timestamp(1),
        text: "the budget for May".to_owned(),
        mentions: Vec::new(),
        reactions: Vec::new(),
        notice: None,
        edited: false,
        deleted: false,
        thread: None,
        also_in_thread: None,
    };
    UiEvent::MessageSearch {
        query: query.to_owned(),
        channels: vec![
            ChannelMatches {
                channel_id: budget,
                covered: vec![segment],
                listed: 2,
                matches: vec![SearchMatch { segment, message }],
            },
            ChannelMatches {
                channel_id: zeta,
                covered: vec![SegmentId::new()],
                listed: 1,
                matches: Vec::new(),
            },
        ],
    }
}

/// The find chord opens the search, from a text field too, and keeps the
/// meaning the other applications give it (`MURMURE-213`). Each change of
/// the words asks once, and an answer to an older query is dropped.
#[wasm_bindgen_test]
fn the_find_chord_opens_the_search_and_its_answer_lists_the_matches() {
    let (_owner, relay, [budget, zeta, _]) = three_named();
    let binding = COMMANDS.iter().find(|c| c.id == SEARCH).unwrap();
    assert_eq!(binding.keybindings, [KeyBinding::primary('f')]);
    let (handler, sent) = handler(relay);
    assert!(matches!(
        handler.on_key(SEARCH, true),
        KeyRoute::Run(actions) if actions.is_empty()
    ));
    assert!(relay.search.get_untracked().open);
    assert!(sent.get_untracked().is_empty(), "opening asks nothing");

    let asked = relay.search_for("bud".to_owned());
    assert!(matches!(asked, Some(LogicCommand::SearchMessages { ref query }) if query == "bud"));
    assert!(
        relay.search_for("bud".to_owned()).is_none(),
        "the same words ask once"
    );
    assert!(relay.search_for("budget".to_owned()).is_some());

    relay.absorb(&found("bud", budget, zeta));
    assert!(
        relay.search_groups().get_untracked().is_empty(),
        "a stale answer"
    );
    relay.absorb(&found("budget", budget, zeta));
    let groups = relay.search_groups().get_untracked();
    assert_eq!(groups.len(), 1, "a channel with no match shows no group");
    assert_eq!(groups[0].name, "budget");
    let texts: Vec<&str> = groups[0].matches.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, ["the budget for May"]);
    assert_eq!(groups[0].coverage, "1 of 2 segments");
    let channels = relay.search.get_untracked().channels;
    assert_eq!(
        coverage(&channels),
        "Searched 2 of 3 segments in 2 open channels"
    );

    assert!(
        relay.search_for("  ".to_owned()).is_none(),
        "no word asks nothing"
    );
    assert!(relay.search.get_untracked().channels.is_empty());
}

/// A match opens its channel at its message, and closes the box
/// (`MURMURE-150`). An answer that arrives after the box closed shows
/// nothing.
#[wasm_bindgen_test]
fn a_match_opens_its_message_in_its_channel_and_closes_the_search() {
    let (_owner, relay, [budget, zeta, _]) = three_named();
    let (handler, _) = handler(relay);
    handler.on_command(SEARCH);
    relay.search_for("budget".to_owned());
    relay.absorb(&found("budget", budget, zeta));

    let groups = relay.search_groups().get_untracked();
    let (at, _) = groups[0].matches[0].clone();
    let opened = relay.choose_result(budget, at);
    assert!(
        matches!(opened, Some(LogicCommand::OpenChannel { channel_id }) if channel_id == budget),
        "{opened:?}"
    );
    assert_eq!(
        relay.pane.get_untracked(),
        MurmurePane::Channel {
            channel: budget,
            at: Some(at)
        }
    );
    assert_eq!(relay.search.get_untracked(), SearchBox::default());
    relay.absorb(&found("", budget, zeta));
    relay.absorb(&found("budget", budget, zeta));
    assert!(relay.search_groups().get_untracked().is_empty());
}

fn news(channel_id: ChannelId, mention: bool) -> UiEvent {
    UiEvent::NewMessage {
        channel_id,
        mention,
        message: None,
    }
}

/// The notice of an open channel's message names it, and a click opens the
/// channel at it (`MURMURE-507`). A notice of a closed channel names none,
/// and the list keeps its own resting place.
#[wasm_bindgen_test]
fn a_notice_opens_its_channel_at_its_message() {
    let (_owner, relay, direct) = shown_direct();
    relay.pane.set(MurmurePane::ChannelList);
    let at = MessageRef {
        segment: SegmentId::new(),
        message: MessageId::new(),
    };
    let event = UiEvent::NewMessage {
        channel_id: direct,
        mention: false,
        message: Some((at.segment, at.message)),
    };
    let notice = relay
        .notice(&event, true)
        .expect("a direct message notifies");
    assert_eq!(notice.at, Some(at));
    // The channel was opened once already: the click asks Logic nothing.
    assert!(relay.open_at(notice.channel_id, notice.at).is_none());
    assert_eq!(
        relay.pane.get_untracked(),
        MurmurePane::Channel {
            channel: direct,
            at: Some(at)
        }
    );
    relay.pane.set(MurmurePane::ChannelList);
    let closed = relay.notice(&news(direct, false), true).expect("notifies");
    assert_eq!(closed.at, None);
    assert!(relay.open_at(direct, closed.at).is_none());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::channel(direct));
}

/// A click on a notice opens its own channel and asks nothing of another
/// one, so no other read cursor moves (`MURMURE-508`).
#[wasm_bindgen_test]
fn a_notice_asks_nothing_of_another_channel() {
    let (_owner, relay, general, _) = a_space_with_members();
    let other = ChannelId::new();
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![
            MemberChannel {
                channel_id: general,
                place: ChannelPlace::Public(SpaceId::new()),
            },
            MemberChannel {
                channel_id: other,
                place: ChannelPlace::Direct,
            },
        ],
        spaces: Vec::new(),
        names: Vec::new(),
        peers: Vec::new(),
    });
    let at = MessageRef {
        segment: SegmentId::new(),
        message: MessageId::new(),
    };
    let opened = relay.open_at(other, Some(at));
    assert!(
        matches!(opened, Some(LogicCommand::OpenChannel { channel_id }) if channel_id == other),
        "{opened:?}"
    );
    assert!(relay.open_at(other, Some(at)).is_none());
}

/// A mention of the user notifies, and a plain message of a public channel
/// does not. A message of a direct message notifies with or without one,
/// under its members' names (`MURMURE-143`).
#[wasm_bindgen_test]
fn a_mention_or_a_direct_message_notifies_and_a_plain_message_does_not() {
    let (_owner, relay, general, _) = a_space_with_members();
    let direct = ChannelId::new();
    relay.absorb(&UiEvent::MemberChannels {
        channels: vec![
            MemberChannel {
                channel_id: general,
                place: ChannelPlace::Public(SpaceId::new()),
            },
            MemberChannel {
                channel_id: direct,
                place: ChannelPlace::Direct,
            },
        ],
        spaces: Vec::new(),
        names: Vec::new(),
        peers: vec![ambre_bus::DirectPeers {
            channel_id: direct,
            names: vec!["Bob".to_owned()],
        }],
    });
    relay.pane.set(MurmurePane::ChannelList);

    assert_eq!(relay.notice(&news(general, false), true), None);
    let mention = relay
        .notice(&news(general, true), true)
        .expect("a mention notifies");
    assert_eq!(mention.channel_id, general);
    assert_eq!(mention.title, "You were mentioned in general");
    for named in [false, true] {
        let message = relay
            .notice(&news(direct, named), true)
            .expect("a direct message notifies");
        assert_eq!(message.channel_id, direct);
        assert_eq!(message.title, "New message from Bob");
    }
    assert_eq!(relay.notice(&UiEvent::UpdateRequired, true), None);
}

/// The channel the user is reading notifies nothing. On a hidden page
/// nobody reads it, and it notifies (`MURMURE-146`).
#[wasm_bindgen_test]
fn the_channel_being_read_notifies_nothing_until_the_page_is_hidden() {
    let (_owner, relay, direct) = shown_direct();
    assert_eq!(relay.notice(&news(direct, true), true), None);
    assert!(relay.notice(&news(direct, true), false).is_some());
    relay.pane.set(MurmurePane::ChannelList);
    assert!(relay.notice(&news(direct, false), true).is_some());
}
