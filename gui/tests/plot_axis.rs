//! Tests for absolute time axis formatting in the falcon MF4 viewer plot panel.

use falcon_mdf_gui::panels::plot::absolute_label;

#[test]
fn epoch_zero_formats_to_unix_epoch_start() {
    // 0 nanoseconds since Unix epoch corresponds to 1970-01-01 00:00:00.000 UTC.
    assert_eq!(
        absolute_label(0, 0.0),
        "1970-01-01 00:00:00.000",
        "zero epoch timestamp must format to Unix epoch origin"
    );
}

#[test]
fn known_epoch_timestamp_formats_correctly() {
    // 2018-11-22 14:26:35.000 UTC
    // 17,857 days since epoch * 86,400s + 14*3600s + 26*60s + 35s = 1,542,896,795 seconds.
    let timestamp_ns = 1_542_896_795_000_000_000_i64;
    assert_eq!(
        absolute_label(timestamp_ns, 0.0),
        "2018-11-22 14:26:35.000",
        "known epoch nanosecond timestamp must match expected UTC calendar date and time"
    );

    // 2023-11-14 22:13:20.000 UTC (1,700,000,000 seconds)
    let timestamp_ns_2 = 1_700_000_000_000_000_000_i64;
    assert_eq!(
        absolute_label(timestamp_ns_2, 0.0),
        "2023-11-14 22:13:20.000",
        "round billion epoch second timestamp must format accurately"
    );
}

#[test]
fn adding_offset_seconds_moves_time_correctly() {
    let timestamp_ns = 1_542_896_795_000_000_000_i64; // 2018-11-22 14:26:35.000

    // Adding whole seconds forward
    assert_eq!(
        absolute_label(timestamp_ns, 10.0),
        "2018-11-22 14:26:45.000",
        "positive offset seconds must advance timestamp"
    );

    // Subtracting seconds backward
    assert_eq!(
        absolute_label(timestamp_ns, -5.0),
        "2018-11-22 14:26:30.000",
        "negative offset seconds must rewind timestamp"
    );
}

#[test]
fn fractional_offset_shows_in_milliseconds() {
    let timestamp_ns = 1_542_896_795_000_000_000_i64; // 2018-11-22 14:26:35.000

    assert_eq!(
        absolute_label(timestamp_ns, 0.439),
        "2018-11-22 14:26:35.439",
        "fractional second offset must format to exact millisecond decimal digits"
    );

    assert_eq!(
        absolute_label(timestamp_ns, 12.007),
        "2018-11-22 14:26:47.007",
        "leading zeros in millisecond fraction must be preserved"
    );

    assert_eq!(
        absolute_label(timestamp_ns, 0.099),
        "2018-11-22 14:26:35.099",
        "sub-100ms offset must format with two-digit zero prefix"
    );
}

#[test]
fn minute_boundary_rollover() {
    // 2018-11-22 14:26:59.000 (1,542,896,819 seconds)
    let timestamp_ns = 1_542_896_819_000_000_000_i64;

    assert_eq!(
        absolute_label(timestamp_ns, 2.0),
        "2018-11-22 14:27:01.000",
        "advancing past 59 seconds must roll minute counter over by +1"
    );

    assert_eq!(
        absolute_label(timestamp_ns, 1.5),
        "2018-11-22 14:27:00.500",
        "fractional offset crossing 60s must roll minute and preserve subseconds"
    );
}

#[test]
fn hour_boundary_rollover() {
    // 2018-11-22 14:59:59.000 (1,542,898,799 seconds)
    let timestamp_ns = 1_542_898_799_000_000_000_i64;

    assert_eq!(
        absolute_label(timestamp_ns, 2.0),
        "2018-11-22 15:00:01.000",
        "advancing past 59:59 must roll hour counter over by +1 and reset minutes"
    );
}

#[test]
fn day_boundary_rollover() {
    // 2018-11-22 23:59:59.000 (1,542,931,199 seconds)
    let timestamp_ns = 1_542_931_199_000_000_000_i64;

    assert_eq!(
        absolute_label(timestamp_ns, 2.0),
        "2018-11-23 00:00:01.000",
        "advancing past midnight must advance calendar day by +1 and reset time to 00:00:01"
    );
}

#[test]
fn month_and_year_boundary_rollover() {
    // 2023-12-31 23:59:59.000 (1,704,067,199 seconds)
    let timestamp_ns = 1_704_067_199_000_000_000_i64;

    assert_eq!(
        absolute_label(timestamp_ns, 2.0),
        "2024-01-01 00:00:01.000",
        "advancing past New Year's Eve must roll year, month, and day to Jan 1 of next year"
    );
}

#[test]
fn leap_year_february_29_formats_correctly() {
    // 2024 is a quadrennial leap year. 2024-02-29 12:00:00.000 is 1,709,208,000 seconds.
    let leap_day_ns = 1_709_208_000_000_000_000_i64;
    assert_eq!(
        absolute_label(leap_day_ns, 0.0),
        "2024-02-29 12:00:00.000",
        "leap day 29 February must format without premature month rollover"
    );

    // Advancing past 23:59:59 on February 29 must roll over into March 1.
    let end_of_leap_day_ns = 1_709_251_199_000_000_000_i64; // 2024-02-29 23:59:59.000
    assert_eq!(
        absolute_label(end_of_leap_day_ns, 2.0),
        "2024-03-01 00:00:01.000",
        "midnight after February 29 in leap year must roll over into March 1"
    );

    // 2000 was a 400-year century leap year (2000-02-29 12:00:00.000 is 951,825,600 seconds).
    let y2k_leap_ns = 951_825_600_000_000_000_i64;
    assert_eq!(
        absolute_label(y2k_leap_ns, 0.0),
        "2000-02-29 12:00:00.000",
        "century divisible by 400 must be treated as a leap year"
    );
}
