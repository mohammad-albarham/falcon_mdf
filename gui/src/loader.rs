//! Opens an MF4 file on a worker thread so the UI never blocks.
//!
//! `Mf4File` is `Send + Sync` (its caches are `RwLock`, and `ByteSource` is
//! declared `Send + Sync`), so the file is opened on a plain OS thread and
//! handed back over a channel; the UI thread only ever touches the `Arc`.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use falcon_mdf::error::Mf4Error;
use falcon_mdf::Mf4File;

use crate::model::LoadedFile;

pub enum LoadResult {
    Ok(LoadedFile),
    Err { path: PathBuf, message: String },
}

/// Starts opening `path` on a new thread and returns a receiver for the
/// result. `ctx` is used to wake the UI once loading finishes, since eframe
/// otherwise only repaints in response to input.
pub fn spawn_load(path: PathBuf, ctx: egui::Context) -> Receiver<LoadResult> {
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        // `open_buffered`, not the mmap default: a GUI holds a file open for a
        // long time while whatever wrote it may still be running, and
        // `memmap2::Mmap` is unsound (SIGBUS, uncatchable) if the file is
        // truncated out from under the mapping while it's held. Buffered I/O
        // copies what it reads instead, so it has no such requirement — see
        // bug B14.
        let result = match Mf4File::open_buffered(&path) {
            Ok(file) => LoadResult::Ok(LoadedFile::new(Arc::new(file), path)),
            Err(e) => LoadResult::Err {
                path,
                message: load_error_message(&e),
            },
        };
        let _ = tx.send(result);
        ctx.request_repaint();
    });

    rx
}

/// The message shown when a file fails to open. An MDF 2.x / 3.x file is
/// named as such: the library's own error ("Unsupported MF4 version: 3.30")
/// reads as a damaged MF4 file, when the file is fine and this viewer simply
/// does not open that format.
pub fn load_error_message(error: &Mf4Error) -> String {
    match error {
        Mf4Error::UnsupportedVersion {
            major: major @ (2 | 3),
            minor,
        } => {
            format!(
                "This is an MDF {major}.{minor:02} file. The viewer opens MDF 4.x (.mf4) files only; \
                 convert it to MF4 first (for example with asammdf)."
            )
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mdf3_file_is_named_as_mdf3_not_as_a_bad_mf4() {
        let message = load_error_message(&Mf4Error::UnsupportedVersion {
            major: 3,
            minor: 30,
        });
        assert!(message.contains("MDF 3.30"), "{message}");
        assert!(message.contains("MDF 4.x"), "{message}");
    }

    #[test]
    fn an_unknown_mf4_version_keeps_the_library_message() {
        let error = Mf4Error::UnsupportedVersion { major: 5, minor: 0 };
        assert_eq!(load_error_message(&error), error.to_string());
    }
}
