//! A batch cut or filter writes a new file that must say what the input
//! said: invalid samples stay invalid, and the recording keeps its start
//! time. Before GUI-03 both were lost — invalid samples were written back as
//! valid measurements and every output claimed to start in 1970 (or now).
//! It also covers outputs colliding in a shared output folder.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use falcon_mdf::{Mf4File, Mf4Writer, SignalValues};
use falcon_mdf_gui::batch::{run_all, run_one, BatchOp};

/// 2024-06-01T12:00:00Z.
const START_NS: i64 = 1_717_243_200_000_000_000;

/// Five samples of `Speed`, the second and fourth invalid.
fn file_with_invalid_samples(dir: &Path, name: &str) -> PathBuf {
    let mut writer = Mf4Writer::with_start_time_ns(START_NS);
    let group = writer.add_group(&[0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
    group
        .add_channel_typed_with(
            "Speed",
            "km/h",
            SignalValues::F64(vec![10.0, -1.0, 30.0, -1.0, 50.0]),
            Some(&[true, false, true, false, true]),
            None,
        )
        .unwrap();
    let path = dir.join(name);
    writer.write_to_file(&path).unwrap();
    path
}

/// The validity of `Speed` and the start time of the file at `path`.
fn read_back(path: &Path) -> (Option<Vec<bool>>, i64) {
    let file = Mf4File::open(path).unwrap();
    let speed = file.channels().find(|c| c.name == "Speed").unwrap();
    let validity = file.signal(speed).unwrap().validity();
    (validity, file.start_time().timestamp_ns)
}

#[test]
fn cut_keeps_invalid_samples_invalid_and_the_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let input = file_with_invalid_samples(dir.path(), "run.mf4");
    let op = BatchOp::Cut {
        start: 1.0,
        end: 3.0,
    };
    run_one(&input, &op, Some(dir.path())).unwrap();

    let (validity, start) = read_back(&dir.path().join("run.cut.mf4"));
    // Samples at t = 1, 2, 3: invalid, valid, invalid.
    assert_eq!(validity, Some(vec![false, true, false]));
    assert_eq!(start, START_NS);
}

#[test]
fn filter_keeps_invalid_samples_invalid_and_the_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let input = file_with_invalid_samples(dir.path(), "run.mf4");
    let op = BatchOp::Filter {
        names: vec!["Speed".to_string()],
    };
    run_one(&input, &op, Some(dir.path())).unwrap();

    let (validity, start) = read_back(&dir.path().join("run.filtered.mf4"));
    assert_eq!(validity, Some(vec![true, false, true, false, true]));
    assert_eq!(start, START_NS);
}

#[test]
fn same_named_inputs_get_separate_outputs_in_a_shared_folder() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let paths = vec![
        file_with_invalid_samples(a.path(), "run.mf4"),
        file_with_invalid_samples(b.path(), "run.mf4"),
    ];
    let op = BatchOp::Filter {
        names: vec!["Speed".to_string()],
    };
    let outcomes = run_all(
        &paths,
        &op,
        Some(out.path()),
        &AtomicBool::new(false),
        |_| {},
    );

    assert!(outcomes.iter().all(|o| o.result.is_ok()), "{outcomes:?}");
    assert!(out.path().join("run.filtered.mf4").exists());
    assert!(out.path().join("run.filtered.2.mf4").exists());
}
