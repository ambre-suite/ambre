// ambre-toile message tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native coverage of the bounded values Toile's message vocabulary
// carries. They moved here with the vocabulary (`BUS-032`).
// ============================================================================

use ambre_core::{Board, Bounded, Px};
use ambre_toile::message::UnitFraction;

/// A fraction of a side lies in 0..=1, and a decoded one outside it is
/// refused.
#[test]
fn a_fraction_of_a_side_lies_between_zero_and_one() {
    for refused in [-0.1, 1.1, f32::NAN] {
        assert_eq!(Bounded::<UnitFraction>::new(refused), None);
    }
    assert_eq!(
        Bounded::<UnitFraction>::HALF.along(Px::<Board>::new(10.0), Px::<Board>::new(30.0)),
        Px::<Board>::new(20.0)
    );
    let past: Result<Bounded<UnitFraction>, _> =
        postcard::from_bytes(&postcard::to_allocvec(&2.0_f32).unwrap());
    assert!(past.is_err(), "a decoded fraction past one is refused");
    let bytes = postcard::to_allocvec(&Bounded::<UnitFraction>::HALF).unwrap();
    assert_eq!(
        bytes,
        postcard::to_allocvec(&0.5_f32).unwrap(),
        "it travels as an f32"
    );
}
