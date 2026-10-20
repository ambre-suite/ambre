//! The seal of the local store's rows, as the Main thread asks for it
//! (`ambre-spec-residency.md` section 3).
//!
//! The Sync worker holds the key and seals each row on request. This thread
//! moves the sealed bytes to the store and back, and never holds the key
//! (`RESID-025`, `RESID-026`). Requests ride the service plane, so Logic
//! relays them verbatim, and their answers are taken off the per-frame relay
//! before any view sees them.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use ambre_bus::conduit::{Sender, TrySendError};
use ambre_bus::{
    LogicCommand, RequestId, RestCommand, RestEvent, RowPlace, SealedRow, SecretBytes,
};

/// What one sealed row comes back as: `None` when the store is locked.
type Sealed = Box<dyn FnOnce(Option<Vec<u8>>)>;
/// What rows read back open to, one slot each: `None` when the store is
/// locked.
type Opened = Box<dyn FnOnce(Option<Vec<Option<SecretBytes>>>)>;

enum Pending {
    Seal(Sealed),
    Open(Opened),
}

/// The requests to the Sync worker's seal, and what each answer goes to.
#[derive(Clone)]
pub(crate) struct Vault {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    to_logic: Sender<LogicCommand>,
    waiting: HashMap<RequestId, Pending>,
    /// Requests the channel to Logic did not take, in order.
    outbox: VecDeque<LogicCommand>,
}

impl Vault {
    pub(crate) fn new(to_logic: Sender<LogicCommand>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                to_logic,
                waiting: HashMap::new(),
                outbox: VecDeque::new(),
            })),
        }
    }

    /// Seal `plaintext` for its `place`, and hand `done` the sealed bytes.
    pub(crate) fn seal(
        &self,
        place: RowPlace,
        plaintext: Vec<u8>,
        done: impl FnOnce(Option<Vec<u8>>) + 'static,
    ) {
        let cmd = RestCommand::SealLocalRow {
            place,
            plaintext: SecretBytes::new(plaintext),
        };
        self.ask(cmd, Pending::Seal(Box::new(done)));
    }

    /// Open `rows` read back from the store, and hand `done` one slot per
    /// row. A row that does not open is `None` in its slot (`RESID-027`).
    pub(crate) fn open(
        &self,
        rows: Vec<SealedRow>,
        done: impl FnOnce(Option<Vec<Option<SecretBytes>>>) + 'static,
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

    fn ask(&self, cmd: RestCommand, pending: Pending) {
        let id = RequestId::next();
        let mut inner = self.inner.borrow_mut();
        inner.waiting.insert(id, pending);
        inner.outbox.push_back(LogicCommand::Rest { id, cmd });
        inner.send();
    }

    /// Offer again what the channel to Logic did not take.
    pub(crate) fn flush(&self) {
        self.inner.borrow_mut().send();
    }

    /// Take the answer to one of this vault's requests. `false` leaves the
    /// event to the rest of the relay.
    pub(crate) fn answer(&self, id: RequestId, evt: &RestEvent) -> bool {
        let Some(pending) = self.inner.borrow_mut().waiting.remove(&id) else {
            return false;
        };
        // The borrow ends before the answer runs, which may ask again.
        match (pending, evt) {
            (Pending::Seal(done), RestEvent::LocalRowSealed { sealed }) => {
                done(Some(sealed.clone()))
            }
            (Pending::Open(done), RestEvent::LocalRowsOpened { rows }) => done(Some(rows.clone())),
            (Pending::Seal(done), _) => done(None),
            (Pending::Open(done), _) => done(None),
        }
        true
    }
}

impl Inner {
    fn send(&mut self) {
        while let Some(cmd) = self.outbox.pop_front() {
            match self.to_logic.try_send(cmd) {
                Ok(()) => {}
                Err(TrySendError::Full(cmd)) => {
                    self.outbox.push_front(cmd);
                    return;
                }
                // Logic is gone, and no answer will come.
                Err(TrySendError::Disconnected(_)) => {
                    self.outbox.clear();
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use ambre_bus::DocumentId;
    use ambre_bus::conduit::{self, Receiver};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// The one request on `rx`, as Logic would relay it to Sync.
    fn request(rx: &Receiver<LogicCommand>) -> (RequestId, RestCommand) {
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
        let (tx, rx) = conduit::local(8);
        let vault = Vault::new(tx);
        let got = Rc::new(RefCell::new(None));
        let doc = DocumentId::new();
        vault.seal(RowPlace::Document(doc), b"plain".to_vec(), {
            let got = Rc::clone(&got);
            move |sealed| *got.borrow_mut() = Some(sealed)
        });
        let (id, cmd) = request(&rx);
        let RestCommand::SealLocalRow { place, plaintext } = cmd else {
            panic!("expected a seal, got {cmd:?}");
        };
        assert_eq!(place, RowPlace::Document(doc));
        assert_eq!(&plaintext[..], b"plain");

        assert!(!vault.answer(RequestId(u64::MAX), &RestEvent::LocalStoreLocked));
        let sealed = RestEvent::LocalRowSealed {
            sealed: b"sealed".to_vec(),
        };
        assert!(vault.answer(id, &sealed));
        assert_eq!(*got.borrow(), Some(Some(b"sealed".to_vec())));
        assert!(!vault.answer(id, &sealed), "an answer is taken once");
    }

    /// Without a session the store is locked: the seal answers nothing and
    /// the rows read back open to nothing.
    #[wasm_bindgen_test]
    fn a_locked_store_answers_nothing() {
        let (tx, rx) = conduit::local(8);
        let vault = Vault::new(tx);
        let sealed = Rc::new(Cell::new(false));
        let opened = Rc::new(Cell::new(false));
        vault.seal(RowPlace::Draft("k".into()), b"x".to_vec(), {
            let sealed = Rc::clone(&sealed);
            move |row| sealed.set(row.is_none())
        });
        let row = SealedRow {
            place: RowPlace::Draft("k".into()),
            sealed: vec![1],
        };
        vault.open(vec![row], {
            let opened = Rc::clone(&opened);
            move |rows| opened.set(rows.is_none())
        });
        for _ in 0..2 {
            let (id, _) = request(&rx);
            assert!(vault.answer(id, &RestEvent::LocalStoreLocked));
        }
        assert!(sealed.get() && opened.get());
    }

    /// A request the channel to Logic does not take waits, and leaves in
    /// order once the channel has room.
    #[wasm_bindgen_test]
    fn a_full_channel_holds_the_requests_in_order() {
        let (tx, rx) = conduit::local(1);
        let vault = Vault::new(tx);
        for key in ["a", "b"] {
            vault.seal(RowPlace::Draft(key.into()), Vec::new(), |_| {});
        }
        let place = |rx: &Receiver<LogicCommand>| match request(rx).1 {
            RestCommand::SealLocalRow { place, .. } => place,
            other => panic!("expected a seal, got {other:?}"),
        };
        assert_eq!(place(&rx), RowPlace::Draft("a".into()));
        assert!(rx.try_recv().is_err(), "the second waits for room");
        vault.flush();
        assert_eq!(place(&rx), RowPlace::Draft("b".into()));
    }

    /// No rows to open answers at once, with no request.
    #[wasm_bindgen_test]
    fn no_rows_open_at_once() {
        let (tx, rx) = conduit::local(8);
        let vault = Vault::new(tx);
        let opened = Rc::new(Cell::new(false));
        vault.open(Vec::new(), {
            let opened = Rc::clone(&opened);
            move |rows| opened.set(rows.is_some_and(|rows| rows.is_empty()))
        });
        assert!(opened.get());
        assert!(rx.try_recv().is_err());
    }
}
