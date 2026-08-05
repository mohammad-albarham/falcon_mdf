//! Block-by-block reading checked against reading all at once.
//!
//! The contract is equality: `signal_chunks` must hand back the same samples, in
//! the same order, as `signal` does in one piece. That is a strong property to
//! test because the eager path is already pinned by `golden.rs` and
//! `reference.rs` against an independent reader, so agreement with it inherits
//! that verification rather than asserting a second opinion of the same bytes.
//!
//! It is also the property most likely to break in a specific way: a record may
//! straddle a data-block boundary, and a reader that drops the straddling
//! remainder loses one sample per boundary and shifts every sample after it.
//! Multi-block files are what catch that, so the test reports how many it saw.

use falcon_mdf::{Mf4Error, Mf4File, SignalValues};
use serde_json::Value;
use std::path::Path;

/// Every corpus file present, from both ground-truth tables.
///
/// Both are needed: the bus logs in `golden.json` are unsorted data groups with
/// variable-length payload channels, and the vendor reference set in
/// `reference_golden.json` is where the sorted, multi-block files are. Either
/// alone would leave half the streaming paths unexercised.
fn corpus() -> Vec<String> {
    let mut paths = Vec::new();
    for raw in [
        include_str!("data/golden.json"),
        include_str!("data/reference_golden.json"),
    ] {
        let table: Value = serde_json::from_str(raw).expect("ground truth is malformed");
        let files = table
            .get("files")
            .and_then(Value::as_object)
            .or_else(|| table.as_object())
            .expect("ground truth must be an object");
        paths.extend(
            files
                .keys()
                .map(|key| resolve(key))
                .filter(|path| Path::new(path).exists()),
        );
    }
    paths
}

/// Reference ground truth is keyed by bare file name; golden.json by path.
fn resolve(key: &str) -> String {
    if key.contains('/') {
        key.to_string()
    } else {
        format!("test_data/reference/{key}")
    }
}

/// True when a channel is one the streaming path declines by contract.
fn declined(err: &Mf4Error) -> bool {
    matches!(err, Mf4Error::Unsupported { .. })
}

#[test]
fn chunks_reproduce_the_whole_signal() {
    let paths = corpus();
    if paths.is_empty() {
        eprintln!("SKIP: no corpus files present under test_data/");
        return;
    }

    let mut compared = 0usize;
    let mut multi_block = 0usize;
    let mut declined_count = 0usize;
    let mut byte_channels = 0usize;

    for path in &paths {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));

        for channel in file.channels() {
            let Ok(whole) = file.signal(channel) else {
                continue;
            };
            let Ok(expected_values) = whole.values() else {
                continue;
            };
            // Byte-valued channels — frame payloads above all — must be compared
            // as bytes. `values_f64` does not carry them, so comparing only that
            // would pass without having looked at a single payload byte.
            let byte_valued = matches!(
                expected_values,
                SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
            );
            let expected: Vec<Vec<u8>> = if byte_valued {
                (0..expected_values.len())
                    .map(|i| expected_values.bytes_at(i).unwrap_or_default().to_vec())
                    .collect()
            } else {
                Vec::new()
            };
            let expected_f64 = if byte_valued {
                Vec::new()
            } else {
                match whole.values_f64() {
                    Ok(values) => values,
                    Err(_) => continue,
                }
            };

            let chunks = match file.signal_chunks(channel) {
                Ok(chunks) => chunks,
                Err(e) if declined(&e) => {
                    declined_count += 1;
                    continue;
                }
                Err(e) => panic!("{path} '{}': {e}", channel.name),
            };

            let mut streamed_bytes: Vec<Vec<u8>> = Vec::new();
            let mut streamed_f64: Vec<f64> = Vec::new();
            let mut blocks = 0usize;
            for chunk in chunks {
                let chunk = chunk.unwrap_or_else(|e| panic!("{path} '{}': {e}", channel.name));
                blocks += 1;
                if byte_valued {
                    let values = chunk
                        .values()
                        .unwrap_or_else(|e| panic!("{path} '{}': {e}", channel.name));
                    streamed_bytes.extend(
                        (0..values.len()).map(|i| values.bytes_at(i).unwrap_or_default().to_vec()),
                    );
                } else {
                    streamed_f64.extend(
                        chunk
                            .values_f64()
                            .unwrap_or_else(|e| panic!("{path} '{}': {e}", channel.name)),
                    );
                }
            }
            if blocks > 1 {
                multi_block += 1;
            }
            compared += 1;
            if byte_valued {
                byte_channels += 1;
            }

            if byte_valued {
                assert_eq!(
                    streamed_bytes.len(),
                    expected.len(),
                    "{path} '{}': payload count over {blocks} block(s)",
                    channel.name
                );
                for (i, (got, want)) in streamed_bytes.iter().zip(&expected).enumerate() {
                    assert_eq!(
                        got, want,
                        "{path} '{}' payload {i} over {blocks} block(s)",
                        channel.name
                    );
                }
            } else {
                assert_eq!(
                    streamed_f64.len(),
                    expected_f64.len(),
                    "{path} '{}': sample count over {blocks} block(s)",
                    channel.name
                );
                for (i, (got, want)) in streamed_f64.iter().zip(&expected_f64).enumerate() {
                    assert!(
                        got == want || (got.is_nan() && want.is_nan()),
                        "{path} '{}' sample {i}: streamed {got} is not {want}",
                        channel.name
                    );
                }
            }
        }
    }

    assert!(compared > 0, "no channel was compared");
    assert!(
        multi_block > 0,
        "every channel fitted in one block, so nothing exercised a block boundary"
    );
    assert!(
        byte_channels > 0,
        "no byte-valued channel was compared, so no payload was checked"
    );
    eprintln!(
        "compared {compared} channel(s): {byte_channels} byte-valued, \
         {multi_block} spanning several blocks, {declined_count} declined by contract"
    );
}

/// A bus log's payload channel — variable-length, in an unsorted group, its
/// payloads interleaved with the records pointing at them — is the case
/// streaming exists for, so it must not merely work but come back byte-identical
/// over more than one chunk's worth of frames.
#[test]
fn a_bus_log_payload_channel_streams() {
    let path = "test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4";
    if !Path::new(path).exists() {
        eprintln!("SKIP: {path} is absent");
        return;
    }

    let file = Mf4File::open(path).unwrap();
    let payload = file.find_channel("CAN_DataFrame.DataBytes").unwrap();

    let mut streamed: Vec<Vec<u8>> = Vec::new();
    for chunk in file.signal_chunks(payload).unwrap() {
        let values = chunk.unwrap().values().unwrap();
        streamed.extend((0..values.len()).map(|i| values.bytes_at(i).unwrap().to_vec()));
    }

    let whole = file.signal(payload).unwrap().values().unwrap();
    assert_eq!(streamed.len(), 145_534, "frame count");
    for (i, got) in streamed.iter().enumerate() {
        assert_eq!(got, whole.bytes_at(i).unwrap(), "payload {i}");
    }
    // Payload lengths vary in this log, so the offsets carried across chunks
    // cannot have been right by accident of every payload being eight bytes.
    assert!(
        streamed.iter().any(|p| p.len() != 8),
        "expected payloads of differing length"
    );
}

/// The one shape still declined is declined by name rather than read wrongly:
/// a variable-length channel whose payloads sit in its own signal-data block,
/// which is a second block chain rather than records in the stream.
#[test]
fn a_signal_data_block_channel_is_refused() {
    let mut checked = 0usize;

    for path in corpus() {
        let file = Mf4File::open(&path).unwrap();
        for channel in file.channels() {
            let Err(err) = file.signal_chunks(channel) else {
                continue;
            };
            assert!(
                declined(&err),
                "{path} '{}': unexpected failure {err}",
                channel.name
            );
            checked += 1;
        }
    }

    if checked == 0 {
        eprintln!("SKIP: no corpus channel stores its payloads in a signal-data block");
    }
}

/// Chunking must not depend on the record cache having been warmed by an eager
/// read of the same group first, and a file whose records arrive in a data list
/// has to come back in list order.
#[test]
fn chunks_read_a_multi_block_file_from_cold() {
    let path = "test_data/reference/Vector_DataList_Deflate.mf4";
    if !Path::new(path).exists() {
        eprintln!("SKIP: {path} is absent");
        return;
    }

    let cold = Mf4File::open(path).unwrap();
    let mut streamed_any = false;

    for channel in cold.channels() {
        let Ok(chunks) = cold.signal_chunks(channel) else {
            continue;
        };
        let mut streamed = Vec::new();
        let mut blocks = 0usize;
        for chunk in chunks {
            blocks += 1;
            streamed.extend(chunk.unwrap().values_f64().unwrap());
        }
        if blocks < 2 {
            continue;
        }
        streamed_any = true;

        // A second handle, so the comparison cannot be served by state the
        // streaming read left behind.
        let warm = Mf4File::open(path).unwrap();
        let expected = warm
            .signal(warm.find_channel(&channel.name).unwrap())
            .unwrap()
            .values_f64()
            .unwrap();
        assert_eq!(
            streamed, expected,
            "'{}' over {blocks} blocks",
            channel.name
        );
    }

    assert!(
        streamed_any,
        "no channel of a data-list file spanned several blocks"
    );
}
