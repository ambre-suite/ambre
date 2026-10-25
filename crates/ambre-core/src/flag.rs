//! A flag: a `bool` that crosses a store, a wire or a public struct, given an
//! `enum` of two named variants (`ai/ambre-rust-style.md`, the `bool` rule).
//!
//! A reader of `Flag::On` sees what the value picks; a reader of `true` does
//! not. The stored bytes and the wire bytes stay those of the `bool`. A field
//! thus moves from `bool` to a flag with no migration and no change to a frame.

/// Defines one two-variant `enum` per item, the variant that was `false`
/// first.
///
/// Each type derives `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`,
/// `PartialOrd`, `Ord`, `Hash` and `Default`, the first variant. It gets
/// `from_bool` and `as_bool` for the one store or wire call that needs the
/// `bool`, and `From<bool>` in that direction only. Serde sees a plain
/// `bool`, so JSON reads `false` or `true` and postcard writes one byte.
///
/// The `@sqlx` arm adds `sqlx::Type`, `Encode` and `Decode` for Postgres as
/// `BOOLEAN`. Only a crate that depends on `sqlx` expands it, so no other
/// crate gains the dependency.
#[macro_export]
macro_rules! define_flag {
    ($(
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(#[$off_meta:meta])* $off:ident,
            $(#[$on_meta:meta])* $on:ident $(,)?
        }
    )+) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(#[$off_meta])*
            #[default]
            $off,
            $(#[$on_meta])*
            $on,
        }

        impl $name {
            /// The variant a stored or wire `bool` reads as.
            #[must_use]
            pub const fn from_bool(value: bool) -> Self {
                if value { Self::$on } else { Self::$off }
            }

            /// The `bool` a store or a wire writes for the variant.
            #[must_use]
            pub const fn as_bool(self) -> bool {
                matches!(self, Self::$on)
            }
        }

        impl ::core::convert::From<bool> for $name {
            fn from(value: bool) -> Self {
                Self::from_bool(value)
            }
        }

        // Serde through core's own paths, so a crate that expands the macro
        // needs no serde dependency of its own.
        impl $crate::index::__private::Serialize for $name {
            fn serialize<S: $crate::index::__private::Serializer>(
                &self,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_bool(self.as_bool())
            }
        }

        impl<'de> $crate::index::__private::Deserialize<'de> for $name {
            fn deserialize<D: $crate::index::__private::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                <bool as $crate::index::__private::Deserialize<'de>>::deserialize(deserializer)
                    .map(Self::from_bool)
            }
        }
    )+};
    (@sqlx $($name:ident),+ $(,)?) => {$(
        impl ::sqlx::Type<::sqlx::Postgres> for $name {
            fn type_info() -> ::sqlx::postgres::PgTypeInfo {
                <bool as ::sqlx::Type<::sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                <bool as ::sqlx::Type<::sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'q> ::sqlx::Encode<'q, ::sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut ::sqlx::postgres::PgArgumentBuffer,
            ) -> ::core::result::Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError> {
                <bool as ::sqlx::Encode<'q, ::sqlx::Postgres>>::encode_by_ref(&self.as_bool(), buf)
            }
        }

        impl<'r> ::sqlx::Decode<'r, ::sqlx::Postgres> for $name {
            fn decode(
                value: ::sqlx::postgres::PgValueRef<'r>,
            ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
                <bool as ::sqlx::Decode<'r, ::sqlx::Postgres>>::decode(value).map(Self::from_bool)
            }
        }
    )+};
}

#[cfg(test)]
mod tests {
    define_flag! {
        /// A test flag.
        pub enum Pin {
            /// The value that was `false`.
            Loose,
            /// The value that was `true`.
            Fixed,
        }
    }

    #[test]
    fn a_flag_reads_false_as_its_first_variant() {
        assert_eq!(Pin::default(), Pin::Loose);
        assert_eq!(Pin::from_bool(false), Pin::Loose);
        assert_eq!(Pin::from(true), Pin::Fixed);
        assert!(Pin::Loose < Pin::Fixed);
        assert!(!Pin::Loose.as_bool());
    }

    #[test]
    fn a_flag_crosses_json_as_a_plain_bool() {
        for (flag, text) in [(Pin::Loose, "false"), (Pin::Fixed, "true")] {
            assert_eq!(serde_json::to_string(&flag).expect("a flag encodes"), text);
            let back: Pin = serde_json::from_str(text).expect("a bool decodes");
            assert_eq!(back, flag);
        }
    }

    #[test]
    fn a_flag_crosses_postcard_as_the_byte_of_its_bool() {
        for (flag, value) in [(Pin::Loose, false), (Pin::Fixed, true)] {
            let bytes = postcard::to_allocvec(&flag).expect("a flag encodes");
            assert_eq!(
                bytes,
                postcard::to_allocvec(&value).expect("a bool encodes")
            );
            let back: Pin = postcard::from_bytes(&bytes).expect("a bool decodes");
            assert_eq!(back, flag);
        }
    }
}
