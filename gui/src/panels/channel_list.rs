//! The searchable, grouped channel list.
//!
//! Files can carry thousands of channels, so rows are drawn through
//! `ScrollArea::show_rows` rather than one widget per channel — only the
//! rows actually on screen are laid out.
//!
//! Clicking a channel toggles it in the plotted set: a plotted row shows the
//! channel's color and a visibility checkbox, and clicking again removes it.
//! Channels the library already knows it cannot decode carry a warning marker
//! with the reason on hover.

use falcon_mdf::Mf4File;

use crate::model::{ChannelLoc, FileSlot, LoadedFile, PlottedChannel, Row};
use crate::search::{compile, matches, MatchMode, Pattern};

/// How many channels one press of "Plot all matching" will add. Past this a
/// plot stops being readable, and the decodes stop being cheap.
const MAX_PLOT_ALL: usize = 32;

/// One filtered channel result with its location and group identity.
#[derive(Debug, Clone)]
struct FilteredChannel {
    loc: ChannelLoc,
    name: String,
    unit: String,
    group_label: String,
    sample_count: u64,
    unreadable: Option<String>,
}

/// Search state and the (cached) filtered row list.
///
/// Filtering rebuilds `filtered_channels` only when the query text or filter
/// toggles change, not every frame — egui redraws continuously while the
/// window is visible, and walking the channel tree is not free for large files.
#[derive(Default)]
pub struct ChannelBrowser {
    pub search: String,
    /// How the query is read: as a substring, a wildcard, or a regex.
    pub mode: MatchMode,
    /// What the last compile said, when it failed. The previous results stay
    /// on screen while it does: a half-typed `[` should not blank the list.
    compile_error: Option<String>,
    /// Outcome of the last "Plot all matching", shown until the next one.
    plot_all_message: Option<String>,
    /// Set by [`ChannelBrowser::request_focus`], cleared when the box takes
    /// focus on the next frame.
    focus_requested: bool,
    pub arrays_only: bool,
    pub unreadable_only: bool,
    pub master_only: bool,
    filtered_channels: Vec<FilteredChannel>,
    last_search: String,
    last_mode: MatchMode,
    last_arrays_only: bool,
    last_unreadable_only: bool,
    last_master_only: bool,
    last_file_ptr: usize,
}

impl ChannelBrowser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.search.clear();
        self.mode = MatchMode::default();
        self.compile_error = None;
        self.plot_all_message = None;
        self.arrays_only = false;
        self.unreadable_only = false;
        self.master_only = false;
        self.filtered_channels.clear();
        self.last_search.clear();
        self.last_arrays_only = false;
        self.last_unreadable_only = false;
        self.last_master_only = false;
        self.last_file_ptr = 0;
    }

    /// Asks for the search box to take focus on the next frame.
    ///
    /// Focus can only be given to a widget that exists, and the box does not
    /// exist until this panel draws — so the keyboard shortcut sets a flag
    /// here rather than trying to focus something that is not there yet.
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        // The file the rows below are from; a plotted channel is only this
        // row's channel when it is that channel *in this file*.
        active: FileSlot,
        plotted: &mut Vec<PlottedChannel>,
        selected: &mut Option<ChannelLoc>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            let search_box = ui.text_edit_singleline(&mut self.search);
            if std::mem::take(&mut self.focus_requested) {
                search_box.request_focus();
            }
            if ui.button("Clear").clicked() {
                self.search.clear();
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Match:");
            for (mode, label) in [
                (MatchMode::Substring, "Substring"),
                (MatchMode::Wildcard, "Wildcard"),
                (MatchMode::Regex, "Regex"),
            ] {
                ui.selectable_value(&mut self.mode, mode, label);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.arrays_only, "Arrays only");
            ui.checkbox(&mut self.unreadable_only, "Unreadable only");
            ui.checkbox(&mut self.master_only, "Master channels only");
        });
        if let Some(reason) = &self.compile_error {
            ui.colored_label(egui::Color32::from_rgb(200, 140, 40), reason);
        }

        ui.separator();

        let is_filtered = !self.search.trim().is_empty()
            || self.arrays_only
            || self.unreadable_only
            || self.master_only;

        let file_ptr = std::sync::Arc::as_ptr(&loaded.file) as usize;
        let filter_changed = file_ptr != self.last_file_ptr
            || self.search != self.last_search
            || self.mode != self.last_mode
            || self.arrays_only != self.last_arrays_only
            || self.unreadable_only != self.last_unreadable_only
            || self.master_only != self.last_master_only;

        if filter_changed {
            self.last_file_ptr = file_ptr;
            self.last_search = self.search.clone();
            self.last_mode = self.mode;
            self.last_arrays_only = self.arrays_only;
            self.last_unreadable_only = self.unreadable_only;
            self.last_master_only = self.master_only;

            // A pattern that does not compile leaves the previous results
            // alone and says why, rather than emptying the list under a
            // half-typed bracket.
            match compile(self.search.trim(), self.mode) {
                Ok(pattern) => {
                    self.compile_error = None;
                    self.filtered_channels = if is_filtered {
                        filter_channels(
                            &loaded.file,
                            &pattern,
                            !self.search.trim().is_empty(),
                            self.arrays_only,
                            self.unreadable_only,
                            self.master_only,
                        )
                    } else {
                        Vec::new()
                    };
                }
                Err(reason) => self.compile_error = Some(reason),
            }
        }

        let total_channels = loaded.file.statistics().channel_count;
        let count_text = if is_filtered {
            format!(
                "{} of {} channels",
                self.filtered_channels.len(),
                total_channels
            )
        } else {
            format!("{} of {} channels", total_channels, total_channels)
        };
        ui.label(egui::RichText::new(count_text).weak());

        // Plotting every match is what a pattern is usually typed for, but a
        // wildcard can match a thousand channels and a thousand lines is not
        // a view of anything — so it is capped and says what it skipped.
        if is_filtered && !self.filtered_channels.is_empty() {
            ui.horizontal(|ui| {
                if ui
                    .button(format!(
                        "Plot all {} matching",
                        self.filtered_channels.len()
                    ))
                    .clicked()
                {
                    let mut added = 0usize;
                    let mut skipped = 0usize;
                    for channel in &self.filtered_channels {
                        if plotted.iter().any(|p| p.is(active, channel.loc)) {
                            continue;
                        }
                        if added >= MAX_PLOT_ALL {
                            skipped += 1;
                            continue;
                        }
                        plotted.push(PlottedChannel::new(
                            active,
                            channel.loc,
                            channel.name.clone(),
                            plotted.len(),
                        ));
                        added += 1;
                    }
                    self.plot_all_message = Some(if skipped == 0 {
                        format!("plotted {added}")
                    } else {
                        format!("plotted {added}, left {skipped} unplotted")
                    });
                }
                if let Some(message) = &self.plot_all_message {
                    ui.weak(message);
                }
            });
        }

        if is_filtered && self.filtered_channels.is_empty() {
            ui.label("No channels match.");
            return;
        }

        if !is_filtered && loaded.all_rows.is_empty() {
            ui.label("This file has no channels.");
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;

        if is_filtered {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, row_height, self.filtered_channels.len(), |ui, range| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for ch in &self.filtered_channels[range] {
                        show_filtered_row(ui, ch, active, plotted, selected);
                    }
                });
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, row_height, loaded.all_rows.len(), |ui, range| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for row in &loaded.all_rows[range] {
                        show_tree_row(ui, row, active, plotted, selected);
                    }
                });
        }
    }
}

fn show_filtered_row(
    ui: &mut egui::Ui,
    ch: &FilteredChannel,
    active: FileSlot,
    plotted: &mut Vec<PlottedChannel>,
    selected: &mut Option<ChannelLoc>,
) {
    let plotted_index = plotted.iter().position(|p| p.is(active, ch.loc));
    ui.horizontal(|ui| {
        // The visibility checkbox and the color swatch only exist once the
        // channel is plotted; before that there is nothing to show or hide.
        if let Some(i) = plotted_index {
            let channel = &mut plotted[i];
            ui.checkbox(&mut channel.visible, "");
            ui.colored_label(channel.color, "\u{25cf}");
        }

        let mut label = if ch.unit.is_empty() {
            format!(
                "    {}  \u{2014} {}  \u{2014} {} samples",
                ch.name, ch.group_label, ch.sample_count
            )
        } else {
            format!(
                "    {}  [{}]  \u{2014} {}  \u{2014} {} samples",
                ch.name, ch.unit, ch.group_label, ch.sample_count
            )
        };
        if ch.unreadable.is_some() {
            label.push_str("  \u{26a0}");
        }

        let response = ui.selectable_label(plotted_index.is_some(), label);
        let response = match &ch.unreadable {
            Some(reason) => response.on_hover_text(reason),
            None => response,
        };
        if response.clicked() {
            match plotted_index {
                Some(i) => {
                    plotted.remove(i);
                }
                None => {
                    plotted.push(PlottedChannel::new(active, ch.loc, ch.name.clone(), plotted.len()));
                }
            }
            *selected = Some(ch.loc);
        }
    });
}

fn show_tree_row(
    ui: &mut egui::Ui,
    row: &Row,
    active: FileSlot,
    plotted: &mut Vec<PlottedChannel>,
    selected: &mut Option<ChannelLoc>,
) {
    match row {
        Row::DataGroupHeader { label } => {
            ui.strong(label);
        }
        Row::ChannelGroupHeader { label } => {
            ui.label(egui::RichText::new(label).italics().weak());
        }
        Row::Channel {
            loc,
            name,
            unit,
            sample_count,
            unreadable,
        } => {
            let plotted_index = plotted.iter().position(|p| p.is(active, *loc));
            ui.horizontal(|ui| {
                if let Some(i) = plotted_index {
                    let channel = &mut plotted[i];
                    ui.checkbox(&mut channel.visible, "");
                    ui.colored_label(channel.color, "\u{25cf}");
                }

                let mut label = if unit.is_empty() {
                    format!("    {name}  \u{2014} {sample_count} samples")
                } else {
                    format!("    {name}  [{unit}]  \u{2014} {sample_count} samples")
                };
                if unreadable.is_some() {
                    label.push_str("  \u{26a0}");
                }
                let response = ui.selectable_label(plotted_index.is_some(), label);
                let response = match unreadable {
                    Some(reason) => response.on_hover_text(reason),
                    None => response,
                };
                if response.clicked() {
                    match plotted_index {
                        Some(i) => {
                            plotted.remove(i);
                        }
                        None => {
                            plotted.push(PlottedChannel::new(active, *loc, name.clone(), plotted.len()));
                        }
                    }
                    *selected = Some(*loc);
                }
            });
        }
    }
}

/// Walks the file hierarchy directly to retain data and channel group indices.
/// Matches on channel name, unit, comment, and channel group acquisition name.
fn filter_channels(
    file: &Mf4File,
    pattern: &Pattern,
    has_query: bool,
    arrays_only: bool,
    unreadable_only: bool,
    master_only: bool,
) -> Vec<FilteredChannel> {
    let match_text = has_query;

    let mut results = Vec::new();

    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            let group_matches_text = match_text && matches(pattern, &cg.acquisition_name);

            let group_label = if cg.acquisition_name.is_empty() {
                format!("Group {dg_idx}/{cg_idx}")
            } else {
                format!("Group {dg_idx}/{cg_idx} \"{}\"", cg.acquisition_name)
            };

            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if arrays_only && !ch.is_array() {
                    continue;
                }
                if unreadable_only && ch.unreadable().is_none() {
                    continue;
                }
                if master_only && !ch.is_master() {
                    continue;
                }

                if match_text {
                    let matches = group_matches_text
                        || matches(pattern, &ch.name)
                        || matches(pattern, &ch.unit)
                        || matches(pattern, &ch.comment);

                    if !matches {
                        continue;
                    }
                }

                results.push(FilteredChannel {
                    loc: ChannelLoc {
                        data_group_index: dg_idx,
                        channel_group_index: cg_idx,
                        channel_index: ch_idx,
                    },
                    name: ch.name.clone(),
                    unit: ch.unit.clone(),
                    group_label: group_label.clone(),
                    sample_count: cg.sample_count,
                    unreadable: ch.unreadable().map(|r| r.to_string()),
                });
            }
        }
    }

    results
}
