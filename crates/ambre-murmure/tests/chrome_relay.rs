//! The Murmure chrome state, driven by the events Logic sends
//! (`features/ambre-spec-murmure-layout.md`).
//!
//! The chrome is wasm-only Leptos, so these tests run under
//! `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`). On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::{
    Authored, ChannelActivity, ChannelName, ChannelPlace, Instant, MemberChannel, MemberInfo,
    MemberStanding, RestCommand, RestEvent, Role,
};
use ambre_chrome::{CommandHandler, KeyRoute, MenuEntry, PaletteKey, PaletteRow, ShellAction};
use ambre_commands::{KeyBinding, KeyChar, Locale};
use ambre_core::id::{Channel, Document, Id, Message, Segment, Space, User};
use ambre_core::index::Pos;
use ambre_core::{Point, Px, View};
use ambre_murmure::chrome::{
    ChannelMentionConfirm, DeleteConfirm, EmojiPicker, KeyList, MarkBarPanel, MentionPanel,
    MurmureHandler, MurmureRelay, Notice, PageVisibility, SearchBox, SpaceList, coverage,
};
use ambre_murmure::commands::{
    COMMANDS, COPY_TEXT, DELETE, EDIT, KEYS, MARK_ALL_READ, MARK_UNREAD, MessageTarget,
    NEXT_ACTIVITY, PALETTE, PREVIOUS_ACTIVITY, PendingEdit, REACT, REPLY, SEARCH, Writer, key_list,
};
use ambre_murmure::message::{
    CatalogueEntry, ChannelMatches, DirectMessageEvent, HeldReply, HeldSend, MemberView, MessageAt,
    MessageDraft, MessageView, MurmureCommand, MurmureEvent, Presence, SearchMatch, SpaceCommand,
    SpaceEvent,
};
use ambre_murmure::message::{LogicCommand, UiEvent};
use ambre_murmure::{MessageRef, MurmurePane, Region};
use leptos::prelude::*;
use wasm_bindgen_test::*;

/// The events the relay reads, as the launcher relays them minus any
/// document event: Murmure reads none.
type Event = UiEvent;

wasm_bindgen_test_configure!(run_in_browser);

/// A relay that holds one direct channel, shown in the pane.
fn shown_direct() -> (Owner, MurmureRelay, Id<Channel>) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let direct = Id::<Channel>::mint();
    relay.absorb(&Event::MemberChannels {
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
    let lost = Event::Channel {
        channel_id: direct,
        event: MurmureEvent::Left,
    };
    assert!(relay.absorb(&lost).is_empty());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::ChannelList);
    assert!(!relay.sidebar.with_untracked(|s| s.is_member(direct)));

    relay.absorb(&Event::MemberChannels {
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

/// The drafts of a channel lost go with it. Those of a channel that only
/// failed to open stay, and a sign-out takes every one (`MURMURE-215`,
/// `MURMURE-216`).
#[wasm_bindgen_test]
fn a_lost_channel_and_a_sign_out_take_their_drafts() {
    use ambre_murmure::composer::DraftOf;
    let (_owner, relay, direct) = shown_direct();
    let (failed, left, deleted) = (
        Id::<Channel>::mint(),
        Id::<Channel>::mint(),
        Id::<Channel>::mint(),
    );
    let shelf = relay.drafts();
    shelf.load(Id::<User>::mint(), Vec::new());
    let draft = MessageDraft {
        text: "kept".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    for channel in [direct, failed, left, deleted] {
        shelf.put(DraftOf::Channel(channel), Some(draft.clone()));
    }
    let thread_of = DraftOf::Thread(left, Id::<Message>::mint());
    shelf.put(thread_of, Some(draft.clone()));
    relay.absorb(&Event::Channel {
        channel_id: direct,
        event: MurmureEvent::Left,
    });
    relay.absorb(&Event::Channel {
        channel_id: failed,
        event: MurmureEvent::OpenFailed {
            message: "unreachable".to_owned(),
        },
    });
    relay.absorb(&Event::Space {
        space_id: Id::<Space>::mint(),
        event: SpaceEvent::ChannelLeft { channel_id: left },
    });
    relay.absorb(&Event::Space {
        space_id: Id::<Space>::mint(),
        event: SpaceEvent::ChannelDeleted {
            channel_id: deleted,
        },
    });
    assert_eq!(shelf.get(DraftOf::Channel(direct)), None);
    assert_eq!(shelf.get(DraftOf::Channel(left)), None);
    assert_eq!(shelf.get(DraftOf::Channel(deleted)), None);
    assert_eq!(shelf.get(thread_of), None, "a thread goes with its channel");
    assert_eq!(shelf.get(DraftOf::Channel(failed)), Some(draft));

    relay.absorb(&Event::Rest {
        id: Some(ambre_bus::next_request()),
        evt: RestEvent::LoggedOut,
    });
    assert_eq!(shelf.get(DraftOf::Channel(failed)), None);
}

/// The root a reply on a copy sought opens its thread in the channel shown,
/// and moves no pane once the user went elsewhere (`MURMURE-545`).
#[wasm_bindgen_test]
fn a_root_found_opens_its_thread_in_the_channel_shown() {
    let (_owner, relay, direct) = shown_direct();
    let root = MessageRef {
        segment: Id::<Segment>::mint(),
        message: Id::<Message>::mint(),
    };
    let found = |channel_id| Event::Channel {
        channel_id,
        event: MurmureEvent::RootFound {
            thread: Id::<Document>::mint(),
            segment: root.segment,
            root: root.message,
        },
    };
    assert!(relay.absorb(&found(Id::<Channel>::mint())).is_empty());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::channel(direct));
    assert!(relay.absorb(&found(direct)).is_empty());
    assert_eq!(
        relay.pane.get_untracked(),
        MurmurePane::Thread {
            channel: direct,
            root
        }
    );
}

/// The text of each toast the Murmure notices raise for `events`, in the
/// order the host draws them.
fn toasts_for(events: &[UiEvent]) -> Vec<String> {
    use wasm_bindgen::JsCast;

    let toasts = ambre_chrome::Toasts::new();
    for event in events {
        ambre_murmure::chrome::notify_event(&toasts, event);
    }
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let handle = leptos::mount::mount_to(
        mount,
        move || view! { <ambre_chrome::ToastHost toasts=toasts /> },
    );
    let shown = root.query_selector_all(".ambre-toast").unwrap();
    let texts = (0..shown.length())
        .map(|i| {
            shown
                .item(i)
                .and_then(|t| t.text_content())
                .unwrap_or_default()
        })
        .collect();
    drop(handle);
    root.remove();
    texts
}

/// A root the seek could not reach reaches the user as a toast, and moves
/// no pane (`MURMURE-548`).
#[wasm_bindgen_test]
fn a_root_lost_tells_the_user_the_thread_cannot_open() {
    let (_owner, relay, direct) = shown_direct();
    let lost = Event::Channel {
        channel_id: direct,
        event: MurmureEvent::RootLost {
            thread: Id::<Document>::mint(),
        },
    };
    assert!(relay.absorb(&lost).is_empty());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::channel(direct));
    let left = Event::Channel {
        channel_id: direct,
        event: MurmureEvent::Left,
    };
    let texts = toasts_for(&[lost, left]);
    assert_eq!(texts.len(), 1, "only the lost root is told: {texts:?}");
    assert!(
        texts[0].contains(ambre_murmure::chrome::ROOT_LOST),
        "{texts:?}"
    );
}

/// A channel that could not open is told, the pane goes back to the channel
/// list, and the sidebar keeps it. The next selection opens it anew
/// (`MURMURE-549`).
#[wasm_bindgen_test]
fn a_channel_that_cannot_open_is_told_and_opens_anew() {
    let (_owner, relay, direct) = shown_direct();
    let failed = Event::Channel {
        channel_id: direct,
        event: MurmureEvent::OpenFailed {
            message: "the relay refused it".to_owned(),
        },
    };
    assert!(relay.absorb(&failed).is_empty());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::ChannelList);
    assert!(relay.sidebar.with_untracked(|s| s.is_member(direct)));
    assert!(
        matches!(relay.select(direct), Some(LogicCommand::OpenChannel { channel_id }) if channel_id == direct),
        "the channel opens anew"
    );
    let texts = toasts_for(&[failed]);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].contains(ambre_murmure::chrome::CHANNEL_NOT_OPENED),
        "{texts:?}"
    );
    assert!(!texts[0].contains("refused"), "the cause stays out of it");
}

/// The files dropped on the channel's canvas wait in the relay, in the order
/// they landed, until the channel's region takes them once (`MURMURE-132`).
#[wasm_bindgen_test]
fn dropped_files_wait_for_the_region_and_are_taken_once() {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let file = |name: &str| ambre_murmure::uploads::PickedFile {
        name: name.to_owned(),
        media_type: ambre_core::MediaType::from("image/png"),
        data: vec![1].into(),
    };
    relay.drop_file(file("a.png"));
    relay.drop_file(file("b.png"));
    let names: Vec<String> = relay.take_files().into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["a.png", "b.png"]);
    assert!(relay.take_files().is_empty(), "taken once");
}

/// A space that could not open is told, and the next listing naming it
/// opens it anew, once (`MURMURE-550`).
#[wasm_bindgen_test]
fn a_space_that_cannot_open_is_told_and_opens_anew() {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let space = Id::<Space>::mint();
    let listing = Event::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: Id::<Channel>::mint(),
            place: ChannelPlace::Public(space),
        }],
        spaces: Vec::new(),
        names: Vec::new(),
        peers: Vec::new(),
    };
    let opens = |commands: Vec<LogicCommand>| {
        commands
            .iter()
            .filter(|c| matches!(c, LogicCommand::OpenSpace { space_id } if *space_id == space))
            .count()
    };
    assert_eq!(opens(relay.absorb(&listing)), 1);
    let failed = Event::Space {
        space_id: space,
        event: SpaceEvent::OpenFailed {
            message: "the relay refused it".to_owned(),
        },
    };
    assert!(relay.absorb(&failed).is_empty());
    assert_eq!(opens(relay.absorb(&listing)), 1, "opened anew");
    assert_eq!(opens(relay.absorb(&listing)), 0, "once");
    let texts = toasts_for(&[failed]);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].contains(ambre_murmure::chrome::SPACE_NOT_OPENED),
        "{texts:?}"
    );
    assert!(!texts[0].contains("refused"), "the cause stays out of it");
}

/// A channel change that failed is told, without its cause (`MURMURE-580`).
#[wasm_bindgen_test]
fn a_channel_change_that_failed_is_told() {
    let failed = Event::Space {
        space_id: Id::<Space>::mint(),
        event: SpaceEvent::ChannelFailed {
            channel_id: Id::<Channel>::mint(),
            message: "the relay refused it".to_owned(),
        },
    };
    let texts = toasts_for(&[failed]);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].contains(ambre_murmure::chrome::CHANNEL_NOT_CHANGED),
        "{texts:?}"
    );
    assert!(!texts[0].contains("refused"), "the cause stays out of it");
}

/// The name an open channel learns is the one its header reads
/// (`MURMURE-428`).
#[wasm_bindgen_test]
fn a_channel_named_shows_its_name_in_the_header() {
    let (_owner, relay, direct) = shown_direct();
    let named = Event::Channel {
        channel_id: direct,
        event: MurmureEvent::Named {
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Ada").unwrap()),
        },
    };
    assert!(relay.absorb(&named).is_empty());
    assert_eq!(
        relay
            .channel_name()
            .get_untracked()
            .as_ref()
            .map(ambre_core::Validated::as_str),
        Some("Ada")
    );
}

/// A relay with two private channels and a direct message, each named by
/// the listing: `[budget, zeta, Budget review]` in sidebar order.
fn three_named() -> (Owner, MurmureRelay, [Id<Channel>; 3]) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let space = Id::<Space>::mint();
    let ids = [
        Id::<Channel>::mint(),
        Id::<Channel>::mint(),
        Id::<Channel>::mint(),
    ];
    let [budget, zeta, direct] = ids;
    let member = |channel_id, place| MemberChannel { channel_id, place };
    let name = |channel_id, name: &str| ChannelName {
        channel_id,
        name: ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
    };
    relay.absorb(&Event::MemberChannels {
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
    let keys: Vec<PaletteKey> = entries.iter().map(|e| e.key).collect();
    assert_eq!(
        keys,
        [budget, zeta, direct].map(PaletteKey::Channel),
        "{entries:?}"
    );
    assert_eq!(entries[2].hint, Some("Direct message"));

    let shown = |query| -> Vec<String> {
        ambre_chrome::palette_rows(&entries, COMMANDS, query, &Locale::En)
            .into_iter()
            .map(|row| match row {
                PaletteRow::Entry(e) => e.label.clone(),
                PaletteRow::Command(c) => c.id.as_str().to_owned(),
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
    let opened = relay.choose(PaletteKey::Channel(zeta));
    assert!(
        matches!(opened, Some(LogicCommand::OpenChannel { channel_id }) if channel_id == zeta),
        "{opened:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(zeta));
}

/// A relay showing `general`, a public channel of a space whose members are
/// this user, Bob and Carol. Returns the space's members in that order.
fn a_space_with_members() -> (Owner, MurmureRelay, Id<Channel>, [Id<User>; 3]) {
    a_space_with(
        Id::<Channel>::mint(),
        [Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint()],
    )
}

/// [`a_space_with_members`], with the channel and the members it names.
fn a_space_with(
    general: Id<Channel>,
    users: [Id<User>; 3],
) -> (Owner, MurmureRelay, Id<Channel>, [Id<User>; 3]) {
    let owner = Owner::new();
    owner.set();
    let relay = MurmureRelay::new();
    let space = Id::<Space>::mint();
    relay.absorb(&Event::Rest {
        id: Some(ambre_bus::next_request()),
        evt: RestEvent::SessionReady {
            user_id: users[0],
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
            email: ambre_bus::Email::from("alice@test.local"),
        },
    });
    relay.absorb(&Event::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: general,
            place: ChannelPlace::Public(space),
        }],
        spaces: vec![space],
        names: Vec::new(),
        peers: Vec::new(),
    });
    relay.absorb(&Event::Space {
        space_id: space,
        event: SpaceEvent::Catalogue {
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Acme").unwrap()),
            role: Role::Viewer,
            categories: Vec::new(),
            channels: vec![CatalogueEntry {
                channel_id: general,
                name: ambre_core::Validated::<ambre_core::Name>::parse("general").unwrap(),
                topic: None,
                category: None,
                position: ambre_bus::Rank::FIRST,
            }],
        },
    });
    let member = |user_id, name: &str| MemberView {
        user_id,
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
        presence: Presence::Offline,
    };
    relay.absorb(&Event::Space {
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

/// The page's refresh, which a hidden tab still runs, ticks the shown
/// channel beside every space. With no channel shown it ticks none
/// (`MURMURE-521`).
#[wasm_bindgen_test]
fn the_refresh_ticks_the_shown_channel_as_well_as_every_space() {
    let (_owner, relay, general, _) = a_space_with_members();
    let now = Instant::<Authored>::new(1_700_000_000_000);
    let refresh = relay.refresh_presence(now);
    assert!(
        matches!(
            refresh[..],
            [
                LogicCommand::Space { .. },
                LogicCommand::Channel {
                    channel_id,
                    command: MurmureCommand::Tick { now: at },
                },
            ] if channel_id == general && at == now
        ),
        "{refresh:?}"
    );
    relay.pane.set(MurmurePane::ChannelList);
    let refresh = relay.refresh_presence(now);
    assert!(
        matches!(refresh[..], [LogicCommand::Space { .. }]),
        "{refresh:?}"
    );
}

/// After the channels, the palette lists the other members of the space of
/// the shown channel, and the query keeps those whose name matches. This
/// user is not among them (`MURMURE-452`).
#[wasm_bindgen_test]
fn the_palette_lists_the_other_members_of_the_space_after_the_channels() {
    let (_owner, relay, general, [_, bob, carol]) = a_space_with_members();
    let entries = relay.palette_entries().get_untracked();
    let keys: Vec<PaletteKey> = entries.iter().map(|e| e.key).collect();
    assert_eq!(
        keys,
        [
            PaletteKey::Channel(general),
            PaletteKey::Member(bob),
            PaletteKey::Member(carol),
        ],
        "{entries:?}"
    );
    assert_eq!(entries[1].hint, Some("Member"));
    let shown: Vec<String> = ambre_chrome::palette_rows(&entries, COMMANDS, "bob", &Locale::En)
        .into_iter()
        .map(|row| match row {
            PaletteRow::Entry(e) => e.label.clone(),
            PaletteRow::Command(c) => c.id.as_str().to_owned(),
        })
        .collect();
    assert_eq!(shown, ["Bob"]);
}

/// A channel entry shows its channel, even beside a member whose id holds
/// the same sixteen bytes. The entry's key says which of the two it names,
/// and no key is fetched for the member.
#[wasm_bindgen_test]
fn a_channel_entry_shows_its_channel_beside_a_member_of_the_same_bytes() {
    let general = Id::<Channel>::mint();
    let twin = Id::<User>::from_bytes(*general.as_bytes());
    let (_owner, relay, _, _) =
        a_space_with(general, [Id::<User>::mint(), twin, Id::<User>::mint()]);
    relay.pane.set(MurmurePane::ChannelList);
    let entry = relay
        .palette_entries()
        .get_untracked()
        .into_iter()
        .find(|entry| entry.label == "general")
        .expect("the channel is listed");
    let chosen = relay.choose(entry.key);
    assert!(chosen.is_none(), "the channel is open already: {chosen:?}");
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(general));
}

/// Choosing a member fetches their key and opens nothing yet: the key is
/// checked against the pin first. The direct message the relay answers
/// then shows, and opens nothing the Logic thread did not (`MURMURE-455`).
#[wasm_bindgen_test]
fn choosing_a_member_fetches_their_key_then_shows_the_direct_message() {
    let (_owner, relay, general, [_, bob, _]) = a_space_with_members();
    let fetched = relay.choose(PaletteKey::Member(bob));
    assert!(
        matches!(
            fetched,
            Some(LogicCommand::Rest { cmd: RestCommand::FetchMemberKey { user_id }, .. })
                if user_id == bob
        ),
        "{fetched:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(general));
    assert_eq!(relay.member_name(bob).unwrap().as_str(), "Bob");

    let direct = Id::<Channel>::mint();
    let after = relay.absorb(&Event::DirectMessage {
        requested: Id::<Channel>::mint(),
        event: DirectMessageEvent::Opened { channel_id: direct },
    });
    assert!(
        matches!(after[..], [LogicCommand::ListMemberChannels]),
        "{after:?}"
    );
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(direct));
    relay.absorb(&Event::MemberChannels {
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
    let other = Id::<Space>::mint();
    let (board, direct, dave) = (
        Id::<Channel>::mint(),
        Id::<Channel>::mint(),
        Id::<User>::mint(),
    );
    let member = |channel_id, place| MemberChannel { channel_id, place };
    relay.absorb(&Event::MemberChannels {
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
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
        presence: Presence::Offline,
    };
    relay.absorb(&Event::Space {
        space_id: other,
        event: SpaceEvent::Members {
            members: vec![view(bob, "Bob"), view(dave, "Dave")],
        },
    });
    let offered = |channel| -> Vec<Id<User>> {
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
    assert!(relay.choose(PaletteKey::Member(bob)).is_some());
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
    let board = Id::<Channel>::mint();
    relay.absorb(&Event::MemberChannels {
        channels: vec![MemberChannel {
            channel_id: board,
            place: ChannelPlace::Private(space),
        }],
        spaces: vec![space],
        names: Vec::new(),
        peers: Vec::new(),
    });
    relay.select(board);
    let offered = || -> Vec<Id<User>> {
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
        display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Member").unwrap(),
        email: ambre_bus::Email::new(String::new()),
        role: Role::Editor,
        standing: MemberStanding::Verified,
    };
    relay.absorb(&Event::Channel {
        channel_id: board,
        event: MurmureEvent::Members {
            members: vec![info(bob)],
        },
    });
    assert_eq!(offered(), [carol], "bob holds the channel");
    assert!(relay.invite(bob).is_none(), "no key for a member");
    assert!(relay.invite(carol).is_some());

    relay.absorb(&Event::Channel {
        channel_id: board,
        event: MurmureEvent::Left,
    });
    relay.absorb(&Event::MemberChannels {
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

/// Let the effects an action scheduled run before the DOM is read. A dialog
/// opens and closes two effects away from its cause: the relay field, then
/// the `open` signal. A single `tick` may land between them.
async fn settle() {
    for _ in 0..2 {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        });
        wasm_bindgen_futures::JsFuture::from(wait).await.unwrap();
    }
}

/// The next native `close` of `dialog`. Listen before the click that closes
/// it: the event is a task of its own, which a `settle` may not wait for.
fn next_close(dialog: &web_sys::HtmlDialogElement) -> wasm_bindgen_futures::JsFuture {
    let closed = js_sys::Promise::new(&mut |resolve, _| {
        dialog
            .add_event_listener_with_callback("close", &resolve)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(closed)
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
        active: ambre_bus::Activity::Active,
        unread: None,
        mentions: None,
    };
    relay.absorb(&Event::ChannelActivity {
        channels: vec![active(budget), active(direct)],
    });
    let (handler, sent) = handler(relay);

    assert!(matches!(
        handler.on_key(NEXT_ACTIVITY, ambre_chrome::KeyTarget::Canvas),
        KeyRoute::Run(actions) if actions.is_empty()
    ));
    assert_eq!(relay.pane.get_untracked().shown_channel(), Some(budget));
    handler.on_key(NEXT_ACTIVITY, ambre_chrome::KeyTarget::Canvas);
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
        handler.on_key(MARK_ALL_READ, ambre_chrome::KeyTarget::TextField),
        KeyRoute::Yield
    ));
    assert!(sent.get_untracked().is_empty());

    handler.on_key(MARK_ALL_READ, ambre_chrome::KeyTarget::Canvas);
    assert!(matches!(
        sent.get_untracked()[..],
        [LogicCommand::MarkAllRead]
    ));
    assert!(matches!(
        handler.on_key(PALETTE, ambre_chrome::KeyTarget::TextField),
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

    let segment = Id::<Segment>::mint();
    let message = Id::<Message>::mint();
    let target = MessageTarget {
        channel: direct,
        at: MessageAt::Segment(segment),
        message,
        author: Id::<User>::mint(),
        text: "hello".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        writer: Writer::Member,
        deleted: ambre_murmure::message::Deletion::Live,
        pressed: None,
        canvas: ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT,
        from_thread: None,
        call: None,
    };
    relay.target.set(Some(target.clone()));
    assert_eq!(
        menu_labels(relay),
        [
            "Reply in thread",
            "Add reaction",
            "Mark unread",
            "Copy text"
        ]
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

/// The member's own message offers its edit. The action sends nothing and
/// hands the message to the channel's region, which takes it once
/// (`MURMURE-203`). A reply goes to the thread's region (`MURMURE-526`).
#[wasm_bindgen_test]
fn an_edit_is_handed_to_the_region_and_sends_nothing() {
    let (_owner, relay, direct) = shown_direct();
    let me = Id::<User>::mint();
    relay.absorb(&Event::Rest {
        id: Some(ambre_bus::next_request()),
        evt: RestEvent::SessionReady {
            user_id: me,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
            email: ambre_bus::Email::from("alice@test.local"),
        },
    });
    let (handler, sent) = handler(relay);
    let target = MessageTarget {
        channel: direct,
        at: MessageAt::Segment(Id::<Segment>::mint()),
        message: Id::<Message>::mint(),
        author: me,
        text: "typo".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        writer: Writer::Member,
        deleted: ambre_murmure::message::Deletion::Live,
        pressed: None,
        canvas: ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT,
        from_thread: None,
        call: None,
    };
    relay.target.set(Some(target.clone()));
    assert!(menu_labels(relay).contains(&"Edit message".to_owned()));
    assert!(handler.on_command(EDIT).is_empty());
    assert!(sent.get_untracked().is_empty(), "the edit sends nothing");
    let asked = PendingEdit {
        channel: direct,
        at: target.at,
        message: target.message,
        text: "typo".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    assert_eq!(
        relay.take_edit(Region::Thread),
        None,
        "the thread's region leaves it"
    );
    assert_eq!(relay.take_edit(Region::Channel), Some(asked.clone()));
    assert_eq!(relay.take_edit(Region::Channel), None, "taken once");

    // A reply goes to the thread's region alone (`MURMURE-526`).
    let reply = PendingEdit {
        at: MessageAt::Thread(Id::<Message>::mint()),
        ..asked
    };
    relay.editing.set(Some(reply.clone()));
    assert_eq!(
        relay.take_edit(Region::Channel),
        None,
        "the channel's region leaves it"
    );
    assert_eq!(relay.take_edit(Region::Thread), Some(reply));
}

/// The member's own message offers its deletion, last. The action sends
/// nothing and opens the dialog. Cancel and a close of the dialog send
/// nothing, and Delete sends the deletion, stamped by the view
/// (`MURMURE-512`).
#[wasm_bindgen_test]
async fn a_deletion_is_sent_only_once_the_dialog_confirms_it() {
    use wasm_bindgen::JsCast;

    let (_owner, relay, direct) = shown_direct();
    let me = Id::<User>::mint();
    relay.absorb(&Event::Rest {
        id: Some(ambre_bus::next_request()),
        evt: RestEvent::SessionReady {
            user_id: me,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
            email: ambre_bus::Email::from("alice@test.local"),
        },
    });
    let (handler, sent) = handler(relay);
    let segment = Id::<Segment>::mint();
    let message = Id::<Message>::mint();
    let target = MessageTarget {
        channel: direct,
        at: MessageAt::Segment(segment),
        message,
        author: me,
        text: "oops".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        writer: Writer::Member,
        deleted: ambre_murmure::message::Deletion::Live,
        pressed: None,
        canvas: ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT,
        from_thread: None,
        call: None,
    };
    let other = MessageTarget {
        author: Id::<User>::mint(),
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
    let now = Instant::<Authored>::new(1_700_000_000_000);
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
    settle().await;
    assert!(dialog.open(), "the action asks first");
    assert_eq!(relay.target.get_untracked(), None, "the menu closed");
    assert!(
        sent.get_untracked().is_empty(),
        "nothing sent before Delete"
    );
    let closed = next_close(&dialog);
    click("murmure-delete-cancel");
    closed.await.unwrap();
    assert!(!dialog.open());
    assert_eq!(relay.deleting.get_untracked(), None);
    assert!(sent.get_untracked().is_empty(), "a cancel sends nothing");

    relay.target.set(Some(target.clone()));
    handler.on_command(DELETE);
    settle().await;
    assert!(dialog.open());
    // Escape closes the dialog natively, as this close does. Its event is
    // queued as a task, after the close returns.
    let closed = next_close(&dialog);
    dialog.close();
    closed.await.unwrap();
    assert_eq!(relay.deleting.get_untracked(), None, "a close cancels");
    assert!(sent.get_untracked().is_empty(), "a close sends nothing");

    relay.target.set(Some(target));
    handler.on_command(DELETE);
    settle().await;
    let closed = next_close(&dialog);
    click("murmure-delete-confirm");
    closed.await.unwrap();
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

/// The keys command opens the list, a row per chord of the table and per
/// key of a conversation, and sends nothing. The chord of the box in a
/// thread is one of them. Close closes it (`MURMURE-540`, `MURMURE-543`).
#[wasm_bindgen_test]
async fn the_keys_command_opens_the_list_of_every_key() {
    use wasm_bindgen::JsCast;

    let (_owner, relay, _) = shown_direct();
    let (handler, sent) = handler(relay);
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let _handle = leptos::mount::mount_to(mount, move || view! { <KeyList relay=relay /> });
    let dialog: web_sys::HtmlDialogElement = root
        .query_selector("dialog")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(!dialog.open());

    let found = COMMANDS.iter().find(|c| c.id == KEYS).unwrap();
    assert!(found.keybindings.is_empty(), "the palette reaches it");
    assert!(handler.on_command(KEYS).is_empty());
    leptos::task::tick().await;
    assert!(dialog.open());
    assert!(sent.get_untracked().is_empty(), "the list sends nothing");
    let rows = root.query_selector_all("tbody tr").unwrap();
    assert_eq!(rows.length() as usize, key_list(&Locale::En).len());
    let cells: Vec<String> = (0..rows.length())
        .map(|i| rows.item(i).unwrap().text_content().unwrap_or_default())
        .collect();
    assert_eq!(cells[0], "Ctrl+KCommand paletteAnywhere");
    let also = "Alt+EnterAlso send the reply to the channelThe composer of a thread";
    assert!(cells.iter().any(|row| row == also), "{cells:?}");

    let close = root.query_selector(".murmure-keys-close").unwrap().unwrap();
    close.dyn_into::<web_sys::HtmlElement>().unwrap().click();
    leptos::task::tick().await;
    assert!(!dialog.open());
    assert_eq!(relay.keys.get_untracked(), ambre_chrome::Openness::Closed);
    root.remove();
}

/// The react action opens the picker, and a pick sends the reaction to the
/// message, stamped by the view (`MURMURE-140`). The search narrows the
/// grid. A pick for a composer sends nothing and waits for its region,
/// which takes it once (`MURMURE-139`).
#[wasm_bindgen_test]
async fn a_pick_reacts_to_the_message_or_goes_to_the_composer() {
    use wasm_bindgen::JsCast;

    let (_owner, relay, direct) = shown_direct();
    let (handler, sent) = handler(relay);
    let segment = Id::<Segment>::mint();
    let message = Id::<Message>::mint();
    relay.target.set(Some(MessageTarget {
        channel: direct,
        at: MessageAt::Segment(segment),
        message,
        author: Id::<User>::mint(),
        text: "ship it".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
        writer: Writer::Member,
        deleted: ambre_murmure::message::Deletion::Live,
        pressed: None,
        canvas: ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT,
        from_thread: None,
        call: None,
    }));
    let send = handler.send;
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let now = Instant::<Authored>::new(1_700_000_000_000);
    let _handle = leptos::mount::mount_to(mount, move || {
        view! { <EmojiPicker relay=relay send=send now=Callback::new(move |()| now) /> }
    });
    let dialog: web_sys::HtmlDialogElement = root
        .query_selector("dialog")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let picks = || root.query_selector_all(".murmure-emoji-pick").unwrap();
    let performance = web_sys::window().unwrap().performance().unwrap();
    let fetched = || performance.get_entries_by_type("resource").length();
    let before = fetched();

    assert!(handler.on_command(REACT).is_empty());
    settle().await;
    assert!(dialog.open(), "the action opens the picker");
    let title = root.query_selector("h2").unwrap().unwrap().text_content();
    assert_eq!(title.as_deref(), Some("Add a reaction"));
    let search: web_sys::HtmlInputElement = root
        .query_selector(".murmure-emoji-search")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    search.set_value("rocket");
    search
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    assert_eq!(picks().length(), 1, "the search narrows the grid");
    let closed = next_close(&dialog);
    let rocket: web_sys::Element = picks().item(0).unwrap().dyn_into().unwrap();
    assert_eq!(
        rocket.get_attribute("aria-label").as_deref(),
        Some("rocket")
    );
    rocket.dyn_into::<web_sys::HtmlElement>().unwrap().click();
    closed.await.unwrap();
    assert!(matches!(
        &sent.get_untracked()[..],
        [LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::React { at, message: m, emoji, now: t },
        }] if *channel_id == direct
            && *at == MessageAt::Segment(segment)
            && *m == message
            && emoji.as_str() == "\u{1F680}"
            && *t == now
    ));

    relay.ask_emoji(Region::Thread);
    settle().await;
    assert!(dialog.open());
    assert_eq!(search.value(), "", "the search starts blank");
    let closed = next_close(&dialog);
    picks()
        .item(0)
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    closed.await.unwrap();
    assert_eq!(
        sent.get_untracked().len(),
        1,
        "a composer pick sends nothing"
    );
    assert_eq!(relay.take_emoji(Region::Channel), None, "not the channel's");
    assert!(relay.take_emoji(Region::Thread).is_some());
    assert_eq!(relay.take_emoji(Region::Thread), None, "taken once");
    // The table is compiled in and the grid is text: the picker loads
    // nothing (`MURMURE-141`).
    let dialog_html = dialog.inner_html();
    assert!(!dialog_html.contains("<img") && !dialog_html.contains("url("));
    assert_eq!(fetched(), before, "the picker fetched nothing");
    root.remove();
}

/// What a search of `query` found: `budget` holds one match in the one
/// segment of two it read, and `zeta` holds none.
fn found(query: &str, budget: Id<Channel>, zeta: Id<Channel>) -> UiEvent {
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
    Event::MessageSearch {
        query: ambre_core::Validated::<ambre_core::SearchQuery>::parse(query.to_owned())
            .expect("a word"),
        channels: vec![
            ChannelMatches {
                channel_id: budget,
                covered: vec![segment],
                listed: ambre_core::Count::<ambre_murmure::message::Segments>::new(2),
                matches: vec![SearchMatch { segment, message }],
            },
            ChannelMatches {
                channel_id: zeta,
                covered: vec![Id::<Segment>::mint()],
                listed: ambre_core::Count::<ambre_murmure::message::Segments>::new(1),
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
    assert_eq!(binding.keybindings, [KeyBinding::primary(KeyChar::F)]);
    let (handler, sent) = handler(relay);
    assert!(matches!(
        handler.on_key(SEARCH, ambre_chrome::KeyTarget::TextField),
        KeyRoute::Run(actions) if actions.is_empty()
    ));
    assert_eq!(
        relay.search.get_untracked().open,
        ambre_chrome::Openness::Open
    );
    assert!(sent.get_untracked().is_empty(), "opening asks nothing");

    let asked = relay.search_for("bud".to_owned());
    assert!(
        matches!(asked, Some(LogicCommand::SearchMessages { ref query }) if query.as_str() == "bud")
    );
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
    assert_eq!(
        groups[0].name.as_ref().map(ambre_core::Validated::as_str),
        Some("budget")
    );
    let texts: Vec<&str> = groups[0].matches.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts, ["the budget for May"]);
    assert_eq!(groups[0].coverage, "1 of 2 segments");
    // The debug forms count the matches and measure the query: no message
    // text and no typed word shows.
    for (shown, length) in [
        (format!("{:?}", groups[0]), "matches: 1"),
        (
            format!("{:?}", relay.search.get_untracked()),
            "query: ByteCount(6)",
        ),
    ] {
        assert!(!shown.contains("May") && !shown.contains("bud"), "{shown}");
        assert!(shown.contains(length), "{shown}");
    }
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
    relay.absorb(&found("bud", budget, zeta));
    relay.absorb(&found("budget", budget, zeta));
    assert!(relay.search_groups().get_untracked().is_empty());
}

fn news(channel_id: Id<Channel>, mention: ambre_bus::MentionState) -> UiEvent {
    Event::NewMessage {
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
        segment: Id::<Segment>::mint(),
        message: Id::<Message>::mint(),
    };
    let event = Event::NewMessage {
        channel_id: direct,
        mention: ambre_bus::MentionState::Unmentioned,
        message: Some((at.segment, at.message)),
    };
    let notice = relay
        .notice(&event, PageVisibility::Visible)
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
    let closed = relay
        .notice(
            &news(direct, ambre_bus::MentionState::Unmentioned),
            PageVisibility::Visible,
        )
        .expect("notifies");
    assert_eq!(closed.at, None);
    assert!(relay.open_at(direct, closed.at).is_none());
    assert_eq!(relay.pane.get_untracked(), MurmurePane::channel(direct));
}

/// The debug form of a notice names the length of its title, never the
/// channel or the member the title names.
#[wasm_bindgen_test]
fn a_notice_debugs_its_title_length_alone() {
    let notice = Notice {
        channel_id: Id::<Channel>::mint(),
        at: None,
        title: "New message from Secretary".to_owned(),
    };
    let shown = format!("{notice:?}");
    assert!(!shown.contains("Secretary"), "{shown}");
    assert!(shown.contains("title: ByteCount(26)"), "{shown}");
}

/// A click on a notice opens its own channel and asks nothing of another
/// one, so no other read cursor moves (`MURMURE-508`).
#[wasm_bindgen_test]
fn a_notice_asks_nothing_of_another_channel() {
    let (_owner, relay, general, _) = a_space_with_members();
    let other = Id::<Channel>::mint();
    relay.absorb(&Event::MemberChannels {
        channels: vec![
            MemberChannel {
                channel_id: general,
                place: ChannelPlace::Public(Id::<Space>::mint()),
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
        segment: Id::<Segment>::mint(),
        message: Id::<Message>::mint(),
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
    let direct = Id::<Channel>::mint();
    relay.absorb(&Event::MemberChannels {
        channels: vec![
            MemberChannel {
                channel_id: general,
                place: ChannelPlace::Public(Id::<Space>::mint()),
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
            names: vec![ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap()],
        }],
    });
    relay.pane.set(MurmurePane::ChannelList);

    assert_eq!(
        relay.notice(
            &news(general, ambre_bus::MentionState::Unmentioned),
            PageVisibility::Visible
        ),
        None
    );
    let mention = relay
        .notice(
            &news(general, ambre_bus::MentionState::Mentioned),
            PageVisibility::Visible,
        )
        .expect("a mention notifies");
    assert_eq!(mention.channel_id, general);
    assert_eq!(mention.title, "You were mentioned in general");
    for named in [
        ambre_bus::MentionState::Unmentioned,
        ambre_bus::MentionState::Mentioned,
    ] {
        let message = relay
            .notice(&news(direct, named), PageVisibility::Visible)
            .expect("a direct message notifies");
        assert_eq!(message.channel_id, direct);
        assert_eq!(message.title, "New message from Bob");
    }
    assert_eq!(
        relay.notice(&Event::UpdateRequired, PageVisibility::Visible),
        None
    );
}

/// The channel the user is reading notifies nothing. On a hidden page
/// nobody reads it, and it notifies (`MURMURE-146`).
#[wasm_bindgen_test]
fn the_channel_being_read_notifies_nothing_until_the_page_is_hidden() {
    let (_owner, relay, direct) = shown_direct();
    assert_eq!(
        relay.notice(
            &news(direct, ambre_bus::MentionState::Mentioned),
            PageVisibility::Visible
        ),
        None
    );
    assert!(
        relay
            .notice(
                &news(direct, ambre_bus::MentionState::Mentioned),
                PageVisibility::Hidden
            )
            .is_some()
    );
    relay.pane.set(MurmurePane::ChannelList);
    assert!(
        relay
            .notice(
                &news(direct, ambre_bus::MentionState::Unmentioned),
                PageVisibility::Visible
            )
            .is_some()
    );
}

/// The `@` list shows its entries as options, the chosen one selected. A
/// press on an entry picks it for its own region alone, and only that
/// region closes the list (`MURMURE-555`).
#[wasm_bindgen_test]
async fn the_mention_list_shows_its_entries_and_a_press_picks_one() {
    use ambre_murmure::mention_list::{MentionEntry, MentionList};
    use wasm_bindgen::JsCast;

    let (_owner, relay, _) = shown_direct();
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let _handle = leptos::mount::mount_to(mount, move || view! { <MentionPanel relay=relay /> });
    assert!(root.query_selector("[role=listbox]").unwrap().is_none());

    let entries = ["@channel", "@Bob", "@Carol"]
        .map(str::to_owned)
        .into_iter()
        .collect();
    let list = MentionList {
        entries,
        chosen: Pos::<MentionEntry>::ZERO,
        at: Point::new(Px::<View>::new(10.0), Px::<View>::new(20.0)),
        region: Region::Channel,
    };
    relay.show_mentions(Region::Channel, Some(list));
    leptos::task::tick().await;
    let options = root.query_selector_all("[role=option]").unwrap();
    assert_eq!(options.length(), 3);
    let selected: Vec<Option<String>> = (0..3)
        .map(|i| {
            let option: web_sys::Element = options.item(i).unwrap().dyn_into().unwrap();
            option.get_attribute("aria-selected")
        })
        .collect();
    let expected = ["true", "false", "false"].map(|s| Some(s.to_owned()));
    assert_eq!(selected, expected.to_vec());
    let bob = options.item(1).unwrap();
    assert_eq!(bob.text_content().as_deref(), Some("@Bob"));

    let press = web_sys::MouseEvent::new("mousedown").unwrap();
    bob.dispatch_event(&press).unwrap();
    assert_eq!(
        relay.take_pick(Region::Thread),
        None,
        "the thread picked nothing"
    );
    assert_eq!(
        relay.take_pick(Region::Channel),
        Some(Pos::<MentionEntry>::new(1))
    );
    assert_eq!(
        relay.take_pick(Region::Channel),
        None,
        "a pick is taken once"
    );

    relay.show_mentions(Region::Thread, None);
    leptos::task::tick().await;
    assert!(root.query_selector("[role=listbox]").unwrap().is_some());
    relay.show_mentions(Region::Channel, None);
    leptos::task::tick().await;
    assert!(root.query_selector("[role=listbox]").unwrap().is_none());
    root.remove();
}

/// The format bar shows the Murmure marks alone, each pressed as its style
/// covers the selection. A press goes to its own region alone, and only
/// that region closes the bar (`MURMURE-432`).
#[wasm_bindgen_test]
async fn the_format_bar_shows_the_murmure_marks_and_a_press_toggles_one() {
    use ambre_murmure::markers::{Cover, MarkBar};
    use ambre_murmure::message::TextStyle;
    use wasm_bindgen::JsCast;

    let (_owner, relay, _) = shown_direct();
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let _handle = leptos::mount::mount_to(mount, move || view! { <MarkBarPanel relay=relay /> });
    assert!(root.query_selector("[role=toolbar]").unwrap().is_none());

    let bar = MarkBar {
        controls: vec![
            (TextStyle::Bold, Cover::Whole),
            (TextStyle::Italic, Cover::Part),
            (TextStyle::Code, Cover::None),
        ],
        at: Point::new(Px::<View>::new(10.0), Px::<View>::new(20.0)),
        region: Region::Channel,
    };
    relay.show_bar(Region::Channel, Some(bar));
    leptos::task::tick().await;
    let buttons = root.query_selector_all("[role=toolbar] button").unwrap();
    let shown: Vec<(String, String)> = (0..buttons.length())
        .map(|i| {
            let button: web_sys::Element = buttons.item(i).unwrap().dyn_into().unwrap();
            let read = |name: &str| button.get_attribute(name).unwrap_or_default();
            (read("aria-label"), read("aria-pressed"))
        })
        .collect();
    let expected = [
        ("Bold", "true"),
        ("Italic", "mixed"),
        ("Inline code", "false"),
    ]
    .map(|(label, pressed)| (label.to_owned(), pressed.to_owned()));
    assert_eq!(shown, expected.to_vec());

    let italic = buttons.item(1).unwrap();
    italic
        .dispatch_event(&web_sys::MouseEvent::new("click").unwrap())
        .unwrap();
    assert_eq!(
        relay.take_press(Region::Thread),
        None,
        "the thread pressed nothing"
    );
    assert_eq!(relay.take_press(Region::Channel), Some(TextStyle::Italic));
    assert_eq!(
        relay.take_press(Region::Channel),
        None,
        "a press is taken once"
    );

    relay.show_bar(Region::Thread, None);
    leptos::task::tick().await;
    assert!(root.query_selector("[role=toolbar]").unwrap().is_some());
    relay.show_bar(Region::Channel, None);
    leptos::task::tick().await;
    assert!(root.query_selector("[role=toolbar]").unwrap().is_none());
    root.remove();
}

/// The `@channel` question names the members it notifies and sends nothing
/// before Send (`MURMURE-559`). Send sends the held draft once, confirmed
/// (`MURMURE-557`). Cancel hands it to its region alone (`MURMURE-558`).
#[wasm_bindgen_test]
async fn the_at_channel_question_sends_on_send_and_gives_back_on_cancel() {
    use wasm_bindgen::JsCast;
    let (_owner, relay, direct) = shown_direct();
    let sent = RwSignal::new(Vec::<LogicCommand>::new());
    let send = Callback::new(move |command| sent.update(|all| all.push(command)));
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let now = Instant::<Authored>::new(1_700_000_000_000);
    let _handle = leptos::mount::mount_to(mount, move || {
        view! { <ChannelMentionConfirm relay=relay send=send now=Callback::new(move |()| now) /> }
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
    let draft = MessageDraft {
        text: "@channel hi".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    let ask = |reply: Option<HeldReply>| Event::Channel {
        channel_id: direct,
        event: MurmureEvent::ConfirmChannelMention {
            members: ambre_core::Count::<ambre_core::Members>::new(12),
            held: HeldSend {
                draft: draft.clone(),
                attachments: Vec::new(),
                reply,
            },
        },
    };

    // A reply asks, and Cancel gives it to the thread's region alone.
    let root_id = Id::<Message>::mint();
    relay.absorb(&ask(Some(HeldReply {
        root: root_id,
        also_in_channel: ambre_murmure::message::ReplyEcho::AlsoInChannel,
    })));
    settle().await;
    assert!(dialog.open(), "the question opens");
    let title = root.query_selector("h2").unwrap().unwrap().text_content();
    assert_eq!(
        title.as_deref(),
        Some("Notify all 12 members of this channel?")
    );
    assert!(sent.get_untracked().is_empty(), "nothing sent before Send");
    let closed = next_close(&dialog);
    click("murmure-all-cancel");
    closed.await.unwrap();
    assert!(!dialog.open());
    assert!(sent.get_untracked().is_empty(), "a cancel sends nothing");
    assert_eq!(
        relay.take_returned(Region::Channel),
        None,
        "the channel takes no reply"
    );
    let back = relay
        .take_returned(Region::Thread)
        .expect("the thread takes it");
    assert_eq!(back.held.draft, draft);
    assert_eq!(relay.take_returned(Region::Thread), None, "taken once");

    // A message asks, and Send sends it once, confirmed.
    relay.absorb(&ask(None));
    settle().await;
    assert!(dialog.open());
    let closed = next_close(&dialog);
    click("murmure-all-confirm");
    closed.await.unwrap();
    assert!(!dialog.open());
    assert!(matches!(
        &sent.get_untracked()[..],
        [LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::SendMessage { draft: d, now: t, confirmed: ambre_murmure::message::Confirmation::Confirmed, .. },
        }] if *channel_id == direct && *d == draft && *t == now
    ));
    assert_eq!(
        relay.take_returned(Region::Channel),
        None,
        "a send gives nothing back"
    );
    root.remove();
}

use wasm_bindgen::JsCast as _;

/// The space list of a sidebar, mounted in the page. `sent` holds what its
/// controls sent the Logic thread, in order.
struct Spaces {
    root: web_sys::Element,
    relay: MurmureRelay,
    sent: RwSignal<Vec<LogicCommand>>,
    _handle: Box<dyn std::any::Any>,
}

impl Spaces {
    const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

    fn mount(owner: &Owner) -> Self {
        owner.set();
        let relay = MurmureRelay::new();
        let sent = RwSignal::new(Vec::<LogicCommand>::new());
        let send = Callback::new(move |command| sent.update(|s| s.push(command)));
        let root = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&root).unwrap();
        let mount: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
        let handle = leptos::mount::mount_to(mount, move || {
            view! { <SpaceList relay=relay send=send now=Callback::new(|()| Spaces::NOW) /> }
        });
        Self {
            root,
            relay,
            sent,
            _handle: Box::new(handle),
        }
    }

    /// The catalogue of `space`, named `name`, with the user as `role`.
    async fn catalogue(&self, space: Id<Space>, name: &str, role: Role) {
        self.relay.absorb(&Event::Space {
            space_id: space,
            event: SpaceEvent::Catalogue {
                name: Some(ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap()),
                role,
                categories: Vec::new(),
                channels: Vec::new(),
            },
        });
        settle().await;
    }

    fn find(&self, selector: &str) -> Option<web_sys::HtmlElement> {
        self.root
            .query_selector(selector)
            .unwrap()
            .map(|el| el.dyn_into().unwrap())
    }

    fn names(&self) -> Vec<String> {
        let list = self.root.query_selector_all(".murmure-space-name").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.text_content())
            .collect()
    }

    /// Opens the control of `class`, types `text` and submits it.
    async fn submit(&self, class: &str, text: &str) {
        self.find(&format!(".{class}-toggle")).unwrap().click();
        settle().await;
        let field: web_sys::HtmlInputElement = self
            .find(&format!(".{class}-field"))
            .unwrap()
            .dyn_into()
            .unwrap();
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        self.find(&format!(".{class} button[type=submit]"))
            .unwrap()
            .click();
        settle().await;
    }

    fn take(&self) -> Vec<LogicCommand> {
        self.sent.try_update(std::mem::take).unwrap()
    }
}

impl Drop for Spaces {
    fn drop(&mut self) {
        self.root.remove();
    }
}

/// The sidebar offers a signed-in user a control that creates a space, and
/// the new space shows once the Logic thread lists it (`MURMURE-583`). A
/// blank name sends nothing and keeps the form open.
#[wasm_bindgen_test]
async fn a_space_created_from_the_sidebar_shows_in_it() {
    let owner = Owner::new();
    let spaces = Spaces::mount(&owner);
    assert!(spaces.names().is_empty());

    spaces.submit("murmure-new-space", "   ").await;
    assert!(spaces.take().is_empty(), "a blank name creates nothing");
    assert!(spaces.find(".murmure-new-space-field").is_some());

    let field: web_sys::HtmlInputElement = spaces
        .find(".murmure-new-space-field")
        .unwrap()
        .dyn_into()
        .unwrap();
    field.set_value("Acme");
    field
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    spaces
        .find(".murmure-new-space button[type=submit]")
        .unwrap()
        .click();
    settle().await;
    let sent = spaces.take();
    let (space_id, name, now) = match sent.as_slice() {
        [
            LogicCommand::CreateSpace {
                space_id,
                name,
                now,
            },
        ] => Some((*space_id, name.clone(), *now)),
        _ => None,
    }
    .expect("one space created");
    assert_eq!((name.as_str(), now), ("Acme", Spaces::NOW));
    assert!(
        spaces.find(".murmure-new-space-field").is_none(),
        "a sent form closes"
    );

    // The Logic thread shows the new space at once, its creator the owner.
    spaces.catalogue(space_id, "Acme", Role::Owner).await;
    assert_eq!(spaces.names(), ["Acme"]);
}

/// A space offers its owner and its editors a control that creates a public
/// channel, last in its catalogue. The channel opens once created
/// (`MURMURE-584`). A viewer is offered no such control.
#[wasm_bindgen_test]
async fn an_admin_creates_a_public_channel_that_opens_once_created() {
    let owner = Owner::new();
    let spaces = Spaces::mount(&owner);
    let (acme, kept) = (Id::<Space>::mint(), Id::<Space>::mint());
    spaces.catalogue(acme, "Acme", Role::Editor).await;
    spaces.catalogue(kept, "Kept", Role::Viewer).await;
    assert_eq!(spaces.names(), ["Acme", "Kept"]);
    let toggles = spaces
        .root
        .query_selector_all(".murmure-new-channel-toggle")
        .unwrap();
    assert_eq!(toggles.length(), 1, "the viewer's space offers none");

    spaces.submit("murmure-new-channel", "budget").await;
    let sent = spaces.take();
    let (space_id, draft, now) = match sent.as_slice() {
        [
            LogicCommand::Space {
                space_id,
                command: SpaceCommand::CreatePublicChannel { draft, now },
            },
        ] => Some((*space_id, draft.clone(), *now)),
        _ => None,
    }
    .expect("one public channel created");
    assert_eq!((space_id, now), (acme, Spaces::NOW));
    assert_eq!(draft.name.as_str(), "budget");
    assert_eq!(draft.position, ambre_bus::Rank::FIRST);
    // A viewer's space creates nothing, whatever calls the relay.
    assert!(
        spaces
            .relay
            .create_channel(kept, "x".to_owned(), Spaces::NOW)
            .is_none()
    );

    let created = draft.channel_id;
    let answered = spaces.relay.absorb(&Event::Space {
        space_id: acme,
        event: SpaceEvent::ChannelCreated {
            channel_id: created,
        },
    });
    assert!(
        matches!(
            answered.as_slice(),
            [LogicCommand::ListMemberChannels, LogicCommand::OpenChannel { channel_id }]
                if *channel_id == created
        ),
        "{answered:?}"
    );
    assert_eq!(
        spaces.relay.pane.get_untracked(),
        MurmurePane::channel(created)
    );
}

/// A space offers its owner a control that invites another user into it. The
/// invite fetches the key of the address typed, against the space document,
/// for the check of the pin (`MURMURE-585`). An editor is offered none.
#[wasm_bindgen_test]
async fn the_owner_of_a_space_invites_another_user_into_it() {
    let owner = Owner::new();
    let spaces = Spaces::mount(&owner);
    let (acme, shared) = (Id::<Space>::mint(), Id::<Space>::mint());
    spaces.catalogue(acme, "Acme", Role::Owner).await;
    spaces.catalogue(shared, "Shared", Role::Editor).await;
    let toggles = spaces
        .root
        .query_selector_all(".murmure-space-invite-toggle")
        .unwrap();
    assert_eq!(toggles.length(), 1, "the editor's space offers none");

    spaces
        .submit("murmure-space-invite", " bob@dev.local ")
        .await;
    let sent = spaces.take();
    let (document_id, invitee_email) = match sent.as_slice() {
        [
            LogicCommand::Rest {
                cmd:
                    RestCommand::FetchInviteeKey {
                        document_id,
                        invitee_email,
                    },
                ..
            },
        ] => Some((*document_id, invitee_email.clone())),
        _ => None,
    }
    .expect("one key fetched");
    assert_eq!(document_id, acme.document_id());
    assert_eq!(invitee_email, ambre_core::Email::from("bob@dev.local"));
    assert!(
        spaces
            .relay
            .invite_to_space(shared, "bob@dev.local".to_owned())
            .is_none(),
        "an editor invites nobody"
    );
}
