//! What the writer emits, the reader — audited against the standard block by
//! block — must read back: names, units, sorted times, exact values, and the
//! invalidation bits. A writer that mislays a link, an offset or a bit fails
//! here; the oracle is not the writer's own expectations but the independent
//! parse path.

use falcon_mdf::{Mf4Error, Mf4File, Mf4Writer};

fn open_written(writer: &Mf4Writer) -> (tempfile::NamedTempFile, Mf4File) {
    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    (temp, file)
}

fn values(file: &Mf4File, name: &str) -> Vec<f64> {
    let channel = file.find_channel(name).unwrap();
    file.signal(channel).unwrap().values_f64().unwrap()
}

fn validity(file: &Mf4File, name: &str) -> Option<Vec<bool>> {
    let channel = file.find_channel(name).unwrap();
    file.signal(channel).unwrap().validity()
}

fn master_times(file: &Mf4File, group: usize) -> Vec<f64> {
    let cg = &file.data_groups()[group].channel_groups[0];
    let master = cg
        .channels
        .iter()
        .find(|c| c.channel_type.is_master())
        .unwrap();
    file.signal(master).unwrap().values_f64().unwrap()
}

#[test]
fn unsorted_input_reads_back_sorted_with_values_in_step() {
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    let group = writer.add_group(&[2.0, 0.0, 1.0]).unwrap();
    group
        .add_channel("Speed", "km/h", &[20.0, 0.0, 10.0])
        .unwrap();

    let (_temp, file) = open_written(&writer);

    assert_eq!(master_times(&file, 0), vec![0.0, 1.0, 2.0]);
    assert_eq!(values(&file, "Speed"), vec![0.0, 10.0, 20.0]);
    assert_eq!(values(&file, "Time"), vec![0.0, 1.0, 2.0]);

    let channel = file.find_channel("Speed").unwrap();
    assert_eq!(channel.unit, "km/h");
    assert_eq!(validity(&file, "Speed"), None);

    // The start time survives as the file's own, not the writer process's.
    assert!(file
        .start_time()
        .to_iso8601()
        .starts_with("2023-11-14T22:13:20"));
}

#[test]
fn two_groups_keep_their_channels_apart() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    writer
        .add_group(&[0.0, 1.0])
        .unwrap()
        .add_channel("Speed", "km/h", &[0.0, 1.0])
        .unwrap();
    writer
        .add_group(&[0.0, 0.5, 1.0])
        .unwrap()
        .add_channel("RPM", "1/min", &[100.0, 200.0, 300.0])
        .unwrap();

    let (_temp, file) = open_written(&writer);

    assert_eq!(file.statistics().data_group_count, 2);
    assert_eq!(file.statistics().channel_group_count, 2);
    assert_eq!(values(&file, "RPM"), vec![100.0, 200.0, 300.0]);
    assert_eq!(master_times(&file, 1), vec![0.0, 0.5, 1.0]);
}

#[test]
fn invalid_samples_survive_the_round_trip() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.0, 1.0, 2.0, 3.0]).unwrap();
    group
        .add_channel_with_validity(
            "A",
            "",
            &[0.0, 1.0, 2.0, 3.0],
            Some(&[true, false, true, true]),
        )
        .unwrap();
    group
        .add_channel_with_validity(
            "B",
            "",
            &[4.0, 5.0, 6.0, 7.0],
            Some(&[true, true, false, true]),
        )
        .unwrap();
    group.add_channel("C", "", &[8.0, 9.0, 10.0, 11.0]).unwrap();

    let (_temp, file) = open_written(&writer);

    // A and B share one invalidation byte at different bit positions; C has
    // none and must still read as wholly valid rather than inheriting noise.
    assert_eq!(validity(&file, "A"), Some(vec![true, false, true, true]));
    assert_eq!(validity(&file, "B"), Some(vec![true, true, false, true]));
    assert_eq!(validity(&file, "C"), None);
    assert_eq!(values(&file, "A"), vec![0.0, 1.0, 2.0, 3.0]);
}

#[test]
fn id_and_header_bytes_are_pinned() {
    let writer = Mf4Writer::with_start_time_ns(0);
    let mut buf = Vec::new();
    writer.write(&mut buf).unwrap();

    assert_eq!(&buf[0..8], b"MDF     ");
    assert_eq!(&buf[8..16], b"4.11    ");
    assert_eq!(u16::from_le_bytes([buf[28], buf[29]]), 411);
    assert_eq!(u16::from_le_bytes([buf[60], buf[61]]), 0); // finalized
    assert_eq!(&buf[64..68], b"##HD");
}

#[test]
fn mismatched_lengths_are_refused() {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 1.0]).unwrap();
    let err = group.add_channel("X", "", &[1.0]).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));

    let err = group
        .add_channel_with_validity("Y", "", &[1.0, 2.0], Some(&[true]))
        .unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
}

#[test]
fn nan_timestamps_are_refused() {
    let mut writer = Mf4Writer::new();
    let err = writer.add_group(&[0.0, f64::NAN]).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
}

#[test]
fn an_empty_file_opens_with_no_channels() {
    let writer = Mf4Writer::with_start_time_ns(0);
    let (_temp, file) = open_written(&writer);
    assert_eq!(file.statistics().channel_count, 0);
}

#[test]
fn a_compressed_file_and_a_plain_one_hold_the_same_measurement() {
    // Not a substitute for the asammdf check in `write_typed_conformance.rs` —
    // our reader agreeing with our writer proves only that they agree. What
    // this adds is that turning compression on changes nothing but the bytes:
    // the two files must decode to the same numbers through the same reader.
    let times: Vec<f64> = (0..500).map(|i| f64::from(i) * 0.02).collect();
    let values: Vec<f64> = times.iter().map(|t| t.sin() * 10.0).collect();

    let write = |compress: bool| {
        let file = tempfile::Builder::new().suffix(".mf4").tempfile().unwrap();
        let mut writer = Mf4Writer::with_start_time_ns(0);
        writer.set_compression(compress);
        let group = writer.add_group(&times).unwrap();
        group.add_channel("Wave", "V", &values).unwrap();
        writer.write_to_file(file.path()).unwrap();
        file
    };

    let plain = write(false);
    let zipped = write(true);
    assert!(
        std::fs::metadata(zipped.path()).unwrap().len()
            < std::fs::metadata(plain.path()).unwrap().len(),
        "the compressed file should be the smaller one"
    );

    let read = |path: &std::path::Path| {
        let f = Mf4File::open(path).unwrap();
        let ch = f.find_channel("Wave").unwrap().clone();
        let values = f.signal(&ch).unwrap().values_f64().unwrap();
        (values, master_times(&f, 0))
    };
    let (a_values, a_times) = read(plain.path());
    let (b_values, b_times) = read(zipped.path());
    assert_eq!(
        a_values, b_values,
        "compression must not change the samples"
    );
    assert_eq!(
        a_times, b_times,
        "compression must not change the time axis"
    );
    assert_eq!(a_values, values, "and both must be what was written");
}

#[test]
fn every_written_block_starts_on_8_byte_boundary() {
    use falcon_mdf::{Conversion, SignalValues, TableEntry};

    fn align8(n: u64) -> u64 {
        n.div_ceil(8) * 8
    }

    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);

    // DG 0: unaligned TX lengths, unaligned SD length, unaligned DT (9-byte record = 33-byte DT),
    // conversion with TX ref, array with CA.
    let g0 = writer.add_group(&[0.0]).unwrap();
    g0.add_channel_full(
        "A",
        "V",
        "odd_cmt",
        SignalValues::U8(vec![7]),
        None,
        Some(Conversion::ValueToText {
            keys: vec![7.0],
            entries: vec![TableEntry::Text("odd".into())],
            default: None,
        }),
    )
    .unwrap();
    g0.add_channel_vlsd_str("S1", "", &["ab"]).unwrap();
    g0.add_channel_array(
        "Mat",
        "m",
        &[2],
        SignalValues::Array {
            values: vec![1.0, 2.0],
            elements_per_sample: 2,
        },
    )
    .unwrap();

    // DG 1: follows DG 0's DT directly, testing inter-DG alignment after an unaligned DT.
    let g1 = writer.add_group(&[0.0, 1.0]).unwrap();
    g1.add_channel_typed("Tiny", "mm", SignalValues::U8(vec![3, 4]))
        .unwrap();

    let mut plain_bytes = Vec::new();
    writer.write(&mut plain_bytes).unwrap();

    // Also test a compressed writer so ##HL, ##DL, and ##DZ are walked.
    writer.set_compression(true);
    let mut comp_bytes = Vec::new();
    writer.write(&mut comp_bytes).unwrap();

    for (label, bytes) in [("plain", &plain_bytes), ("compressed", &comp_bytes)] {
        let mut offset = 64u64;
        let mut seen = std::collections::BTreeSet::new();
        while (offset as usize) < bytes.len() {
            assert_eq!(
                offset % 8,
                0,
                "{label}: block at offset {offset} is not 8-byte aligned"
            );
            let pos = offset as usize;
            let id: [u8; 4] = bytes[pos..pos + 4].try_into().unwrap();
            assert_eq!(
                &id[0..2],
                b"##",
                "{label}: expected block header at {offset}, found {id:?}"
            );
            let length = u64::from_le_bytes(bytes[pos + 8..pos + 16].try_into().unwrap());
            assert!(length >= 24, "{label}: invalid block length {length}");
            if matches!(&id, b"##TX" | b"##MD") {
                assert_eq!(
                    length % 8,
                    0,
                    "{label}: text/metadata block {id:?} length {length} should be 8-byte aligned"
                );
            }
            seen.insert(String::from_utf8_lossy(&id).to_string());
            let end = offset + length;
            if (end as usize) == bytes.len() {
                offset = end;
            } else {
                offset = align8(end);
            }
        }
        assert_eq!(
            offset as usize,
            bytes.len(),
            "{label}: sequential walk did not land on EOF"
        );
        for required in ["##HD", "##FH", "##MD", "##DG", "##CG", "##CN", "##TX"] {
            assert!(seen.contains(required), "{label}: missing {required}");
        }
        if label == "plain" {
            assert!(seen.contains("##DT"));
            assert!(seen.contains("##SD"));
            assert!(seen.contains("##CC"));
            assert!(seen.contains("##CA"));
        } else {
            assert!(seen.contains("##HL"));
            assert!(seen.contains("##DL"));
            assert!(seen.contains("##DZ"));
        }

        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), bytes).unwrap();
        let file = Mf4File::open(temp.path()).unwrap();
        let map = file.block_map();
        assert!(
            map.warnings.is_empty(),
            "{label}: unexpected block_map warnings: {:?}",
            map.warnings
        );
        for block in &map.blocks {
            assert_eq!(
                block.address % 8,
                0,
                "{label}: reachable block {} at {:#x} is not 8-byte aligned",
                block.block_type,
                block.address
            );
        }
    }
}

#[test]
fn file_history_uses_aligned_md_xml_and_consistent_negative_start_time() {
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    writer
        .add_group(&[0.0, 1.0])
        .unwrap()
        .add_channel("X", "V", &[1.0, 2.0])
        .unwrap();

    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();

    // HD starts at 64; link 1 (offset 64 + 24 + 8 = 96) is fh_first.
    let fh_off = u64::from_le_bytes(bytes[96..104].try_into().unwrap()) as usize;
    assert_eq!(fh_off % 8, 0);
    assert_eq!(&bytes[fh_off..fh_off + 4], b"##FH");
    // FH link 1 (offset fh_off + 24 + 8) is fh_md_comment.
    let md_off = u64::from_le_bytes(bytes[fh_off + 32..fh_off + 40].try_into().unwrap()) as usize;
    assert_eq!(md_off % 8, 0);
    assert_eq!(&bytes[md_off..md_off + 4], b"##MD");

    let temp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp.path(), &bytes).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let history = file.file_history();
    assert_eq!(history.len(), 1);
    let created = &history[0];
    assert!(created.comment.starts_with("created by falcon_mdf "));
    assert_eq!(created.tool_id(), Some("falcon_mdf"));
    assert_eq!(created.tool_vendor(), Some("falcon_mdf"));
    assert_eq!(
        created.metadata.get("tool_version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(created.metadata.xml().contains("<FHcomment>"));

    // Negative start_time_ns must be handled consistently between ##HD and ##FH.
    let neg_writer = Mf4Writer::with_start_time_ns(-123_456);
    let mut neg_bytes = Vec::new();
    neg_writer.write(&mut neg_bytes).unwrap();
    let neg_fh_off = u64::from_le_bytes(neg_bytes[96..104].try_into().unwrap()) as usize;
    let hd_time = u64::from_le_bytes(neg_bytes[64 + 24 + 48..64 + 24 + 56].try_into().unwrap());
    let fh_time = u64::from_le_bytes(
        neg_bytes[neg_fh_off + 40..neg_fh_off + 48]
            .try_into()
            .unwrap(),
    );
    assert_eq!(hd_time, fh_time);
    assert_eq!(hd_time, 0);

    let (_tmp, neg_file) = open_written(&neg_writer);
    assert_eq!(neg_file.start_time().timestamp_ns, 0);
    assert_eq!(neg_file.file_history()[0].time.timestamp_ns, 0);
}

#[test]
fn non_finite_timestamps_are_rejected_in_add_group_and_set_times() {
    let mut writer = Mf4Writer::new();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = writer.add_group(&[0.0, bad]).unwrap_err();
        assert!(matches!(err, Mf4Error::WriteError { .. }));
    }

    let g = writer.add_group(&[0.0, 1.0]).unwrap();
    g.add_channel("X", "", &[1.0, 2.0]).unwrap();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = g.set_times(&[0.0, bad]).unwrap_err();
        assert!(matches!(err, Mf4Error::WriteError { .. }));
    }

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = writer.add_group_in(0, &[0.0, bad]).unwrap_err();
        assert!(matches!(err, Mf4Error::WriteError { .. }));
    }
}

#[test]
fn checked_integer_overflow_returns_write_error() {
    use falcon_mdf::{Conversion, SignalValues};

    // > u16::MAX values in a ##CC block must return WriteError rather than truncate.
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let g = writer.add_group(&[0.0]).unwrap();
    let keys: Vec<f64> = (0..33_000).map(|i| i as f64).collect();
    let vals: Vec<f64> = (0..33_000).map(|i| i as f64).collect();
    let err = g
        .add_channel_typed_with(
            "HugeTable",
            "",
            SignalValues::F64(vec![0.0]),
            None,
            Some(Conversion::TableLookup { keys, values: vals }),
        )
        .unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));

    // Record data size exceeding u32::MAX must return WriteError from write() / write_cg.
    let mut wide_writer = Mf4Writer::with_start_time_ns(0);
    let wg = wide_writer.add_group(&[]).unwrap();
    wg.add_channel_typed(
        "WideA",
        "",
        SignalValues::Bytes {
            data: Vec::new(),
            width: 3_000_000_000,
        },
    )
    .unwrap();
    wg.add_channel_typed(
        "WideB",
        "",
        SignalValues::Bytes {
            data: Vec::new(),
            width: 3_000_000_000,
        },
    )
    .unwrap();
    let mut out = Vec::new();
    let err = wide_writer.write(&mut out).unwrap_err();
    assert!(matches!(err, Mf4Error::WriteError { .. }));
}
