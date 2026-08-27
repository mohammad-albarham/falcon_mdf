//! Fuzz target for MDF 3.x file parsing.
//!
//! Any panic, abort or hang here is a bug. Malformed input must produce an
//! `Err`, because these files arrive from loggers, customers and network
//! shares, and a library that crashes on one takes its caller down with it.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run mdf3
//! ```
//!
//! Seed the corpus from real files first, so the fuzzer starts from inputs that
//! parse rather than having to discover the format:
//!
//! ```text
//! mkdir -p fuzz/corpus/mdf3
//! find test_data -name '*.MF3' -exec cp {} fuzz/corpus/mdf3/ \;
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use falcon_mdf::mdf3::Mdf3File;

fuzz_target!(|data: &[u8]| {
    // The reader is path-based, so the input has to reach it as a file.
    let path = std::env::temp_dir().join(format!("falcon_mdf_mdf3_fuzz_{}.mf3", std::process::id()));

    if std::fs::write(&path, data).is_err() {
        return;
    }

    if let Ok(file) = Mdf3File::open(&path) {
        // Opening is only half the work — decoding channels is where
        // record striding, bit extraction and conversions run.
        let channels: Vec<_> = file.channel_names().into_iter().collect();
        for name in &channels {
            // Raw values
            let _ = file.values_by_name(name);
            // Physical values (with conversion applied)
            let _ = file.physical_by_name(name);
        }
        // Also try indexed access
        for (gi, dg) in file.data_groups().iter().enumerate() {
            for (ci, cg) in dg.channel_groups.iter().enumerate() {
                for (chi, _) in cg.channels.iter().enumerate() {
                    let _ = file.channel_values(gi, ci, chi);
                    let _ = file.channel_physical(gi, ci, chi);
                }
            }
        }
    }

    let _ = std::fs::remove_file(&path);
});
