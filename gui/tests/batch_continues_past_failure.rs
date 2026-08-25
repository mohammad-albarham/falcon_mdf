//! A batch is run over files nobody hand-checked first.
//!
//! One of them will be truncated, or be a `.mf4` that is not one. The rule
//! this file exists to hold down is that such a file is reported **by name,
//! with its reason**, and the rest of the queue still runs. A batch that
//! stopped at the first bad file would have done the user no favour: they
//! wanted the other nine processed.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use falcon_mdf_gui::batch::{run_all, run_one, summarise, BatchOp, Outcome, Progress};
use falcon_mdf::Mf4Writer;

/// Writes a small, valid measurement and returns its path.
fn good_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
    group
        .add_channel("Speed", "km/h", &[10.0, 20.0, 30.0, 40.0, 50.0])
        .unwrap();
    group
        .add_channel("Temp", "degC", &[80.0, 81.0, 82.0, 83.0, 84.0])
        .unwrap();
    let path = dir.join(name);
    writer.write_to_file(&path).unwrap();
    path
}

/// Writes something with an `.mf4` name that is not a measurement.
fn broken_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"this is not an MF4 file, not even close").unwrap();
    path
}

/// The rule: the bad file is named, the good ones still run.
#[test]
fn a_failing_file_is_reported_and_the_batch_continues() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();

    // The broken file sits in the middle, so a batch that aborts on failure
    // and one that never reaches the end look different.
    let first = good_file(dir.path(), "first.mf4");
    let bad = broken_file(dir.path(), "middle.mf4");
    let last = good_file(dir.path(), "last.mf4");
    let paths = vec![first.clone(), bad.clone(), last.clone()];

    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let outcomes = run_all(
        &paths,
        &BatchOp::Export,
        Some(out.path()),
        &cancel,
        |p| seen.push(p),
    );

    // Every file got an outcome, in queue order.
    assert_eq!(outcomes.len(), 3, "one outcome per queued file");
    assert_eq!(outcomes[0].path, first);
    assert_eq!(outcomes[1].path, bad);
    assert_eq!(outcomes[2].path, last);

    // The batch did not stop at the failure.
    assert!(outcomes[0].succeeded(), "first: {:?}", outcomes[0].result);
    assert!(!outcomes[1].succeeded(), "the broken file should have failed");
    assert!(outcomes[2].succeeded(), "last: {:?}", outcomes[2].result);

    // The failure names the file and says why.
    assert_eq!(outcomes[1].file_name(), "middle.mf4");
    let reason = outcomes[1].result.as_ref().unwrap_err();
    assert!(
        !reason.trim().is_empty(),
        "a failure must carry a reason, not an empty string"
    );

    // The good files actually produced output; the bad one did not.
    assert!(out.path().join("first.export.csv").is_file());
    assert!(out.path().join("last.export.csv").is_file());
    assert!(
        !out.path().join("middle.export.csv").is_file(),
        "a failed file must not leave a half-written output behind"
    );

    // Progress was reported for all three, not just up to the failure.
    let started: Vec<usize> = seen
        .iter()
        .filter_map(|p| match p {
            Progress::Started { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    let finished: Vec<usize> = seen
        .iter()
        .filter_map(|p| match p {
            Progress::Finished { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec![0, 1, 2], "every file should be announced");
    assert_eq!(finished, vec![0, 1, 2], "every file should report an outcome");

    assert_eq!(summarise(&outcomes), "2 processed, 1 failed");
}

/// A queue that is nothing but broken files still returns one outcome each.
#[test]
fn every_file_failing_is_still_one_outcome_each() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let paths = vec![
        broken_file(dir.path(), "a.mf4"),
        broken_file(dir.path(), "b.mf4"),
    ];

    let cancel = AtomicBool::new(false);
    let outcomes = run_all(&paths, &BatchOp::Export, Some(out.path()), &cancel, |_| {});

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| !o.succeeded()));
    assert_eq!(summarise(&outcomes), "all 2 files failed");
}

/// A missing file is a failure like any other, not a panic.
#[test]
fn a_missing_file_fails_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let present = good_file(dir.path(), "present.mf4");
    let absent = dir.path().join("nowhere.mf4");

    let cancel = AtomicBool::new(false);
    let outcomes = run_all(
        &[absent.clone(), present.clone()],
        &BatchOp::Export,
        Some(out.path()),
        &cancel,
        |_| {},
    );

    assert_eq!(outcomes.len(), 2);
    assert!(!outcomes[0].succeeded(), "a missing file cannot be exported");
    assert!(outcomes[1].succeeded(), "the file that is there still runs");
}

/// Cutting is applied to every file, and the output is a readable measurement
/// holding only the samples in range.
#[test]
fn cut_runs_over_the_whole_queue() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let a = good_file(dir.path(), "a.mf4");
    let b = good_file(dir.path(), "b.mf4");
    let bad = broken_file(dir.path(), "bad.mf4");

    let cancel = AtomicBool::new(false);
    let outcomes = run_all(
        &[a, bad, b],
        &BatchOp::Cut {
            start: 1.0,
            end: 3.0,
        },
        Some(out.path()),
        &cancel,
        |_| {},
    );

    assert_eq!(outcomes.len(), 3);
    assert!(outcomes[0].succeeded(), "{:?}", outcomes[0].result);
    assert!(!outcomes[1].succeeded());
    assert!(outcomes[2].succeeded(), "{:?}", outcomes[2].result);

    // The cut file opens and holds exactly the samples in range. The oracle
    // is the range that was asked for, not anything the cut reported.
    let cut = falcon_mdf::Mf4File::open(out.path().join("a.cut.mf4")).unwrap();
    let speed = cut.find_channel("Speed").expect("Speed survives the cut");
    let values = cut.signal(speed).unwrap().values_f64().unwrap();
    assert_eq!(values, vec![20.0, 30.0, 40.0]);
}

/// Keeping a set of channels drops the rest, and says which names were absent.
#[test]
fn filter_keeps_only_the_named_channels() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = good_file(dir.path(), "a.mf4");

    let message = run_one(
        &path,
        &BatchOp::Filter {
            names: vec!["Speed".to_string(), "NotThere".to_string()],
        },
        Some(out.path()),
    )
    .expect("filtering should succeed when at least one name matches");

    assert!(message.contains("NotThere"), "absent names should be named: {message}");

    let filtered = falcon_mdf::Mf4File::open(out.path().join("a.filtered.mf4")).unwrap();
    assert!(filtered.find_channel("Speed").is_some());
    assert!(
        filtered.find_channel("Temp").is_none(),
        "a channel that was not asked for should be gone"
    );
}

/// A filter that matches nothing is a failure, not an empty file that looks
/// like success.
#[test]
fn a_filter_matching_nothing_fails_rather_than_writing_an_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = good_file(dir.path(), "a.mf4");

    let err = run_one(
        &path,
        &BatchOp::Filter {
            names: vec!["Nonexistent".to_string()],
        },
        Some(out.path()),
    )
    .expect_err("a filter that keeps nothing should fail");
    assert!(err.contains("none of"), "{err}");
    assert!(!out.path().join("a.filtered.mf4").exists());
}

/// A cut whose range misses every sample fails rather than writing an empty
/// measurement.
#[test]
fn a_cut_outside_the_data_fails() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = good_file(dir.path(), "a.mf4");

    let err = run_one(
        &path,
        &BatchOp::Cut {
            start: 100.0,
            end: 200.0,
        },
        Some(out.path()),
    )
    .expect_err("a range holding no samples should fail");
    assert!(err.contains("nothing to write"), "{err}");
}

/// A bad time range is refused once, before any file is opened.
#[test]
fn an_impossible_range_is_refused_before_the_run() {
    assert!(BatchOp::Cut { start: 5.0, end: 1.0 }.validate().is_err());
    assert!(BatchOp::Cut { start: f64::NAN, end: 1.0 }.validate().is_err());
    assert!(BatchOp::Cut { start: 0.0, end: 1.0 }.validate().is_ok());
    assert!(BatchOp::Filter { names: vec![" ".into()] }.validate().is_err());
    assert!(BatchOp::Export.validate().is_ok());
}

/// Outputs are named after their input and never overwrite it.
#[test]
fn output_is_named_after_its_input() {
    use falcon_mdf_gui::batch::output_path;
    let input = PathBuf::from("/data/run_07.mf4");

    let cut = output_path(&input, &BatchOp::Cut { start: 0.0, end: 1.0 }, None);
    assert_eq!(cut, PathBuf::from("/data/run_07.cut.mf4"));
    assert_ne!(cut, input, "a batch must never write over its input");

    let export = output_path(&input, &BatchOp::Export, Some(std::path::Path::new("/out")));
    assert_eq!(export, PathBuf::from("/out/run_07.export.csv"));
}

/// Cancelling stops the run without losing the outcomes already collected.
#[test]
fn cancelling_stops_the_run_and_keeps_what_finished() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let paths = vec![
        good_file(dir.path(), "a.mf4"),
        good_file(dir.path(), "b.mf4"),
        good_file(dir.path(), "c.mf4"),
    ];

    // Cancelled after the first file is finished.
    let cancel = AtomicBool::new(false);
    let outcomes = run_all(&paths, &BatchOp::Export, Some(out.path()), &cancel, |p| {
        if matches!(p, Progress::Finished { index: 0, .. }) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });

    assert_eq!(outcomes.len(), 1, "the run stops between files");
    assert!(outcomes[0].succeeded());
}

/// An outcome carries the file's own name, since a batch is usually a
/// directory of paths that differ only at the end.
#[test]
fn an_outcome_names_the_file_not_the_path() {
    let outcome = Outcome {
        path: PathBuf::from("/a/very/long/path/to/run_07.mf4"),
        result: Err("truncated".to_string()),
    };
    assert_eq!(outcome.file_name(), "run_07.mf4");
    assert!(!outcome.succeeded());
}
