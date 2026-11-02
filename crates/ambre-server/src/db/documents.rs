// ambre-server/src/db/documents.rs
// Document rows and document CRUD queries.

use super::*;
use ambre_core::StoredAsI64;
use ambre_core::{Instant, UnixSec};

/// Row returned by `SELECT * FROM documents`.
#[derive(Debug)]
pub struct DocumentRow {
    pub id: Id<Document>,
    pub owner_id: Id<User>,
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Document joined with the caller's membership row.
#[derive(Debug)]
pub struct DocumentSummaryRow {
    pub id: Id<Document>,
    pub owner_id: Id<User>,
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub role: ambre_core::Role,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Exchange public key of the member who wrapped the caller's `doc_key_enc`
    /// (the inviter, or the owner at creation). `None` for a self-wrapped owner
    /// row on older data; the client then falls back to its own key.
    pub wrapper_exchange_pubkey: Option<FixedBytes<ambre_core::ExchangePublicKey>>,
    /// Total number of members on the document (routing metadata, plaintext).
    /// `> 1` means the document is shared.
    pub member_count: ambre_core::MemberCount,
}

/// Fetches a document by id.
pub async fn get_document(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<Option<DocumentRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(
        "SELECT id, owner_id, doc_type, name_enc, created_at, updated_at FROM documents WHERE id = $1"
    )
    .bind(did)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(DocumentRow {
            id: r.try_get::<PgId, _>("id")?.into(),
            owner_id: r.try_get::<PgId, _>("owner_id")?.into(),
            doc_type: decode_doc_type(r.try_get("doc_type")?)?,
            name_enc: r
                .try_get::<Option<Vec<u8>>, _>("name_enc")?
                .map(ambre_core::Encoded::new),
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
        })),
        None => Ok(None),
    }
}

/// Lists all documents the user is a member of, with role and doc_key_enc.
pub async fn list_documents_for_user(
    pool: &PgPool,
    user_id: &Id<User>,
) -> Result<Vec<DocumentSummaryRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    // `inv` resolves the exchange pubkey of whoever wrapped our `doc_key_enc`,
    // so the client can ECDH-unwrap each doc key and decrypt the name for the
    // launcher. The correlated subquery counts members for the shared badge.
    let rows = sqlx::query(
        "SELECT d.id, d.owner_id, d.doc_type, d.name_enc, d.updated_at, dm.role, dm.doc_key_enc, \
         inv.exchange_pubkey AS wrapper_exchange_pubkey, \
         (SELECT COUNT(*) FROM document_members dm2 WHERE dm2.document_id = d.id) AS member_count \
         FROM documents d \
         INNER JOIN document_members dm ON dm.document_id = d.id \
         LEFT JOIN users inv ON inv.id = dm.invited_by \
         WHERE dm.user_id = $1 ORDER BY d.updated_at DESC",
    )
    .bind(uid)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(DocumentSummaryRow {
                id: r.try_get::<PgId, _>("id")?.into(),
                owner_id: r.try_get::<PgId, _>("owner_id")?.into(),
                doc_type: decode_doc_type(r.try_get("doc_type")?)?,
                name_enc: r
                    .try_get::<Option<Vec<u8>>, _>("name_enc")?
                    .map(ambre_core::Encoded::new),
                updated_at: r.try_get("updated_at")?,
                role: decode_role(r.try_get("role")?)?,
                doc_key_enc: ambre_core::Encoded::new(r.try_get("doc_key_enc")?),
                wrapper_exchange_pubkey: decode_opt_exchange_key(
                    r.try_get("wrapper_exchange_pubkey")?,
                )?,
                member_count: decode_member_count(r.try_get("member_count")?)?,
            })
        })
        .collect()
}

/// Whether a create call inserted its row, or found the id already taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insertion {
    /// An idempotent retry, or another caller's id: the row was already there.
    Existing,
    /// This call inserted the row.
    Inserted,
}

/// Creates a document, optionally with a client-supplied id, and returns the full
/// row. Lazy creation: the WASM client generates the `Id<Document>` up front, and
/// only materializes the row on the first modification. Returns the row and
/// whether this call inserted it ([`Insertion::Existing`] when an idempotent
/// retry hit an existing id).
///
/// When `document_id` is provided the insert is idempotent (`ON CONFLICT DO
/// NOTHING`). A second call with the same id returns the existing row without
/// touching its `owner_id`. A different caller can never seize ownership of an id
/// that already exists.
pub async fn create_document(
    pool: &PgPool,
    owner_id: &Id<User>,
    doc_type: ambre_core::DocType,
    name_enc: Option<&ambre_core::Encoded<ambre_core::SealedName>>,
    document_id: Option<&Id<Document>>,
) -> Result<(DocumentRow, Insertion), AppError> {
    let owner_uid = id_to_uuid(*owner_id);
    let doc_type = doc_type.as_str();

    let row_to_doc = |row: sqlx::postgres::PgRow| -> Result<DocumentRow, AppError> {
        Ok(DocumentRow {
            id: row.try_get::<PgId, _>("id")?.into(),
            owner_id: row.try_get::<PgId, _>("owner_id")?.into(),
            doc_type: decode_doc_type(row.try_get("doc_type")?)?,
            name_enc: row
                .try_get::<Option<Vec<u8>>, _>("name_enc")?
                .map(ambre_core::Encoded::new),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    };

    match document_id {
        Some(did) => {
            let did = id_to_uuid(*did);
            // Idempotent insert: DO NOTHING returns no row on conflict.
            let inserted = sqlx::query(
                "INSERT INTO documents (id, owner_id, doc_type, name_enc) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING RETURNING id, owner_id, doc_type, name_enc, created_at, updated_at"
            )
            .bind(did)
            .bind(owner_uid)
            .bind(doc_type)
            .bind(name_enc.map(|v| v.as_bytes()))
            .fetch_optional(pool)
            .await?;

            match inserted {
                Some(row) => Ok((row_to_doc(row)?, Insertion::Inserted)),
                None => {
                    // Already existed: return it untouched (owner preserved).
                    let row = sqlx::query(
                        "SELECT id, owner_id, doc_type, name_enc, created_at, updated_at FROM documents WHERE id = $1"
                    )
                    .bind(did)
                    .fetch_one(pool)
                    .await?;
                    Ok((row_to_doc(row)?, Insertion::Existing))
                }
            }
        }
        None => {
            let row = sqlx::query(
                "INSERT INTO documents (owner_id, doc_type, name_enc) VALUES ($1, $2, $3) RETURNING id, owner_id, doc_type, name_enc, created_at, updated_at"
            )
            .bind(owner_uid)
            .bind(doc_type)
            .bind(name_enc.map(|v| v.as_bytes()))
            .fetch_one(pool)
            .await?;
            Ok((row_to_doc(row)?, Insertion::Inserted))
        }
    }
}

/// Updates the encrypted name of a document.
pub async fn rename_document(
    pool: &PgPool,
    document_id: &Id<Document>,
    name_enc: &ambre_core::Encoded<ambre_core::SealedName>,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("UPDATE documents SET name_enc = $2, updated_at = now() WHERE id = $1")
        .bind(did)
        .bind(name_enc.as_bytes())
        .execute(pool)
        .await?;
    Ok(())
}

/// A delete marked in the register and not finished: its rows may still be
/// there, and its second sweep is due at `sweep_due`.
#[derive(Debug)]
pub struct PendingDeletion {
    pub document_id: Id<Document>,
    pub sweep_due: Instant<UnixSec>,
    /// Whether the document row is still there: the delete stopped before its
    /// last step.
    pub rows_left: DocumentRows,
}

ambre_core::define_flag! {
    /// Whether the rows of a document whose delete is marked are still there.
    pub enum DocumentRows {
        /// The document row is gone: only the second sweep is left.
        Gone,
        /// The document row is there: the delete stopped before its last step.
        Left,
    }
}

/// Marks a document as being deleted, before anything of it goes (`SRV-165`).
/// A second mark keeps the later of the two sweeps, so a delete begun again
/// never sweeps before a link it handed out has expired.
///
/// A space goes with every channel of it, in the same transaction
/// (`SRV-143`). Each channel is deleted and its documents marked, then the
/// space is marked. Those channels are answered, empty for any other
/// document. A direct message belongs to no space, so none is touched.
pub async fn begin_deletion(
    pool: &PgPool,
    document_id: &Id<Document>,
    sweep_due: Instant<UnixSec>,
) -> Result<Vec<super::DeletedChannel>, AppError> {
    let mut tx = pool.begin().await?;
    let channels = super::channels::delete_space_channels_in(
        &mut tx,
        Id::<Space>::from_document(*document_id),
        sweep_due,
    )
    .await?;
    begin_deletions_in(&mut tx, &[*document_id], sweep_due).await?;
    tx.commit().await?;
    Ok(channels)
}

/// Marks the delete of each of `documents` inside `tx`, as [`begin_deletion`]
/// marks one. The rows stay: the pass empties a marked document whose rows
/// are left, and forgets them last (`SRV-165`).
pub(super) async fn begin_deletions_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    documents: &[Id<Document>],
    sweep_due: Instant<UnixSec>,
) -> Result<(), AppError> {
    let documents: Vec<uuid::Uuid> = documents.iter().copied().map(id_to_uuid).collect();
    sqlx::query(
        "INSERT INTO document_deletions (document_id, sweep_due) \
         SELECT unnest($1::uuid[]), $2 \
         ON CONFLICT (document_id) DO UPDATE \
         SET sweep_due = GREATEST(document_deletions.sweep_due, EXCLUDED.sweep_due)",
    )
    .bind(&documents)
    .bind(sweep_due.to_i64_saturating())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Every delete still marked, the earliest sweep first, at most `limit`.
pub async fn pending_deletions(
    pool: &PgPool,
    limit: ambre_core::Bounded<QueryLimit>,
) -> Result<Vec<PendingDeletion>, AppError> {
    let rows: Vec<(uuid::Uuid, i64, bool)> = sqlx::query_as(
        "SELECT d.document_id, d.sweep_due, \
                EXISTS (SELECT 1 FROM documents WHERE id = d.document_id) \
         FROM document_deletions d ORDER BY d.sweep_due LIMIT $1",
    )
    .bind(i64::from(limit.get()))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, sweep_due, rows_left)| PendingDeletion {
            document_id: uuid_row_to_id(id),
            // A negative mark, which no relay writes, is due at once.
            sweep_due: Instant::<UnixSec>::from_i64(sweep_due),
            rows_left: DocumentRows::from_bool(rows_left),
        })
        .collect())
}

/// Drops the mark of a delete whose second sweep ran at `swept_at`, unless a
/// later mark pushed the sweep past it meanwhile.
pub async fn end_deletion(
    pool: &PgPool,
    document_id: &Id<Document>,
    swept_at: Instant<UnixSec>,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM document_deletions WHERE document_id = $1 AND sweep_due <= $2")
        .bind(id_to_uuid(*document_id))
        .bind(swept_at.to_i64_saturating())
        .execute(pool)
        .await?;
    Ok(())
}

/// Cascade-deletes a document and all associated rows (members, snapshots, updates).
pub async fn delete_document(pool: &PgPool, document_id: &Id<Document>) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(did)
        .execute(pool)
        .await?;
    Ok(())
}
