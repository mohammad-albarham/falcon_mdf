//! Which channels count as plottable: the X-Y view picks its default axes
//! from them, so a value-to-text channel plotted first must not be chosen.
//!
//! The corpus is not checked in; this skips when it is absent, as the
//! other reference-file tests do.

use std::path::{Path, PathBuf};

use falcon_mdf::Mf4File;
use falcon_mdf_gui::model::{is_plottable, ChannelLoc};

fn reference_file() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gui/ has a parent")
        .join("test_data")
        .join("reference")
        .join("ASAP2_Demo_V171.mf4");
    path.exists().then_some(path)
}

fn loc_of(file: &Mf4File, name: &str) -> ChannelLoc {
    for (dg, group) in file.data_groups().iter().enumerate() {
        for (cg, channel_group) in group.channel_groups.iter().enumerate() {
            if let Some(ch) = channel_group.channels.iter().position(|c| c.name == name) {
                return ChannelLoc {
                    data_group_index: dg,
                    channel_group_index: cg,
                    channel_index: ch,
                };
            }
        }
    }
    panic!("{name} is not in the reference file");
}

#[test]
fn text_conversions_are_not_plottable_and_numbers_are() {
    let Some(path) = reference_file() else {
        return;
    };
    let file = Mf4File::open_buffered(&path).expect("the reference file opens");

    for name in [
        "ASAM.M.SCALAR.UBYTE.VTAB_RANGE_NO_DEFAULT_VALUE",
        "ASAM.M.SCALAR.UBYTE.TAB_VERB_DEFAULT_VALUE",
    ] {
        assert!(
            !is_plottable(&file, loc_of(&file, name)),
            "{name} converts to text"
        );
    }
    for name in [
        "ASAM.M.SCALAR.SLONG.IDENTICAL",
        "ASAM.M.SCALAR.FLOAT64.IDENTICAL",
    ] {
        assert!(
            is_plottable(&file, loc_of(&file, name)),
            "{name} is numeric"
        );
    }
    // A location the file does not have is not plottable either.
    let nowhere = ChannelLoc {
        data_group_index: 999,
        channel_group_index: 0,
        channel_index: 0,
    };
    assert!(!is_plottable(&file, nowhere));
}
