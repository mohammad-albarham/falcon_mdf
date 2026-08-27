//! Fuzz target for AUTOSAR ARXML database parsing and decoding.
//!
//! Any panic, abort or hang here is a bug. Malformed input must produce an
//! `Err`, because ARXML files arrive from tools, OEM databases and network shares,
//! and a library that crashes on one takes its caller down with it.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run arxml
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use falcon_mdf::candb::CanDatabase;

fuzz_target!(|data: &[u8]| {
    let path = std::env::temp_dir().join(format!("falcon_mdf_arxml_fuzz_{}.arxml", std::process::id()));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    if let Ok(db) = CanDatabase::from_arxml_path(&path) {
        for message in db.messages() {
            let dummy = [0u8; 64];
            let len = (message.length as usize).min(64);
            let _ = db.decode(message.id, &dummy[..len]);
        }
    }
    let _ = std::fs::remove_file(&path);
});
