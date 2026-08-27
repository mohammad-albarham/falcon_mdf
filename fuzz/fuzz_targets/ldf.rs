//! Fuzz target for LIN Description File (LDF) database parsing.
//!
//! Any panic, abort or hang here is a bug. Malformed input must produce an
//! `Err`, because LDF files arrive from tools, databases and network shares,
//! and a library that crashes on one takes its caller down with it.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run ldf
//! ```
//!
//! Seed the corpus from real LDF files:
//!
//! ```text
//! mkdir -p fuzz/corpus/ldf
//! find test_data -name '*.ldf' -exec cp {} fuzz/corpus/ldf/ \;
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use falcon_mdf::candb::CanDatabase;

fuzz_target!(|data: &[u8]| {
    // Try to parse as LDF (which expects UTF-8 text)
    if let Ok(db) = CanDatabase::from_ldf(data) {
        for message in db.messages() {
            let dummy = [0u8; 64];
            let len = (message.length as usize).min(64);
            let _ = db.decode(message.id, &dummy[..len]);
        }
    }
});
