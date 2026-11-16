//! The files a delete asks the relay to erase, until it has
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! A delete of a message of this user's asks for each file it alone carried
//! (`ambre-spec-murmure-attachments.md`). The relay drops this user's claim,
//! and the blob goes with the last one, so a file another member uploaded
//! stays. An
//! erase that does not reach the relay waits, and the next connection or
//! session asks again. The queue lives in memory: a reload
//! before the retry leaves the blob to the channel's deletion.

use super::*;
use ambre_bus::{ContentHash, RestCommand, RestEvent};
use ambre_core::Validated;

impl<D> Runtime<D> {
    /// Ask the relay to erase each of `files`, stored under `document_id`.
    pub(crate) fn erase_files(
        &mut self,
        document_id: Id<Document>,
        files: Vec<Validated<ContentHash>>,
    ) {
        for file in files {
            self.erase(document_id, file);
        }
    }

    fn erase(
        &mut self,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    ) {
        let id = ambre_bus::next_request();
        self.erasing.insert(id, (document_id, content_hash.clone()));
        self.ask(SyncCommand::Rest {
            id,
            cmd: RestCommand::EraseMedia {
                document_id,
                content_hash,
            },
        });
    }

    /// The answer to an erase this thread asked for, which no view awaits.
    /// Returns `false` for any other answer.
    pub fn erased(&mut self, id: Seq<Request>, evt: &RestEvent) -> bool {
        if self.erasing.remove(&id).is_none() {
            return false;
        }
        if !matches!(evt, RestEvent::MediaErased { .. }) {
            tracing::warn!("Logic: an erase answered {evt:?}");
        }
        true
    }

    /// An erase that did not reach the relay waits for the next connection
    /// (`ambre-spec-murmure-attachments.md`). Returns `false` for any other
    /// request.
    pub fn erase_failed(&mut self, id: Seq<Request>) -> bool {
        let Some(erase) = self.erasing.remove(&id) else {
            return false;
        };
        self.erase_owed.push(erase);
        true
    }

    /// Ask again for every erase that waits.
    pub fn retry_erases(&mut self) {
        for (document_id, content_hash) in std::mem::take(&mut self.erase_owed) {
            self.erase(document_id, content_hash);
        }
    }
}
