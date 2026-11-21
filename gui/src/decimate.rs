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
/// Samples outside `x_range` are dropped, except the one nearest sample on
/// each side: without it, a view zoomed in between two samples would contain
/// none and draw nothing, and the line would stop short of the view's edges
/// instead of running off them. When the visible sample count is
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
    // Widened by one sample each side; see the doc comment.
    let start = times.partition_point(|&t| t < x0).saturating_sub(1);
    let end = (times.partition_point(|&t| t <= x1) + 1).min(times.len());
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
            // A NaN never wins a comparison, so a NaN held as the column's
            // min or max would hide every spike after it; any number
            // replaces it. (`decimate_min_max_gaps` keeps non-finite values
            // out entirely; this is the guard for direct callers.)
            if values[i] < values[min_i] || values[min_i].is_nan() {
                min_i = i;
            }
            if values[i] > values[max_i] || values[max_i].is_nan() {
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

/// Validity-aware variant of [`decimate_min_max`]: the drawable result is a
/// list of *segments*, one per maximal run of valid samples.
///
/// The file's invalidation bit marks samples whose record bits are not a
/// measurement (`Signal::validity`); `values` still holds whatever garbage
/// those bits decode to. Such a sample must neither be drawn nor contribute
/// to a column's min/max — a garbage 1e9 next to a 0..5 V channel would
/// otherwise stretch every column it touches and flatten the real data. So
/// each valid run is decimated on its own, and invalid runs simply produce
/// no segment: the plot then draws the runs as separate lines with a visible
/// gap where the file said "not measured". Segments are split rather than
/// emitted as one point list because `egui_plot`'s `Line` connects every
/// consecutive pair of points it is given (and does not treat NaN as a
/// break), so a single list would draw a fake line across the gap.
///
/// Splitting per run does not change the aggregation: column boundaries are
/// computed from absolute times (`(t - x0) / col_width`), so a pixel column
/// means the same thing in every segment — a column straddling a gap may
/// emit its min/max once per neighbouring segment, which is conservative,
/// never lossy. The spike-survival and no-hang properties of
/// [`decimate_min_max`] carry over unchanged, since every segment is produced
/// by that same function.
///
/// A NaN or infinite value is a gap too, whatever `valid` says: it has no
/// place on the axis, and a NaN inside a column would otherwise poison that
/// column's min/max comparisons and hide its spikes.
///
/// A segment of one point is a valid sample with gaps on both sides. It is
/// returned like any other; the caller must draw it as a marker, since a
/// `Line` of one point draws nothing.
///
/// `valid` of `None` (the channel carries no invalidation info) splits only
/// on non-finite values. A `valid` slice whose length doesn't match `values`
/// cannot be lined up with the samples, so it is treated as "everything
/// valid" rather than trusted.
pub fn decimate_min_max_gaps(
    times: &[f64],
    values: &[f64],
    valid: Option<&[bool]>,
    x_range: (f64, f64),
    n_columns: usize,
) -> Vec<Vec<[f64; 2]>> {
    let valid = valid.filter(|v| v.len() == values.len());
    let n = times.len().min(values.len());
    let drawable = |i: usize| values[i].is_finite() && valid.is_none_or(|v| v[i]);

    // Walk for runs of valid samples and decimate each run separately. The
    // loop index runs one past the end so a run that reaches the last sample
    // is closed by the same code path as one closed by an invalid sample.
    let mut segments = Vec::new();
    let mut run_start = None;
    for i in 0..=n {
        let still_valid = i < n && drawable(i);
        match (run_start, still_valid) {
            (None, true) => run_start = Some(i),
            (Some(start), false) => {
                let points =
                    decimate_min_max(&times[start..i], &values[start..i], x_range, n_columns);
                if !points.is_empty() {
                    segments.push(points);
                }
                run_start = None;
            }
            _ => {}
        }
    }
    segments
}

/// Decimates a parametric curve — an X-Y plot or a GPS track — to at most
/// four points per column of `n_columns` covering `x_range` per segment,
/// preserving the curve's traversal order.
///
/// The time plot's [`decimate_min_max`] leans on its x axis being sorted; a
/// curve's x is whatever the measured channel did and can double back, so
/// emitting min-then-max per column would reorder the curve and invent
/// crossings. Instead each column keeps the first, lowest, highest and last
/// of *its own* points, and the kept points are emitted in the order the
/// curve visited them. A single-sample spike survives (it is a column's min
/// or max); so does a reversal (first/last bracket it); only repetition
/// inside one column is dropped.
///
/// The result is a list of segments: one per stretch of the traversal that
/// stays inside `x_range`. A track that leaves the view and comes back is
/// two segments, not one list — joined, `Line` would draw a straight chord
/// from the exit to the re-entry that the curve never took. Each segment
/// keeps the point just outside the view on either end (when there is one),
/// so the line runs off the edge where the curve did. A NaN or infinite
/// coordinate breaks the curve the same way. A one-point segment must be
/// drawn as a marker by the caller.
pub fn decimate_curve(
    points: &[[f64; 2]],
    x_range: (f64, f64),
    n_columns: usize,
) -> Vec<Vec<[f64; 2]>> {
    if points.is_empty() || n_columns == 0 {
        return Vec::new();
    }
    let (x0, x1) = x_range;
    let finite = |p: &[f64; 2]| p[0].is_finite() && p[1].is_finite();
    let inside = |p: &[f64; 2]| finite(p) && p[0] >= x0 && p[0] <= x1;

    let mut segments = Vec::new();
    let mut i = 0;
    while i < points.len() {
        if !inside(&points[i]) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < points.len() && inside(&points[end]) {
            end += 1;
        }
        // The neighbours just outside the view, if they are real points.
        let first = if i > 0 && finite(&points[i - 1]) {
            i - 1
        } else {
            i
        };
        let last = if end < points.len() && finite(&points[end]) {
            end + 1
        } else {
            end
        };
        segments.push(decimate_run(&points[first..last], x_range, n_columns));
        i = end;
    }
    segments
}

/// The per-column first/min/max/last reduction of one curve segment.
fn decimate_run(run: &[[f64; 2]], (x0, x1): (f64, f64), n_columns: usize) -> Vec<[f64; 2]> {
    if run.len() <= n_columns * 4 || x1 <= x0 {
        return run.to_vec();
    }

    let col_width = (x1 - x0) / n_columns as f64;
    // One more than the nominal column count: a point landing exactly on `x1`
    // computes the index one past the last column. The edge points outside
    // the view clamp into the first or last column (`as usize` saturates a
    // negative index to 0).
    #[derive(Clone, Copy)]
    struct Kept {
        first: usize,
        min: usize,
        max: usize,
        last: usize,
    }
    let mut columns: Vec<Option<Kept>> = vec![None; n_columns + 1];
    for (i, &[x, y]) in run.iter().enumerate() {
        let col = (((x - x0) / col_width) as usize).min(n_columns);
        columns[col] = Some(match columns[col] {
            None => Kept {
                first: i,
                min: i,
                max: i,
                last: i,
            },
            Some(kept) => Kept {
                first: kept.first,
                min: if y < run[kept.min][1] { i } else { kept.min },
                max: if y > run[kept.max][1] { i } else { kept.max },
                last: i,
            },
        });
    }

    // Traversal order: sort by visit index, dedup the roles that coincide.
    // The run's own first and last points are always kept, so the segment
    // still reaches the view's edges.
    let mut keep: Vec<usize> = columns
        .iter()
        .flatten()
        .flat_map(|k| [k.first, k.min, k.max, k.last])
        .chain([0, run.len() - 1])
        .collect();
    keep.sort_unstable();
    keep.dedup();
    keep.into_iter().map(|i| run[i]).collect()
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
    fn samples_outside_the_range_are_excluded_but_one_each_side() {
        let times: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let values: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let out = decimate_min_max(&times, &values, (2.0, 4.0), 100);
        assert_eq!(
            out,
            vec![[1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [4.0, 4.0], [5.0, 5.0]]
        );
    }

    #[test]
    fn a_view_between_two_samples_still_draws_the_line_through_it() {
        let out = decimate_min_max(&[0.0, 10.0], &[1.0, 2.0], (4.0, 6.0), 100);
        assert_eq!(out, vec![[0.0, 1.0], [10.0, 2.0]]);
    }

    #[test]
    fn a_leading_nan_does_not_hide_a_spike_in_its_column() {
        // 400 samples into 10 columns: column 0 starts with a NaN and holds
        // a spike. Called directly (no gap splitting) to pin the comparison.
        let times: Vec<f64> = (0..400).map(|i| i as f64).collect();
        let mut values = vec![0.0; 400];
        values[0] = f64::NAN;
        values[5] = 500.0;
        let out = decimate_min_max(&times, &values, (0.0, 399.0), 10);
        assert!(out.iter().any(|p| p[1] == 500.0), "{out:?}");
    }

    #[test]
    fn non_finite_values_are_gaps_and_isolated_points_survive() {
        let times = [0.0, 1.0, 2.0, 3.0, 4.0];
        let values = [1.0, f64::NAN, 2.0, f64::INFINITY, 3.0];
        let segments = decimate_min_max_gaps(&times, &values, None, (0.0, 4.0), 100);
        assert_eq!(
            segments,
            vec![vec![[0.0, 1.0]], vec![[2.0, 2.0]], vec![[4.0, 3.0]]]
        );
    }

    #[test]
    fn a_curve_leaving_and_reentering_the_view_is_two_segments() {
        // In view, out to x = 50, back in view: no chord from 2 to 3.
        let points = [[0.0, 0.0], [2.0, 1.0], [50.0, 2.0], [3.0, 3.0], [4.0, 4.0]];
        let segments = decimate_curve(&points, (0.0, 10.0), 100);
        assert_eq!(
            segments,
            vec![
                vec![[0.0, 0.0], [2.0, 1.0], [50.0, 2.0]],
                vec![[50.0, 2.0], [3.0, 3.0], [4.0, 4.0]],
            ]
        );
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

    #[test]
    fn invalid_runs_break_segments_and_never_contribute_extremes() {
        // Two valid runs around an invalid one whose values are pure garbage
        // (1e9 against a 0-valued signal). 400 valid samples decimated to 10
        // columns takes the aggregation path, so if an invalid sample leaked
        // into a column's min/max it would show up as a 1e9 point.
        let n = 1000;
        let times: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut values = vec![0.0; n];
        let mut valid = vec![true; n];
        for i in 400..600 {
            values[i] = 1e9;
            valid[i] = false;
        }

        let segments =
            decimate_min_max_gaps(&times, &values, Some(&valid), (0.0, (n - 1) as f64), 10);

        assert_eq!(segments.len(), 2, "one invalid run, two valid runs");
        assert!(
            segments[0].iter().all(|p| p[0] < 400.0),
            "first segment ends at the gap: {:?}",
            segments[0]
        );
        assert!(
            segments[1].iter().all(|p| p[0] >= 600.0),
            "second segment starts after the gap: {:?}",
            segments[1]
        );
        assert!(
            segments.iter().flatten().all(|p| p[1] == 0.0),
            "invalid garbage must never reach the output: {segments:?}"
        );
    }

    #[test]
    fn a_spike_inside_a_valid_run_survives_gap_decimation() {
        // Same shape as the legacy spike test, plus an invalid run elsewhere
        // in the signal: the spike must still come out, and the invalid
        // run's garbage must not.
        let n = 2000;
        let times: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut values = vec![0.0; n];
        values[733] = 999.0;
        let mut valid = vec![true; n];
        for i in 1200..1300 {
            values[i] = -500.0;
            valid[i] = false;
        }

        let segments =
            decimate_min_max_gaps(&times, &values, Some(&valid), (0.0, (n - 1) as f64), 50);

        assert_eq!(segments.len(), 2);
        assert!(
            segments.iter().flatten().any(|p| p[1] == 999.0),
            "the spike must survive decimation: {segments:?}"
        );
        assert!(
            segments.iter().flatten().all(|p| p[1] != -500.0),
            "invalid samples must not be drawn: {segments:?}"
        );
    }

    #[test]
    fn all_invalid_yields_no_points() {
        let times: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let values = vec![1.0; 100];
        let valid = vec![false; 100];
        let segments = decimate_min_max_gaps(&times, &values, Some(&valid), (0.0, 99.0), 10);
        assert!(segments.is_empty());
    }

    #[test]
    fn no_validity_info_matches_legacy_decimation() {
        let n = 2000;
        let times: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let values: Vec<f64> = (0..n).map(|i| ((i * 37) % 100) as f64).collect();
        let range = (0.0, (n - 1) as f64);

        let legacy = decimate_min_max(&times, &values, range, 50);
        let segments = decimate_min_max_gaps(&times, &values, None, range, 50);
        assert_eq!(segments, vec![legacy]);
    }

    #[test]
    fn a_mismatched_validity_length_is_treated_as_all_valid() {
        // A validity vector that doesn't line up with the samples can't be
        // trusted; fall back to legacy behavior rather than panic or guess.
        let times: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let values = vec![1.0; 100];
        let too_short = vec![false; 50];

        let legacy = decimate_min_max(&times, &values, (0.0, 99.0), 10);
        let segments = decimate_min_max_gaps(&times, &values, Some(&too_short), (0.0, 99.0), 10);
        assert_eq!(segments, vec![legacy]);
    }
}
