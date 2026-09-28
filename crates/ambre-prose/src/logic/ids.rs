//! Where the engine takes the ids it mints for a note, a comment thread, a
//! reply and a suggestion.
//!
//! A live document mints a fresh v7 id each time. A document built with a
//! pinned peer mints a sequence instead, so a generator that inserts a note
//! writes the same bytes twice (`FIXTURE-002`). One constructor pins the peer
//! and the ids together.

use std::cell::Cell;

use ambre_core::Uuid;

/// The id minter a [`ProseState`](super::ProseState) owns.
#[derive(Debug)]
pub(super) enum IdSource {
    /// A fresh v7 id per call: the live path.
    Random,
    /// The pinned peer and the next counter, packed into a version 8 id.
    /// Interior mutability, because the writers that mint read the state
    /// through `&self`.
    Seeded { peer: u64, next: Cell<u64> },
}

impl IdSource {
    /// The sequence for a pinned peer, starting at zero.
    pub(super) fn seeded(peer: u64) -> Self {
        Self::Seeded {
            peer,
            next: Cell::new(0),
        }
    }

    /// The next id.
    pub(super) fn mint(&self) -> Uuid {
        match self {
            Self::Random => Uuid::new(),
            Self::Seeded { peer, next } => {
                let counter = next.get();
                next.set(counter.wrapping_add(1));
                Uuid::from_bytes(pack(*peer, counter).to_be_bytes())
            }
        }
    }
}

/// `peer` in the high half and `counter` in the low half, with the version
/// nibble set to 8 and the variant bits to RFC 9562. The six bits those take
/// cost nothing here: a pinned peer builds one document, alone.
fn pack(peer: u64, counter: u64) -> u128 {
    const VERSION: u128 = 0xF << 76;
    const VARIANT: u128 = 0b11 << 62;
    let raw = (u128::from(peer) << 64) | u128::from(counter);
    (raw & !VERSION & !VARIANT) | (0x8 << 76) | (0b10 << 62)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sources pinned to one peer mint one sequence, and no id repeats
    /// inside it.
    #[test]
    fn a_seeded_source_repeats_its_sequence_and_never_an_id() {
        let first = IdSource::seeded(0x5EED);
        let second = IdSource::seeded(0x5EED);
        let a: Vec<Uuid> = (0..4).map(|_| first.mint()).collect();
        let b: Vec<Uuid> = (0..4).map(|_| second.mint()).collect();
        assert_eq!(a, b);
        for (i, id) in a.iter().enumerate() {
            assert!(!a.iter().skip(i + 1).any(|other| other == id));
        }
    }

    /// The packed id reads as a version 8 id of the RFC variant.
    #[test]
    fn a_seeded_id_carries_version_8() {
        let bytes = pack(u64::MAX, u64::MAX).to_be_bytes();
        assert_eq!(bytes[6] >> 4, 0x8);
        assert_eq!(bytes[8] >> 6, 0b10);
    }
}
