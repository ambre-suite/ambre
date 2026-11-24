// ambre-server/src/db/settings.rs
// The pointer from an account to the document that holds its settings.

use super::*;

/// What a claim of the settings document found (`SETTINGS-009`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsClaim {
    /// This call created the document and set the pointer.
    Created(Id<Document>),
    /// The account already pointed at a document; nothing was stored.
    Existing(Id<Document>),
}

impl SettingsClaim {
    /// The document the account points at once the claim returns.
    #[must_use]
    pub const fn document(self) -> Id<Document> {
        match self {
            Self::Created(document) | Self::Existing(document) => document,
        }
    }
}

/// The document that holds the settings of `user`, if the account has one.
pub async fn settings_document(
    pool: &PgPool,
    user: &Id<User>,
) -> Result<Option<Id<Document>>, AppError> {
    let found: Option<Option<uuid::Uuid>> =
        sqlx::query_scalar("SELECT settings_document FROM users WHERE id = $1")
            .bind(id_to_uuid(*user))
            .fetch_optional(pool)
            .await?;
    Ok(found.flatten().map(|uuid| PgId(uuid).into()))
}

/// Creates the settings document of `user` with its owner row and its first
/// log entry, and points the account at it, in one transaction. An account
/// that already points at a document keeps it, and the call stores nothing
/// (`SETTINGS-002`, `SETTINGS-009`).
pub async fn claim_settings_document(
    pool: &PgPool,
    user: &Id<User>,
    document: &Id<Document>,
    doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    entry: &LogEntry,
) -> Result<SettingsClaim, AppError> {
    let uid = id_to_uuid(*user);
    let mut tx = pool.begin().await?;
    // The row lock orders two first unlocks: the second reads the pointer
    // the first wrote.
    let current: Option<Option<uuid::Uuid>> =
        sqlx::query_scalar("SELECT settings_document FROM users WHERE id = $1 FOR UPDATE")
            .bind(uid)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(current) = current else {
        return Err(AppError::NotFound {
            resource: crate::error::Resource::User,
        });
    };
    if let Some(existing) = current {
        tx.commit().await?;
        return Ok(SettingsClaim::Existing(PgId(existing).into()));
    }
    let inserted: Option<uuid::Uuid> = sqlx::query_scalar(
        "INSERT INTO documents (id, owner_id, doc_type) VALUES ($1, $2, 'settings') \
         ON CONFLICT (id) DO NOTHING RETURNING id",
    )
    .bind(id_to_uuid(*document))
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        // The id names a document of its own already: never adopt it.
        return Err(AppError::Conflict {
            detail: "the document id is taken".into(),
        });
    }
    append_entry(&mut tx, *document, entry).await?;
    upsert_member(
        &mut tx,
        document,
        &NewMember {
            user_id: user,
            role: ambre_core::Role::Owner,
            doc_key_enc,
            invited_by: user,
        },
    )
    .await?;
    sqlx::query("UPDATE users SET settings_document = $2 WHERE id = $1")
        .bind(uid)
        .bind(id_to_uuid(*document))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(SettingsClaim::Created(*document))
}
