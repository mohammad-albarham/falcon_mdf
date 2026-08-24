//! Shared types describing an opened file and its channels for display.
//!
//! This is the seam G2/G3 (plotting) build on: [`LoadedFile`] holds the
//! `Arc<Mf4File>` plus the flattened rows the browser renders,
//! `App::selected` (in `app.rs`) tracks a channel for the detail pane, and
//! `App::plotted` holds the [`PlottedChannel`] set the plot panel draws —
//! all keyed by the same (data group, channel group, channel) location.

use std::sync::Arc;

use falcon_mdf::{BlockMap, Mf4File};

/// Where a channel lives in the file, independent of any borrow of it.
///
/// Small and `Copy` so it can be stored as "the selected channel" without
/// holding a reference into `Mf4File` across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelLoc {
    pub data_group_index: usize,
    pub channel_group_index: usize,
    pub channel_index: usize,
}

/// Colors assigned to plotted channels in insertion order. A fixed palette
/// rather than anything computed: eight distinguishable hues is plenty
/// before lines become unreadable anyway, and cycling is easier to recognize
/// than a hash-derived rainbow.
pub const PALETTE: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x1f, 0x77, 0xb4),
    egui::Color32::from_rgb(0xff, 0x7f, 0x0e),
    egui::Color32::from_rgb(0x2c, 0xa0, 0x2c),
    egui::Color32::from_rgb(0xd6, 0x27, 0x28),
    egui::Color32::from_rgb(0x94, 0x67, 0xbd),
    egui::Color32::from_rgb(0x8c, 0x56, 0x4b),
    egui::Color32::from_rgb(0xe3, 0x77, 0xc2),
    egui::Color32::from_rgb(0xbc, 0xbd, 0x22),
];

/// A channel the user has asked to plot, with the display state that only
/// the plot cares about. The unit is not duplicated here: the decoded
/// `ChannelSignal` carries it for the plot's axis labels and legend.
pub struct PlottedChannel {
    pub loc: ChannelLoc,
    pub name: String,
    pub color: egui::Color32,
    /// Drawn or just remembered: unticking a channel keeps its slot in the
    /// list (and its color) instead of throwing the decode away.
    pub visible: bool,
}

impl PlottedChannel {
    /// `insertion_index` is the plotted-list length at the moment the channel
    /// is added, so colors are handed out in palette order and stay with the
    /// channel for as long as it stays plotted.
    pub fn new(loc: ChannelLoc, name: String, insertion_index: usize) -> Self {
        Self {
            loc,
            name,
            color: PALETTE[insertion_index % PALETTE.len()],
            visible: true,
        }
    }
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
        /// Why this channel cannot be read (`Channel::unreadable`), rendered
        /// to text once at row-build time. `None` for an ordinary channel.
        unreadable: Option<String>,
    },
}

/// What the left-hand trees have selected, and therefore what the content
/// area on the right is about.
///
/// One enum rather than one field per kind: the three trees pick different
/// things out of the same file, and the content area has to answer "what am
/// I showing" with a single question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The file as a whole — the state a freshly opened file starts in.
    File,
    DataGroup(usize),
    ChannelGroup {
        data_group_index: usize,
        channel_group_index: usize,
    },
    Channel(ChannelLoc),
    /// A block, by its file offset. Blocks are addressed rather than indexed
    /// because that is how the file itself refers to them, and because it
    /// survives a re-scan.
    Block(u64),
    Attachment(usize),
    Event(usize),
    HistoryEntry(usize),
}

/// Which view of the file the content area on the right is showing.
///
/// The Details tab follows the selection; the others are about a channel or
/// a group and say so when the selection is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTab {
    Details,
    Plot,
    Numeric,
    Table,
    Bus,
    Statistics,
}

impl ContentTab {
    pub const ALL: [ContentTab; 6] = [
        ContentTab::Details,
        ContentTab::Plot,
        ContentTab::Numeric,
        ContentTab::Table,
        ContentTab::Bus,
        ContentTab::Statistics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ContentTab::Details => "Details",
            ContentTab::Plot => "Plot",
            ContentTab::Numeric => "Numeric",
            ContentTab::Table => "Samples",
            ContentTab::Bus => "Bus",
            ContentTab::Statistics => "Statistics",
        }
    }

    /// The tab a stored session names, or the default when it names
    /// something this version does not have.
    pub fn from_label(label: &str) -> Self {
        ContentTab::ALL
            .into_iter()
            .find(|tab| tab.label() == label)
            .unwrap_or(ContentTab::Details)
    }
}

/// A successfully opened file, plus the display rows built from it once at
/// load time so the UI does not walk the whole channel tree every frame.
pub struct LoadedFile {
    pub file: Arc<Mf4File>,
    pub path: std::path::PathBuf,
    /// Every channel, grouped under data-group/channel-group headers, in file
    /// order. This is what the browser shows when the search box is empty.
    pub all_rows: Vec<Row>,
    /// Every block in the file, in address order. Built once on the loader
    /// thread: the walk is one read per block, which is cheap, but it is
    /// still I/O and does not belong in the frame loop.
    pub blocks: BlockMap,
}

impl LoadedFile {
    pub fn new(file: Arc<Mf4File>, path: std::path::PathBuf) -> Self {
        let all_rows = build_rows(&file);
        let blocks = file.block_map();
        Self {
            file,
            path,
            all_rows,
            blocks,
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
                    unreadable: ch.unreadable().map(|r| r.to_string()),
                });
            }
        }
    }
    rows
}
