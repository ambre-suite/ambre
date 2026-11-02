//! How many rows one query may return.

use ambre_core::{Bounded, Bounds, Defaulted, Refusal};

/// The most rows one query returns: never zero, never past 1000 (`SRV-075`).
/// The value is a `Bounded<QueryLimit>`.
///
/// A request names any number, and [`clamped_limit`] brings it into range instead
/// of refusing it. A pass of the relay names its own as a constant, through
/// [`Bounded::saturating`]. The count is a `u32`, the one representation
/// with a `const` door. No wire and no column holds it: the store binds it
/// as an `i64`.
pub enum QueryLimit {}

impl Bounds for QueryLimit {
    type Repr = u32;
    const MIN: u32 = 1;
    const MAX: u32 = 1000;
    const NAME: &'static str = "QueryLimit";
    const REFUSAL: Refusal = Refusal::new("a query limit lies in 1..=1000, not ", "");
}

/// The page a request that names no limit is served.
impl Defaulted for QueryLimit {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::saturating(100);
}

/// A request's `limit` reads as any integer, clamped (`SRV-075`): zero or
/// below is one row, and past the cap is the cap. A word that is not an
/// integer is still refused. A field takes it through `deserialize_with`,
/// since the decode of [`Bounded`] refuses a number out of range.
///
/// # Errors
///
/// The error of the deserializer, on a value that is not an integer.
pub fn clamped_limit<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Bounded<QueryLimit>>, D::Error> {
    <i64 as serde::Deserialize>::deserialize(deserializer)
        .map(|asked| Some(Bounded::saturating_from(asked)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The query of a request that may name a page size.
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
        assert_eq!(Bounded::<QueryLimit>::saturating(0), Bounded::MIN);
        assert_eq!(Bounded::<QueryLimit>::saturating(u32::MAX), Bounded::MAX);
        assert_eq!(Bounded::<QueryLimit>::default().get(), 100);
        let absent: Asked = serde_json::from_str("{}").expect("no limit reads");
        assert_eq!(absent.limit, None);
        assert!(serde_json::from_str::<Asked>(r#"{"limit": "many"}"#).is_err());
    }

    /// The query string form, as the route reads it: `?limit=5000` serves
    /// the cap.
    #[test]
    fn a_query_string_limit_is_clamped_like_a_json_one() {
        let read = |uri: &'static str| {
            axum::extract::Query::<Asked>::try_from_uri(&axum::http::Uri::from_static(uri))
                .map(|query| query.0.limit)
                .ok()
        };
        assert_eq!(read("/deltas?limit=5000"), Some(Some(Bounded::MAX)));
        assert_eq!(read("/deltas?limit=-2"), Some(Some(Bounded::MIN)));
        assert_eq!(read("/deltas"), Some(None));
        assert_eq!(read("/deltas?limit=many"), None);
    }
}
