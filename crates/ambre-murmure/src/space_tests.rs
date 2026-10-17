//! The space document and its catalogue
//! (`features/ambre-spec-murmure-spaces.md`).

use ambre_bus::{SealedChannelKey, Timestamp, Topic};
use ambre_core::{CategoryId, ChannelId, SpaceId};
use loro::{LoroDoc, LoroValue};

use crate::schema::{HEADER, HEADER_KIND, HEADER_SPACE, read_version};
use crate::{ChannelState, ChannelType, Listing, MurmureError, SCHEMA_VERSION, SpaceState};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn channel(channel_type: ChannelType, name: &str) -> ChannelState {
    ChannelState::create(ChannelId::new(), channel_type, Some(name), NOW).expect("create")
}

fn sealed(byte: u8) -> SealedChannelKey {
    SealedChannelKey::from_bytes(vec![byte; 72])
}

fn topic(text: &str) -> Topic {
    Topic::new(text).expect("a topic")
}

/// A listing with no topic and no category.
fn listing(key: SealedChannelKey) -> Listing<'static> {
    Listing {
        topic: None,
        category: None,
        position: 3,
        key,
    }
}

fn root_names(doc: &LoroDoc) -> Vec<String> {
    let LoroValue::Map(roots) = doc.get_value() else {
        return Vec::new();
    };
    let mut names: Vec<String> = roots.keys().cloned().collect();
    names.sort();
    names
}

// --- MURMURE-031, MURMURE-035, MURMURE-036 ---

/// A space is one document: a header naming the space, its name, and no
/// member list. Its members are the relay's rows for that document.
#[test]
fn a_new_space_is_one_document_that_holds_no_member() {
    let space_id = SpaceId::new();
    let space = SpaceState::create(space_id, "Acme", NOW).expect("create");
    assert_eq!(space.document_id(), space_id.document_id());
    assert_eq!(space.name().as_deref(), Some("Acme"));
    let header = space.doc().get_map(HEADER);
    assert_eq!(
        crate::schema::string(&header, HEADER_KIND).as_deref(),
        Some("space")
    );
    assert_eq!(
        crate::schema::string(&header, HEADER_SPACE),
        Some(space_id.to_string())
    );
    assert_eq!(read_version(space.doc()), Ok(SCHEMA_VERSION));
    assert_eq!(root_names(space.doc()), ["header", "meta", "space"]);
    assert!(space.catalogue().is_empty());
}

#[test]
fn a_space_reopens_from_its_payloads() {
    let space_id = SpaceId::new();
    let mut space = SpaceState::create(space_id, "Acme", NOW)
        .expect("create")
        .identify();
    let first = space.export_all().expect("export");
    let public = channel(ChannelType::Public, "general");
    let delta = space
        .list_channel(&public, &listing(sealed(1)))
        .expect("list");
    let reopened = SpaceState::open(space_id, &[&first, &delta]).expect("open");
    assert_eq!(reopened.name().as_deref(), Some("Acme"));
    assert_eq!(reopened.catalogue(), space.catalogue());
}

#[test]
fn a_document_of_another_space_is_refused() {
    let space = SpaceState::create(SpaceId::new(), "Acme", NOW).expect("create");
    let other = SpaceId::new();
    let refused = SpaceState::open(other, &[&space.export_all().expect("export")]);
    assert_eq!(
        refused.err(),
        Some(MurmureError::ForeignSpace {
            document_id: other.document_id(),
            found: space.space_id(),
        })
    );
}

#[test]
fn a_channel_document_is_no_space() {
    let public = channel(ChannelType::Public, "general");
    let space_id = SpaceId::new();
    let refused = SpaceState::open(
        space_id,
        &[&public
            .snapshot(public.meta_id().document_id())
            .expect("snapshot")],
    );
    assert!(matches!(
        refused,
        Err(MurmureError::WrongKind {
            expected: "space",
            ..
        })
    ));
}

// --- MURMURE-037, MURMURE-033, MURMURE-157 ---

/// A listed channel carries its name, topic, category, position and key.
/// The key is the sealed value the Sync worker produced, stored and answered
/// unopened: the engine has no key to open it with.
#[test]
fn a_listed_channel_carries_its_name_topic_category_position_and_key() {
    let mut space = SpaceState::create(SpaceId::new(), "Acme", NOW)
        .expect("create")
        .identify();
    let category = CategoryId::new();
    space.put_category(category, "Teams", 1).expect("category");
    let public = channel(ChannelType::Public, "budget");
    let figures = topic("the weekly figures");
    space
        .list_channel(
            &public,
            &Listing {
                topic: Some(&figures),
                category: Some(category),
                position: 2,
                key: sealed(7),
            },
        )
        .expect("list");

    let [entry] = space.catalogue().try_into().expect("one entry");
    assert_eq!(entry.channel_id, public.channel_id());
    assert_eq!(entry.name, "budget");
    assert_eq!(entry.topic, Some(figures));
    assert_eq!(entry.category, Some(category));
    assert_eq!(entry.position, 2);
    assert_eq!(space.sealed_key(public.channel_id()), Ok(sealed(7)));
    let [category_view] = space.categories().try_into().expect("one category");
    assert_eq!(category_view.name, "Teams");
}

#[test]
fn an_unset_topic_and_category_are_absent_keys() {
    let mut space = SpaceState::create(SpaceId::new(), "Acme", NOW)
        .expect("create")
        .identify();
    let public = channel(ChannelType::Public, "general");
    space
        .list_channel(
            &public,
            &Listing {
                topic: None,
                category: None,
                position: 0,
                key: sealed(1),
            },
        )
        .expect("list");
    let [entry] = space.catalogue().try_into().expect("one entry");
    assert_eq!((entry.topic, entry.category), (None, None));
}

#[test]
fn a_channel_the_catalogue_does_not_list_has_no_key() {
    let space = SpaceState::create(SpaceId::new(), "Acme", NOW).expect("create");
    let absent = ChannelId::new();
    assert_eq!(
        space.sealed_key(absent),
        Err(MurmureError::NotListed(absent))
    );
}

// --- MURMURE-014 ---

/// Every member of the space reads the catalogue, so a private channel or a
/// direct message never enters it, and the refusal writes nothing.
#[test]
fn the_catalogue_refuses_a_private_channel_and_a_direct_message() {
    let mut space = SpaceState::create(SpaceId::new(), "Acme", NOW)
        .expect("create")
        .identify();
    let before = space.version();
    for channel_type in [ChannelType::Private, ChannelType::Direct] {
        let closed = channel(channel_type, "board");
        assert_eq!(
            space.list_channel(&closed, &listing(sealed(1))),
            Err(MurmureError::NotPublic(closed.channel_id()))
        );
    }
    assert_eq!(space.version(), before);
    assert!(space.catalogue().is_empty());
}

// --- MURMURE-038 ---

/// Listing writes the space document and nothing else: no document of the
/// channel moves, and the space holds no member list for it.
#[test]
fn listing_a_channel_writes_the_space_document_alone() {
    let mut space = SpaceState::create(SpaceId::new(), "Acme", NOW)
        .expect("create")
        .identify();
    let public = channel(ChannelType::Public, "general");
    let meta_before = public
        .export_all(public.meta_id().document_id())
        .expect("meta");
    let active_before = public
        .export_all(public.active_id().document_id())
        .expect("active");
    space
        .list_channel(&public, &listing(sealed(1)))
        .expect("list");
    assert_eq!(
        public
            .export_all(public.meta_id().document_id())
            .expect("meta"),
        meta_before
    );
    assert_eq!(
        public
            .export_all(public.active_id().document_id())
            .expect("active"),
        active_before
    );
    assert_eq!(
        root_names(space.doc()),
        ["channels", "header", "meta", "peers", "space"]
    );
}

// --- MURMURE-247 ---

/// Unlisting a deleted channel removes its entry, its sealed key with it,
/// and leaves every other entry. A peer that replays the delta lists it no
/// more. A channel the catalogue does not list writes nothing.
#[test]
fn unlisting_a_channel_removes_its_entry_and_leaves_the_others() {
    let space_id = SpaceId::new();
    let mut space = SpaceState::create(space_id, "Acme", NOW)
        .expect("create")
        .identify();
    let first = space.export_all().expect("export");
    let (gone, kept) = (
        channel(ChannelType::Public, "budget"),
        channel(ChannelType::Public, "general"),
    );
    let listed = [
        space
            .list_channel(&gone, &listing(sealed(1)))
            .expect("list"),
        space
            .list_channel(&kept, &listing(sealed(2)))
            .expect("list"),
    ];

    let delta = space
        .unlist_channel(gone.channel_id())
        .expect("unlist")
        .expect("a delta");
    let [entry] = space.catalogue().try_into().expect("one entry left");
    assert_eq!(entry.channel_id, kept.channel_id());
    assert_eq!(
        space.sealed_key(gone.channel_id()),
        Err(MurmureError::NotListed(gone.channel_id()))
    );
    assert_eq!(space.sealed_key(kept.channel_id()), Ok(sealed(2)));
    let peer = SpaceState::open(space_id, &[&first, &listed[0], &listed[1], &delta]).expect("open");
    assert_eq!(peer.catalogue(), space.catalogue());

    let before = space.version();
    assert_eq!(space.unlist_channel(gone.channel_id()), Ok(None));
    assert_eq!(space.version(), before, "an absent entry writes nothing");
}
