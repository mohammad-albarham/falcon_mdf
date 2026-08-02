//! Fuzzes the whole read path: open a file, then decode every channel.
//!
//! Any panic, abort or hang here is a bug. Malformed input must produce an
//! `Err`, because these files arrive from loggers, customers and network
//! shares, and a library that crashes on one takes its caller down with it.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run parse
//! ```
//!
//! Seed the corpus from real files first, so the fuzzer starts from inputs that
//! parse rather than having to discover the format:
//!
//! ```text
//! mkdir -p fuzz/corpus/parse
//! find test_data -name '*.MF4' -exec cp {} fuzz/corpus/parse/ \;
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The reader is path-based, so the input has to reach it as a file. Each
    // process gets its own path; libFuzzer runs one input at a time per process.
    let path = std::env::temp_dir().join(format!("falcon_mdf_fuzz_{}.mf4", std::process::id()));

    if std::fs::write(&path, data).is_err() {
        return;
    }

    if let Ok(file) = falcon_mdf::Mf4File::open(&path) {
        // Opening is only half the work — decoding is where record striding,
        // bit extraction and conversions run, so exercise it too.
        let channels: Vec<_> = file.channels().cloned().collect();
        for channel in &channels {
            if let Ok(signal) = file.signal(channel) {
                let _ = signal.values();
                let _ = signal.validity();
            }
        }
    }

    let _ = std::fs::remove_file(&path);
});
