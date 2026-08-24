//! What a number looks like in a readout, pinned at the boundaries.
//!
//! Every one of these is a place where a plausible implementation gets it
//! subtly wrong: the decade where the prefix changes, the value that is
//! exactly a thousand, the duration that is exactly a minute, and the
//! not-a-numbers that must not be dressed up as measurements.

use falcon_mdf_gui::format::{duration, engineering};

#[test]
fn values_take_the_prefix_of_their_decade() {
    assert_eq!(engineering(0.000_42, "V"), "420 uV");
    assert_eq!(engineering(15_000.0, "Hz"), "15.0 kHz");
    assert_eq!(engineering(1.5, "V"), "1.50 V");
    assert_eq!(engineering(0.0015, "V"), "1.50 mV");
}

#[test]
fn three_significant_digits_are_kept_across_the_decade() {
    // The decimals move as the integer part grows, so the number stays the
    // same width and the column does not jump.
    assert_eq!(engineering(1_230.0, "Hz"), "1.23 kHz");
    assert_eq!(engineering(12_300.0, "Hz"), "12.3 kHz");
    assert_eq!(engineering(123_000.0, "Hz"), "123 kHz");
}

#[test]
fn exactly_one_thousand_steps_up_a_prefix() {
    // The boundary itself: 1000 is one kilo, not a thousand units.
    assert_eq!(engineering(1_000.0, "Hz"), "1.00 kHz");
    assert_eq!(engineering(999.0, "Hz"), "999 Hz");
}

#[test]
fn zero_carries_no_prefix() {
    assert_eq!(engineering(0.0, "V"), "0 V");
    assert_eq!(engineering(0.0, ""), "0");
}

#[test]
fn a_negative_value_keeps_its_sign() {
    assert_eq!(engineering(-0.0015, "V"), "-1.50 mV");
    assert_eq!(engineering(-15_000.0, "Hz"), "-15.0 kHz");
}

#[test]
fn a_channel_with_no_unit_gets_no_trailing_space() {
    assert_eq!(engineering(1.5, ""), "1.50");
    assert_eq!(engineering(15_000.0, ""), "15.0 k");
}

#[test]
fn values_past_the_prefixes_fall_back_to_scientific_notation() {
    assert!(
        engineering(1e15, "V").contains('e'),
        "a value past tera should not be scaled by a prefix nobody reads"
    );
    assert!(engineering(1e-15, "V").contains('e'));
}

#[test]
fn not_a_number_is_named_rather_than_dressed_as_a_measurement() {
    assert_eq!(engineering(f64::NAN, "V"), "NaN");
    assert_eq!(engineering(f64::INFINITY, "V"), "inf");
    assert_eq!(engineering(f64::NEG_INFINITY, "V"), "-inf");
}

#[test]
fn short_durations_keep_their_milliseconds() {
    assert_eq!(duration(1.5), "1.500 s");
    assert_eq!(duration(0.001), "0.001 s");
    assert_eq!(duration(59.999), "59.999 s");
}

#[test]
fn exactly_a_minute_becomes_minutes() {
    // The boundary: 60 s is one minute and no seconds, not "60.000 s".
    assert_eq!(duration(60.0), "1 min 00 s");
    assert_eq!(duration(123.0), "2 min 03 s");
}

#[test]
fn exactly_an_hour_becomes_hours() {
    assert_eq!(duration(3_600.0), "1 h 00 min");
    assert_eq!(duration(3_900.0), "1 h 05 min");
}

#[test]
fn seconds_are_zero_padded_so_the_column_does_not_jump() {
    assert_eq!(duration(61.0), "1 min 01 s");
    assert_eq!(duration(3_660.0), "1 h 01 min");
}

#[test]
fn a_negative_duration_keeps_its_sign_on_the_leading_number() {
    assert_eq!(duration(-1.5), "-1.500 s");
    assert_eq!(duration(-90.0), "-1 min 30 s");
}

#[test]
fn a_duration_that_is_not_a_number_is_named() {
    assert_eq!(duration(f64::NAN), "NaN");
    assert_eq!(duration(f64::INFINITY), "inf");
}
