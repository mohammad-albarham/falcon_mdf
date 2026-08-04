//! A small recently-opened-files list, persisted via eframe's storage.
//!
//! Stored as newline-separated paths under one key; a hand-rolled format was
//! chosen over pulling in `serde` just to serialize a `Vec<PathBuf>`.

use std::path::{Path, PathBuf};

const STORAGE_KEY: &str = "recent_files";
const MAX_ENTRIES: usize = 10;

#[derive(Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths = storage
            .and_then(|s| s.get_string(STORAGE_KEY))
            .map(|s| s.lines().map(PathBuf::from).collect())
            .unwrap_or_default();
        Self { paths }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let joined = self
            .paths
            .iter()
            .filter_map(|p| p.to_str())
            .collect::<Vec<_>>()
            .join("\n");
        storage.set_string(STORAGE_KEY, joined);
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Moves `path` to the front, deduplicating and capping the list length.
    pub fn push(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_ENTRIES);
    }
}
