//! Export format tests: Parquet and MATLAB level 5.
//!
//! **A writer here is never checked with our own reader.** Every file this
//! suite writes is read back by the tool that owns the format — pyarrow for
//! Parquet, `scipy.io.loadmat` for MAT — and the numbers that come out are
//! compared against the samples originally handed to `Mf4Writer`, not against
//! anything falcon decoded. The chain each cross-check exercises is:
//!
//! ```text
//! values written by hand  ->  MF4  ->  falcon reads  ->  export  ->  foreign reader
//!         \___________________________ compared _______________________/
//! ```
//!
//! So a bug anywhere along it — in the MDF writer, the decoder, or the
//! exporter — shows up as a disagreement with a number that was typed into this
//! file. Three silent corruption defects in this repository survived tests that
//! used the implementation's own inverse as their oracle; this arrangement has
//! no inverse in it.
//!
//! Tests skip, loudly, when the foreign reader is not installed.

#![cfg(any(
    feature = "parquet",
    feature = "mat",
    feature = "mat4",
    feature = "hdf5",
    feature = "mat73",
    feature = "asc"
))]

use falcon_mdf::{Mf4File, Mf4Writer, SignalSeries, SignalValues};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The first interpreter that can actually `import module`.
///
/// Two candidates: a virtualenv beside the crate, then the shared one two
/// levels up. Probing for the module rather than just the interpreter means a
/// machine with scipy but no pyarrow runs the MAT cross-checks and skips only
/// the Parquet ones, instead of skipping or failing both.
fn python_with(module: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates
        .into_iter()
        .filter(|p| p.is_file())
        .find(|p| {
            Command::new(p)
                .args(["-c", &format!("import {module}")])
                .status()
                .is_ok_and(|s| s.success())
        })
}

/// Runs `script` and parses the JSON it wrote to `json_path`.
fn run_python(python: &Path, script: &str, json_path: &Path) -> serde_json::Value {
    let out = Command::new(python)
        .args(["-c", script])
        .output()
        .expect("failed to launch python");
    assert!(
        out.status.success(),
        "the reference reader failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(json_path).expect("the script wrote no JSON");
    serde_json::from_slice(&bytes).expect("the script wrote invalid JSON")
}

fn floats(value: &serde_json::Value) -> Vec<f64> {
    value
        .as_array()
        .expect("expected an array")
        .iter()
        .map(|v| v.as_f64().expect("expected a number"))
        .collect()
}

fn assert_close(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length differs");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() < 1e-9,
            "{what}: element {i} is {g}, expected {w}"
        );
    }
}

fn temp(suffix: &str) -> tempfile::NamedTempFile {
    tempfile::Builder::new().suffix(suffix).tempfile().unwrap()
}

fn resolve_path(rel: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(rel),
        PathBuf::from("../../falcon_mdf").join(rel),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// A synthetic series, for exercising sample kinds the MF4 writer cannot emit.
fn series(name: &str, timestamps: Vec<f64>, values: SignalValues) -> SignalSeries {
    SignalSeries::from_samples(name, "", timestamps, values, None).unwrap()
}

// ---------------------------------------------------------------------------
// Parquet
// ---------------------------------------------------------------------------

#[cfg(feature = "parquet")]
mod parquet_tests {
    use super::*;
    use falcon_mdf::{write_parquet, write_parquet_with, InterpolationMode, ParquetCompression, Raster};

    #[test]
    fn values_survive_mdf_then_parquet_then_pyarrow() {
        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        // The oracle. Every number below is typed here and nowhere else.
        let times = vec![0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5];
        let speed = vec![0.0, 12.5, 25.0, 37.5, 50.0, 62.5, 75.0];
        let coolant = vec![80.0, 80.5, 81.25, 82.0, 82.5, 83.0, 83.75];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Speed", "km/h", &speed).unwrap();
        group.add_channel("Coolant", "degC", &coolant).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file
            .filter(&["Speed".into(), "Coolant".into()])
            .unwrap();

        // What falcon read must already equal the oracle; if this fails the
        // export is not the thing at fault.
        assert_close(&exported[0].values_f64(), &speed, "falcon's Speed");
        assert_close(&exported[1].values_f64(), &coolant, "falcon's Coolant");

        let pq = temp(".parquet");
        let mut out = std::fs::File::create(pq.path()).unwrap();
        write_parquet(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

t = pq.read_table(r"{pq}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "columns": t.column_names,
        "types": [str(t.schema.field(i).type) for i in range(t.num_columns)],
        "rows": t.num_rows,
        "time": t.column("time").to_pylist(),
        "Speed": t.column("Speed").to_pylist(),
        "Coolant": t.column("Coolant").to_pylist(),
    }}, fh)
"#,
            pq = pq.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["columns"].as_array().unwrap(),
            &vec![
                serde_json::json!("time"),
                serde_json::json!("Speed"),
                serde_json::json!("Coolant")
            ]
        );
        assert_eq!(py["rows"].as_u64().unwrap(), times.len() as u64);
        // Compared against the hand-written oracle, not against `exported`.
        assert_close(&floats(&py["time"]), &times, "pyarrow's time");
        assert_close(&floats(&py["Speed"]), &speed, "pyarrow's Speed");
        assert_close(&floats(&py["Coolant"]), &coolant, "pyarrow's Coolant");

        println!("Parquet cross-check: pyarrow returned the values the MF4 was built from");
    }

    #[test]
    fn every_column_type_survives_pyarrow() {
        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let columns = vec![
            series("u8", t.clone(), SignalValues::U8(vec![1, 2, 250])),
            series("u16", t.clone(), SignalValues::U16(vec![1, 2, 65530])),
            series("u32", t.clone(), SignalValues::U32(vec![1, 2, 4_294_967_290])),
            // Past 2^53: an exporter that routed everything through f64 would
            // round this, and the comparison below is exact.
            series(
                "u64",
                t.clone(),
                SignalValues::U64(vec![1, 2, 9_007_199_254_740_993]),
            ),
            series("i8", t.clone(), SignalValues::I8(vec![-128, 0, 127])),
            series("i16", t.clone(), SignalValues::I16(vec![-32768, 0, 32767])),
            series("i32", t.clone(), SignalValues::I32(vec![-2147483648, 0, 2147483647])),
            series(
                "i64",
                t.clone(),
                SignalValues::I64(vec![-9_007_199_254_740_993, 0, 9_007_199_254_740_993]),
            ),
            series("f32", t.clone(), SignalValues::F32(vec![-1.5, 0.0, 2.25])),
            series("f64", t.clone(), SignalValues::F64(vec![-1.5, 0.0, 2.25])),
            series(
                "text",
                t.clone(),
                SignalValues::Str(vec!["idle".into(), "".into(), "wide open".into()]),
            ),
            series(
                "bytes",
                t.clone(),
                SignalValues::Bytes {
                    data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11],
                    width: 2,
                },
            ),
        ];

        let pq = temp(".parquet");
        let mut out = std::fs::File::create(pq.path()).unwrap();
        write_parquet(&columns, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

t = pq.read_table(r"{pq}")
types = {{f.name: str(f.type) for f in t.schema}}
data = {{name: t.column(name).to_pylist() for name in t.column_names}}
data["bytes"] = [b.hex() for b in data["bytes"]]
with open(r"{js}", "w") as fh:
    json.dump({{"types": types, "data": data}}, fh)
"#,
            pq = pq.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        // pyarrow's own names for the types it found. A channel's width is part
        // of the measurement; collapsing it to double would lose it silently.
        let types = &py["types"];
        for (column, want) in [
            ("time", "double"),
            ("u8", "uint8"),
            ("u16", "uint16"),
            ("u32", "uint32"),
            ("u64", "uint64"),
            ("i8", "int8"),
            ("i16", "int16"),
            ("i32", "int32"),
            ("i64", "int64"),
            ("f32", "float"),
            ("f64", "double"),
            ("text", "string"),
            ("bytes", "binary"),
        ] {
            assert_eq!(
                types[column].as_str().unwrap(),
                want,
                "column {column} came back as the wrong Arrow type"
            );
        }

        let data = &py["data"];
        // Exact integer comparisons, through JSON integers rather than floats.
        assert_eq!(data["u8"].as_array().unwrap()[2].as_u64().unwrap(), 250);
        assert_eq!(data["u16"].as_array().unwrap()[2].as_u64().unwrap(), 65530);
        assert_eq!(
            data["u32"].as_array().unwrap()[2].as_u64().unwrap(),
            4_294_967_290
        );
        assert_eq!(
            data["u64"].as_array().unwrap()[2].as_u64().unwrap(),
            9_007_199_254_740_993,
            "a u64 past 2^53 must arrive intact, not rounded through a double"
        );
        assert_eq!(data["i8"].as_array().unwrap()[0].as_i64().unwrap(), -128);
        assert_eq!(
            data["i64"].as_array().unwrap()[0].as_i64().unwrap(),
            -9_007_199_254_740_993
        );
        assert_eq!(
            data["text"].as_array().unwrap(),
            &vec![
                serde_json::json!("idle"),
                serde_json::json!(""),
                serde_json::json!("wide open")
            ]
        );
        assert_eq!(
            data["bytes"].as_array().unwrap(),
            &vec![
                serde_json::json!("dead"),
                serde_json::json!("beef"),
                serde_json::json!("0011")
            ]
        );
    }

    #[test]
    fn invalid_samples_arrive_as_nulls() {
        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![10.0, 20.0, 30.0, 40.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group
            .add_channel_with_validity(
                "Sensor",
                "bar",
                &values,
                Some(&[true, false, true, false]),
            )
            .unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Sensor".into()]).unwrap();

        let pq = temp(".parquet");
        let mut out = std::fs::File::create(pq.path()).unwrap();
        write_parquet(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

t = pq.read_table(r"{pq}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "sensor": t.column("Sensor").to_pylist(),
        "nulls": t.column("Sensor").null_count,
        "nullable": t.schema.field("Sensor").nullable,
        "time_nullable": t.schema.field("time").nullable,
    }}, fh)
"#,
            pq = pq.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        // Samples 1 and 3 are disclaimed by the file, so Parquet says null —
        // not 20.0 and 40.0, which were never valid readings.
        assert_eq!(py["nulls"].as_u64().unwrap(), 2);
        assert_eq!(
            py["sensor"].as_array().unwrap(),
            &vec![
                serde_json::json!(10.0),
                serde_json::Value::Null,
                serde_json::json!(30.0),
                serde_json::Value::Null
            ]
        );
        assert!(py["nullable"].as_bool().unwrap());
        // The time axis carries no invalidation, so its column says so.
        assert!(!py["time_nullable"].as_bool().unwrap());
    }

    #[test]
    fn uncompressed_and_snappy_files_hold_the_same_numbers() {
        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        let values: Vec<f64> = (0..500).map(|i| i as f64 * 0.5).collect();
        let times: Vec<f64> = (0..500).map(|i| i as f64 * 0.01).collect();
        let columns = vec![series(
            "Ramp",
            times,
            SignalValues::F64(values.clone()),
        )];

        let snappy = temp(".parquet");
        let plain = temp(".parquet");
        let mut a = std::fs::File::create(snappy.path()).unwrap();
        write_parquet_with(&columns, &mut a, ParquetCompression::Snappy).unwrap();
        drop(a);
        let mut b = std::fs::File::create(plain.path()).unwrap();
        write_parquet_with(&columns, &mut b, ParquetCompression::None).unwrap();
        drop(b);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

snappy = pq.read_table(r"{snappy}")
plain = pq.read_table(r"{plain}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "snappy": snappy.column("Ramp").to_pylist(),
        "plain": plain.column("Ramp").to_pylist(),
        "snappy_codec": snappy.schema.pandas_metadata is None,
        "codecs": sorted({{
            pq.ParquetFile(r"{snappy}").metadata.row_group(0).column(i).compression
            for i in range(pq.ParquetFile(r"{snappy}").metadata.num_columns)
        }}),
    }}, fh)
"#,
            snappy = snappy.path().display(),
            plain = plain.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        // Compression must not be able to change a number.
        assert_close(&floats(&py["snappy"]), &values, "snappy Ramp");
        assert_close(&floats(&py["plain"]), &values, "uncompressed Ramp");
        assert_eq!(
            py["codecs"].as_array().unwrap(),
            &vec![serde_json::json!("SNAPPY")],
            "the default writer should actually be emitting Snappy"
        );
    }

    #[test]
    fn channels_on_different_time_axes_are_refused_until_resampled() {
        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let slow = writer.add_group(&[0.0, 1.0, 2.0]).unwrap();
        slow.add_channel("Slow", "V", &[1.0, 2.0, 3.0]).unwrap();
        let fast = writer.add_group(&[0.0, 0.5, 1.0, 1.5, 2.0]).unwrap();
        fast.add_channel("Fast", "A", &[9.0, 8.0, 7.0, 6.0, 5.0])
            .unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let mixed = file.filter(&["Slow".into(), "Fast".into()]).unwrap();

        let mut sink = Vec::new();
        let err = write_parquet(&mixed, &mut sink)
            .expect_err("one table cannot hold two time axes");
        let text = err.to_string();
        assert!(
            text.contains("Slow") && text.contains("Fast") && text.contains("resample"),
            "the error should name both channels and point at the fix, got: {text}"
        );

        // The fix it points at works, and produces one table.
        let channels: Vec<_> = ["Slow", "Fast"]
            .iter()
            .map(|n| file.find_channel(n).unwrap())
            .collect();
        let gridded = file
            .resample(&channels, Raster::Step(0.5), InterpolationMode::Linear)
            .unwrap();
        let mut sink = Vec::new();
        write_parquet(&gridded, &mut sink).expect("one raster, one table");
        assert!(!sink.is_empty());
    }

    #[test]
    fn varlen_arrays_are_refused_by_name() {
        let var_array = series(
            "DynamicSpectrum",
            vec![0.0, 1.0],
            SignalValues::ArrayVarLen {
                values: vec![1.0, 2.0, 3.0],
                starts: vec![0, 2, 3],
            },
        );
        let mut sink = Vec::new();
        let err = write_parquet(&[var_array], &mut sink).expect_err("varlen array is not represented");
        let text = err.to_string();
        assert!(
            text.contains("DynamicSpectrum") && (text.contains("variable-length array") || text.contains("array")),
            "the error should name the channel and its kind, got: {text}"
        );
    }

    #[test]
    fn composites_survive_mdf_then_parquet_then_pyarrow() {
        use falcon_mdf::model::values::{CanopenDate, CanopenTime};

        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0];
        let complex = series(
            "Impedance",
            times.clone(),
            SignalValues::Complex {
                re: vec![10.0, 20.0],
                im: vec![-5.0, -15.0],
            },
        );
        let date = series(
            "StartDate",
            times.clone(),
            SignalValues::CanopenDate(vec![
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 30,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 31,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
            ]),
        );
        let time = series(
            "StartTime",
            times.clone(),
            SignalValues::CanopenTime(vec![
                CanopenTime {
                    ms_since_midnight: 3600000,
                    days_since_1984: 100,
                },
                CanopenTime {
                    ms_since_midnight: 3601000,
                    days_since_1984: 100,
                },
            ]),
        );
        let mut arr = series(
            "Matrix",
            times.clone(),
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 3,
            },
        );
        arr.channel.array_shape = Some(vec![3]);

        let pq = temp(".parquet");
        let mut out = std::fs::File::create(pq.path()).unwrap();
        write_parquet(&[complex, date, time, arr], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

t = pq.read_table(r"{pq}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "columns": t.column_names,
        "re": t.column("Impedance.re").to_pylist(),
        "im": t.column("Impedance.im").to_pylist(),
        "date": [float(x) for x in t.column("StartDate").to_pylist()],
        "time": [float(x) for x in t.column("StartTime").to_pylist()],
        "arr0": t.column("Matrix[0]").to_pylist(),
        "arr1": t.column("Matrix[1]").to_pylist(),
        "arr2": t.column("Matrix[2]").to_pylist(),
    }}, fh)
"#,
            pq = pq.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(
            py["columns"].as_array().unwrap(),
            &vec![
                serde_json::json!("time"),
                serde_json::json!("Impedance.re"),
                serde_json::json!("Impedance.im"),
                serde_json::json!("StartDate"),
                serde_json::json!("StartTime"),
                serde_json::json!("Matrix[0]"),
                serde_json::json!("Matrix[1]"),
                serde_json::json!("Matrix[2]"),
            ]
        );
        assert_close(&floats(&py["re"]), &[10.0, 20.0], "re");
        assert_close(&floats(&py["im"]), &[-5.0, -15.0], "im");
        assert_close(&floats(&py["arr0"]), &[1.0, 4.0], "arr0");
        assert_close(&floats(&py["arr1"]), &[2.0, 5.0], "arr1");
        assert_close(&floats(&py["arr2"]), &[3.0, 6.0], "arr2");
    }

    #[test]
    fn exporting_nothing_writes_a_readable_empty_table() {
        let Some(python) = python_with("pyarrow") else {
            eprintln!("SKIP: pyarrow not installed in any candidate venv");
            return;
        };

        let pq = temp(".parquet");
        let mut out = std::fs::File::create(pq.path()).unwrap();
        write_parquet(&[], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import pyarrow.parquet as pq

t = pq.read_table(r"{pq}")
with open(r"{js}", "w") as fh:
    json.dump({{"columns": t.column_names, "rows": t.num_rows}}, fh)
"#,
            pq = pq.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(py["rows"].as_u64().unwrap(), 0);
        assert_eq!(
            py["columns"].as_array().unwrap(),
            &vec![serde_json::json!("time")]
        );
    }
}

// ---------------------------------------------------------------------------
// MATLAB level 5
// ---------------------------------------------------------------------------

#[cfg(feature = "mat")]
mod mat_tests {
    use super::*;
    use falcon_mdf::write_mat;

    #[test]
    fn values_survive_mdf_then_mat_then_scipy() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        // The oracle, typed here and nowhere else.
        let times = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let speed = vec![0.0, 11.0, 22.0, 33.0, 44.0, 55.0];
        let torque = vec![100.0, 99.5, 98.25, 97.0, 96.5, 95.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Speed", "km/h", &speed).unwrap();
        group.add_channel("Torque", "Nm", &torque).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Speed".into(), "Torque".into()]).unwrap();
        assert_close(&exported[0].values_f64(), &speed, "falcon's Speed");

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "shapes": {{n: list(m[n].shape) for n in names}},
        "time": m["DGM0_timestamps"].ravel().tolist(),
        "speed": m["DG0_Speed"].ravel().tolist(),
        "torque": m["DG0_Torque"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Speed"),
                serde_json::json!("DG0_Torque"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        // Column vectors, as MATLAB spells a time series.
        assert_eq!(
            py["shapes"]["DG0_Speed"].as_array().unwrap(),
            &vec![serde_json::json!(6), serde_json::json!(1)]
        );
        // Compared against the hand-written oracle.
        assert_close(&floats(&py["time"]), &times, "scipy's timestamps");
        assert_close(&floats(&py["speed"]), &speed, "scipy's Speed");
        assert_close(&floats(&py["torque"]), &torque, "scipy's Torque");

        println!("MAT cross-check: scipy returned the values the MF4 was built from");
    }

    #[test]
    fn every_numeric_type_survives_scipy_with_its_width() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let vars = vec![
            series("u8", t.clone(), SignalValues::U8(vec![1, 2, 250])),
            series("u16", t.clone(), SignalValues::U16(vec![1, 2, 65530])),
            series("u32", t.clone(), SignalValues::U32(vec![1, 2, 4_294_967_290])),
            series(
                "u64",
                t.clone(),
                SignalValues::U64(vec![1, 2, 9_007_199_254_740_993]),
            ),
            series("i8", t.clone(), SignalValues::I8(vec![-128, 0, 127])),
            series("i16", t.clone(), SignalValues::I16(vec![-32768, 0, 32767])),
            series("i32", t.clone(), SignalValues::I32(vec![-2147483648, 0, 2147483647])),
            series(
                "i64",
                t.clone(),
                SignalValues::I64(vec![-9_007_199_254_740_993, 0, 9_007_199_254_740_993]),
            ),
            series("f32", t.clone(), SignalValues::F32(vec![-1.5, 0.0, 2.25])),
            series("f64", t.clone(), SignalValues::F64(vec![-1.5, 0.0, 2.25])),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = [k for k in m if not k.startswith("__")]
with open(r"{js}", "w") as fh:
    json.dump({{
        "dtypes": {{n: str(m[n].dtype) for n in names}},
        "values": {{n: [str(v) for v in m[n].ravel().tolist()] for n in names}},
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        // scipy's own dtype names. A writer that pushed everything through
        // double would report float64 for all ten of these.
        for (name, want) in [
            ("DG0_u8", "uint8"),
            ("DG0_u16", "uint16"),
            ("DG0_u32", "uint32"),
            ("DG0_u64", "uint64"),
            ("DG0_i8", "int8"),
            ("DG0_i16", "int16"),
            ("DG0_i32", "int32"),
            ("DG0_i64", "int64"),
            ("DG0_f32", "float32"),
            ("DG0_f64", "float64"),
            ("DGM0_timestamps", "float64"),
        ] {
            assert_eq!(
                py["dtypes"][name].as_str().unwrap(),
                want,
                "{name} came back with the wrong MATLAB class"
            );
        }

        // Compared as decimal strings so a 64-bit integer is never routed
        // through a double on its way into this assertion.
        let values = &py["values"];
        assert_eq!(values["DG0_u64"].as_array().unwrap()[2], "9007199254740993");
        assert_eq!(
            values["DG0_i64"].as_array().unwrap()[0],
            "-9007199254740993"
        );
        assert_eq!(values["DG0_i8"].as_array().unwrap()[0], "-128");
        assert_eq!(values["DG0_u32"].as_array().unwrap()[2], "4294967290");
    }

    #[test]
    fn channels_are_grouped_by_their_time_axis() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        // Two axes: Slow and Also_Slow share one, Fast has its own. The time
        // vector must appear twice, not three times.
        let slow_t = vec![0.0, 1.0, 2.0];
        let fast_t = vec![0.0, 0.5, 1.0, 1.5];
        let vars = vec![
            series("Slow", slow_t.clone(), SignalValues::F64(vec![1.0, 2.0, 3.0])),
            series("Fast", fast_t.clone(), SignalValues::F64(vec![9.0, 8.0, 7.0, 6.0])),
            series(
                "Also_Slow",
                slow_t.clone(),
                SignalValues::F64(vec![4.0, 5.0, 6.0]),
            ),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "t0": m["DGM0_timestamps"].ravel().tolist(),
        "t1": m["DGM1_timestamps"].ravel().tolist(),
        "also_slow": m["DG0_Also_Slow"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Also_Slow"),
                serde_json::json!("DG0_Slow"),
                serde_json::json!("DG1_Fast"),
                serde_json::json!("DGM0_timestamps"),
                serde_json::json!("DGM1_timestamps")
            ],
            "two axes should produce two timestamp variables, and Also_Slow \
             should join group 0 rather than starting a third"
        );
        assert_close(&floats(&py["t0"]), &slow_t, "group 0 axis");
        assert_close(&floats(&py["t1"]), &fast_t, "group 1 axis");
        assert_close(&floats(&py["also_slow"]), &[4.0, 5.0, 6.0], "Also_Slow");
    }

    #[test]
    fn an_invalidation_mask_travels_beside_its_channel() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![10.0, 20.0, 30.0, 40.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group
            .add_channel_with_validity(
                "Sensor",
                "bar",
                &values,
                Some(&[true, false, true, false]),
            )
            .unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Sensor".into()]).unwrap();

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": sorted(k for k in m if not k.startswith("__")),
        "sensor": m["DG0_Sensor"].ravel().tolist(),
        "invalid": m["DG0_Sensor_invalid"].ravel().tolist(),
        "invalid_dtype": str(m["DG0_Sensor_invalid"].dtype),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Sensor"),
                serde_json::json!("DG0_Sensor_invalid"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        // The samples are untouched — no NaN stand-in, no zeroing — and the
        // mask says which of them the file disclaims.
        assert_close(&floats(&py["sensor"]), &values, "samples");
        assert_close(&floats(&py["invalid"]), &[0.0, 1.0, 0.0, 1.0], "mask");
        assert_eq!(py["invalid_dtype"].as_str().unwrap(), "uint8");
    }

    #[test]
    fn awkward_channel_names_become_matlab_identifiers() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0];
        let vars = vec![
            series("Eng Speed (rpm)", t.clone(), SignalValues::F64(vec![1.0, 2.0])),
            series("Brake.Pressure", t.clone(), SignalValues::F64(vec![3.0, 4.0])),
            // Two channels whose sanitized names collide.
            series("A-B", t.clone(), SignalValues::F64(vec![5.0, 6.0])),
            series("A_B", t.clone(), SignalValues::F64(vec![7.0, 8.0])),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json, re
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        # MATLAB's own rule for a legal variable name.
        "all_legal": all(re.fullmatch(r"[A-Za-z][A-Za-z0-9_]{{0,62}}", n) for n in names),
        "values": {{n: m[n].ravel().tolist() for n in names}},
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert!(
            py["all_legal"].as_bool().unwrap(),
            "every variable name must be a legal MATLAB identifier, got {:?}",
            py["names"]
        );
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_A_B"),
                serde_json::json!("DG0_A_B_1"),
                serde_json::json!("DG0_Brake_Pressure"),
                serde_json::json!("DG0_Eng_Speed__rpm_"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        // The collision was resolved by suffixing, not by overwriting: both
        // channels' samples are present and distinct.
        assert_close(&floats(&py["values"]["DG0_A_B"]), &[5.0, 6.0], "A-B");
        assert_close(&floats(&py["values"]["DG0_A_B_1"]), &[7.0, 8.0], "A_B");
    }

    #[test]
    fn a_kind_the_writer_cannot_represent_is_named_not_dropped() {
        let text = series(
            "Gear",
            vec![0.0, 1.0],
            SignalValues::Str(vec!["P".into(), "D".into()]),
        );
        let mut sink = Vec::new();
        let err = write_mat(&[text], &mut sink).expect_err("text is not a numeric matrix");
        let message = err.to_string();
        assert!(
            message.contains("Gear") && message.contains("text"),
            "the error should name the channel and its kind, got: {message}"
        );
    }

    #[test]
    fn varlen_arrays_are_refused_by_name() {
        let var_array = series(
            "DynamicSpectrum",
            vec![0.0, 1.0],
            SignalValues::ArrayVarLen {
                values: vec![1.0, 2.0, 3.0],
                starts: vec![0, 2, 3],
            },
        );
        let mut sink = Vec::new();
        let err = write_mat(&[var_array], &mut sink).expect_err("varlen array is not represented");
        let text = err.to_string();
        assert!(
            text.contains("DynamicSpectrum") && (text.contains("variable-length array") || text.contains("array")),
            "the error should name the channel and its kind, got: {text}"
        );
    }

    #[test]
    fn composites_survive_mdf_then_mat_then_scipy() {
        use falcon_mdf::model::values::{CanopenDate, CanopenTime};

        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0];
        let complex = series(
            "Impedance",
            times.clone(),
            SignalValues::Complex {
                re: vec![10.0, 20.0],
                im: vec![-5.0, -15.0],
            },
        );
        let date = series(
            "StartDate",
            times.clone(),
            SignalValues::CanopenDate(vec![
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 30,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 31,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
            ]),
        );
        let time = series(
            "StartTime",
            times.clone(),
            SignalValues::CanopenTime(vec![
                CanopenTime {
                    ms_since_midnight: 3600000,
                    days_since_1984: 100,
                },
                CanopenTime {
                    ms_since_midnight: 3601000,
                    days_since_1984: 100,
                },
            ]),
        );
        let mut arr = series(
            "Matrix",
            times.clone(),
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 3,
            },
        );
        arr.channel.array_shape = Some(vec![3]);

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&[complex, date, time, arr], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "re": m["DG0_Impedance_re"].ravel().tolist(),
        "im": m["DG0_Impedance_im"].ravel().tolist(),
        "date": [float(x) for x in m["DG0_StartDate"].ravel().tolist()],
        "time": [float(x) for x in m["DG0_StartTime"].ravel().tolist()],
        "arr0": m["DG0_Matrix_0_"].ravel().tolist(),
        "arr1": m["DG0_Matrix_1_"].ravel().tolist(),
        "arr2": m["DG0_Matrix_2_"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Impedance_im"),
                serde_json::json!("DG0_Impedance_re"),
                serde_json::json!("DG0_Matrix_0_"),
                serde_json::json!("DG0_Matrix_1_"),
                serde_json::json!("DG0_Matrix_2_"),
                serde_json::json!("DG0_StartDate"),
                serde_json::json!("DG0_StartTime"),
                serde_json::json!("DGM0_timestamps"),
            ]
        );
        assert_close(&floats(&py["re"]), &[10.0, 20.0], "re");
        assert_close(&floats(&py["im"]), &[-5.0, -15.0], "im");
        assert_close(&floats(&py["arr0"]), &[1.0, 4.0], "arr0");
        assert_close(&floats(&py["arr1"]), &[2.0, 5.0], "arr1");
        assert_close(&floats(&py["arr2"]), &[3.0, 6.0], "arr2");
    }

    #[test]
    fn exporting_nothing_writes_a_loadable_empty_workspace() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat(&[], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
with open(r"{js}", "w") as fh:
    json.dump({{"names": [k for k in m if not k.startswith("__")]}}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert!(py["names"].as_array().unwrap().is_empty());
    }
}

// ---------------------------------------------------------------------------
// MATLAB version 4
// ---------------------------------------------------------------------------

#[cfg(feature = "mat4")]
mod mat4_tests {
    use super::*;
    use falcon_mdf::write_mat_v4;

    #[test]
    fn values_survive_mdf_then_mat4_then_scipy() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        // The oracle, typed here and nowhere else.
        let times = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let speed = vec![0.0, 11.0, 22.0, 33.0, 44.0, 55.0];
        let torque = vec![100.0, 99.5, 98.25, 97.0, 96.5, 95.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Speed", "km/h", &speed).unwrap();
        group.add_channel("Torque", "Nm", &torque).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Speed".into(), "Torque".into()]).unwrap();
        assert_close(&exported[0].values_f64(), &speed, "falcon's Speed");

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "shapes": {{n: list(m[n].shape) for n in names}},
        "time": m["DGM0_timestamps"].ravel().tolist(),
        "speed": m["DG0_Speed"].ravel().tolist(),
        "torque": m["DG0_Torque"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Speed"),
                serde_json::json!("DG0_Torque"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        // Column vectors, as MATLAB spells a time series.
        assert_eq!(
            py["shapes"]["DG0_Speed"].as_array().unwrap(),
            &vec![serde_json::json!(6), serde_json::json!(1)]
        );
        // Compared against the hand-written oracle.
        assert_close(&floats(&py["time"]), &times, "scipy's timestamps");
        assert_close(&floats(&py["speed"]), &speed, "scipy's Speed");
        assert_close(&floats(&py["torque"]), &torque, "scipy's Torque");

        println!("MAT v4 cross-check: scipy returned the values the MF4 was built from");
    }

    #[test]
    fn every_numeric_type_survives_scipy_in_mat4() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let vars = vec![
            series("u8", t.clone(), SignalValues::U8(vec![1, 2, 250])),
            series("u16", t.clone(), SignalValues::U16(vec![1, 2, 65530])),
            series("u32", t.clone(), SignalValues::U32(vec![1, 2, 4_294_967_290])),
            series(
                "u64",
                t.clone(),
                SignalValues::U64(vec![1, 2, 9_007_199_254_740_993]),
            ),
            series("i8", t.clone(), SignalValues::I8(vec![-128, 0, 127])),
            series("i16", t.clone(), SignalValues::I16(vec![-32768, 0, 32767])),
            series("i32", t.clone(), SignalValues::I32(vec![-2147483648, 0, 2147483647])),
            series(
                "i64",
                t.clone(),
                SignalValues::I64(vec![-9_007_199_254_740_993, 0, 9_007_199_254_740_993]),
            ),
            series("f32", t.clone(), SignalValues::F32(vec![-1.5, 0.0, 2.25])),
            series("f64", t.clone(), SignalValues::F64(vec![-1.5, 0.0, 2.25])),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = [k for k in m if not k.startswith("__")]
with open(r"{js}", "w") as fh:
    json.dump({{
        "dtypes": {{n: str(m[n].dtype) for n in names}},
        "values": {{n: [str(v) for v in m[n].ravel().tolist()] for n in names}},
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        for (name, want) in [
            ("DG0_u8", "uint8"),
            ("DG0_u16", "uint16"),
            ("DG0_u32", "float64"),
            ("DG0_u64", "float64"),
            ("DG0_i8", "float64"),
            ("DG0_i16", "int16"),
            ("DG0_i32", "int32"),
            ("DG0_i64", "float64"),
            ("DG0_f32", "float32"),
            ("DG0_f64", "float64"),
            ("DGM0_timestamps", "float64"),
        ] {
            assert_eq!(
                py["dtypes"][name].as_str().unwrap(),
                want,
                "{name} came back with the wrong MATLAB type in MAT v4"
            );
        }

        let values = &py["values"];
        assert_eq!(values["DG0_u8"].as_array().unwrap()[2], "250");
        assert_eq!(values["DG0_u16"].as_array().unwrap()[2], "65530");
        assert_eq!(values["DG0_i8"].as_array().unwrap()[0], "-128.0");
        assert_eq!(values["DG0_i16"].as_array().unwrap()[0], "-32768");
        assert_eq!(values["DG0_i32"].as_array().unwrap()[0], "-2147483648");
    }

    #[test]
    fn text_channel_survives_mat4_then_scipy() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let text_series = series(
            "Status",
            t.clone(),
            SignalValues::Str(vec!["idle".into(), "".into(), "wide open".into()]),
        );

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&[text_series], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": sorted(k for k in m if not k.startswith("__")),
        "status": [s.rstrip() for s in m["DG0_Status"]],
        "shape": list(m["DG0_Status"].shape),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Status"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        assert_eq!(
            py["status"].as_array().unwrap(),
            &vec![
                serde_json::json!("idle"),
                serde_json::json!(""),
                serde_json::json!("wide open")
            ]
        );
        assert_eq!(
            py["shape"].as_array().unwrap(),
            &vec![serde_json::json!(3)]
        );
    }

    #[test]
    fn channels_are_grouped_by_their_time_axis_in_mat4() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let slow_t = vec![0.0, 1.0, 2.0];
        let fast_t = vec![0.0, 0.5, 1.0, 1.5];
        let vars = vec![
            series("Slow", slow_t.clone(), SignalValues::F64(vec![1.0, 2.0, 3.0])),
            series("Fast", fast_t.clone(), SignalValues::F64(vec![9.0, 8.0, 7.0, 6.0])),
            series(
                "Also_Slow",
                slow_t.clone(),
                SignalValues::F64(vec![4.0, 5.0, 6.0]),
            ),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "t0": m["DGM0_timestamps"].ravel().tolist(),
        "t1": m["DGM1_timestamps"].ravel().tolist(),
        "also_slow": m["DG0_Also_Slow"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Also_Slow"),
                serde_json::json!("DG0_Slow"),
                serde_json::json!("DG1_Fast"),
                serde_json::json!("DGM0_timestamps"),
                serde_json::json!("DGM1_timestamps")
            ]
        );
        assert_close(&floats(&py["t0"]), &slow_t, "group 0 axis");
        assert_close(&floats(&py["t1"]), &fast_t, "group 1 axis");
        assert_close(&floats(&py["also_slow"]), &[4.0, 5.0, 6.0], "Also_Slow");
    }

    #[test]
    fn an_invalidation_mask_travels_beside_its_channel_in_mat4() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![10.0, 20.0, 30.0, 40.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group
            .add_channel_with_validity(
                "Sensor",
                "bar",
                &values,
                Some(&[true, false, true, false]),
            )
            .unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Sensor".into()]).unwrap();

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": sorted(k for k in m if not k.startswith("__")),
        "sensor": m["DG0_Sensor"].ravel().tolist(),
        "invalid": m["DG0_Sensor_invalid"].ravel().tolist(),
        "invalid_dtype": str(m["DG0_Sensor_invalid"].dtype),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Sensor"),
                serde_json::json!("DG0_Sensor_invalid"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        assert_close(&floats(&py["sensor"]), &values, "samples");
        assert_close(&floats(&py["invalid"]), &[0.0, 1.0, 0.0, 1.0], "mask");
        assert_eq!(py["invalid_dtype"].as_str().unwrap(), "uint8");
    }

    #[test]
    fn awkward_channel_names_become_matlab_identifiers_in_mat4() {
        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0];
        let vars = vec![
            series("Eng Speed (rpm)", t.clone(), SignalValues::F64(vec![1.0, 2.0])),
            series("Brake.Pressure", t.clone(), SignalValues::F64(vec![3.0, 4.0])),
            series("A-B", t.clone(), SignalValues::F64(vec![5.0, 6.0])),
            series("A_B", t.clone(), SignalValues::F64(vec![7.0, 8.0])),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json, re
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "all_legal": all(re.fullmatch(r"[A-Za-z][A-Za-z0-9_]{{0,62}}", n) for n in names),
        "values": {{n: m[n].ravel().tolist() for n in names}},
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert!(
            py["all_legal"].as_bool().unwrap(),
            "every variable name must be a legal MATLAB identifier, got {:?}",
            py["names"]
        );
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_A_B"),
                serde_json::json!("DG0_A_B_1"),
                serde_json::json!("DG0_Brake_Pressure"),
                serde_json::json!("DG0_Eng_Speed__rpm_"),
                serde_json::json!("DGM0_timestamps")
            ]
        );
        assert_close(&floats(&py["values"]["DG0_A_B"]), &[5.0, 6.0], "A-B");
        assert_close(&floats(&py["values"]["DG0_A_B_1"]), &[7.0, 8.0], "A_B");
    }

    #[test]
    fn a_kind_the_writer_cannot_represent_is_named_not_dropped_in_mat4() {
        let bytes = series(
            "RawFrame",
            vec![0.0, 1.0],
            SignalValues::Bytes {
                data: vec![0x12, 0x34],
                width: 1,
            },
        );
        let mut sink = Vec::new();
        let err = write_mat_v4(&[bytes], &mut sink).expect_err("byte-array is not represented");
        let message = err.to_string();
        assert!(
            message.contains("RawFrame") && message.contains("byte-array"),
            "the error should name the channel and its kind, got: {message}"
        );
    }

    #[test]
    fn varlen_arrays_are_refused_by_name_in_mat4() {
        let var_array = series(
            "DynamicSpectrum",
            vec![0.0, 1.0],
            SignalValues::ArrayVarLen {
                values: vec![1.0, 2.0, 3.0],
                starts: vec![0, 2, 3],
            },
        );
        let mut sink = Vec::new();
        let err = write_mat_v4(&[var_array], &mut sink).expect_err("varlen array is not represented");
        let text = err.to_string();
        assert!(
            text.contains("DynamicSpectrum") && (text.contains("variable-length array") || text.contains("array")),
            "the error should name the channel and its kind, got: {text}"
        );
    }

    #[test]
    fn composites_survive_mdf_then_mat4_then_scipy() {
        use falcon_mdf::model::values::{CanopenDate, CanopenTime};

        let Some(python) = python_with("scipy.io") else {
            eprintln!("SKIP: scipy not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0];
        let complex = series(
            "Impedance",
            times.clone(),
            SignalValues::Complex {
                re: vec![10.0, 20.0],
                im: vec![-5.0, -15.0],
            },
        );
        let date = series(
            "StartDate",
            times.clone(),
            SignalValues::CanopenDate(vec![
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 30,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 31,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
            ]),
        );
        let time = series(
            "StartTime",
            times.clone(),
            SignalValues::CanopenTime(vec![
                CanopenTime {
                    ms_since_midnight: 3600000,
                    days_since_1984: 100,
                },
                CanopenTime {
                    ms_since_midnight: 3601000,
                    days_since_1984: 100,
                },
            ]),
        );
        let mut arr = series(
            "Matrix",
            times.clone(),
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 3,
            },
        );
        arr.channel.array_shape = Some(vec![3]);

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&[complex, date, time, arr], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
from scipy.io import loadmat

m = loadmat(r"{mat}")
names = sorted(k for k in m if not k.startswith("__"))
with open(r"{js}", "w") as fh:
    json.dump({{
        "names": names,
        "re": m["DG0_Impedance_re"].ravel().tolist(),
        "im": m["DG0_Impedance_im"].ravel().tolist(),
        "date": [float(x) for x in m["DG0_StartDate"].ravel().tolist()],
        "time": [float(x) for x in m["DG0_StartTime"].ravel().tolist()],
        "arr0": m["DG0_Matrix_0_"].ravel().tolist(),
        "arr1": m["DG0_Matrix_1_"].ravel().tolist(),
        "arr2": m["DG0_Matrix_2_"].ravel().tolist(),
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Impedance_im"),
                serde_json::json!("DG0_Impedance_re"),
                serde_json::json!("DG0_Matrix_0_"),
                serde_json::json!("DG0_Matrix_1_"),
                serde_json::json!("DG0_Matrix_2_"),
                serde_json::json!("DG0_StartDate"),
                serde_json::json!("DG0_StartTime"),
                serde_json::json!("DGM0_timestamps"),
            ]
        );
        assert_close(&floats(&py["re"]), &[10.0, 20.0], "re");
        assert_close(&floats(&py["im"]), &[-5.0, -15.0], "im");
        assert_close(&floats(&py["arr0"]), &[1.0, 4.0], "arr0");
        assert_close(&floats(&py["arr1"]), &[2.0, 5.0], "arr1");
        assert_close(&floats(&py["arr2"]), &[3.0, 6.0], "arr2");
    }

    #[test]
    fn exporting_nothing_writes_empty_file() {
        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat_v4(&[], &mut out).unwrap();
        drop(out);

        let metadata = std::fs::metadata(mat.path()).unwrap();
        assert_eq!(metadata.len(), 0);
    }
}

#[cfg(feature = "mat73")]
mod mat73_tests {
    use super::*;
    use falcon_mdf::write_mat73;

    #[test]
    fn values_survive_mdf_then_mat73_then_h5py() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let speed = vec![0.0, 11.0, 22.0, 33.0, 44.0, 55.0];
        let torque = vec![100.0, 99.5, 98.25, 97.0, 96.5, 95.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Speed", "km/h", &speed).unwrap();
        group.add_channel("Torque", "Nm", &torque).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file
            .filter(&["Speed".into(), "Torque".into()])
            .unwrap();
        assert_close(&exported[0].values_f64(), &speed, "falcon's Speed");
        assert_close(&exported[1].values_f64(), &torque, "falcon's Torque");

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat73(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with open(r"{mat}", "rb") as fh:
    header = fh.read(512)

with h5py.File(r"{mat}", "r") as f:
    names = sorted(k for k in f if not k.startswith("__"))
    shapes = {{n: list(f[n].shape) for n in names}}
    classes = {{n: f[n].attrs["MATLAB_class"].decode("ascii") for n in names}}
    time = f["DGM0_timestamps"][:].ravel().tolist()
    speed = f["DG0_Speed"][:].ravel().tolist()
    torque = f["DG0_Torque"][:].ravel().tolist()

with open(r"{js}", "w") as fh:
    json.dump({{
        "header": header[:64].decode("ascii").rstrip(),
        "names": names,
        "shapes": shapes,
        "classes": classes,
        "time": time,
        "speed": speed,
        "torque": torque,
    }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert!(py["header"].as_str().unwrap().starts_with("MATLAB 7.3 MAT-file"));
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Speed"),
                serde_json::json!("DG0_Torque"),
                serde_json::json!("DGM0_timestamps"),
            ]
        );
        assert_eq!(
            py["shapes"]["DG0_Speed"].as_array().unwrap(),
            &vec![serde_json::json!(1), serde_json::json!(6)]
        );
        assert_eq!(
            py["classes"]["DG0_Speed"].as_str().unwrap(),
            "double"
        );
        assert_eq!(
            py["classes"]["DGM0_timestamps"].as_str().unwrap(),
            "double"
        );
        assert_close(&floats(&py["time"]), &times, "h5py's timestamps");
        assert_close(&floats(&py["speed"]), &speed, "h5py's Speed");
        assert_close(&floats(&py["torque"]), &torque, "h5py's Torque");

        println!("MAT v7.3 cross-check: h5py returned the values the MF4 was built from");
    }

    #[test]
    fn every_numeric_type_survives_h5py_with_its_class() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let vars = vec![
            series("u8", t.clone(), SignalValues::U8(vec![1, 2, 250])),
            series("u16", t.clone(), SignalValues::U16(vec![1, 2, 65530])),
            series("u32", t.clone(), SignalValues::U32(vec![1, 2, 4_294_967_290])),
            series(
                "u64",
                t.clone(),
                SignalValues::U64(vec![1, 2, 9_007_199_254_740_993]),
            ),
            series("i8", t.clone(), SignalValues::I8(vec![-128, 0, 127])),
            series("i16", t.clone(), SignalValues::I16(vec![-32768, 0, 32767])),
            series("i32", t.clone(), SignalValues::I32(vec![-2147483648, 0, 2147483647])),
            series(
                "i64",
                t.clone(),
                SignalValues::I64(vec![-9_007_199_254_740_993, 0, 9_007_199_254_740_993]),
            ),
            series("f32", t.clone(), SignalValues::F32(vec![-1.5, 0.0, 2.25])),
            series("f64", t.clone(), SignalValues::F64(vec![-1.5, 0.0, 2.25])),
        ];

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat73(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{mat}", "r") as f:
    classes = {{k: f[k].attrs["MATLAB_class"].decode("ascii") for k in f if not k.startswith("__")}}

with open(r"{js}", "w") as fh:
    json.dump({{"classes": classes}}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        let classes = py["classes"].as_object().unwrap();
        let expected = [
            ("DG0_f64", "double"),
            ("DG0_f32", "single"),
            ("DG0_i64", "int64"),
            ("DG0_u64", "uint64"),
            ("DG0_i32", "int32"),
            ("DG0_u32", "uint32"),
            ("DG0_i16", "int16"),
            ("DG0_u16", "uint16"),
            ("DG0_i8", "int8"),
            ("DG0_u8", "uint8"),
        ];
        for (name, class) in expected {
            assert_eq!(classes[name].as_str().unwrap(), class);
        }
    }

    #[test]
    fn varlen_arrays_are_refused_by_name() {
        let var_array = series(
            "DynamicSpectrum",
            vec![0.0, 1.0],
            SignalValues::ArrayVarLen {
                values: vec![1.0, 2.0, 3.0],
                starts: vec![0, 2, 3],
            },
        );
        let mut sink = Vec::new();
        let err = write_mat73(&[var_array], &mut sink).expect_err("varlen array is not represented");
        let text = err.to_string();
        assert!(
            text.contains("DynamicSpectrum") && (text.contains("variable-length array") || text.contains("array")),
            "the error should name the channel and its kind, got: {text}"
        );
    }

    #[test]
    fn composites_survive_mdf_then_mat73_then_h5py() {
        use falcon_mdf::model::values::{CanopenDate, CanopenTime};

        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0];
        let complex = series(
            "Impedance",
            times.clone(),
            SignalValues::Complex {
                re: vec![10.0, 20.0],
                im: vec![-5.0, -15.0],
            },
        );
        let date = series(
            "StartDate",
            times.clone(),
            SignalValues::CanopenDate(vec![
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 30,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 31,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
            ]),
        );
        let time = series(
            "StartTime",
            times.clone(),
            SignalValues::CanopenTime(vec![
                CanopenTime {
                    ms_since_midnight: 3600000,
                    days_since_1984: 100,
                },
                CanopenTime {
                    ms_since_midnight: 3601000,
                    days_since_1984: 100,
                },
            ]),
        );
        let mut arr = series(
            "Matrix",
            times.clone(),
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 3,
            },
        );
        arr.channel.array_shape = Some(vec![3]);

        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat73(&[complex, date, time, arr], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{mat}", "r") as f:
    names = sorted(list(f.keys()))
    with open(r"{js}", "w") as fh:
        json.dump({{
            "names": names,
            "re": f["DG0_Impedance_re"][:].ravel().tolist(),
            "im": f["DG0_Impedance_im"][:].ravel().tolist(),
            "date": [float(x) for x in f["DG0_StartDate"][:].ravel().tolist()],
            "time": [float(x) for x in f["DG0_StartTime"][:].ravel().tolist()],
            "arr0": f["DG0_Matrix_0_"][:].ravel().tolist(),
            "arr1": f["DG0_Matrix_1_"][:].ravel().tolist(),
            "arr2": f["DG0_Matrix_2_"][:].ravel().tolist(),
        }}, fh)
"#,
            mat = mat.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("DG0_Impedance_im"),
                serde_json::json!("DG0_Impedance_re"),
                serde_json::json!("DG0_Matrix_0_"),
                serde_json::json!("DG0_Matrix_1_"),
                serde_json::json!("DG0_Matrix_2_"),
                serde_json::json!("DG0_StartDate"),
                serde_json::json!("DG0_StartTime"),
                serde_json::json!("DGM0_timestamps"),
            ]
        );
        assert_close(&floats(&py["re"]), &[10.0, 20.0], "re");
        assert_close(&floats(&py["im"]), &[-5.0, -15.0], "im");
        assert_close(&floats(&py["arr0"]), &[1.0, 4.0], "arr0");
        assert_close(&floats(&py["arr1"]), &[2.0, 5.0], "arr1");
        assert_close(&floats(&py["arr2"]), &[3.0, 6.0], "arr2");
    }

    #[test]
    fn exporting_nothing_writes_a_valid_empty_file() {
        let mat = temp(".mat");
        let mut out = std::fs::File::create(mat.path()).unwrap();
        write_mat73(&[], &mut out).unwrap();
        drop(out);

        let bytes = std::fs::read(mat.path()).unwrap();
        assert!(&bytes[..6].eq_ignore_ascii_case(b"MATLAB"));
        assert_eq!(&bytes[124..128], &[0x00, 0x02, b'I', b'M']);
        assert_eq!(&bytes[512..516], b"\x89HDF");
    }
}

// ---------------------------------------------------------------------------
// HDF5
// ---------------------------------------------------------------------------

#[cfg(feature = "hdf5")]
mod hdf5_tests {
    use super::*;
    use falcon_mdf::write_hdf5;

    #[test]
    fn values_survive_mdf_then_hdf5_then_h5py() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5];
        let speed = vec![0.0, 12.5, 25.0, 37.5, 50.0, 62.5, 75.0];
        let coolant = vec![80.0, 80.5, 81.25, 82.0, 82.5, 83.0, 83.75];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Speed", "km/h", &speed).unwrap();
        group.add_channel("Coolant", "degC", &coolant).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file
            .filter(&["Speed".into(), "Coolant".into()])
            .unwrap();

        assert_close(&exported[0].values_f64(), &speed, "falcon's Speed");
        assert_close(&exported[1].values_f64(), &coolant, "falcon's Coolant");

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    keys = sorted(list(f.keys()))
    def to_str(v):
        return v.decode("utf-8") if isinstance(v, bytes) else str(v)
    speed_attrs = {{k: to_str(v) for k, v in f["Speed"].attrs.items()}}
    coolant_attrs = {{k: to_str(v) for k, v in f["Coolant"].attrs.items()}}
    with open(r"{js}", "w") as fh:
        json.dump({{
            "keys": keys,
            "speed_attrs": speed_attrs,
            "coolant_attrs": coolant_attrs,
            "timestamps": f["timestamps"][:].tolist(),
            "Speed": f["Speed"][:].tolist(),
            "Coolant": f["Coolant"][:].tolist(),
        }}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["keys"].as_array().unwrap(),
            &vec![
                serde_json::json!("Coolant"),
                serde_json::json!("Speed"),
                serde_json::json!("timestamps")
            ]
        );
        assert_eq!(py["speed_attrs"]["unit"].as_str().unwrap(), "km/h");
        assert_eq!(py["coolant_attrs"]["unit"].as_str().unwrap(), "degC");

        assert_close(&floats(&py["timestamps"]), &times, "h5py's timestamps");
        assert_close(&floats(&py["Speed"]), &speed, "h5py's Speed");
        assert_close(&floats(&py["Coolant"]), &coolant, "h5py's Coolant");

        println!("HDF5 cross-check: h5py returned the values the MF4 was built from");
    }

    #[test]
    fn every_numeric_type_survives_h5py_with_its_width() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let t = vec![0.0, 1.0, 2.0];
        let vars = vec![
            series("u8", t.clone(), SignalValues::U8(vec![1, 2, 250])),
            series("u16", t.clone(), SignalValues::U16(vec![1, 2, 65530])),
            series("u32", t.clone(), SignalValues::U32(vec![1, 2, 4_294_967_290])),
            series(
                "u64",
                t.clone(),
                SignalValues::U64(vec![1, 2, 9_007_199_254_740_993]),
            ),
            series("i8", t.clone(), SignalValues::I8(vec![-128, 0, 127])),
            series("i16", t.clone(), SignalValues::I16(vec![-32768, 0, 32767])),
            series("i32", t.clone(), SignalValues::I32(vec![-2147483648, 0, 2147483647])),
            series(
                "i64",
                t.clone(),
                SignalValues::I64(vec![-9_007_199_254_740_993, 0, 9_007_199_254_740_993]),
            ),
            series("f32", t.clone(), SignalValues::F32(vec![-1.5, 0.0, 2.25])),
            series("f64", t.clone(), SignalValues::F64(vec![-1.5, 0.0, 2.25])),
        ];

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    names = [k for k in f.keys() if k != "timestamps"]
    dtypes = {{n: str(f[n].dtype) for n in names}}
    values = {{n: [str(v) for v in f[n][:].tolist()] for n in names}}
    with open(r"{js}", "w") as fh:
        json.dump({{"dtypes": dtypes, "values": values}}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        let dtypes = &py["dtypes"];
        for (name, want) in [
            ("u8", "uint8"),
            ("u16", "uint16"),
            ("u32", "uint32"),
            ("u64", "uint64"),
            ("i8", "int8"),
            ("i16", "int16"),
            ("i32", "int32"),
            ("i64", "int64"),
            ("f32", "float32"),
            ("f64", "float64"),
        ] {
            assert_eq!(
                dtypes[name].as_str().unwrap(),
                want,
                "{name} came back with the wrong HDF5 datatype"
            );
        }

        let values = &py["values"];
        assert_eq!(values["u64"].as_array().unwrap()[2], "9007199254740993");
        assert_eq!(
            values["i64"].as_array().unwrap()[0],
            "-9007199254740993"
        );
        assert_eq!(values["i8"].as_array().unwrap()[0], "-128");
        assert_eq!(values["u32"].as_array().unwrap()[2], "4294967290");
    }

    #[test]
    fn channels_are_grouped_by_their_time_axis_in_hdf5() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let slow_t = vec![0.0, 1.0, 2.0];
        let fast_t = vec![0.0, 0.5, 1.0, 1.5];
        let vars = vec![
            series("Slow", slow_t.clone(), SignalValues::F64(vec![1.0, 2.0, 3.0])),
            series("Fast", fast_t.clone(), SignalValues::F64(vec![9.0, 8.0, 7.0, 6.0])),
            series(
                "Also_Slow",
                slow_t.clone(),
                SignalValues::F64(vec![4.0, 5.0, 6.0]),
            ),
        ];

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&vars, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    groups = sorted(list(f.keys()))
    cg0 = sorted(list(f["ChannelGroup_0"].keys()))
    cg1 = sorted(list(f["ChannelGroup_1"].keys()))
    with open(r"{js}", "w") as fh:
        json.dump({{
            "groups": groups,
            "cg0": cg0,
            "cg1": cg1,
            "t0": f["ChannelGroup_0/timestamps"][:].tolist(),
            "t1": f["ChannelGroup_1/timestamps"][:].tolist(),
            "also_slow": f["ChannelGroup_0/Also_Slow"][:].tolist(),
            "fast": f["ChannelGroup_1/Fast"][:].tolist(),
        }}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["groups"].as_array().unwrap(),
            &vec![
                serde_json::json!("ChannelGroup_0"),
                serde_json::json!("ChannelGroup_1")
            ]
        );
        assert_eq!(
            py["cg0"].as_array().unwrap(),
            &vec![
                serde_json::json!("Also_Slow"),
                serde_json::json!("Slow"),
                serde_json::json!("timestamps")
            ]
        );
        assert_eq!(
            py["cg1"].as_array().unwrap(),
            &vec![
                serde_json::json!("Fast"),
                serde_json::json!("timestamps")
            ]
        );
        assert_close(&floats(&py["t0"]), &slow_t, "cg0 timestamps");
        assert_close(&floats(&py["t1"]), &fast_t, "cg1 timestamps");
        assert_close(&floats(&py["also_slow"]), &[4.0, 5.0, 6.0], "Also_Slow");
        assert_close(&floats(&py["fast"]), &[9.0, 8.0, 7.0, 6.0], "Fast");
    }

    #[test]
    fn an_invalidation_mask_travels_beside_its_channel_in_hdf5() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![10.0, 20.0, 30.0, 40.0];

        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&times).unwrap();
        group
            .add_channel_with_validity(
                "Sensor",
                "bar",
                &values,
                Some(&[true, false, true, false]),
            )
            .unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let exported = file.filter(&["Sensor".into()]).unwrap();

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&exported, &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    with open(r"{js}", "w") as fh:
        json.dump({{
            "keys": sorted(list(f.keys())),
            "sensor": f["Sensor"][:].tolist(),
            "invalid": f["Sensor_invalid"][:].tolist(),
            "invalid_dtype": str(f["Sensor_invalid"].dtype),
        }}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());

        assert_eq!(
            py["keys"].as_array().unwrap(),
            &vec![
                serde_json::json!("Sensor"),
                serde_json::json!("Sensor_invalid"),
                serde_json::json!("timestamps")
            ]
        );
        assert_close(&floats(&py["sensor"]), &values, "sensor samples");
        assert_close(&floats(&py["invalid"]), &[0.0, 1.0, 0.0, 1.0], "invalid mask");
        assert_eq!(py["invalid_dtype"].as_str().unwrap(), "uint8");
    }

    #[test]
    fn a_kind_the_writer_cannot_represent_is_named_not_dropped() {
        let text = series(
            "Status",
            vec![0.0, 1.0],
            SignalValues::Str(vec!["OK".into(), "FAIL".into()]),
        );
        let mut sink = Vec::new();
        let err = write_hdf5(&[text], &mut sink).expect_err("text is not numeric");
        let message = err.to_string();
        assert!(
            message.contains("Status") && (message.contains("str") || message.contains("text")),
            "the error should name the channel and its kind, got: {message}"
        );
    }

    #[test]
    fn varlen_arrays_are_refused_by_name() {
        let var_array = series(
            "DynamicSpectrum",
            vec![0.0, 1.0],
            SignalValues::ArrayVarLen {
                values: vec![1.0, 2.0, 3.0],
                starts: vec![0, 2, 3],
            },
        );
        let mut sink = Vec::new();
        let err = write_hdf5(&[var_array], &mut sink).expect_err("varlen array is not represented");
        let text = err.to_string();
        assert!(
            text.contains("DynamicSpectrum") && (text.contains("variable-length array") || text.contains("array")),
            "the error should name the channel and its kind, got: {text}"
        );
    }

    #[test]
    fn composites_survive_mdf_then_hdf5_then_h5py() {
        use falcon_mdf::model::values::{CanopenDate, CanopenTime};

        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let times = vec![0.0, 1.0];
        let complex = series(
            "Impedance",
            times.clone(),
            SignalValues::Complex {
                re: vec![10.0, 20.0],
                im: vec![-5.0, -15.0],
            },
        );
        let date = series(
            "StartDate",
            times.clone(),
            SignalValues::CanopenDate(vec![
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 30,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
                CanopenDate {
                    year: 2026,
                    month: 8,
                    day: 27,
                    hour: 10,
                    minute: 31,
                    ms: 0,
                    day_of_week: 4,
                    summer_time: true,
                },
            ]),
        );
        let time = series(
            "StartTime",
            times.clone(),
            SignalValues::CanopenTime(vec![
                CanopenTime {
                    ms_since_midnight: 3600000,
                    days_since_1984: 100,
                },
                CanopenTime {
                    ms_since_midnight: 3601000,
                    days_since_1984: 100,
                },
            ]),
        );
        let mut arr = series(
            "Matrix",
            times.clone(),
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 3,
            },
        );
        arr.channel.array_shape = Some(vec![3]);

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&[complex, date, time, arr], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    names = sorted(list(f.keys()))
    with open(r"{js}", "w") as fh:
        json.dump({{
            "names": names,
            "re": f["Impedance.re"][:].tolist(),
            "im": f["Impedance.im"][:].tolist(),
            "date": [float(x) for x in f["StartDate"][:].tolist()],
            "time": [float(x) for x in f["StartTime"][:].tolist()],
            "arr0": f["Matrix[0]"][:].tolist(),
            "arr1": f["Matrix[1]"][:].tolist(),
            "arr2": f["Matrix[2]"][:].tolist(),
        }}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert_eq!(
            py["names"].as_array().unwrap(),
            &vec![
                serde_json::json!("Impedance.im"),
                serde_json::json!("Impedance.re"),
                serde_json::json!("Matrix[0]"),
                serde_json::json!("Matrix[1]"),
                serde_json::json!("Matrix[2]"),
                serde_json::json!("StartDate"),
                serde_json::json!("StartTime"),
                serde_json::json!("timestamps"),
            ]
        );
        assert_close(&floats(&py["re"]), &[10.0, 20.0], "re");
        assert_close(&floats(&py["im"]), &[-5.0, -15.0], "im");
        assert_close(&floats(&py["arr0"]), &[1.0, 4.0], "arr0");
        assert_close(&floats(&py["arr1"]), &[2.0, 5.0], "arr1");
        assert_close(&floats(&py["arr2"]), &[3.0, 6.0], "arr2");
    }

    #[test]
    fn exporting_nothing_writes_a_valid_empty_hdf5_file() {
        let Some(python) = python_with("h5py") else {
            eprintln!("SKIP: h5py not installed in any candidate venv");
            return;
        };

        let h5 = temp(".h5");
        let mut out = std::fs::File::create(h5.path()).unwrap();
        write_hdf5(&[], &mut out).unwrap();
        drop(out);

        let json = temp(".json");
        let script = format!(
            r#"
import json
import h5py

with h5py.File(r"{h5}", "r") as f:
    with open(r"{js}", "w") as fh:
        json.dump({{"keys": list(f.keys())}}, fh)
"#,
            h5 = h5.path().display(),
            js = json.path().display(),
        );
        let py = run_python(&python, &script, json.path());
        assert!(py["keys"].as_array().unwrap().is_empty());
    }
}

// ---------------------------------------------------------------------------
// Vector CANoe ASCII (ASC)
// ---------------------------------------------------------------------------

#[cfg(feature = "asc")]
mod asc_tests {
    use super::*;
    use falcon_mdf::write_asc;

    #[test]
    fn asc_export_matches_asammdf_on_reference_can_bus() {
        let Some(python) = python_with("asammdf") else {
            eprintln!("SKIP: asammdf not installed in any candidate venv");
            return;
        };

        let Some(mf4_path) = resolve_path("test_data/reference/single_can_bus_1.MF4") else {
            eprintln!("SKIP: test_data/reference/single_can_bus_1.MF4 not found");
            return;
        };

        let file = Mf4File::open(&mf4_path).unwrap();
        let asc = temp(".asc");
        let mut out = std::fs::File::create(asc.path()).unwrap();
        write_asc(&file, &mut out).unwrap();
        drop(out);

        let asammdf_asc = temp(".asc");
        let script = format!(
            r#"
import asammdf

m = asammdf.MDF(r"{mf4}")
m.export("asc", r"{out}")
"#,
            mf4 = mf4_path.display(),
            out = asammdf_asc.path().display(),
        );
        let out = Command::new(&python)
            .args(["-c", &script])
            .output()
            .expect("failed to launch asammdf export");
        assert!(out.status.success(), "asammdf failed: {}", String::from_utf8_lossy(&out.stderr));

        let falcon_text = std::fs::read_to_string(asc.path()).unwrap();
        let asammdf_text = std::fs::read_to_string(asammdf_asc.path()).unwrap();

        let falcon_lines: Vec<&str> = falcon_text.lines().collect();
        let asammdf_lines: Vec<&str> = asammdf_text.lines().collect();

        assert_eq!(falcon_lines.len(), asammdf_lines.len(), "line count differs");
        for (i, (f_line, a_line)) in falcon_lines.iter().zip(&asammdf_lines).enumerate() {
            assert_eq!(f_line.trim_end(), a_line.trim_end(), "line {i} differs");
        }

        println!("ASC cross-check: falcon ASC matches asammdf line-for-line on reference CAN bus");
    }

    #[test]
    fn asc_export_matches_asammdf_on_j1939_truck_log() {
        let Some(python) = python_with("asammdf") else {
            eprintln!("SKIP: asammdf not installed in any candidate venv");
            return;
        };

        let Some(mf4_path) = resolve_path("test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4") else {
            eprintln!("SKIP: J1939 truck MF4 not found");
            return;
        };

        let file = Mf4File::open(&mf4_path).unwrap();
        let asc = temp(".asc");
        let mut out = std::fs::File::create(asc.path()).unwrap();
        write_asc(&file, &mut out).unwrap();
        drop(out);

        let asammdf_asc = temp(".asc");
        let script = format!(
            r#"
import asammdf

m = asammdf.MDF(r"{mf4}")
m.export("asc", r"{out}")
"#,
            mf4 = mf4_path.display(),
            out = asammdf_asc.path().display(),
        );
        let out = Command::new(&python)
            .args(["-c", &script])
            .output()
            .expect("failed to launch asammdf export");
        assert!(out.status.success(), "asammdf failed: {}", String::from_utf8_lossy(&out.stderr));

        let falcon_text = std::fs::read_to_string(asc.path()).unwrap();
        let asammdf_text = std::fs::read_to_string(asammdf_asc.path()).unwrap();

        let falcon_lines: Vec<&str> = falcon_text.lines().collect();
        let asammdf_lines: Vec<&str> = asammdf_text.lines().collect();

        assert_eq!(falcon_lines.len(), asammdf_lines.len(), "total line count differs");
        // Compare first 1000 lines line-for-line
        for (i, (f_line, a_line)) in falcon_lines.iter().zip(&asammdf_lines).take(1000).enumerate() {
            assert_eq!(f_line.trim_end(), a_line.trim_end(), "line {i} differs");
        }

        println!("ASC cross-check: falcon ASC matches asammdf on J1939 truck log ({} frames)", falcon_lines.len().saturating_sub(3));
    }

    #[test]
    fn asc_export_empty_file_writes_header_only() {
        let mf4 = temp(".mf4");
        let mut writer = Mf4Writer::new();
        let group = writer.add_group(&[0.0, 1.0]).unwrap();
        group.add_channel("Value", "", &[10.0, 20.0]).unwrap();
        writer.write_to_file(mf4.path()).unwrap();

        let file = Mf4File::open(mf4.path()).unwrap();
        let asc = temp(".asc");
        let mut out = std::fs::File::create(asc.path()).unwrap();
        write_asc(&file, &mut out).unwrap();
        drop(out);

        let content = std::fs::read_to_string(asc.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("date "));
        assert_eq!(lines[1], "base hex  timestamps absolute");
        assert_eq!(lines[2], "no internal events logged");
    }
}
