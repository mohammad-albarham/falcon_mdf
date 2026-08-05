//! The CSV export's format, pinned byte for byte.
//!
//! The expected texts below are exactly what the `export_to_csv` example
//! produced for these channels before the example and the GUI were unified
//! on `write_csv` — captured, not derived, so the test fails if the shared
//! function ever drifts from the format the example established. The
//! multi-column case has no pre-existing artefact to capture; its expected
//! text is hand-derived from the same two single-column exports.

use falcon_mdf::Mf4File;

const FILE: &str = "test_data/reference/dSPACE_LinearConversion.mf4";

#[test]
fn a_single_channel_export_is_byte_identical_to_the_example() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let channel = file
        .find_channel("Signal_LinearConversion")
        .expect("channel");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[channel], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],Signal_LinearConversion\n\
         0.000000000,0.000000000\n\
         0.001000000,3.280950000\n\
         0.002000000,6.561900000\n\
         0.003000000,9.842850000\n\
         0.004000000,13.123800000\n"
    );
}

#[test]
fn exporting_the_master_uses_its_own_times() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let master = file.find_channel("XAxis").expect("master");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[master], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],XAxis\n\
         0.000000000,0.000000000\n\
         0.001000000,0.001000000\n\
         0.002000000,0.002000000\n\
         0.003000000,0.003000000\n\
         0.004000000,0.004000000\n"
    );
}

#[test]
fn several_channels_share_the_first_channels_time_column() {
    let file = Mf4File::open(FILE).expect("reference file opens");
    let signal = file
        .find_channel("Signal_LinearConversion")
        .expect("channel");
    let master = file.find_channel("XAxis").expect("master");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[signal, master], &mut out).expect("export");

    assert_eq!(
        String::from_utf8(out).expect("csv is text"),
        "Time [],Signal_LinearConversion,XAxis\n\
         0.000000000,0.000000000,0.000000000\n\
         0.001000000,3.280950000,0.001000000\n\
         0.002000000,6.561900000,0.002000000\n\
         0.003000000,9.842850000,0.003000000\n\
         0.004000000,13.123800000,0.004000000\n"
    );
}

#[test]
fn exporting_nothing_writes_nothing() {
    let file = Mf4File::open(FILE).expect("reference file opens");

    let mut out = Vec::new();
    falcon_mdf::write_csv(&file, &[], &mut out).expect("export");
    assert!(out.is_empty());
}
