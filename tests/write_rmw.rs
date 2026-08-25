//! Read-Modify-Write (RMW) round-trip conformance tests verified with asammdf.
//!
//! Verifies opening existing MF4 files, modifying metadata (channel names, units, comments),
//! dropping channels, modifying channel data, and writing valid MF4 files back.
//! All outputs are independently verified with `asammdf` at `../../falcon_mdf/.venv/bin/python`.

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::{Conversion, Mf4File, Mf4Writer, SignalValues};

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn python() -> Option<PathBuf> {
    let Some(p) = venv_python() else {
        eprintln!("SKIPPING: no .venv/bin/python beside the crate or at ../../falcon_mdf");
        return None;
    };
    let ok = Command::new(&p)
        .args(["-c", "import asammdf"])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIPPING: asammdf is not installed in {}", p.display());
        return None;
    }
    Some(p)
}

fn json(python: &Path, script: &str) -> serde_json::Value {
    let out = Command::new(python)
        .args(["-c", script])
        .output()
        .expect("failed to launch python");
    assert!(
        out.status.success(),
        "asammdf failed to execute:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout.lines().last().unwrap_or_else(|| {
        panic!(
            "python printed nothing; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    serde_json::from_str(last).unwrap_or_else(|e| panic!("expected JSON, got {last:?}: {e}"))
}

#[test]
fn rmw_modify_channel_metadata_and_verify_with_asammdf() {
    let Some(py) = python() else { return };

    let src_path = "test_data/reference/dSPACE_LinearConversion.mf4";
    let file = Mf4File::open(src_path).expect("failed to open source mf4");

    let mut writer = file.to_writer().expect("to_writer failed");
    assert!(!writer.groups().is_empty());

    let group = &mut writer.groups_mut()[0];
    let ch = group
        .find_channel_mut("Signal_LinearConversion")
        .expect("channel Signal_LinearConversion not found");

    // Modify channel metadata
    ch.set_name("Renamed_Linear_Formula");
    ch.set_unit("km/h");
    ch.set_comment("Renamed channel with custom unit and comment");

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let out_path = tmp_dir.path().join("modified_metadata.mf4");
    writer.write_to_file(&out_path).expect("write_to_file failed");

    // Independent verification with asammdf
    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{src_path}")
mod = MDF(r"{}")

orig_sig = orig.get("Signal_LinearConversion")
mod_sig = mod.get("Renamed_Linear_Formula")

# Assert metadata changes
assert mod_sig.name == "Renamed_Linear_Formula"
assert mod_sig.unit == "km/h"
assert mod_sig.comment == "Renamed channel with custom unit and comment"
assert "Signal_LinearConversion" not in mod.channels_db

# Assert untouched numerical data is identical
np.testing.assert_allclose(mod_sig.timestamps, orig_sig.timestamps, rtol=1e-7, atol=1e-7)
np.testing.assert_allclose(mod_sig.samples, orig_sig.samples, rtol=1e-7, atol=1e-7)

print(json.dumps({{"ok": True}}))
"#,
        out_path.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);

    // Also verify reading back with falcon_mdf
    let readback = Mf4File::open(&out_path).expect("failed to open written file with falcon_mdf");
    let ch_read = readback.channels().find(|c| c.name == "Renamed_Linear_Formula").expect("channel found");
    assert_eq!(ch_read.unit, "km/h");
    assert_eq!(ch_read.comment, "Renamed channel with custom unit and comment");
    assert!(readback.channels().find(|c| c.name == "Signal_LinearConversion").is_none());
}

#[test]
fn rmw_drop_channel_and_verify_with_asammdf() {
    let Some(py) = python() else { return };

    let src_path = "test_data/reference/dSPACE_IntegerTypes.mf4";
    let file = Mf4File::open(src_path).expect("failed to open source mf4");

    let mut writer = file.to_writer().expect("to_writer failed");
    let group = &mut writer.groups_mut()[0];

    let initial_count = group.channels().len();
    assert!(initial_count >= 2);

    let dropped_name = group.channels()[0].name().to_string();
    let kept_name = group.channels()[1].name().to_string();

    let removed = group.remove_channel_by_name(&dropped_name);
    assert!(removed.is_some());
    assert_eq!(group.channels().len(), initial_count - 1);

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let out_path = tmp_dir.path().join("dropped_channel.mf4");
    writer.write_to_file(&out_path).expect("write_to_file failed");

    // Independent verification with asammdf
    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{src_path}")
mod = MDF(r"{}")

assert "{dropped_name}" not in mod.channels_db
assert "{kept_name}" in mod.channels_db

orig_sig = orig.get("{kept_name}")
mod_sig = mod.get("{kept_name}")

np.testing.assert_array_equal(mod_sig.timestamps, orig_sig.timestamps)
np.testing.assert_array_equal(mod_sig.samples, orig_sig.samples)

print(json.dumps({{"ok": True}}))
"#,
        out_path.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);

    // Verify falcon_mdf view
    let readback = Mf4File::open(&out_path).expect("open with falcon_mdf");
    assert!(readback.channels().find(|c| c.name == dropped_name).is_none());
    assert!(readback.channels().find(|c| c.name == kept_name).is_some());
}

#[test]
fn rmw_modify_conversion_and_compress_with_asammdf() {
    let Some(py) = python() else { return };

    let src_path = "test_data/reference/Vector_LinearConversion.mf4";
    let file = Mf4File::open(src_path).expect("failed to open source mf4");

    let mut writer = file.to_writer().expect("to_writer failed");
    writer.set_compression(true); // compress DZ blocks

    let ch_name = {
        let group = &mut writer.groups_mut()[0];
        let ch = &mut group.channels_mut()[0];

        // Change conversion to a new linear conversion: y = 2.5 * x + 10.0
        ch.set_conversion(Some(Conversion::Linear {
            offset: 10.0,
            factor: 2.5,
        }))
        .expect("set_conversion failed");
        ch.name().to_string()
    };

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let out_path = tmp_dir.path().join("compressed_modified_conv.mf4");
    writer.write_to_file(&out_path).expect("write_to_file failed");

    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{src_path}")
mod = MDF(r"{}")

orig_raw = orig.get("{ch_name}", raw=True)
mod_raw = mod.get("{ch_name}", raw=True)
mod_phys = mod.get("{ch_name}", raw=False)

# Raw counts must be untouched
np.testing.assert_array_equal(mod_raw.samples, orig_raw.samples)

# Physical values must follow new conversion: y = 2.5 * x + 10.0
expected_phys = 2.5 * np.asarray(orig_raw.samples, dtype=float) + 10.0
np.testing.assert_allclose(mod_phys.samples, expected_phys, rtol=1e-7, atol=1e-7)

print(json.dumps({{"ok": True}}))
"#,
        out_path.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);
}

#[test]
fn rmw_synthetic_roundtrip_metadata_values_validity() {
    let Some(py) = python() else { return };

    let times = vec![0.0, 0.1, 0.2, 0.3, 0.4];
    let mut initial_writer = Mf4Writer::with_start_time_ns(1_600_000_000_000_000_000);
    let g = initial_writer.add_group(&times).unwrap();
    g.add_channel_full(
        "Sensor_A",
        "bar",
        "Pressure sensor A",
        SignalValues::F64(vec![1.0, 2.0, 3.0, 4.0, 5.0]),
        Some(&[true, true, false, true, true]),
        None,
    )
    .unwrap();
    g.add_channel_full(
        "Counter",
        "counts",
        "Cycle counter",
        SignalValues::U16(vec![10, 20, 30, 40, 50]),
        None,
        Some(Conversion::Linear {
            offset: -5.0,
            factor: 0.5,
        }),
    )
    .unwrap();

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let file1 = tmp_dir.path().join("file1.mf4");
    initial_writer.write_to_file(&file1).unwrap();

    // Read file1 and modify
    let mf4 = Mf4File::open(&file1).unwrap();
    let mut rmw_writer = mf4.to_writer().unwrap();
    rmw_writer.set_start_time_ns(1_700_000_000_000_000_000);

    let grp = &mut rmw_writer.groups_mut()[0];
    let ch_a = grp.find_channel_mut("Sensor_A").unwrap();
    ch_a.set_name("Pressure_Main");
    ch_a.set_unit("kPa");
    ch_a.set_comment("Main line pressure");

    let file2 = tmp_dir.path().join("file2.mf4");
    rmw_writer.write_to_file(&file2).unwrap();

    // Verify with asammdf
    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

m = MDF(r"{}")
assert "Pressure_Main" in m.channels_db
assert "Counter" in m.channels_db
assert "Sensor_A" not in m.channels_db

p_sig = m.get("Pressure_Main")
assert p_sig.unit == "kPa"
assert p_sig.comment == "Main line pressure"
# Note: Sample 2 (t=0.2) had validity=false, so asammdf physical signal filters it out
np.testing.assert_array_equal(p_sig.timestamps, [0.0, 0.1, 0.3, 0.4])

# Raw access with ignore_invalidation_bits includes all 5 timestamps and samples
p_raw = m.get("Pressure_Main", raw=True, ignore_invalidation_bits=True)
np.testing.assert_array_equal(p_raw.timestamps, [0.0, 0.1, 0.2, 0.3, 0.4])
np.testing.assert_array_equal(p_raw.samples, [1.0, 2.0, 3.0, 4.0, 5.0])

c_sig = m.get("Counter", raw=False)
np.testing.assert_allclose(c_sig.samples, [0.0, 5.0, 10.0, 15.0, 20.0])

print(json.dumps({{"ok": True}}))
"#,
        file2.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);
}

#[test]
fn rmw_strings_and_byte_arrays_with_asammdf() {
    let Some(py) = python() else { return };

    let times = vec![0.0, 1.0, 2.0];
    let mut initial_writer = Mf4Writer::new();
    let g = initial_writer.add_group(&times).unwrap();
    g.add_channel_typed("VIN", "", SignalValues::Str(vec!["WBA123".into(), "WBA456".into(), "WBA789".into()])).unwrap();
    g.add_channel_typed("Payload", "", SignalValues::Bytes { data: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66], width: 2 }).unwrap();

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let file1 = tmp_dir.path().join("str_bytes_1.mf4");
    initial_writer.write_to_file(&file1).unwrap();

    let mf4 = Mf4File::open(&file1).unwrap();
    let mut rmw = mf4.to_writer().unwrap();
    let grp = &mut rmw.groups_mut()[0];
    let vin = grp.find_channel_mut("VIN").unwrap();
    vin.set_name("Vehicle_VIN");
    vin.set_comment("Vehicle Identification Number");

    let file2 = tmp_dir.path().join("str_bytes_2.mf4");
    rmw.write_to_file(&file2).unwrap();

    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

m = MDF(r"{}")
assert "Vehicle_VIN" in m.channels_db
assert "Payload" in m.channels_db

vin_sig = m.get("Vehicle_VIN")
assert vin_sig.comment == "Vehicle Identification Number"
v_samples = [s.decode("latin-1").rstrip("\x00") for s in vin_sig.samples]
assert v_samples == ["WBA123", "WBA456", "WBA789"]

p_sig = m.get("Payload")
assert p_sig.samples.tobytes() == b"\x11\x22\x33\x44\x55\x66"

print(json.dumps({{"ok": True}}))
"#,
        file2.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);
}

#[test]
fn rmw_multi_group_file_with_asammdf() {
    let Some(py) = python() else { return };

    let mut writer = Mf4Writer::new();
    let g1 = writer.add_group(&[0.0, 0.1, 0.2]).unwrap();
    g1.add_channel("G1_Ch1", "V", &[10.0, 11.0, 12.0]).unwrap();
    g1.add_channel("G1_Ch2", "A", &[1.0, 2.0, 3.0]).unwrap();

    let g2 = writer.add_group(&[0.0, 0.5]).unwrap();
    g2.add_channel("G2_Ch1", "degC", &[25.0, 26.0]).unwrap();

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let file1 = tmp_dir.path().join("mg1.mf4");
    writer.write_to_file(&file1).unwrap();

    // RMW: open, drop G1_Ch2, rename G2_Ch1 -> Temp_Ambient
    let mf4 = Mf4File::open(&file1).unwrap();
    let mut rmw = mf4.to_writer().unwrap();
    assert_eq!(rmw.groups().len(), 2);

    rmw.groups_mut()[0].remove_channel_by_name("G1_Ch2").unwrap();
    let temp_ch = rmw.groups_mut()[1].find_channel_mut("G2_Ch1").unwrap();
    temp_ch.set_name("Temp_Ambient");
    temp_ch.set_unit("C");

    let file2 = tmp_dir.path().join("mg2.mf4");
    rmw.write_to_file(&file2).unwrap();

    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

m = MDF(r"{}")
assert len(m.groups) == 2
assert "G1_Ch1" in m.channels_db
assert "G1_Ch2" not in m.channels_db
assert "Temp_Ambient" in m.channels_db
assert "G2_Ch1" not in m.channels_db

g1_sig = m.get("G1_Ch1")
np.testing.assert_allclose(g1_sig.samples, [10.0, 11.0, 12.0])

t_sig = m.get("Temp_Ambient")
assert t_sig.unit == "C"
np.testing.assert_allclose(t_sig.samples, [25.0, 26.0])

print(json.dumps({{"ok": True}}))
"#,
        file2.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);
}
