//! WebAssembly portability and in-memory I/O backend tests.
//!
//! Asserts that:
//! 1. `Mf4File::from_bytes` reads every sample of every channel identically
//!    to `Mf4File::open` and `Mf4File::open_buffered` for self-contained synthetic MDF
//!    files (exercising all channel types, validity masks, conversions, compression,
//!    multi-channel groups) as well as real corpus files from `test_data/reference/` when present.
//! 2. `Mf4File::from_bytes` works completely in-memory without any filesystem access.
//! 3. Out-of-range reads, corrupted blocks, and truncated files through the in-memory backend
//!    return an `Err` rather than panicking, verified unconditionally on synthetic fixtures.

use falcon_mdf::error::Mf4Error;
use falcon_mdf::io::memory::MemorySource;
use falcon_mdf::io::ByteSource;
use falcon_mdf::{
    Conversion, Mf4File, Mf4Writer, OpenOptions, Signal, SignalValues, TableEntry, WriteCodec,
};
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

fn assert_signal_values_match(v1: &SignalValues, v2: &SignalValues, context: &str) {
    assert_eq!(
        v1.len(),
        v2.len(),
        "{context}: signal values length mismatch"
    );
    match (v1, v2) {
        (SignalValues::F64(f1), SignalValues::F64(f2)) => {
            for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: f64 sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        (SignalValues::F32(f1), SignalValues::F32(f2)) => {
            for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: f32 sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        (
            SignalValues::Array {
                values: v1,
                elements_per_sample: e1,
            },
            SignalValues::Array {
                values: v2,
                elements_per_sample: e2,
            },
        ) => {
            assert_eq!(e1, e2, "{context}: elements_per_sample mismatch");
            assert_eq!(v1.len(), v2.len(), "{context}: array length mismatch");
            for (idx, (a, b)) in v1.iter().zip(v2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: array sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        (
            SignalValues::ArrayVarLen {
                values: v1,
                starts: s1,
            },
            SignalValues::ArrayVarLen {
                values: v2,
                starts: s2,
            },
        ) => {
            assert_eq!(s1, s2, "{context}: starts mismatch");
            assert_eq!(
                v1.len(),
                v2.len(),
                "{context}: array varlen length mismatch"
            );
            for (idx, (a, b)) in v1.iter().zip(v2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: array varlen sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        (SignalValues::Complex { re: r1, im: i1 }, SignalValues::Complex { re: r2, im: i2 }) => {
            assert_eq!(r1.len(), r2.len(), "{context}: complex re length mismatch");
            assert_eq!(i1.len(), i2.len(), "{context}: complex im length mismatch");
            for (idx, (a, b)) in r1.iter().zip(r2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: complex re sample {idx} mismatch: got {b}, want {a}"
                );
            }
            for (idx, (a, b)) in i1.iter().zip(i2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{context}: complex im sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        _ => {
            assert_eq!(v1, v2, "{context}: signal values mismatch");
        }
    }
}

fn assert_signals_match(open_sig: &Signal, mem_sig: &Signal, channel_name: &str, file_name: &str) {
    assert_eq!(
        open_sig.len(),
        mem_sig.len(),
        "{file_name} '{channel_name}': sample count mismatch"
    );
    assert_eq!(
        open_sig.is_empty(),
        mem_sig.is_empty(),
        "{file_name} '{channel_name}': is_empty mismatch"
    );

    // Compare validity masks
    assert_eq!(
        open_sig.validity(),
        mem_sig.validity(),
        "{file_name} '{channel_name}': validity mismatch"
    );

    // Compare raw values
    match (open_sig.raw_values(), mem_sig.raw_values()) {
        (Ok(v1), Ok(v2)) => {
            assert_signal_values_match(
                &v1,
                &v2,
                &format!("{file_name} '{channel_name}' raw_values"),
            );
        }
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => {
            panic!("{file_name} '{channel_name}': raw_values open succeeded but from_bytes failed: {e}")
        }
        (Err(e), Ok(_)) => {
            panic!("{file_name} '{channel_name}': raw_values open failed ({e}) but from_bytes succeeded")
        }
    }

    // Compare converted values
    match (open_sig.values(), mem_sig.values()) {
        (Ok(v1), Ok(v2)) => {
            assert_signal_values_match(&v1, &v2, &format!("{file_name} '{channel_name}' values"));
        }
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => {
            panic!("{file_name} '{channel_name}': values open succeeded but from_bytes failed: {e}")
        }
        (Err(e), Ok(_)) => {
            panic!(
                "{file_name} '{channel_name}': values open failed ({e}) but from_bytes succeeded"
            )
        }
    }

    // Compare numeric values_f64 when available
    match (open_sig.values_f64(), mem_sig.values_f64()) {
        (Ok(f1), Ok(f2)) => {
            assert_eq!(
                f1.len(),
                f2.len(),
                "{file_name} '{channel_name}': values_f64 length mismatch"
            );
            for (idx, (a, b)) in f1.iter().zip(f2.iter()).enumerate() {
                if a.is_nan() && b.is_nan() {
                    continue;
                }
                assert_eq!(
                    a, b,
                    "{file_name} '{channel_name}': values_f64 sample {idx} mismatch: got {b}, want {a}"
                );
            }
        }
        (Err(_), Err(_)) => {}
        (Ok(_), Err(e)) => {
            panic!("{file_name} '{channel_name}': values_f64 open succeeded but from_bytes failed: {e}")
        }
        (Err(e), Ok(_)) => {
            panic!("{file_name} '{channel_name}': values_f64 open failed ({e}) but from_bytes succeeded")
        }
    }

    // Compare value_at sample by sample (check up to 500 samples)
    let sample_check_count = open_sig.len().min(500);
    for idx in 0..sample_check_count {
        match (open_sig.value_at(idx), mem_sig.value_at(idx)) {
            (Ok(v1), Ok(v2)) => {
                if v1.is_nan() && v2.is_nan() {
                    continue;
                }
                assert_eq!(
                    v1, v2,
                    "{file_name} '{channel_name}': value_at({idx}) mismatch: got {v2}, want {v1}"
                );
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(e)) => {
                panic!("{file_name} '{channel_name}': value_at({idx}) open succeeded but from_bytes failed: {e}")
            }
            (Err(e), Ok(_)) => {
                panic!("{file_name} '{channel_name}': value_at({idx}) open failed ({e}) but from_bytes succeeded")
            }
        }
    }
}

struct SyntheticFixture {
    name: &'static str,
    bytes: Vec<u8>,
}

fn fixture_comprehensive_uncompressed() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    let times = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5];
    let n = times.len();
    let group = writer.add_group(&times).unwrap();

    // Floats
    group
        .add_channel_typed(
            "F64_Chan",
            "V",
            SignalValues::F64(vec![1.1, 2.2, 3.3, 4.4, 5.5, 6.6, 7.7, 8.8]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "F32_Chan",
            "A",
            SignalValues::F32(vec![10.5, 20.5, 30.5, 40.5, 50.5, 60.5, 70.5, 80.5]),
        )
        .unwrap();

    // Unsigned and Signed Integers (all widths)
    group
        .add_channel_typed(
            "U8_Chan",
            "count",
            SignalValues::U8(vec![0, 1, 2, 127, 128, 200, 254, 255]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I8_Chan",
            "degC",
            SignalValues::I8(vec![-128, -50, -1, 0, 1, 50, 126, 127]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "U16_Chan",
            "rpm",
            SignalValues::U16(vec![0, 100, 1000, 5000, 10000, 32767, 65534, 65535]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I16_Chan",
            "Nm",
            SignalValues::I16(vec![-32768, -1000, -1, 0, 1, 1000, 32766, 32767]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "U32_Chan",
            "ticks",
            SignalValues::U32(vec![
                0, 1000, 1000000, 2147483648, 3000000000, 4000000000, 4294967294, 4294967295,
            ]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I32_Chan",
            "Pa",
            SignalValues::I32(vec![
                -2147483648,
                -100000,
                -1,
                0,
                1,
                100000,
                2147483646,
                2147483647,
            ]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "U64_Chan",
            "ns",
            SignalValues::U64(vec![
                0,
                1000,
                1000000000,
                9000000000000000000,
                18000000000000000000,
                18446744073709551614,
                18446744073709551615,
                42,
            ]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "I64_Chan",
            "diff",
            SignalValues::I64(vec![
                i64::MIN,
                -9000000000000000000,
                -1,
                0,
                1,
                9000000000000000000,
                i64::MAX - 1,
                i64::MAX,
            ]),
        )
        .unwrap();

    // Fixed-length string channel
    group
        .add_channel_typed(
            "FixedStr_Chan",
            "",
            SignalValues::Str(vec![
                "START".into(),
                "RUN".into(),
                "WARN".into(),
                "CRIT".into(),
                "IDLE".into(),
                "STOP".into(),
                "TEST".into(),
                "DONE".into(),
            ]),
        )
        .unwrap();

    // Fixed-width bytes channel
    let raw_bytes: Vec<u8> = (0..(n * 4)).map(|i| (i * 31 + 7) as u8).collect();
    group
        .add_channel_typed(
            "FixedBytes_Chan",
            "",
            SignalValues::Bytes {
                data: raw_bytes,
                width: 4,
            },
        )
        .unwrap();

    // VLSD String channel
    group
        .add_channel_vlsd_str(
            "VlsdStr_Chan",
            "",
            &[
                "short",
                "",
                "medium string length payload",
                "another one",
                "a fairly long string that will verify dynamic sizing in vlsd blocks without issue",
                "x",
                "final",
                "end",
            ],
        )
        .unwrap();

    // VLSD Bytes channel
    group
        .add_channel_vlsd_bytes(
            "VlsdBytes_Chan",
            "",
            &[
                b"data",
                b"",
                b"\x00\x01\x02\x03\x04\x05\x06\x07\x08",
                b"hello world bytes",
                b"\xff\xfe\xfd",
                b"1234567890",
                b"a",
                b"tail",
            ],
        )
        .unwrap();

    // Channel with validity mask
    group
        .add_channel_with_validity(
            "Validity_Chan",
            "m/s",
            &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0],
            Some(&[true, false, true, true, false, true, false, true]),
        )
        .unwrap();

    // Channel with Linear conversion
    group
        .add_channel_typed_with(
            "LinearConv_Chan",
            "degC",
            SignalValues::U16(vec![0, 10, 20, 30, 40, 50, 60, 70]),
            None,
            Some(Conversion::Linear {
                offset: -40.0,
                factor: 0.1,
            }),
        )
        .unwrap();

    // Channel with Rational conversion
    group
        .add_channel_typed_with(
            "RationalConv_Chan",
            "ratio",
            SignalValues::U16(vec![100, 200, 300, 400, 500, 600, 700, 800]),
            None,
            Some(Conversion::Rational {
                coefficients: [2.0, 1.0, 0.0, 0.0, 0.0, 5.0],
            }),
        )
        .unwrap();

    // Channel with TableInterpolated conversion
    group
        .add_channel_typed_with(
            "TableConv_Chan",
            "pct",
            SignalValues::F64(vec![0.0, 2.5, 5.0, 7.5, 10.0, 12.5, 15.0, 20.0]),
            None,
            Some(Conversion::TableInterpolated {
                keys: vec![0.0, 10.0, 20.0],
                values: vec![0.0, 50.0, 100.0],
            }),
        )
        .unwrap();

    // Channel with ValueToText conversion
    group
        .add_channel_typed_with(
            "ValueToText_Chan",
            "",
            SignalValues::U8(vec![0, 1, 2, 0, 1, 2, 99, 1]),
            None,
            Some(Conversion::ValueToText {
                keys: vec![0.0, 1.0, 2.0],
                entries: vec![
                    TableEntry::Text("OFF".into()),
                    TableEntry::Text("ON".into()),
                    TableEntry::Text("STANDBY".into()),
                ],
                default: Some(TableEntry::Text("FAULT".into())),
            }),
        )
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_comprehensive_uncompressed",
        bytes,
    }
}

fn fixture_compressed_deflate() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_600_000_000_000_000_000);
    writer.set_compression(true);
    writer.set_codec(WriteCodec::Deflate);

    let times: Vec<f64> = (0..50).map(|i| i as f64 * 0.1).collect();
    let group = writer.add_group(&times).unwrap();

    let f64_vals: Vec<f64> = (0..50).map(|i| (i as f64 * 0.2).sin()).collect();
    let u32_vals: Vec<u32> = (0..50).map(|i| (i * 100) as u32).collect();
    let valid_mask: Vec<bool> = (0..50).map(|i| i % 5 != 0).collect();

    group.add_channel("SineWave", "V", &f64_vals).unwrap();
    group
        .add_channel_typed("Counter", "ticks", SignalValues::U32(u32_vals))
        .unwrap();
    group
        .add_channel_with_validity("SineValid", "V", &f64_vals, Some(&valid_mask))
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_compressed_deflate",
        bytes,
    }
}

fn fixture_compressed_transposed_deflate() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_650_000_000_000_000_000);
    writer.set_compression(true);
    writer.set_codec(WriteCodec::TransposedDeflate);

    let times: Vec<f64> = (0..60).map(|i| i as f64 * 0.05).collect();
    let group = writer.add_group(&times).unwrap();

    let f32_vals: Vec<f32> = (0..60).map(|i| i as f32 * 1.5).collect();
    let i16_vals: Vec<i16> = (0..60).map(|i| i as i16 * 10 - 300).collect();

    group
        .add_channel_typed("Temp_F32", "degC", SignalValues::F32(f32_vals))
        .unwrap();
    group
        .add_channel_typed("Torque_I16", "Nm", SignalValues::I16(i16_vals))
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_compressed_transposed_deflate",
        bytes,
    }
}

#[cfg(feature = "lz4")]
fn fixture_compressed_lz4() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_660_000_000_000_000_000);
    writer.set_compression(true);
    writer.set_codec(WriteCodec::Lz4);

    let times: Vec<f64> = (0..40).map(|i| i as f64 * 0.1).collect();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel(
            "Lz4_F64",
            "bar",
            &(0..40).map(|i| i as f64 * 2.5).collect::<Vec<_>>(),
        )
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_compressed_lz4",
        bytes,
    }
}

#[cfg(feature = "lz4")]
fn fixture_compressed_transposed_lz4() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_670_000_000_000_000_000);
    writer.set_compression(true);
    writer.set_codec(WriteCodec::TransposedLz4);

    let times: Vec<f64> = (0..40).map(|i| i as f64 * 0.1).collect();
    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_typed(
            "TransposedLz4_U32",
            "count",
            SignalValues::U32((0..40).map(|i| i * 250).collect()),
        )
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_compressed_transposed_lz4",
        bytes,
    }
}

fn fixture_multi_group_and_sibling_cgs() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(1_500_000_000_000_000_000);

    // DG 0, CG 0
    let times1 = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
    let g1 = writer.add_group(&times1).unwrap();
    g1.add_channel("Speed_G1", "km/h", &[0.0, 10.0, 20.0, 30.0, 40.0, 50.0])
        .unwrap();
    g1.add_channel_typed("Gear_G1", "", SignalValues::U8(vec![1, 1, 2, 2, 3, 3]))
        .unwrap();

    // DG 0, CG 1 (sibling sharing DG 0)
    let times2 = vec![0.1, 0.3, 0.5, 0.7, 0.9];
    let g2 = writer.add_group_in(0, &times2).unwrap();
    g2.add_channel("Brake_G2", "bar", &[0.0, 5.0, 15.0, 25.0, 0.0])
        .unwrap();
    g2.add_channel_vlsd_str("Status_G2", "", &["OK", "OK", "BRAKING", "BRAKING", "OK"])
        .unwrap();

    // DG 1, CG 0 (separate DG)
    let times3 = vec![0.0, 0.5, 1.0];
    let g3 = writer.add_group(&times3).unwrap();
    g3.add_channel("AmbientTemp_G3", "degC", &[21.5, 21.6, 21.7])
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_multi_group_and_sibling_cgs",
        bytes,
    }
}

fn fixture_edge_cases_and_unsorted_times() -> SyntheticFixture {
    let mut writer = Mf4Writer::with_start_time_ns(0);

    // Unsorted timestamps (writer sorts them)
    let times = vec![3.0, 1.0, 0.0, 2.0];
    let g = writer.add_group(&times).unwrap();
    g.add_channel("UnsortedVals", "m", &[30.0, 10.0, 0.0, 20.0])
        .unwrap();

    // Extreme integer values
    g.add_channel_typed(
        "ExtremeI64",
        "",
        SignalValues::I64(vec![i64::MIN, -1, 0, i64::MAX]),
    )
    .unwrap();
    g.add_channel_typed(
        "ExtremeU64",
        "",
        SignalValues::U64(vec![0, 1, u64::MAX / 2, u64::MAX]),
    )
    .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    SyntheticFixture {
        name: "synthetic_edge_cases_and_unsorted_times",
        bytes,
    }
}

fn all_synthetic_fixtures() -> Vec<SyntheticFixture> {
    #[allow(unused_mut)]
    let mut fixtures = vec![
        fixture_comprehensive_uncompressed(),
        fixture_compressed_deflate(),
        fixture_compressed_transposed_deflate(),
        fixture_multi_group_and_sibling_cgs(),
        fixture_edge_cases_and_unsorted_times(),
    ];
    #[cfg(feature = "lz4")]
    {
        fixtures.push(fixture_compressed_lz4());
        fixtures.push(fixture_compressed_transposed_lz4());
    }
    fixtures
}

fn compare_file_backends(file_open: &Mf4File, file_mem: &Mf4File, file_name: &str) -> usize {
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
    assert_eq!(
        file_open.start_time().timestamp_ns,
        file_mem.start_time().timestamp_ns,
        "{file_name}: start time mismatch"
    );

    let channels_open: Vec<_> = file_open.channels().cloned().collect();
    let channels_mem: Vec<_> = file_mem.channels().cloned().collect();

    assert_eq!(
        channels_open.len(),
        channels_mem.len(),
        "{file_name}: channels list length mismatch"
    );

    let mut verified_channels = 0;
    for (ch_open, ch_mem) in channels_open.iter().zip(channels_mem.iter()) {
        assert_eq!(
            ch_open.name, ch_mem.name,
            "{file_name}: channel name mismatch"
        );
        assert_eq!(
            ch_open.unit, ch_mem.unit,
            "{file_name} '{}': unit mismatch",
            ch_open.name
        );
        assert_eq!(
            ch_open.channel_type, ch_mem.channel_type,
            "{file_name} '{}': channel_type mismatch",
            ch_open.name
        );
        assert_eq!(
            ch_open.bit_count, ch_mem.bit_count,
            "{file_name} '{}': bit_count mismatch",
            ch_open.name
        );
        assert_eq!(
            ch_open.data_type, ch_mem.data_type,
            "{file_name} '{}': data_type mismatch",
            ch_open.name
        );

        let sig_open = file_open.signal(ch_open);
        let sig_mem = file_mem.signal(ch_mem);

        match (sig_open, sig_mem) {
            (Ok(s_open), Ok(s_mem)) => {
                assert_signals_match(&s_open, &s_mem, &ch_open.name, file_name);
                verified_channels += 1;
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
    verified_channels
}

#[test]
fn from_bytes_matches_open_for_every_sample_of_every_channel() {
    let synthetic = all_synthetic_fixtures();
    assert!(
        !synthetic.is_empty(),
        "Synthetic fixtures must always be present"
    );

    let mut total_synthetic_channels = 0;
    for fixture in &synthetic {
        let temp = tempfile::NamedTempFile::new().expect("create temp file");
        std::fs::write(temp.path(), &fixture.bytes).expect("write temp file");

        let file_open = Mf4File::open(temp.path())
            .unwrap_or_else(|e| panic!("Mf4File::open failed on {}: {e}", fixture.name));
        let file_mem = Mf4File::from_bytes(fixture.bytes.clone())
            .unwrap_or_else(|e| panic!("Mf4File::from_bytes failed on {}: {e}", fixture.name));
        let file_buf = Mf4File::open_buffered(temp.path())
            .unwrap_or_else(|e| panic!("Mf4File::open_buffered failed on {}: {e}", fixture.name));

        let verified = compare_file_backends(&file_open, &file_mem, fixture.name);
        let _ = compare_file_backends(&file_buf, &file_mem, fixture.name);
        assert!(
            verified > 0,
            "{}: expected at least 1 verified channel, got 0",
            fixture.name
        );
        total_synthetic_channels += verified;
    }
    assert!(
        total_synthetic_channels >= 20,
        "Expected at least 20 synthetic channels verified, got {total_synthetic_channels}"
    );

    // If reference files are present on disk, also test them
    let ref_files = reference_files();
    for path in &ref_files {
        let file_name = path.file_name().unwrap().to_string_lossy();
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("Failed to read {path:?}: {e}"));

        let file_open =
            Mf4File::open(path).unwrap_or_else(|e| panic!("Mf4File::open failed on {path:?}: {e}"));
        let file_mem = Mf4File::from_bytes(bytes)
            .unwrap_or_else(|e| panic!("Mf4File::from_bytes failed on {path:?}: {e}"));

        compare_file_backends(&file_open, &file_mem, &file_name);
    }
}

#[test]
fn pure_in_memory_from_bytes_without_disk() {
    let fixture = fixture_comprehensive_uncompressed();
    let bytes = fixture.bytes;

    // Open entirely from in-memory byte buffer
    let file = Mf4File::from_bytes(bytes.clone())
        .expect("Mf4File::from_bytes must succeed on in-memory bytes");

    assert_eq!(file.version().to_string(), "4.11");
    assert_eq!(file.start_time().timestamp_ns, 1_700_000_000_000_000_000);
    assert_eq!(file.data_group_count(), 1);
    assert!(file.channel_count() > 10);

    // Channel lookup by name in memory
    let f64_ch = file.find_channel("F64_Chan").expect("find F64_Chan");
    let sig = file.signal(f64_ch).expect("read F64_Chan signal");
    assert_eq!(sig.len(), 8);
    let vals = sig.values_f64().expect("values_f64");
    assert_eq!(vals, vec![1.1, 2.2, 3.3, 4.4, 5.5, 6.6, 7.7, 8.8]);

    let u8_ch = file.find_channel("U8_Chan").expect("find U8_Chan");
    let sig_u8 = file.signal(u8_ch).expect("read U8_Chan signal");
    assert_eq!(
        sig_u8.raw_values().expect("raw_values"),
        SignalValues::U8(vec![0, 1, 2, 127, 128, 200, 254, 255])
    );

    let str_ch = file
        .find_channel("FixedStr_Chan")
        .expect("find FixedStr_Chan");
    let sig_str = file.signal(str_ch).expect("read FixedStr_Chan signal");
    assert_eq!(
        sig_str.values().expect("values"),
        SignalValues::Str(vec![
            "START".into(),
            "RUN".into(),
            "WARN".into(),
            "CRIT".into(),
            "IDLE".into(),
            "STOP".into(),
            "TEST".into(),
            "DONE".into(),
        ])
    );

    let vlsd_str_ch = file
        .find_channel("VlsdStr_Chan")
        .expect("find VlsdStr_Chan");
    let sig_vlsd_str = file.signal(vlsd_str_ch).expect("read VlsdStr_Chan signal");
    assert_eq!(
        sig_vlsd_str.values().expect("values"),
        SignalValues::Str(vec![
            "short".into(),
            "".into(),
            "medium string length payload".into(),
            "another one".into(),
            "a fairly long string that will verify dynamic sizing in vlsd blocks without issue"
                .into(),
            "x".into(),
            "final".into(),
            "end".into(),
        ])
    );

    let inval_ch = file
        .find_channel("Validity_Chan")
        .expect("find Validity_Chan");
    let sig_inval = file.signal(inval_ch).expect("read Validity_Chan signal");
    assert_eq!(
        sig_inval.validity(),
        Some(vec![true, false, true, true, false, true, false, true])
    );

    let conv_ch = file
        .find_channel("LinearConv_Chan")
        .expect("find LinearConv_Chan");
    let sig_conv = file.signal(conv_ch).expect("read LinearConv_Chan signal");
    let conv_vals = sig_conv.values_f64().expect("converted values_f64");
    assert_eq!(
        conv_vals,
        vec![-40.0, -39.0, -38.0, -37.0, -36.0, -35.0, -34.0, -33.0]
    );

    // Also test with explicit OpenOptions
    let custom_options = OpenOptions {
        build_channels_db: true,
        parallel_parsing: false,
        parallel_threshold: 50,
        max_alloc: 16 * 1024 * 1024,
        max_decompressed: 64 * 1024 * 1024,
    };
    let file_opts = Mf4File::from_bytes_with_options(bytes, custom_options)
        .expect("from_bytes_with_options must succeed");
    assert_eq!(file_opts.channel_count(), file.channel_count());
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

    // 2. Unconditional synthetic file truncation checks (NEVER SKIPS)
    let synth_fixtures = all_synthetic_fixtures();
    for fixture in &synth_fixtures {
        let full_bytes = &fixture.bytes;
        assert!(
            full_bytes.len() > 128,
            "{}: fixture byte length too small ({})",
            fixture.name,
            full_bytes.len()
        );

        let cutoffs = [
            0,
            1,
            2,
            8,
            16,
            24,
            32,
            63,
            64,
            80,
            100,
            full_bytes.len() / 4,
            full_bytes.len() / 2,
            full_bytes.len() - 1,
        ];

        for &cutoff in &cutoffs {
            let truncated = full_bytes[..cutoff].to_vec();
            let result = Mf4File::from_bytes(truncated);
            assert!(
                result.is_err(),
                "{}: from_bytes on truncated file ({cutoff}/{} bytes) must return Err, not Ok",
                fixture.name,
                full_bytes.len()
            );
        }

        // Specific check that a partway-truncated file reports a truncation error or missing block
        let mid_truncated = full_bytes[..full_bytes.len() / 2].to_vec();
        let err = Mf4File::from_bytes(mid_truncated).unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("truncated")
                || err_str.contains("File is truncated")
                || err_str.contains("Missing required block")
                || err_str.contains("Invalid block header")
                || err_str.contains("Unexpected EOF")
                || matches!(err, Mf4Error::TruncatedFile { .. }),
            "{}: error for truncated file should name truncation or missing/invalid block: {err_str}",
            fixture.name
        );
    }

    // 3. Corrupted blocks and out-of-range link bounds (NEVER SKIPS)
    let base_fixture = fixture_comprehensive_uncompressed();

    // Corrupted magic bytes in ID block
    let mut corrupt_magic = base_fixture.bytes.clone();
    corrupt_magic[0..8].copy_from_slice(b"BAD_MDF ");
    let err = Mf4File::from_bytes(corrupt_magic).unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("Invalid file identifier") || err_str.contains("MDF"),
        "Magic bytes corruption error must identify bad format: {err_str}"
    );

    // Corrupted version in ID block
    let mut corrupt_version = base_fixture.bytes.clone();
    corrupt_version[8..16].copy_from_slice(b"9.99    ");
    corrupt_version[28..30].copy_from_slice(&999u16.to_le_bytes());
    let err = Mf4File::from_bytes(corrupt_version).unwrap_err();
    assert!(
        err.to_string().contains("version")
            || err.to_string().contains("9.99")
            || err.to_string().contains("999")
            || err.to_string().contains("Unsupported"),
        "Version corruption must return error: {err}"
    );

    // Corrupted HD block identifier (at offset 64)
    let mut corrupt_hd = base_fixture.bytes.clone();
    corrupt_hd[64..68].copy_from_slice(b"##XX");
    let err = Mf4File::from_bytes(corrupt_hd).unwrap_err();
    assert!(
        err.to_string().contains("HD")
            || err.to_string().contains("header")
            || err.to_string().contains("block")
            || err.to_string().contains("Invalid"),
        "Corrupted HD block magic must return error: {err}"
    );

    // Corrupted HD block length field (at offset 72)
    let mut corrupt_hd_len = base_fixture.bytes.clone();
    corrupt_hd_len[72..80].copy_from_slice(&u64::MAX.to_le_bytes());
    let err = Mf4File::from_bytes(corrupt_hd_len).unwrap_err();
    assert!(
        err.to_string().contains("truncated")
            || err.to_string().contains("exceeds")
            || err.to_string().contains("block")
            || matches!(err, Mf4Error::TruncatedFile { .. }),
        "Corrupted HD length must return truncation/out-of-bounds error: {err}"
    );

    // Corrupted link offset pointing beyond buffer length (offset 88 = HD dg_first link)
    let mut corrupt_dg_link = base_fixture.bytes.clone();
    corrupt_dg_link[88..96]
        .copy_from_slice(&(base_fixture.bytes.len() as u64 + 100_000).to_le_bytes());
    let err = Mf4File::from_bytes(corrupt_dg_link).unwrap_err();
    assert!(
        err.to_string().contains("truncated")
            || err.to_string().contains("bounds")
            || err.to_string().contains("offset")
            || err.to_string().contains("block")
            || matches!(err, Mf4Error::TruncatedFile { .. }),
        "Corrupted link pointer must return error: {err}"
    );

    // Corrupted DZ block compressed data
    let compressed_fixture = fixture_compressed_deflate();
    let mut corrupt_compressed = compressed_fixture.bytes.clone();
    let mid = corrupt_compressed.len() - 20;
    for b in &mut corrupt_compressed[mid..mid + 10] {
        *b ^= 0xFF;
    }
    // Attempting from_bytes or reading signals on corrupted compressed file must not panic
    if let Ok(corrupt_file) = Mf4File::from_bytes(corrupt_compressed) {
        for ch in corrupt_file.channels() {
            let _ = corrupt_file.signal(ch).and_then(|s| s.values());
        }
    }

    // 4. Reference file truncation (if present on disk)
    let files = reference_files();
    if !files.is_empty() {
        let sample_file = &files[0];
        if let Ok(full_bytes) = std::fs::read(sample_file) {
            if full_bytes.len() > 128 {
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
                        "from_bytes on truncated reference file ({cutoff}/{} bytes) must return Err, not Ok",
                        full_bytes.len()
                    );
                }
            }
        }
    }
}
