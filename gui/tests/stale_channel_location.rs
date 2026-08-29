//! A `ChannelLoc` that no longer names a channel must come back as an error,
//! not as a panic.
//!
//! Locations outlive the file they were made against: a session restored
//! against a file that was rewritten shorter, a computed channel carrying a
//! sentinel location, a second file swapped under a plot. `decode_channel`
//! runs on a worker thread, so a panic there reaches the panels only as "the
//! worker thread ended without a result" — a message that names neither the
//! channel nor the reason, on a thread whose unwind the user never sees.
//!
//! The corpus is not checked in; this skips when it is absent, as the
//! library's own corpus tests do.

use std::path::{Path, PathBuf};

use falcon_mdf::Mf4File;
use falcon_mdf_gui::model::ChannelLoc;
use falcon_mdf_gui::signal_loader::{decode_channel, SignalLoadResult};

fn reference_file() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gui/ has a parent")
        .join("test_data")
        .join("reference")
        .join("ASAP2_Demo_V171.mf4");
    path.exists().then_some(path)
}

fn assert_reported(file: &Mf4File, loc: ChannelLoc, what: &str) {
    match decode_channel(file, loc) {
        SignalLoadResult::Err { message } => assert!(
            message.contains("no channel at"),
            "{what}: expected a located error, got {message:?}"
        ),
        SignalLoadResult::Ok(signal) => {
            panic!("{what}: expected an error, decoded {:?}", signal.name)
        }
    }
}

#[test]
fn out_of_range_locations_are_reported_not_panicked() {
    let Some(path) = reference_file() else {
        return;
    };
    let file = Mf4File::open_buffered(&path).expect("the reference file opens");

    let groups = file.data_groups().len();
    let channels = file.data_groups()[0].channel_groups[0].channels.len();

    assert_reported(
        &file,
        ChannelLoc {
            data_group_index: groups,
            channel_group_index: 0,
            channel_index: 0,
        },
        "data group past the end",
    );
    assert_reported(
        &file,
        ChannelLoc {
            data_group_index: 0,
            channel_group_index: usize::MAX,
            channel_index: 0,
        },
        "channel group past the end",
    );
    assert_reported(
        &file,
        ChannelLoc {
            data_group_index: 0,
            channel_group_index: 0,
            channel_index: channels,
        },
        "channel past the end",
    );
}

#[test]
fn the_computed_channel_sentinel_location_is_reported_not_panicked() {
    let Some(path) = reference_file() else {
        return;
    };
    let file = Mf4File::open_buffered(&path).expect("the reference file opens");

    // `computed::eval_expr` gives a constant expression this location, since
    // no channel in any file produced it. Nothing should route it back here,
    // but "nothing should" is what a crash in a released build is made of.
    assert_reported(
        &file,
        ChannelLoc {
            data_group_index: usize::MAX,
            channel_group_index: 0,
            channel_index: 0,
        },
        "the computed-channel sentinel",
    );
}
