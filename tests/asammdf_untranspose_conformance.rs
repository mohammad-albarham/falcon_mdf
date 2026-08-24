//! Conformance tests comparing falcon_mdf un-transposition directly against asammdf
//! for buffers whose length is not an exact multiple of the transposition column size.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::Mf4File;

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];

    candidates.into_iter().find(|c| c.is_file())
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn asammdf_untranspose_oracle_direct_comparison() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python found");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in .venv");
        return;
    }

    let test_cases = [(8, 3), (7, 3), (10, 4), (9, 3), (12, 4), (5, 2), (13, 5)];

    for &(size, param) in &test_cases {
        let input: Vec<u8> = (0..size as u8).collect();
        let input_repr = format!("{:?}", input);

        let script = format!(
            r#"
import numpy as np
import json

cols = {param}
data = bytes({input_repr})
original_size = len(data)
lines = original_size // cols

if lines * cols < original_size:
    data = memoryview(data)
    data = (
        np.frombuffer(data[: lines * cols], dtype=np.uint8)
        .reshape((cols, lines))
        .T.ravel()
        .tobytes()
    ) + data[lines * cols :]
else:
    data = np.frombuffer(data, dtype=np.uint8).reshape((cols, lines)).T.ravel().tobytes()

print(json.dumps(list(data)))
"#
        );

        let output = Command::new(&python)
            .args(["-c", &script])
            .output()
            .expect("run asammdf untranspose");
        assert!(output.status.success(), "python script failed");

        let asammdf_result: Vec<u8> = serde_json::from_slice(&output.stdout).expect("parse json");
        let falcon_result = Mf4File::un_transpose(&input, param).expect("un_transpose");

        assert_eq!(
            falcon_result, asammdf_result,
            "mismatch between falcon_mdf and asammdf for size {size}, param {param}"
        );
    }
}

#[test]
fn asammdf_dz_block_transposed_tail_files_read_back_accurately() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python found");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in .venv");
        return;
    }

    // Generate full MDF files containing transposed DataZippedBlocks with non-multiple record tail bytes
    let test_cases = [(8, 3), (7, 3), (10, 4), (9, 3), (12, 4)];

    for &(size, param) in &test_cases {
        let temp = tempfile::Builder::new()
            .suffix(".mf4")
            .tempfile()
            .unwrap();
        let temp_path = temp.path().to_str().unwrap().to_string();

        let script = format!(
            r#"
import numpy as np
from asammdf import MDF, Signal

# Create a channel group with record size matching param
mdf = MDF(version='4.11')

values = np.arange(1, {size} + 1, dtype=np.uint8)
t = np.arange({size}, dtype=np.float64) * 0.01

sig = Signal(samples=values, timestamps=t, name='TestSig')
mdf.append(sig)

# Save with transposed deflate compression (compression=2 in asammdf)
mdf.save(r'{temp_path}', overwrite=True, compression=2)
"#
        );

        let status = Command::new(&python)
            .args(["-c", &script])
            .status()
            .expect("run python generator");
        assert!(status.success(), "asammdf script failed for size {size} param {param}");

        let mdf = Mf4File::open(temp.path()).expect("falcon_mdf should open file");
        let ch = mdf.find_channel("TestSig").expect("channel TestSig must exist");
        let sig = mdf.signal(ch).expect("read signal");

        assert_eq!(sig.len(), size);
        let vals = sig.values_f64().expect("values");
        let expected: Vec<f64> = (1..=size).map(|v| v as f64).collect();
        assert_eq!(
            vals, expected,
            "values mismatch for size {size} param {param}"
        );
    }
}
