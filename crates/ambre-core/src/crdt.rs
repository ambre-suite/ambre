//! The names a CRDT document is addressed by: its root containers and the keys
//! of its maps.
//!
//! A bare `&str` lets a key of one map stand for a key of another. It also
//! lets a root name stand for a key, and a user's string stand for either. Each application
//! therefore declares its names as closed enums, one per container, with
//! [`crdt_names!`]. A root container name implements [`ContainerName`], a map
//! key implements [`FieldKey`], and the two traits never meet.
//!
//! `ambre-crdt` holds the map handle that takes those keys. This crate names no
//! Loro type itself: `ambre-server` links it, and the server never links a
//! crate that reads a document (`ZK-004`).
//!
//! The spelling stays the one the document stores. A variant names the string
//! and never encodes it again. A document therefore reads the same before and
//! after its names were typed.

/// A closed set of words a document stores: the names of its root
/// containers, the keys of its maps, or the words a value spells.
///
/// [`crdt_names!`] writes the set and its spellings. The reading back is
/// written once, here, for every set.
pub trait CrdtName: Copy + 'static {
    /// Every name of the set, in declaration order. The schema belts walk it,
    /// so a declared name with no line in the frozen shape fails there.
    const ALL: &'static [Self];

    /// The name as the document stores it, for the call that opens the
    /// container or reads the key.
    fn as_str(self) -> &'static str;

    /// The name a stored word spells, or `None` for a word outside the set.
    #[must_use]
    fn parse(word: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|name| name.as_str() == word)
    }
}

/// The name of a root container, which a document opens by name.
pub trait ContainerName: CrdtName {}

/// The name a family of root containers shares, one container per id. The
/// application joins the name and the id into the root name, and only its own
/// constructor does the join.
pub trait ContainerFamily: CrdtName {}

/// A key of one map schema. A handle typed by the schema takes this key and no
/// other, so a key of another map does not compile there.
pub trait FieldKey: CrdtName {}

/// Declares a closed set of CRDT names as an enum, one variant per name, and
/// implements [`CrdtName`] for it.
///
/// `: ContainerName`, `: ContainerFamily` or `: FieldKey` after the enum name
/// implements that trait too.
/// With neither, the enum names words of another kind, a text mark key for
/// example.
///
/// A macro writes the set because its variants differ per set, which no
/// generic type can hold. The calling crate owns the enum, so a set of one
/// schema never stands for a set of another. Everything else is a method of
/// [`CrdtName`].
///
/// The enum also carries an inherent `const fn as_str`, the stored spelling,
/// so a spelling stays a constant expression. A spelling is any constant
/// `&'static str` expression. A literal is the usual case. A name another
/// module owns is spelled through that module, so the two never drift apart.
#[macro_export]
macro_rules! crdt_names {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident $(: $role:ident)? {
            $($(#[$variant_meta:meta])* $variant:ident = $spelling:expr),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $crate::crdt::CrdtName for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling),+
                }
            }
        }

        $(impl $crate::crdt::$role for $name {})?
    };
}

crate::crdt_names! {
    /// The root containers the foundation writes into every document. Each
    /// application's own root set spells these through here.
    pub enum FoundationRoot: ContainerName {
        /// The map every application writes its schema version into
        /// (`ambre-spec-schema-version.md`).
        Meta = "meta",
        /// The map of the peer registry (`AUTHOR-004`).
        Peers = "peers",
    }
}

/// The origin a commit carries, which an undo manager reads to keep a write
/// that no user gesture made off its stack.
///
/// An undo manager excludes an origin by prefix, so no spelling here starts
/// another one. Loro keeps the origin on the local commit event and exports
/// none of it, so a spelling may change without moving a stored byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitOrigin {
    /// A write of the peer registry: the entry is no gesture of the user.
    Peers,
    /// A write opening a document makes, so an undo on a restored document
    /// retracts what its author wrote and never what opening it did.
    Load,
    /// A clock a document stamps on itself, such as the date it was last
    /// modified.
    Stamp,
    /// A comment write, which an undo of the document never retracts once a
    /// teammate may have read it.
    Comment,
    /// A pass that removes for good what an ended session left hidden.
    Sweep,
    /// The scaffolding an edit forces into existence around the value the user
    /// wrote.
    Materialize,
    /// The replay of a step of the undo history (`ambre-spec-undo.md section
    /// 3`): an undo or a redo, which records no step of its own.
    Recorded,
}

impl CommitOrigin {
    /// Every origin, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Peers,
        Self::Load,
        Self::Stamp,
        Self::Comment,
        Self::Sweep,
        Self::Materialize,
        Self::Recorded,
    ];

    /// The spelling the commit carries, for the call that sets or excludes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Peers => "ambre-peers",
            Self::Load => "load",
            Self::Stamp => "stamp",
            Self::Comment => "comment",
            Self::Sweep => "sweep",
            Self::Materialize => "mat",
            Self::Recorded => "recorded",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CrdtName;

    crate::crdt_names! {
        /// Two roots of a test schema.
        enum Root: ContainerName {
            Board = "board",
            Layers = "layers",
        }
    }

    crate::crdt_names! {
        /// A key spelled through another set.
        enum Key: FieldKey {
            Board = Root::Board.as_str(),
            Width = "w",
        }
    }

    #[test]
    fn no_commit_origin_is_a_prefix_of_another() {
        use super::CommitOrigin;
        for a in CommitOrigin::ALL {
            for b in CommitOrigin::ALL {
                if a != b {
                    assert!(
                        !b.as_str().starts_with(a.as_str()),
                        "excluding {a:?} by prefix would also exclude {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_name_reads_back_from_its_spelling_and_a_stranger_does_not() {
        for name in Root::ALL {
            assert_eq!(Root::parse(name.as_str()), Some(*name));
        }
        assert_eq!(Key::parse("board"), Some(Key::Board));
        assert_eq!(Key::Width.as_str(), "w");
        assert_eq!(Root::parse("w"), None);
        assert_eq!(<Key as CrdtName>::as_str(Key::Width), "w");
        assert_eq!(<Root as CrdtName>::as_str(Root::Layers), "layers");
    }

    /// A generic reader names the set by its role trait alone, and reads the
    /// spelling and the set through it.
    #[test]
    fn a_role_trait_reads_the_set_it_bounds() {
        fn first<K: super::FieldKey>() -> Option<&'static str> {
            K::ALL.first().map(|key| key.as_str())
        }
        assert_eq!(first::<Key>(), Some("board"));
    }
}
