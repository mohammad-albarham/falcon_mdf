//! Multi-channel aligned streaming tests.
//!
//! Asserts that:
//! 1. Concatenating the yielded windows from `signals_chunks` is exactly equal to the
//!    non-streaming `signals()` result for the same channels — same values, same order, same length.
//! 2. Uneven tails (where `chunk_size` does not divide the sample count evenly) are handled cleanly.
//! 3. Cross-group channel selections are refused by name.

use falcon_mdf::{Mf4Error, Mf4File, Mf4Writer, SignalValues};
use serde_json::Value;
use std::path::Path;

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

fn resolve(key: &str) -> String {
    if key.contains('/') {
        key.to_string()
    } else {
        format!("test_data/reference/{key}")
    }
}

fn declined(err: &Mf4Error) -> bool {
    matches!(err, Mf4Error::Unsupported { .. })
}

#[test]
fn aligned_streaming_reproduces_eager_signals_across_corpus() {
    let paths = corpus();
    if paths.is_empty() {
        eprintln!("SKIP: no corpus files present under test_data/");
        return;
    }

    for path in &paths {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));

        // Group channels by (dg_index, cg_index)
        let mut groups: std::collections::BTreeMap<(usize, usize), Vec<falcon_mdf::Channel>> =
            std::collections::BTreeMap::new();
        for ch in file.channels() {
            groups
                .entry((ch.data_group_index, ch.channel_group_index))
                .or_default()
                .push(ch.clone());
        }

        for ((dg_idx, cg_idx), group_channels) in groups {
            if group_channels.is_empty() {
                continue;
            }

            let eager_signals = match file.signals(&group_channels) {
                Ok(sigs) => sigs,
                Err(e) if declined(&e) => continue,
                Err(e) => panic!("{path} dg {dg_idx} cg {cg_idx} eager signals failed: {e}"),
            };

            for chunk_size in [1, 7, 33, 100, 500] {
                let stream = match file.signals_chunks(&group_channels, chunk_size) {
                    Ok(s) => s,
                    Err(e) if declined(&e) => continue,
                    Err(e) => panic!("{path} signals_chunks failed: {e}"),
                };

                let mut accumulated_f64: Vec<Vec<f64>> = vec![Vec::new(); group_channels.len()];
                let mut accumulated_bytes: Vec<Vec<Vec<u8>>> =
                    vec![Vec::new(); group_channels.len()];
                let mut window_count = 0usize;

                for chunk_res in stream {
                    let chunk = chunk_res.unwrap_or_else(|e| {
                        panic!("{path} dg {dg_idx} cg {cg_idx} chunk error: {e}")
                    });
                    assert_eq!(
                        chunk.len(),
                        group_channels.len(),
                        "each yielded chunk must carry all requested channels"
                    );

                    let window_sample_count = chunk[0].len();
                    for sig in &chunk {
                        assert_eq!(
                            sig.len(),
                            window_sample_count,
                            "every channel in chunk must cover the same window length"
                        );
                    }

                    for (i, sig) in chunk.iter().enumerate() {
                        let Ok(vals) = sig.values() else { continue };
                        match vals {
                            SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => {
                                accumulated_bytes[i]
                                    .extend((0..vals.len()).map(|idx| {
                                        vals.bytes_at(idx).unwrap_or_default().to_vec()
                                    }));
                            }
                            _ => {
                                if let Ok(f64_vals) = sig.values_f64() {
                                    accumulated_f64[i].extend(f64_vals);
                                }
                            }
                        }
                    }
                    window_count += 1;
                }

                for (i, (ch, eager)) in group_channels.iter().zip(&eager_signals).enumerate() {
                    let Ok(eager_vals) = eager.values() else {
                        continue;
                    };
                    match eager_vals {
                        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => {
                            let expected_bytes: Vec<Vec<u8>> = (0..eager_vals.len())
                                .map(|idx| eager_vals.bytes_at(idx).unwrap_or_default().to_vec())
                                .collect();
                            assert_eq!(
                                accumulated_bytes[i], expected_bytes,
                                "{path} '{}' bytes mismatch over {window_count} window(s) of chunk_size {chunk_size}",
                                ch.name
                            );
                        }
                        _ => {
                            if let Ok(expected_f64) = eager.values_f64() {
                                assert_eq!(
                                    accumulated_f64[i].len(),
                                    expected_f64.len(),
                                    "{path} '{}' length mismatch over {window_count} window(s) of chunk_size {chunk_size}",
                                    ch.name
                                );
                                for (sample_idx, (got, want)) in
                                    accumulated_f64[i].iter().zip(&expected_f64).enumerate()
                                {
                                    if got.is_nan() && want.is_nan() {
                                        continue;
                                    }
                                    assert_eq!(
                                        got, want,
                                        "{path} '{}' sample {sample_idx} mismatch over {window_count} window(s) of chunk_size {chunk_size}",
                                        ch.name
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn uneven_tail_chunk_sizes_match_eager_signals() {
    let mut writer = Mf4Writer::new();
    let sample_count: usize = 143; // prime number to force uneven tails
    let times: Vec<f64> = (0..sample_count).map(|i| i as f64 * 0.05).collect();
    let speed: Vec<f64> = times.iter().map(|t| (t * 2.0).sin() * 100.0).collect();
    let torque: Vec<f64> = times.iter().map(|t| t * 15.0 + 5.0).collect();
    let valid_mask: Vec<bool> = (0..sample_count).map(|i| i % 7 != 0).collect();

    let group = writer.add_group(&times).expect("add group");
    group
        .add_channel("Speed", "km/h", &speed)
        .expect("add Speed");
    group
        .add_channel_with_validity("Torque", "Nm", &torque, Some(&valid_mask))
        .expect("add Torque");

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writer.write_to_file(tmp.path()).expect("write");

    let file = Mf4File::open(tmp.path()).expect("open");
    let speed_ch = file.find_channel("Speed").expect("find Speed");
    let torque_ch = file.find_channel("Torque").expect("find Torque");
    let channels = vec![speed_ch, torque_ch];

    let eager = file.signals(&channels).expect("eager signals");
    let eager_speed = eager[0].values_f64().expect("eager speed values");
    let eager_torque = eager[1].values_f64().expect("eager torque values");
    let eager_torque_valid = eager[1].validity().expect("eager torque validity");

    // Test a variety of chunk sizes that don't divide 143 evenly
    for chunk_size in [1, 2, 3, 5, 10, 13, 17, 25, 50, 100, 142, 143, 144, 1000] {
        let stream = file
            .signals_chunks(&channels, chunk_size)
            .expect("signals_chunks");

        let mut streamed_speed = Vec::new();
        let mut streamed_torque = Vec::new();
        let mut streamed_torque_valid = Vec::new();
        let mut chunk_count = 0usize;

        for chunk_res in stream {
            let chunk = chunk_res.expect("chunk");
            assert_eq!(chunk.len(), 2);
            let n = chunk[0].len();
            assert_eq!(chunk[1].len(), n);
            assert!(n <= chunk_size, "chunk size cannot exceed requested budget");

            streamed_speed.extend(chunk[0].values_f64().expect("speed values"));
            streamed_torque.extend(chunk[1].values_f64().expect("torque values"));
            streamed_torque_valid.extend(chunk[1].validity().expect("torque validity"));
            chunk_count += 1;
        }

        let expected_chunks = sample_count.div_ceil(chunk_size);
        assert_eq!(
            chunk_count, expected_chunks,
            "chunk_size {chunk_size} must yield {expected_chunks} chunks"
        );
        assert_eq!(streamed_speed, eager_speed);
        assert_eq!(streamed_torque, eager_torque);
        assert_eq!(streamed_torque_valid, eager_torque_valid);
    }
}

#[test]
fn cross_group_channels_are_refused_by_name() {
    let mut writer = Mf4Writer::new();
    let times1: Vec<f64> = (0..50).map(|i| i as f64 * 0.1).collect();
    let times2: Vec<f64> = (0..30).map(|i| i as f64 * 0.2).collect();
    let speed: Vec<f64> = times1.iter().map(|t| t * 10.0).collect();
    let rpm: Vec<f64> = times2.iter().map(|t| t * 50.0).collect();

    let g1 = writer.add_group(&times1).expect("add group 1");
    g1.add_channel("Speed_G1", "km/h", &speed)
        .expect("add speed");

    let g2 = writer.add_group(&times2).expect("add group 2");
    g2.add_channel("RPM_G2", "rpm", &rpm).expect("add rpm");

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writer.write_to_file(tmp.path()).expect("write");

    let file = Mf4File::open(tmp.path()).expect("open");
    let speed_ch = file.find_channel("Speed_G1").expect("find Speed_G1");
    let rpm_ch = file.find_channel("RPM_G2").expect("find RPM_G2");

    let err = file
        .signals_chunks(&[speed_ch, rpm_ch], 10)
        .expect_err("cross-group signals_chunks must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("Speed_G1") && msg.contains("RPM_G2"),
        "error message must name the offending channels by name: {msg}"
    );
    assert!(
        msg.contains("different channel groups"),
        "error message must describe that channels belong to different channel groups: {msg}"
    );
}

#[test]
fn chunk_size_zero_is_refused() {
    let mut writer = Mf4Writer::new();
    let times: Vec<f64> = (0..10).map(|i| i as f64 * 0.1).collect();
    let speed: Vec<f64> = times.iter().map(|t| t * 10.0).collect();
    let group = writer.add_group(&times).expect("group");
    group.add_channel("Speed", "km/h", &speed).expect("channel");

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    writer.write_to_file(tmp.path()).expect("write");

    let file = Mf4File::open(tmp.path()).expect("open");
    let speed_ch = file.find_channel("Speed").expect("channel");

    let err = file
        .signals_chunks(&[speed_ch], 0)
        .expect_err("chunk_size 0 must fail");
    assert!(err.to_string().contains("chunk_size"));
}
