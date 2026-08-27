//! Fuzz target for DBC CAN database parsing and decoding.
//!
//! Any panic, abort or hang here is a bug. Malformed input must produce an
//! `Err`, because DBC files arrive from tools, databases and network shares,
//! and a library that crashes on one takes its caller down with it.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run dbc
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use falcon_mdf::candb::CanDatabase;

fuzz_target!(|data: &[u8]| {
    if let Ok(db) = CanDatabase::from_dbc(data) {
        // Exercise decoding on parsed messages
        for message in db.messages() {
            let dummy = [0u8; 64];
            let len = (message.length as usize).min(64);
            let _ = db.decode(message.id, &dummy[..len]);
        }
    }
});
