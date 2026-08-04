//! Opens an MF4 file on a worker thread so the UI never blocks.
//!
//! `Mf4File` is `Send + Sync` (its caches are `RwLock`, and `ByteSource` is
//! declared `Send + Sync`), so the file is opened on a plain OS thread and
//! handed back over a channel; the UI thread only ever touches the `Arc`.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

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
                message: e.to_string(),
            },
        };
        let _ = tx.send(result);
        ctx.request_repaint();
    });

    rx
}
