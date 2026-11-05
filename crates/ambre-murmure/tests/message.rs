// ambre-murmure message tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native coverage of the value types Murmure's message vocabulary
// carries. They moved here with the vocabulary (`BUS-032`).
// ============================================================================

/// A catalogue rank keeps the integer a Loro map stored before the rank
/// family, and the `Debug` text the hand-written type printed.
#[test]
fn a_catalogue_rank_keeps_its_stored_integer_and_its_debug_text() {
    use ambre_bus::Rank;
    use ambre_murmure::message::CatalogPosition;
    let rank = Rank::<CatalogPosition>::new(7);
    assert_eq!(rank.to_stored(), 7_i64);
    assert_eq!(Rank::<CatalogPosition>::from_stored(7), rank);
    assert_eq!(Rank::<CatalogPosition>::FIRST.to_stored(), 0_i64);
    assert_eq!(format!("{rank:?}"), "CatalogPosition(7)");
    assert_eq!(rank.to_string(), "7");
}
