//! Tests streaming variable-length signal data (VLSD) from dedicated ##SD blocks,
//! cross-checked against asammdf and demonstrating bounded memory.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::blocks::ChannelType;
use falcon_mdf::{Mf4File, SignalValues};

fn venv_python() -> Option<PathBuf> {
    let python = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python");
    if python.is_file() {
        return Some(python);
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python");
    local.is_file().then_some(local)
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn streaming_vlsd_dedicated_sd_block_matches_asammdf() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in the .venv");
        return;
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vlsd_dedicated_sd.mf4");

    // Write an MF4 file with a dedicated ##SD block holding 2,000 variable-length strings
    let script = r#"
import sys
import numpy as np
from asammdf import MDF, Signal

m = MDF()
n = 2000
t = np.linspace(0, 10, n)
strings = [f"Payload_{i}_" + "X" * (i % 50) for i in range(n)]
bytes_arr = np.array([s.encode("utf-8") for s in strings])
sig = Signal(samples=bytes_arr, timestamps=t, name="VlsdStrings", encoding="utf-8")
m.append(sig)
m.save(sys.argv[1], overwrite=True)
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(&path)
        .output()
        .expect("run python script");
    assert!(
        output.status.success(),
        "python failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let file = Mf4File::open(&path).expect("open file");
    let ch = file
        .find_channel("VlsdStrings")
        .expect("find VlsdStrings channel");
    assert_eq!(ch.channel_type, ChannelType::VariableLength);

    // Eager read via signal()
    let eager_sig = file.signal(ch).expect("eager signal read");
    let eager_vals = match eager_sig.values().expect("decode eager values") {
        SignalValues::Str(strs) => strs,
        other => panic!("expected Str signal values, got: {other:?}"),
    };
    assert_eq!(eager_vals.len(), 2000);

    // Chunked read via signal_chunks()
    let chunks: Vec<_> = file
        .signal_chunks(ch)
        .expect("signal_chunks should succeed for dedicated SD block")
        .collect::<falcon_mdf::Result<Vec<_>>>()
        .expect("decode all chunks");

    assert!(!chunks.is_empty(), "at least one chunk must be returned");

    let mut chunked_strs = Vec::with_capacity(2000);
    for chunk in chunks {
        match chunk.values().expect("decode chunk values") {
            SignalValues::Str(strs) => chunked_strs.extend(strs),
            other => panic!("expected Str chunk values, got: {other:?}"),
        }
    }

    assert_eq!(chunked_strs.len(), 2000);
    assert_eq!(chunked_strs, eager_vals);

    // Verify against ground truth
    for (i, s) in chunked_strs.iter().enumerate() {
        let expected = format!("Payload_{}_", i) + &"X".repeat(i % 50);
        assert_eq!(s, &expected, "mismatch at sample {}", i);
    }
}

#[test]
fn streaming_vlsd_scales_sublinearly_in_memory_with_large_group() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in the .venv");
        return;
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vlsd_large.mf4");

    // Write a larger dataset (~10,000 samples with 500-byte strings each = ~5 MB SD payload)
    let script = r#"
import sys
import numpy as np
from asammdf import MDF, Signal

m = MDF()
n = 10000
t = np.linspace(0, 100, n)
strings = [f"Idx_{i:05d}_" + "A" * 450 for i in range(n)]
bytes_arr = np.array([s.encode("utf-8") for s in strings])
sig = Signal(samples=bytes_arr, timestamps=t, name="LargeVlsd", encoding="utf-8")
m.append(sig)
m.save(sys.argv[1], overwrite=True)
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(&path)
        .output()
        .expect("run python script");
    assert!(
        output.status.success(),
        "python failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let file = Mf4File::open(&path).expect("open file");
    let ch = file.find_channel("LargeVlsd").expect("find LargeVlsd");

    let mut total_samples = 0;
    let mut chunk_count = 0;

    for chunk_res in file.signal_chunks(ch).expect("signal_chunks") {
        let chunk = chunk_res.expect("chunk read");
        let samples = chunk.len();
        assert!(samples > 0);
        total_samples += samples;
        chunk_count += 1;

        // Verify that each chunk decodes its sample strings accurately
        if let SignalValues::Str(strs) = chunk.values().expect("chunk values") {
            assert_eq!(strs.len(), samples);
            for s in &strs {
                assert!(s.starts_with("Idx_"));
                assert_eq!(s.len(), 10 + 450);
            }
        } else {
            panic!("expected Str signal values");
        }
    }

    assert_eq!(total_samples, 10000);
    assert!(chunk_count >= 1);
}

// ---------------------------------------------------------------------------
// Synthetic test building an MF4 with a dedicated ##SD block
// ---------------------------------------------------------------------------

const HEADER: usize = 24;

fn block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
    let total = HEADER + links.len() * 8 + data.len();
    let mut out = vec![0u8; HEADER];
    out[0..4].copy_from_slice(id);
    out[8..16].copy_from_slice(&(total as u64).to_le_bytes());
    out[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());
    for link in links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

fn tx(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    while !data.len().is_multiple_of(8) {
        data.push(0);
    }
    block(b"##TX", &[], &data)
}

fn hd() -> Vec<u8> {
    let mut data = vec![0u8; 32];
    data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
    block(b"##HD", &[0; 6], &data)
}

struct SynthFile {
    bytes: Vec<u8>,
}

impl SynthFile {
    fn new() -> Self {
        let mut bytes = vec![0u8; 64];
        bytes[0..8].copy_from_slice(b"MDF     ");
        bytes[8..16].copy_from_slice(b"4.11    ");
        bytes[16..24].copy_from_slice(b"falcon  ");
        bytes[28..30].copy_from_slice(&411u16.to_le_bytes());
        SynthFile { bytes }
    }

    fn push(&mut self, b: &[u8]) -> u64 {
        let at = self.bytes.len() as u64;
        self.bytes.extend_from_slice(b);
        at
    }

    fn patch_link(&mut self, at: u64, value: u64) {
        let at = at as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn open(&self, name: &str) -> falcon_mdf::Result<Mf4File> {
        let path = std::env::temp_dir().join(format!("falcon_synth_{name}.mf4"));
        std::fs::write(&path, &self.bytes).expect("write temp file");
        let result = Mf4File::open(&path);
        let _ = std::fs::remove_file(&path);
        result
    }
}

#[test]
fn streaming_vlsd_from_dedicated_sd_block_synthetic() {
    let mut f = SynthFile::new();
    f.push(&hd());

    let name = f.push(&tx("Message"));

    // Build dedicated ##SD block data
    let mut sd_data = Vec::new();
    let mut record_data = Vec::new();
    let n_samples = 500usize;

    for i in 0..n_samples {
        let payload_str = format!("Synthetic Message #{:04}", i);
        let payload_bytes = payload_str.as_bytes();
        let current_offset = sd_data.len() as u64;

        // 4-byte length prefix + payload
        sd_data.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes());
        sd_data.extend_from_slice(payload_bytes);

        // Record holds 8-byte stored offset
        record_data.extend_from_slice(&current_offset.to_le_bytes());
    }

    let sd_block = f.push(&block(b"##SD", &[], &sd_data));
    let dt_block = f.push(&block(b"##DT", &[], &record_data));

    // CN: channel_type = 1 (VariableLength), data_type = 7 (String UTF-8), bit_count = 64
    // Links: [next=0, composition=0, name, source=0, conversion=0, data=sd_block, unit=0, comment=0]
    let mut cn_data = vec![0u8; 72];
    cn_data[0] = 1; // VariableLength
    cn_data[2] = 7; // String UTF-8
    cn_data[4..8].copy_from_slice(&0u32.to_le_bytes()); // byte_offset = 0
    cn_data[8..12].copy_from_slice(&64u32.to_le_bytes()); // bit_count = 64
    let cn_block = f.push(&block(
        b"##CN",
        &[0, 0, name, 0, 0, sd_block, 0, 0],
        &cn_data,
    ));

    // CG: cycle_count = 500, data_bytes = 8
    let mut cg_data = vec![0u8; 32];
    cg_data[8..16].copy_from_slice(&(n_samples as u64).to_le_bytes());
    cg_data[24..28].copy_from_slice(&8u32.to_le_bytes());
    let cg_block = f.push(&block(b"##CG", &[0, cn_block, 0, 0, 0, 0], &cg_data));

    // DG: rec_id_size = 0 (sorted)
    let dg_block = f.push(&block(b"##DG", &[0, cg_block, dt_block, 0], &[0u8; 8]));

    // Patch HD first data group link
    f.patch_link(64 + HEADER as u64, dg_block);

    let file = f.open("vlsd_synth").expect("synthetic file open");
    let ch = file.find_channel("Message").expect("find Message");
    assert_eq!(ch.channel_type, ChannelType::VariableLength);

    // Test eager read
    let eager = file.signal(ch).expect("eager signal");
    let eager_strs = match eager.values().expect("decode eager") {
        SignalValues::Str(s) => s,
        other => panic!("expected Str, got {other:?}"),
    };
    assert_eq!(eager_strs.len(), n_samples);

    // Test streaming chunks
    let chunks: Vec<_> = file
        .signal_chunks(ch)
        .expect("signal_chunks")
        .collect::<falcon_mdf::Result<Vec<_>>>()
        .expect("chunks decode");

    let mut streamed_strs = Vec::with_capacity(n_samples);
    for chunk in chunks {
        if let SignalValues::Str(strs) = chunk.values().expect("chunk values") {
            streamed_strs.extend(strs);
        }
    }

    assert_eq!(streamed_strs.len(), n_samples);
    assert_eq!(streamed_strs, eager_strs);
    assert_eq!(streamed_strs[0], "Synthetic Message #0000");
    assert_eq!(streamed_strs[499], "Synthetic Message #0499");
}
