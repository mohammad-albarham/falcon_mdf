//! Tests for writing multiple channel groups sharing a single data group.
//!
//! Verifies that:
//! 1. Three channel groups with different record sizes and different sample counts
//!    on interleaving time axes round-trip through `Mf4File::open` with all channel
//!    names, sample counts, and sample values matching.
//! 2. The on-disk `dg_rec_id_size` byte is 1 and the `##CG` blocks carry record IDs
//!    1, 2, and 3 with `cg_next` correctly linking them.
//! 3. The existing `add_group` path continues to write `dg_rec_id_size = 0` and
//!    `record_id = 0` (no regression on sorted single-CG layout).
//! 4. `add_group_in` refuses out-of-range sibling indices and NaN timestamps.
//! 5. Compressed (deflated) multi-CG files round-trip accurately.
//! 6. Channels with invalidation bits across multi-CG groups preserve validity masks.
//! 7. Cross-checks against `asammdf` when available.

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::{Mf4Error, Mf4File, Mf4Writer, SignalValues};

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
fn three_channel_groups_in_one_data_group_roundtrip_all_samples() {
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);

    // Group 1: 7 samples, record size: master(8) + f32(4) + u8(1) = 13 bytes
    let times_g1 = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let temp_vals: Vec<f32> = vec![20.5, 21.0, 21.5, 22.0, 22.5, 23.0, 23.5];
    let status_vals: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7];

    let g1 = writer.add_group(&times_g1).unwrap();
    g1.add_channel_typed("Temperature", "degC", SignalValues::F32(temp_vals.clone()))
        .unwrap();
    g1.add_channel_typed("StatusFlag", "", SignalValues::U8(status_vals.clone()))
        .unwrap();

    // Group 2: 5 samples, record size: master(8) + u32(4) + f64(8) + str(7) = 27 bytes
    let times_g2 = vec![0.5, 1.5, 2.5, 3.5, 4.5];
    let rpm_vals: Vec<u32> = vec![800, 1200, 1500, 2000, 2500];
    let torque_vals: Vec<f64> = vec![100.25, 150.5, 200.75, 250.0, 300.25];
    let gear_vals: Vec<String> = vec![
        "PARK".into(),
        "DRIVE".into(),
        "NEUTRAL".into(),
        "REVERSE".into(),
        "SPORT".into(),
    ];

    let g2 = writer.add_group_in(0, &times_g2).unwrap();
    g2.add_channel_typed("EngineSpeed", "rpm", SignalValues::U32(rpm_vals.clone()))
        .unwrap();
    g2.add_channel_typed("Torque", "Nm", SignalValues::F64(torque_vals.clone()))
        .unwrap();
    g2.add_channel_typed("GearName", "", SignalValues::Str(gear_vals.clone()))
        .unwrap();

    // Group 3: 10 samples, record size: master(8) + i16(2) + i64(8) + bytes(3) = 21 bytes
    let times_g3 = vec![0.2, 0.8, 1.2, 1.8, 2.2, 2.8, 3.2, 3.8, 4.2, 4.8];
    let count_vals: Vec<i16> = vec![-5, -4, -3, -2, -1, 0, 1, 2, 3, 4];
    let current_vals: Vec<i64> = vec![1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000];
    let raw_payload_bytes: Vec<u8> = (0..30).map(|i| (i * 7 + 3) as u8).collect();

    let g3 = writer.add_group_in(0, &times_g3).unwrap();
    g3.add_channel_typed("Counter", "", SignalValues::I16(count_vals.clone()))
        .unwrap();
    g3.add_channel_typed("Current", "mA", SignalValues::I64(current_vals.clone()))
        .unwrap();
    g3.add_channel_typed(
        "RawPayload",
        "",
        SignalValues::Bytes {
            data: raw_payload_bytes.clone(),
            width: 3,
        },
    )
    .unwrap();

    // Write to a temporary file
    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    // Re-open with Mf4File::open
    let file = Mf4File::open(temp.path()).unwrap();

    // Assert overall structure: exactly 1 data group containing 3 channel groups
    assert_eq!(file.data_groups().len(), 1);
    let dg = &file.data_groups()[0];
    assert_eq!(dg.channel_groups.len(), 3);
    assert_eq!(dg.rec_id_size(), 1);

    // Group 0 assertions (Group 1 in writer)
    let cg0 = &dg.channel_groups[0];
    assert_eq!(cg0.record_id(), 1);
    assert_eq!(cg0.sample_count, 7);
    let cg0_names: Vec<&str> = cg0.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cg0_names, vec!["Time", "Temperature", "StatusFlag"]);

    let master0 = cg0.channels.iter().find(|c| c.is_master()).unwrap();
    let sig_t0 = file.signal(master0).unwrap();
    assert_eq!(sig_t0.values_f64().unwrap(), times_g1);

    let sig_temp = file.signal(&cg0.channels[1]).unwrap();
    assert_eq!(
        sig_temp.raw_values().unwrap(),
        SignalValues::F32(temp_vals.clone())
    );
    assert_eq!(sig_temp.len(), 7);

    let sig_status = file.signal(&cg0.channels[2]).unwrap();
    assert_eq!(
        sig_status.raw_values().unwrap(),
        SignalValues::U8(status_vals.clone())
    );
    assert_eq!(sig_status.len(), 7);

    // Group 1 assertions (Group 2 in writer)
    let cg1 = &dg.channel_groups[1];
    assert_eq!(cg1.record_id(), 2);
    assert_eq!(cg1.sample_count, 5);
    let cg1_names: Vec<&str> = cg1.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cg1_names, vec!["Time", "EngineSpeed", "Torque", "GearName"]);

    let master1 = cg1.channels.iter().find(|c| c.is_master()).unwrap();
    let sig_t1 = file.signal(master1).unwrap();
    assert_eq!(sig_t1.values_f64().unwrap(), times_g2);

    let sig_rpm = file.signal(&cg1.channels[1]).unwrap();
    assert_eq!(
        sig_rpm.raw_values().unwrap(),
        SignalValues::U32(rpm_vals.clone())
    );
    assert_eq!(sig_rpm.len(), 5);

    let sig_torque = file.signal(&cg1.channels[2]).unwrap();
    assert_eq!(
        sig_torque.raw_values().unwrap(),
        SignalValues::F64(torque_vals.clone())
    );
    assert_eq!(sig_torque.len(), 5);

    let sig_gear = file.signal(&cg1.channels[3]).unwrap();
    assert_eq!(
        sig_gear.raw_values().unwrap(),
        SignalValues::Str(gear_vals.clone())
    );
    assert_eq!(sig_gear.len(), 5);

    // Group 2 assertions (Group 3 in writer)
    let cg2 = &dg.channel_groups[2];
    assert_eq!(cg2.record_id(), 3);
    assert_eq!(cg2.sample_count, 10);
    let cg2_names: Vec<&str> = cg2.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cg2_names, vec!["Time", "Counter", "Current", "RawPayload"]);

    let master2 = cg2.channels.iter().find(|c| c.is_master()).unwrap();
    let sig_t2 = file.signal(master2).unwrap();
    assert_eq!(sig_t2.values_f64().unwrap(), times_g3);

    let sig_count = file.signal(&cg2.channels[1]).unwrap();
    assert_eq!(
        sig_count.raw_values().unwrap(),
        SignalValues::I16(count_vals.clone())
    );
    assert_eq!(sig_count.len(), 10);

    let sig_current = file.signal(&cg2.channels[2]).unwrap();
    assert_eq!(
        sig_current.raw_values().unwrap(),
        SignalValues::I64(current_vals.clone())
    );
    assert_eq!(sig_current.len(), 10);

    let sig_bytes = file.signal(&cg2.channels[3]).unwrap();
    assert_eq!(
        sig_bytes.raw_values().unwrap(),
        SignalValues::Bytes {
            data: raw_payload_bytes.clone(),
            width: 3,
        }
    );
    assert_eq!(sig_bytes.len(), 10);
}

#[test]
fn raw_bytes_match_dg_rec_id_size_and_cg_record_ids() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let g1 = writer.add_group(&[0.0, 1.0]).unwrap();
    g1.add_channel("A", "", &[10.0, 20.0]).unwrap();

    let g2 = writer.add_group_in(0, &[0.5, 1.5, 2.5]).unwrap();
    g2.add_channel("B", "", &[100.0, 200.0, 300.0]).unwrap();

    let g3 = writer.add_group_in(0, &[0.2]).unwrap();
    g3.add_channel("C", "", &[999.0]).unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();

    // Verify identification block
    assert_eq!(&bytes[0..8], b"MDF     ");
    assert_eq!(&bytes[8..16], b"4.11    ");

    // Header block at offset 64
    assert_eq!(&bytes[64..68], b"##HD");
    let dg_off = u64::from_le_bytes(bytes[88..96].try_into().unwrap()) as usize;

    // Data group block
    assert_eq!(&bytes[dg_off..dg_off + 4], b"##DG");
    let dg_next = u64::from_le_bytes(bytes[dg_off + 24..dg_off + 32].try_into().unwrap());
    assert_eq!(dg_next, 0); // Only 1 DG

    let cg1_off = u64::from_le_bytes(bytes[dg_off + 32..dg_off + 40].try_into().unwrap()) as usize;
    let rec_id_size = bytes[dg_off + 56];
    assert_eq!(rec_id_size, 1, "dg_rec_id_size must be 1 for multi-CG");

    // CG 1
    assert_eq!(&bytes[cg1_off..cg1_off + 4], b"##CG");
    let cg2_off =
        u64::from_le_bytes(bytes[cg1_off + 24..cg1_off + 32].try_into().unwrap()) as usize;
    assert_ne!(cg2_off, 0, "cg1_next must point to CG 2");
    let cg1_rec_id = u64::from_le_bytes(bytes[cg1_off + 72..cg1_off + 80].try_into().unwrap());
    assert_eq!(cg1_rec_id, 1, "CG 1 record_id must be 1");

    // CG 2
    assert_eq!(&bytes[cg2_off..cg2_off + 4], b"##CG");
    let cg3_off =
        u64::from_le_bytes(bytes[cg2_off + 24..cg2_off + 32].try_into().unwrap()) as usize;
    assert_ne!(cg3_off, 0, "cg2_next must point to CG 3");
    let cg2_rec_id = u64::from_le_bytes(bytes[cg2_off + 72..cg2_off + 80].try_into().unwrap());
    assert_eq!(cg2_rec_id, 2, "CG 2 record_id must be 2");

    // CG 3
    assert_eq!(&bytes[cg3_off..cg3_off + 4], b"##CG");
    let cg3_next = u64::from_le_bytes(bytes[cg3_off + 24..cg3_off + 32].try_into().unwrap());
    assert_eq!(cg3_next, 0, "cg3_next must be 0 (last CG in DG)");
    let cg3_rec_id = u64::from_le_bytes(bytes[cg3_off + 72..cg3_off + 80].try_into().unwrap());
    assert_eq!(cg3_rec_id, 3, "CG 3 record_id must be 3");
}

#[test]
fn single_group_and_multi_dg_sorted_layout_rec_id_size_is_zero() {
    // 1 group via add_group: must have dg_rec_id_size = 0 and record_id = 0
    let mut writer1 = Mf4Writer::with_start_time_ns(0);
    let g = writer1.add_group(&[0.0, 1.0]).unwrap();
    g.add_channel("Speed", "km/h", &[10.0, 20.0]).unwrap();

    let mut bytes1 = Vec::new();
    writer1.write(&mut bytes1).unwrap();

    let dg_off1 = u64::from_le_bytes(bytes1[88..96].try_into().unwrap()) as usize;
    assert_eq!(&bytes1[dg_off1..dg_off1 + 4], b"##DG");
    assert_eq!(
        bytes1[dg_off1 + 56],
        0,
        "single-CG dg_rec_id_size must remain 0"
    );

    let cg_off1 =
        u64::from_le_bytes(bytes1[dg_off1 + 32..dg_off1 + 40].try_into().unwrap()) as usize;
    assert_eq!(&bytes1[cg_off1..cg_off1 + 4], b"##CG");
    let cg1_rec_id = u64::from_le_bytes(bytes1[cg_off1 + 72..cg_off1 + 80].try_into().unwrap());
    assert_eq!(cg1_rec_id, 0, "single-CG record_id must be 0");
    let cg1_next = u64::from_le_bytes(bytes1[cg_off1 + 24..cg_off1 + 32].try_into().unwrap());
    assert_eq!(cg1_next, 0, "single-CG cg_next must be 0");

    // 2 groups via add_group (2 separate DGs): each must have dg_rec_id_size = 0
    let mut writer2 = Mf4Writer::with_start_time_ns(0);
    writer2
        .add_group(&[0.0, 1.0])
        .unwrap()
        .add_channel("A", "", &[1.0, 2.0])
        .unwrap();
    writer2
        .add_group(&[0.0, 0.5])
        .unwrap()
        .add_channel("B", "", &[3.0, 4.0])
        .unwrap();

    let mut bytes2 = Vec::new();
    writer2.write(&mut bytes2).unwrap();

    let dg1_off = u64::from_le_bytes(bytes2[88..96].try_into().unwrap()) as usize;
    assert_eq!(bytes2[dg1_off + 56], 0, "DG1 dg_rec_id_size must be 0");
    let dg2_off =
        u64::from_le_bytes(bytes2[dg1_off + 24..dg1_off + 32].try_into().unwrap()) as usize;
    assert_ne!(dg2_off, 0);
    assert_eq!(bytes2[dg2_off + 56], 0, "DG2 dg_rec_id_size must be 0");

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer2.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    assert_eq!(file.data_groups().len(), 2);
    assert_eq!(file.data_groups()[0].rec_id_size(), 0);
    assert_eq!(file.data_groups()[1].rec_id_size(), 0);
}

#[test]
fn add_group_in_refuses_out_of_range_sibling_and_nan() {
    let mut writer = Mf4Writer::new();

    // Out of range on empty writer
    let err = writer.add_group_in(0, &[0.0, 1.0]).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
    assert!(
        err.to_string().contains("out of range"),
        "error message should explain out of range: {err}"
    );

    // Add 1 valid group
    writer.add_group(&[0.0, 1.0]).unwrap();

    // Out of range index 1 (valid index is only 0)
    let err = writer.add_group_in(1, &[0.0, 1.0]).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
    assert!(
        err.to_string().contains("out of range"),
        "error message should explain out of range: {err}"
    );

    // NaN timestamp
    let err = writer.add_group_in(0, &[0.0, f64::NAN]).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
    assert!(
        err.to_string().contains("NaN"),
        "error message should mention NaN: {err}"
    );
}

#[test]
fn three_channel_groups_compressed_deflated_roundtrip() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    writer.set_compression(true);

    let g1 = writer.add_group(&[0.0, 1.0, 2.0]).unwrap();
    g1.add_channel("G1_Val", "V", &[1.1, 2.2, 3.3]).unwrap();

    let g2 = writer.add_group_in(0, &[0.5, 1.5]).unwrap();
    g2.add_channel("G2_Val", "A", &[10.0, 20.0]).unwrap();

    let g3 = writer.add_group_in(0, &[0.25, 0.75, 1.25, 1.75]).unwrap();
    g3.add_channel("G3_Val", "rpm", &[100.0, 200.0, 300.0, 400.0])
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    let file = Mf4File::open(temp.path()).unwrap();
    assert_eq!(file.data_groups().len(), 1);
    assert_eq!(file.data_groups()[0].channel_groups.len(), 3);

    let ch1 = file.find_channel("G1_Val").unwrap();
    assert_eq!(
        file.signal(ch1).unwrap().values_f64().unwrap(),
        vec![1.1, 2.2, 3.3]
    );

    let ch2 = file.find_channel("G2_Val").unwrap();
    assert_eq!(
        file.signal(ch2).unwrap().values_f64().unwrap(),
        vec![10.0, 20.0]
    );

    let ch3 = file.find_channel("G3_Val").unwrap();
    assert_eq!(
        file.signal(ch3).unwrap().values_f64().unwrap(),
        vec![100.0, 200.0, 300.0, 400.0]
    );
}

#[test]
fn three_channel_groups_with_validity_masks() {
    let mut writer = Mf4Writer::with_start_time_ns(0);

    let g1 = writer.add_group(&[0.0, 1.0, 2.0, 3.0]).unwrap();
    g1.add_channel_with_validity(
        "G1_Valid",
        "",
        &[1.0, 2.0, 3.0, 4.0],
        Some(&[true, false, true, true]),
    )
    .unwrap();

    let g2 = writer.add_group_in(0, &[0.5, 1.5, 2.5]).unwrap();
    g2.add_channel_with_validity(
        "G2_Valid",
        "",
        &[10.0, 20.0, 30.0],
        Some(&[true, true, false]),
    )
    .unwrap();

    let g3 = writer.add_group_in(0, &[0.2, 1.2]).unwrap();
    g3.add_channel("G3_Plain", "", &[100.0, 200.0]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    let file = Mf4File::open(temp.path()).unwrap();
    assert_eq!(file.data_groups().len(), 1);
    assert_eq!(file.data_groups()[0].channel_groups.len(), 3);

    let sig1 = file.signal(file.find_channel("G1_Valid").unwrap()).unwrap();
    assert_eq!(sig1.validity(), Some(vec![true, false, true, true]));
    assert_eq!(sig1.values_f64().unwrap(), vec![1.0, 2.0, 3.0, 4.0]);

    let sig2 = file.signal(file.find_channel("G2_Valid").unwrap()).unwrap();
    assert_eq!(sig2.validity(), Some(vec![true, true, false]));
    assert_eq!(sig2.values_f64().unwrap(), vec![10.0, 20.0, 30.0]);

    let sig3 = file.signal(file.find_channel("G3_Plain").unwrap()).unwrap();
    assert_eq!(sig3.validity(), None);
    assert_eq!(sig3.values_f64().unwrap(), vec![100.0, 200.0]);
}

#[test]
fn asammdf_reads_multi_cg_in_single_dg() {
    let Some(python) = venv_python() else {
        eprintln!("skipping asammdf cross-check: python venv not found");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping asammdf cross-check: asammdf not installed");
        return;
    }

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let g1 = writer.add_group(&[0.0, 1.0, 2.0]).unwrap();
    g1.add_channel("SigA", "degC", &[10.0, 20.0, 30.0]).unwrap();

    let g2 = writer.add_group_in(0, &[0.5, 1.5]).unwrap();
    g2.add_channel("SigB", "rpm", &[100.0, 200.0]).unwrap();

    let g3 = writer.add_group_in(0, &[0.2, 0.8, 1.2, 1.8]).unwrap();
    g3.add_channel("SigC", "V", &[1.5, 2.5, 3.5, 4.5]).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    let py_script = r#"
import sys
import asammdf
import numpy as np

mdf = asammdf.MDF(sys.argv[1])
assert len(mdf.groups) == 3, f"expected 3 channel groups, got {len(mdf.groups)}"

siga = mdf.get("SigA")
np.testing.assert_allclose(siga.timestamps, [0.0, 1.0, 2.0])
np.testing.assert_allclose(siga.samples, [10.0, 20.0, 30.0])

sigb = mdf.get("SigB")
np.testing.assert_allclose(sigb.timestamps, [0.5, 1.5])
np.testing.assert_allclose(sigb.samples, [100.0, 200.0])

sigc = mdf.get("SigC")
np.testing.assert_allclose(sigc.timestamps, [0.2, 0.8, 1.2, 1.8])
np.testing.assert_allclose(sigc.samples, [1.5, 2.5, 3.5, 4.5])

print("asammdf verification succeeded")
"#;

    let output = Command::new(&python)
        .args(["-c", py_script, temp.path().to_str().unwrap()])
        .output()
        .unwrap();

    if !output.status.success() {
        panic!(
            "asammdf failed to verify multi-CG file:\nSTDOUT:\n{}\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
