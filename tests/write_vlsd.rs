//! Tests for variable-length (VLSD) channel writing.
//!
//! Verifies that:
//! 1. The on-disk bytes of ##SD blocks match hand-written expected binary sequences.
//! 2. Variable-length string and byte channels with uneven sample lengths (e.g. 0 to 300+ bytes)
//!    round-trip accurately through falcon_mdf.
//! 3. asammdf reads the generated files and decodes the exact sample values.
//! 4. Compression and from_file round-tripping preserve VLSD channels.

use std::path::{Path, PathBuf};
use std::process::Command;

use falcon_mdf::blocks::ChannelType;
use falcon_mdf::{Mf4File, Mf4Writer, SignalValues};

fn venv_python() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python"),
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
fn vlsd_raw_sd_block_bytes_match_hand_written_expected_sequence() {
    let times = vec![0.0, 1.0, 2.0];
    let strings = vec!["", "a", "abcdef"];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group.add_channel_vlsd_str("StrChannel", "", &strings).unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();

    // Verify file starts with MDF 4.11 header
    assert_eq!(&bytes[0..8], b"MDF     ");
    assert_eq!(&bytes[8..16], b"4.11    ");

    // Hand-written expected ##SD block binary sequence:
    // Block header:
    //   b"##SD" (4 bytes)
    //   reserved [0u8; 4] (4 bytes)
    //   length: 24 (header) + 19 (payload data) = 43 (8 bytes LE: 0x2b)
    //   link_count: 0 (8 bytes LE)
    // Payload data:
    //   Sample 0 (""): len = 0 (4 bytes LE) -> [0, 0, 0, 0]
    //   Sample 1 ("a"): len = 1 (4 bytes LE) -> [1, 0, 0, 0], payload: b"a"
    //   Sample 2 ("abcdef"): len = 6 (4 bytes LE) -> [6, 0, 0, 0], payload: b"abcdef"
    let expected_sd_block: &[u8] = &[
        b'#', b'#', b'S', b'D', 0, 0, 0, 0, // header id & reserved
        43, 0, 0, 0, 0, 0, 0, 0,            // length = 43
        0, 0, 0, 0, 0, 0, 0, 0,             // link_count = 0
        0, 0, 0, 0,                         // sample 0: len = 0
        1, 0, 0, 0, b'a',                   // sample 1: len = 1, "a"
        6, 0, 0, 0, b'a', b'b', b'c', b'd', b'e', b'f', // sample 2: len = 6, "abcdef"
    ];

    // Find ##SD block in the written file
    let sd_pos = bytes
        .windows(4)
        .position(|w| w == b"##SD")
        .expect("##SD block must be present in written file");

    assert_eq!(
        &bytes[sd_pos..sd_pos + expected_sd_block.len()],
        expected_sd_block,
        "The on-disk bytes of ##SD block must match the hand-written expected sequence"
    );

    // Read back with falcon_mdf parser
    let temp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp.path(), &bytes).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let ch = file.find_channel("StrChannel").expect("find StrChannel");
    assert_eq!(ch.channel_type, ChannelType::VariableLength);
    assert_eq!(ch.data_link(), sd_pos as u64);

    let sig = file.signal(ch).expect("read signal");
    match sig.values().expect("decode values") {
        SignalValues::Str(vals) => assert_eq!(vals, vec!["", "a", "abcdef"]),
        other => panic!("expected SignalValues::Str, got {other:?}"),
    }
}

#[test]
fn vlsd_uneven_lengths_roundtrip_falcon_and_asammdf() {
    let long_string = "x".repeat(300);
    let special_string = "hello 🚀 world ✨🚗 test";
    let strings = vec![
        "",
        "a",
        "abcdef",
        &long_string,
        "middle",
        "",
        special_string,
        "end",
    ];
    let times: Vec<f64> = (0..strings.len()).map(|i| i as f64 * 0.25).collect();
    let valid_mask = vec![true, true, false, true, true, false, true, true];

    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_vlsd_with(
            "UnevenStr",
            "text",
            SignalValues::Str(strings.iter().map(|s| s.to_string()).collect()),
            Some(&valid_mask),
            None,
        )
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    // 1. Assert falcon_mdf reads it back accurately
    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("UnevenStr").expect("find UnevenStr");
    assert_eq!(ch.channel_type, ChannelType::VariableLength);
    assert_eq!(ch.unit, "text");

    let sig = file.signal(ch).expect("read signal");
    let read_strings = match sig.values().expect("decode values") {
        SignalValues::Str(v) => v,
        other => panic!("expected SignalValues::Str, got {other:?}"),
    };
    assert_eq!(read_strings, strings);
    assert_eq!(sig.validity(), Some(valid_mask.clone()));

    // 2. Cross-check with asammdf if available
    let Some(python) = venv_python() else {
        eprintln!("skipping asammdf check: no .venv python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping asammdf check: asammdf not installed in {}", python.display());
        return;
    }

    let script = r#"
import sys, json
from asammdf import MDF

path = sys.argv[1]
m = MDF(path)

# Test with all samples (ignore invalidation bits)
sig_all = m.get("UnevenStr", raw=True, ignore_invalidation_bits=True)
samples_all = []
for s in sig_all.samples:
    if isinstance(s, bytes):
        samples_all.append(s.decode('utf-8'))
    else:
        samples_all.append(str(s))

# Test with invalid samples filtered out
sig_valid = m.get("UnevenStr", raw=True, ignore_invalidation_bits=False)
samples_valid = []
for s in sig_valid.samples:
    if isinstance(s, bytes):
        samples_valid.append(s.decode('utf-8'))
    else:
        samples_valid.append(str(s))

timestamps_all = [float(t) for t in sig_all.timestamps]
timestamps_valid = [float(t) for t in sig_valid.timestamps]

print(json.dumps({
    "samples_all": samples_all,
    "samples_valid": samples_valid,
    "timestamps_all": timestamps_all,
    "timestamps_valid": timestamps_valid,
    "unit": sig_all.unit,
}))
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(temp.path())
        .output()
        .expect("run python");

    assert!(
        output.status.success(),
        "asammdf failed to read VLSD file:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse json");

    let asammdf_samples_all: Vec<String> = parsed["samples_all"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    let asammdf_samples_valid: Vec<String> = parsed["samples_valid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert_eq!(
        asammdf_samples_all, strings,
        "asammdf decoded samples (ignoring invalidation) must match original uneven strings"
    );

    let expected_valid_strings: Vec<&str> = strings
        .iter()
        .enumerate()
        .filter(|(i, _)| valid_mask[*i])
        .map(|(_, s)| *s)
        .collect();
    assert_eq!(
        asammdf_samples_valid, expected_valid_strings,
        "asammdf filtered samples must match valid mask subset"
    );
    assert_eq!(parsed["unit"].as_str().unwrap(), "text");
}

#[test]
fn vlsd_byte_arrays_roundtrip_falcon_and_asammdf() {
    let large_blob = vec![0xABu8; 400];
    let byte_slices: Vec<&[u8]> = vec![
        b"",
        b"\x01\x02",
        b"\xde\xad\xbe\xef",
        &large_blob,
        b"raw binary \x00 with zeroes \xff",
    ];
    let times: Vec<f64> = (0..byte_slices.len()).map(|i| i as f64 * 0.1).collect();

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&times).unwrap();
    group.add_channel_vlsd_bytes("VarBytesChannel", "bytes", &byte_slices).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    // 1. falcon_mdf verification
    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("VarBytesChannel").expect("find VarBytesChannel");
    assert_eq!(ch.channel_type, ChannelType::VariableLength);

    let sig = file.signal(ch).expect("signal");
    match sig.values().expect("values") {
        SignalValues::VarBytes { data, starts } => {
            let recovered: Vec<&[u8]> = starts
                .windows(2)
                .map(|w| &data[w[0]..w[1]])
                .collect();
            assert_eq!(recovered, byte_slices);
        }
        other => panic!("expected VarBytes, got {other:?}"),
    }

    // 2. asammdf cross-check
    let Some(python) = venv_python() else { return };
    if !asammdf_available(&python) { return };

    let script = r#"
import sys, json
from asammdf import MDF

path = sys.argv[1]
m = MDF(path)
sig = m.get("VarBytesChannel", raw=True)

samples = []
for s in sig.samples:
    if isinstance(s, (bytes, bytearray)):
        samples.append(list(s))
    elif hasattr(s, 'tobytes'):
        samples.append(list(s.tobytes()))
    else:
        samples.append(list(bytes(s)))

print(json.dumps({"samples": samples}))
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(temp.path())
        .output()
        .expect("run python");

    assert!(
        output.status.success(),
        "asammdf failed to read VarBytes:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).expect("parse json");
    let asammdf_bytes: Vec<Vec<u8>> = parsed["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arr| {
            arr.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect()
        })
        .collect();

    let expected_vecs: Vec<Vec<u8>> = byte_slices.iter().map(|s| s.to_vec()).collect();
    for (i, (a, b)) in asammdf_bytes.iter().zip(&expected_vecs).enumerate() {
        assert!(
            a.starts_with(b),
            "asammdf sample {i} does not start with expected bytes:\nasammdf: {a:?}\nexpected: {b:?}"
        );
    }
    assert_eq!(asammdf_bytes.len(), expected_vecs.len());
}

#[test]
fn vlsd_channel_in_compressed_mf4() {
    let strings = vec!["alpha", "beta", "gamma", "delta", "epsilon"];
    let times = vec![0.0, 1.0, 2.0, 3.0, 4.0];

    let mut writer = Mf4Writer::with_start_time_ns(0);
    writer.set_compression(true);
    let group = writer.add_group(&times).unwrap();
    group.add_channel_vlsd_str("CompressedVlsd", "", &strings).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    let file = Mf4File::open(temp.path()).unwrap();
    let ch = file.find_channel("CompressedVlsd").unwrap();
    let sig = file.signal(ch).unwrap();
    match sig.values().unwrap() {
        SignalValues::Str(v) => assert_eq!(v, strings),
        other => panic!("expected Str, got {other:?}"),
    }

    let Some(python) = venv_python() else { return };
    if !asammdf_available(&python) { return };

    let script = r#"
import sys
from asammdf import MDF

m = MDF(sys.argv[1])
sig = m.get("CompressedVlsd")
samples = [s.decode('utf-8') if isinstance(s, bytes) else str(s) for s in sig.samples]
assert samples == ["alpha", "beta", "gamma", "delta", "epsilon"], samples
print("OK")
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(temp.path())
        .output()
        .expect("run python");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn vlsd_channel_roundtrip_via_from_file() {
    let strings = vec!["first", "second_longer_payload", "", "fourth"];
    let times = vec![0.0, 0.1, 0.2, 0.3];

    let mut writer = Mf4Writer::with_start_time_ns(123456);
    let group = writer.add_group(&times).unwrap();
    group.add_channel_vlsd_str("PreservedVlsd", "unit_test", &strings).unwrap();

    let temp1 = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp1.path()).unwrap();

    // Load with from_file and write out to temp2
    let file1 = Mf4File::open(temp1.path()).unwrap();
    let writer2 = Mf4Writer::from_file(&file1).unwrap();

    let temp2 = tempfile::NamedTempFile::new().unwrap();
    writer2.write_to_file(temp2.path()).unwrap();

    // Verify temp2 has the exact VLSD channel and values
    let file2 = Mf4File::open(temp2.path()).unwrap();
    let ch2 = file2.find_channel("PreservedVlsd").unwrap();
    assert_eq!(ch2.channel_type, ChannelType::VariableLength);
    assert_eq!(ch2.unit, "unit_test");

    let sig2 = file2.signal(ch2).unwrap();
    match sig2.values().unwrap() {
        SignalValues::Str(v) => assert_eq!(v, strings),
        other => panic!("expected Str, got {other:?}"),
    }
}
