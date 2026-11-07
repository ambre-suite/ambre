//! The map handles of a Loro document, typed by the keys of one schema.
//!
//! [`Fields<K>`] wraps a live `LoroMap`, and [`FieldValues<'_, K>`] a map read
//! whole as a deep value. Each takes a key of the schema `K` and no other type.
//! The stored spelling reaches Loro inside the handle and nowhere else.
//!
//! The handles are generic over their key, so one definition serves every
//! application. They live outside `ambre-core` because that crate links no
//! CRDT: the relay links it, and the relay never reads a document (`ZK-004`).
//! `deny.toml` names the crates that may link Loro, this one among them.

#![forbid(unsafe_code)]

use std::marker::PhantomData;

use ambre_core::crdt::FieldKey;

/// A live Loro map, read and written through the keys of the schema
/// `K` alone.
#[derive(Clone, Debug)]
pub struct Fields<K> {
    map: loro::LoroMap,
    schema: PhantomData<fn() -> K>,
}

impl<K: FieldKey> Fields<K> {
    /// Reads `map` as a map of the schema `K`. The caller names the
    /// schema the container holds.
    #[must_use]
    pub fn new(map: loro::LoroMap) -> Self {
        Self {
            map,
            schema: PhantomData,
        }
    }

    /// The map, for a call that takes no key: its id, its length, a
    /// whole read.
    #[must_use]
    pub fn map(&self) -> &loro::LoroMap {
        &self.map
    }

    /// The value or the container under `key`.
    #[must_use]
    pub fn get(&self, key: K) -> Option<loro::ValueOrContainer> {
        self.map.get(key.as_str())
    }

    /// The plain value under `key`, or `None` when the key is absent
    /// or holds a container.
    #[must_use]
    pub fn value(&self, key: K) -> Option<loro::LoroValue> {
        match self.get(key)? {
            loro::ValueOrContainer::Value(value) => Some(value),
            loro::ValueOrContainer::Container(_) => None,
        }
    }

    /// Whether the map holds `key`.
    #[must_use]
    pub fn contains(&self, key: K) -> bool {
        self.get(key).is_some()
    }

    /// Writes `value` under `key`.
    ///
    /// # Errors
    /// Loro's refusal of the write.
    pub fn insert(&self, key: K, value: impl Into<loro::LoroValue>) -> loro::LoroResult<()> {
        self.map.insert(key.as_str(), value)
    }

    /// Writes the new container `child` under `key`, replacing what
    /// the key held.
    ///
    /// # Errors
    /// Loro's refusal of the write.
    pub fn insert_container<C: loro::ContainerTrait>(
        &self,
        key: K,
        child: C,
    ) -> loro::LoroResult<C> {
        self.map.insert_container(key.as_str(), child)
    }

    /// The map under `key`, created when the key holds none. Two
    /// peers creating it at once merge into one map.
    ///
    /// # Errors
    /// Loro's refusal of the write, or a key holding another kind.
    pub fn ensure_mergeable_map(&self, key: K) -> loro::LoroResult<loro::LoroMap> {
        self.map.ensure_mergeable_map(key.as_str())
    }

    /// Removes `key`. Absence is a value, so this is how a property
    /// is cleared.
    ///
    /// # Errors
    /// Loro's refusal of the write.
    pub fn delete(&self, key: K) -> loro::LoroResult<()> {
        self.map.delete(key.as_str())
    }
}

/// A map read whole as a deep value, read through the keys of the
/// schema `K` alone.
#[derive(Clone, Copy, Debug)]
pub struct FieldValues<'a, K> {
    map: &'a loro::LoroMapValue,
    schema: PhantomData<fn() -> K>,
}

impl<'a, K: FieldKey> FieldValues<'a, K> {
    /// Reads `map` as a deep value of the schema `K`.
    #[must_use]
    pub fn new(map: &'a loro::LoroMapValue) -> Self {
        Self {
            map,
            schema: PhantomData,
        }
    }

    /// The deep value, for a read that takes no key.
    #[must_use]
    pub fn map(&self) -> &'a loro::LoroMapValue {
        self.map
    }

    /// The value under `key`.
    #[must_use]
    pub fn get(&self, key: K) -> Option<&'a loro::LoroValue> {
        self.map.get(key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldValues, Fields};
    use loro::{LoroDoc, LoroValue};

    ambre_core::crdt_names! {
        /// Two keys of a test schema, one spelled apart from its variant name.
        enum Key: FieldKey {
            Width = "w",
            Title = "title",
        }
    }

    /// The handle writes under the stored spelling, so a document that a
    /// bare string wrote reads the same through it.
    #[test]
    fn a_key_reaches_loro_as_its_stored_spelling() {
        let doc = LoroDoc::new();
        let fields = Fields::<Key>::new(doc.get_map("board"));
        assert!(fields.insert(Key::Width, 3_i64).is_ok());
        assert_eq!(
            doc.get_map("board")
                .get("w")
                .and_then(|v| v.into_value().ok()),
            Some(LoroValue::I64(3))
        );
        assert!(fields.contains(Key::Width));
        assert!(!fields.contains(Key::Title));
        assert!(fields.delete(Key::Width).is_ok());
        assert_eq!(fields.value(Key::Width), None);
    }

    /// The deep value of a map reads through the same keys as the live map.
    #[test]
    fn a_deep_value_reads_through_the_schema_keys() {
        let doc = LoroDoc::new();
        assert!(doc.get_map("board").insert("title", "plan").is_ok());
        let LoroValue::Map(deep) = doc.get_map("board").get_deep_value() else {
            panic!("a map reads as a map");
        };
        let values = FieldValues::<Key>::new(&deep);
        assert_eq!(values.get(Key::Title), Some(&LoroValue::from("plan")));
        assert_eq!(values.get(Key::Width), None);
    }
}
