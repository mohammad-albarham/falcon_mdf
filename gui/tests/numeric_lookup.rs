//! Tests for instantaneous value lookup in the falcon MF4 viewer numeric panel.

use falcon_mdf_gui::panels::numeric::value_at;

#[test]
fn exact_hit_on_sample_timestamp() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0, 40.0];

    assert_eq!(
        value_at(&times, &values, None, 0.0),
        Some((0.0, 10.0)),
        "exact match at start of series"
    );
    assert_eq!(
        value_at(&times, &values, None, 1.0),
        Some((1.0, 20.0)),
        "exact match at middle timestamp"
    );
    assert_eq!(
        value_at(&times, &values, None, 2.0),
        Some((2.0, 30.0)),
        "exact match at another timestamp"
    );
}

#[test]
fn time_between_samples_takes_earlier_one() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0, 40.0];

    assert_eq!(
        value_at(&times, &values, None, 0.5),
        Some((0.0, 10.0)),
        "time between 0.0 and 1.0 takes sample at 0.0"
    );
    assert_eq!(
        value_at(&times, &values, None, 1.999),
        Some((1.0, 20.0)),
        "time immediately before 2.0 takes sample at 1.0"
    );
    assert_eq!(
        value_at(&times, &values, None, 2.5),
        Some((2.0, 30.0)),
        "time between 2.0 and 3.0 takes sample at 2.0"
    );
}

#[test]
fn time_before_first_sample_gives_none() {
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0];

    assert_eq!(
        value_at(&times, &values, None, 0.0),
        None,
        "time before first timestamp gives None"
    );
    assert_eq!(
        value_at(&times, &values, None, 0.999),
        None,
        "time immediately before first timestamp gives None"
    );
    assert_eq!(
        value_at(&times, &values, None, -50.0),
        None,
        "negative time before first timestamp gives None"
    );
}

#[test]
fn time_after_last_sample_gives_last() {
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0];

    assert_eq!(
        value_at(&times, &values, None, 3.5),
        Some((3.0, 30.0)),
        "time after last sample holds the last sample value"
    );
    assert_eq!(
        value_at(&times, &values, None, 1000.0),
        Some((3.0, 30.0)),
        "far future time holds the last sample value"
    );
}

#[test]
fn time_exactly_on_last_sample() {
    let times = [1.0, 2.0, 5.0];
    let values = [10.0, 20.0, 50.0];

    assert_eq!(
        value_at(&times, &values, None, 5.0),
        Some((5.0, 50.0)),
        "exact hit on the last sample timestamp returns that sample"
    );
}

#[test]
fn invalid_sample_is_skipped_and_prior_used() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 999.0, 40.0];
    let valid = [true, true, false, true];

    // Looking at t=2.0 where sample is invalid should step back to t=1.0
    assert_eq!(
        value_at(&times, &values, Some(&valid), 2.0),
        Some((1.0, 20.0)),
        "query at invalid sample steps back to prior valid sample"
    );
    // Looking between t=2.0 and t=3.0 should still step back to t=1.0
    assert_eq!(
        value_at(&times, &values, Some(&valid), 2.5),
        Some((1.0, 20.0)),
        "query between invalid sample and next valid sample uses prior valid sample"
    );
    // Looking at t=3.0 where sample is valid should return t=3.0
    assert_eq!(
        value_at(&times, &values, Some(&valid), 3.0),
        Some((3.0, 40.0)),
        "query at subsequent valid sample returns it"
    );
}

#[test]
fn every_sample_invalid_gives_none() {
    let times = [0.0, 1.0, 2.0];
    let values = [10.0, 20.0, 30.0];
    let valid = [false, false, false];

    assert_eq!(
        value_at(&times, &values, Some(&valid), 2.0),
        None,
        "series with all invalid samples returns None"
    );
    assert_eq!(
        value_at(&times, &values, Some(&valid), 10.0),
        None,
        "query after all invalid samples returns None"
    );
}

#[test]
fn empty_series_gives_none() {
    assert_eq!(
        value_at(&[], &[], None, 0.0),
        None,
        "empty series at t=0 returns None"
    );
    assert_eq!(
        value_at(&[], &[], None, 10.0),
        None,
        "empty series at t=10 returns None"
    );
}

#[test]
fn single_sample_series() {
    let times = [2.5];
    let values = [42.0];

    assert_eq!(
        value_at(&times, &values, None, 1.0),
        None,
        "before single sample returns None"
    );
    assert_eq!(
        value_at(&times, &values, None, 2.5),
        Some((2.5, 42.0)),
        "exactly at single sample returns that sample"
    );
    assert_eq!(
        value_at(&times, &values, None, 5.0),
        Some((2.5, 42.0)),
        "after single sample returns that sample"
    );

    let invalid = [false];
    assert_eq!(
        value_at(&times, &values, Some(&invalid), 2.5),
        None,
        "single invalid sample returns None"
    );
}

#[test]
fn large_series_binary_search() {
    const N: usize = 10_000;
    let times: Vec<f64> = (0..N).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..N).map(|i| i as f64 * 2.0).collect();

    // Query 4567.8 -> should pick sample 4567 at t=4567.0
    let res = value_at(&times, &values, None, 4567.8);
    assert_eq!(
        res,
        Some((4567.0, 4567.0 * 2.0)),
        "binary search in 10,000 samples correctly locates earlier sample"
    );

    // Past the end of the series: the last sample is what was last measured,
    // and it stays the answer however far past it the query is. The bound is
    // read off the series rather than written as a literal, so a change to
    // how the series is built cannot quietly move this query inside it.
    let res_end = value_at(&times, &values, None, times[N - 1] + 100.0);
    assert_eq!(
        res_end,
        Some((times[N - 1], values[N - 1])),
        "a query past the end takes the last sample"
    );

    // An exact hit on a sample timestamp takes that sample, not the one
    // before it.
    let res_exact = value_at(&times, &values, None, times[1]);
    assert_eq!(
        res_exact,
        Some((times[1], values[1])),
        "an exact hit takes its own sample"
    );

    // Before the first sample there is nothing to report.
    assert_eq!(
        value_at(&times, &values, None, -1.0),
        None,
        "a query before the first sample has no value"
    );
}
