//! Plotting one channel against another: how the two are paired, and every
//! case where they cannot be.
//!
//! An X-Y curve has no time axis, so nothing in the picture tells the reader
//! whether the pairing behind it was exact, interpolated, or invented. That
//! makes the pairing rules the whole substance of the feature, and it is why
//! the refusals below are refusals rather than best guesses with a warning.

use std::path::PathBuf;

use falcon_mdf::{Mf4File, Mf4Writer};
use falcon_mdf_gui::model::{ChannelLoc, ChannelRef, FileSlot, XyChannels};
use falcon_mdf_gui::session::{format_line, parse_line, prune_xy, Session};
use falcon_mdf_gui::signal_loader::ChannelSignal;
use falcon_mdf_gui::xy::{pair_xy, Axis, XyPairing, XyRefusal};

fn loc(dg: usize, cg: usize, ch: usize) -> ChannelLoc {
    ChannelLoc {
        data_group_index: dg,
        channel_group_index: cg,
        channel_index: ch,
    }
}

fn signal(
    name: &str,
    times: Vec<f64>,
    values: Vec<f64>,
    valid: Option<Vec<bool>>,
) -> ChannelSignal {
    ChannelSignal {
        loc: loc(0, 0, 0),
        name: name.to_string(),
        unit: "u".to_string(),
        time_name: "Time".to_string(),
        time_unit: "s".to_string(),
        times,
        values,
        valid,
    }
}

/// The happy case that needs no interpolation at all.
#[test]
fn channels_sharing_a_master_pair_sample_for_sample() {
    let x = signal("Steering", vec![0.0, 1.0, 2.0], vec![0.0, 5.0, 10.0], None);
    let y = signal("LatAccel", vec![0.0, 1.0, 2.0], vec![0.0, 0.5, 1.0], None);

    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).expect("one master, so exact");

    assert_eq!(series.pairing, XyPairing::Exact);
    assert_eq!(series.points, vec![[0.0, 0.0], [5.0, 0.5], [10.0, 1.0]]);
    assert_eq!(series.times, vec![0.0, 1.0, 2.0]);
    assert_eq!(series.dropped, 0);
}

#[test]
fn different_rasters_interpolate_y_onto_x_over_the_overlap() {
    // X runs 0..3 on a 1 s raster; Y runs 0..2 with a sample only at each
    // end. The pairing keeps X's own timestamps — those are measurements —
    // and interpolates Y onto them, over 0..2 where both have data. X's
    // sample at t = 3 has no Y to pair with and is left out rather than
    // pinned to Y's last value.
    let x = signal(
        "X",
        vec![0.0, 1.0, 2.0, 3.0],
        vec![0.0, 10.0, 20.0, 30.0],
        None,
    );
    let y = signal("Y", vec![0.0, 2.0], vec![0.0, 20.0], None);

    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).expect("the spans overlap");

    assert_eq!(series.pairing, XyPairing::Resampled);
    assert_eq!(
        series.times,
        vec![0.0, 1.0, 2.0],
        "t = 3 is outside Y's span"
    );
    assert_eq!(series.points, vec![[0.0, 0.0], [10.0, 10.0], [20.0, 20.0]]);
}

#[test]
fn channels_that_never_overlap_are_refused() {
    // Two recordings in one file that ran at different times. Every pairing
    // here would be an extrapolation, and the curve it drew would look like
    // a measured relationship.
    let x = signal("X", vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0], None);
    let y = signal("Y", vec![10.0, 11.0], vec![4.0, 5.0], None);

    let refusal = pair_xy(&x, 0.0, &y, 0.0, false, false).expect_err("no shared instant");

    match refusal {
        XyRefusal::NoOverlap { x_span, y_span } => {
            assert_eq!(x_span, (0.0, 2.0));
            assert_eq!(y_span, (10.0, 11.0));
        }
        other => panic!("expected NoOverlap, got {other:?}"),
    }
    // The message has to say what is wrong, not just that something is.
    let message = refusal.message();
    assert!(
        message.contains("never recording at the same time"),
        "{message}"
    );
}

#[test]
fn an_empty_channel_is_refused_and_names_its_axis() {
    let full = signal("X", vec![0.0, 1.0], vec![1.0, 2.0], None);
    let empty = signal("Y", vec![], vec![], None);

    assert_eq!(
        pair_xy(&full, 0.0, &empty, 0.0, false, false).expect_err("Y is empty"),
        XyRefusal::NoSamples { axis: Axis::Y }
    );
    assert_eq!(
        pair_xy(&empty, 0.0, &full, 0.0, false, false).expect_err("X is empty"),
        XyRefusal::NoSamples { axis: Axis::X }
    );
}

#[test]
fn a_cross_file_pair_is_refused_until_the_files_share_a_clock() {
    // This is the X-Y counterpart of W9's cross-file expression rule. Under
    // each file's own zero, t = 1 in one file and t = 1 in the other are
    // different instants that happen to share a number; pairing them draws a
    // relationship that is an artefact of where each run was triggered.
    let x = signal("Speed", vec![0.0, 1.0, 2.0], vec![10.0, 20.0, 30.0], None);
    let y = signal("Rpm", vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0], None);

    let refusal = pair_xy(&x, 0.0, &y, 0.0, true, false).expect_err("own-zero cannot pair files");
    assert_eq!(refusal, XyRefusal::CrossFileNeedsAbsoluteTime);
    let message = refusal.message();
    assert!(
        message.contains("Align B to A"),
        "the refusal should name the fix: {message}"
    );

    // Aligned on the headers' wall clock, the same pair is meaningful and is
    // drawn. Same data, same call, one flag different.
    let series = pair_xy(&x, 0.0, &y, 0.0, true, true).expect("absolute alignment pairs them");
    assert_eq!(series.pairing, XyPairing::Exact);
    assert_eq!(series.points.len(), 3);
}

#[test]
fn the_second_files_alignment_offset_moves_its_samples() {
    // Y is file B's, shifted +1 s onto A's clock. Overlap becomes 1..2, and
    // Y's value at a paired instant is the one it held at that *shared* time,
    // not at the same number on its own clock.
    let x = signal("X", vec![0.0, 1.0, 2.0], vec![10.0, 20.0, 30.0], None);
    let y = signal("Y", vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0], None);

    let series = pair_xy(&x, 0.0, &y, 1.0, true, true).expect("overlapping after the shift");

    assert_eq!(series.times, vec![1.0, 2.0], "only 1..2 is covered by both");
    // At shared t = 1, Y's own clock reads 0, where it held 1.0.
    assert_eq!(series.points, vec![[20.0, 1.0], [30.0, 2.0]]);
}

#[test]
fn invalid_and_nan_samples_are_dropped_and_counted() {
    // A sample the file marks invalid holds whatever bits the record had, not
    // data. Pairing it would put a real X against a meaningless Y.
    let x = signal("X", vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0], None);
    let y = signal(
        "Y",
        vec![0.0, 1.0, 2.0],
        vec![10.0, 20.0, 30.0],
        Some(vec![true, false, true]),
    );

    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).expect("two pairs survive");

    assert_eq!(series.points, vec![[1.0, 10.0], [3.0, 30.0]]);
    assert_eq!(series.times, vec![0.0, 2.0]);
    assert_eq!(series.dropped, 1);

    // A NaN is dropped for the same reason.
    let nan_y = signal("Y", vec![0.0, 1.0], vec![f64::NAN, 5.0], None);
    let x2 = signal("X", vec![0.0, 1.0], vec![1.0, 2.0], None);
    let series = pair_xy(&x2, 0.0, &nan_y, 0.0, false, false).expect("one pair survives");
    assert_eq!(series.points, vec![[2.0, 5.0]]);
    assert_eq!(series.dropped, 1);
}

#[test]
fn overlapping_channels_with_nothing_valid_are_refused() {
    // The spans overlap, so "no overlap" would be the wrong answer; there is
    // simply nothing measured to draw.
    let x = signal("X", vec![0.0, 1.0], vec![1.0, 2.0], None);
    let y = signal(
        "Y",
        vec![0.0, 1.0],
        vec![10.0, 20.0],
        Some(vec![false, false]),
    );

    let refusal = pair_xy(&x, 0.0, &y, 0.0, false, false).expect_err("nothing usable");

    assert_eq!(refusal, XyRefusal::NothingValid { dropped: 2 });
}

#[test]
fn a_cursor_marks_the_point_the_curve_was_at_that_instant() {
    let x = signal("X", vec![0.0, 1.0, 2.0], vec![0.0, 10.0, 20.0], None);
    let y = signal("Y", vec![0.0, 1.0, 2.0], vec![5.0, 6.0, 7.0], None);
    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).unwrap();

    // A cursor is a time; the X-Y view answers with the point, and with the
    // instant that point was actually measured at.
    let m = series.point_at(0.0).unwrap();
    assert_eq!(m.point, [0.0, 5.0]);
    assert_eq!(m.time, 0.0);
    assert_eq!(series.point_at(1.0).unwrap().point, [10.0, 6.0]);
    // Between samples it snaps to the nearer one, like the time plot's readout.
    assert_eq!(series.point_at(1.4).unwrap().point, [10.0, 6.0]);
    assert_eq!(series.point_at(1.6).unwrap().point, [20.0, 7.0]);
    assert_eq!(series.span(), Some((0.0, 2.0)));
}

#[test]
fn a_cursor_outside_the_paired_span_marks_nothing() {
    // Clamping to the nearest end would pin a marker to the last point and
    // read as though the curve were there at that time. It was not.
    let x = signal("X", vec![0.0, 1.0], vec![0.0, 10.0], None);
    let y = signal("Y", vec![0.0, 1.0], vec![5.0, 6.0], None);
    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).unwrap();

    assert_eq!(series.point_at(9.0), None);
    assert_eq!(series.point_at(-3.0), None);
}

#[test]
fn the_xy_axes_round_trip_through_a_session() {
    let path = PathBuf::from("/measurements/run_a.mf4");
    let original = Session {
        plotted: vec![(FileSlot::A, loc(0, 0, 1)), (FileSlot::B, loc(2, 1, 7))],
        nav: "Channels".to_string(),
        tab: "X-Y".to_string(),
        second: Some(PathBuf::from("/measurements/run_b.mf4")),
        xy: Some(XyChannels {
            x: ChannelRef::new(FileSlot::A, loc(0, 0, 1)),
            y: ChannelRef::new(FileSlot::B, loc(2, 1, 7)),
        }),
        ..Session::default()
    };

    let (read_path, read) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_path, path);
    assert_eq!(read, original);
    let xy = read.xy.expect("the axes come back");
    assert_eq!(xy.x.file, FileSlot::A);
    assert_eq!(xy.y.file, FileSlot::B);
    assert!(xy.is_cross_file());
}

#[test]
fn an_xy_selection_with_no_cursors_or_second_file_still_round_trips() {
    // The trailing fields are positional: X-Y is the last of them, so every
    // earlier field has to be written even when it has nothing to say.
    let path = PathBuf::from("/f.mf4");
    let original = Session {
        xy: Some(XyChannels {
            x: ChannelRef::new(FileSlot::A, loc(0, 0, 1)),
            y: ChannelRef::new(FileSlot::A, loc(0, 0, 2)),
        }),
        ..Session::default()
    };

    let (_, read) = parse_line(&format_line(&path, &original)).expect("should parse");

    assert_eq!(read.xy, original.xy);
    assert_eq!(read.second, None);
    assert!(read.computed.is_empty());
}

#[test]
fn a_line_from_before_the_xy_view_existed_still_reads() {
    let (_, session) = parse_line("/f.mf4\t0:0:1\tChannels\tPlot").expect("should parse");
    assert_eq!(session.xy, None);
}

/// Writes a one-group file whose channels are `(name, values)` and opens it.
fn write_file(tag: &str, channels: &[(&str, Vec<f64>)]) -> Mf4File {
    let times: Vec<f64> = (0..channels[0].1.len()).map(|i| i as f64).collect();
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    for (name, values) in channels {
        group.add_channel(name, "u", values).unwrap();
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "falcon_gui_xy_{tag}_{}_{serial}.mf4",
        std::process::id()
    ));
    writer.write_to_file(&path).unwrap();
    let file = Mf4File::open(&path).expect("the written file should open");
    let _ = std::fs::remove_file(&path);
    file
}

#[test]
fn restored_axes_are_checked_against_the_file_each_one_names() {
    // Index 0 is the master; A has Speed at 1, B has Speed at 1 and Rpm at 2.
    let a = write_file("prune_a", &[("Speed", vec![1.0, 2.0])]);
    let b = write_file(
        "prune_b",
        &[("Speed", vec![3.0, 4.0]), ("Rpm", vec![5.0, 6.0])],
    );
    let files = [(FileSlot::A, &a), (FileSlot::B, &b)];

    let good = Session {
        xy: Some(XyChannels {
            x: ChannelRef::new(FileSlot::A, loc(0, 0, 1)),
            y: ChannelRef::new(FileSlot::B, loc(0, 0, 2)),
        }),
        ..Session::default()
    };
    assert_eq!(
        prune_xy(&good, &files),
        good.xy,
        "both axes are still there"
    );

    // B's Rpm has no location in A. An axis pointing at it must drop the
    // whole selection: half an X-Y plot still draws a curve, and that curve
    // would be about a channel the user never picked.
    let moved = Session {
        xy: Some(XyChannels {
            x: ChannelRef::new(FileSlot::A, loc(0, 0, 2)),
            y: ChannelRef::new(FileSlot::B, loc(0, 0, 1)),
        }),
        ..Session::default()
    };
    assert_eq!(
        prune_xy(&moved, &files),
        None,
        "X is past the end of file A"
    );

    // And with only the first file open, an axis in B cannot be checked at
    // all, so it is dropped rather than assumed.
    assert_eq!(prune_xy(&good, &[(FileSlot::A, &a)]), None);
}

// --- Regressions from the R3 review of this commit ---

#[test]
fn a_target_a_hair_outside_ys_range_is_dropped_not_clamped() {
    // R3 finding 2. `resample_linear` clamps outside its source range rather
    // than refusing, so an X timestamp that slipped past the overlap bound by
    // less than the epsilon used to come back holding Y's endpoint value.
    // Invented data is exactly what this module exists not to produce, so the
    // overlap bounds are exact and the stray sample is dropped.
    let x = signal(
        "X",
        vec![-0.5e-9, 0.5, 1.0 + 0.5e-9],
        vec![10.0, 20.0, 30.0],
        None,
    );
    let y = signal("Y", vec![0.0, 1.0], vec![100.0, 200.0], None);

    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).expect("the middle sample pairs");

    assert_eq!(
        series.points,
        vec![[20.0, 150.0]],
        "only the sample genuinely inside Y's span survives"
    );
    assert_eq!(series.times, vec![0.5]);
}

#[test]
fn a_cursor_in_a_recording_gap_reports_the_samples_own_time() {
    // R3 finding 5.2. The cursor is inside the outer span, so a point is
    // matched — but the nearest sample is half an hour away. Reporting the
    // cursor's own time beside that sample's values would claim a measurement
    // that was never taken.
    let x = signal("X", vec![0.0, 3600.0], vec![10.0, 20.0], None);
    let y = signal("Y", vec![0.0, 3600.0], vec![50.0, 60.0], None);
    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).unwrap();

    let m = series
        .point_at(1800.0)
        .expect("inside the span, so something matches");
    assert_eq!(m.point, [10.0, 50.0]);
    assert_eq!(
        m.time, 0.0,
        "the match carries the sample's instant, not the cursor's"
    );
    assert!(
        (m.time - 1800.0).abs() > 1.0,
        "and it is far enough from the cursor for the panel to say so"
    );
}

#[test]
fn a_single_point_overlap_still_produces_a_series() {
    // R3 finding 5.1. One paired sample is a legitimate — if minimal —
    // answer, and the panel forces the sample markers on for it, because
    // egui_plot draws no line through a single point and the canvas would
    // otherwise be blank under a caption saying "1 points".
    let x = signal("X", vec![0.0, 1.0, 2.0], vec![10.0, 20.0, 30.0], None);
    let y = signal("Y", vec![1.0], vec![50.0], None);

    let series = pair_xy(&x, 0.0, &y, 0.0, false, false).expect("one instant is covered by both");

    assert_eq!(series.points, vec![[20.0, 50.0]]);
    assert_eq!(series.times, vec![1.0]);
    assert_eq!(series.span(), Some((1.0, 1.0)));
}
