use falcon_mdf::blocks::common::BlockHeader;
use falcon_mdf::{Mf4File, Mf4Writer, SignalValues};
use falcon_mdf_gui::model::ChannelLoc;
use falcon_mdf_gui::signal_loader::{decode_channel, SignalLoadResult};

fn fixture() -> Vec<u8> {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    writer
        .add_group(&[0.0, 1.0, 2.0])
        .unwrap()
        .add_channel("Speed", "km/h", &[10.0, 20.0, 30.0])
        .unwrap();
    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    bytes
}

fn decode(bytes: Vec<u8>) -> SignalLoadResult {
    let file = Mf4File::from_bytes(bytes).unwrap();
    let channel_index = file.data_groups()[0].channel_groups[0]
        .channels
        .iter()
        .position(|ch| ch.name != "Time")
        .unwrap();
    decode_channel(
        &file,
        ChannelLoc {
            data_group_index: 0,
            channel_group_index: 0,
            channel_index,
        },
    )
}

fn rejects(bytes: Vec<u8>, reason: &str) {
    match decode(bytes) {
        SignalLoadResult::Err { message } => assert!(message.contains(reason), "{message}"),
        SignalLoadResult::Ok(_) => panic!("expected {reason}"),
    }
}

#[test]
fn invalid_master_is_reported_instead_of_using_its_garbage_timestamps() {
    let mut bytes = fixture();
    let file = Mf4File::from_bytes(bytes.clone()).unwrap();
    let at = file.master_channel(0, 0).unwrap().block_offset() as usize;
    let data = at
        + BlockHeader::parse(&bytes[at..], at as u64)
            .unwrap()
            .data_offset();
    bytes[data + 12..data + 16].copy_from_slice(&1u32.to_le_bytes());
    rejects(bytes, "invalid timestamp");
}

#[test]
fn unusable_time_axes_do_not_reach_binary_search_or_plotting() {
    for bad_time in [f64::NAN, f64::INFINITY, -1.0] {
        let mut bytes = fixture();
        let file = Mf4File::from_bytes(bytes.clone()).unwrap();
        let dg = &file.data_groups()[0];
        let at = dg.data_block_offset() as usize;
        assert_eq!(&bytes[at..at + 4], b"##DT");
        let data = at
            + BlockHeader::parse(&bytes[at..], at as u64)
                .unwrap()
                .data_offset();
        let stride = dg.channel_groups[0].payload_size();
        let master_offset = file.master_channel(0, 0).unwrap().byte_offset as usize;
        let second_time = data + stride + master_offset;
        bytes[second_time..second_time + 8].copy_from_slice(&bad_time.to_le_bytes());
        rejects(bytes, "timestamp");
    }
}

#[test]
fn a_scalar_channel_still_decodes_with_its_master() {
    let SignalLoadResult::Ok(series) = decode(fixture()) else {
        panic!("valid signal")
    };
    assert_eq!(series.times, [0.0, 1.0, 2.0]);
    assert_eq!(series.values, [10.0, 20.0, 30.0]);
}

#[test]
fn arrays_are_not_flattened_into_scalar_time_pairs() {
    let mut writer = Mf4Writer::with_start_time_ns(0);
    writer
        .add_group(&[0.0, 1.0])
        .unwrap()
        .add_channel_array(
            "Position",
            "m",
            &[2],
            SignalValues::Array {
                values: vec![1.0, 2.0, 3.0, 4.0],
                elements_per_sample: 2,
            },
        )
        .unwrap();
    let mut bytes = Vec::new();
    writer.write(&mut bytes).unwrap();
    rejects(bytes, "array");
}
