//! The batch engine run over every file in the corpus.
//!
//! The unit tests beside this one build their inputs with `Mf4Writer`, which
//! means they only ever meet the shapes that writer produces. Vendor files
//! carry the rest: array channels, CANopen dates, complex samples, groups
//! whose time base starts nowhere near zero. This runs the real operations
//! over all of them and checks the property that has to hold whatever is in
//! the queue — **every file gets exactly one outcome, and a failure is a
//! message naming a reason rather than a panic or a stalled run.**
//!
//! The corpus is fetched rather than committed, so in a worktree it lives in
//! the primary checkout; both locations are tried, and the test skips when
//! neither has it.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use falcon_mdf_gui::batch::{run_all, BatchOp, Progress};

fn corpus_dir() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("../test_data/reference"),
        manifest.join("../../../falcon_mdf/test_data/reference"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
}

fn reference_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mf4"))
        })
        .collect();
    files.sort();
    files
}

/// Every file gets exactly one outcome, whatever is in it.
#[test]
fn every_queued_file_gets_exactly_one_outcome() {
    let Some(dir) = corpus_dir() else {
        eprintln!("corpus absent; skipping");
        return;
    };
    let paths = reference_files(&dir);
    assert!(!paths.is_empty(), "the corpus directory holds no MF4 files");

    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);

    for op in [
        BatchOp::Export,
        BatchOp::Cut {
            start: 0.0,
            end: 1.0,
        },
        BatchOp::Filter {
            names: vec!["Data channel".to_string()],
        },
    ] {
        let mut finished = Vec::new();
        let outcomes = run_all(&paths, &op, Some(out.path()), &cancel, |p| {
            if let Progress::Finished { index, .. } = p {
                finished.push(index);
            }
        });

        let label = op.label();
        assert_eq!(
            outcomes.len(),
            paths.len(),
            "{label}: one outcome per queued file, however many failed"
        );
        assert_eq!(
            finished,
            (0..paths.len()).collect::<Vec<_>>(),
            "{label}: every file must be reported, in queue order"
        );
        for (outcome, path) in outcomes.iter().zip(&paths) {
            assert_eq!(&outcome.path, path, "{label}: outcomes stay in queue order");
            if let Err(reason) = &outcome.result {
                assert!(
                    !reason.trim().is_empty(),
                    "{label}: {} failed with an empty reason",
                    outcome.file_name()
                );
            }
        }
    }
}

/// Exporting is the operation with no way to refuse a readable file, so every
/// file the viewer can open must come out as CSV.
#[test]
fn every_readable_file_exports() {
    let Some(dir) = corpus_dir() else {
        eprintln!("corpus absent; skipping");
        return;
    };
    let paths = reference_files(&dir);
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);

    let outcomes = run_all(&paths, &BatchOp::Export, Some(out.path()), &cancel, |_| {});

    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.succeeded())
        .map(|o| format!("{}: {}", o.file_name(), o.result.as_ref().unwrap_err()))
        .collect();
    assert!(failed.is_empty(), "these files did not export: {failed:#?}");

    // And every one of them left a file behind.
    for outcome in &outcomes {
        let csv = out.path().join(format!(
            "{}.export.csv",
            outcome.path.file_stem().unwrap().to_string_lossy()
        ));
        assert!(csv.is_file(), "no CSV for {}", outcome.file_name());
    }
}

/// Cutting meets channel types the writer has no record layout for. Those
/// files must fail by name and leave the rest of the queue alone — which is
/// the whole batch rule, exercised against files nobody wrote for the test.
#[test]
fn a_corpus_cut_reports_its_failures_without_stopping() {
    let Some(dir) = corpus_dir() else {
        eprintln!("corpus absent; skipping");
        return;
    };
    let paths = reference_files(&dir);
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);

    let outcomes = run_all(
        &paths,
        &BatchOp::Cut {
            start: 0.0,
            end: 1.0,
        },
        Some(out.path()),
        &cancel,
        |_| {},
    );

    let succeeded = outcomes.iter().filter(|o| o.succeeded()).count();
    let failed = outcomes.len() - succeeded;

    // The corpus is deliberately full of awkward files, so both must happen:
    // a run where nothing failed would mean this test had stopped covering the
    // rule, and one where nothing succeeded would mean cutting was broken.
    assert!(succeeded > 0, "no file survived a cut");
    assert!(
        failed > 0,
        "the corpus holds channel types the writer refuses; none were met"
    );
    assert_eq!(
        succeeded + failed,
        paths.len(),
        "every file is accounted for"
    );

    // The last file in the queue was still processed, so no failure aborted
    // the run part-way.
    assert_eq!(outcomes.last().unwrap().path, *paths.last().unwrap());

    // Every file that succeeded wrote a measurement that opens again.
    for outcome in outcomes.iter().filter(|o| o.succeeded()) {
        let cut = out.path().join(format!(
            "{}.cut.mf4",
            outcome.path.file_stem().unwrap().to_string_lossy()
        ));
        assert!(cut.is_file(), "no output for {}", outcome.file_name());
        falcon_mdf::Mf4File::open(&cut)
            .unwrap_or_else(|e| panic!("{} produced an unreadable cut: {e}", outcome.file_name()));
    }
}
