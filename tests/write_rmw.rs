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

fn resolve_path(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.exists() {
        return Some(p.to_path_buf());
    }
    let parent = Path::new("../../falcon_mdf").join(rel);
    if parent.exists() {
        return Some(parent);
    }
    None
}

#[test]
fn rmw_modify_channel_metadata_and_verify_with_asammdf() {
    let Some(py) = python() else { return };

    let Some(src_path) = resolve_path("test_data/reference/dSPACE_LinearConversion.mf4") else {
        eprintln!("SKIPPING: dSPACE_LinearConversion.mf4 not found");
        return;
    };
    let file = Mf4File::open(&src_path).expect("failed to open source mf4");

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
    writer
        .write_to_file(&out_path)
        .expect("write_to_file failed");

    // Independent verification with asammdf
    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{}")
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
        src_path.display(),
        out_path.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);

    // Also verify reading back with falcon_mdf
    let readback = Mf4File::open(&out_path).expect("failed to open written file with falcon_mdf");
    let ch_read = readback
        .channels()
        .find(|c| c.name == "Renamed_Linear_Formula")
        .expect("channel found");
    assert_eq!(ch_read.unit, "km/h");
    assert_eq!(
        ch_read.comment,
        "Renamed channel with custom unit and comment"
    );
    assert!(readback
        .channels()
        .find(|c| c.name == "Signal_LinearConversion")
        .is_none());
}

#[test]
fn rmw_drop_channel_and_verify_with_asammdf() {
    let Some(py) = python() else { return };

    let Some(src_path) = resolve_path("test_data/reference/dSPACE_IntegerTypes.mf4") else {
        eprintln!("SKIPPING: dSPACE_IntegerTypes.mf4 not found");
        return;
    };
    let file = Mf4File::open(&src_path).expect("failed to open source mf4");

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
    writer
        .write_to_file(&out_path)
        .expect("write_to_file failed");

    // Independent verification with asammdf
    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{}")
mod = MDF(r"{}")

assert "{dropped_name}" not in mod.channels_db
assert "{kept_name}" in mod.channels_db

orig_sig = orig.get("{kept_name}")
mod_sig = mod.get("{kept_name}")

np.testing.assert_array_equal(mod_sig.timestamps, orig_sig.timestamps)
np.testing.assert_array_equal(mod_sig.samples, orig_sig.samples)

print(json.dumps({{"ok": True}}))
"#,
        src_path.display(),
        out_path.display()
    );

    let res = json(&py, &script);
    assert_eq!(res["ok"], true);

    // Verify falcon_mdf view
    let readback = Mf4File::open(&out_path).expect("open with falcon_mdf");
    assert!(readback
        .channels()
        .find(|c| c.name == dropped_name)
        .is_none());
    assert!(readback.channels().find(|c| c.name == kept_name).is_some());
}

#[test]
fn rmw_modify_conversion_and_compress_with_asammdf() {
    let Some(py) = python() else { return };

    let Some(src_path) = resolve_path("test_data/reference/Vector_LinearConversion.mf4") else {
        eprintln!("SKIPPING: Vector_LinearConversion.mf4 not found");
        return;
    };
    let file = Mf4File::open(&src_path).expect("failed to open source mf4");

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
    writer
        .write_to_file(&out_path)
        .expect("write_to_file failed");

    let script = format!(
        r#"
import json
import numpy as np
from asammdf import MDF

orig = MDF(r"{}")
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
        src_path.display(),
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
    g.add_channel_typed(
        "VIN",
        "",
        SignalValues::Str(vec!["WBA123".into(), "WBA456".into(), "WBA789".into()]),
    )
    .unwrap();
    g.add_channel_typed(
        "Payload",
        "",
        SignalValues::Bytes {
            data: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            width: 2,
        },
    )
    .unwrap();

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

    rmw.groups_mut()[0]
        .remove_channel_by_name("G1_Ch2")
        .unwrap();
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

#[test]
fn strict_rewrite_succeeds_on_lossless_files_and_skips_vlsd_companion_cgs() {
    use falcon_mdf::blocks::ChannelType;
    use falcon_mdf::write::RewriteMode;

    // 1. A standard Mf4Writer file with typed channels, linear conversion,
    //    validity bits, VLSD strings, and an array channel must rewrite losslessly.
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    let g = writer.add_group(&[0.0, 0.5, 1.0]).unwrap();
    g.add_channel_typed_with(
        "Count",
        "rpm",
        SignalValues::U16(vec![10, 20, 30]),
        Some(&[true, false, true]),
        Some(Conversion::Linear {
            offset: 5.0,
            factor: 2.0,
        }),
    )
    .unwrap();
    g.add_channel_vlsd_str("Msg", "", &["hi", "there", "!"])
        .unwrap();
    g.add_channel_array(
        "Vec",
        "m",
        &[2],
        SignalValues::Array {
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            elements_per_sample: 2,
        },
    )
    .unwrap();

    let tmp1 = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(tmp1.path()).unwrap();
    let file1 = Mf4File::open(tmp1.path()).unwrap();

    let (rmw, report) = Mf4Writer::from_file_with_report(&file1, RewriteMode::Strict).unwrap();
    assert!(
        report.is_lossless(),
        "unexpected issues: {:?}",
        report.issues
    );
    assert!(
        !report.notes.is_empty(),
        "expected informational notes for ##FH and master channel"
    );

    let tmp2 = tempfile::NamedTempFile::new().unwrap();
    rmw.write_to_file(tmp2.path()).unwrap();
    let file2 = Mf4File::open(tmp2.path()).unwrap();
    assert_eq!(
        file2
            .signal(file2.find_channel("Count").unwrap())
            .unwrap()
            .values_f64()
            .unwrap(),
        vec![25.0, 45.0, 65.0]
    );
    assert_eq!(
        file2
            .signal(file2.find_channel("Count").unwrap())
            .unwrap()
            .validity(),
        Some(vec![true, false, true])
    );

    // 2. Build a synthetic file containing a CG-stored VLSD companion group (cg_flags bit 0)
    //    alongside a regular Time-mastered CG, and verify Strict mode skips the companion CG
    //    instead of creating a bogus masterless group.
    fn synth_block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
        let len = (24 + links.len() * 8 + data.len()) as u64;
        let mut out = vec![0u8; 24];
        out[0..4].copy_from_slice(id);
        out[8..16].copy_from_slice(&len.to_le_bytes());
        out[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());
        for l in links {
            out.extend_from_slice(&l.to_le_bytes());
        }
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        out
    }
    fn synth_tx(s: &str) -> Vec<u8> {
        let mut data = s.as_bytes().to_vec();
        data.push(0);
        while !data.len().is_multiple_of(8) {
            data.push(0);
        }
        synth_block(b"##TX", &[], &data)
    }

    let mut bytes = vec![0u8; 64];
    bytes[0..8].copy_from_slice(b"MDF     ");
    bytes[8..16].copy_from_slice(b"4.11    ");
    bytes[16..24].copy_from_slice(b"synth   ");
    bytes[28..30].copy_from_slice(&411u16.to_le_bytes());

    let push = |b: &mut Vec<u8>, blk: &[u8]| -> u64 {
        let off = b.len() as u64;
        b.extend_from_slice(blk);
        off
    };

    let hd_off = push(&mut bytes, &synth_block(b"##HD", &[0; 6], &[0u8; 32]));
    let fh_md = push(
        &mut bytes,
        &synth_tx("<FHcomment><TX>init</TX><tool_id>t</tool_id></FHcomment>"),
    );
    let fh_off = push(&mut bytes, &synth_block(b"##FH", &[0, fh_md], &[0u8; 16]));

    let tx_time = push(&mut bytes, &synth_tx("Time"));
    let tx_sec = push(&mut bytes, &synth_tx("s"));
    let tx_vlsd = push(&mut bytes, &synth_tx("Note"));

    let mut rec_bytes = Vec::new();
    // Sample 0 of CG 1 (record_id = 1): time=0.0, offset=0
    rec_bytes.push(1u8);
    rec_bytes.extend_from_slice(&0.0f64.to_le_bytes());
    rec_bytes.extend_from_slice(&0u64.to_le_bytes());
    // VLSD payload 0 (record_id = 2): len=5, "hello"
    rec_bytes.push(2u8);
    rec_bytes.extend_from_slice(&5u32.to_le_bytes());
    rec_bytes.extend_from_slice(b"hello");
    // Sample 1 of CG 1 (record_id = 1): time=1.0, offset=9
    rec_bytes.push(1u8);
    rec_bytes.extend_from_slice(&1.0f64.to_le_bytes());
    rec_bytes.extend_from_slice(&9u64.to_le_bytes());
    // VLSD payload 1 (record_id = 2): len=5, "world"
    rec_bytes.push(2u8);
    rec_bytes.extend_from_slice(&5u32.to_le_bytes());
    rec_bytes.extend_from_slice(b"world");

    let dt_off = push(&mut bytes, &synth_block(b"##DT", &[], &rec_bytes));

    // Companion VLSD CG (cg_flags = 1 at data[16..18], record_id = 2)
    let mut cg_vlsd_data = vec![0u8; 32];
    cg_vlsd_data[0..8].copy_from_slice(&2u64.to_le_bytes());
    cg_vlsd_data[8..16].copy_from_slice(&2u64.to_le_bytes());
    cg_vlsd_data[16..18].copy_from_slice(&1u16.to_le_bytes());
    let cg_vlsd_off = push(&mut bytes, &synth_block(b"##CG", &[0; 6], &cg_vlsd_data));

    // CN for VLSD channel linking cn_data -> cg_vlsd_off
    let mut cn_vlsd_data = vec![0u8; 72];
    cn_vlsd_data[0] = 1; // VariableLength
    cn_vlsd_data[2] = 7; // UTF-8
    cn_vlsd_data[4..8].copy_from_slice(&8u32.to_le_bytes());
    cn_vlsd_data[8..12].copy_from_slice(&64u32.to_le_bytes());
    let cn_vlsd_off = push(
        &mut bytes,
        &synth_block(
            b"##CN",
            &[0, 0, tx_vlsd, 0, 0, cg_vlsd_off, 0, 0],
            &cn_vlsd_data,
        ),
    );

    // Master CN
    let mut cn_time_data = vec![0u8; 72];
    cn_time_data[0] = 2; // Master
    cn_time_data[1] = 1; // Time
    cn_time_data[2] = 4; // FloatLe
    cn_time_data[8..12].copy_from_slice(&64u32.to_le_bytes());
    let cn_time_off = push(
        &mut bytes,
        &synth_block(
            b"##CN",
            &[cn_vlsd_off, 0, tx_time, 0, 0, 0, tx_sec, 0],
            &cn_time_data,
        ),
    );

    // Main CG (record_id = 1, cg_next = cg_vlsd_off)
    let mut cg_main_data = vec![0u8; 32];
    cg_main_data[0..8].copy_from_slice(&1u64.to_le_bytes());
    cg_main_data[8..16].copy_from_slice(&2u64.to_le_bytes());
    cg_main_data[24..28].copy_from_slice(&16u32.to_le_bytes());
    let cg_main_off = push(
        &mut bytes,
        &synth_block(
            b"##CG",
            &[cg_vlsd_off, cn_time_off, 0, 0, 0, 0],
            &cg_main_data,
        ),
    );

    let mut dg_data = vec![0u8; 8];
    dg_data[0] = 1; // rec_id_size = 1
    let dg_off = push(
        &mut bytes,
        &synth_block(b"##DG", &[0, cg_main_off, dt_off, 0], &dg_data),
    );

    // Patch HD dg_first and fh_first
    bytes[hd_off as usize + 24..hd_off as usize + 32].copy_from_slice(&dg_off.to_le_bytes());
    bytes[hd_off as usize + 32..hd_off as usize + 40].copy_from_slice(&fh_off.to_le_bytes());

    let synth_tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(synth_tmp.path(), &bytes).unwrap();
    let synth_file = Mf4File::open(synth_tmp.path()).unwrap();
    assert_eq!(synth_file.data_groups()[0].channel_groups.len(), 2);
    assert!(synth_file.data_groups()[0].channel_groups[1].is_vlsd());

    let (synth_writer, synth_report) =
        Mf4Writer::from_file_with_report(&synth_file, RewriteMode::Strict).unwrap();
    assert!(
        synth_report.is_lossless(),
        "issues: {:?}",
        synth_report.issues
    );
    assert_eq!(
        synth_writer.groups().len(),
        1,
        "companion VLSD CG must be skipped rather than creating a bogus masterless group"
    );
    let out_tmp = tempfile::NamedTempFile::new().unwrap();
    synth_writer.write_to_file(out_tmp.path()).unwrap();
    let reopened = Mf4File::open(out_tmp.path()).unwrap();
    let note_ch = reopened.find_channel("Note").unwrap();
    assert_eq!(note_ch.channel_type, ChannelType::VariableLength);
    assert_eq!(
        reopened.signal(note_ch).unwrap().values().unwrap(),
        SignalValues::Str(vec!["hello".into(), "world".into()])
    );
}

#[test]
fn strict_rewrite_rejects_genuinely_lossy_files() {
    use falcon_mdf::write::RewriteMode;
    use falcon_mdf::Mf4Error;

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let g = writer.add_group(&[0.0, 1.0]).unwrap();
    g.add_channel("X", "V", &[10.0, 20.0]).unwrap();

    let mut base_bytes = Vec::new();
    writer.write(&mut base_bytes).unwrap();

    let open_and_check = |bytes: &[u8], expected_substr: &str| {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bytes).unwrap();
        let file = Mf4File::open(tmp.path()).unwrap();
        let (_, report) = Mf4Writer::from_file_with_report(&file, RewriteMode::BestEffort).unwrap();
        assert!(!report.is_lossless());
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.reason.contains(expected_substr)),
            "expected issue containing '{expected_substr}', got {:?}",
            report.issues
        );
        let err = Mf4Writer::from_file_with_report(&file, RewriteMode::Strict).unwrap_err();
        assert!(matches!(err, Mf4Error::WriteError { .. }));
    };

    // (a) Patch HD.md_comment (link 5 at offset 64 + 24 + 5*8 = 128) to point to a ##TX block.
    let mut with_hd_comment = base_bytes.clone();
    let tx_off = with_hd_comment.len() as u64;
    let mut tx_blk = vec![0u8; 32];
    tx_blk[0..4].copy_from_slice(b"##TX");
    tx_blk[8..16].copy_from_slice(&32u64.to_le_bytes());
    tx_blk[24..28].copy_from_slice(b"note");
    with_hd_comment.extend_from_slice(&tx_blk);
    with_hd_comment[128..136].copy_from_slice(&tx_off.to_le_bytes());
    open_and_check(&with_hd_comment, "editable writer");

    // (b) Turn the master CN into a plain value channel (channel_type = 0, sync_type = 0)
    //     so the CG becomes masterless.
    let dg_off = u64::from_le_bytes(base_bytes[88..96].try_into().unwrap()) as usize;
    let cg_off =
        u64::from_le_bytes(base_bytes[dg_off + 32..dg_off + 40].try_into().unwrap()) as usize;
    let master_cn_off =
        u64::from_le_bytes(base_bytes[cg_off + 32..cg_off + 40].try_into().unwrap()) as usize;

    let mut masterless = base_bytes.clone();
    masterless[master_cn_off + 88] = 0; // cn_type = FixedLength (no longer Master)
    masterless[master_cn_off + 89] = 0; // cn_sync_type = None
    open_and_check(&masterless, "masterless");

    // (c) Change master cn_sync_type to 2 (Angle) so rewriting as Time [s] is lossy.
    let mut angle_master = base_bytes.clone();
    angle_master[master_cn_off + 89] = 2; // SyncType::Angle
    open_and_check(&angle_master, "synchronization");
}
