// ambre-server/src/deletion.rs
// The delete of a document, and the pass that finishes one a failure stopped.

use crate::error::AppError;
use crate::state::AppState;
use ambre_core::id::{Document, Id};
use ambre_core::{Instant, Sec, TimeSpan, UnixSec};

/// How many marked deletes one pass works through.
const PASS_LIMIT: ambre_core::Bounded<crate::db::QueryLimit> =
    ambre_core::Bounded::<crate::db::QueryLimit>::of::<100>();

/// Begins the delete of a document, then runs it. The mark in the register
/// goes first, so a failure at any later step leaves a delete the pass finishes
/// (`SRV-165`). It also schedules the second sweep of the objects (`SRV-057`).
///
/// A space goes with every channel of it, the channels first (`SRV-143`).
/// Each member of each channel drops it (`SRV-188`), and each member of the
/// space drops the space (`SRV-218`).
///
/// # Errors
///
/// The first step that fails. The mark stays, so the delete still completes.
pub async fn delete_document(state: &AppState, document_id: &Id<Document>) -> Result<(), AppError> {
    let sweep_due = sweep_due(state);
    let space_members = crate::db::space_members(&state.db, document_id).await?;
    let channels = crate::db::begin_deletion(&state.db, document_id, sweep_due).await?;
    for member in &space_members {
        crate::routes::notify_removed(state, document_id, member).await;
    }
    for channel in &channels {
        empty_marked(state, &channel.documents, sweep_due).await;
        let Some(meta) = channel.meta else { continue };
        for member in &channel.members {
            crate::routes::notify_removed(state, &meta, member).await;
        }
    }
    empty_document(state, document_id, sweep_due).await
}

/// When the second sweep of a delete begun now is due (`SRV-057`).
pub fn sweep_due(state: &AppState) -> Instant<UnixSec> {
    let window = crate::s3::late_write_window(state.config.presigned_url_ttl);
    crate::s3::unix_now().plus(window)
}

/// Empties documents whose delete a transaction already marked, as a deleted
/// channel marks each of its documents (`SRV-142`). A document that fails
/// keeps its mark, and the pass finishes it (`SRV-165`).
pub async fn empty_marked(
    state: &AppState,
    documents: &[Id<Document>],
    sweep_due: Instant<UnixSec>,
) {
    for doc in documents {
        if let Err(e) = empty_document(state, doc, sweep_due).await {
            tracing::warn!(error = %e, "a marked delete is left to the pass");
        }
    }
}

/// Fences a document, then empties it. The tombstone goes first: from then on
/// no update, presence or flush writes a key of the document. The objects and
/// the relay state go next, and the rows last. Forgetting the rows first would
/// strand what a failed purge missed, with nothing left pointing at it. Every
/// step may run again, so a pass repeats the whole of it (`SRV-017`).
async fn empty_document(
    state: &AppState,
    document_id: &Id<Document>,
    sweep_due: Instant<UnixSec>,
) -> Result<(), AppError> {
    let ttl = fence_ttl(sweep_due, crate::s3::unix_now());
    crate::redis_ops::mark_document_gone(&state.redis, document_id, ttl).await?;
    let prefix = crate::s3::format_s3_document_prefix(&state.config.s3_env_prefix, document_id);
    crate::s3::delete_prefix(&state.s3, &state.s3_bucket, &prefix).await?;
    crate::redis_ops::purge_document_state(&state.redis, document_id, ttl).await?;
    crate::db::delete_document(&state.db, document_id).await
}

/// Finishes every delete the register still marks. A delete a failure stopped
/// is run again from its first step. One whose rows are gone has its tombstone
/// set again, since a reset of the ephemeral store loses it. Once its window
/// has passed, its objects are swept a second time and its mark goes.
/// A step that fails keeps the mark, and the next pass retries it.
pub async fn finish_deletions(state: &AppState) {
    let pending = match crate::db::pending_deletions(&state.db, PASS_LIMIT).await {
        Ok(pending) => pending,
        Err(e) => {
            tracing::error!("pending deletions read failed: {e}");
            return;
        }
    };
    for d in pending {
        let doc = d.document_id;
        if let Err(e) = finish_one(state, &doc, d.sweep_due, d.rows_left).await {
            tracing::error!(error = %e, "unfinished delete failed again");
        }
    }
}

/// One pending delete, one pass.
async fn finish_one(
    state: &AppState,
    document_id: &Id<Document>,
    sweep_due: Instant<UnixSec>,
    rows_left: crate::db::DocumentRows,
) -> Result<(), AppError> {
    let now = crate::s3::unix_now();
    if rows_left == crate::db::DocumentRows::Left {
        empty_document(state, document_id, sweep_due).await?;
        tracing::debug!(document_id = %document_id, "an unfinished delete completed");
    }
    if now < sweep_due {
        let ttl = fence_ttl(sweep_due, now);
        return crate::redis_ops::mark_document_gone(&state.redis, document_id, ttl).await;
    }
    let prefix = crate::s3::format_s3_document_prefix(&state.config.s3_env_prefix, document_id);
    let count = crate::s3::delete_prefix(&state.s3, &state.s3_bucket, &prefix).await?;
    if !count.is_zero() {
        tracing::debug!(document_id = %document_id, count = count.get(), "second sweep removed late objects");
    }
    crate::db::end_deletion(&state.db, document_id, now).await
}

/// How long the tombstone must still live: until the sweep is due, and never
/// less than a second, which Redis refuses.
fn fence_ttl(sweep_due: Instant<UnixSec>, now: Instant<UnixSec>) -> TimeSpan<Sec> {
    sweep_due.since(now).max(TimeSpan::<Sec>::new(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_lives_until_the_sweep_and_never_less_than_a_second() {
        use ambre_core::{Instant, Sec, TimeSpan, UnixSec};
        let at = Instant::<UnixSec>::new;
        assert_eq!(fence_ttl(at(100), at(40)), TimeSpan::<Sec>::new(60));
        assert_eq!(fence_ttl(at(100), at(100)), TimeSpan::<Sec>::new(1));
        assert_eq!(fence_ttl(at(100), at(500)), TimeSpan::<Sec>::new(1));
    }
}
