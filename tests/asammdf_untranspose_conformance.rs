//! Independent transpose oracle and read-back of asammdf-written DZ files.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::Mf4File;

fn venv_python() -> Option<PathBuf> {
    let python = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python");
    if !python.is_file()
        || !Command::new(&python)
            .args(["-c", "import asammdf"])
            .status()
            .is_ok_and(|status| status.success())
    {
        eprintln!("skipping: .venv with asammdf is required");
        return None;
    }
    Some(python)
}

#[test]
fn untranspose_matches_numpy_for_tails_and_cache_tile_boundaries() {
    let Some(python) = venv_python() else {
        return;
    };
    let cases = [
        (8, 3),
        (7, 3),
        (10, 4),
        (9, 3),
        (12, 4),
        (5, 2),
        (13, 5),
        (0, 1),
        (2, 4),
        (16_383, 19),
        (16_384, 19),
        (16_385, 19),
        (65_539, 19),
        (131_075, 65),
        (32_773, 32_768),
    ];
    let script = format!(
        r#"
import json
import numpy as np
results = []
for size, cols in {cases:?}:
    data = bytes((i * 37 + i // 251) % 256 for i in range(size))
    prefix = size // cols * cols
    result = np.frombuffer(data[:prefix], dtype=np.uint8).reshape((cols, size // cols)).T.ravel().tobytes()
    results.append(list(result + data[prefix:]))
print(json.dumps(results))
"#
    );
    let output = Command::new(python).args(["-c", &script]).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let expected: Vec<Vec<u8>> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(expected.len(), cases.len());
    for ((size, cols), expected) in cases.into_iter().zip(expected) {
        let input: Vec<u8> = (0..size)
            .map(|i| ((i * 37 + i / 251) % 256) as u8)
            .collect();
        assert_eq!(Mf4File::un_transpose(&input, cols).unwrap(), expected);
    }
}

#[test]
fn asammdf_transposed_deflate_files_read_back_accurately() {
    let Some(python) = venv_python() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let script = r#"
import sys
from pathlib import Path
import numpy as np
from asammdf import MDF, Signal
for size in (7, 8, 9, 10, 12, 20000):
    values = np.arange(size, dtype=np.uint32)
    times = np.arange(size, dtype=np.float64) * 0.01
    with MDF(version='4.11') as mdf:
        mdf.append(Signal(samples=values, timestamps=times, name='TestSig'))
        mdf.save(Path(sys.argv[1]) / f'{size}.mf4', overwrite=True, compression=2)
"#;
    let output = Command::new(python)
        .args(["-c", script])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    for size in [7, 8, 9, 10, 12, 20_000] {
        let file = Mf4File::open(directory.path().join(format!("{size}.mf4"))).unwrap();
        let signal = file.signal(file.find_channel("TestSig").unwrap()).unwrap();
        assert_eq!(signal.len(), size);
        let expected: Vec<f64> = (0..size).map(|i| i as f64).collect();
        assert_eq!(signal.values_f64().unwrap(), expected);
    }
}
