//! Tests for measurement window region statistics in the falcon MF4 viewer plot panel.

use falcon_mdf_gui::panels::plot::{region_stats, RegionStats};

#[test]
fn whole_series_region_computes_exact_min_max_mean() {
    let times = [0.0, 1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 20.0, 50.0, 5.0, 40.0];

    let stats = region_stats(&times, &values, None, 0.0, 4.0)
        .expect("whole series region must produce statistics");

    assert_eq!(
        stats,
        RegionStats {
            count: 5,
            excluded: 0,
            min: 5.0,
            max: 50.0,
            mean: 25.0,
        },
        "statistics must match expected RegionStats struct"
    );
    assert_eq!(stats.count, 5, "all 5 samples should be included");
    assert_eq!(stats.excluded, 0, "no samples should be excluded");
    assert_eq!(stats.min, 5.0, "minimum value must match minimum in series");
    assert_eq!(
        stats.max, 50.0,
        "maximum value must match maximum in series"
    );
    assert_eq!(
        stats.mean, 25.0,
        "mean value must match arithmetic mean (125/5)"
    );
}

#[test]
fn empty_region_outside_time_range_returns_none() {
    let times = [1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0];

    // Query entirely before the data range
    assert_eq!(
        region_stats(&times, &values, None, -5.0, 0.5),
        None,
        "region before the series start must return None"
    );

    // Query entirely after the data range
    assert_eq!(
        region_stats(&times, &values, None, 4.0, 10.0),
        None,
        "region after the series end must return None"
    );

    // Query in an empty gap with no sample timestamps
    let sparse_times = [1.0, 5.0];
    let sparse_values = [10.0, 50.0];
    assert_eq!(
        region_stats(&sparse_times, &sparse_values, None, 2.0, 4.0),
        None,
        "region in a timestamp gap must return None"
    );
}

#[test]
fn region_bounds_are_inclusive() {
    let times = [0.0, 1.0, 2.0, 3.0, 4.0];
    let values = [100.0, 20.0, 30.0, 40.0, 500.0];

    // Selecting exactly [1.0, 3.0] should include 1.0, 2.0, and 3.0, excluding 0.0 and 4.0
    let stats = region_stats(&times, &values, None, 1.0, 3.0)
        .expect("inclusive region must include boundary samples");

    assert_eq!(stats.count, 3, "should include indices 1, 2, and 3");
    assert_eq!(stats.excluded, 0, "no samples should be excluded");
    assert_eq!(stats.min, 20.0, "min must be 20.0 at t=1.0");
    assert_eq!(stats.max, 40.0, "max must be 40.0 at t=3.0");
    assert_eq!(stats.mean, 30.0, "mean must be (20+30+40)/3 = 30.0");
}

#[test]
fn reversed_cursor_bounds_give_identical_statistics() {
    let times = [0.0, 1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 25.0, 15.0, 30.0, 5.0];

    let forward = region_stats(&times, &values, None, 1.0, 3.0);
    let reversed = region_stats(&times, &values, None, 3.0, 1.0);

    assert!(forward.is_some(), "forward bounds should yield statistics");
    assert_eq!(
        forward, reversed,
        "swapping cursor order (B before A) must produce identical RegionStats"
    );
}

#[test]
fn invalid_samples_are_excluded_from_stats_and_counted() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 9999.0, 30.0, 40.0];
    let valid = [true, false, true, true];

    let stats = region_stats(&times, &values, Some(&valid), 0.0, 3.0)
        .expect("partially valid region should produce statistics");

    assert_eq!(stats.count, 3, "should count the 3 valid samples");
    assert_eq!(
        stats.excluded, 1,
        "should record the 1 invalid sample in excluded"
    );
    assert_eq!(
        stats.min, 10.0,
        "invalid sample 9999.0 must not affect minimum"
    );
    assert_eq!(stats.max, 40.0, "max must be 40.0 among valid samples");
    assert_eq!(
        stats.mean,
        (10.0 + 30.0 + 40.0) / 3.0,
        "mean must only average valid values"
    );
}

#[test]
fn region_where_every_sample_is_invalid_returns_none() {
    let times = [0.0, 1.0, 2.0];
    let values = [10.0, 20.0, 30.0];
    let valid = [false, false, false];

    assert_eq!(
        region_stats(&times, &values, Some(&valid), 0.0, 2.0),
        None,
        "a region where all samples are marked invalid must return None"
    );

    // Also test a sub-slice where only the selected region is invalid
    let mixed_valid = [true, false, false, true];
    let mixed_times = [0.0, 1.0, 2.0, 3.0];
    let mixed_values = [10.0, 20.0, 30.0, 40.0];
    assert_eq!(
        region_stats(&mixed_times, &mixed_values, Some(&mixed_valid), 1.0, 2.0),
        None,
        "region selecting only the invalid portion must return None"
    );
}

#[test]
fn single_sample_region_reports_sample_as_min_max_and_mean() {
    let times = [0.0, 1.5, 3.0];
    let values = [10.0, 42.5, 80.0];

    let stats = region_stats(&times, &values, None, 1.5, 1.5)
        .expect("single-point region must produce statistics");

    assert_eq!(stats.count, 1, "sample count must be exactly 1");
    assert_eq!(stats.excluded, 0, "no samples excluded");
    assert_eq!(stats.min, 42.5, "min must match the single sample value");
    assert_eq!(stats.max, 42.5, "max must match the single sample value");
    assert_eq!(stats.mean, 42.5, "mean must match the single sample value");
}

#[test]
fn nan_values_do_not_become_minimum_and_are_excluded() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [f64::NAN, 20.0, 60.0, f64::NAN];

    let stats = region_stats(&times, &values, None, 0.0, 3.0)
        .expect("region with finite values and NaNs should compute stats over finite values");

    assert_eq!(stats.count, 2, "only the 2 finite values should be counted");
    assert_eq!(
        stats.excluded, 2,
        "both NaN values should be counted as excluded"
    );
    assert_eq!(stats.min, 20.0, "NaN must not become min");
    assert_eq!(stats.max, 60.0, "max must be 60.0");
    assert_eq!(stats.mean, 40.0, "mean must be (20+60)/2 = 40.0");
}
