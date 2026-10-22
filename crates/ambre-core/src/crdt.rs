//! The names a CRDT document is addressed by: its root containers and the keys
//! of its maps.
//!
//! A bare `&str` lets a key of one map stand for a key of another. It also
//! lets a root name stand for a key, and a user's string stand for either. Each application
//! therefore declares its names as closed enums, one per container, with
//! [`crdt_names!`]. A root container name implements [`ContainerName`], a map
//! key implements [`FieldKey`], and the two traits never meet.
//!
//! [`loro_fields!`] writes the map handle that takes those keys. It expands in
//! an application crate, which links Loro. This crate names no Loro type
//! itself: `ambre-server` links it, and the server never decodes a document
//! (`ZK-001`).
//!
//! The spelling stays the one the document stores. A variant names the string
//! and never encodes it again. A document therefore reads the same before and
//! after its names were typed.

/// The name of a root container, which a document opens by name.
pub trait ContainerName: Copy + 'static {
    /// The name as the document stores it, for the call that opens the
    /// container.
    fn as_str(self) -> &'static str;
}

/// The name a family of root containers shares, one container per id. The
/// application joins the name and the id into the root name, and only its own
/// constructor does the join.
pub trait ContainerFamily: Copy + 'static {
    /// The shared part of the name, as the document stores it.
    fn as_str(self) -> &'static str;
}

/// A key of one map schema. A handle typed by the schema takes this key and no
/// other, so a key of another map does not compile there.
pub trait FieldKey: Copy + 'static {
    /// The key as the document stores it, for the call into the map.
    fn as_str(self) -> &'static str;
}

/// Declares a closed set of CRDT names as an enum, one variant per name.
///
/// `: ContainerName`, `: ContainerFamily` or `: FieldKey` after the enum name
/// implements that trait.
/// With neither, the enum names words of another kind, a text mark key for
/// example, and only the inherent items are written.
///
/// Each enum carries `ALL`, every name in declaration order, `as_str`, the
/// stored spelling, and `parse`, its inverse. The schema belts walk `ALL`, so a
/// declared name with no line in the frozen shape fails there.
///
/// A spelling is any constant `&'static str` expression. A literal is the
/// usual case. A name another module owns is spelled through that module, so
/// the two never drift apart.
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

        // A schema whose belt never walks `ALL`, or whose reader never parses
        // a stored word, leaves one of the two unused.
        #[allow(dead_code)]
        impl $name {
            /// Every name of the set, in declaration order.
            $vis const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The name as the document stores it.
            #[must_use]
            $vis const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling),+
                }
            }

            /// The name a stored word spells, or `None` for a word outside
            /// the set.
            #[must_use]
            $vis fn parse(word: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|name| name.as_str() == word)
            }
        }

        $(
            impl $crate::crdt::$role for $name {
                fn as_str(self) -> &'static str {
                    $name::as_str(self)
                }
            }
        )?
    };
}

/// Writes `Fields<K>` and `FieldValues<K>`, the two map handles typed by a
/// schema, into the calling crate.
///
/// `Fields<K>` wraps a live `loro::LoroMap`, and `FieldValues<'_, K>` a map
/// read whole as a deep value. Each takes a key of the schema `K` and no other
/// type. The string reaches Loro inside the handle and nowhere else.
///
/// The handle is written by a macro because no crate every application shares
/// links Loro. One definition, expanded in each application, keeps the copies
/// from drifting. An application that calls only part of the handle leaves the
/// rest unused, so the impls allow dead code.
#[macro_export]
macro_rules! loro_fields {
    ($vis:vis) => {
        /// A live Loro map, read and written through the keys of the schema
        /// `K` alone.
        #[derive(Clone, Debug)]
        $vis struct Fields<K> {
            map: ::loro::LoroMap,
            schema: ::core::marker::PhantomData<fn() -> K>,
        }

        #[allow(dead_code)]
        impl<K: $crate::crdt::FieldKey> Fields<K> {
            /// Reads `map` as a map of the schema `K`. The caller names the
            /// schema the container holds.
            #[must_use]
            $vis fn of(map: ::loro::LoroMap) -> Self {
                Self {
                    map,
                    schema: ::core::marker::PhantomData,
                }
            }

            /// The map, for a call that takes no key: its id, its length, a
            /// whole read.
            #[must_use]
            $vis fn map(&self) -> &::loro::LoroMap {
                &self.map
            }

            /// The value or the container under `key`.
            #[must_use]
            $vis fn get(&self, key: K) -> Option<::loro::ValueOrContainer> {
                self.map.get(key.as_str())
            }

            /// The plain value under `key`, or `None` when the key is absent
            /// or holds a container.
            #[must_use]
            $vis fn value(&self, key: K) -> Option<::loro::LoroValue> {
                match self.get(key)? {
                    ::loro::ValueOrContainer::Value(value) => Some(value),
                    ::loro::ValueOrContainer::Container(_) => None,
                }
            }

            /// Whether the map holds `key`.
            #[must_use]
            $vis fn contains(&self, key: K) -> bool {
                self.get(key).is_some()
            }

            /// Writes `value` under `key`.
            ///
            /// # Errors
            /// Loro's refusal of the write.
            $vis fn insert(
                &self,
                key: K,
                value: impl Into<::loro::LoroValue>,
            ) -> ::loro::LoroResult<()> {
                self.map.insert(key.as_str(), value)
            }

            /// Writes the new container `child` under `key`, replacing what
            /// the key held.
            ///
            /// # Errors
            /// Loro's refusal of the write.
            $vis fn insert_container<C: ::loro::ContainerTrait>(
                &self,
                key: K,
                child: C,
            ) -> ::loro::LoroResult<C> {
                self.map.insert_container(key.as_str(), child)
            }

            /// The map under `key`, created when the key holds none. Two
            /// peers creating it at once merge into one map.
            ///
            /// # Errors
            /// Loro's refusal of the write, or a key holding another kind.
            $vis fn ensure_mergeable_map(&self, key: K) -> ::loro::LoroResult<::loro::LoroMap> {
                self.map.ensure_mergeable_map(key.as_str())
            }

            /// Removes `key`. Absence is a value, so this is how a property
            /// is cleared.
            ///
            /// # Errors
            /// Loro's refusal of the write.
            $vis fn delete(&self, key: K) -> ::loro::LoroResult<()> {
                self.map.delete(key.as_str())
            }
        }

        /// A map read whole as a deep value, read through the keys of the
        /// schema `K` alone.
        #[derive(Clone, Copy, Debug)]
        $vis struct FieldValues<'a, K> {
            map: &'a ::loro::LoroMapValue,
            schema: ::core::marker::PhantomData<fn() -> K>,
        }

        #[allow(dead_code)]
        impl<'a, K: $crate::crdt::FieldKey> FieldValues<'a, K> {
            /// Reads `map` as a deep value of the schema `K`.
            #[must_use]
            $vis fn of(map: &'a ::loro::LoroMapValue) -> Self {
                Self {
                    map,
                    schema: ::core::marker::PhantomData,
                }
            }

            /// The deep value, for a read that takes no key.
            #[must_use]
            $vis fn map(&self) -> &'a ::loro::LoroMapValue {
                self.map
            }

            /// The value under `key`.
            #[must_use]
            $vis fn get(&self, key: K) -> Option<&'a ::loro::LoroValue> {
                self.map.get(key.as_str())
            }
        }
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

#[cfg(test)]
mod tests {
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
    fn a_name_reads_back_from_its_spelling_and_a_stranger_does_not() {
        for name in Root::ALL {
            assert_eq!(Root::parse(name.as_str()), Some(*name));
        }
        assert_eq!(Key::parse("board"), Some(Key::Board));
        assert_eq!(Key::Width.as_str(), "w");
        assert_eq!(Root::parse("w"), None);
        assert_eq!(<Key as crate::crdt::FieldKey>::as_str(Key::Width), "w");
        assert_eq!(
            <Root as crate::crdt::ContainerName>::as_str(Root::Layers),
            "layers"
        );
    }
}
