//! Two measurements open at once: keeping their channels apart, placing one
//! file's clock on the other's, and refusing an expression that spans both.
//!
//! These are the parts of comparing two runs that a window cannot be asked
//! about. The rule they pin down is that a `ChannelLoc` is only half an
//! address once a second file is open — the same three indices name a
//! channel in both files, usually with the same name — so everything that
//! addresses a channel carries the [`FileSlot`] too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use falcon_mdf::{Mf4File, Mf4Writer};
use falcon_mdf_gui::computed::{evaluate_visible_defs, ComputedDef};
use falcon_mdf_gui::model::{ChannelLoc, FileSlot, PlottedChannel};
use falcon_mdf_gui::panels::plot::{
    absolute_alignment_available, alignment_offset_seconds, TimeAlign,
};
use falcon_mdf_gui::session::{format_line, parse_line, prune_to_file, Session};
use falcon_mdf_gui::signal_loader::{decode_channel, SignalLoadResult};

fn loc(dg: usize, cg: usize, ch: usize) -> ChannelLoc {
    ChannelLoc {
        data_group_index: dg,
        channel_group_index: cg,
        channel_index: ch,
    }
}

/// Writes a one-group file whose channels are `(name, values)` and opens it.
/// `start_time_ns` goes into the `##HD` header, which is what absolute
/// alignment reads.
fn write_file(tag: &str, start_time_ns: i64, channels: &[(&str, Vec<f64>)]) -> Arc<Mf4File> {
    let times: Vec<f64> = (0..channels[0].1.len()).map(|i| i as f64).collect();
    let mut writer = Mf4Writer::with_start_time_ns(start_time_ns);
    let group = writer.add_group(&times).unwrap();
    for (name, values) in channels {
        group.add_channel(name, "km/h", values).unwrap();
    }
    // Unique per call: several of these tests build the same fixture, and
    // `cargo test` runs them on threads that would otherwise write and
    // delete one another's file.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "falcon_gui_compare_{tag}_{}_{serial}.mf4",
        std::process::id()
    ));
    writer.write_to_file(&path).unwrap();
    let file = Mf4File::open(&path).expect("the written file should open");
    let _ = std::fs::remove_file(&path);
    Arc::new(file)
}

/// The pair the comparison tests use: the same channel name in both files, at
/// the same location, holding different data. This is the case that makes a
/// bare `ChannelLoc` ambiguous, and it is the ordinary case — two runs of one
/// test are logged with one configuration.
fn two_runs() -> (Arc<Mf4File>, Arc<Mf4File>) {
    let a = write_file(
        "run_a",
        1_700_000_000_000_000_000,
        &[("Speed", vec![10.0, 11.0, 12.0, 13.0])],
    );
    let b = write_file(
        "run_b",
        1_700_000_012_500_000_000,
        &[("Speed", vec![90.0, 91.0, 92.0, 93.0])],
    );
    (a, b)
}

#[test]
fn two_files_open_at_once_keep_their_channels_distinct() {
    let (a, b) = two_runs();
    // Index 0 is the group's master; the first data channel is index 1.
    let speed = loc(0, 0, 1);

    // Same name, same location, two files — and two different signals. If a
    // plotted channel were addressed by its location alone, one of these
    // would be drawn where the other belongs.
    let SignalLoadResult::Ok(from_a) = decode_channel(&a, speed) else {
        panic!("file A's Speed should decode");
    };
    let SignalLoadResult::Ok(from_b) = decode_channel(&b, speed) else {
        panic!("file B's Speed should decode");
    };
    assert_eq!(from_a.name, from_b.name, "the fixture's point is one name");
    assert_eq!(from_a.values[0], 10.0);
    assert_eq!(from_b.values[0], 90.0);

    // Both plotted at once: two entries, not one, and each answers only to
    // its own file.
    let plotted = [
        PlottedChannel::new(FileSlot::A, speed, from_a.name.clone(), 0),
        PlottedChannel::new(FileSlot::B, speed, from_b.name.clone(), 1),
    ];
    assert_eq!(plotted.len(), 2);
    assert!(plotted[0].is(FileSlot::A, speed));
    assert!(!plotted[0].is(FileSlot::B, speed));
    assert!(plotted[1].is(FileSlot::B, speed));
    assert!(!plotted[1].is(FileSlot::A, speed));

    // Ticking the channel in one file's browser must not read as ticked in
    // the other's: that check is `is`, and it is what the tree and the
    // channel list use to decide whether a row is already plotted.
    assert_eq!(
        plotted.iter().filter(|p| p.is(FileSlot::A, speed)).count(),
        1
    );
    assert_eq!(
        plotted.iter().filter(|p| p.is(FileSlot::B, speed)).count(),
        1
    );

    // Their palette colours differ, so the two runs are told apart in the
    // legend by more than the badge.
    assert_ne!(plotted[0].color, plotted[1].color);
}

#[test]
fn each_file_restores_only_its_own_channels() {
    // Both files have Speed; only file B also has Rpm. Pruning is per file,
    // so B's second channel must not be restored against A — where that
    // location does not exist at all.
    let a = write_file("prune_a", 0, &[("Speed", vec![1.0, 2.0])]);
    let b = write_file(
        "prune_b",
        0,
        &[("Speed", vec![3.0, 4.0]), ("Rpm", vec![5.0, 6.0])],
    );

    // Index 1 is Speed in both files; index 2 is B's Rpm, which A does not
    // have at all.
    let session = Session {
        plotted: vec![
            (FileSlot::A, loc(0, 0, 1)),
            (FileSlot::B, loc(0, 0, 1)),
            (FileSlot::B, loc(0, 0, 2)),
        ],
        ..Session::default()
    };

    assert_eq!(
        prune_to_file(&session, FileSlot::A, &a),
        vec![loc(0, 0, 1)],
        "file A should get back its own one channel and nothing of B's"
    );
    assert_eq!(
        prune_to_file(&session, FileSlot::B, &b),
        vec![loc(0, 0, 1), loc(0, 0, 2)],
        "file B should get back both of its channels"
    );

    // Rpm's location is past the end of A's only group, so asking for slot B
    // against file A drops it — an entry is checked against the file it says
    // it is in, and restoring it blindly would index past a group.
    assert_eq!(
        prune_to_file(&session, FileSlot::B, &a),
        vec![loc(0, 0, 1)],
        "B's Rpm has no location in A, so it must not come back"
    );

    // And the slot filter comes first: pruning slot A returns A's one entry
    // even against a file that has every location the session names.
    assert_eq!(
        prune_to_file(&session, FileSlot::A, &b),
        vec![loc(0, 0, 1)],
        "B's entries are not A's, whichever file they are checked against"
    );
}

#[test]
fn a_session_with_two_files_round_trips() {
    let path = PathBuf::from("/measurements/run_a.mf4");
    let original = Session {
        plotted: vec![
            (FileSlot::A, loc(0, 0, 1)),
            (FileSlot::B, loc(2, 1, 7)),
            (FileSlot::A, loc(0, 0, 3)),
        ],
        nav: "Channels".to_string(),
        tab: "Plot".to_string(),
        cursor_a: Some(1.25),
        cursor_b: Some(4.75),
        computed: vec![ComputedDef::new("Power", "Speed * 2", "kW")],
        second: Some(PathBuf::from("/measurements/run_b.mf4")),
    };

    let (read_path, read_session) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_path, path);
    assert_eq!(read_session, original);
    assert_eq!(
        read_session.second,
        Some(PathBuf::from("/measurements/run_b.mf4")),
        "reopening the first file has to bring back the file it was compared against"
    );
    // Order survives too: it is what decides each channel's colour.
    assert_eq!(read_session.plotted[1].0, FileSlot::B);
}

#[test]
fn a_session_with_a_second_file_but_no_cursors_still_names_it() {
    // The trailing fields are positional. A second file with no cursors and
    // no computed channels has to write the empty fields before it, or the
    // path would be read back as a cursor.
    let path = PathBuf::from("/measurements/run_a.mf4");
    let original = Session {
        plotted: vec![(FileSlot::B, loc(1, 0, 2))],
        second: Some(PathBuf::from("/measurements/run_b.mf4")),
        ..Session::default()
    };

    let (_, read_session) =
        parse_line(&format_line(&path, &original)).expect("the line it wrote should parse");

    assert_eq!(read_session, original);
}

#[test]
fn a_session_written_before_there_was_a_second_file_still_reads() {
    // Lines the previous version wrote have no file prefix and no trailing
    // path. They mean: everything is in the file this line is keyed by, and
    // there is no comparison.
    let (_, session) = parse_line("/f.mf4\t0:0:1,2:1:7\tChannels\tPlot").expect("should parse");

    assert_eq!(
        session.plotted,
        vec![(FileSlot::A, loc(0, 0, 1)), (FileSlot::A, loc(2, 1, 7))]
    );
    assert_eq!(session.second, None);
}

#[test]
fn the_alignment_choice_is_read_from_the_headers() {
    // File B started 12.5 s after file A.
    let a_start = 1_700_000_000_000_000_000;
    let b_start = 1_700_000_012_500_000_000;

    // Each file's own zero: nothing moves, so both runs start at t = 0 and a
    // manoeuvre is compared against the same manoeuvre.
    assert_eq!(
        alignment_offset_seconds(TimeAlign::OwnZero, a_start, b_start),
        0.0
    );

    // Absolute time: B is shifted onto A's clock by the difference between
    // the two header start times.
    assert!(absolute_alignment_available(a_start, b_start));
    let offset = alignment_offset_seconds(TimeAlign::Absolute, a_start, b_start);
    assert!(
        (offset - 12.5).abs() < 1e-9,
        "expected B shifted by +12.5 s, got {offset}"
    );

    // And the other way round: a file that started earlier shifts back.
    let offset_back = alignment_offset_seconds(TimeAlign::Absolute, b_start, a_start);
    assert!(
        (offset_back + 12.5).abs() < 1e-9,
        "expected -12.5 s, got {offset_back}"
    );

    // Same instant in both headers: absolute alignment is available and is a
    // no-op, which is not the same thing as being unavailable.
    assert!(absolute_alignment_available(a_start, a_start));
    assert_eq!(
        alignment_offset_seconds(TimeAlign::Absolute, a_start, a_start),
        0.0
    );
}

#[test]
fn absolute_alignment_is_refused_when_a_header_has_no_start_time() {
    // A start time of zero is an unset header, not midnight in 1970.
    // Offering absolute alignment against it would shift a file by 53 years.
    let real = 1_700_000_000_000_000_000;
    assert!(!absolute_alignment_available(0, real));
    assert!(!absolute_alignment_available(real, 0));
    assert!(!absolute_alignment_available(0, 0));

    // And if the choice is asked for anyway, it is no shift rather than an
    // enormous one.
    assert_eq!(alignment_offset_seconds(TimeAlign::Absolute, 0, real), 0.0);
    assert_eq!(alignment_offset_seconds(TimeAlign::Absolute, real, 0), 0.0);
}

#[test]
fn the_files_own_start_times_drive_the_alignment_offset() {
    // The same check as above, but against files rather than integers: the
    // number the toolbar reports comes out of the two `##HD` headers.
    let (a, b) = two_runs();
    let a_start = a.start_time().timestamp_ns;
    let b_start = b.start_time().timestamp_ns;

    let offset = alignment_offset_seconds(TimeAlign::Absolute, a_start, b_start);
    assert!(
        (offset - 12.5).abs() < 1e-6,
        "the fixture's headers are 12.5 s apart; got {offset}"
    );
    assert_eq!(
        alignment_offset_seconds(TimeAlign::OwnZero, a_start, b_start),
        0.0
    );
}

#[test]
fn a_cross_file_expression_is_refused_rather_than_guessed() {
    // File A has Speed; file B has Speed and Rpm. An expression is evaluated
    // against one file, so `Rpm` — which only file B has — is not found when
    // the active file is A. It is refused with a named error rather than
    // resolved against the other file and resampled onto an alignment the
    // toolbar can change afterwards.
    let a = write_file("expr_a", 0, &[("Speed", vec![1.0, 2.0, 3.0])]);
    let b = write_file(
        "expr_b",
        0,
        &[("Speed", vec![4.0, 5.0, 6.0]), ("Rpm", vec![7.0, 8.0, 9.0])],
    );

    let def = ComputedDef::new("Cross", "Speed - Rpm", "");
    let mut operands = HashMap::new();
    let mut results = HashMap::new();
    let out = evaluate_visible_defs(
        std::slice::from_ref(&def),
        &a,
        1,
        &mut operands,
        &mut results,
    );

    assert_eq!(out.len(), 1);
    let message = out[0]
        .1
        .as_ref()
        .expect_err("an expression naming the other file's channel must fail");
    assert!(
        message.contains("unknown channel") && message.contains("Rpm"),
        "the error should name the channel it could not find, got: {message}"
    );

    // The same expression against file B, which has both channels, is fine —
    // so the refusal is about crossing files, not about the expression.
    let mut operands_b = HashMap::new();
    let mut results_b = HashMap::new();
    let out_b = evaluate_visible_defs(
        std::slice::from_ref(&def),
        &b,
        2,
        &mut operands_b,
        &mut results_b,
    );
    let signal = out_b[0]
        .1
        .as_ref()
        .expect("both operands are in file B, so it evaluates");
    assert_eq!(signal.values, vec![-3.0, -3.0, -3.0]);
}

#[test]
fn an_expression_resolves_in_the_file_it_is_evaluated_against() {
    // Both files have `Speed`, holding different data. Which one an
    // expression sees is decided by the file it is handed, and nothing else:
    // there is no fallback to the other file when a name is ambiguous,
    // because with two runs of one test every name is ambiguous.
    let (a, b) = two_runs();
    let def = ComputedDef::new("Doubled", "Speed * 2", "km/h");

    let evaluate = |file: &Arc<Mf4File>, id: usize| {
        let mut operands = HashMap::new();
        let mut results = HashMap::new();
        let out = evaluate_visible_defs(
            std::slice::from_ref(&def),
            file,
            id,
            &mut operands,
            &mut results,
        );
        out[0].1.as_ref().expect("Speed is in both files").values[0]
    };

    assert_eq!(evaluate(&a, 1), 20.0, "against A, Speed is A's Speed");
    assert_eq!(evaluate(&b, 2), 180.0, "against B, Speed is B's Speed");
}

#[test]
fn a_second_file_path_with_no_tab_survives_the_line() {
    // Paths are written raw into a tab-separated line. A path with a tab in
    // it would tear the line apart, so it is dropped rather than misread —
    // the same bargain the first file's path already makes.
    let path = Path::new("/measurements/run_a.mf4");
    let session = Session {
        second: Some(PathBuf::from("/measurements/a run/run_b.mf4")),
        ..Session::default()
    };

    let (_, read) = parse_line(&format_line(path, &session)).expect("should parse");

    assert_eq!(
        read.second,
        Some(PathBuf::from("/measurements/a run/run_b.mf4")),
        "a space in the path is not a separator"
    );
}
