//! The JSON read of the worker: every JSON value the relay or the SFU sends
//! decodes through [`read`].
//!
//! The relay sends each struct as a JSON object. `serde_json` also reads a
//! struct from a JSON array, so each struct of an answer linked a second
//! decoder that no answer reaches: about 41 KB of the worker
//! (`ambre-spec-budgets.md` section 5). [`Objects`] wraps the deserializer of
//! `serde_json` and reads every struct, at any depth, as a JSON map. A struct
//! sent as an array is refused, as any value of the wrong JSON type is.
//!
//! Every other read passes through unchanged, with the error and the position
//! of `serde_json`. The wrapper is pure forwarding, with no trait object, so
//! each call stays one static call.

use serde::de::{self, DeserializeSeed, Deserializer, Visitor};

/// Reads one JSON value of `T` from `json`, then refuses anything but white
/// space after it, as `serde_json::from_slice` does.
///
/// # Errors
///
/// The error of `serde_json` for a body that does not decode as `T`,
/// a struct sent as a JSON array included.
pub(crate) fn read_whole<T: de::DeserializeOwned>(
    mut json: serde_json::Deserializer<serde_json::de::SliceRead<'_>>,
) -> Result<T, serde_json::Error> {
    let value = read(&mut json)?;
    json.end()?;
    Ok(value)
}

/// Reads one JSON value of `T` from `json`, every struct as a map. The
/// caller checks the end of the body.
///
/// # Errors
///
/// The error of `serde_json` for a value that does not decode as `T`.
pub(crate) fn read<T: de::DeserializeOwned>(
    json: &mut serde_json::Deserializer<serde_json::de::SliceRead<'_>>,
) -> Result<T, serde_json::Error> {
    T::deserialize(Objects(json))
}

/// A part of a `serde_json` read that hands every struct to a map read.
///
/// It wraps, in turn, the deserializer, each visitor, each seed and each
/// access of a sequence, a map or an enum. So a struct nested at any depth
/// reaches [`Deserializer::deserialize_map`] too.
///
/// The visitor half forwards the calls that `serde_json` makes: the other
/// calls keep their default, which no read of `serde_json` reaches.
struct Objects<T>(T);

/// Forwards a read that takes a visitor alone.
macro_rules! forward_read {
    ($($read:ident)*) => {$(
        fn $read<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
            self.0.$read(Objects(visitor))
        }
    )*};
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Objects<D> {
    type Error = D::Error;

    forward_read! {
        deserialize_any deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32
        deserialize_i64 deserialize_i128 deserialize_u8 deserialize_u16 deserialize_u32
        deserialize_u64 deserialize_u128 deserialize_f32 deserialize_f64 deserialize_char
        deserialize_str deserialize_string deserialize_bytes deserialize_byte_buf
        deserialize_option deserialize_unit deserialize_seq deserialize_map
        deserialize_identifier deserialize_ignored_any
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_unit_struct(name, Objects(visitor))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_newtype_struct(name, Objects(visitor))
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple(len, Objects(visitor))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple_struct(name, len, Objects(visitor))
    }

    /// The one change: a struct reads as a map, so the array read of the
    /// struct's visitor is never built.
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(Objects(visitor))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_enum(name, variants, Objects(visitor))
    }
}

/// Forwards a visit of a value that holds no other value.
macro_rules! forward_visit {
    ($($visit:ident($value:ty))*) => {$(
        fn $visit<E: de::Error>(self, value: $value) -> Result<V::Value, E> {
            self.0.$visit(value)
        }
    )*};
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Objects<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.expecting(f)
    }

    forward_visit! {
        visit_bool(bool) visit_i64(i64) visit_i128(i128) visit_u64(u64) visit_u128(u128)
        visit_f64(f64) visit_str(&str) visit_borrowed_str(&'de str) visit_bytes(&[u8])
        visit_borrowed_bytes(&'de [u8])
    }

    fn visit_none<E: de::Error>(self) -> Result<V::Value, E> {
        self.0.visit_none()
    }

    fn visit_unit<E: de::Error>(self) -> Result<V::Value, E> {
        self.0.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, value: D) -> Result<V::Value, D::Error> {
        self.0.visit_some(Objects(value))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, value: D) -> Result<V::Value, D::Error> {
        self.0.visit_newtype_struct(Objects(value))
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, seq: A) -> Result<V::Value, A::Error> {
        self.0.visit_seq(Objects(seq))
    }

    fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(Objects(map))
    }

    fn visit_enum<A: de::EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        self.0.visit_enum(Objects(data))
    }
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Objects<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, value: D) -> Result<S::Value, D::Error> {
        self.0.deserialize(Objects(value))
    }
}

impl<'de, A: de::SeqAccess<'de>> de::SeqAccess<'de> for Objects<A> {
    type Error = A::Error;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, A::Error> {
        self.0.next_element_seed(Objects(seed))
    }
}

impl<'de, A: de::MapAccess<'de>> de::MapAccess<'de> for Objects<A> {
    type Error = A::Error;

    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, A::Error> {
        self.0.next_key_seed(Objects(seed))
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, A::Error> {
        self.0.next_value_seed(Objects(seed))
    }
}

impl<'de, A: de::EnumAccess<'de>> de::EnumAccess<'de> for Objects<A> {
    type Error = A::Error;
    type Variant = Objects<A::Variant>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self::Variant), A::Error> {
        self.0
            .variant_seed(Objects(seed))
            .map(|(value, variant)| (value, Objects(variant)))
    }
}

impl<'de, A: de::VariantAccess<'de>> de::VariantAccess<'de> for Objects<A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, A::Error> {
        self.0.newtype_variant_seed(Objects(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.tuple_variant(len, Objects(visitor))
    }

    /// A struct variant reads as a map too: `serde_json` reads its body as a
    /// struct, and the newtype read hands that body to the deserializer
    /// above.
    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.newtype_variant_seed(StructBody(visitor))
    }
}

/// The body of a struct variant, read as a map.
struct StructBody<V>(V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for StructBody<V> {
    type Value = V::Value;

    fn deserialize<D: Deserializer<'de>>(self, value: D) -> Result<V::Value, D::Error> {
        value.deserialize_map(Objects(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::read_whole;

    /// The read under test, over the bytes of one body.
    fn from_slice<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
        read_whole(serde_json::Deserializer::from_slice(bytes))
    }
    use proptest::prelude::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct Leaf {
        name: String,
        n: u64,
        flag: bool,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct Name(String);

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    enum Kind {
        Unit,
        Newtype(Leaf),
        Tuple(u8, String),
        Struct { leaf: Leaf, at: i64 },
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct Root {
        leaves: Vec<Leaf>,
        kind: Kind,
        maybe: Option<Leaf>,
        map: BTreeMap<String, i32>,
        pair: (u16, Option<bool>),
        name: Name,
        #[serde(default)]
        absent: Option<u8>,
        big: u128,
        ratio: f64,
    }

    fn leaf() -> impl Strategy<Value = Leaf> {
        (".{0,8}", any::<u64>(), any::<bool>()).prop_map(|(name, n, flag)| Leaf { name, n, flag })
    }

    fn kind() -> impl Strategy<Value = Kind> {
        prop_oneof![
            Just(Kind::Unit),
            leaf().prop_map(Kind::Newtype),
            (any::<u8>(), ".{0,8}").prop_map(|(a, b)| Kind::Tuple(a, b)),
            (leaf(), any::<i64>()).prop_map(|(leaf, at)| Kind::Struct { leaf, at }),
        ]
    }

    fn root() -> impl Strategy<Value = Root> {
        (
            prop::collection::vec(leaf(), 0..4),
            kind(),
            prop::option::of(leaf()),
            prop::collection::btree_map(".{0,6}", any::<i32>(), 0..4),
            (any::<u16>(), prop::option::of(any::<bool>())),
            ".{0,8}",
            any::<u128>(),
            // Quarters print and parse back exactly, so the value is its own oracle.
            any::<i32>().prop_map(|n| f64::from(n) / 4.0),
        )
            .prop_map(|(leaves, kind, maybe, map, pair, name, big, ratio)| Root {
                leaves,
                kind,
                maybe,
                map,
                pair,
                name: Name(name),
                absent: None,
                big,
                ratio,
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// The oracle is `serde_json::from_slice`: on every value the relay
        /// can send, the read through the map-only wrapper agrees with it.
        #[test]
        fn a_value_sent_as_objects_reads_as_serde_json_reads_it(value in root()) {
            let bytes = serde_json::to_vec(&value).expect("a value encodes");
            let ours: Root = from_slice(&bytes).expect("the wrapper reads it");
            let theirs: Root = serde_json::from_slice(&bytes).expect("serde_json reads it");
            prop_assert_eq!(&ours, &theirs);
            prop_assert_eq!(ours, value);
        }

        /// A body cut short fails in both reads, with the same message and
        /// the same position.
        #[test]
        fn a_truncated_body_fails_with_the_error_of_serde_json(value in root(), cut in any::<prop::sample::Index>()) {
            let bytes = serde_json::to_vec(&value).expect("a value encodes");
            let short = &bytes[..cut.index(bytes.len())];
            let ours = from_slice::<Root>(short).map(|_| ()).map_err(|e| e.to_string());
            let theirs = serde_json::from_slice::<Root>(short).map(|_| ()).map_err(|e| e.to_string());
            prop_assert_eq!(ours, theirs);
        }
    }

    #[test]
    fn a_struct_sent_as_an_array_is_refused() {
        let array = br#"["a", 1, true]"#;
        assert!(serde_json::from_slice::<Leaf>(array).is_ok());
        let refused = from_slice::<Leaf>(array).expect_err("an array is no struct");
        assert!(
            refused
                .to_string()
                .starts_with("invalid type: sequence, expected struct Leaf")
        );
    }

    #[test]
    fn a_nested_struct_sent_as_an_array_is_refused() {
        let nested = br#"{"name":"x","leaves":[["a",1,true]]}"#;
        #[derive(Deserialize, Debug)]
        struct Outer {
            #[expect(dead_code, reason = "the read alone is under test")]
            leaves: Vec<Leaf>,
        }
        assert!(serde_json::from_slice::<Outer>(nested).is_ok());
        assert!(from_slice::<Outer>(nested).is_err());
    }

    #[test]
    fn a_struct_variant_reads_from_an_object() {
        let value = Kind::Struct {
            leaf: Leaf {
                name: "x".into(),
                n: 7,
                flag: false,
            },
            at: -3,
        };
        let bytes = serde_json::to_vec(&value).expect("a value encodes");
        assert_eq!(from_slice::<Kind>(&bytes).expect("it reads"), value);
        assert!(from_slice::<Kind>(br#"{"Struct":[{"name":"x","n":7,"flag":false},-3]}"#).is_err());
    }

    /// The worker reads a word it does not know as an update to install from
    /// the start of this message (`SYNC-152`).
    #[test]
    fn an_unknown_variant_keeps_the_message_of_serde_json() {
        let ours = from_slice::<Kind>(br#""Later""#).expect_err("no such variant");
        let theirs = serde_json::from_slice::<Kind>(br#""Later""#).expect_err("no such variant");
        assert_eq!(ours.to_string(), theirs.to_string());
        assert!(ours.to_string().starts_with("unknown variant `"));
    }

    #[test]
    fn characters_after_the_value_are_refused() {
        let ours = from_slice::<Name>(br#""x" y"#).expect_err("trailing characters");
        let theirs = serde_json::from_slice::<Name>(br#""x" y"#).expect_err("trailing characters");
        assert_eq!(ours.to_string(), theirs.to_string());
    }
}
