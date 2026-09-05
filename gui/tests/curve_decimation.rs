//! The X-Y / GPS curve decimator: order-preserving min/max per pixel column
//! for a parametric curve whose x axis is *not* sorted. Pinned separately
//! from the time-plot decimator because the two make different promises —
//! `decimate_min_max` may emit min-then-max per column since its x is
//! ascending; a curve would get crossings invented that way.

use falcon_mdf_gui::decimate::decimate_curve;

#[test]
fn a_small_curve_is_returned_untouched() {
    let points = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.5]];
    let out = decimate_curve(&points, (0.0, 2.0), 100);
    assert_eq!(out, points, "nothing to aggregate away");
}

#[test]
fn a_single_sample_spike_survives_decimation() {
    // A dense run with one spike, all inside a few columns. The spike is the
    // column's max, so it must survive — deleting it would silently redraw
    // the measurement without its most interesting sample.
    let mut points: Vec<[f64; 2]> = (0..1000)
        .map(|i| [i as f64, 1.0 + (i % 7) as f64 * 0.1])
        .collect();
    points[500] = [500.0, 1000.0];

    let out = decimate_curve(&points, (0.0, 999.0), 8);
    assert!(
        out.len() < points.len(),
        "the point count must actually shrink"
    );
    assert!(out.iter().any(|p| p[1] == 1000.0), "the spike must be kept");
}

#[test]
fn a_curve_that_doubles_back_keeps_its_traversal_order() {
    // x alternates between two columns: a curve sweeping left and right. The
    // kept points must still alternate — grouping each column's points
    // together (or emitting min-then-max) would invent a different curve.
    let points: Vec<[f64; 2]> = (0..40)
        .map(|i| {
            [
                if i % 2 == 0 { 0.0 } else { 1.0 },
                (i / 2) as f64, // rises monotonically along the traversal
            ]
        })
        .collect();

    let out = decimate_curve(&points, (0.0, 1.0), 2);
    assert!(out.len() < points.len(), "repetition must be dropped");
    for pair in out.windows(2) {
        assert!(
            pair[0][0] != pair[1][0],
            "consecutive kept points must keep alternating columns, got {:?}",
            pair
        );
    }
    // And the traversal is still in time order: y never decreases.
    for pair in out.windows(2) {
        assert!(pair[0][1] <= pair[1][1], "y must stay non-decreasing");
    }
}

#[test]
fn points_outside_the_view_are_dropped() {
    let points: Vec<[f64; 2]> = (0..50)
        .map(|i| [i as f64, i as f64])
        .chain((100..150).map(|i| [i as f64, i as f64]))
        .collect();
    let out = decimate_curve(&points, (0.0, 49.0), 4);
    assert!(out.iter().all(|p| p[0] <= 49.0), "only the view survives");
    assert!(!out.is_empty());
}

#[test]
fn the_kept_count_is_bounded_by_the_columns() {
    // At most four kept points per column (first, min, max, last), whatever
    // the input density.
    let points: Vec<[f64; 2]> = (0..100_000)
        .map(|i| [(i % 1000) as f64, ((i * 37) % 1000) as f64])
        .collect();
    let n_columns = 64usize;
    let out = decimate_curve(&points, (0.0, 999.0), n_columns);
    assert!(
        out.len() <= n_columns * 4,
        "kept {} points for {n_columns} columns",
        out.len()
    );
    assert!(out.len() < points.len() / 10, "the point count must shrink");
}
