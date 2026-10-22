//! The names a board is addressed by: its root containers and the keys of each
//! map it holds (`ambre-spec-toile-schema.md` section 3).
//!
//! Each map schema is one closed enum, so a key of one map does not compile
//! where a key of another is expected. The spelling is the one the board
//! stores, and a board therefore reads the same as before.

use loro::{LoroDoc, LoroMap, LoroMovableList};

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
    /// The root a board before version 3 kept its deck in. Only the `2 -> 3`
    /// step reads it, and no build writes it.
    pub(crate) enum LegacyRoot {
        Presentation = "presentation",
    }
}

ambre_core::crdt_names! {
    /// The two sub-containers of the legacy `presentation` map.
    pub(crate) enum LegacyDeckKey: FieldKey {
        Order = "order",
        Config = "config",
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

ambre_core::crdt_names! {
    /// The fields of one element map. Each kind writes a part of the set.
    pub(crate) enum ElementKey: FieldKey {
        /// The element type tag, as `ElementKind::type_tag` spells it.
        Type = "type",
        /// The world AABB, top-left, before the rotation.
        X = "x",
        Y = "y",
        W = "w",
        H = "h",
        /// The rotation in radians, about the AABB centre.
        Rot = "rot",
        /// The advisory lock.
        Locked = "locked",
        /// The whole style, one register (`TOILE-018`).
        Style = "style",
        /// The link target. An absent key means no link.
        Link = "link",
        /// The frame title.
        Name = "name",
        /// The body of a sticky or a text element, a text container
        /// (`TOILE-030`).
        Body = "text",
        /// The shape kind tag of a shape.
        Shape = "shape",
        /// The SVG path data of a vector node.
        Path = "path",
        /// The raw stroke samples of a drawing.
        Points = "points",
        /// The key of an image in `media`.
        Hash = "hash",
        /// The address of an embed.
        Url = "url",
        /// The reference of a nested document.
        DocId = "doc_id",
        DocType = "doc_type",
        /// The auto-layout register of a frame. An absent key means free
        /// positioning.
        Autolayout = "autolayout",
        /// The member list of a group or a frame.
        Children = "children",
        /// The speaker notes of a frame, a text container.
        Notes = "notes",
    }
}

ambre_core::crdt_names! {
    /// The fields of one connector map.
    pub(crate) enum ConnectorKey: FieldKey {
        /// The endpoint element ids.
        From = "from",
        To = "to",
        /// The endpoint anchors.
        FromAnch = "from_anch",
        ToAnch = "to_anch",
        PathType = "path_type",
        /// The two end caps.
        ArrowStart = "arrow_start",
        ArrowEnd = "arrow_end",
        /// The edge caption, a text container seeded at birth (`TOILE-047`).
        Label = "label",
        /// The whole style, one register.
        Style = "style",
    }
}

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

/// A root list of the board: `layers` or `presentation_order`.
#[must_use]
pub(crate) fn root_list(doc: &LoroDoc, root: Root) -> LoroMovableList {
    doc.get_movable_list(root.as_str())
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
