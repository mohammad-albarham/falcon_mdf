//! Ethernet frame extraction round-tripped through `Mf4Writer`.
//!
//! There is no Ethernet reference file in the corpus, so these tests build
//! synthetic bus-logged groups with `Mf4Writer` and read them back. This is a
//! round-trip through two paths in the same crate, which is weaker evidence than
//! `tests/bus_frames.rs` and `tests/lin_frames.rs` get from real logger files,
//! and a reader should know that.

use falcon_mdf::{Mf4Error, Mf4File, Mf4Writer, SignalValues};

fn open_written(writer: &Mf4Writer) -> (tempfile::NamedTempFile, Mf4File) {
    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();
    (temp, file)
}

#[test]
fn roundtrip_ethernet_frames_with_varying_payload_lengths() {
    let timestamps = [0.0, 0.05, 0.12];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&timestamps).unwrap();

    // Source MAC addresses (6 bytes each)
    let src_macs: [[u8; 6]; 3] = [
        [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
        [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
    ];
    let src_data: Vec<u8> = src_macs.iter().flat_map(|m| m.iter().copied()).collect();
    group
        .add_channel_typed(
            "ETH_Frame.Source",
            "",
            SignalValues::Bytes {
                data: src_data,
                width: 6,
            },
        )
        .unwrap();

    // Destination MAC addresses (6 bytes each)
    let dst_macs: [[u8; 6]; 3] = [
        [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        [0x00, 0x50, 0x56, 0xC0, 0x00, 0x01],
    ];
    let dst_data: Vec<u8> = dst_macs.iter().flat_map(|m| m.iter().copied()).collect();
    group
        .add_channel_typed(
            "ETH_Frame.Destination",
            "",
            SignalValues::Bytes {
                data: dst_data,
                width: 6,
            },
        )
        .unwrap();

    // EtherType (unsigned 16-bit)
    let ether_types: [u16; 3] = [0x0800, 0x0806, 0x86DD];
    group
        .add_channel_typed(
            "ETH_Frame.EtherType",
            "",
            SignalValues::U16(ether_types.to_vec()),
        )
        .unwrap();

    // DataLength: differing payload lengths: 46, 12, 60
    let lengths: [u32; 3] = [46, 12, 60];
    group
        .add_channel_typed(
            "ETH_Frame.DataLength",
            "",
            SignalValues::U32(lengths.to_vec()),
        )
        .unwrap();

    // DataBytes: 64-byte-wide fixed channel with padding
    let p0: Vec<u8> = (0..46u8).collect();
    let p1: Vec<u8> = (100..112u8).collect();
    let p2: Vec<u8> = (150..210u8).collect();
    let payloads = [&p0, &p1, &p2];

    let mut padded_payload_data = Vec::with_capacity(3 * 64);
    for p in &payloads {
        padded_payload_data.extend_from_slice(p);
        padded_payload_data.resize(padded_payload_data.len() + (64 - p.len()), 0xEE);
    }
    assert_eq!(padded_payload_data.len(), 3 * 64);

    group
        .add_channel_typed(
            "ETH_Frame.DataBytes",
            "",
            SignalValues::Bytes {
                data: padded_payload_data,
                width: 64,
            },
        )
        .unwrap();

    // BusChannel: unsigned 8-bit
    let bus_channels: [u8; 3] = [1, 2, 1];
    group
        .add_channel_typed(
            "ETH_Frame.BusChannel",
            "",
            SignalValues::U8(bus_channels.to_vec()),
        )
        .unwrap();

    let (_temp, file) = open_written(&writer);

    let eth_groups = file.eth_frame_groups();
    assert_eq!(eth_groups.len(), 1, "found 1 Ethernet frame group");

    let frames = file.eth_frames(eth_groups[0]).expect("decode eth frames");
    assert_eq!(frames.len(), 3);
    assert!(!frames.is_empty());

    for i in 0..3 {
        let frame = frames.get(i).unwrap_or_else(|| panic!("frame {i}"));
        assert_eq!(frame.timestamp, timestamps[i], "frame {i} timestamp");
        assert_eq!(frame.source, Some(src_macs[i]), "frame {i} source MAC");
        assert_eq!(
            frame.destination,
            Some(dst_macs[i]),
            "frame {i} destination MAC"
        );
        assert_eq!(frame.ether_type, ether_types[i], "frame {i} EtherType");
        assert_eq!(frame.bus_channel, bus_channels[i], "frame {i} bus channel");
        assert_eq!(
            frame.data,
            payloads[i].as_slice(),
            "frame {i} data trimmed to logged length"
        );
    }

    assert!(frames.get(3).is_none(), "frame 3 should be past the end");

    let collected: Vec<_> = frames.iter().collect();
    assert_eq!(collected.len(), 3);
    for (i, frame) in collected.iter().enumerate() {
        assert_eq!(frame.timestamp, timestamps[i]);
        assert_eq!(frame.source, Some(src_macs[i]));
        assert_eq!(frame.destination, Some(dst_macs[i]));
        assert_eq!(frame.ether_type, ether_types[i]);
        assert_eq!(frame.bus_channel, bus_channels[i]);
        assert_eq!(frame.data, payloads[i].as_slice());
    }
}

#[test]
fn ethernet_frames_without_optional_channels() {
    let timestamps = [1.0, 2.5];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&timestamps).unwrap();

    // No Source, no Destination, no BusChannel
    let ether_types: [u16; 2] = [0x0800, 0x88F7];
    group
        .add_channel_typed(
            "ETH_Frame.EtherType",
            "",
            SignalValues::U16(ether_types.to_vec()),
        )
        .unwrap();

    let lengths: [u32; 2] = [20, 30];
    group
        .add_channel_typed(
            "ETH_Frame.DataLength",
            "",
            SignalValues::U32(lengths.to_vec()),
        )
        .unwrap();

    let p0 = vec![0x11u8; 20];
    let p1 = vec![0x22u8; 30];
    let payloads = [&p0, &p1];

    let mut padded_payload_data = Vec::with_capacity(2 * 32);
    for p in &payloads {
        padded_payload_data.extend_from_slice(p);
        padded_payload_data.resize(padded_payload_data.len() + (32 - p.len()), 0x00);
    }

    group
        .add_channel_typed(
            "ETH_Frame.DataBytes",
            "",
            SignalValues::Bytes {
                data: padded_payload_data,
                width: 32,
            },
        )
        .unwrap();

    let (_temp, file) = open_written(&writer);

    let eth_groups = file.eth_frame_groups();
    assert_eq!(eth_groups.len(), 1);

    let frames = file
        .eth_frames(eth_groups[0])
        .expect("decode eth frames without optional channels");
    assert_eq!(frames.len(), 2);

    for i in 0..2 {
        let frame = frames.get(i).unwrap();
        assert_eq!(frame.timestamp, timestamps[i]);
        assert_eq!(frame.source, None, "source MAC should be None when absent");
        assert_eq!(
            frame.destination, None,
            "destination MAC should be None when absent"
        );
        assert_eq!(frame.bus_channel, 0, "bus channel should be 0 when absent");
        assert_eq!(frame.ether_type, ether_types[i]);
        assert_eq!(frame.data, payloads[i].as_slice());
    }
}

#[test]
fn missing_data_bytes_channel_returns_error_naming_the_channel() {
    let timestamps = [0.0, 1.0];
    let mut writer = Mf4Writer::with_start_time_ns(0);
    let group = writer.add_group(&timestamps).unwrap();

    group
        .add_channel_typed(
            "ETH_Frame.EtherType",
            "",
            SignalValues::U16(vec![0x0800, 0x0800]),
        )
        .unwrap();
    group
        .add_channel_typed("ETH_Frame.DataLength", "", SignalValues::U32(vec![10, 10]))
        .unwrap();

    let (_temp, file) = open_written(&writer);

    // Group lacks DataBytes, so eth_frame_groups should not find it
    assert!(file.eth_frame_groups().is_empty());

    // Calling eth_frames directly on the channel group must return Err naming ETH_Frame.DataBytes
    let raw_group = &file.data_groups()[0].channel_groups[0];
    let err = file.eth_frames(raw_group).unwrap_err();

    match &err {
        Mf4Error::ChannelNotFound { name } => {
            assert_eq!(name, "ETH_Frame.DataBytes");
        }
        other => panic!("expected Mf4Error::ChannelNotFound, got {other:?}"),
    }

    let err_string = err.to_string();
    assert!(
        err_string.contains("ETH_Frame.DataBytes"),
        "error message must name the missing channel: {err_string}"
    );
}
