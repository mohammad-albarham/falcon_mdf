//! Tests for fixed-shape ##CA array channel writing.
//!
//! Verifies that:
//! 1. Non-square 2D array channels (e.g. 2x3) round-trip accurately in falcon_mdf,
//!    preserving exact element values and reported shape.
//! 2. The on-disk bytes of ##CA blocks match hand-written expected binary sequences.
//! 3. asammdf reads the generated files and decodes the exact sample values and shapes.
//! 4. 1D, 2D, and 3D arrays round-trip accurately.
//! 5. Compression and from_file round-tripping preserve array channels.
//! 6. Array channels with invalidation bits preserve validity masks.
//! 7. Invalid shapes (empty, zero dimensions, element count mismatch) are refused.

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::{Mf4File, Mf4Writer, SignalValues};

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../falcon_mdf/.venv/bin/python"),
        PathBuf::from("/Users/pain/Desktop/hoppy_projects/falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn asammdf_available(python: &Path) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn array_raw_ca_block_bytes_match_hand_written_expected_sequence() {
    let times = vec![0.0, 1.0];
    // 2 samples, 6 elements per sample (2x3 array)
    let values = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, // sample 0
        7.0, 8.0, 9.0, 10.0, 11.0, 12.0, // sample 1
    ];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Matrix2x3",
            "V",
            &[2, 3],
            SignalValues::Array {
                values,
                elements_per_sample: 6,
            },
        )
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();

    // Verify file starts with MDF 4.11 header
    assert_eq!(&bytes[0..8], b"MDF     ");
    assert_eq!(&bytes[8..16], b"4.11    ");

    // Hand-written expected ##CA block binary sequence:
    // Block header:
    //   b"##CA" (4 bytes)
    //   reserved [0u8; 4] (4 bytes)
    //   length: 48 (base) + 2 * 8 (2 dims) = 64 (8 bytes LE: 0x40)
    //   link_count: 1 (8 bytes LE)
    // Link section:
    //   ca_composition: 0 (8 bytes LE) - parent describes element
    // Data section:
    //   ca_type: 0 (Array) (1 byte)
    //   ca_storage: 0 (CnTemplate) (1 byte)
    //   ca_ndim: 2 (2 bytes LE)
    //   flags: 0 (4 bytes LE)
    //   ca_byte_offset_base: 8 (4 bytes LE, i32) - 8 bytes per f64
    //   ca_invalidation_bit_base: 0 (4 bytes LE)
    //   ca_dim_size[0]: 2 (8 bytes LE)
    //   ca_dim_size[1]: 3 (8 bytes LE)
    let expected_ca_block: &[u8] = &[
        b'#', b'#', b'C', b'A', 0, 0, 0, 0, // header id & reserved (8 bytes)
        64, 0, 0, 0, 0, 0, 0, 0, // length = 64 (8 bytes)
        1, 0, 0, 0, 0, 0, 0, 0, // link_count = 1 (8 bytes)
        0, 0, 0, 0, 0, 0, 0, 0, // ca_composition = 0 (8 bytes)
        0, // ca_type = 0 (Array)
        0, // ca_storage = 0 (CnTemplate)
        2, 0, // ca_ndim = 2
        0, 0, 0, 0, // flags = 0
        8, 0, 0, 0, // ca_byte_offset_base = 8
        0, 0, 0, 0, // ca_invalidation_bit_base = 0
        2, 0, 0, 0, 0, 0, 0, 0, // ca_dim_size[0] = 2
        3, 0, 0, 0, 0, 0, 0, 0, // ca_dim_size[1] = 3
    ];

    let ca_pos = bytes
        .windows(4)
        .position(|w| w == b"##CA")
        .expect("##CA block must be present in written bytes");

    assert_eq!(
        &bytes[ca_pos..ca_pos + expected_ca_block.len()],
        expected_ca_block,
        "written ##CA block bytes do not match expected sequence"
    );
}

#[test]
fn array_raw_ca_block_1d_bytes_match_hand_written_expected_sequence() {
    let times = vec![0.0, 1.0];
    let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Vec4",
            "A",
            &[4],
            SignalValues::Array {
                values,
                elements_per_sample: 4,
            },
        )
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();

    // Hand-written expected 1D ##CA block binary sequence: length = 48 + 8 = 56
    let expected_ca_1d: &[u8] = &[
        b'#', b'#', b'C', b'A', 0, 0, 0, 0, // header id & reserved
        56, 0, 0, 0, 0, 0, 0, 0, // length = 56
        1, 0, 0, 0, 0, 0, 0, 0, // link_count = 1
        0, 0, 0, 0, 0, 0, 0, 0, // ca_composition = 0
        0, // ca_type = 0
        0, // ca_storage = 0
        1, 0, // ca_ndim = 1
        0, 0, 0, 0, // flags = 0
        8, 0, 0, 0, // ca_byte_offset_base = 8
        0, 0, 0, 0, // ca_invalidation_bit_base = 0
        4, 0, 0, 0, 0, 0, 0, 0, // ca_dim_size[0] = 4
    ];

    let ca_pos = bytes
        .windows(4)
        .position(|w| w == b"##CA")
        .expect("##CA block must be present in written bytes");

    assert_eq!(
        &bytes[ca_pos..ca_pos + expected_ca_1d.len()],
        expected_ca_1d,
        "written 1D ##CA block bytes do not match expected sequence"
    );
}

#[test]
fn non_square_2d_array_roundtrip_falcon() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("non_square_2d.mf4");

    let times = vec![0.0, 0.1, 0.2];
    // Shape: 2 rows x 3 columns = 6 elements per sample
    // Non-square shape ensures row vs column major or transposed reads fail.
    let values = vec![
        10.0, 20.0, 30.0, 40.0, 50.0, 60.0, // sample 0: [[10, 20, 30], [40, 50, 60]]
        11.0, 21.0, 31.0, 41.0, 51.0, 61.0, // sample 1: [[11, 21, 31], [41, 51, 61]]
        12.0, 22.0, 32.0, 42.0, 52.0, 62.0, // sample 2: [[12, 22, 32], [42, 52, 62]]
    ];

    let mut writer = Mf4Writer::with_start_time_ns(1_000_000_000);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Matrix2x3",
            "Nm",
            &[2, 3],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 6,
            },
        )
        .unwrap();

    writer.write_to_file(&path).unwrap();

    let file = Mf4File::open(&path).unwrap();
    let ch = file
        .channels()
        .find(|c| c.name == "Matrix2x3")
        .expect("Matrix2x3 channel not found");

    assert!(ch.is_array(), "channel must be reported as array");
    assert_eq!(
        ch.array_shape(),
        Some(&[2, 3][..]),
        "channel array shape must be [2, 3]"
    );
    assert_eq!(ch.unit, "Nm");

    let master_ch = file.channels().find(|c| c.is_master()).unwrap();
    let master_sig = file.signal(master_ch).unwrap();
    assert_eq!(master_sig.values_f64().unwrap(), vec![0.0, 0.1, 0.2]);

    let signal = file.signal(ch).unwrap();
    assert_eq!(signal.len(), 3);

    match signal.values().unwrap() {
        SignalValues::Array {
            values: read_values,
            elements_per_sample,
        } => {
            assert_eq!(elements_per_sample, 6);
            assert_eq!(&read_values, &values);
        }
        other => panic!("expected SignalValues::Array, got {:?}", other.kind()),
    }
}

#[test]
fn array_cross_check_with_asammdf() {
    let python = match venv_python() {
        Some(p) if asammdf_available(&p) => p,
        _ => {
            eprintln!("skipping asammdf cross-check: asammdf not available");
            return;
        }
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("array_test.mf4");

    let times = vec![0.0, 0.5, 1.0];
    let matrix_vals = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, // sample 0
        10.0, 20.0, 30.0, 40.0, 50.0, 60.0, // sample 1
        100.0, 200.0, 300.0, 400.0, 500.0, 600.0, // sample 2
    ];
    let vec_vals = vec![
        0.1, 0.2, 0.3, 0.4, // sample 0
        1.1, 1.2, 1.3, 1.4, // sample 1
        2.1, 2.2, 2.3, 2.4, // sample 2
    ];

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Matrix2x3",
            "degC",
            &[2, 3],
            SignalValues::Array {
                values: matrix_vals.clone(),
                elements_per_sample: 6,
            },
        )
        .unwrap();
    group
        .add_channel_array(
            "Vec4",
            "bar",
            &[4],
            SignalValues::Array {
                values: vec_vals.clone(),
                elements_per_sample: 4,
            },
        )
        .unwrap();

    writer.write_to_file(&path).unwrap();

    // Verify with asammdf
    let py_script = format!(
        r#"
import sys
import numpy as np
from asammdf import MDF

mdf = MDF(r"{}")
assert len(mdf.groups) == 1, f"expected 1 group, got {{len(mdf.groups)}}"

# Check Matrix2x3
sig_m = mdf.get("Matrix2x3")
samples_m = sig_m.samples["Matrix2x3"] if sig_m.samples.dtype.names else sig_m.samples
assert samples_m.shape == (3, 2, 3), f"expected shape (3, 2, 3), got {{samples_m.shape}}"
expected_m = np.array({:?}, dtype=np.float64).reshape((3, 2, 3))
np.testing.assert_array_almost_equal(samples_m, expected_m)
np.testing.assert_array_almost_equal(sig_m.timestamps, [0.0, 0.5, 1.0])

# Check Vec4
sig_v = mdf.get("Vec4")
samples_v = sig_v.samples["Vec4"] if sig_v.samples.dtype.names else sig_v.samples
assert samples_v.shape == (3, 4), f"expected shape (3, 4), got {{samples_v.shape}}"
expected_v = np.array({:?}, dtype=np.float64).reshape((3, 4))
np.testing.assert_array_almost_equal(samples_v, expected_v)
np.testing.assert_array_almost_equal(sig_v.timestamps, [0.0, 0.5, 1.0])

print("ASAMMDF_OK")
"#,
        path.display(),
        matrix_vals,
        vec_vals,
    );

    let output = Command::new(&python)
        .args(["-c", &py_script])
        .output()
        .expect("failed to execute python script");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("ASAMMDF_OK"),
        "asammdf verification failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn array_3d_roundtrip_falcon_and_asammdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("array_3d.mf4");

    let times = vec![0.0, 1.0];
    // 3D array: 2 x 3 x 4 = 24 elements per sample
    let sample0: Vec<f64> = (0..24).map(|i| i as f64 * 1.5).collect();
    let sample1: Vec<f64> = (0..24).map(|i| (i + 100) as f64 * 2.0).collect();
    let mut values = sample0;
    values.extend(sample1);

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Tensor2x3x4",
            "unit",
            &[2, 3, 4],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 24,
            },
        )
        .unwrap();

    writer.write_to_file(&path).unwrap();

    // Verify falcon reading
    let file = Mf4File::open(&path).unwrap();
    let ch = file
        .channels()
        .into_iter()
        .find(|c| c.name == "Tensor2x3x4")
        .unwrap();
    assert_eq!(ch.array_shape(), Some(&[2, 3, 4][..]));

    let sig = file.signal(ch).unwrap();
    match sig.values().unwrap() {
        SignalValues::Array {
            values: read_vals,
            elements_per_sample,
        } => {
            assert_eq!(elements_per_sample, 24);
            assert_eq!(&read_vals, &values);
        }
        other => panic!("expected SignalValues::Array, got {:?}", other.kind()),
    }

    // Verify with asammdf if present
    if let Some(python) = venv_python() {
        if asammdf_available(&python) {
            let py_script = format!(
                r#"
import numpy as np
from asammdf import MDF

mdf = MDF(r"{}")
sig = mdf.get("Tensor2x3x4")
samples = sig.samples["Tensor2x3x4"] if sig.samples.dtype.names else sig.samples
assert samples.shape == (2, 2, 3, 4), f"expected shape (2, 2, 3, 4), got {{samples.shape}}"
expected = np.array({:?}, dtype=np.float64).reshape((2, 2, 3, 4))
np.testing.assert_array_almost_equal(samples, expected)
print("3D_OK")
"#,
                path.display(),
                values,
            );

            let output = Command::new(&python)
                .args(["-c", &py_script])
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("3D_OK"),
                "asammdf 3d check failed:\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn array_channel_in_compressed_mf4() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("array_compressed.mf4");

    let times: Vec<f64> = (0..50).map(|i| i as f64 * 0.01).collect();
    let mut values = Vec::with_capacity(50 * 6);
    for t in 0..50 {
        for e in 0..6 {
            values.push(t as f64 * 10.0 + e as f64);
        }
    }

    let mut writer = Mf4Writer::new();
    writer.set_compression(true);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "MatrixCompressed",
            "kPa",
            &[2, 3],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 6,
            },
        )
        .unwrap();

    writer.write_to_file(&path).unwrap();

    // Verify falcon reading
    let file = Mf4File::open(&path).unwrap();
    let ch = file
        .channels()
        .into_iter()
        .find(|c| c.name == "MatrixCompressed")
        .unwrap();
    assert_eq!(ch.array_shape(), Some(&[2, 3][..]));
    let sig = file.signal(ch).unwrap();
    match sig.values().unwrap() {
        SignalValues::Array {
            values: read_vals,
            elements_per_sample,
        } => {
            assert_eq!(elements_per_sample, 6);
            assert_eq!(&read_vals, &values);
        }
        other => panic!("expected SignalValues::Array, got {:?}", other.kind()),
    }

    // Verify asammdf
    if let Some(python) = venv_python() {
        if asammdf_available(&python) {
            let py_script = format!(
                r#"
import numpy as np
from asammdf import MDF

mdf = MDF(r"{}")
sig = mdf.get("MatrixCompressed")
samples = sig.samples["MatrixCompressed"] if sig.samples.dtype.names else sig.samples
assert samples.shape == (50, 2, 3), f"expected shape (50, 2, 3), got {{samples.shape}}"
expected = np.array({:?}, dtype=np.float64).reshape((50, 2, 3))
np.testing.assert_array_almost_equal(samples, expected)
print("COMPRESSED_OK")
"#,
                path.display(),
                values,
            );

            let output = Command::new(&python)
                .args(["-c", &py_script])
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("COMPRESSED_OK"),
                "asammdf compressed check failed:\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn array_channel_roundtrip_via_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path1 = dir.path().join("array_source.mf4");
    let path2 = dir.path().join("array_roundtrip.mf4");

    let times = vec![0.0, 1.0, 2.0];
    let values = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, // sample 0
        7.0, 8.0, 9.0, 10.0, 11.0, 12.0, // sample 1
        13.0, 14.0, 15.0, 16.0, 17.0, 18.0, // sample 2
    ];

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array(
            "Matrix2x3",
            "rpm",
            &[2, 3],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 6,
            },
        )
        .unwrap();
    writer.write_to_file(&path1).unwrap();

    // Read back and use from_file
    let file1 = Mf4File::open(&path1).unwrap();
    let writer2 = Mf4Writer::from_file(&file1).unwrap();
    writer2.write_to_file(&path2).unwrap();

    let file2 = Mf4File::open(&path2).unwrap();
    let ch = file2
        .channels()
        .into_iter()
        .find(|c| c.name == "Matrix2x3")
        .unwrap();
    assert_eq!(ch.array_shape(), Some(&[2, 3][..]));
    assert_eq!(ch.unit, "rpm");

    let sig = file2.signal(ch).unwrap();
    match sig.values().unwrap() {
        SignalValues::Array {
            values: read_vals,
            elements_per_sample,
        } => {
            assert_eq!(elements_per_sample, 6);
            assert_eq!(&read_vals, &values);
        }
        other => panic!("expected SignalValues::Array, got {:?}", other.kind()),
    }
}

#[test]
fn array_channel_with_invalidation_bits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("array_inval.mf4");

    let times = vec![0.0, 1.0, 2.0, 3.0];
    let values = vec![
        1.0, 2.0, 3.0, 4.0, // sample 0 (valid)
        5.0, 6.0, 7.0, 8.0, // sample 1 (invalid)
        9.0, 10.0, 11.0, 12.0, // sample 2 (valid)
        13.0, 14.0, 15.0, 16.0, // sample 3 (invalid)
    ];
    let valid = vec![true, false, true, false];

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array_with(
            "ArrayWithInval",
            "V",
            &[2, 2],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 4,
            },
            Some(&valid),
            None,
        )
        .unwrap();

    writer.write_to_file(&path).unwrap();

    let file = Mf4File::open(&path).unwrap();
    let ch = file
        .channels()
        .into_iter()
        .find(|c| c.name == "ArrayWithInval")
        .unwrap();
    assert_eq!(ch.array_shape(), Some(&[2, 2][..]));

    let sig = file.signal(ch).unwrap();
    assert_eq!(sig.validity().as_deref(), Some(&valid[..]));

    match sig.values().unwrap() {
        SignalValues::Array {
            values: read_vals,
            elements_per_sample,
        } => {
            assert_eq!(elements_per_sample, 4);
            assert_eq!(&read_vals, &values);
        }
        other => panic!("expected SignalValues::Array, got {:?}", other.kind()),
    }
}

#[test]
fn array_invalid_shapes_are_refused() {
    let times = vec![0.0, 1.0];
    let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times).unwrap();

    // Empty shape
    let err = group
        .add_channel_array(
            "EmptyShape",
            "",
            &[],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 3,
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("invalid shape"));

    // Zero dimension
    let err = group
        .add_channel_array(
            "ZeroDim",
            "",
            &[2, 0],
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 3,
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("invalid shape"));

    // Mismatched dimension product
    let err = group
        .add_channel_array(
            "MismatchProduct",
            "",
            &[2, 3], // product = 6
            SignalValues::Array {
                values: values.clone(),
                elements_per_sample: 3, // values claims 3 per sample
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("implies 6 elements per sample"));

    // Non-array SignalValues
    let err = group
        .add_channel_array("NotArray", "", &[2, 3], SignalValues::F64(values))
        .unwrap_err();
    assert!(err.to_string().contains("cannot be written as an array"));
}

#[test]
fn scaled_array_channel_roundtrip_does_not_double_convert() {
    use falcon_mdf::write::RewriteMode;
    use falcon_mdf::{Conversion, Mf4Error, TableEntry};

    let times = vec![0.0, 1.0];
    let raw_vals = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let linear = Conversion::Linear {
        offset: 10.0,
        factor: 2.0,
    };
    let expected_linear: Vec<f64> = raw_vals.iter().map(|&v| 10.0 + 2.0 * v).collect();

    let table = Conversion::TableInterpolated {
        keys: vec![1.0, 5.0, 8.0],
        values: vec![100.0, 500.0, 800.0],
    };
    let expected_table: Vec<f64> = raw_vals.iter().map(|&v| v * 100.0).collect();

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_array_with(
            "ScaledMat",
            "N",
            &[2, 2],
            SignalValues::Array {
                values: raw_vals.clone(),
                elements_per_sample: 4,
            },
            None,
            Some(linear),
        )
        .unwrap();
    group
        .add_channel_array_with(
            "TableMat",
            "Pa",
            &[2, 2],
            SignalValues::Array {
                values: raw_vals,
                elements_per_sample: 4,
            },
            None,
            Some(table),
        )
        .unwrap();

    let file1_tmp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(file1_tmp.path()).unwrap();
    let file1 = Mf4File::open(file1_tmp.path()).unwrap();

    // Rewrite strictly: numeric array conversions are folded into physical f64
    // values without attaching a duplicate ##CC block.
    let (writer2, report) = Mf4Writer::from_file_with_report(&file1, RewriteMode::Strict).unwrap();
    assert!(report.is_lossless(), "issues: {:?}", report.issues);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.name == "ScaledMat" && n.reason.contains("folded")),
        "expected informational note for ScaledMat, got {:?}",
        report.notes
    );

    let file2_tmp = tempfile::NamedTempFile::new().unwrap();
    writer2.write_to_file(file2_tmp.path()).unwrap();
    let file2 = Mf4File::open(file2_tmp.path()).unwrap();

    for (name, expected) in [("ScaledMat", expected_linear), ("TableMat", expected_table)] {
        let ch = file2.find_channel(name).unwrap();
        assert_eq!(ch.array_shape, Some(vec![2, 2]));
        assert!(
            ch.conversion.is_identity(),
            "{name} should not retain a ##CC block after folding conversion"
        );
        match file2.signal(ch).unwrap().values().unwrap() {
            SignalValues::Array {
                values,
                elements_per_sample,
            } => {
                assert_eq!(elements_per_sample, 4);
                assert_eq!(values, expected, "{name} was double-converted or altered");
            }
            other => panic!("expected Array for {name}, got {other:?}"),
        }
    }

    // An array channel with a non-numeric (text) conversion cannot preserve its
    // labels as f64 array elements and must be rejected in Strict mode.
    let mut text_writer = Mf4Writer::with_start_time_ns(0);
    let tg = text_writer.add_group(&[0.0]).unwrap();
    tg.add_channel_array_with(
        "TextMat",
        "",
        &[2],
        SignalValues::Array {
            values: vec![1.0, 2.0],
            elements_per_sample: 2,
        },
        None,
        Some(Conversion::ValueToText {
            keys: vec![1.0, 2.0],
            entries: vec![
                TableEntry::Text("ONE".into()),
                TableEntry::Text("TWO".into()),
            ],
            default: None,
        }),
    )
    .unwrap();
    let text_tmp = tempfile::NamedTempFile::new().unwrap();
    text_writer.write_to_file(text_tmp.path()).unwrap();
    let text_file = Mf4File::open(text_tmp.path()).unwrap();
    let err = Mf4Writer::from_file_with_report(&text_file, RewriteMode::Strict).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
}

#[test]
fn set_array_shape_none_is_refused_for_array_channels() {
    use falcon_mdf::Mf4Error;

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.0, 1.0]).unwrap();
    group
        .add_channel_array(
            "Vec2",
            "m",
            &[2],
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0],
                elements_per_sample: 2,
            },
        )
        .unwrap();

    let ch = &mut group.channels_mut()[0];
    let err = ch.set_array_shape(None).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
    assert_eq!(ch.array_shape(), Some(&[2][..]));

    // Replacing with scalar values clears array_shape cleanly.
    ch.set_values(SignalValues::F64(vec![10.0, 20.0])).unwrap();
    assert_eq!(ch.array_shape(), None);

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &bytes).unwrap();
    let file = Mf4File::open(tmp.path()).unwrap();
    let read_ch = file.find_channel("Vec2").unwrap();
    assert_eq!(read_ch.array_shape, None);
    assert_eq!(
        file.signal(read_ch).unwrap().values_f64().unwrap(),
        vec![10.0, 20.0]
    );
}
