// ambre-server/src/db/members.rs
// Document-member rows and membership / key-rotation queries.

use super::*;
use ambre_core::{Instant, UnixSec};

/// Row from `document_members`.
#[derive(Debug)]
pub struct MemberRow {
    pub role: ambre_core::Role,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only a test reads who invited the member")
    )]
    pub invited_by: Option<Id<User>>,
}

/// What a removal posted alone did, decided inside its transaction.
#[derive(Debug, PartialEq, Eq)]
pub enum Removed {
    /// The document owes a rotation (`SRV-122`).
    Owed,
    /// The key stays: every member of the space reads a public channel's key
    /// in the catalogue anyway (`SRV-122`).
    Kept,
    /// The last member left a channel that is not public, which went with
    /// every document it grouped (`SRV-142`). Each is marked for its delete,
    /// which the caller runs (`SRV-165`).
    ChannelDeleted(Vec<Id<Document>>),
    /// The document is a space. It owes a rotation, and the member left
    /// every channel of it too (`SRV-136`).
    SpaceLeft(SpaceLeave),
}

/// What leaving a space did to the channels of that space (`SRV-136`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpaceLeave {
    /// The `meta` of each channel the member left and that stays.
    pub left: Vec<Id<Document>>,
    /// Those of them that owe a rotation: every channel that is not public.
    pub owed: Vec<Id<Document>>,
    /// Each channel the member was the last of, by its `meta`, with every
    /// document it grouped, marked for its delete (`SRV-142`).
    pub deleted: Vec<(Id<Document>, Vec<Id<Document>>)>,
}

/// `document_members` joined with `users` for display purposes. Carries the
/// member's public keys and their wrapped copy of the document key. A client
/// can then locate its own entry and unwrap the key, or rewrap on rotation.
#[derive(Debug)]
pub struct MemberWithUserRow {
    pub user_id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether the member's email was provider-verified vs self-asserted at
    /// the retired account setup (#3). Lets the member list flag an unverified peer for the
    /// TOFU out-of-band key check.
    pub email_verified: super::EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub role: ambre_core::Role,
    pub signing_pubkey: FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: FixedBytes<ambre_core::ExchangePublicKey>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Exchange public key of the member who wrapped `doc_key_enc` (the inviter,
    /// or the member themselves for a self-wrap at creation). The recipient needs
    /// this peer key to perform the ECDH unwrap -- their own key is the wrong DH
    /// partner. `None` only for legacy rows with no `invited_by`.
    pub wrapper_exchange_pubkey: Option<FixedBytes<ambre_core::ExchangePublicKey>>,
}

/// One signed membership entry a request carries, checked by the route that
/// received it (`SRV-104`). `bytes` are stored as they came.
#[derive(Debug)]
pub struct LogEntry {
    /// The place the entry was signed for.
    pub position: ambre_core::Seq<ambre_protocol::LogPlaces>,
    pub bytes: ambre_core::Encoded<ambre_core::EncodedLogEntry>,
}

/// The entries a removal appends to the log: its own, then the cap
/// continuations that follow it at the next places (`SRV-344`). The route
/// checked each one. A removal of a document with no log carries none.
#[derive(Debug, Default)]
pub struct RemovalEntries {
    pub entry: Option<LogEntry>,
    /// The continuations, in log order. Empty without an entry.
    pub continued: Vec<LogEntry>,
}

/// The document whose member rows decide access to `$1` (`SRV-103`).
///
/// A document of a channel resolves to the channel's `meta` document, so
/// membership of the channel reaches every document it groups. A member row on
/// the channel document itself grants nothing. Any other document resolves to
/// itself. A channel document whose channel has no `meta` resolves to `NULL`,
/// which matches no row.
///
/// A macro rather than a `const`, so `concat!` keeps each query `&'static`.
macro_rules! access_document {
    () => {
        "(SELECT CASE \
         WHEN EXISTS (SELECT 1 FROM channel_documents WHERE document_id = $1) \
         THEN (SELECT m.document_id FROM channel_documents c \
               JOIN channel_documents m ON m.channel_id = c.channel_id AND m.kind = 'meta' \
               WHERE c.document_id = $1) \
         ELSE $1 END)"
    };
}

/// Adds (or updates) a user to a document's membership with a role and encrypted document key.
///
/// The upsert doubles as the role-change path (`ON CONFLICT DO UPDATE`), so it enforces the
/// same last-owner invariant as `update_document_member_role`. It refuses to demote the
/// document's only remaining owner. The check and the write run in one transaction with the
/// target row locked (`FOR UPDATE`) so concurrent demotions cannot both pass the count.
/// A creation's first `entry` lands in the same transaction (`SRV-104`).
pub async fn add_document_member(
    pool: &PgPool,
    document_id: &Id<Document>,
    member: &NewMember<'_>,
    entry: Option<&LogEntry>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    if let Some(entry) = entry {
        lock_document(&mut tx, *document_id).await?;
        append_entry(&mut tx, *document_id, entry).await?;
    }
    upsert_member(&mut tx, document_id, member).await?;
    tx.commit().await?;
    Ok(())
}

/// Whether a creator's retry of a creation may still register its owner row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// No member row, no log entry and no channel names the document. The
    /// first call stored the row and lost its membership on the way.
    Unclaimed,
    /// The document holds a member row, a log entry, or a place in a channel.
    Held,
}

/// Whether a retry of the creation of the document may register its creator
/// (`SRV-268`, `SRV-283`, audits `N7` and `P32`). A member row and a log entry
/// land in one transaction, so a lost first call leaves neither. A channel is
/// registered after its creation, so a channel document is never a lost
/// creation. That holds once its last member left it too (`SRV-173`).
pub async fn document_claim(pool: &PgPool, document_id: &Id<Document>) -> Result<Claim, AppError> {
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM document_members WHERE document_id = $1) \
         OR EXISTS (SELECT 1 FROM document_membership_log WHERE document_id = $1) \
         OR EXISTS (SELECT 1 FROM channel_documents WHERE document_id = $1)",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_one(pool)
    .await?;
    Ok(if held { Claim::Held } else { Claim::Unclaimed })
}

/// Appends `entry` at the next place of the document's log, inside the
/// caller's transaction and under its document lock (`SRV-104`). An entry
/// signed for another place is refused: another landed first, and the
/// client signs again after it. A stored entry is never rewritten
/// (`SYNC-107`).
pub(super) async fn append_entry(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    entry: &LogEntry,
) -> Result<(), AppError> {
    let document_id = id_to_uuid(document_id);
    let next: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM document_membership_log WHERE document_id = $1")
            .bind(document_id)
            .fetch_one(&mut **tx)
            .await?;
    if i64::try_from(entry.position.get()).ok() != Some(next) {
        return Err(AppError::LogMoved);
    }
    sqlx::query(
        "INSERT INTO document_membership_log (document_id, position, entry) VALUES ($1, $2, $3)",
    )
    .bind(document_id)
    .bind(next)
    .bind(entry.bytes.as_bytes())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records the entry a membership change carries (`SYNC-132`). A document
/// whose log holds entries takes no change without one. A document with an
/// empty log predates it, and its changes carry none.
pub(super) async fn record_change(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    entry: Option<&LogEntry>,
) -> Result<(), AppError> {
    let did = id_to_uuid(document_id);
    if let Some(entry) = entry {
        return append_entry(tx, document_id, entry).await;
    }
    let logged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM document_membership_log WHERE document_id = $1)",
    )
    .bind(did)
    .fetch_one(&mut **tx)
    .await?;
    if logged {
        return Err(AppError::BadRequest(
            "the document keeps a membership log: the change needs its entry".into(),
        ));
    }
    Ok(())
}

/// Records the entries of a removal: its own, as [`record_change`] does, then
/// each continuation at the next place, under the same lock (`SRV-344`). A
/// continuation at another place than the next refuses the whole removal.
async fn record_removal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    entries: &RemovalEntries,
) -> Result<(), AppError> {
    if entries.entry.is_none() && !entries.continued.is_empty() {
        return Err(AppError::BadRequest(
            "continued: a continuation follows the entry of its removal".into(),
        ));
    }
    record_change(tx, document_id, entries.entry.as_ref()).await?;
    for entry in &entries.continued {
        append_entry(tx, document_id, entry).await?;
    }
    Ok(())
}

/// Every entry of a document's log, in order (`SRV-104`).
///
/// For a document of a channel, the entries are those of the channel's
/// `meta`, whose rows and links the document serves too (`SRV-272`). The
/// client walks them under the `meta` its open names (`SYNC-229`).
pub async fn membership_log(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>, AppError> {
    let rows: Vec<Vec<u8>> = sqlx::query_scalar(concat!(
        "SELECT entry FROM document_membership_log WHERE document_id = ",
        access_document!(),
        " ORDER BY position"
    ))
    .bind(id_to_uuid(*document_id))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ambre_core::Encoded::new).collect())
}

/// Adds a member with a copy of the key named `key_id`, refused when a
/// rotation has replaced that key (`SRV-168`). The check runs under the
/// document lock every rotation takes, so the two are ordered. An invitation
/// that lands first is among the members a rotation must name (`SRV-167`).
pub async fn invite_document_member(
    pool: &PgPool,
    document_id: &Id<Document>,
    member: &NewMember<'_>,
    key_id: &FixedBytes<ambre_core::KeyId>,
    entry: Option<&LogEntry>,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let mut tx = pool.begin().await?;
    lock_document(&mut tx, *document_id).await?;
    // No link yet means no rotation yet: the key the document was created
    // with is the only one, and the copy wraps it.
    let last: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT key_id FROM document_key_links WHERE document_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(did)
    .fetch_optional(&mut *tx)
    .await?;
    if last
        .map(decode_key_id)
        .transpose()?
        .is_some_and(|last| last != *key_id)
    {
        return Err(AppError::KeyChanged);
    }
    record_change(&mut tx, *document_id, entry).await?;
    upsert_member(&mut tx, document_id, member).await?;
    tx.commit().await?;
    Ok(())
}

/// The columns of one `document_members` row that a join writes. They name
/// who joins, the role, the key copy wrapped for them, and who let them in.
pub struct NewMember<'a> {
    pub user_id: &'a Id<User>,
    pub role: ambre_core::Role,
    pub doc_key_enc: &'a ambre_core::Encoded<ambre_core::WrappedDocKey>,
    pub invited_by: &'a Id<User>,
}

/// The upsert behind both adds, inside the caller's transaction.
async fn upsert_member(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: &Id<Document>,
    member: &NewMember<'_>,
) -> Result<(), AppError> {
    let NewMember {
        user_id,
        role,
        doc_key_enc,
        invited_by,
    } = *member;
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    let inviter = id_to_uuid(*invited_by);

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;

    if current_role == Some(ambre_core::Role::Owner) && role != ambre_core::Role::Owner {
        ensure_not_last_owner(tx, *document_id).await?;
    }

    sqlx::query(
        "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (document_id, user_id) DO UPDATE SET role = $3, doc_key_enc = $4, invited_by = $5"
    )
    .bind(did)
    .bind(uid)
    .bind(role.as_str())
    .bind(doc_key_enc.as_bytes())
    .bind(inviter)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Fails with a 409 if the document has only one owner left. Callers hold the
/// target owner's row locked, so the count is stable for the transaction.
///
/// A document also carries an `owner_id` column that survives membership
/// changes. But every management operation (invite, revoke, role change)
/// requires a live `owner` membership row. Without one, nobody could manage
/// sharing anymore.
async fn ensure_not_last_owner(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
) -> Result<(), AppError> {
    let document_id = id_to_uuid(document_id);
    let owners: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM document_members WHERE document_id = $1 AND role = 'owner'",
    )
    .bind(document_id)
    .fetch_one(&mut **tx)
    .await?;
    if owners <= 1 {
        return Err(AppError::Conflict {
            detail: "cannot remove the last owner of the document".into(),
        });
    }
    Ok(())
}

/// Fetches the membership row that decides access to a document.
///
/// For a document of a channel, the row is the caller's on the channel's
/// `meta` document (`access_document!`).
pub async fn get_document_member(
    pool: &PgPool,
    document_id: &Id<Document>,
    user_id: &Id<User>,
) -> Result<Option<MemberRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    let row = sqlx::query(concat!(
        "SELECT role, doc_key_enc, invited_by FROM document_members \
         WHERE document_id = ",
        access_document!(),
        " AND user_id = $2"
    ))
    .bind(did)
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(MemberRow {
            role: decode_role(r.try_get("role")?)?,
            doc_key_enc: ambre_core::Encoded::new(r.try_get("doc_key_enc")?),
            invited_by: r.try_get::<PgOptId, _>("invited_by")?.into(),
        })),
        None => Ok(None),
    }
}

/// Lists the members who hold access to a document, with user details.
///
/// For a document of a channel, they are the members of the channel's `meta`
/// document, each with the channel key sealed for them (`access_document!`).
///
/// A member whose account holds no keys is left out. No wrap can address it,
/// so a key rotation that rewraps the listed members drops it (`SRV-163`).
/// Only a row written before the invite refused such an account can match.
pub async fn list_document_members(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<Vec<MemberWithUserRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(concat!(
        "SELECT dm.user_id, u.email, u.email_verified, u.display_name, dm.role, u.signing_pubkey, u.exchange_pubkey, dm.doc_key_enc, inv.exchange_pubkey AS wrapper_exchange_pubkey FROM document_members dm INNER JOIN users u ON u.id = dm.user_id LEFT JOIN users inv ON inv.id = dm.invited_by \
         WHERE u.signing_pubkey IS NOT NULL AND dm.document_id = ",
        access_document!()
    ))
    .bind(did)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(MemberWithUserRow {
                user_id: r.try_get::<PgId, _>("user_id")?.into(),
                email: ambre_core::Email::new(r.try_get("email")?),
                email_verified: ambre_core::Flag::from_bool(r.try_get("email_verified")?),
                display_name: decode_display_name(r.try_get("display_name")?)?,
                role: decode_role(r.try_get("role")?)?,
                signing_pubkey: decode_key(r.try_get("signing_pubkey")?)?,
                exchange_pubkey: decode_exchange_key(r.try_get("exchange_pubkey")?)?,
                doc_key_enc: ambre_core::Encoded::new(r.try_get("doc_key_enc")?),
                wrapper_exchange_pubkey: decode_opt_exchange_key(
                    r.try_get("wrapper_exchange_pubkey")?,
                )?,
            })
        })
        .collect()
}

/// Removes a user from a document's membership, with the read cursors the
/// membership held (`SRV-137`), and leaves the document owing a rotation
/// (`SRV-122`). Every key it held is one the member removed holds. The `meta`
/// of a public channel owes nothing: every member of the space reads its key
/// in the catalogue anyway.
///
/// Refuses to delete the document's only remaining owner (last-owner lockout):
/// with no owner left, nobody could manage sharing. The check and the delete
/// share one transaction with the target row locked so two concurrent removals
/// cannot both see two owners.
///
/// The `meta` of a channel that is not public is the one exception. Its last
/// member leaves, owner or not, and the channel goes with it (`SRV-142`). Each
/// of its documents is marked for its delete, at `sweep_due`, in the same
/// transaction. An owner who leaves other members behind is still refused.
pub async fn remove_document_member(
    pool: &PgPool,
    document_id: &Id<Document>,
    user_id: &Id<User>,
    entries: &RemovalEntries,
    sweep_due: Instant<UnixSec>,
) -> Result<Removed, AppError> {
    let mut tx = pool.begin().await?;
    lock_document(&mut tx, *document_id).await?;
    record_removal(&mut tx, *document_id, entries).await?;
    let channel = super::channels::channel_of_meta(&mut tx, *document_id).await?;
    if let Some(emptied) = delete_member(&mut tx, *document_id, *user_id, channel).await? {
        let deleted = super::channels::delete_channel_in(&mut tx, emptied, sweep_due).await?;
        tx.commit().await?;
        return Ok(Removed::ChannelDeleted(deleted));
    }
    if channel.is_some_and(|meta| meta.visibility == ambre_core::Visibility::Public) {
        tx.commit().await?;
        return Ok(Removed::Kept);
    }
    owe_rotation(&mut tx, *document_id).await?;
    let left = super::channels::leave_space_channels_in(
        &mut tx,
        Id::<Space>::from_document(*document_id),
        *user_id,
        sweep_due,
    )
    .await?;
    tx.commit().await?;
    Ok(left.map_or(Removed::Owed, Removed::SpaceLeft))
}

/// Marks a document as owing a rotation (`SRV-122`).
pub(super) async fn owe_rotation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
) -> Result<(), AppError> {
    let document_id = id_to_uuid(document_id);
    sqlx::query("UPDATE documents SET rotation_owed = true WHERE id = $1")
        .bind(document_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Removes a member and re-wraps the new key for every member who stays, in
/// one transaction (`ambre-spec-key-rotation.md`, `SYNC-126`). Either both
/// land or neither does: a removal never stands without its rotation.
///
/// A removal from a space takes the member out of every channel of it, in
/// the same transaction (`SRV-136`). Those channels are answered.
pub async fn revoke_and_rotate(
    pool: &PgPool,
    document_id: &Id<Document>,
    user_id: &Id<User>,
    rotation: &Rotation<'_>,
    entries: &RemovalEntries,
    sweep_due: Instant<UnixSec>,
) -> Result<Option<SpaceLeave>, AppError> {
    let mut tx = pool.begin().await?;
    // The document row first, as every rotation takes it: two rotations of one
    // document queue on it instead of deadlocking on the member rows.
    lock_document(&mut tx, *document_id).await?;
    record_removal(&mut tx, *document_id, entries).await?;
    delete_member(&mut tx, *document_id, *user_id, None).await?;
    rewrap_members(&mut tx, *document_id, rotation).await?;
    let left = super::channels::leave_space_channels_in(
        &mut tx,
        Id::<Space>::from_document(*document_id),
        *user_id,
        sweep_due,
    )
    .await?;
    tx.commit().await?;
    Ok(left)
}

/// Locks the document row for the rest of the transaction. A rotation reads
/// the last link under this lock, so two rotations cannot both chain from it.
pub(super) async fn lock_document(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
) -> Result<(), AppError> {
    let document_id = id_to_uuid(document_id);
    let found: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM documents WHERE id = $1 FOR UPDATE")
            .bind(document_id)
            .fetch_optional(&mut **tx)
            .await?;
    found.map(|_| ()).ok_or(AppError::NotFound {
        resource: crate::error::Resource::Document,
    })
}

/// Appends a rotation's link, inside the caller's transaction and under the
/// document lock (`SYNC-118`). A link must start from the last key chained
/// (`SRV-193`). Otherwise another rotation landed first, and its key is the
/// one to replace.
async fn append_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    link: &ambre_protocol::KeyLink,
) -> Result<(), AppError> {
    let document_id = id_to_uuid(document_id);
    let last: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT key_id FROM document_key_links WHERE document_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(document_id)
    .fetch_optional(&mut **tx)
    .await?;
    if last
        .map(decode_key_id)
        .transpose()?
        .is_some_and(|last| last != link.previous)
    {
        return Err(AppError::Conflict {
            detail: "the key this rotation replaces is no longer the current one".into(),
        });
    }
    let bytes = link
        .encode()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let stored = sqlx::query(
        "INSERT INTO document_key_links (document_id, key_id, previous_key_id, link) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(document_id)
    .bind(link.key.as_array().as_slice())
    .bind(link.previous.as_array().as_slice())
    .bind(bytes.as_bytes())
    .execute(&mut **tx)
    .await?;
    if stored.rows_affected() == 0 {
        return Err(AppError::Conflict {
            detail: "a link from this key is already stored".into(),
        });
    }
    Ok(())
}

/// Every link of a document, in the order the rotations landed (`SYNC-117`).
/// A channel document reads the links of its access document, the channel's
/// `meta`, where every document of the channel finds them.
pub async fn document_key_links(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<Vec<ambre_core::Encoded<ambre_core::EncodedKeyLink>>, AppError> {
    let rows = sqlx::query_scalar::<_, Vec<u8>>(concat!(
        "SELECT link FROM document_key_links WHERE document_id = ",
        access_document!(),
        " ORDER BY seq"
    ))
    .bind(id_to_uuid(*document_id))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ambre_core::Encoded::new).collect())
}

/// Whether the document owes a rotation (`SRV-122`).
///
/// A document of a channel is sealed under the key of its `meta`, so it owes
/// the rotation its `meta` owes. A removal marks the `meta` alone, and the
/// relay then refuses each frame of the segment under the key the removed
/// member holds (`SRV-302`).
pub async fn rotation_owed(pool: &PgPool, document_id: &Id<Document>) -> Result<bool, AppError> {
    let owed: Option<bool> = sqlx::query_scalar(concat!(
        "SELECT d.rotation_owed OR a.rotation_owed FROM documents d, documents a \
         WHERE d.id = $1 AND a.id = ",
        access_document!()
    ))
    .bind(id_to_uuid(*document_id))
    .fetch_optional(pool)
    .await?;
    Ok(owed.unwrap_or(false))
}

/// Deletes one member row with its read cursors, refusing the last owner. The
/// target row is locked so two concurrent removals cannot both see two owners.
///
/// `channel` is the channel whose `meta` the document is, with its
/// visibility. The last-owner check holds for neither the last member of a channel
/// nor a public channel. A channel that is not public is answered for its delete
/// (`SRV-142`); a public one stays with its documents (`SRV-173`).
async fn delete_member(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    user_id: Id<User>,
    channel: Option<super::channels::ChannelMeta>,
) -> Result<Option<Id<Channel>>, AppError> {
    let did = id_to_uuid(document_id);
    let uid = id_to_uuid(user_id);
    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;
    // The lock on the document serializes every removal from it, so the
    // count holds until the commit.
    let empties_channel = match channel {
        Some(_) if current_role.is_some() => {
            let members: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM document_members WHERE document_id = $1")
                    .bind(did)
                    .fetch_one(&mut **tx)
                    .await?;
            members == 1
        }
        _ => false,
    };
    let empties_private = match channel {
        Some(super::channels::ChannelMeta {
            id,
            visibility: ambre_core::Visibility::Private,
        }) if empties_channel => Some(id),
        _ => None,
    };
    // A public channel outlives its last member: the space reads its key in
    // the catalogue, so the next member joins it alone (`SRV-173`). It
    // outlives its last owner too: an owner of the space stands in for one
    // (`SRV-174`).
    let public = channel.is_some_and(|meta| meta.visibility == ambre_core::Visibility::Public);
    if current_role == Some(ambre_core::Role::Owner) && !empties_channel && !public {
        ensure_not_last_owner(tx, document_id).await?;
    }

    drop_member_row(tx, document_id, user_id).await?;
    Ok(empties_private)
}

/// Deletes one member row with its read cursors, and checks nothing.
pub(super) async fn drop_member_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    user_id: Id<User>,
) -> Result<(), AppError> {
    let did = id_to_uuid(document_id);
    let uid = id_to_uuid(user_id);
    sqlx::query("DELETE FROM document_members WHERE document_id = $1 AND user_id = $2")
        .bind(did)
        .bind(uid)
        .execute(&mut **tx)
        .await?;
    super::read_cursors::delete_read_cursors_of(tx, document_id, user_id).await
}

/// Updates the role of a document member.
///
/// Enforces the last-owner invariant: demoting the document's only owner is
/// refused with a 409 (see `remove_document_member`).
pub async fn update_document_member_role(
    pool: &PgPool,
    document_id: &Id<Document>,
    user_id: &Id<User>,
    role: ambre_core::Role,
    entry: Option<&LogEntry>,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);

    let mut tx = pool.begin().await?;
    lock_document(&mut tx, *document_id).await?;
    record_change(&mut tx, *document_id, entry).await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;

    if current_role == Some(ambre_core::Role::Owner) && role != ambre_core::Role::Owner {
        ensure_not_last_owner(&mut tx, *document_id).await?;
    }

    sqlx::query("UPDATE document_members SET role = $3 WHERE document_id = $1 AND user_id = $2")
        .bind(did)
        .bind(uid)
        .bind(role.as_str())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Atomically replaces all `doc_key_enc` values for a document inside a transaction.
///
/// Called after key rotation: the caller computes new encrypted keys for
/// each remaining member and this function applies them all at once.
pub async fn rotate_document_keys(
    pool: &PgPool,
    document_id: &Id<Document>,
    rotation: &Rotation<'_>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    lock_document(&mut tx, *document_id).await?;
    rewrap_members(&mut tx, *document_id, rotation).await?;
    tx.commit().await?;
    Ok(())
}

/// One rotation of a document key. It names who rotates, the new key wrapped
/// for each member who stays, the members named, and the link that signs it.
pub struct Rotation<'a> {
    pub rotated_by: &'a Id<User>,
    pub keys: &'a [(Id<User>, ambre_core::Encoded<ambre_core::WrappedDocKey>)],
    pub members: &'a [Id<User>],
    pub link: &'a ambre_protocol::KeyLink,
}

/// Stores the rotation's link and writes each member's new wrapped copy. It
/// clears the document's debt (`SRV-124`, `SYNC-118`), inside the caller's
/// transaction and under its document lock.
///
/// `members` are the members the rotation read before it wrapped. A member
/// outside them joined since and holds no copy of the new key. It would keep
/// the one the relay now refuses, so the rotation is refused whole (`SRV-167`).
/// So is a rotation whose `keys` leave out a current member (`SRV-299`).
async fn rewrap_members(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: Id<Document>,
    rotation: &Rotation<'_>,
) -> Result<(), AppError> {
    let Rotation {
        rotated_by,
        keys,
        members,
        link,
    } = *rotation;
    let did = id_to_uuid(document_id);
    let rotator_uuid = id_to_uuid(*rotated_by);
    // The same rows the member list serves (`SRV-163`): an account without
    // keys is in neither, so it cannot hold a rotation back.
    let current: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT dm.user_id FROM document_members dm INNER JOIN users u ON u.id = dm.user_id \
         WHERE dm.document_id = $1 AND u.signing_pubkey IS NOT NULL",
    )
    .bind(did)
    .fetch_all(&mut **tx)
    .await?;
    let read: std::collections::HashSet<uuid::Uuid> =
        members.iter().map(|m| id_to_uuid(*m)).collect();
    if current.iter().any(|m| !read.contains(m)) {
        return Err(AppError::MembersChanged);
    }
    // Each member who stays takes a copy of the new key. A member left out
    // keeps a copy of the retired key alone, and loses the document from this
    // rotation on. The debt would still read settled (`SRV-299`).
    let copied: std::collections::HashSet<uuid::Uuid> =
        keys.iter().map(|(user, _)| id_to_uuid(*user)).collect();
    if current.iter().any(|m| !copied.contains(m)) {
        return Err(AppError::BadRequest(
            "a rotation carries a copy for every member".into(),
        ));
    }
    append_link(tx, document_id, link).await?;
    // The rotator re-wrapped every key with ITS exchange key, so it becomes the
    // new wrapper of record. Update `invited_by` alongside `doc_key_enc`, or
    // the ECDH unwrap would later use the stale inviter's key.
    for (user_id, key_enc) in keys {
        let uid = id_to_uuid(*user_id);
        sqlx::query("UPDATE document_members SET doc_key_enc = $3, invited_by = $4 WHERE document_id = $1 AND user_id = $2")
            .bind(did)
            .bind(uid)
            .bind(key_enc.as_bytes())
            .bind(rotator_uuid)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("UPDATE documents SET rotation_owed = false WHERE id = $1")
        .bind(did)
        .execute(&mut **tx)
        .await?;
    // A channel's key is its `meta`'s. Every segment still open was sealed
    // under the key this rotation replaces, so each is locked once it is
    // closing (`SRV-147`).
    sqlx::query(
        "UPDATE channel_documents s SET key_retired = true FROM channel_documents m \
         WHERE m.document_id = $1 AND m.kind = 'meta' AND s.channel_id = m.channel_id \
           AND s.kind = 'segment' AND s.state IN ('active', 'closing')",
    )
    .bind(did)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
