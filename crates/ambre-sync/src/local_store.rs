//! The seal of the local store (`ambre-spec-residency.md` section 3).
//!
//! Two service requests never touch the relay, like the `.ambre` file's.
//! [`RestCommand::SealLocalRow`] seals one row for the browser's database, and
//! [`RestCommand::OpenLocalRows`] opens rows read back from it. The Main thread
//! performs the reads and the writes and moves only sealed bytes.
//!
//! The key is derived from the session's seed for each request and dropped at
//! its end. It is never stored and never leaves this worker (`RESID-025`,
//! `RESID-026`). Without a seed both answer [`RestEvent::LocalStoreLocked`].
use ambre_bus::{RowPlace, SealedRow};

use super::*;

/// The associated data of a row: where it sits in the store.
fn place_bytes(place: &RowPlace) -> ambre_core::Aad {
    ambre_core::Aad::new(match place {
        RowPlace::Document(id) => [b"doc:".as_slice(), id.as_bytes()].concat(),
        RowPlace::Draft(key) => [b"draft:".as_slice(), key.as_str().as_bytes()].concat(),
    })
}

impl SyncState {
    /// Seals one row under the store key.
    pub(crate) fn handle_seal_local_row(
        &self,
        id: RequestId,
        place: RowPlace,
        plaintext: SecretBytes,
    ) {
        let Some(seed) = self.master_seed.as_ref() else {
            self.reply(id, RestEvent::LocalStoreLocked);
            return;
        };
        let key = seed.derive_store_key();
        match ambre_crypto::aead::seal_row(&plaintext, &place_bytes(&place), &key) {
            Ok(sealed) => self.reply(id, RestEvent::LocalRowSealed { sealed }),
            // Past the AEAD size limit: nothing is stored, as for a seed that
            // is missing.
            Err(_) => {
                tracing::warn!("Sync: SealLocalRow: seal failed");
                self.reply(id, RestEvent::LocalStoreLocked);
            }
        }
    }

    /// Opens rows under the store key, each at its own place. A row that does
    /// not open answers `None` in its slot (`RESID-027`).
    pub(crate) fn handle_open_local_rows(&self, id: RequestId, rows: Vec<SealedRow>) {
        let Some(seed) = self.master_seed.as_ref() else {
            self.reply(id, RestEvent::LocalStoreLocked);
            return;
        };
        let key = seed.derive_store_key();
        let rows = rows
            .iter()
            .map(|row| {
                ambre_crypto::aead::open_row(&row.sealed, &place_bytes(&row.place), &key).ok()
            })
            .collect();
        self.reply(id, RestEvent::LocalRowsOpened { rows });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A worker whose session holds `seed`, and what it answers.
    fn worker(seed: Option<[u8; 32]>) -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, crate::RelayHost::new(String::new()));
        state.master_seed = seed.map(MasterSeed::from_array);
        (state, rx)
    }

    fn answer(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> RestEvent {
        match rx.try_iter().last() {
            Some(LogicEvent::Rest { evt, .. }) => evt,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn seal(
        state: &mut SyncState,
        rx: &ambre_bus::conduit::Receiver<LogicEvent>,
        place: RowPlace,
        plain: &[u8],
    ) -> ambre_core::SealedRowBytes {
        state.handle_rest(
            RequestId(1),
            RestCommand::SealLocalRow {
                place,
                plaintext: SecretBytes::new(plain.to_vec()),
            },
        );
        match answer(rx) {
            RestEvent::LocalRowSealed { sealed } => sealed,
            other => panic!("expected a sealed row, got {other:?}"),
        }
    }

    fn open(
        state: &mut SyncState,
        rx: &ambre_bus::conduit::Receiver<LogicEvent>,
        rows: Vec<SealedRow>,
    ) -> RestEvent {
        state.handle_rest(RequestId(2), RestCommand::OpenLocalRows { rows });
        answer(rx)
    }

    /// A row leaves sealed under a key the seed derives, so the stored bytes
    /// carry no plaintext. The same seed opens them on a later session
    /// (`RESID-025`).
    #[test]
    fn a_row_is_sealed_under_the_seed_and_opens_on_a_later_session() {
        let doc = DocumentId::new();
        let (mut state, rx) = worker(Some([9; 32]));
        let sealed = seal(
            &mut state,
            &rx,
            RowPlace::Document(doc),
            b"the whole document",
        );
        assert!(
            !sealed.as_bytes().windows(5).any(|w| w == b"whole"),
            "plaintext reached the store"
        );

        let (mut later, rx) = worker(Some([9; 32]));
        let row = SealedRow {
            place: RowPlace::Document(doc),
            sealed,
        };
        let RestEvent::LocalRowsOpened { rows } = open(&mut later, &rx, vec![row]) else {
            panic!("expected opened rows");
        };
        let [Some(plain)] = rows.as_slice() else {
            panic!("the row did not open: {rows:?}");
        };
        assert_eq!(plain.to_plaintext(), b"the whole document");
    }

    /// Each row opens on its own. Another user's row, one moved to another
    /// document and one stored in the clear answer `None` in their slot. The
    /// good row beside them still opens (`RESID-027`).
    #[test]
    fn a_row_that_does_not_open_answers_none_in_its_slot() {
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let (mut stranger, rx) = worker(Some([3; 32]));
        let foreign = seal(&mut stranger, &rx, RowPlace::Document(doc), b"theirs");
        let (mut state, rx) = worker(Some([9; 32]));
        let mine = seal(
            &mut state,
            &rx,
            RowPlace::Draft(ambre_bus::DraftKey::new("u|c".into())),
            b"a draft",
        );
        let moved = seal(&mut state, &rx, RowPlace::Document(other), b"elsewhere");
        let rows = vec![
            SealedRow {
                place: RowPlace::Document(doc),
                sealed: foreign,
            },
            SealedRow {
                place: RowPlace::Document(doc),
                sealed: moved,
            },
            SealedRow {
                place: RowPlace::Document(doc),
                sealed: ambre_core::SealedRowBytes::new(b"in the clear".to_vec()),
            },
            SealedRow {
                place: RowPlace::Draft(ambre_bus::DraftKey::new("u|c".into())),
                sealed: mine,
            },
        ];
        let RestEvent::LocalRowsOpened { rows } = open(&mut state, &rx, rows) else {
            panic!("expected opened rows");
        };
        let opened: Vec<Option<Vec<u8>>> = rows
            .iter()
            .map(|r| r.as_ref().map(SecretBytes::to_plaintext))
            .collect();
        assert_eq!(opened, [None, None, None, Some(b"a draft".to_vec())]);
    }

    /// Without a session there is no key: nothing is sealed or opened.
    #[test]
    fn without_a_session_the_store_is_locked() {
        let (mut state, rx) = worker(None);
        state.handle_rest(
            RequestId(1),
            RestCommand::SealLocalRow {
                place: RowPlace::Document(DocumentId::new()),
                plaintext: SecretBytes::new(b"x".to_vec()),
            },
        );
        assert!(matches!(answer(&rx), RestEvent::LocalStoreLocked));
        assert!(matches!(
            open(&mut state, &rx, Vec::new()),
            RestEvent::LocalStoreLocked
        ));
    }
}
