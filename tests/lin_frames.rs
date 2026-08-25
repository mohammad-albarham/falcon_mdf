//! LIN frame extraction checked against real bus logs.
//!
//! The counterpart to `bus_frames.rs` for LIN: the frames come out of
//! reference logs written by a logger, not out of a fixture written to agree
//! with this reader. The LIN groups in these files compose their fields
//! under `LIN_Frame` — `ID`, `DataLength`, `DataBytes`, `BusChannel`, `Dir` —
//! with the group master as the timestamp.
//!
//! The corpus is not checked in. These tests skip when it is absent, as
//! `golden.rs` and `reference.rs` do.

use std::path::PathBuf;

use falcon_mdf::{ChannelGroup, Mf4File};

/// The reference files that hold LIN groups.
const PATHS: [&str; 2] = [
    "test_data/reference/single_lin_bus_1.MF4",
    "test_data/reference/multiple.MF4",
];

fn resolve_path(rel: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(rel),
        PathBuf::from("../../falcon_mdf").join(rel),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn present_paths() -> Vec<String> {
    PATHS
        .into_iter()
        .filter_map(|p| resolve_path(p).map(|pb| pb.to_string_lossy().to_string()))
        .collect()
}

fn skip_if_empty(paths: &[String]) -> bool {
    if paths.is_empty() {
        eprintln!("SKIP: no LIN reference files present under test_data/reference/");
        return true;
    }
    false
}

/// The groups holding logged LIN frames. There is no `lin_frame_groups` on
/// `Mf4File` the way there is for CAN, so detection is by the frame channels
/// being present — which is what reading them needs anyway.
fn lin_groups(file: &Mf4File) -> Vec<&ChannelGroup> {
    file.data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .filter(|cg| {
            cg.find_channel("LIN_Frame.ID").is_some()
                && cg.find_channel("LIN_Frame.DataBytes").is_some()
        })
        .collect()
}

/// A reader that returned more or fewer frames than the group logged would
/// have to have invented or dropped records somewhere.
#[test]
fn frame_count_matches_the_group_sample_count() {
    let paths = present_paths();
    if skip_if_empty(&paths) {
        return;
    }
    let mut checked = 0usize;

    for path in &paths {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        for group in lin_groups(&file) {
            let frames = file
                .lin_frames(group)
                .unwrap_or_else(|e| panic!("{path} group {}: {e}", group.index));
            assert_eq!(
                frames.len() as u64,
                group.sample_count,
                "{path} group '{}': frame count",
                group.acquisition_name
            );
            checked += 1;
        }
    }

    assert!(checked > 0, "no LIN frame group was found in the corpus");
    eprintln!("checked {checked} LIN frame group(s)");
}

#[test]
fn frames_are_in_logging_order() {
    let paths = present_paths();
    if skip_if_empty(&paths) {
        return;
    }

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in lin_groups(&file) {
            let mut previous = f64::NEG_INFINITY;
            for (i, frame) in file.lin_frames(group).unwrap().iter().enumerate() {
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

/// LIN identifiers are six bits; a bit that does not belong to the number
/// (a parity bit, say) must not leak into the reported identifier.
#[test]
fn identifiers_are_six_bits() {
    let paths = present_paths();
    if skip_if_empty(&paths) {
        return;
    }
    let mut frames_seen = 0usize;

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in lin_groups(&file) {
            for (i, frame) in file.lin_frames(group).unwrap().iter().enumerate() {
                frames_seen += 1;
                assert!(
                    frame.id <= 63,
                    "{path} group {} frame {i}: identifier {:#X} exceeds 6 bits",
                    group.index,
                    frame.id
                );
            }
        }
    }

    assert!(frames_seen > 0, "no LIN frame was read from the corpus");
}

/// Every frame's payload is exactly as long as the record's DataLength field
/// says it is — a fixed-width payload channel pads short frames out to its
/// full width, and the frame must not hand that padding back as data.
#[test]
fn payloads_are_trimmed_to_the_logged_length() {
    let paths = present_paths();
    if skip_if_empty(&paths) {
        return;
    }

    for path in &paths {
        let file = Mf4File::open(path).unwrap();
        for group in lin_groups(&file) {
            let length_channel = group
                .find_channel("LIN_Frame.DataLength")
                .expect("a LIN frame group has a DataLength channel");
            let lengths = file
                .signal(length_channel)
                .unwrap()
                .values()
                .unwrap()
                .to_f64();
            let frames = file.lin_frames(group).unwrap();

            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(
                    frame.data.len() as f64,
                    lengths[i],
                    "{path} group {} frame {i}: payload length",
                    group.index
                );
            }
        }
    }
}
