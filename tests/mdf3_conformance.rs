//! MDF 3.x structure, checked against files asammdf wrote.
//!
//! The expected values here come from asammdf reading the same file, not from
//! this crate reading it. That matters: three silent-corruption defects in this
//! repository survived their tests because the tests used the implementation's
//! own inverse as their oracle, and so only ever agreed with themselves.

#![cfg(feature = "mdf3")]

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::mdf3::Mdf3File;

/// Locates the virtualenv python that has asammdf installed.
fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|c| c.is_file())
}

/// Fails loudly rather than passing quietly.
///
/// A test that returns early when its fixture generator is missing reports as
/// a pass while proving nothing, which is worse than no test at all.
fn python_or_skip() -> PathBuf {
    match venv_python() {
        Some(p) => p,
        None => panic!(
            "no .venv/bin/python found; this test needs asammdf to generate its \
             fixtures. Run it from the repository, or mark it ignored deliberately."
        ),
    }
}

/// Writes an MDF 3.x file with asammdf and returns its path plus the channel
/// names and units asammdf itself reports for it.
fn generate(version: &str, dir: &Path) -> (PathBuf, Vec<String>, Vec<String>) {
    let python = python_or_skip();
    let path = dir.join(format!("gen_{}.mdf", version.replace('.', "_")));
    let script = format!(
        r#"
import json, numpy as np
from asammdf import MDF, Signal

t = np.arange(0.0, 1.0, 0.1)
sigs = [
    Signal(samples=(t * 3.0).astype(np.float64), timestamps=t, name="Speed", unit="km/h"),
    Signal(samples=(t * 100).astype(np.int16), timestamps=t, name="Rpm", unit="1/min"),
    Signal(samples=(t * 2).astype(np.uint8), timestamps=t, name="Gear", unit=""),
]
m = MDF(version="{version}")
m.append(sigs)
m.save(r"{path}", overwrite=True)
m.close()

back = MDF(r"{path}")
names, units = [], []
for group_index, group in enumerate(back.groups):
    for ch_index, ch in enumerate(group.channels):
        names.append(ch.name)
        units.append(back.get_channel_unit(group=group_index, index=ch_index) or "")
print(json.dumps({{"names": names, "units": units, "version": back.version}}))
back.close()
"#,
        version = version,
        path = path.display()
    );

    let out = Command::new(&python)
        .arg("-c")
        .arg(&script)
        .output()
        .expect("running asammdf should succeed");
    assert!(
        out.status.success(),
        "asammdf failed to write {version}: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .last()
        .expect("the generator should print one JSON line");
    let parsed: serde_json::Value =
        serde_json::from_str(line).expect("the generator's output should be JSON");

    let names = parsed["names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let units = parsed["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    (path, names, units)
}

#[test]
fn a_file_asammdf_wrote_opens_and_reports_the_same_channels() {
    let dir = tempfile::tempdir().expect("a temp dir");

    for version in ["3.30", "3.20", "2.14"] {
        let (path, expected_names, expected_units) = generate(version, dir.path());

        let file = Mdf3File::open(&path)
            .unwrap_or_else(|e| panic!("falcon should open the {version} file asammdf wrote: {e}"));

        assert_eq!(
            file.version(),
            version,
            "the version falcon reports should be the one asammdf wrote"
        );

        let got: Vec<String> = file.channel_names().iter().map(|s| s.to_string()).collect();
        assert_eq!(
            got, expected_names,
            "{version}: channel names should match what asammdf reports for the same file"
        );

        // Units come from the conversion block; asammdf writes an empty unit as
        // an absent one, so compare only where asammdf reported something.
        for (i, expected) in expected_units.iter().enumerate() {
            if expected.is_empty() {
                continue;
            }
            let channel = file
                .find_channel(&expected_names[i])
                .unwrap_or_else(|| panic!("{version}: channel {} should be found", expected_names[i]));
            assert_eq!(
                &channel.unit, expected,
                "{version}: unit for {} should match asammdf",
                expected_names[i]
            );
        }
    }
}

#[test]
fn the_record_layout_matches_what_asammdf_declares() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (path, _, _) = generate("3.30", dir.path());
    let python = python_or_skip();

    let script = format!(
        r#"
import json
from asammdf import MDF
m = MDF(r"{path}")
out = []
for g in m.groups:
    out.append({{
        "record_size": g.channel_group.samples_byte_nr,
        "cycles": g.channel_group.cycles_nr,
        "channels": len(g.channels),
    }})
print(json.dumps(out))
m.close()
"#,
        path = path.display()
    );
    let out = Command::new(&python).arg("-c").arg(&script).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected: serde_json::Value =
        serde_json::from_str(stdout.lines().last().unwrap()).unwrap();

    let file = Mdf3File::open(&path).expect("falcon should open it");
    let groups: Vec<_> = file
        .data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .collect();

    let expected = expected.as_array().unwrap();
    assert_eq!(
        groups.len(),
        expected.len(),
        "falcon should find the same number of channel groups asammdf does"
    );

    for (got, want) in groups.iter().zip(expected) {
        assert_eq!(
            got.record_size as u64,
            want["record_size"].as_u64().unwrap(),
            "record size should match asammdf"
        );
        assert_eq!(
            got.cycle_count as u64,
            want["cycles"].as_u64().unwrap(),
            "cycle count should match asammdf"
        );
        assert_eq!(
            got.channels.len() as u64,
            want["channels"].as_u64().unwrap(),
            "channel count should match asammdf"
        );
    }
}

#[test]
fn every_group_has_exactly_one_time_channel() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (path, _, _) = generate("3.30", dir.path());
    let file = Mdf3File::open(&path).expect("falcon should open it");

    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            let masters = cg.channels.iter().filter(|c| c.is_time()).count();
            assert_eq!(
                masters, 1,
                "a v3 channel group carries exactly one time channel"
            );
        }
    }
}

#[test]
fn an_mdf4_file_is_refused_by_the_v3_reader() {
    // The two readers must not accept each other's files: handing a v4 file to
    // the v3 reader and getting a result would give the caller the wrong one.
    let dir = tempfile::tempdir().expect("a temp dir");
    let python = python_or_skip();
    let path = dir.path().join("v4.mf4");
    let script = format!(
        r#"
import numpy as np
from asammdf import MDF, Signal
t = np.arange(0.0, 1.0, 0.1)
m = MDF(version="4.10")
m.append([Signal(samples=t, timestamps=t, name="X", unit="")])
m.save(r"{path}", overwrite=True)
m.close()
"#,
        path = path.display()
    );
    let out = Command::new(&python).arg("-c").arg(&script).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let err = match Mdf3File::open(&path) {
        Ok(_) => panic!("the v3 reader must refuse a v4 file"),
        Err(e) => e,
    };
    let text = err.to_string();
    assert!(
        text.contains("nsupported") || text.contains("ignature") || text.contains("ersion"),
        "the refusal should say why, got: {text}"
    );
}

#[test]
fn a_truncated_file_is_an_error_rather_than_a_panic() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (path, _, _) = generate("3.30", dir.path());
    let whole = std::fs::read(&path).expect("reading the generated file");

    // Truncating anywhere must produce an error, never a panic and never a
    // half-described file presented as whole.
    for cut in [1usize, 32, 63, 64, 100, 164, 200, whole.len() / 2] {
        if cut >= whole.len() {
            continue;
        }
        let cut_path = dir.path().join(format!("cut_{cut}.mdf"));
        std::fs::write(&cut_path, &whole[..cut]).expect("writing the truncated file");
        let result = Mdf3File::open(&cut_path);
        assert!(
            result.is_err(),
            "a file truncated to {cut} bytes should be refused, not read"
        );
    }
}

#[test]
fn a_corrupted_block_identifier_is_an_error_rather_than_a_panic() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (path, _, _) = generate("3.30", dir.path());
    let mut bytes = std::fs::read(&path).expect("reading the generated file");

    // The header block sits directly after the 64-byte identification block.
    bytes[64] = b'X';
    bytes[65] = b'X';
    let bad = dir.path().join("bad_hd.mdf");
    std::fs::write(&bad, &bytes).expect("writing the corrupted file");

    let err = match Mdf3File::open(&bad) {
        Ok(_) => panic!("a corrupted header must be refused"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("HD") || err.to_string().to_lowercase().contains("block"),
        "the refusal should name the block, got: {err}"
    );
}
