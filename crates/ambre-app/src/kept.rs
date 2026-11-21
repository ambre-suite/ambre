//! The inviters earlier opens proved, as the Main thread holds them
//! (`ambre-spec-membership.md` section 3).
//!
//! The local store keeps one pin per user and document (`SYNC-238`,
//! `SYNC-244`). The Main thread reads the signed-in user's pins once per
//! page, and routes no open before they arrive (`SYNC-240`). It reads no
//! pin before a session names the user, so one account's pins never reach
//! another account's opens. The open of a kept document then checks its log against
//! the kept pin. A kept pin wins over the pin of a link, and is never
//! replaced (`SYNC-239`).
//!
//! A Murmure page names no document, so the Main thread cannot tell which
//! opens its session asks for. Its boot waits for the store too, and every
//! kept pin goes ahead of it (`SYNC-243`).
//!
//! The store keeps, per user and log, where the log stood when the Sync
//! worker last accepted a key of it (`SYNC-290`). The Main thread reads them
//! with the pins and sends each one ahead of every open of the session. A
//! sign-in of the same user on the page sends them again from memory.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use ambre_core::id::{Document, Id, User};
use ambre_core::{FixedBytes, InviterPin};

/// The kept pins, shared by the store's link and the route binding.
#[derive(Clone, Default)]
pub(crate) struct KeptInviters(Rc<RefCell<Held>>);

/// The signed-in user, and the pins the store answered for that user.
#[derive(Default)]
struct Held {
    /// The user the session names, `None` before a session.
    user: Option<Id<User>>,
    kept: Kept,
    /// The logs the store answered for that user, and each one the Sync
    /// worker moved since, by the log's document (`SYNC-290`).
    logs: HashMap<Id<Document>, ambre_bus::KeptLog>,
}

/// Whether the store answered yet, for the signed-in user.
#[derive(Default)]
enum Kept {
    /// No session names a user, or the store has not answered for that
    /// user: no open may go.
    #[default]
    Reading,
    /// The store answered, with no pin at all included.
    Read(HashMap<Id<Document>, FixedBytes<InviterPin>>),
}

/// What a session's user asks of the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Load {
    /// The user is new to this page: the store reads that user's pins.
    Needed,
    /// The page names that user already: its pins are read or on the way.
    Asked,
}

/// The pin an open sends ahead of it, once the store answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenPin {
    /// The store has not answered: the open waits.
    Waiting,
    /// The pin to send, or none.
    Ready(Option<FixedBytes<InviterPin>>),
}

/// What an open's proof adds to the kept pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proof {
    /// The pin may be new to the store, which takes the write.
    Fresh,
    /// The document keeps a pin already: the store has its row.
    Held,
}

/// Every kept pin, once the store answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeptPins {
    /// The store has not answered: the Murmure boot waits.
    Waiting,
    /// Each kept pin, with its document.
    Ready(Vec<(Id<Document>, FixedBytes<InviterPin>)>),
}

impl KeptInviters {
    /// A session names `user`. A new user drops every pin held so far, so
    /// no pin of another account reaches its opens (`SYNC-244`).
    pub(crate) fn signed_in(&self, user: Id<User>) -> Load {
        let mut held = self.0.borrow_mut();
        if held.user == Some(user) {
            return Load::Asked;
        }
        *held = Held {
            user: Some(user),
            kept: Kept::Reading,
            logs: HashMap::new(),
        };
        Load::Needed
    }

    /// The store answered with `logs` for `user`. A log the worker moved on
    /// this page already stays. An answer for another user is dropped.
    pub(crate) fn read_logs(&self, user: Id<User>, logs: Vec<(Id<Document>, ambre_bus::KeptLog)>) {
        let mut held = self.0.borrow_mut();
        if held.user != Some(user) {
            return;
        }
        for (document_id, kept) in logs {
            held.logs.entry(document_id).or_insert(kept);
        }
    }

    /// The Sync worker moved the log of `document_id` to `kept`, which the
    /// store writes too (`SYNC-290`).
    pub(crate) fn keep_log(&self, document_id: Id<Document>, kept: ambre_bus::KeptLog) {
        self.0.borrow_mut().logs.insert(document_id, kept);
    }

    /// Every kept log of the session's user, each sent to the Sync worker
    /// ahead of the opens of a session (`SYNC-290`).
    pub(crate) fn logs(&self) -> Vec<(Id<Document>, ambre_bus::KeptLog)> {
        self.0
            .borrow()
            .logs
            .iter()
            .map(|(document_id, kept)| (*document_id, kept.clone()))
            .collect()
    }

    /// The user the session names, `None` before a session.
    pub(crate) fn user(&self) -> Option<Id<User>> {
        self.0.borrow().user
    }

    /// The store answered with `pins` for `user`, empty when it never
    /// opened. An answer for another user than the session's is dropped.
    pub(crate) fn read(&self, user: Id<User>, pins: Vec<(Id<Document>, FixedBytes<InviterPin>)>) {
        let mut held = self.0.borrow_mut();
        if held.user != Some(user) {
            return;
        }
        let kept = &mut held.kept;
        match kept {
            Kept::Reading => *kept = Kept::Read(pins.into_iter().collect()),
            Kept::Read(held) => {
                for (document_id, pin) in pins {
                    held.entry(document_id).or_insert(pin);
                }
            }
        }
    }

    /// An open proved `pin` for `document_id`. It becomes the kept pin,
    /// unless the document keeps one already (`SYNC-239`). Each Murmure open
    /// proves its pin again, so a held one asks no write of the store.
    pub(crate) fn keep(&self, document_id: Id<Document>, pin: FixedBytes<InviterPin>) -> Proof {
        match &mut self.0.borrow_mut().kept {
            Kept::Reading => Proof::Fresh,
            Kept::Read(held) => match held.entry(document_id) {
                Entry::Occupied(_) => Proof::Held,
                Entry::Vacant(slot) => {
                    slot.insert(pin);
                    Proof::Fresh
                }
            },
        }
    }

    /// Every kept pin, which the Murmure boot sends ahead of it
    /// (`SYNC-243`).
    pub(crate) fn every(&self) -> KeptPins {
        match &self.0.borrow().kept {
            Kept::Reading => KeptPins::Waiting,
            Kept::Read(held) => KeptPins::Ready(held.iter().map(|(d, p)| (*d, *p)).collect()),
        }
    }

    /// The pin the open of `document_id` sends: the kept one, else the pin
    /// `linked` of its link.
    pub(crate) fn pin(
        &self,
        document_id: Id<Document>,
        linked: Option<FixedBytes<InviterPin>>,
    ) -> OpenPin {
        match &self.0.borrow().kept {
            Kept::Reading => OpenPin::Waiting,
            Kept::Read(held) => OpenPin::Ready(held.get(&document_id).copied().or(linked)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(byte: u8) -> FixedBytes<InviterPin> {
        FixedBytes::from_array([byte; 32])
    }

    fn user(byte: u8) -> Id<User> {
        Id::<User>::from_bytes([byte; 16])
    }

    /// The pins the store kept for `me`, once a session names that user.
    fn signed_in(me: u8, pins: Vec<(Id<Document>, FixedBytes<InviterPin>)>) -> KeptInviters {
        let kept = KeptInviters::default();
        assert_eq!(kept.signed_in(user(me)), Load::Needed);
        kept.read(user(me), pins);
        kept
    }

    /// Holds `SYNC-240` on the Main thread: no open goes before the store
    /// answers, and then the kept pin goes ahead of it.
    #[test]
    fn an_open_waits_for_the_kept_pins() {
        let kept = KeptInviters::default();
        let doc = Id::<Document>::mint();
        assert_eq!(kept.pin(doc, Some(pin(1))), OpenPin::Waiting);
        kept.signed_in(user(1));
        assert_eq!(kept.pin(doc, Some(pin(1))), OpenPin::Waiting);
        kept.read(user(1), vec![(doc, pin(2))]);
        assert_eq!(kept.pin(doc, None), OpenPin::Ready(Some(pin(2))));
        // A store that never opened answers with none: the link alone goes.
        let empty = signed_in(1, Vec::new());
        assert_eq!(empty.pin(doc, Some(pin(1))), OpenPin::Ready(Some(pin(1))));
        assert_eq!(empty.pin(doc, None), OpenPin::Ready(None));
    }

    /// Holds `SYNC-243` on the Main thread: the Murmure boot waits for the
    /// store, then takes every kept pin, an open's proof since included.
    #[test]
    fn a_murmure_boot_waits_for_every_kept_pin() {
        let kept = KeptInviters::default();
        let (space, channel) = (Id::<Document>::mint(), Id::<Document>::mint());
        assert_eq!(kept.every(), KeptPins::Waiting);
        kept.signed_in(user(1));
        kept.read(user(1), vec![(space, pin(2))]);
        assert_eq!(kept.keep(channel, pin(3)), Proof::Fresh);
        let KeptPins::Ready(mut every) = kept.every() else {
            panic!("the store answered");
        };
        every.sort_by_key(|(id, _)| *id.as_bytes());
        let mut expected = vec![(space, pin(2)), (channel, pin(3))];
        expected.sort_by_key(|(id, _)| *id.as_bytes());
        assert_eq!(every, expected);
        let empty = signed_in(1, Vec::new());
        assert_eq!(empty.every(), KeptPins::Ready(Vec::new()));
    }

    /// Holds `SYNC-239` on the Main thread: a kept pin wins over the link's,
    /// and a later proof replaces nothing.
    #[test]
    fn a_kept_pin_wins_over_a_link_and_is_never_replaced() {
        let (doc, fresh) = (Id::<Document>::mint(), Id::<Document>::mint());
        let kept = signed_in(1, vec![(doc, pin(2))]);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(2))));
        assert_eq!(kept.keep(doc, pin(3)), Proof::Held);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(2))));
        assert_eq!(kept.keep(fresh, pin(3)), Proof::Fresh);
        assert_eq!(kept.keep(fresh, pin(3)), Proof::Held);
        assert_eq!(kept.pin(fresh, None), OpenPin::Ready(Some(pin(3))));
    }

    /// Holds `SYNC-244` on the Main thread: no pin goes before a session
    /// names the user. A second account takes none of the first account's
    /// pins, the ones this page proved included.
    #[test]
    fn a_second_account_reads_none_of_the_first_accounts_pins() {
        let kept = KeptInviters::default();
        let (doc, proved) = (Id::<Document>::mint(), Id::<Document>::mint());
        // An answer before any session goes nowhere.
        kept.read(user(1), vec![(doc, pin(2))]);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Waiting);
        assert_eq!(kept.signed_in(user(1)), Load::Needed);
        assert_eq!(kept.signed_in(user(1)), Load::Asked);
        kept.read(user(1), vec![(doc, pin(2))]);
        assert_eq!(kept.keep(proved, pin(4)), Proof::Fresh);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(2))));
        // A second account waits for its own answer, and a late answer for
        // the first one is dropped.
        assert_eq!(kept.signed_in(user(5)), Load::Needed);
        assert_eq!(kept.user(), Some(user(5)));
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Waiting);
        assert_eq!(kept.every(), KeptPins::Waiting);
        kept.read(user(1), vec![(doc, pin(2))]);
        assert_eq!(kept.every(), KeptPins::Waiting);
        kept.read(user(5), Vec::new());
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(3))));
        assert_eq!(kept.pin(proved, None), OpenPin::Ready(None));
        assert_eq!(kept.every(), KeptPins::Ready(Vec::new()));
    }

    /// A kept log whose key names `byte`.
    fn log(byte: u8) -> ambre_bus::KeptLog {
        ambre_bus::KeptLog {
            anchor: FixedBytes::from_array([1; 32]),
            head: ambre_bus::LogHead {
                position: ambre_core::Seq::new(2),
                previous: Some(FixedBytes::from_array([2; 32])),
            },
            key: FixedBytes::from_array([byte; 16]),
            links: Vec::new(),
        }
    }

    /// Holds `SYNC-290` on the Main thread. The kept logs of the session's
    /// user are held for its opens. A log the worker moved on this page wins
    /// over the store's older answer. A second account takes none of the
    /// first account's logs.
    #[test]
    fn the_kept_logs_of_a_user_go_ahead_of_its_opens_alone() {
        let kept = KeptInviters::default();
        let (doc, moved) = (Id::<Document>::mint(), Id::<Document>::mint());
        kept.read_logs(user(1), vec![(doc, log(3))]);
        assert!(kept.logs().is_empty(), "no session names the user yet");
        kept.signed_in(user(1));
        kept.keep_log(moved, log(5));
        kept.read_logs(user(1), vec![(doc, log(3)), (moved, log(4))]);
        let mut logs = kept.logs();
        logs.sort_by_key(|(id, _)| *id.as_bytes());
        let mut expected = vec![(doc, log(3)), (moved, log(5))];
        expected.sort_by_key(|(id, _)| *id.as_bytes());
        assert_eq!(logs, expected);
        assert_eq!(kept.signed_in(user(5)), Load::Needed);
        assert!(kept.logs().is_empty(), "a second account holds none");
    }
}
