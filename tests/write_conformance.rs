//! The writer's output read by an independent tool: asammdf, if the project's
//! `.venv` provides it. The reader in this crate was audited against the
//! standard, but a writer and a reader built in one repo can share a misreading
//! and still round-trip happily — an outside parser is the only oracle that
//! breaks that symmetry. Skips cleanly where the `.venv` (or asammdf) is
//! absent, so CI without Python still runs everything else.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::Mf4Writer;

/// The first interpreter that exists: one beside the crate, then the shared one
/// two levels up.
///
/// The second candidate is not optional housekeeping. Agent worktrees are
/// checked out beside the repository and have no `.venv` of their own, so with
/// only the first path this test skipped — and reported as a pass — in every
/// one of them. A writer conformance test that silently does nothing is worse
/// than no test at all.
fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn asammdf_reads_what_the_writer_writes() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in the .venv");
        return;
    }

    let times: Vec<f64> = (0..100).map(|i| f64::from(i) * 0.1).collect();
    let speed: Vec<f64> = times
        .iter()
        .map(|t| 50.0 + (t * 0.5).sin() * 30.0)
        .collect();
    let boost: Vec<f64> = times.iter().map(|t| t * 0.2).collect();
    let boost_valid: Vec<bool> = (0..100).map(|i| !(40..60).contains(&i)).collect();

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group.add_channel("Speed", "km/h", &speed).unwrap();
    group
        .add_channel_with_validity("Boost", "psi", &boost, Some(&boost_valid))
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    let script = r#"
import math
import sys

from asammdf import MDF

path = sys.argv[1]
m = MDF(path)
assert m.version.startswith("4.1"), m.version
assert set(m.channels_db) == {"Time", "Speed", "Boost"}, sorted(m.channels_db)

speed = m.get("Speed")
assert speed.unit == "km/h", speed.unit
assert len(speed.samples) == 100, len(speed.samples)
for i, (t, v) in enumerate(zip(speed.timestamps, speed.samples)):
    assert abs(t - i * 0.1) < 1e-9, (i, t)
    assert abs(v - (50.0 + math.sin(i * 0.1 * 0.5) * 30.0)) < 1e-9, (i, v)

# asammdf drops invalid samples on get(): 20 were marked invalid, so exactly
# 80 must come back — the same polarity the standard assigns the bit — and
# the first sample after the gap is the one at t = 6.0.
boost = m.get("Boost")
assert boost.unit == "psi", boost.unit
assert len(boost.samples) == 80, len(boost.samples)
assert abs(boost.timestamps[39] - 3.9) < 1e-9, boost.timestamps[39]
assert abs(boost.timestamps[40] - 6.0) < 1e-9, boost.timestamps[40]
assert abs(boost.samples[40] - 6.0 * 0.2) < 1e-9, boost.samples[40]
print("OK")
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(temp.path())
        .output()
        .expect("failed to run the .venv python");
    assert!(
        output.status.success(),
        "asammdf rejected the written file:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
