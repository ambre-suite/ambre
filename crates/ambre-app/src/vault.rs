//! The seal of the local store's rows, as the Main thread asks for it
//! (`ambre-spec-residency.md` section 3).
//!
//! The Sync worker holds the key and seals each row on request. This thread
//! moves the sealed bytes to the store and back, and never holds the key
//! (`RESID-025`, `RESID-026`). Requests ride the service plane, so Logic
//! relays them verbatim, and their answers are taken off the switchboard
//! before any view sees them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ambre_core::{Payload, SealedRowBytes};

use ambre_bus::{Confidential, Plaintext, Request, RestCommand, RestEvent, RowPlace, SealedRow};
use ambre_core::Seq;
use ambre_logic::app::LogicCommand;

use crate::outbox::ShellOutbox;

/// What rows read back open to, one slot each: `None` when the store is
/// locked.
///
/// Written out at its uses, the type fails `clippy::type_complexity`.
type Opened = Box<dyn FnOnce(Option<Vec<Option<Confidential<Plaintext>>>>)>;

enum Pending {
    /// What one sealed row comes back as: `None` when the store is locked.
    Seal(Box<dyn FnOnce(Option<Payload<SealedRowBytes>>)>),
    Open(Opened),
}

/// The requests to the Sync worker's seal, and what each answer goes to.
#[derive(Clone)]
pub(crate) struct Vault {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    /// The launcher's outbox, which holds a request the channel to Logic did
    /// not take, in order with every other launcher command (`BUS-011`).
    to_logic: ShellOutbox,
    waiting: HashMap<Seq<Request>, Pending>,
}

impl Vault {
    pub(crate) fn new(to_logic: ShellOutbox) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                to_logic,
                waiting: HashMap::new(),
            })),
        }
    }

    /// Seal `plaintext` for its `place`, and hand `done` the sealed bytes.
    pub(crate) fn seal(
        &self,
        place: RowPlace,
        plaintext: Confidential<Plaintext>,
        done: impl FnOnce(Option<Payload<SealedRowBytes>>) + 'static,
    ) {
        let cmd = RestCommand::SealLocalRow { place, plaintext };
        self.ask(cmd, Pending::Seal(Box::new(done)));
    }

    /// Open `rows` read back from the store, and hand `done` one slot per
    /// row. A row that does not open is `None` in its slot (`RESID-027`).
    pub(crate) fn open(
        &self,
        rows: Vec<SealedRow>,
        done: impl FnOnce(Option<Vec<Option<Confidential<Plaintext>>>>) + 'static,
    ) {
        if rows.is_empty() {
            done(Some(Vec::new()));
            return;
        }
        self.ask(
            RestCommand::OpenLocalRows { rows },
            Pending::Open(Box::new(done)),
        );
    }

    /// Send one request. A request the outbox refuses answers `None` at once,
    /// as a locked store does, and so does one no Logic thread will read. Its
    /// caller then keeps the row for a later turn (`BUS-018`).
    fn ask(&self, cmd: RestCommand, pending: Pending) {
        let id = ambre_bus::next_request();
        let to_logic = self.inner.borrow().to_logic.clone();
        self.inner.borrow_mut().waiting.insert(id, pending);
        // The borrow ends before the send, and before an answer that may ask
        // again.
        if !to_logic.send(LogicCommand::Rest { id, cmd }).accepted() {
            let _ = self.answer(Some(id), RestEvent::LocalStoreLocked);
        }
    }

    /// Take the answer to one of this vault's requests, by value. Each opened
    /// row then reaches `done` with one owner, so it moves out of its wrapper
    /// with no copy. An event that is not the vault's comes back for the rest
    /// of the relay.
    pub(crate) fn answer(&self, id: Option<Seq<Request>>, evt: RestEvent) -> Option<RestEvent> {
        let pending = id.and_then(|id| self.inner.borrow_mut().waiting.remove(&id));
        let Some(pending) = pending else {
            return Some(evt);
        };
        // The borrow ends before the answer runs, which may ask again.
        match (pending, evt) {
            (Pending::Seal(done), RestEvent::LocalRowSealed { sealed }) => done(Some(sealed)),
            (Pending::Open(done), RestEvent::LocalRowsOpened { rows }) => done(Some(rows)),
            (Pending::Seal(done), _) => done(None),
            (Pending::Open(done), _) => done(None),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use ambre_bus::conduit::{self, Receiver};
    use ambre_core::id::{Document, Id};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// The one request on `rx`, as Logic would relay it to Sync.
    fn request(rx: &Receiver<LogicCommand>) -> (Seq<Request>, RestCommand) {
        match rx.try_recv() {
            Ok(LogicCommand::Rest { id, cmd }) => (id, cmd),
            other => panic!("expected a service request, got {other:?}"),
        }
    }

    /// A row leaves for the Sync worker to seal, and its sealed bytes come
    /// back to the one who asked. This thread holds no key, and the answer
    /// is the vault's, so no view sees it (`RESID-026`).
    #[wasm_bindgen_test]
    fn a_seal_asks_the_worker_and_takes_its_answer() {
        let (tx, rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let vault = Vault::new(ShellOutbox::new(tx));
        let got = Rc::new(RefCell::new(None));
        let doc = Id::<Document>::mint();
        vault.seal(
            RowPlace::Document(doc),
            Confidential::<Plaintext>::new(b"plain".to_vec()),
            {
                let got = Rc::clone(&got);
                move |sealed| *got.borrow_mut() = Some(sealed)
            },
        );
        let (id, cmd) = request(&rx);
        let RestCommand::SealLocalRow { place, plaintext } = cmd else {
            panic!("expected a seal, got {cmd:?}");
        };
        assert_eq!(place, RowPlace::Document(doc));
        assert_eq!(plaintext.into_plaintext(), b"plain");

        assert!(
            vault
                .answer(Some(Seq::nth(u64::MAX)), RestEvent::LocalStoreLocked)
                .is_some()
        );
        let sealed = RestEvent::LocalRowSealed {
            sealed: Payload::<SealedRowBytes>::new(b"sealed".to_vec()),
        };
        assert!(vault.answer(Some(id), sealed.clone()).is_none());
        assert_eq!(
            *got.borrow(),
            Some(Some(Payload::<SealedRowBytes>::new(b"sealed".to_vec())))
        );
        assert!(
            vault.answer(Some(id), sealed).is_some(),
            "an answer is taken once"
        );
    }

    /// Without a session the store is locked: the seal answers nothing and
    /// the rows read back open to nothing.
    #[wasm_bindgen_test]
    fn a_locked_store_answers_nothing() {
        let (tx, rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let vault = Vault::new(ShellOutbox::new(tx));
        let sealed = Rc::new(Cell::new(false));
        let opened = Rc::new(Cell::new(false));
        vault.seal(
            RowPlace::Draft(ambre_bus::DraftKey::new("k".into())),
            Confidential::<Plaintext>::new(b"x".to_vec()),
            {
                let sealed = Rc::clone(&sealed);
                move |row| sealed.set(row.is_none())
            },
        );
        let row = SealedRow {
            place: RowPlace::Draft(ambre_bus::DraftKey::new("k".into())),
            sealed: Payload::<SealedRowBytes>::new(vec![1]),
        };
        vault.open(vec![row], {
            let opened = Rc::clone(&opened);
            move |rows| opened.set(rows.is_none())
        });
        for _ in 0..2 {
            let (id, _) = request(&rx);
            assert!(
                vault
                    .answer(Some(id), RestEvent::LocalStoreLocked)
                    .is_none()
            );
        }
        assert!(sealed.get() && opened.get());
    }

    /// An opened row reaches the one who asked with one owner. So it moves
    /// out of its wrapper with no copy, and its bytes keep their address.
    #[wasm_bindgen_test]
    fn an_opened_row_moves_out_with_no_copy() {
        let (tx, rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let vault = Vault::new(ShellOutbox::new(tx));
        let moved = Rc::new(Cell::new(None));
        let row = SealedRow {
            place: RowPlace::Draft(ambre_bus::DraftKey::new("k".into())),
            sealed: Payload::<SealedRowBytes>::new(vec![1]),
        };
        vault.open(vec![row], {
            let moved = Rc::clone(&moved);
            move |rows| {
                let row = rows.and_then(|rows| rows.into_iter().next().flatten());
                moved.set(row.map(|row| row.into_plaintext()));
            }
        });
        let (id, _) = request(&rx);
        let opened = Confidential::<Plaintext>::new(b"the row".to_vec());
        let before = opened.with_plaintext(<[u8]>::as_ptr);
        let answer = RestEvent::LocalRowsOpened {
            rows: vec![Some(opened)],
        };
        assert!(vault.answer(Some(id), answer).is_none());
        assert_eq!(moved.take().map(|bytes| bytes.as_ptr()), Some(before));
    }

    /// A request the channel to Logic does not take waits in the launcher's
    /// outbox, and leaves in order on the relay's flush.
    #[wasm_bindgen_test]
    fn a_full_channel_holds_the_requests_in_order() {
        let (tx, rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<1>());
        let outbox = ShellOutbox::new(tx);
        let vault = Vault::new(outbox.clone());
        for key in ["a", "b"] {
            vault.seal(
                RowPlace::Draft(ambre_bus::DraftKey::new(key.into())),
                Confidential::<Plaintext>::new(Vec::new()),
                |_| {},
            );
        }
        let place = |rx: &Receiver<LogicCommand>| match request(rx).1 {
            RestCommand::SealLocalRow { place, .. } => place,
            other => panic!("expected a seal, got {other:?}"),
        };
        assert_eq!(
            place(&rx),
            RowPlace::Draft(ambre_bus::DraftKey::new("a".into()))
        );
        assert!(rx.try_recv().is_err(), "the second waits for room");
        outbox.flush();
        assert_eq!(
            place(&rx),
            RowPlace::Draft(ambre_bus::DraftKey::new("b".into()))
        );
    }

    /// No rows to open answers at once, with no request.
    #[wasm_bindgen_test]
    fn no_rows_open_at_once() {
        let (tx, rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let vault = Vault::new(ShellOutbox::new(tx));
        let opened = Rc::new(Cell::new(false));
        vault.open(Vec::new(), {
            let opened = Rc::clone(&opened);
            move |rows| opened.set(rows.is_some_and(|rows| rows.is_empty()))
        });
        assert!(opened.get());
        assert!(rx.try_recv().is_err());
    }
}
