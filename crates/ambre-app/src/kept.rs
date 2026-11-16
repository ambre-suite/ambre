//! The inviters earlier opens proved, as the Main thread holds them
//! (`ambre-spec-membership.md` section 3).
//!
//! The local store keeps one pin per document (`SYNC-238`). The Main thread
//! reads them once per page, and routes no open before they arrive
//! (`SYNC-240`). The open of a kept document then checks its log against
//! the kept pin. A kept pin wins over the pin of a link, and is never
//! replaced (`SYNC-239`).
//!
//! A Murmure page names no document, so the Main thread cannot tell which
//! opens its session asks for. Its boot waits for the store too, and every
//! kept pin goes ahead of it (`SYNC-243`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use ambre_core::id::{Document, Id};
use ambre_core::{FixedBytes, InviterPin};

/// The kept pins, shared by the store's link and the route binding.
#[derive(Clone, Default)]
pub(crate) struct KeptInviters(Rc<RefCell<Kept>>);

/// Whether the store answered yet.
#[derive(Default)]
enum Kept {
    /// The store has not answered: no open may go.
    #[default]
    Reading,
    /// The store answered, with no pin at all included.
    Read(HashMap<Id<Document>, FixedBytes<InviterPin>>),
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
    /// The store answered with `pins`, empty when it never opened.
    pub(crate) fn read(&self, pins: Vec<(Id<Document>, FixedBytes<InviterPin>)>) {
        let mut kept = self.0.borrow_mut();
        match &mut *kept {
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
        match &mut *self.0.borrow_mut() {
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
        match &*self.0.borrow() {
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
        match &*self.0.borrow() {
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

    /// Holds `SYNC-240` on the Main thread: no open goes before the store
    /// answers, and then the kept pin goes ahead of it.
    #[test]
    fn an_open_waits_for_the_kept_pins() {
        let kept = KeptInviters::default();
        let doc = Id::<Document>::mint();
        assert_eq!(kept.pin(doc, Some(pin(1))), OpenPin::Waiting);
        kept.read(vec![(doc, pin(2))]);
        assert_eq!(kept.pin(doc, None), OpenPin::Ready(Some(pin(2))));
        // A store that never opened answers with none: the link alone goes.
        let empty = KeptInviters::default();
        empty.read(Vec::new());
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
        kept.read(vec![(space, pin(2))]);
        assert_eq!(kept.keep(channel, pin(3)), Proof::Fresh);
        let KeptPins::Ready(mut every) = kept.every() else {
            panic!("the store answered");
        };
        every.sort_by_key(|(id, _)| *id.as_bytes());
        let mut expected = vec![(space, pin(2)), (channel, pin(3))];
        expected.sort_by_key(|(id, _)| *id.as_bytes());
        assert_eq!(every, expected);
        let empty = KeptInviters::default();
        empty.read(Vec::new());
        assert_eq!(empty.every(), KeptPins::Ready(Vec::new()));
    }

    /// Holds `SYNC-239` on the Main thread: a kept pin wins over the link's,
    /// and a later proof replaces nothing.
    #[test]
    fn a_kept_pin_wins_over_a_link_and_is_never_replaced() {
        let kept = KeptInviters::default();
        let (doc, fresh) = (Id::<Document>::mint(), Id::<Document>::mint());
        kept.read(vec![(doc, pin(2))]);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(2))));
        assert_eq!(kept.keep(doc, pin(3)), Proof::Held);
        assert_eq!(kept.pin(doc, Some(pin(3))), OpenPin::Ready(Some(pin(2))));
        assert_eq!(kept.keep(fresh, pin(3)), Proof::Fresh);
        assert_eq!(kept.keep(fresh, pin(3)), Proof::Held);
        assert_eq!(kept.pin(fresh, None), OpenPin::Ready(Some(pin(3))));
    }
}
