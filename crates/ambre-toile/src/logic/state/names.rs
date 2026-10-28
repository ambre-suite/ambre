//! The names a board is addressed by: its root containers and the keys of each
//! map it holds (`ambre-spec-toile-schema.md` section 3).
//!
//! Each map schema is one closed enum, so a key of one map does not compile
//! where a key of another is expected. The spelling is the one the board
//! stores, and a board therefore reads the same as before.

use loro::{LoroDoc, LoroMap};

use ambre_core::crdt::FieldKey;

ambre_core::loro_fields!(pub(crate));

ambre_core::crdt_names! {
    /// The root containers of a board. A root container's identity is its
    /// name, so every peer opens the same one (`TOILE-029`).
    pub(crate) enum Root: ContainerName {
        /// The schema version register.
        Meta = ambre_core::crdt::FoundationRoot::Meta.as_str(),
        /// The board settings, keyed by [`BoardKey`].
        Board = "board",
        /// One element map per element id, keyed by [`ElementKey`].
        Elements = "elements",
        /// One connector map per connector id, keyed by [`ConnectorKey`].
        Connectors = "connectors",
        /// The paint order, one tagged node id per entry.
        Layers = "layers",
        /// One media map per content hash, keyed by [`MediaKey`]. An `image`
        /// element's `hash` field is a key here, and many elements can share
        /// one entry.
        Media = "media",
        /// The slide sequence, one frame id per entry.
        PresentationOrder = "presentation_order",
        /// The presentation settings, keyed by [`ConfigKey`].
        PresentationConfig = "presentation_config",
        /// One reaction per [`ReactionKey`].
        Reactions = "reactions",
        /// One thread map per thread id, keyed by [`ThreadKey`].
        Comments = "comments",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `board` register.
    pub(crate) enum BoardKey: FieldKey {
        BgColor = "bg_color",
        GridSize = "grid_size",
        GridSnap = "grid_snap",
    }
}

/// The fields of an element map and of a connector map. They live in
/// `ambre-bus`, because a copied element carries its fields under them.
pub(crate) use ambre_bus::app::toile::{ConnectorKey, ElementKey};

ambre_core::crdt_names! {
    /// The fields of one media map.
    pub(crate) enum MediaKey: FieldKey {
        W = "w",
        H = "h",
        Mime = "mime",
        Thumb = "thumb",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `presentation_config` register.
    pub(crate) enum ConfigKey: FieldKey {
        TransitionMs = "transition_ms",
        Autoplay = "autoplay",
        Loop = "loop",
    }
}

ambre_core::crdt_names! {
    /// The fields of one comment thread map.
    pub(crate) enum ThreadKey: FieldKey {
        /// The anchor of the thread.
        Anchor = "anchor",
        /// The resolved flag.
        Resolved = "resolved",
        /// The ordered messages, each keyed by [`MessageKey`].
        Messages = "messages",
    }
}

ambre_core::crdt_names! {
    /// The fields of one comment message map.
    pub(crate) enum MessageKey: FieldKey {
        /// The author's canonical `UserId`, rendered.
        Author = "author",
        /// The wall clock. The reader sees it, and nothing orders by it
        /// (`TOILE-FEAT-012`).
        Ts = "ts",
        /// The body, a text container (`TOILE-FEAT-013`).
        Body = "body",
    }
}

/// A root map of the board.
#[must_use]
pub(crate) fn root_map(doc: &LoroDoc, root: Root) -> LoroMap {
    doc.get_map(root.as_str())
}

/// The `board` register.
#[must_use]
pub(crate) fn board(doc: &LoroDoc) -> Fields<BoardKey> {
    Fields::of(root_map(doc, Root::Board))
}

/// The `presentation_config` register.
#[must_use]
pub(crate) fn config(doc: &LoroDoc) -> Fields<ConfigKey> {
    Fields::of(root_map(doc, Root::PresentationConfig))
}

/// The entry `id` of the dictionary `dict`, read as a map of the schema `K`.
/// An absent entry, or one that holds no map, reads as `None`.
#[must_use]
pub(crate) fn entry<K: FieldKey>(dict: &LoroMap, id: &str) -> Option<Fields<K>> {
    dict.get(id)?
        .into_container()
        .ok()?
        .into_map()
        .ok()
        .map(Fields::of)
}

/// The key of one reaction: the target's id and the user's id, joined by `/`.
/// Neither half can carry the separator (`TOILE-229`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ReactionKey(String);

impl ReactionKey {
    /// The key of the reaction `user` placed on `target`.
    #[must_use]
    pub(crate) fn of(target: &crate::logic::model::ElementId, user: &ambre_bus::UserId) -> Self {
        Self(format!("{}{REACTION_SEPARATOR}{user}", target.as_str()))
    }

    /// The key as the board stores it.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The target id a stored key names, or `None` for a key with no
    /// separator.
    #[must_use]
    pub(crate) fn target_of(key: &str) -> Option<&str> {
        key.split_once(REACTION_SEPARATOR).map(|(target, _)| target)
    }
}

/// The separator of a reaction key.
const REACTION_SEPARATOR: char = '/';
