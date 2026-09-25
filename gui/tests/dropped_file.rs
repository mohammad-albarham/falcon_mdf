//! Tests for which file a drag-and-drop onto the falcon window opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use falcon_mdf_gui::app::dropped_path;

/// Stands in for the file handle the windowing integration hands egui; only
/// the path matters to the viewer, which reads the file itself.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("not read in tests".to_string())
    }
}

fn dropped(paths: &[&str]) -> Vec<egui::DroppedFileHandle> {
    paths
        .iter()
        .map(|p| Arc::new(Dropped(PathBuf::from(p))) as egui::DroppedFileHandle)
        .collect()
}

#[test]
fn a_single_drop_opens_that_file() {
    assert_eq!(
        dropped_path(&dropped(&["/data/run.mf4"])),
        Some(PathBuf::from("/data/run.mf4"))
    );
}

#[test]
fn a_multi_file_drop_opens_the_first() {
    assert_eq!(
        dropped_path(&dropped(&["/data/a.mf4", "/data/b.mf4"])),
        Some(PathBuf::from("/data/a.mf4"))
    );
}

#[test]
fn an_entry_without_a_path_is_skipped() {
    assert_eq!(
        dropped_path(&dropped(&["", "/data/b.mf4"])),
        Some(PathBuf::from("/data/b.mf4"))
    );
    assert_eq!(dropped_path(&dropped(&[""])), None);
}

#[test]
fn nothing_dropped_opens_nothing() {
    assert_eq!(dropped_path(&[]), None);
}
