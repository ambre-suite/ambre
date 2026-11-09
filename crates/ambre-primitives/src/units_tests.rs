use super::unit_of;

#[test]
fn a_name_that_ends_on_a_unit_is_found_in_every_case() {
    for item in [
        "Retry.delay_ms",
        "DELAY_MS",
        "Retry.delayMs",
        "Retry::delay_ms()",
        "Retry::wait(delay_ms)",
        "Retry::wait(_delay_ms)",
        "Pacer.target_bps",
        "TIMEOUT_10MS",
        "<Pacer as Trait>::rateHz()",
        "slice::REPLAY_SLICE_BYTES",
    ] {
        assert!(unit_of(item).is_some(), "missed {item}");
    }
}

#[test]
fn a_unit_before_the_last_segment_passes() {
    for item in [
        "Retry.gain",
        "BITS_PER_PIXEL",
        "PX_PER_PT",
        "<Ms as ClockUnit>::PER_SECOND",
        "Rgba::bits_set()",
        "Retry::delay_ms(factor)",
        "Criterion::Num.1",
        "msg",
        "Status",
    ] {
        assert_eq!(unit_of(item), None, "refused {item}");
    }
}
