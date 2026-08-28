//! WebAssembly portability and in-memory I/O backend tests.
//!
//! Asserts that:
//! 1. `Mf4File::from_bytes` reads every sample of every channel identically
//!    to `Mf4File::open` for real corpus files from `test_data/reference/`.
//! 2. Out-of-range reads and truncated files through the in-memory backend
//!    return an `Err` naming the truncation rather than panicking.

use falcon_mdf::error::Mf4Error;
use falcon_mdf::io::memory::MemorySource;
use falcon_mdf::io::ByteSource;
use falcon_mdf::{Mf4File, Signal, SignalValues};
use std::path::{Path, PathBuf};

fn reference_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let ref_dir = Path::new("test_data/reference");
    if let Ok(entries) = std::fs::read_dir(ref_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ext.eq_ignore_ascii_case("mf4") {
                        files.push(path);
                    }
                }
            }
        }
    }
    files.sort();
    files
}

fn assert_signals_match(open_sig: &Signal, mem_sig: &Signal, channel_name: &str, file_name: &str) {
    assert_eq!(
        open_sig.len(),
        mem_sig.len(),
        "{file_name} '{channel_name}': sample count mismatch"
    );

    // Compare validity masks
    assert_eq!(
        open_sig.validity(),
        mem_sig.validity(),
        "{file_name} '{channel_name}': validity mismatch"
    );

    // Compare raw values
    match (open_sig.raw_values(), mem_sig.raw_values()) {
        (Ok(v1), Ok(v2)) => match (&v1, &v2) {
            (SignalValues::F64(f1), SignalValues::F64(f2)) => {
                assert_eq!(
                    f1.len(),
                    f2.len(),
                    "{file_name} '{channel_name}': raw f64 values length mismatch"
                );
                for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                    if a.is_nan() && b.is_nan() {
                        continue;
                    }
                    assert_eq!(
                        a, b,
                        "{file_name} '{channel_name}': raw f64 sample {idx} mismatch: got {b}, want {a}"
                    );
                }
            }
            (SignalValues::F32(f1), SignalValues::F32(f2)) => {
                assert_eq!(
                    f1.len(),
                    f2.len(),
                    "{file_name} '{channel_name}': raw f32 values length mismatch"
                );
                for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                    if a.is_nan() && b.is_nan() {
                        continue;
                    }
                    assert_eq!(
                        a, b,
                        "{file_name} '{channel_name}': raw f32 sample {idx} mismatch: got {b}, want {a}"
                    );
                }
            }
            _ => {
                assert_eq!(
                    v1, v2,
                    "{file_name} '{channel_name}': raw signal values mismatch"
                );
            }
        },
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => {
            panic!("{file_name} '{channel_name}': raw_values open succeeded but from_bytes failed: {e}")
        }
        (Err(e), Ok(_)) => {
            panic!("{file_name} '{channel_name}': raw_values open failed ({e}) but from_bytes succeeded")
        }
    }

    // Compare every sample of the converted values
    match (open_sig.values(), mem_sig.values()) {
        (Ok(v1), Ok(v2)) => match (&v1, &v2) {
            (SignalValues::F64(f1), SignalValues::F64(f2)) => {
                assert_eq!(
                    f1.len(),
                    f2.len(),
                    "{file_name} '{channel_name}': f64 values length mismatch"
                );
                for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                    if a.is_nan() && b.is_nan() {
                        continue;
                    }
                    assert_eq!(
                        a, b,
                        "{file_name} '{channel_name}': f64 sample {idx} mismatch: got {b}, want {a}"
                    );
                }
            }
            (SignalValues::F32(f1), SignalValues::F32(f2)) => {
                assert_eq!(
                    f1.len(),
                    f2.len(),
                    "{file_name} '{channel_name}': f32 values length mismatch"
                );
                for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                    if a.is_nan() && b.is_nan() {
                        continue;
                    }
                    assert_eq!(
                        a, b,
                        "{file_name} '{channel_name}': f32 sample {idx} mismatch: got {b}, want {a}"
                    );
                }
            }
            _ => {
                assert_eq!(
                    v1, v2,
                    "{file_name} '{channel_name}': signal values mismatch"
                );
            }
        },
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => {
            panic!("{file_name} '{channel_name}': open succeeded but from_bytes failed: {e}")
        }
        (Err(e), Ok(_)) => {
            panic!("{file_name} '{channel_name}': open failed ({e}) but from_bytes succeeded")
        }
    }
}

#[test]
fn from_bytes_matches_open_for_every_sample_of_every_channel() {
    let files = reference_files();
    assert!(
        !files.is_empty(),
        "Reference test files must be present under test_data/reference/"
    );

    for path in &files {
        let file_name = path.file_name().unwrap().to_string_lossy();
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("Failed to read {path:?}: {e}"));

        let file_open =
            Mf4File::open(path).unwrap_or_else(|e| panic!("Mf4File::open failed on {path:?}: {e}"));
        let file_mem = Mf4File::from_bytes(bytes)
            .unwrap_or_else(|e| panic!("Mf4File::from_bytes failed on {path:?}: {e}"));

        assert_eq!(
            file_open.channel_count(),
            file_mem.channel_count(),
            "{file_name}: channel count mismatch"
        );
        assert_eq!(
            file_open.version(),
            file_mem.version(),
            "{file_name}: version mismatch"
        );
        assert_eq!(
            file_open.data_group_count(),
            file_mem.data_group_count(),
            "{file_name}: data group count mismatch"
        );

        let channels_open: Vec<_> = file_open.channels().cloned().collect();
        let channels_mem: Vec<_> = file_mem.channels().cloned().collect();

        assert_eq!(
            channels_open.len(),
            channels_mem.len(),
            "{file_name}: channels list length mismatch"
        );

        for (ch_open, ch_mem) in channels_open.iter().zip(channels_mem.iter()) {
            assert_eq!(
                ch_open.name, ch_mem.name,
                "{file_name}: channel name mismatch"
            );

            let sig_open = file_open.signal(ch_open);
            let sig_mem = file_mem.signal(ch_mem);

            match (sig_open, sig_mem) {
                (Ok(s_open), Ok(s_mem)) => {
                    assert_signals_match(&s_open, &s_mem, &ch_open.name, &file_name);
                }
                (Err(_), Err(_)) => {}
                (Ok(_), Err(e)) => panic!(
                    "{file_name} '{}': signal open succeeded but memory failed: {e}",
                    ch_open.name
                ),
                (Err(e), Ok(_)) => panic!(
                    "{file_name} '{}': signal open failed ({e}) but memory succeeded",
                    ch_open.name
                ),
            }
        }
    }
}

#[test]
fn memory_backend_out_of_range_and_truncation_returns_error_not_panic() {
    // 1. Direct MemorySource out-of-range checks
    let source = MemorySource::new(vec![0u8; 100]);
    assert_eq!(source.len(), 100);

    // Read beyond total length
    let err = source.read_bytes(50, 100).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("truncated") || msg.contains("File is truncated"),
        "error must name truncation: {msg}"
    );
    assert!(matches!(
        err,
        Mf4Error::TruncatedFile {
            offset: 50,
            expected: 100,
            actual: 50
        }
    ));

    // Offset at or beyond length
    let err = source.read_bytes(100, 10).unwrap_err();
    assert!(matches!(
        err,
        Mf4Error::TruncatedFile {
            offset: 100,
            expected: 10,
            actual: 0
        }
    ));

    let err = source.read_bytes(200, 10).unwrap_err();
    assert!(matches!(
        err,
        Mf4Error::TruncatedFile {
            offset: 200,
            expected: 10,
            actual: 0
        }
    ));

    // Empty memory source
    let empty_source = MemorySource::new(Vec::new());
    let err = empty_source.read_bytes(0, 64).unwrap_err();
    assert!(matches!(
        err,
        Mf4Error::TruncatedFile {
            offset: 0,
            expected: 64,
            actual: 0
        }
    ));

    // 2. Truncated real file parsing through Mf4File::from_bytes
    let files = reference_files();
    assert!(!files.is_empty(), "Reference files must be present");

    let sample_file = &files[0];
    let full_bytes = std::fs::read(sample_file).expect("read sample file");
    assert!(full_bytes.len() > 128, "sample file too small");

    // Test multiple truncation cutoffs
    let cutoffs = [
        0,
        16,
        32,
        63,
        64,
        80,
        full_bytes.len() / 4,
        full_bytes.len() / 2,
        full_bytes.len() - 1,
    ];

    for &cutoff in &cutoffs {
        let truncated = full_bytes[..cutoff].to_vec();
        let result = Mf4File::from_bytes(truncated);
        assert!(
            result.is_err(),
            "from_bytes on truncated file ({cutoff}/{} bytes) must return Err, not Ok",
            full_bytes.len()
        );
    }

    // Specific check that a partway-truncated file reports a truncation error
    let mid_truncated = full_bytes[..full_bytes.len() / 2].to_vec();
    let err = Mf4File::from_bytes(mid_truncated).unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("truncated")
            || err_str.contains("File is truncated")
            || err_str.contains("Missing required block")
            || matches!(err, Mf4Error::TruncatedFile { .. }),
        "Error for truncated file should name truncation or missing block: {err_str}"
    );
}
