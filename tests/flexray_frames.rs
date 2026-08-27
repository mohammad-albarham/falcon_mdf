//! FlexRay frame extraction tested against synthetic fixtures.
//!
//! There are no real FlexRay logger files in the corpus, so these fixtures are
//! built with `Mf4Writer` and read back through `Mf4File::flexray_frame_groups`
//! and `Mf4File::flexray_frames` — a round trip through two paths in the same
//! crate, weaker evidence than `tests/bus_frames.rs` gets from real logger
//! files, and a reader should know that.

use falcon_mdf::write::Mf4Writer;
use falcon_mdf::{Mf4File, SignalValues};

fn open_written(writer: &Mf4Writer) -> (tempfile::NamedTempFile, Mf4File) {
    let temp = tempfile::NamedTempFile::new().expect("create tempfile");
    writer
        .write_to_file(temp.path())
        .expect("write to tempfile");
    let file = Mf4File::open(temp.path()).expect("open tempfile");
    (temp, file)
}

/// 1. Varying payload lengths.
///
/// Three frames of 8, 32 and 254 bytes in a 254-byte-wide `DataBytes` channel.
/// Asserts the returned `data` is exactly the bytes written, byte for byte,
/// for every frame. A reader that returned the padding must fail this.
#[test]
fn varying_payload_lengths_are_trimmed_to_logged_length() {
    let payload0: Vec<u8> = (0..8).map(|x| (x * 17) as u8).collect();
    let payload1: Vec<u8> = (100..132).map(|x| x as u8).collect();
    let payload2: Vec<u8> = (0..254).map(|x| (x ^ 0x5A) as u8).collect();

    let width = 254;
    let mut row0 = payload0.clone();
    row0.resize(width, 0xEE);
    let mut row1 = payload1.clone();
    row1.resize(width, 0xEE);
    let row2 = payload2.clone();
    let data = [row0, row1, row2].concat();

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1, 0.2, 0.3]).unwrap();
    group
        .add_channel("FLX_Frame.FrameID", "", &[10.0, 20.0, 30.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.DataLength", "", &[8.0, 32.0, 254.0])
        .unwrap();
    group
        .add_channel_typed(
            "FLX_Frame.DataBytes",
            "",
            SignalValues::Bytes { data, width },
        )
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let cg = &file.data_groups()[0].channel_groups[0];
    let frames = file.flexray_frames(cg).unwrap();

    assert_eq!(frames.len(), 3);
    assert!(!frames.is_empty());

    let f0 = frames.get(0).expect("frame 0");
    assert_eq!(f0.data.len(), 8);
    assert_eq!(f0.data, payload0.as_slice());

    let f1 = frames.get(1).expect("frame 1");
    assert_eq!(f1.data.len(), 32);
    assert_eq!(f1.data, payload1.as_slice());

    let f2 = frames.get(2).expect("frame 2");
    assert_eq!(f2.data.len(), 254);
    assert_eq!(f2.data, payload2.as_slice());

    assert!(frames.get(3).is_none());

    let collected: Vec<&[u8]> = frames.iter().map(|f| f.data).collect();
    assert_eq!(
        collected,
        vec![
            payload0.as_slice(),
            payload1.as_slice(),
            payload2.as_slice()
        ]
    );
}

/// 2. All fields.
///
/// Asserts timestamp, frame ID, cycle, bus channel and all three flags for every frame.
#[test]
fn all_fields_read_back_correctly() {
    let payload0 = vec![0x11, 0x22];
    let payload1 = vec![0x33, 0x44, 0x55];

    let width = 4;
    let mut row0 = payload0.clone();
    row0.resize(width, 0);
    let mut row1 = payload1.clone();
    row1.resize(width, 0);
    let data = [row0, row1].concat();

    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[1.25, 2.75]).unwrap();
    group
        .add_channel("FLX_Frame.FrameID", "", &[100.0, 200.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.DataLength", "", &[2.0, 3.0])
        .unwrap();
    group
        .add_channel_typed(
            "FLX_Frame.DataBytes",
            "",
            SignalValues::Bytes { data, width },
        )
        .unwrap();
    group
        .add_channel("FLX_Frame.Cycle", "", &[5.0, 42.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.BusChannel", "", &[1.0, 2.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.NullFrameFlag", "", &[0.0, 1.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.SyncFrameFlag", "", &[1.0, 0.0])
        .unwrap();
    group
        .add_channel("FLX_Frame.StartupFlag", "", &[0.0, 1.0])
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let cg = &file.data_groups()[0].channel_groups[0];
    let frames = file.flexray_frames(cg).unwrap();

    assert_eq!(frames.len(), 2);

    let f0 = frames.get(0).expect("frame 0");
    assert_eq!(f0.timestamp, 1.25);
    assert_eq!(f0.frame_id, 100);
    assert_eq!(f0.cycle, 5);
    assert_eq!(f0.bus_channel, 1);
    assert!(!f0.null_frame);
    assert!(f0.sync_frame);
    assert!(!f0.startup);
    assert_eq!(f0.data, payload0.as_slice());

    let f1 = frames.get(1).expect("frame 1");
    assert_eq!(f1.timestamp, 2.75);
    assert_eq!(f1.frame_id, 200);
    assert_eq!(f1.cycle, 42);
    assert_eq!(f1.bus_channel, 2);
    assert!(f1.null_frame);
    assert!(!f1.sync_frame);
    assert!(f1.startup);
    assert_eq!(f1.data, payload1.as_slice());
}

/// 3. Masking.
///
/// Write a `FrameID` with bits above bit 10 set and a `Cycle` with bits above
/// bit 5 set; assert both come back masked. A reader that skipped the mask
/// must fail this.
#[test]
fn frame_id_and_cycle_are_masked() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1, 0.2]).unwrap();
    // 0xF7FF has upper bits set; masked to 11 bits (0x7FF) -> 2047
    // 0x1005 has upper bits set; masked to 11 bits (0x7FF) -> 5
    group
        .add_channel("FLX_Frame.FrameID", "", &[0xF7FF as f64, 0x1005 as f64])
        .unwrap();
    group
        .add_channel("FLX_Frame.DataLength", "", &[1.0, 1.0])
        .unwrap();
    group
        .add_channel_typed(
            "FLX_Frame.DataBytes",
            "",
            SignalValues::Bytes {
                data: vec![0xAA, 0xBB],
                width: 1,
            },
        )
        .unwrap();
    // 0xFF has bits above bit 5 set; masked to 6 bits (0x3F) -> 63
    // 0x91 (0x80 | 17) has bit 7 set; masked to 6 bits (0x3F) -> 17
    group
        .add_channel("FLX_Frame.Cycle", "", &[0xFF as f64, 0x91 as f64])
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let cg = &file.data_groups()[0].channel_groups[0];
    let frames = file.flexray_frames(cg).unwrap();

    assert_eq!(frames.len(), 2);
    let f0 = frames.get(0).expect("frame 0");
    assert_eq!(f0.frame_id, 2047);
    assert_eq!(f0.cycle, 63);

    let f1 = frames.get(1).expect("frame 1");
    assert_eq!(f1.frame_id, 5);
    assert_eq!(f1.cycle, 17);
}

/// 4. Optional channels absent.
///
/// A fixture with only the three required channels: assert cycle and bus
/// channel are `0` and the three flags `false` — not an error.
#[test]
fn optional_channels_absent_default_to_zero_and_false() {
    let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.5]).unwrap();
    group.add_channel("FLX_Frame.FrameID", "", &[42.0]).unwrap();
    group
        .add_channel("FLX_Frame.DataLength", "", &[4.0])
        .unwrap();
    group
        .add_channel_typed(
            "FLX_Frame.DataBytes",
            "",
            SignalValues::Bytes {
                data: payload.clone(),
                width: 4,
            },
        )
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let cg = &file.data_groups()[0].channel_groups[0];
    let frames = file.flexray_frames(cg).unwrap();

    assert_eq!(frames.len(), 1);
    let f0 = frames.get(0).expect("frame 0");
    assert_eq!(f0.timestamp, 0.5);
    assert_eq!(f0.frame_id, 42);
    assert_eq!(f0.cycle, 0);
    assert_eq!(f0.bus_channel, 0);
    assert!(!f0.null_frame);
    assert!(!f0.sync_frame);
    assert!(!f0.startup);
    assert_eq!(f0.data, payload.as_slice());
}

/// 5. Missing required channel.
///
/// A group with no `FLX_Frame.DataBytes`: assert `flexray_frames` returns an
/// `Err` whose message names the missing channel.
#[test]
fn missing_required_channel_returns_error_naming_channel() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&[0.1]).unwrap();
    group.add_channel("FLX_Frame.FrameID", "", &[10.0]).unwrap();
    group
        .add_channel("FLX_Frame.DataLength", "", &[1.0])
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let cg = &file.data_groups()[0].channel_groups[0];
    let err = match file.flexray_frames(cg) {
        Err(e) => e,
        Ok(_) => panic!("expected flexray_frames to fail when DataBytes is missing"),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("FLX_Frame.DataBytes"),
        "error message should name the missing channel 'FLX_Frame.DataBytes', got: {msg}"
    );
}

/// 6. Sample-for-sample refusal.
///
/// A group whose `DataLength` channel has fewer samples than its `FrameID`
/// channel: assert an `Err`, not a truncated frame list.
#[test]
fn sample_for_sample_refusal_when_channel_lengths_mismatch() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let g0 = writer.add_group(&[0.1, 0.2, 0.3]).unwrap();
    g0.add_channel("FLX_Frame.FrameID", "", &[1.0, 2.0, 3.0])
        .unwrap();
    g0.add_channel_typed(
        "FLX_Frame.DataBytes",
        "",
        SignalValues::Bytes {
            data: vec![0x11, 0x22, 0x33],
            width: 1,
        },
    )
    .unwrap();

    let g1 = writer.add_group(&[0.1, 0.2]).unwrap();
    g1.add_channel("FLX_Frame.DataLength", "", &[1.0, 1.0])
        .unwrap();

    let (_temp, file) = open_written(&writer);
    let mut group = file.data_groups()[0].channel_groups[0].clone();
    let length_channel = file.data_groups()[1].channel_groups[0]
        .find_channel("FLX_Frame.DataLength")
        .expect("DataLength channel in group 1");
    group.channels.push(length_channel.clone());

    let err = match file.flexray_frames(&group) {
        Err(e) => e,
        Ok(_) => panic!("expected error when DataLength has fewer samples than FrameID"),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("agree sample for sample"),
        "error message should explain sample agreement, got: {msg}"
    );
}

/// 7. `flexray_frame_groups` finds it, and does not claim a CAN or LIN group.
#[test]
fn flexray_frame_groups_finds_flexray_and_ignores_can_and_lin() {
    let mut writer = Mf4Writer::with_start_time_ns(0);

    // Group 0: FlexRay frame group
    let flx = writer.add_group(&[0.1]).unwrap();
    flx.add_channel("FLX_Frame.FrameID", "", &[10.0]).unwrap();
    flx.add_channel("FLX_Frame.DataLength", "", &[2.0]).unwrap();
    flx.add_channel_typed(
        "FLX_Frame.DataBytes",
        "",
        SignalValues::Bytes {
            data: vec![0xAA, 0xBB],
            width: 2,
        },
    )
    .unwrap();

    // Group 1: CAN frame group
    let can = writer.add_group(&[0.1]).unwrap();
    can.add_channel("CAN_DataFrame.ID", "", &[0x123 as f64])
        .unwrap();
    can.add_channel_typed(
        "CAN_DataFrame.DataBytes",
        "",
        SignalValues::Bytes {
            data: vec![0x11, 0x22],
            width: 2,
        },
    )
    .unwrap();

    // Group 2: LIN frame group
    let lin = writer.add_group(&[0.1]).unwrap();
    lin.add_channel("LIN_Frame.ID", "", &[0x12 as f64]).unwrap();
    lin.add_channel("LIN_Frame.DataLength", "", &[1.0]).unwrap();
    lin.add_channel_typed(
        "LIN_Frame.DataBytes",
        "",
        SignalValues::Bytes {
            data: vec![0x55],
            width: 1,
        },
    )
    .unwrap();

    // Group 3: Plain non-bus signals
    let plain = writer.add_group(&[0.1]).unwrap();
    plain.add_channel("Speed", "km/h", &[50.0]).unwrap();
    plain.add_channel("RPM", "rpm", &[2000.0]).unwrap();

    let (_temp, file) = open_written(&writer);

    let flx_groups = file.flexray_frame_groups();
    assert_eq!(
        flx_groups.len(),
        1,
        "flexray_frame_groups should find exactly 1 FlexRay group"
    );
    assert!(flx_groups[0].find_channel("FLX_Frame.FrameID").is_some());
    assert!(flx_groups[0].find_channel("FLX_Frame.DataBytes").is_some());

    let can_groups = file.can_frame_groups();
    assert_eq!(
        can_groups.len(),
        1,
        "can_frame_groups should find exactly 1 CAN group"
    );
    assert!(can_groups[0].find_channel("CAN_DataFrame.ID").is_some());

    let lin_groups = file.lin_frame_groups();
    assert_eq!(
        lin_groups.len(),
        1,
        "lin_frame_groups should find exactly 1 LIN group"
    );
    assert!(lin_groups[0].find_channel("LIN_Frame.ID").is_some());
}
