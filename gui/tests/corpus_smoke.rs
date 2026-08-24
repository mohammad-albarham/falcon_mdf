//! The viewer's own logic, run over every file in the corpus.
//!
//! The unit tests beside this one pin each helper against inputs chosen to
//! break it. This one does the opposite: it takes what the panels actually
//! build when a real file is opened — the channel rows, the block map, the
//! session pruning, the human-readable sizes — and checks the invariants
//! that must hold whatever the file contains. Vendor files carry shapes
//! nobody writes by hand: groups with no channels, channels with no name,
//! sample counts of zero, blocks nothing points at.
//!
//! It is the closest thing to opening all 67 files in the window and looking
//! at every panel, minus the window.
//!
//! The corpus is not checked in; this skips when it is absent, as the
//! library's own corpus tests do.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use falcon_mdf::Mf4File;
use falcon_mdf_gui::model::{ChannelLoc, LoadedFile, Row};
use falcon_mdf_gui::panels::blocks::human_bytes;
use falcon_mdf_gui::session::{prune_to_file, Session};

fn reference_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gui/ has a parent")
        .join("test_data")
        .join("reference");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mf4"))
        })
        .collect();
    files.sort();
    files
}

fn skip_if_empty(files: &[PathBuf]) -> bool {
    if files.is_empty() {
        eprintln!("SKIP: no corpus under test_data/reference");
        return true;
    }
    false
}

#[test]
fn every_corpus_file_builds_the_views_the_panels_draw() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    for path in &files {
        let file =
            Mf4File::open_buffered(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let loaded = LoadedFile::new(Arc::new(file), path.clone());
        let name = path.display();

        // Every channel in the file appears exactly once as a row, and every
        // channel row points at a channel that is really there. A row that
        // indexes past its group would panic the moment the list scrolled to
        // it, which is a crash the user finds rather than the test.
        let mut channel_rows = 0usize;
        for row in &loaded.all_rows {
            let Row::Channel {
                loc,
                name: channel_name,
                ..
            } = row
            else {
                continue;
            };
            channel_rows += 1;
            let channel = loaded
                .file
                .data_groups()
                .get(loc.data_group_index)
                .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
                .and_then(|cg| cg.channels.get(loc.channel_index))
                .unwrap_or_else(|| panic!("{name}: a row points at a channel that is not there"));
            assert_eq!(
                &channel.name, channel_name,
                "{name}: the row's name is not the channel's name"
            );
        }
        assert_eq!(
            channel_rows,
            loaded.file.channel_count(),
            "{name}: the channel list shows {channel_rows} of {} channels",
            loaded.file.channel_count()
        );

        // The block map the Blocks tab lists, and the addresses the Details
        // tab turns into buttons: every group and channel must name a block
        // the map actually found, or the button would lead nowhere.
        for dg in loaded.file.data_groups() {
            assert!(
                loaded.blocks.block_at(dg.block_offset()).is_some(),
                "{name}: the data group at {:#x} has no block in the map",
                dg.block_offset()
            );
            for cg in &dg.channel_groups {
                assert!(
                    loaded.blocks.block_at(cg.block_offset()).is_some(),
                    "{name}: the channel group at {:#x} has no block in the map",
                    cg.block_offset()
                );
                for channel in &cg.channels {
                    assert!(
                        loaded.blocks.block_at(channel.block_offset()).is_some(),
                        "{name}: channel {:?} names block {:#x}, which is not in the map",
                        channel.name,
                        channel.block_offset()
                    );
                }
            }
        }
    }

    eprintln!(
        "built the viewer's rows and block links for {} files",
        files.len()
    );
}

#[test]
fn a_session_never_restores_a_channel_the_file_does_not_have() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    // The session store is keyed by path, and the file at a path can be
    // rewritten between runs. Pruning is what stops a stale session from
    // indexing past the end of a group that has since shrunk.
    let path = &files[0];
    let file = Mf4File::open_buffered(path).expect("the first corpus file opens");
    let far_past_the_end = Session {
        plotted: vec![
            ChannelLoc {
                data_group_index: 999,
                channel_group_index: 0,
                channel_index: 0,
            },
            ChannelLoc {
                data_group_index: 0,
                channel_group_index: 999,
                channel_index: 0,
            },
            ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 999,
            },
        ],
        nav: String::new(),
        tab: String::new(),
    };

    assert!(
        prune_to_file(&far_past_the_end, &file).is_empty(),
        "{}: pruning kept a channel the file does not have",
        path.display()
    );
}

#[test]
fn a_session_keeps_the_channels_that_are_still_there() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    for path in files.iter().take(10) {
        let file = Mf4File::open_buffered(path).expect("corpus file opens");
        let Some(first) = file
            .data_groups()
            .first()
            .and_then(|dg| dg.channel_groups.first())
            .filter(|cg| !cg.channels.is_empty())
        else {
            continue;
        };
        let _ = first;
        let session = Session {
            plotted: vec![ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 0,
            }],
            nav: String::new(),
            tab: String::new(),
        };
        assert_eq!(
            prune_to_file(&session, &file).len(),
            1,
            "{}: pruning dropped a channel that is there",
            path.display()
        );
    }
}

#[test]
fn every_block_size_renders_as_something_a_person_can_read() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    // `human_bytes` is what the block list's size column shows, on every row
    // of every file. An empty string or a NaN in that column would be a hole
    // in the middle of the list.
    for path in &files {
        let file = Mf4File::open_buffered(path).expect("corpus file opens");
        for block in &file.block_map().blocks {
            let text = human_bytes(block.length);
            assert!(
                !text.is_empty() && !text.contains("NaN"),
                "{}: block at {:#x} of {} bytes rendered as {text:?}",
                path.display(),
                block.address,
                block.length
            );
        }
    }
}
