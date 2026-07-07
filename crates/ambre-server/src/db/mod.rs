use crate::error::AppError;
use sqlx::{PgPool, Row};

// Query functions are split by domain into sibling modules; each reaches the
// shared imports and the private uuid bridges below via `use super::*`. All
// items are re-globbed so every `db::*` path stays valid.
mod content;
mod documents;
mod members;
mod orgs;
mod users;

pub use content::*;
pub use documents::*;
pub use members::*;
pub use orgs::*;
pub use users::*;

#[allow(dead_code)]
fn uuid_to_core(u: uuid::Uuid) -> ambre_core::Uuid {
    ambre_core::Uuid::from_bytes(*u.as_bytes())
}

#[allow(dead_code)]
fn uuid_row_to_id<T: From<ambre_core::Uuid>>(u: uuid::Uuid) -> T {
    T::from(uuid_to_core(u))
}

fn id_to_uuid(id: impl Into<ambre_core::Uuid>) -> uuid::Uuid {
    let core_uuid: ambre_core::Uuid = id.into();
    uuid::Uuid::from_bytes(*core_uuid.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bridge a stored `uuid::Uuid` (FK columns) back to a typed core id.
    fn core_id<T: From<ambre_core::Uuid>>(id: uuid::Uuid) -> T {
        ambre_core::Uuid::from_bytes(*id.as_bytes()).into()
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
            "snap@test.dev",
            "Snap",
            &[1u8; 32],
            &[2u8; 16],
            &[3u8; 32],
            &[4u8; 32],
        )
        .await
        .expect("create user");
        let owner: ambre_core::UserId = core_id(user.id);
        let (doc, created) = create_document(&pool, &owner, "trame", None, None)
            .await
            .expect("create document");
        assert!(created);
        let document_id: ambre_core::DocumentId = core_id(doc.id);

        let author = [5u8; 32];
        let clocks = serde_json::json!({});
        let sid = create_snapshot(
            &pool,
            &document_id,
            &author,
            None,
            None,
            &clocks,
            "s3/key/snap",
            1500,
        )
        .await
        .expect("create snapshot");

        // get_latest_snapshot (the open path's GET /snapshot) carries it.
        let latest = get_latest_snapshot(&pool, &document_id)
            .await
            .expect("query latest")
            .expect("snapshot exists");
        assert_eq!(latest.id, id_to_uuid(sid));
        assert_eq!(latest.s3_key, "s3/key/snap");
        assert_eq!(latest.covered_seq, 1500);

        // get_snapshot (the download-url path) carries it too.
        let by_id = get_snapshot(&pool, &document_id, &sid)
            .await
            .expect("query by id")
            .expect("snapshot exists");
        assert_eq!(by_id.covered_seq, 1500);

        // IDOR regression guard: resolving the same snapshot id under a foreign
        // document must return None (never leak another document's blob).
        let other_doc = ambre_core::DocumentId::new();
        let cross = get_snapshot(&pool, &other_doc, &sid)
            .await
            .expect("query by id under wrong doc");
        assert!(cross.is_none(), "snapshot must be scoped to its document");
    }
}
