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

/// Big-endian ("Motorola") channels of every numeric kind, from mdflib — the
/// first big-endian MDF 4 file in the test set this crate did not write. A
/// little-endian twin sits beside them with the same values, so a reader that
/// ignored the byte order would disagree with the twin rather than pass.
#[test]
fn big_endian_channels_from_mdflib_decode_to_what_it_was_told_to_write() {
    use falcon_mdf::SignalValues;

    let Some(path) = fixture("mdflib_big_endian.mf4") else {
        return;
    };
    let file = Mf4File::open(&path).expect("mdflib's big-endian file opens");
    let values = |name: &str| {
        let ch = file
            .find_channel(name)
            .unwrap_or_else(|| panic!("{name} listed"));
        file.signal(ch).unwrap().values().unwrap()
    };
    let n = 50u64;
    let u16s: Vec<u16> = (0..n).map(|i| ((i * 1000 + 7) & 0xFFFF) as u16).collect();
    assert_eq!(values("U16_Le"), SignalValues::U16(u16s.clone()));
    assert_eq!(values("U16_Be"), SignalValues::U16(u16s));
    assert_eq!(
        values("I32_Be"),
        SignalValues::I32((0..n as i32).map(|i| -i * 100_003 + 17).collect())
    );
    assert_eq!(
        values("U64_Be"),
        SignalValues::U64((0..n).map(|i| 0x0102_0304_0506_0000 + i).collect())
    );
    assert_eq!(
        values("F32_Be"),
        SignalValues::F32((0..n).map(|i| i as f32 * 1.5 - 20.25).collect())
    );
    assert_eq!(
        values("F64_Be"),
        SignalValues::F64((0..n).map(|i| i as f64 * 0.125 - 3.0).collect())
    );
}

/// Mixed content — both byte orders, invalidation bits, UTF-8 and ASCII
/// variable-length strings — over 5,000 samples, written by mdflib once plain
/// and once through its `##DZ` compression.
#[test]
fn mixed_content_from_mdflib_decodes_plain_and_compressed() {
    use falcon_mdf::SignalValues;

    for name in ["mdflib_mixed.mf4", "mdflib_mixed_compressed.mf4"] {
        let Some(path) = fixture(name) else {
            return;
        };
        let file = Mf4File::open(&path).unwrap_or_else(|e| panic!("{name} opens: {e}"));
        let signal = |ch: &str| {
            let c = file
                .find_channel(ch)
                .unwrap_or_else(|| panic!("{name}: {ch} listed"));
            file.signal(c).unwrap()
        };
        let n = 5000u64;
        assert_eq!(
            signal("U32").values().unwrap(),
            SignalValues::U32((0..n).map(|i| (i * 7919) as u32).collect()),
            "{name}: U32"
        );
        assert_eq!(
            signal("I16_Be").values().unwrap(),
            SignalValues::I16((0..5000i16).map(|i| -(i % 1000)).collect()),
            "{name}: I16_Be"
        );
        assert_eq!(
            signal("F64").values().unwrap(),
            SignalValues::F64((0..n).map(|i| i as f64 * 0.001 - 1.0).collect()),
            "{name}: F64"
        );

        let gappy = signal("F32_Invalid");
        let validity: Vec<bool> = (0..n).map(|i| i % 7 != 0).collect();
        assert_eq!(
            gappy.validity().expect("invalidation bits"),
            validity.as_slice(),
            "{name}: F32_Invalid validity"
        );
        let values = gappy.values_f64().unwrap();
        for i in (0..n as usize).filter(|i| i % 7 != 0) {
            assert_eq!(values[i], i as f64 * 0.5, "{name}: F32_Invalid[{i}]");
        }

        let utf8: Vec<String> = (0..n)
            .map(|i| {
                let mut s = format!("sample-{i}");
                if i % 3 == 0 {
                    s.push_str("-grüß");
                }
                s
            })
            .collect();
        assert_eq!(
            signal("Text_Utf8").values().unwrap(),
            SignalValues::Str(utf8),
            "{name}: Text_Utf8"
        );
        let ascii: Vec<String> = (0..n)
            .map(|i| format!("{}{i}", "x".repeat((i % 5) as usize)))
            .collect();
        assert_eq!(
            signal("Text_Ascii").values().unwrap(),
            SignalValues::Str(ascii),
            "{name}: Text_Ascii"
        );
    }
}

/// A 3-by-4 CN-template array channel from mdflib, element `index` of sample
/// `s` holding `s * 100 + index` at mdflib's linear array index — pins both the
/// shape and the element order against an independent writer.
#[test]
fn array_channel_from_mdflib_decodes_in_its_element_order() {
    use falcon_mdf::SignalValues;

    let Some(path) = fixture("mdflib_array.mf4") else {
        return;
    };
    let file = Mf4File::open(&path).expect("mdflib's array file opens");
    let ch = file.find_channel("Matrix").expect("Matrix listed");
    assert_eq!(ch.array_shape(), Some(&[3u64, 4][..]));
    let SignalValues::Array {
        values,
        elements_per_sample,
    } = file.signal(ch).unwrap().values().unwrap()
    else {
        panic!("a fixed-shape array");
    };
    assert_eq!(elements_per_sample, 12);
    let expected: Vec<f64> = (0..10u64)
        .flat_map(|s| (0..12u64).map(move |i| (s * 100 + i) as f64))
        .collect();
    assert_eq!(values, expected);
}
