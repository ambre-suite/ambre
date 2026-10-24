use crate::error::AppError;
use sqlx::{PgPool, Row};

// Query functions are split by domain into sibling modules; each reaches the
// shared imports and the private uuid bridges below via `use super::*`. All
// items are re-globbed so every `db::*` path stays valid.
mod channels;
mod content;
mod documents;
mod limit;
mod members;
mod orgs;
mod read_cursors;
mod users;

pub use channels::*;
pub use content::*;
pub use documents::*;
pub use limit::QueryLimit;
pub use members::*;
pub use orgs::*;
pub use read_cursors::*;
pub use users::*;

/// A stored role word, as a [`Role`](ambre_core::Role).
///
/// A word no build wrote is a decode error: the column's check constraint
/// admits none, so it can only mean a corrupt row.
fn decode_role(word: &str) -> Result<ambre_core::Role, sqlx::Error> {
    word.parse()
        .map_err(|e: ambre_core::error::RoleError| sqlx::Error::Decode(Box::new(e)))
}

/// A stored role column that may hold no row, as a [`Role`](ambre_core::Role).
fn decode_role_opt(word: Option<String>) -> Result<Option<ambre_core::Role>, sqlx::Error> {
    word.as_deref().map(decode_role).transpose()
}

/// A stored document type word, as a [`DocType`](ambre_core::DocType).
///
/// A word no build wrote is a decode error, as for [`decode_role`]
/// (`SRV-180`).
fn decode_doc_type(word: &str) -> Result<ambre_core::DocType, sqlx::Error> {
    word.parse()
        .map_err(|e: ambre_core::error::DocTypeError| sqlx::Error::Decode(Box::new(e)))
}

/// A stored display name. One that breaks the rule is a decode error: every
/// write path builds a [`DisplayName`](ambre_core::DisplayName) first
/// (`SRV-234`).
fn decode_display_name(text: String) -> Result<ambre_core::DisplayName, sqlx::Error> {
    ambre_core::DisplayName::try_from(text).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored list of display names, each read as [`decode_display_name`]
/// reads one.
fn decode_display_names(texts: Vec<String>) -> Result<Vec<ambre_core::DisplayName>, sqlx::Error> {
    texts.into_iter().map(decode_display_name).collect()
}

/// A stored member count, checked into 32 bits. A count that does not fit is
/// a decode error, never a truncation (`SRV-238`).
fn decode_member_count(count: i64) -> Result<ambre_core::MemberCount, sqlx::Error> {
    ambre_core::MemberCount::try_from(count).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored sequence number column, as a [`Seq`](ambre_core::Seq). A
/// negative value is a decode error: the relay numbers from zero up.
fn decode_seq(value: i64) -> Result<ambre_core::Seq, sqlx::Error> {
    ambre_core::Seq::try_from(value).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored clock. A negative one is a decode error, as for [`decode_seq`].
fn decode_clock(value: i64) -> Result<ambre_protocol::messages::Clock, sqlx::Error> {
    ambre_protocol::messages::Clock::try_from(value).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored public key. Any length but 32 bytes is a decode error: the relay
/// never hands a peer a key it could not have registered.
pub(crate) fn decode_key(bytes: Vec<u8>) -> Result<ambre_core::PublicKey, sqlx::Error> {
    ambre_core::PublicKey::from_slice(&bytes).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored document key identifier. Any length but its own is a decode
/// error.
fn decode_key_id(bytes: Vec<u8>) -> Result<ambre_core::KeyId, sqlx::Error> {
    <[u8; ambre_core::KEY_ID_LEN]>::try_from(bytes.as_slice())
        .map(ambre_core::KeyId::from_array)
        .map_err(|_| sqlx::Error::Decode(format!("a key id of {} bytes", bytes.len()).into()))
}

/// A stored sealed value that may be `NULL`, read with
/// `#[sqlx(try_from = "PgOptBytes")]` into its role type.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgOptBytes(Option<Vec<u8>>);

impl<T: From<Vec<u8>>> From<PgOptBytes> for Option<T> {
    fn from(PgOptBytes(bytes): PgOptBytes) -> Self {
        bytes.map(T::from)
    }
}

/// A stored public key, read with `#[sqlx(try_from = "PgKey")]` into the key
/// itself, refused as [`decode_key`] refuses one.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgKey(Vec<u8>);

impl TryFrom<PgKey> for ambre_core::PublicKey {
    type Error = ambre_core::PublicKeyError;

    fn try_from(PgKey(bytes): PgKey) -> Result<Self, Self::Error> {
        ambre_core::PublicKey::from_slice(&bytes)
    }
}

impl TryFrom<PgKey> for ambre_core::ExchangePublicKey {
    type Error = ambre_core::PublicKeyError;

    fn try_from(PgKey(bytes): PgKey) -> Result<Self, Self::Error> {
        ambre_core::ExchangePublicKey::from_slice(&bytes)
    }
}

/// A stored master-key salt. Any length but [`ambre_core::MasterKeySalt::LEN`]
/// is a decode error, as for [`decode_key`].
fn decode_salt(bytes: Vec<u8>) -> Result<ambre_core::MasterKeySalt, sqlx::Error> {
    ambre_core::MasterKeySalt::from_slice(&bytes).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored public key that may be `NULL`, as [`decode_key`] reads one.
fn decode_opt_key(bytes: Option<Vec<u8>>) -> Result<Option<ambre_core::PublicKey>, sqlx::Error> {
    bytes.map(decode_key).transpose()
}

/// A stored X25519 exchange key. Any length but 32 bytes is a decode error,
/// as for [`decode_key`].
pub(crate) fn decode_exchange_key(
    bytes: Vec<u8>,
) -> Result<ambre_core::ExchangePublicKey, sqlx::Error> {
    ambre_core::ExchangePublicKey::from_slice(&bytes).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// A stored exchange key that may be `NULL`, as [`decode_exchange_key`]
/// reads one.
fn decode_opt_exchange_key(
    bytes: Option<Vec<u8>>,
) -> Result<Option<ambre_core::ExchangePublicKey>, sqlx::Error> {
    bytes.map(decode_exchange_key).transpose()
}

fn uuid_to_core(u: uuid::Uuid) -> ambre_core::Uuid {
    ambre_core::Uuid::from_bytes(*u.as_bytes())
}

fn uuid_row_to_id<T: From<ambre_core::Uuid>>(u: uuid::Uuid) -> T {
    T::from(uuid_to_core(u))
}

/// A stored identifier column. A row field reads it with
/// `#[sqlx(try_from = "PgId")]`, so the row holds the core identifier itself.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgId(uuid::Uuid);

/// A stored identifier column that may be `NULL`, read with
/// `#[sqlx(try_from = "PgOptId")]`.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgOptId(Option<uuid::Uuid>);

/// A stored sequence number that may be `NULL`, read with
/// `#[sqlx(try_from = "PgOptSeq")]`. A negative one is a decode error, as
/// [`decode_seq`] refuses one.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgOptSeq(Option<i64>);

impl TryFrom<PgOptSeq> for Option<ambre_core::Seq> {
    type Error = std::num::TryFromIntError;

    fn try_from(PgOptSeq(stored): PgOptSeq) -> Result<Self, Self::Error> {
        stored.map(ambre_core::Seq::try_from).transpose()
    }
}

/// The milliseconds left until a deadline the register computed, read with
/// `#[sqlx(try_from = "PgTimeLeft")]`: `None` once the deadline passed, or
/// when the row holds none.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgTimeLeft(Option<i64>);

impl From<PgTimeLeft> for Option<std::time::Duration> {
    fn from(PgTimeLeft(ms): PgTimeLeft) -> Self {
        ms.filter(|ms| *ms > 0)
            .map(|ms| std::time::Duration::from_millis(ms.unsigned_abs()))
    }
}

/// A stored segment state, `NULL` for a document that is no segment, read
/// with `#[sqlx(try_from = "PgOptState")]`.
#[derive(sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct PgOptState(Option<String>);

impl TryFrom<PgOptState> for Option<ambre_core::SegmentState> {
    type Error = String;

    fn try_from(PgOptState(stored): PgOptState) -> Result<Self, Self::Error> {
        stored
            .map(|word| {
                ambre_core::SegmentState::parse(&word)
                    .ok_or_else(|| format!("unknown segment state {word:?}"))
            })
            .transpose()
    }
}

macro_rules! pg_id {
    ($($id:ty),* $(,)?) => {$(
        impl From<PgId> for $id {
            fn from(PgId(u): PgId) -> Self {
                uuid_row_to_id(u)
            }
        }

        impl From<PgOptId> for Option<$id> {
            fn from(PgOptId(u): PgOptId) -> Self {
                u.map(uuid_row_to_id)
            }
        }
    )*};
}

pg_id!(
    ambre_core::DocumentId,
    ambre_core::UserId,
    ambre_core::ChannelId,
    ambre_core::SnapshotId,
    ambre_core::OrgId,
    ambre_core::LeaseId,
);

pub(crate) fn id_to_uuid(id: impl Into<ambre_core::Uuid>) -> uuid::Uuid {
    let core_uuid: ambre_core::Uuid = id.into();
    uuid::Uuid::from_bytes(*core_uuid.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SRV-238`: a count past 32 bits is refused when read, never truncated.
    #[test]
    fn a_member_count_past_u32_is_refused_not_truncated() {
        let fits = decode_member_count(i64::from(u32::MAX)).expect("the widest count fits");
        assert_eq!(fits.get(), u32::MAX);
        for count in [i64::from(u32::MAX) + 1, -1] {
            assert!(
                matches!(decode_member_count(count), Err(sqlx::Error::Decode(_))),
                "{count} must not decode"
            );
        }
    }

    /// `covered_seq` survives the insert and both read paths. This is the value a
    /// client gates its post-open `RequestMissing` on, so a silent reset to 0
    /// would resurrect the full-history replay.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_snapshot_roundtrips_covered_seq(pool: PgPool) {
        // A snapshot row needs an owning document, which needs an owner user (FKs).
        let user = create_user(
            &pool,
            &ambre_core::Email::from("snap@test.dev"),
            &ambre_core::DisplayName::new("Snap").unwrap(),
            &ambre_core::WrappedMasterKey::new([1u8; 32].to_vec()),
            &ambre_core::MasterKeySalt::from_array([2u8; 32]),
            &ambre_core::PublicKey::from_array([3u8; 32]),
            &ambre_core::ExchangePublicKey::from_array([4u8; 32]),
        )
        .await
        .expect("create user");
        let owner = user.id;
        let (doc, created) = create_document(&pool, &owner, ambre_core::DocType::Trame, None, None)
            .await
            .expect("create document");
        assert!(created);
        let document_id = doc.id;

        let author = [5u8; 32];
        let clocks = serde_json::json!({});
        let sid = create_snapshot(
            &pool,
            &document_id,
            &ambre_core::PublicKey::from_array(author),
            &clocks,
            &ambre_core::ObjectKey::from("s3/key/snap"),
            ambre_core::Seq(1500),
        )
        .await
        .expect("create snapshot");
        confirm_snapshot(
            &pool,
            &document_id,
            &sid,
            &ambre_core::PublicKey::from_array([9u8; 32]),
        )
        .await
        .expect("confirm snapshot");

        // get_latest_snapshot (the open path's GET /snapshot) carries it.
        let latest = get_latest_snapshot(&pool, &document_id)
            .await
            .expect("query latest")
            .expect("snapshot exists");
        assert_eq!(latest.id, sid);
        assert_eq!(latest.s3_key, ambre_core::ObjectKey::from("s3/key/snap"));
        assert_eq!(latest.covered_seq, ambre_core::Seq(1500));

        // get_snapshot (the download-url path) carries it too.
        let by_id = get_snapshot(&pool, &document_id, &sid)
            .await
            .expect("query by id")
            .expect("snapshot exists");
        assert_eq!(by_id.covered_seq, ambre_core::Seq(1500));

        // IDOR regression guard: resolving the same snapshot id under a foreign
        // document must return None (never leak another document's blob).
        let other_doc = ambre_core::DocumentId::new();
        let cross = get_snapshot(&pool, &other_doc, &sid)
            .await
            .expect("query by id under wrong doc");
        assert!(cross.is_none(), "snapshot must be scoped to its document");
    }

    /// F1: once a snapshot has folded in a prefix of the delta history, the
    /// compaction pass must trim the superseded `updates` rows. The dropped
    /// `updates.snapshot_id` FK no longer cascades them away, so without an
    /// explicit purge the delta index grows without bound.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn purge_superseded_updates_trims_covered_deltas(pool: PgPool) {
        let user = create_user(
            &pool,
            &ambre_core::Email::from("purge@test.dev"),
            &ambre_core::DisplayName::new("Purge").unwrap(),
            &ambre_core::WrappedMasterKey::new([1u8; 32].to_vec()),
            &ambre_core::MasterKeySalt::from_array([2u8; 32]),
            &ambre_core::PublicKey::from_array([3u8; 32]),
            &ambre_core::ExchangePublicKey::from_array([4u8; 32]),
        )
        .await
        .expect("create user");
        let owner = user.id;
        let (doc, _) = create_document(&pool, &owner, ambre_core::DocType::Prose, None, None)
            .await
            .expect("create document");
        let document_id = doc.id;
        let author = [5u8; 32];
        let snap = ambre_core::SnapshotId::new();

        // Ten deltas, seq (and clock) 1..=10.
        for seq in 1..=10i64 {
            insert_update(
                &pool,
                &document_id,
                &ambre_core::PublicKey::from_array(author),
                &snap,
                ambre_protocol::messages::Clock(seq.unsigned_abs()),
                ambre_core::Seq(seq.unsigned_abs()),
                &ambre_core::ObjectKey(format!("k{seq}")),
            )
            .await
            .expect("insert update");
        }
        // A snapshot folding in everything through seq 6, which another
        // member confirmed.
        let sid = create_snapshot(
            &pool,
            &document_id,
            &ambre_core::PublicKey::from_array(author),
            &serde_json::json!({}),
            &ambre_core::ObjectKey::from("s3/snap"),
            ambre_core::Seq(6),
        )
        .await
        .expect("create snapshot");
        confirm_snapshot(
            &pool,
            &document_id,
            &sid,
            &ambre_core::PublicKey::from_array([9u8; 32]),
        )
        .await
        .expect("confirm snapshot");

        let deleted = purge_superseded_updates(&pool).await.expect("purge");
        assert_eq!(
            deleted.rows, 6,
            "seq 1..=6 are covered by the snapshot and purged"
        );

        let remaining = get_updates_after(
            &pool,
            &document_id,
            ambre_core::Seq(0),
            crate::db::QueryLimit::saturating(100),
        )
        .await
        .expect("list remaining");
        let seqs: Vec<i64> = remaining
            .iter()
            .map(|u| u.seq.to_i64_saturating())
            .collect();
        assert_eq!(seqs, vec![7, 8, 9, 10], "only the uncovered tail survives");
    }

    /// F1: the purge horizon is the MIN `covered_seq` across the RETAINED
    /// snapshots. An update still needed by the oldest retained snapshot for
    /// incremental catch-up is never deleted.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn purge_horizon_is_min_covered_seq(pool: PgPool) {
        let user = create_user(
            &pool,
            &ambre_core::Email::from("horizon@test.dev"),
            &ambre_core::DisplayName::new("Horizon").unwrap(),
            &ambre_core::WrappedMasterKey::new([1u8; 32].to_vec()),
            &ambre_core::MasterKeySalt::from_array([2u8; 32]),
            &ambre_core::PublicKey::from_array([3u8; 32]),
            &ambre_core::ExchangePublicKey::from_array([4u8; 32]),
        )
        .await
        .expect("create user");
        let owner = user.id;
        let (doc, _) = create_document(&pool, &owner, ambre_core::DocType::Prose, None, None)
            .await
            .expect("create document");
        let document_id = doc.id;
        let author = [5u8; 32];
        let snap = ambre_core::SnapshotId::new();

        for seq in 1..=10i64 {
            insert_update(
                &pool,
                &document_id,
                &ambre_core::PublicKey::from_array(author),
                &snap,
                ambre_protocol::messages::Clock(seq.unsigned_abs()),
                ambre_core::Seq(seq.unsigned_abs()),
                &ambre_core::ObjectKey(format!("k{seq}")),
            )
            .await
            .expect("insert update");
        }
        // Two retained snapshots: covered_seq 3 (oldest) and 8 (newest).
        for covered in [3i64, 8] {
            let sid = create_snapshot(
                &pool,
                &document_id,
                &ambre_core::PublicKey::from_array(author),
                &serde_json::json!({}),
                &ambre_core::ObjectKey(format!("s3/snap-{covered}")),
                ambre_core::Seq::try_from(covered).expect("a relay number"),
            )
            .await
            .expect("create snapshot");
            confirm_snapshot(
                &pool,
                &document_id,
                &sid,
                &ambre_core::PublicKey::from_array([9u8; 32]),
            )
            .await
            .expect("confirm snapshot");
        }

        let deleted = purge_superseded_updates(&pool).await.expect("purge");
        assert_eq!(
            deleted.rows, 3,
            "MIN(3, 8) = 3, so only seq 1..=3 are purged"
        );

        let remaining = get_updates_after(
            &pool,
            &document_id,
            ambre_core::Seq(0),
            crate::db::QueryLimit::saturating(100),
        )
        .await
        .expect("list remaining");
        let seqs: Vec<i64> = remaining
            .iter()
            .map(|u| u.seq.to_i64_saturating())
            .collect();
        assert_eq!(seqs, vec![4, 5, 6, 7, 8, 9, 10]);
    }
}
