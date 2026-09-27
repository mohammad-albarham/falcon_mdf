//! Ethernet and FlexRay frames read from files written by `ihedvall/mdflib`, an
//! independent C++ MDF implementation — the first check of these readers
//! against bytes this crate did not produce. Until these fixtures, both were
//! pinned only by files `Mf4Writer` wrote, which shared every guess the
//! readers made about channel names and flag meanings.
//!
//! The fixtures are generated, not committed: run
//! `scripts/make_mdflib_bus_fixtures.sh`, which builds mdflib at a pinned
//! commit and runs `scripts/mdflib_bus_fixtures/main.cpp`. Every expected value
//! below is recomputed from the formulas in that program, not read back from
//! the files.

use std::path::{Path, PathBuf};

use falcon_mdf::Mf4File;

const FRAMES: usize = 24;

fn fixture(name: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_data/generated")
        .join(name);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "skipping: {} is missing. Run scripts/make_mdflib_bus_fixtures.sh to generate it.",
            path.display()
        );
        None
    }
}

#[test]
fn ethernet_frames_from_mdflib_match_what_it_was_told_to_write() {
    let Some(path) = fixture("mdflib_ethernet.mf4") else {
        return;
    };
    let file = Mf4File::open(&path).expect("mdflib's Ethernet log opens");
    let group = file
        .data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .find(|cg| cg.acquisition_name == "ETH_Frame")
        .expect("an ETH_Frame group");
    let frames = file.eth_frames(group).expect("ETH_Frame reads as frames");
    assert_eq!(frames.len(), FRAMES);

    for (i, frame) in frames.iter().enumerate() {
        let len = i % 9 + 1;
        let payload: Vec<u8> = (0..len).map(|k| ((i * 7 + k) & 0xFF) as u8).collect();
        assert_eq!(frame.data, &payload[..], "frame {i} payload");
        assert_eq!(frame.ether_type, 0x0800 + i as u16, "frame {i} EtherType");
        assert_eq!(
            frame.bus_channel,
            (i % 3 + 1) as u8,
            "frame {i} bus channel"
        );
        assert_eq!(
            frame.source,
            Some([0x02, 0, 0, 0, 0, i as u8]),
            "frame {i} source"
        );
        assert_eq!(
            frame.destination,
            Some([0x02, 0xFF, 0, 0, 0, i as u8]),
            "frame {i} destination"
        );
    }
    let t: Vec<f64> = frames.iter().map(|f| f.timestamp).collect();
    for pair in t.windows(2) {
        assert!(
            (pair[1] - pair[0] - 0.001).abs() < 1e-9,
            "1 ms apart: {pair:?}"
        );
    }
}

#[test]
fn flexray_frames_from_mdflib_match_what_it_was_told_to_write() {
    let Some(path) = fixture("mdflib_flexray.mf4") else {
        return;
    };
    let file = Mf4File::open(&path).expect("mdflib's FlexRay log opens");
    let group = file
        .data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .find(|cg| cg.acquisition_name == "FLX_Frame")
        .expect("an FLX_Frame group");
    let frames = file
        .flexray_frames(group)
        .expect("FLX_Frame reads as frames");
    assert_eq!(frames.len(), FRAMES);

    for (i, frame) in frames.iter().enumerate() {
        let len = 2 * (i % 5);
        let payload: Vec<u8> = (0..len).map(|k| ((i * 5 + k) & 0xFF) as u8).collect();
        assert_eq!(frame.data, &payload[..], "frame {i} payload");
        assert_eq!(frame.frame_id, 100 + i as u16, "frame {i} ID");
        assert_eq!(frame.cycle, ((i * 3) & 0x3F) as u8, "frame {i} cycle");
        assert_eq!(
            frame.bus_channel,
            (i % 2 + 1) as u8,
            "frame {i} bus channel"
        );
        assert_eq!(frame.null_frame, i % 6 == 5, "frame {i} null frame");
        assert_eq!(frame.sync_frame, i % 4 == 0, "frame {i} sync frame");
        assert_eq!(frame.startup, i % 8 == 0, "frame {i} startup frame");
    }
}
