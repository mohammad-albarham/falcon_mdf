//! The batch queue survives closing the viewer.
//!
//! Queueing twenty files is the slow part of a batch; losing that list on
//! exit would make the tab useful only for as long as the window stays open.
//! The stored form is one path per line, and the round trip is tested through
//! `encode`/`decode` so it needs no `eframe::Storage` and no window.

use std::path::{Path, PathBuf};

use falcon_mdf_gui::batch_queue::BatchQueue;

#[test]
fn a_queue_survives_encoding_and_decoding() {
    let mut queue = BatchQueue::default();
    queue.push(Path::new("/data/run_01.mf4"));
    queue.push(Path::new("/data/run_02.mf4"));
    queue.push(Path::new("/other/run_03.mf4"));

    let restored = BatchQueue::decode(&queue.encode());

    assert_eq!(restored.len(), 3);
    assert_eq!(
        restored.paths(),
        vec![
            PathBuf::from("/data/run_01.mf4"),
            PathBuf::from("/data/run_02.mf4"),
            PathBuf::from("/other/run_03.mf4"),
        ],
        "order is part of the queue: a batch runs top to bottom"
    );
}

#[test]
fn an_empty_queue_round_trips_as_empty() {
    let queue = BatchQueue::default();
    assert_eq!(queue.encode(), "");
    assert!(BatchQueue::decode("").is_empty());
    assert!(BatchQueue::decode("\n").is_empty());
}

#[test]
fn the_same_file_is_not_queued_twice() {
    let mut queue = BatchQueue::default();
    assert!(queue.push(Path::new("/data/a.mf4")));
    assert!(
        !queue.push(Path::new("/data/a.mf4")),
        "one operation per file: the same file twice is the same output twice"
    );
    assert_eq!(queue.len(), 1);
}

#[test]
fn removing_takes_the_right_entry_and_ignores_the_rest() {
    let mut queue = BatchQueue::default();
    queue.push(Path::new("/a.mf4"));
    queue.push(Path::new("/b.mf4"));
    queue.push(Path::new("/c.mf4"));

    queue.remove(1);
    assert_eq!(
        queue.paths(),
        vec![PathBuf::from("/a.mf4"), PathBuf::from("/c.mf4")]
    );

    // Out of range is ignored rather than panicking: the index comes from a
    // list the user is clicking while a run may be changing it.
    queue.remove(99);
    assert_eq!(queue.len(), 2);

    queue.clear();
    assert!(queue.is_empty());
}

#[test]
fn a_channel_count_is_shown_but_not_stored() {
    let mut queue = BatchQueue::default();
    queue.push(Path::new("/a.mf4"));
    queue.entries_mut()[0].channels = Some(42);
    assert_eq!(queue.entries()[0].channels, Some(42));

    // The count is a fact about the file, and the file can be rewritten
    // between runs, so it is looked up again rather than restored.
    let restored = BatchQueue::decode(&queue.encode());
    assert_eq!(restored.entries()[0].channels, None);
}

#[test]
fn an_entry_shows_its_own_name() {
    let mut queue = BatchQueue::default();
    queue.push(Path::new("/a/very/long/path/run_07.mf4"));
    assert_eq!(queue.entries()[0].file_name(), "run_07.mf4");
}
