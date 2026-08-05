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
