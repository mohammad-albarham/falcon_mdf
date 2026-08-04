//! Min/max-per-pixel-column decimation for the plot.
//!
//! `egui_plot` tessellates every point it is given, every frame. Handing it a
//! signal's raw samples does not scale: a channel with hundreds of thousands
//! of samples would allocate and re-tessellate that many vertices on every
//! redraw, including frames where nothing changed. So the plot panel never
//! gives `egui_plot` more than roughly two points per pixel column.
//!
//! The aggregation is min/max, not "take every Nth sample" (stride
//! sampling). A single-sample spike is frequently the whole reason someone
//! opened the file, and stride sampling deletes it silently: the plot still
//! looks plausible, just wrong. Min/max per column instead scans every
//! visible sample once and, for each pixel-wide slice of the visible time
//! range, keeps the smallest and largest value seen — emitted in time order,
//! so a spike still reads as a vertical excursion rather than being averaged
//! or stepped away.
//!
//! A free function, not a method on some plot-owning type, so it can be
//! called from a test without an `egui::Context` — see `gui/tests/`.

/// Decimates `(times[i], values[i])` pairs to at most two points per column
/// of `n_columns` covering `x_range`, by min/max.
///
/// `times` must be sorted ascending and the same length as `values` — true of
/// every channel signal paired with its master, since both come from the same
/// channel group's records.
///
/// Samples outside `x_range` are dropped. When the visible sample count is
/// already at or below `n_columns * 2`, the visible samples are returned
/// untouched: there is nothing to aggregate away, and returning fewer points
/// than were asked for would just be a different way of lying about what the
/// file contains.
pub fn decimate_min_max(
    times: &[f64],
    values: &[f64],
    x_range: (f64, f64),
    n_columns: usize,
) -> Vec<[f64; 2]> {
    debug_assert_eq!(times.len(), values.len());
    if times.is_empty() || n_columns == 0 {
        return Vec::new();
    }

    let (x0, x1) = x_range;
    let start = times.partition_point(|&t| t < x0);
    let end = times.partition_point(|&t| t <= x1);
    if start >= end {
        return Vec::new();
    }

    let span = x1 - x0;
    if end - start <= n_columns * 2 || span <= 0.0 {
        return (start..end).map(|i| [times[i], values[i]]).collect();
    }

    let col_width = span / n_columns as f64;
    // At most one column beyond the nominal `n_columns`: a sample landing
    // exactly on `x1` computes `col_index == n_columns` below, one past the
    // last nominal column.
    let mut out = Vec::with_capacity((n_columns + 1) * 2);
    let mut i = start;
    while i < end {
        // This column runs from `col_end - col_width` to `col_end`, found from
        // the first untouched sample rather than a running counter, so gaps in
        // the data (an empty column) don't desynchronize later columns from
        // their true pixel boundaries.
        let col_index = ((times[i] - x0) / col_width) as usize;
        let col_end = x0 + (col_index + 1) as f64 * col_width;

        // `i` is unconditionally consumed before the boundary is tested
        // against any *other* sample, so the outer loop always makes
        // progress — one sample, at minimum — regardless of what `col_end`
        // evaluates to. This matters because it can legitimately equal `x0`
        // when `x0` is large and `col_width` is far smaller than `x0`'s ulp
        // (a narrow zoom on an epoch-seconds master, or many samples sharing
        // one timestamp defeating the below-threshold early return): testing
        // `times[i] < col_end` *before* consuming `i` could then find it
        // false forever and spin without ever advancing `i`.
        let mut min_i = i;
        let mut max_i = i;
        i += 1;
        while i < end && times[i] < col_end {
            if values[i] < values[min_i] {
                min_i = i;
            }
            if values[i] > values[max_i] {
                max_i = i;
            }
            i += 1;
        }

        if min_i == max_i {
            out.push([times[min_i], values[min_i]]);
        } else if min_i < max_i {
            out.push([times[min_i], values[min_i]]);
            out.push([times[max_i], values[max_i]]);
        } else {
            out.push([times[max_i], values[max_i]]);
            out.push([times[min_i], values[min_i]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_points() {
        let empty: Vec<[f64; 2]> = Vec::new();
        assert_eq!(decimate_min_max(&[], &[], (0.0, 1.0), 100), empty);
    }

    #[test]
    fn below_threshold_returns_samples_untouched() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![10.0, 20.0, 5.0, 15.0];
        let out = decimate_min_max(&times, &values, (0.0, 3.0), 100);
        assert_eq!(out, vec![[0.0, 10.0], [1.0, 20.0], [2.0, 5.0], [3.0, 15.0]]);
    }

    #[test]
    fn samples_outside_the_range_are_excluded() {
        let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let values: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let out = decimate_min_max(&times, &values, (2.0, 4.0), 100);
        assert_eq!(out, vec![[2.0, 2.0], [3.0, 3.0], [4.0, 4.0]]);
    }

    #[test]
    fn a_single_sample_spike_survives_min_max_decimation() {
        // 2000 flat samples, one spike, squeezed down to 50 columns (100
        // points max). The spike is the only excursion in the signal, so it
        // must come back out as the max of whatever column it lands in.
        let n = 2000;
        let spike_index = 733;
        let times: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut values = vec![0.0; n];
        values[spike_index] = 999.0;

        let out = decimate_min_max(&times, &values, (0.0, (n - 1) as f64), 50);
        assert!(out.len() <= 100);
        assert!(
            out.iter().any(|p| p[1] == 999.0),
            "the spike must survive decimation: {out:?}"
        );
    }

    #[test]
    fn identical_timestamps_at_a_large_epoch_do_not_hang() {
        // Regression. `col_end` is reconstructed as
        // `x0 + (col_index + 1) * col_width`; once `col_width` drops below
        // the ulp of `x0` (an epoch-seconds master, zoomed to a
        // few-nanosecond span), that addition rounds straight back to `x0`,
        // so `col_end <= times[i]` for every sample. The inner loop used to
        // test the boundary *before* consuming a sample, so it ran zero
        // iterations, `i` never advanced, and the outer loop spun forever.
        // 1000 identical timestamps also defeats the below-threshold early
        // return, which only looks at sample *count*, not whether the
        // samples are actually spread across the visible span — so the
        // decimation path is entered no matter how far this is zoomed.
        //
        // Run with a watchdog rather than just waiting: on the old code this
        // genuinely never returns, and a fixed timeout is how a test proves
        // that without stalling the suite.
        let x0: f64 = 1.7e9;
        let times = vec![x0; 1000];
        let values: Vec<f64> = (0..1000).map(|i| i as f64).collect();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = decimate_min_max(&times, &values, (x0, x0 + 1e-6), 200);
            let _ = tx.send(out);
        });

        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(out) => assert!(
                !out.is_empty(),
                "should still produce the (identical-timestamp) points, just not hang"
            ),
            Err(_) => panic!(
                "decimate_min_max hung: 1000 identical timestamps at x0={x0} over a \
                 1e-6-wide visible range never returned within 5s"
            ),
        }
    }

    #[test]
    fn identical_timestamps_with_a_sensible_range_collapse_to_one_column() {
        // Same duplicate-timestamp shape as the hang above, but `col_width`
        // here (1.0 / 200) is nowhere near `x0`'s ulp, so every sample lands
        // in column 0 and the loop finishes in one pass: this was never
        // broken, and stays that way.
        let x0: f64 = 5.0;
        let times = vec![x0; 1000];
        let values: Vec<f64> = (0..1000).map(|i| i as f64).collect();

        let out = decimate_min_max(&times, &values, (x0, x0 + 1.0), 200);
        assert!(
            out.len() <= 2,
            "every sample shares one timestamp, so it's all one column: {out:?}"
        );
        assert!(out.iter().any(|p| p[1] == 0.0));
        assert!(out.iter().any(|p| p[1] == 999.0));
    }

    #[test]
    fn a_reversed_range_yields_no_points() {
        // `x1 < x0` is a range no visible sample can fall in: `end`
        // (samples `<= x1`) can never exceed `start` (samples `< x0`) when
        // `x1 < x0`, so the `start >= end` guard already catches it.
        let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let values = times.clone();
        let empty: Vec<[f64; 2]> = Vec::new();
        assert_eq!(decimate_min_max(&times, &values, (7.0, 3.0), 100), empty);
    }

    #[test]
    fn min_and_max_of_a_noisy_column_both_survive() {
        // A column holding several samples keeps its extremes, not just one.
        let times = vec![0.0, 0.1, 0.2, 0.3, 0.4];
        let values = vec![5.0, -3.0, 8.0, 1.0, 5.0];
        // A range wider than the data so every sample falls in the same
        // single column; a range ending exactly on the last sample would put
        // it in the next (empty) column instead.
        let out = decimate_min_max(&times, &values, (0.0, 0.5), 1);
        // One column: the min (-3.0 at t=0.1) before the max (8.0 at t=0.2)
        // in the source, so they come back in that order.
        assert_eq!(out, vec![[0.1, -3.0], [0.2, 8.0]]);
    }
}
