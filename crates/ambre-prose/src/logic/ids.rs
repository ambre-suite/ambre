//! Where the engine takes the ids it mints for a note, a comment thread, a
//! reply and a suggestion.
//!
//! A live document mints a fresh v7 id each time. A document built with a
//! pinned peer mints a sequence instead, so a generator that inserts a note
//! writes the same bytes twice (`FIXTURE-002`). One constructor pins the peer
//! and the ids together.

use std::cell::Cell;

use ambre_core::Uuid;

/// The place of an id in a pinned peer's sequence, the first being zero.
///
/// It holds the 62 bits of the low half that the variant bits leave. So no
/// two places pack to one id, and the sequence ends rather than wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct IdSequence(u64);

impl IdSequence {
    /// The last place the 62 bits hold.
    const LAST: Self = Self((1 << 62) - 1);

    /// The place after this one, `None` past [`Self::LAST`].
    fn next(self) -> Option<Self> {
        self.0
            .checked_add(1)
            .filter(|place| *place <= Self::LAST.0)
            .map(Self)
    }
}

/// The id minter a [`ProseState`](super::ProseState) owns.
#[derive(Debug)]
pub(super) enum IdSource {
    /// A fresh v7 id per call: the live path.
    Random,
    /// The pinned peer and the next place of its sequence, packed into a
    /// version 8 id. `None` once the last place is minted. Interior mutability,
    /// because the writers that mint read the state through `&self`.
    Seeded {
        peer: ambre_core::PeerId,
        next: Cell<Option<IdSequence>>,
    },
}

impl IdSource {
    /// The sequence for a pinned peer, starting at zero.
    pub(super) fn seeded(peer: ambre_core::PeerId) -> Self {
        Self::Seeded {
            peer,
            next: Cell::new(Some(IdSequence::default())),
        }
    }

    /// The next id, of whichever annotation kind the caller names. Every kind
    /// draws from the one sequence, so no two annotations share an id.
    ///
    /// # Panics
    ///
    /// A pinned source panics on the mint after its 2^62 places. Past them,
    /// an id either repeats or comes from a random source, and `FIXTURE-035`
    /// refuses both. Only a host-side generator pins a source, so the panic
    /// never reaches a live document.
    #[expect(
        clippy::expect_used,
        reason = "FIXTURE-035: a pinned sequence never repeats an id nor falls back to a random one"
    )]
    pub(super) fn mint<T: From<Uuid>>(&self) -> T {
        let uuid = match self {
            Self::Random => Uuid::new(),
            Self::Seeded { peer, next } => {
                let place = next
                    .get()
                    .expect("a pinned peer mints at most 2^62 ids, and none twice (FIXTURE-035)");
                next.set(place.next());
                pack(*peer, place)
            }
        };
        T::from(uuid)
    }
}

/// `peer` in the high half and `place` in the low half, with the version
/// nibble set to 8 and the variant bits to RFC 9562. The four peer bits the
/// version takes cost nothing here: a pinned peer builds one document, alone.
fn pack(peer: ambre_core::PeerId, place: IdSequence) -> Uuid {
    const VERSION: u128 = 0xF << 76;
    const VARIANT: u128 = 0b11 << 62;
    let raw = (u128::from(peer.get()) << 64) | u128::from(place.0);
    let bits = (raw & !VERSION & !VARIANT) | (0x8 << 76) | (0b10 << 62);
    Uuid::from_bytes(bits.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sources pinned to one peer mint one sequence, and no id repeats
    /// inside it.
    #[test]
    fn a_seeded_source_repeats_its_sequence_and_never_an_id() {
        let first = IdSource::seeded(ambre_core::PeerId::new(0x5EED));
        let second = IdSource::seeded(ambre_core::PeerId::new(0x5EED));
        let a: Vec<Uuid> = (0..4).map(|_| first.mint()).collect();
        let b: Vec<Uuid> = (0..4).map(|_| second.mint()).collect();
        assert_eq!(a, b);
        for (i, id) in a.iter().enumerate() {
            assert!(!a.iter().skip(i + 1).any(|other| other == id));
        }
    }

    /// The last place of a sequence mints an id of its own. A wrapping `u64`
    /// counter came back to zero there. Its two top bits also fell under the
    /// variant mask, so place 2^62 packed to the id of place zero.
    #[test]
    fn a_seeded_source_never_reissues_an_id_at_the_end_of_its_sequence() {
        let peer = ambre_core::PeerId::new(0x5EED);
        let start: Uuid = IdSource::seeded(peer).mint();
        let end = IdSource::Seeded {
            peer,
            next: Cell::new(Some(IdSequence::LAST)),
        };
        let last: Uuid = end.mint();
        assert_ne!(last, start);
        assert_eq!(IdSequence::LAST.next(), None);
    }

    /// Past its last place, a pinned source panics. It neither wraps to an id
    /// it minted, nor saturates on the last one, nor mints a random one.
    #[test]
    #[should_panic(expected = "a pinned peer mints at most 2^62 ids")]
    fn a_seeded_source_panics_past_the_end_of_its_sequence() {
        let end = IdSource::Seeded {
            peer: ambre_core::PeerId::new(0x5EED),
            next: Cell::new(Some(IdSequence::LAST)),
        };
        let _last: Uuid = end.mint();
        let _past: Uuid = end.mint();
    }

    /// The packed id reads as a version 8 id of the RFC variant.
    #[test]
    fn a_seeded_id_carries_version_8() {
        let bytes = pack(ambre_core::PeerId::new(u64::MAX), IdSequence::LAST);
        let bytes = bytes.as_bytes();
        assert_eq!(bytes[6] >> 4, 0x8);
        assert_eq!(bytes[8] >> 6, 0b10);
    }
}
