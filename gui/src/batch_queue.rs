//! The list of files a batch will run over, remembered between runs.
//!
//! Stored as newline-separated paths under one key, the same hand-rolled
//! arrangement [`crate::recent`] uses and for the same reason: `serde` is a
//! large dependency to add for a `Vec<PathBuf>`. Kept apart from
//! [`crate::session`], which is keyed by the open file — a batch queue is not
//! about any one measurement, and survives whichever file happens to be open.
//!
//! The channel count beside each entry is filled in when the file is first
//! read and is not stored: it is a fact about the file, and the file can be
//! rewritten between runs.

use std::path::{Path, PathBuf};

const STORAGE_KEY: &str = "batch_queue";

/// How many files a queue holds. A batch is a working set, not an archive,
/// and a stored list this long already takes longer to run than to rebuild.
const MAX_ENTRIES: usize = 200;

/// One queued file.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    /// The file itself.
    pub path: PathBuf,
    /// How many channels it holds, once something has looked. `None` before
    /// anything has looked, and after a look that failed — which is not an
    /// error here, only a column with nothing in it.
    pub channels: Option<usize>,
    /// Whether the file has been looked at yet.
    ///
    /// Separate from `channels` because a file that will not open leaves that
    /// `None` for good: without this the panel would reopen it on every
    /// frame, which is the freeze the lazy count exists to avoid.
    pub inspected: bool,
}

impl QueueEntry {
    /// A new entry, not yet inspected.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            channels: None,
            inspected: false,
        }
    }

    /// The file's own name, which is what the queue shows.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// The files a batch will run over, in the order they were added.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct BatchQueue {
    entries: Vec<QueueEntry>,
}

impl BatchQueue {
    /// Reads the stored queue.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut queue = Self::default();
        let Some(text) = storage.and_then(|s| s.get_string(STORAGE_KEY)) else {
            return queue;
        };
        for line in text.lines() {
            queue.push(Path::new(line));
        }
        queue
    }

    /// Writes the queue out.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.encode());
    }

    /// The stored form: one path per line, blank paths dropped.
    ///
    /// A free function's worth of logic kept separate from [`Self::save`] so a
    /// round trip can be tested without an `eframe::Storage` — see
    /// `gui/tests/batch_queue_roundtrip.rs`.
    pub fn encode(&self) -> String {
        self.entries
            .iter()
            .filter_map(|e| e.path.to_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Reads the form [`Self::encode`] writes.
    pub fn decode(text: &str) -> Self {
        let mut queue = Self::default();
        for line in text.lines() {
            queue.push(Path::new(line));
        }
        queue
    }

    /// Adds `path` to the end, ignoring one already queued. A batch applies
    /// one operation to each file, so the same file twice would only be the
    /// same output written twice.
    pub fn push(&mut self, path: &Path) -> bool {
        if path.as_os_str().is_empty() || self.entries.len() >= MAX_ENTRIES {
            return false;
        }
        if self.entries.iter().any(|e| e.path == path) {
            return false;
        }
        self.entries.push(QueueEntry::new(path.to_path_buf()));
        true
    }

    /// Drops the entry at `index`, if there is one.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// Empties the queue.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Every entry, in queue order.
    pub fn entries(&self) -> &[QueueEntry] {
        &self.entries
    }

    /// Every entry, mutably, so a channel count can be filled in.
    pub fn entries_mut(&mut self) -> &mut [QueueEntry] {
        &mut self.entries
    }

    /// The paths, in queue order — what a run is given.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.iter().map(|e| e.path.clone()).collect()
    }

    /// How many files are queued.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the queue holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
