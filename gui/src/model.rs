//! Shared types describing an opened file and its channels for display.
//!
//! This is the seam G2 (plotting) builds on: [`LoadedFile`] holds the
//! `Arc<Mf4File>` plus the flattened rows the browser renders, and
//! `App::selected` (in `app.rs`) already tracks a single selected channel by
//! its (data group, channel group, channel) location. G2 reads that same
//! selection to decide what to plot; nothing here needs to change for it.

use std::sync::Arc;

use falcon_mdf::Mf4File;

/// Where a channel lives in the file, independent of any borrow of it.
///
/// Small and `Copy` so it can be stored as "the selected channel" without
/// holding a reference into `Mf4File` across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelLoc {
    pub data_group_index: usize,
    pub channel_group_index: usize,
    pub channel_index: usize,
}

/// One row of the flattened, virtualized channel list.
#[derive(Debug, Clone)]
pub enum Row {
    /// A data-group heading, shown once above its channel groups.
    DataGroupHeader { label: String },
    /// A channel-group heading, shown once above its channels.
    ChannelGroupHeader { label: String },
    /// A single channel.
    Channel {
        loc: ChannelLoc,
        name: String,
        unit: String,
        sample_count: u64,
    },
}

/// A successfully opened file, plus the display rows built from it once at
/// load time so the UI does not walk the whole channel tree every frame.
pub struct LoadedFile {
    pub file: Arc<Mf4File>,
    pub path: std::path::PathBuf,
    /// Every channel, grouped under data-group/channel-group headers, in file
    /// order. This is what the browser shows when the search box is empty.
    pub all_rows: Vec<Row>,
}

impl LoadedFile {
    pub fn new(file: Arc<Mf4File>, path: std::path::PathBuf) -> Self {
        let all_rows = build_rows(&file);
        Self {
            file,
            path,
            all_rows,
        }
    }
}

fn build_rows(file: &Mf4File) -> Vec<Row> {
    let mut rows = Vec::new();
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        let label = if dg.comment.is_empty() {
            format!("Data Group {dg_idx}")
        } else {
            format!("Data Group {dg_idx} \u{2014} {}", dg.comment)
        };
        rows.push(Row::DataGroupHeader { label });

        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            let label = if cg.acquisition_name.is_empty() {
                format!("Channel Group {cg_idx} ({} samples)", cg.sample_count)
            } else {
                format!(
                    "Channel Group {cg_idx} \"{}\" ({} samples)",
                    cg.acquisition_name, cg.sample_count
                )
            };
            rows.push(Row::ChannelGroupHeader { label });

            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                rows.push(Row::Channel {
                    loc: ChannelLoc {
                        data_group_index: dg_idx,
                        channel_group_index: cg_idx,
                        channel_index: ch_idx,
                    },
                    name: ch.name.clone(),
                    unit: ch.unit.clone(),
                    sample_count: cg.sample_count,
                });
            }
        }
    }
    rows
}
