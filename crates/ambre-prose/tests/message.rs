// ambre-prose message tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native coverage of the bounded values Prose's message vocabulary
// carries: the line-height multiple. They moved here with the vocabulary
// (`BUS-032`).
// ============================================================================

use ambre_prose::message::{LineHeight, LineHeightMode, LineSpacingMultiple};

/// A line-height multiple is finite and above zero, and a stored number that
/// is not reads as no setting at all.
#[test]
fn a_line_height_multiple_is_finite_and_above_zero() {
    for refused in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(LineSpacingMultiple::new(refused), None);
        assert_eq!(
            LineHeight::from_stored(LineHeightMode::Multiple, refused),
            None
        );
    }
    for mode in [LineHeightMode::Exact, LineHeightMode::AtLeast] {
        for refused in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(LineHeight::from_stored(mode, refused), None, "{refused}");
        }
        assert!(LineHeight::from_stored(mode, 12.0).is_some());
    }
    let read = LineHeight::from_stored(LineHeightMode::Multiple, 1.5);
    assert_eq!(
        read,
        Some(LineHeight::Multiple(LineSpacingMultiple::ONE_AND_HALF))
    );
    assert_eq!(read.map(LineHeight::stored_value), Some(1.5));
}
