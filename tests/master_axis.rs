use falcon_mdf::blocks::common::BlockHeader;
use falcon_mdf::{Mf4File, Mf4Writer};

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

fn rejects(bytes: Vec<u8>, reason: &str) {
    let file = Mf4File::from_bytes(bytes).unwrap();
    let channel = file.find_channel("Speed").unwrap();
    assert!(file.signal(channel).unwrap().values().is_ok(), "raw tables remain readable");
    let error = file.time_series(channel).expect_err("unusable master");
    assert!(error.to_string().contains(reason), "{error}");
    assert!(file.filter(&["Speed".into()]).is_err());
    let mut chunks = file.signals_chunks(&[channel], 2).unwrap();
    let first = chunks.next().unwrap().unwrap();
    assert!(chunks.signals_to_series(&first, 0).is_err());
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
fn duplicate_timestamps_and_non_time_masters_preserve_raw_coordinates() {
    let mut writer = Mf4Writer::new();
    writer.add_group(&[0.0, 1.0, 1.0]).unwrap().add_channel("Speed", "", &[1.0, 2.0, 3.0]).unwrap();
    let mut bytes = Vec::new(); writer.write(&mut bytes).unwrap();
    let file = Mf4File::from_bytes(bytes.clone()).unwrap();
    let at = file.master_channel(0, 0).unwrap().block_offset() as usize;
    let data = at + BlockHeader::parse(&bytes[at..], at as u64).unwrap().data_offset();
    bytes[data + 1] = 2; // angle synchronization
    let file = Mf4File::from_bytes(bytes).unwrap();
    let series = file.time_series(file.find_channel("Speed").unwrap()).unwrap();
    assert_eq!(&**series.timestamps, &[0.0, 1.0, 1.0]);
    assert_eq!(series.values.to_f64(), vec![1.0, 2.0, 3.0]);
}
