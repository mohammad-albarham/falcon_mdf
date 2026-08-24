//! Tests for measurement cursors (A and B, Δt and ΔY maths) in the falcon MF4 viewer plot panel.

use falcon_mdf_gui::panels::plot::{cursor_measurement, nearest_index, CursorMeasurement};

#[test]
fn cursors_at_known_positions_compute_exact_dt_and_dy() {
    let times = [0.0, 1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 25.0, 50.0, 15.0, 40.0];

    let m = cursor_measurement(&times, &values, None, Some(1.0), Some(3.0));

    assert_eq!(
        m,
        CursorMeasurement {
            value_a: Some(25.0),
            valid_a: true,
            value_b: Some(15.0),
            valid_b: true,
            delta_t: Some(2.0),
            delta_y: Some(-10.0),
        },
        "measurement at known sample points must match exact values, dt, and dy"
    );
    assert_eq!(m.value_a, Some(25.0), "value at cursor A (t=1.0) must be 25.0");
    assert_eq!(m.value_b, Some(15.0), "value at cursor B (t=3.0) must be 15.0");
    assert_eq!(m.delta_t, Some(2.0), "delta t must be 3.0 - 1.0 = 2.0");
    assert_eq!(m.delta_y, Some(-10.0), "delta Y must be 15.0 - 25.0 = -10.0");
}

#[test]
fn reversed_cursors_swap_dt_and_dy_signs() {
    let times = [0.0, 1.0, 2.0, 3.0, 4.0];
    let values = [10.0, 25.0, 50.0, 15.0, 40.0];

    // Cursor A at t=3.0, Cursor B at t=1.0
    let m = cursor_measurement(&times, &values, None, Some(3.0), Some(1.0));

    assert_eq!(m.value_a, Some(15.0), "value at cursor A (t=3.0) is 15.0");
    assert_eq!(m.value_b, Some(25.0), "value at cursor B (t=1.0) is 25.0");
    assert_eq!(m.delta_t, Some(-2.0), "delta t (B - A) must be 1.0 - 3.0 = -2.0");
    assert_eq!(m.delta_y, Some(10.0), "delta Y (val_B - val_A) must be 25.0 - 15.0 = 10.0");
}

#[test]
fn cursor_between_samples_snaps_to_nearest_sample() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [100.0, 200.0, 300.0, 400.0];

    // Cursor A at 1.2 -> closer to 1.0 (val 200.0) than 2.0
    // Cursor B at 2.8 -> closer to 3.0 (val 400.0) than 2.0
    let m = cursor_measurement(&times, &values, None, Some(1.2), Some(2.8));

    assert_eq!(m.value_a, Some(200.0), "cursor A at t=1.2 snaps to sample at t=1.0");
    assert_eq!(m.value_b, Some(400.0), "cursor B at t=2.8 snaps to sample at t=3.0");
    assert!((m.delta_t.unwrap() - 1.6).abs() < 1e-9, "delta t is exact cursor diff (2.8 - 1.2 = 1.6)");
    assert_eq!(m.delta_y, Some(200.0), "delta Y is difference of snapped samples (400 - 200 = 200)");
}

#[test]
fn cursor_outside_signal_range_clamps_to_boundary_samples() {
    let times = [5.0, 6.0, 7.0];
    let values = [10.0, 20.0, 30.0];

    // Cursor A before first sample, Cursor B after last sample
    let m = cursor_measurement(&times, &values, None, Some(-10.0), Some(100.0));

    assert_eq!(m.value_a, Some(10.0), "cursor before range clamps to first sample");
    assert_eq!(m.value_b, Some(30.0), "cursor after range clamps to last sample");
    assert_eq!(m.delta_t, Some(110.0), "delta t preserves cursor difference (100 - (-10))");
    assert_eq!(m.delta_y, Some(20.0), "delta Y is last sample minus first sample (30 - 10)");
}

#[test]
fn invalid_sample_at_cursor_reports_invalid_and_no_delta_y() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 9999.0, 30.0, 40.0];
    let valid = [true, false, true, true];

    // Cursor A on valid sample (t=0.0), Cursor B on invalid sample (t=1.0)
    let m = cursor_measurement(&times, &values, Some(&valid), Some(0.0), Some(1.0));

    assert_eq!(m.valid_a, true, "sample at cursor A is valid");
    assert_eq!(m.valid_b, false, "sample at cursor B is invalid");
    assert_eq!(m.delta_t, Some(1.0), "delta t is still computed when a sample is invalid");
    assert_eq!(m.delta_y, None, "delta Y cannot be computed when either sample is invalid");

    // Reversed: Cursor A on invalid, Cursor B on valid
    let m_rev = cursor_measurement(&times, &values, Some(&valid), Some(1.0), Some(3.0));
    assert_eq!(m_rev.valid_a, false);
    assert_eq!(m_rev.valid_b, true);
    assert_eq!(m_rev.delta_y, None);
}

#[test]
fn nan_sample_at_cursor_reports_no_delta_y() {
    let times = [0.0, 1.0, 2.0];
    let values = [10.0, f64::NAN, 30.0];

    let m = cursor_measurement(&times, &values, None, Some(0.0), Some(1.0));

    assert!(m.value_b.unwrap().is_nan(), "value at cursor B is NaN");
    assert_eq!(m.delta_y, None, "NaN sample prevents delta Y calculation");
}

#[test]
fn single_cursor_reports_individual_value_with_no_deltas() {
    let times = [0.0, 1.0, 2.0];
    let values = [10.0, 20.0, 30.0];

    // Only cursor A placed
    let m_a = cursor_measurement(&times, &values, None, Some(1.0), None);
    assert_eq!(m_a.value_a, Some(20.0));
    assert_eq!(m_a.value_b, None);
    assert_eq!(m_a.delta_t, None);
    assert_eq!(m_a.delta_y, None);

    // Only cursor B placed
    let m_b = cursor_measurement(&times, &values, None, None, Some(2.0));
    assert_eq!(m_b.value_a, None);
    assert_eq!(m_b.value_b, Some(30.0));
    assert_eq!(m_b.delta_t, None);
    assert_eq!(m_b.delta_y, None);

    // Neither cursor placed
    let m_none = cursor_measurement(&times, &values, None, None, None);
    assert_eq!(m_none.value_a, None);
    assert_eq!(m_none.value_b, None);
    assert_eq!(m_none.delta_t, None);
    assert_eq!(m_none.delta_y, None);
}

#[test]
fn coincident_cursors_yield_zero_delta_t_and_delta_y() {
    let times = [0.0, 1.0, 2.0, 3.0];
    let values = [10.0, 20.0, 30.0, 40.0];

    let m = cursor_measurement(&times, &values, None, Some(2.0), Some(2.0));

    assert_eq!(m.value_a, Some(30.0));
    assert_eq!(m.value_b, Some(30.0));
    assert_eq!(m.delta_t, Some(0.0));
    assert_eq!(m.delta_y, Some(0.0));
}

#[test]
fn nearest_index_finds_exact_and_midpoint_samples() {
    let times = [0.0, 1.0, 2.0, 3.0];
    assert_eq!(nearest_index(&times, 0.0), 0);
    assert_eq!(nearest_index(&times, 0.49), 0);
    assert_eq!(nearest_index(&times, 0.51), 1);
    assert_eq!(nearest_index(&times, 1.0), 1);
    assert_eq!(nearest_index(&times, 2.99), 3);
    assert_eq!(nearest_index(&times, 3.0), 3);
}
