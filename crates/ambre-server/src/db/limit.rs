//! How many rows one query may return.

use ambre_core::{Bounded, Bounds, Defaulted, Refusal, Saturates};

/// The most rows one query returns: never zero, never past 1000 (`SRV-075`).
/// The value is a `Bounded<QueryLimit>`.
///
/// A request names any number, and [`clamped_limit`] brings it into range
/// instead of refusing it: the quantity takes [`Saturates`]. A pass of the
/// relay names its own as a constant, through [`Bounded::of`], which checks
/// it while the crate compiles. The count is a `u32`, the one representation
/// with a `const` door. No wire and no column holds it: the store binds it
/// as an `i64`.
pub enum QueryLimit {}

impl Bounds for QueryLimit {
    type Repr = u32;
    const MIN: u32 = 1;
    const MAX: u32 = 1000;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("QueryLimit");
    const REFUSAL: Refusal = Refusal::new("a query limit lies in 1..=1000, not ", "");
}

/// A request may name any page size, and is served the nearest one.
impl Saturates for QueryLimit {}

/// The page a request that names no limit is served.
impl Defaulted for QueryLimit {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::of::<100>();
}

/// A request's `limit` reads as any integer, clamped (`SRV-075`): zero or
/// below is one row, and past the cap is the cap. A `null` reads as no
/// limit, as an absent field does. A word that is not an integer is still
/// refused. A field takes it through `deserialize_with` with
/// `#[serde(default)]`, since the decode of [`Bounded`] refuses a number out
/// of range.
///
/// # Errors
///
/// The error of the deserializer, on a value that is neither an integer nor
/// `null`.
pub fn clamped_limit<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Bounded<QueryLimit>>, D::Error> {
    <Option<i64> as serde::Deserialize>::deserialize(deserializer)
        .map(|asked| asked.map(Bounded::saturating_from))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that may name a page size, as a JSON caller writes it. The
    /// route's own query is decoded in `routes::sync`.
    #[derive(serde::Deserialize)]
    struct Asked {
        #[serde(default, deserialize_with = "clamped_limit")]
        limit: Option<Bounded<QueryLimit>>,
    }

    #[test]
    fn a_query_limit_is_never_zero_and_never_past_the_cap() {
        for (asked, served) in [
            (i64::MIN, 1),
            (-3, 1),
            (0, 1),
            (1, 1),
            (250, 250),
            (1000, 1000),
            (1001, 1000),
            (70_000, 1000),
            (i64::MAX, 1000),
        ] {
            let text = format!(r#"{{"limit": {asked}}}"#);
            let read: Asked = serde_json::from_str(&text).expect("an integer reads");
            assert_eq!(read.limit.map(Bounded::get), Some(served), "asked {asked}");
        }
        assert_eq!(Bounded::<QueryLimit>::default().get(), 100);
        let absent: Asked = serde_json::from_str("{}").expect("no limit reads");
        assert_eq!(absent.limit, None);
        let null: Asked = serde_json::from_str(r#"{"limit": null}"#).expect("a null reads");
        assert_eq!(null.limit, None);
        assert!(serde_json::from_str::<Asked>(r#"{"limit": "many"}"#).is_err());
    }
}
