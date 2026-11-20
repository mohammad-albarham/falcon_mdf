//! CAN frame extraction checked against real bus logs.
//!
//! The frames come out of the CANedge sample corpus — a J1939 truck, an OBD2
//! car, and a logger's own internal bus — not out of a fixture written to agree
//! with this reader. That distinction is the point: a synthetic bus record would
//! encode whatever layout the code already assumes, and the 0.3.0 audit found
//! five defects that exactly such fixtures had pinned in place.
//!
//! What holds the assembly honest here is that each check has a source outside
//! this crate:
//!
//! - The OBD2 log can only contain `0x7DF` (the broadcast request) and `0x7E8`
//!   (the first ECU's response), both standard 11-bit identifiers. Those two
//!   numbers are fixed by ISO 15765, not by anything in this repository, and a
//!   misplaced identifier field would not produce them.
//! - The J1939 log can only contain extended 29-bit identifiers, and the ones it
//!   contains have to decompose into real parameter group numbers.
//! - The logger writes one channel group per bus and *names* it after that bus,
//!   so a group called `CAN9_Rx` must yield frames whose bus channel is 9. The
//!   name comes from a text block and the bus channel from a two-bit field
//!   inside the record; agreement means two unrelated paths tell the same story.
//! - `tests/data/golden.json` already pins every frame *channel* — identifier,
//!   payload, length, timestamp — so frame assembly is checked against readings
//!   that were locked down before it existed.
//!
//! The corpus is not checked in. These tests skip when it is absent, as
//! `golden.rs` and `reference.rs` do.

use falcon_mdf::{Mf4File, SignalValues};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Relative tolerance for timestamps, as `golden.rs` uses. Decoding is exact;
/// this absorbs the decimal round-trip through golden.json, which is where a
/// timestamp of 1210048.8090000001 comes back as 1210048.809.
const REL_TOL: f64 = 1e-9;

fn close(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= REL_TOL * scale
}

fn load_golden() -> Value {
    let raw = include_str!("data/golden.json");
    serde_json::from_str(raw).expect("golden.json is malformed")
}

/// The corpus files that hold bus logs, in golden.json's order.
fn bus_log_paths() -> Vec<String> {
    let golden = load_golden();
    let files = golden.as_object().expect("golden root must be an object");
    files
        .keys()
        .filter(|path| Path::new(path).exists())
        .cloned()
        .collect()
}

/// Maps `(data_group_index, channel_group_index)` to golden.json's flat group
/// index, which numbers channel groups sequentially across the whole file.
fn group_indices(file: &Mf4File) -> HashMap<(usize, usize), usize> {
    let mut map = HashMap::new();
    let mut flat = 0usize;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            map.insert((dg.index, cg.index), flat);
            flat += 1;
        }
    }
    map
}

/// The bus number a logger encoded in a group's name, as in `CAN9_Rx`.
///
/// `None` for a group named after the frame type rather than the bus, which is
/// what the single-bus logs use.
fn bus_from_group_name(name: &str) -> Option<u8> {
    let digits: String = name
        .strip_prefix("CAN")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn skip_if_empty(paths: &[String]) -> bool {
    if paths.is_empty() {
        eprintln!("SKIP: no corpus files present under test_data/");
        return true;
    }
    false
}

#[test]
fn frames_agree_with_the_golden_frame_channels() {
    let paths = bus_log_paths();
    if skip_if_empty(&paths) {
        return;
    }
    let golden = load_golden();
    let mut checked = 0usize;

    for path in &paths {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let expected = &golden[path]["channels"];
        let flat = group_indices(&file);

        for group in file.can_frame_groups() {
            let gi = flat[&(group.data_group_index, group.index)];
            let frames = file
                .can_frames(group)
                .unwrap_or_else(|e| panic!("{path} group {gi}: {e}"));

            let want_ids = &expected[format!("{gi}:CAN_DataFrame.ID")];
            assert_eq!(
                frames.len() as u64,
                want_ids["n"].as_u64().unwrap(),
                "{path} group {gi}: frame count"
            );
            if frames.is_empty() {
                continue;
            }
            checked += 1;

            let first = |key: &str| -> Vec<f64> {
                expected[format!("{gi}:CAN_DataFrame.{key}")]["first"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{path} group {gi}: no golden {key}"))
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect()
            };
            let want_id = first("ID");
            let want_ide = first("IDE");
            let want_bus = first("BusChannel");
            let want_len = first("DataLength");
            let want_time: Vec<f64> = expected[format!("{gi}:Timestamp")]["first"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();

            for i in 0..want_id.len().min(frames.len()) {
                let frame = frames.get(i).expect("frame within length");
                assert_eq!(
                    u64::from(frame.id),
                    want_id[i] as u64,
                    "{path} group {gi} frame {i}: identifier"
                );
                assert_eq!(
                    frame.extended,
                    Some(want_ide[i] != 0.0),
                    "{path} group {gi} frame {i}: extended flag"
                );
                assert_eq!(
                    f64::from(frame.bus_channel),
                    want_bus[i],
                    "{path} group {gi} frame {i}: bus channel"
                );
                assert_eq!(
                    frame.data.len() as f64,
                    want_len[i],
                    "{path} group {gi} frame {i}: payload length"
                );
                assert!(
                    close(frame.timestamp, want_time[i]),
                    "{path} group {gi} frame {i}: timestamp {} is not {}",
                    frame.timestamp,
                    want_time[i]
                );
            }

            // The golden payload is the channel's stored sample, which a
            // fixed-width payload channel pads out; a frame trims it back to
            // the logged length, so it must be that sample's leading bytes.
            let stored = expected[format!("{gi}:CAN_DataFrame.DataBytes")]["first_bytes"]
                .as_str()
                .unwrap_or_else(|| panic!("{path} group {gi}: no golden payload"));
            let stored = decode_hex(stored);
            let frame = frames.get(0).unwrap();
            assert_eq!(
                frame.data,
                &stored[..frame.data.len()],
                "{path} group {gi} frame 0: payload"
            );
        }
    }

    assert!(checked > 0, "no non-empty CAN frame group was checked");
    eprintln!("checked {checked} non-empty CAN frame group(s)");
}

fn decode_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("golden payload is hex"))
        .collect()
}

/// The bus number in a group's name and the bus channel in its records are
/// written by different parts of the logger and read by different parts of this
/// crate. They have to agree.
#[test]
fn bus_channel_agrees_with_the_group_name() {
    let paths = bus_log_paths();
    if skip_if_empty(&paths) {
        return;
    }
    let mut checked = 0usize;

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in file.can_frame_groups() {
            let Some(want) = bus_from_group_name(&group.acquisition_name) else {
                continue;
            };
            let frames = file.can_frames(group).unwrap();
            if frames.is_empty() {
                continue;
            }
            checked += 1;
            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(
                    frame.bus_channel, want,
                    "{path} group '{}' frame {i}: bus channel",
                    group.acquisition_name
                );
            }
        }
    }

    assert!(
        checked > 0,
        "no group named after its bus was found in the corpus"
    );
}

/// Every frame's payload is exactly as long as the record says it is, and no
/// longer than CAN allows.
#[test]
fn payloads_are_trimmed_to_the_logged_length() {
    let paths = bus_log_paths();
    if skip_if_empty(&paths) {
        return;
    }

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in file.can_frame_groups() {
            let lengths = group
                .find_channel("CAN_DataFrame.DataLength")
                .map(|ch| file.signal(ch).unwrap().values().unwrap().to_f64())
                .unwrap_or_default();
            let frames = file.can_frames(group).unwrap();

            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(
                    frame.data.len() as f64,
                    lengths[i],
                    "{path} group {} frame {i}: payload length",
                    group.index
                );
                assert!(
                    frame.data.len() <= 64,
                    "{path} group {} frame {i}: {} payload bytes exceeds CAN FD",
                    group.index,
                    frame.data.len()
                );
            }
        }
    }
}

#[test]
fn frames_are_in_logging_order() {
    let paths = bus_log_paths();
    if skip_if_empty(&paths) {
        return;
    }

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in file.can_frame_groups() {
            let mut previous = f64::NEG_INFINITY;
            for (i, frame) in file.can_frames(group).unwrap().iter().enumerate() {
                assert!(
                    frame.timestamp >= previous,
                    "{path} group {} frame {i}: timestamp {} precedes {previous}",
                    group.index,
                    frame.timestamp
                );
                previous = frame.timestamp;
            }
        }
    }
}

/// An OBD2 request-response log can hold no identifier but the broadcast
/// request and the first ECU's reply, both of them standard 11-bit frames.
#[test]
fn obd2_log_holds_the_two_obd2_identifiers() {
    let path = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4";
    if !Path::new(path).exists() {
        eprintln!("SKIP: {path} is absent");
        return;
    }

    let file = Mf4File::open(path).unwrap();
    let mut ids = BTreeSet::new();
    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            ids.insert(frame.id);
            assert_eq!(frame.extended, Some(false), "OBD2 frames are 11-bit");
            assert!(
                frame.id <= 0x7FF,
                "identifier {:#X} exceeds 11 bits",
                frame.id
            );
        }
    }

    assert_eq!(
        ids,
        BTreeSet::from([0x7DF, 0x7E8]),
        "expected the OBD2 request and response identifiers"
    );
}

/// A J1939 log holds nothing but extended frames, and their identifiers have to
/// decompose into plausible parameter group numbers.
#[test]
fn j1939_log_holds_extended_identifiers_with_real_pgns() {
    let path = "test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4";
    if !Path::new(path).exists() {
        eprintln!("SKIP: {path} is absent");
        return;
    }

    let file = Mf4File::open(path).unwrap();
    let mut pgns = BTreeSet::new();
    let mut frames_seen = 0usize;

    for group in file.can_frame_groups() {
        for frame in file.can_frames(group).unwrap().iter() {
            frames_seen += 1;
            assert_eq!(frame.extended, Some(true), "J1939 frames are 29-bit");
            assert!(
                frame.id <= 0x1FFF_FFFF,
                "identifier {:#X} exceeds 29 bits",
                frame.id
            );
            // Priority occupies the top three bits of a 29-bit J1939
            // identifier, so nothing above bit 28 may be set — which the
            // assertion above covers — and the PGN is the middle 18.
            assert!(frame.id >> 26 <= 7, "priority field out of range");
            pgns.insert((frame.id >> 8) & 0x3FFFF);
        }
    }

    assert_eq!(frames_seen, 145_534, "frame count");
    // EEC1 (0xF004) carries engine speed and is on every J1939 engine bus;
    // 0xFEE5 is total engine hours. Both are standard SAE parameter groups.
    for pgn in [0xF004, 0xFEE5] {
        assert!(
            pgns.contains(&pgn),
            "expected PGN {pgn:#X} among {} parameter groups",
            pgns.len()
        );
    }
}

/// A group that is not a CAN frame group is refused rather than half-read. The
/// LIN group in the truck logs is the case that matters: it sets the bus-event
/// flag and composes its fields under `LIN_Frame`, so only the channel names
/// distinguish it.
#[test]
fn a_non_can_group_is_refused() {
    let path = "test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4";
    if !Path::new(path).exists() {
        eprintln!("SKIP: {path} is absent");
        return;
    }

    let file = Mf4File::open(path).unwrap();
    let lin = file
        .data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .find(|cg| cg.acquisition_name == "LIN_Frame")
        .expect("the truck logs carry an empty LIN group");

    assert!(lin.is_bus_event(), "the LIN group is a bus-event group");
    assert!(
        !file
            .can_frame_groups()
            .iter()
            .any(|cg| cg.acquisition_name == "LIN_Frame"),
        "a LIN group must not be offered as a CAN frame group"
    );
    let err = file.can_frames(lin).expect_err("LIN is not CAN");
    assert!(
        matches!(err, falcon_mdf::Mf4Error::ChannelNotFound { .. }),
        "expected ChannelNotFound, got {err}"
    );
}

/// Reading frames must not depend on `SignalValues` being a particular variant:
/// the payload channel is variable-length in these logs and fixed-length in
/// others, and both must trim to the same frames.
#[test]
fn payload_channels_of_both_storage_kinds_are_handled() {
    let paths = bus_log_paths();
    if skip_if_empty(&paths) {
        return;
    }
    let mut kinds = BTreeSet::new();

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in file.can_frame_groups() {
            let Some(channel) = group.find_channel("CAN_DataFrame.DataBytes") else {
                continue;
            };
            if group.sample_count == 0 {
                continue;
            }
            match file.signal(channel).unwrap().values().unwrap() {
                SignalValues::Bytes { .. } => kinds.insert("fixed"),
                SignalValues::VarBytes { .. } => kinds.insert("variable"),
                other => panic!("{path}: payload came back as {}", other.kind().name()),
            };
            // Whatever the storage, the frames read out of it are usable.
            assert!(file.can_frames(group).unwrap().get(0).is_some());
        }
    }

    assert!(!kinds.is_empty(), "no payload channel was read");
    eprintln!("payload storage kinds exercised: {kinds:?}");
}

#[test]
fn dlc_fallback_trims_classic_and_can_fd_payloads_and_exposes_flags() {
    use falcon_mdf::bus::CanFrameFlags;
    use falcon_mdf::Mf4Writer;

    // 4 frames:
    // 0: Classic CAN (EDL=0), DLC=5 -> 5 bytes
    // 1: Classic CAN (EDL=0), DLC=12 -> clamped to 8 bytes
    // 2: CAN FD (EDL=1, BRS=1, ESI=1, Dir=1), DLC=9 -> 12 bytes
    // 3: CAN FD (EDL=1, BRS=0, RTR=1, Dir=0), DLC=15 -> 64 bytes
    let mut payload_bytes = Vec::with_capacity(4 * 64);
    for frame_idx in 0..4u8 {
        for b in 0..64u8 {
            payload_bytes.push(frame_idx.wrapping_mul(16).wrapping_add(b));
        }
    }

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1, 0.2, 0.3, 0.4]).unwrap();
    group
        .add_channel_typed(
            "CAN_DataFrame.ID",
            "",
            SignalValues::U32(vec![0x100, 0x101, 0x200, 0x201]),
        )
        .unwrap();
    // Deliberately omit DataLength and supply DLC + CAN FD flags.
    group
        .add_channel_typed(
            "CAN_DataFrame.DLC",
            "",
            SignalValues::U8(vec![5, 12, 9, 15]),
        )
        .unwrap();
    group
        .add_channel_typed("CAN_DataFrame.EDL", "", SignalValues::U8(vec![0, 0, 1, 1]))
        .unwrap();
    group
        .add_channel_typed("CAN_DataFrame.BRS", "", SignalValues::U8(vec![0, 0, 1, 0]))
        .unwrap();
    group
        .add_channel_typed("CAN_DataFrame.ESI", "", SignalValues::U8(vec![0, 0, 1, 0]))
        .unwrap();
    group
        .add_channel_typed("CAN_DataFrame.RTR", "", SignalValues::U8(vec![0, 1, 0, 1]))
        .unwrap();
    group
        .add_channel_typed("CAN_DataFrame.Dir", "", SignalValues::U8(vec![0, 0, 1, 0]))
        .unwrap();
    group
        .add_channel_typed(
            "CAN_DataFrame.DataBytes",
            "",
            SignalValues::Bytes {
                data: payload_bytes,
                width: 64,
            },
        )
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let groups = file.can_frame_groups();
    assert_eq!(groups.len(), 1);
    let frames = file.can_frames(groups[0]).unwrap();
    assert_eq!(frames.len(), 4);

    assert_eq!(frames.get(0).unwrap().data.len(), 5);
    assert_eq!(frames.get(1).unwrap().data.len(), 8);
    assert_eq!(frames.get(2).unwrap().data.len(), 12);
    assert_eq!(frames.get(3).unwrap().data.len(), 64);

    assert_eq!(frames.dlc(2), Some(9));
    assert_eq!(frames.edl(2), Some(true));
    assert_eq!(frames.brs(2), Some(true));
    assert_eq!(frames.esi(2), Some(true));
    assert_eq!(frames.rtr(2), Some(false));
    assert_eq!(frames.dir(2), Some(true));
    assert_eq!(
        frames.flags(2),
        CanFrameFlags {
            edl: Some(true),
            brs: Some(true),
            esi: Some(true),
            rtr: Some(false),
            dir: Some(true),
        }
    );
    assert_eq!(frames.rtr(1), Some(true));
    assert_eq!(frames.edl(0), Some(false));
}

#[test]
fn can_nan_frames_skipped_and_non_bytes_payload_rejected() {
    use falcon_mdf::Mf4Writer;

    // NaN in CAN_DataFrame.ID must skip that frame instead of producing id == 0.
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1, 0.2, 0.3]).unwrap();
    group
        .add_channel(
            "CAN_DataFrame.ID",
            "",
            &[0x100 as f64, f64::NAN, 0x200 as f64],
        )
        .unwrap();
    group
        .add_channel("CAN_DataFrame.DataLength", "", &[2.0, 2.0, 2.0])
        .unwrap();
    group
        .add_channel_typed(
            "CAN_DataFrame.DataBytes",
            "",
            SignalValues::Bytes {
                data: vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
                width: 2,
            },
        )
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let frames = file.can_frames(file.can_frame_groups()[0]).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames.get(0).unwrap().id, 0x100);
    assert_eq!(frames.get(0).unwrap().data, &[0xAA, 0xBB]);
    assert_eq!(frames.get(1).unwrap().id, 0x200);
    assert_eq!(frames.get(1).unwrap().data, &[0xEE, 0xFF]);

    // Non-bytes DataBytes channel must return Err rather than yielding 0 frames.
    let mut bad = Mf4Writer::with_start_time_ns(0);
    let g = bad.add_group(&[0.1]).unwrap();
    g.add_channel("CAN_DataFrame.ID", "", &[0x100 as f64])
        .unwrap();
    g.add_channel("CAN_DataFrame.DataBytes", "", &[42.0])
        .unwrap();
    let temp_bad = tempfile::NamedTempFile::new().unwrap();
    bad.write_to_file(temp_bad.path()).unwrap();
    let file_bad = Mf4File::open(temp_bad.path()).unwrap();
    let cg = &file_bad.data_groups()[0].channel_groups[0];
    assert!(
        file_bad.can_frames(cg).is_err(),
        "non-bytes DataBytes channel must return Err"
    );
}

#[test]
fn can_fd_dataframe_prefix_is_detected_and_read() {
    use falcon_mdf::Mf4Writer;

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.5]).unwrap();
    group
        .add_channel_typed("CAN_FD_DataFrame.ID", "", SignalValues::U32(vec![0x321]))
        .unwrap();
    group
        .add_channel_typed("CAN_FD_DataFrame.DataLength", "", SignalValues::U8(vec![3]))
        .unwrap();
    group
        .add_channel_typed(
            "CAN_FD_DataFrame.DataBytes",
            "",
            SignalValues::Bytes {
                data: vec![1, 2, 3, 0, 0, 0, 0, 0],
                width: 8,
            },
        )
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let groups = file.can_frame_groups();
    assert_eq!(groups.len(), 1);
    let frames = file.can_frames(groups[0]).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames.get(0).unwrap().id, 0x321);
    assert_eq!(frames.get(0).unwrap().data, &[1, 2, 3]);
}

#[test]
fn can_invalid_samples_via_invalidation_bits_are_filtered_and_optional_mismatch_rejected() {
    use falcon_mdf::Mf4Writer;

    // Frame 1 has its invalidation bit set on CAN_DataFrame.ID (validity = false);
    // it must be filtered out while frames 0 and 2 remain intact with their IDE/EDL flags.
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1, 0.2, 0.3]).unwrap();
    group
        .add_channel_typed_with(
            "CAN_DataFrame.ID",
            "",
            SignalValues::U32(vec![0x111, 0x999, 0x222]),
            Some(&[true, false, true]),
            None,
        )
        .unwrap();
    group
        .add_channel_typed_with(
            "CAN_DataFrame.IDE",
            "",
            SignalValues::U8(vec![0, 1, 1]),
            Some(&[true, true, false]),
            None,
        )
        .unwrap();
    group
        .add_channel_typed(
            "CAN_DataFrame.DataLength",
            "",
            SignalValues::U8(vec![2, 2, 2]),
        )
        .unwrap();
    group
        .add_channel_typed(
            "CAN_DataFrame.DataBytes",
            "",
            SignalValues::Bytes {
                data: vec![1, 2, 3, 4, 5, 6],
                width: 2,
            },
        )
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    let frames = file.can_frames(file.can_frame_groups()[0]).unwrap();
    assert_eq!(frames.len(), 2, "invalidated sample 1 must be skipped");
    assert_eq!(frames.get(0).unwrap().id, 0x111);
    assert_eq!(frames.get(0).unwrap().extended, Some(false));
    assert_eq!(frames.get(0).unwrap().data, &[1, 2]);
    assert_eq!(frames.get(1).unwrap().id, 0x222);
    assert_eq!(
        frames.get(1).unwrap().extended,
        None,
        "invalidated optional IDE sample becomes None"
    );
    assert_eq!(frames.get(1).unwrap().data, &[5, 6]);

    // Optional channel length mismatch (IDE has 1 sample vs 2 IDs) must return Err.
    let mut mismatch_writer = Mf4Writer::with_start_time_ns(0);
    let g0 = mismatch_writer.add_group(&[0.1, 0.2]).unwrap();
    g0.add_channel_typed("CAN_DataFrame.ID", "", SignalValues::U32(vec![1, 2]))
        .unwrap();
    g0.add_channel_typed(
        "CAN_DataFrame.DataBytes",
        "",
        SignalValues::Bytes {
            data: vec![0xAA, 0xBB],
            width: 1,
        },
    )
    .unwrap();
    let g1 = mismatch_writer.add_group(&[0.1]).unwrap();
    g1.add_channel_typed("CAN_DataFrame.IDE", "", SignalValues::U8(vec![0]))
        .unwrap();

    let temp2 = tempfile::NamedTempFile::new().unwrap();
    mismatch_writer.write_to_file(temp2.path()).unwrap();
    let file2 = Mf4File::open(temp2.path()).unwrap();
    let mut cg = file2.data_groups()[0].channel_groups[0].clone();
    let ide = file2.data_groups()[1].channel_groups[0]
        .find_channel("CAN_DataFrame.IDE")
        .unwrap()
        .clone();
    cg.channels.push(ide);
    assert!(
        file2.can_frames(&cg).is_err(),
        "mismatched optional channel length must return Err"
    );
}
