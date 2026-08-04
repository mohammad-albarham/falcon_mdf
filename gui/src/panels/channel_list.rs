//! The searchable, grouped channel list.
//!
//! Files can carry thousands of channels, so rows are drawn through
//! `ScrollArea::show_rows` rather than one widget per channel — only the
//! rows actually on screen are laid out.
//!
//! Clicking a channel toggles it in the plotted set (G3): a plotted row
//! shows the channel's color and a visibility checkbox, and clicking again
//! removes it. Channels the library already knows it cannot decode carry a
//! warning marker with the reason — hiding them would look like data loss,
//! and showing an empty plot later would be worse.

use falcon_mdf::Mf4File;

use crate::model::{ChannelLoc, LoadedFile, PlottedChannel, Row};

/// Search state and the (cached) filtered row list.
///
/// Filtering rebuilds `filtered_rows` only when the query text changes, not
/// every frame — egui redraws continuously while the window is visible, and
/// `find_channels` is a lookup per surviving name, not free.
#[derive(Default)]
pub struct ChannelBrowser {
    pub search: String,
    filtered_rows: Vec<Row>,
    last_search: String,
}

impl ChannelBrowser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.search.clear();
        self.filtered_rows.clear();
        self.last_search.clear();
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        plotted: &mut Vec<PlottedChannel>,
        selected: &mut Option<ChannelLoc>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.search);
            if ui.button("Clear").clicked() {
                self.search.clear();
            }
        });
        ui.separator();

        if self.search != self.last_search {
            self.last_search = self.search.clone();
            self.filtered_rows = if self.search.trim().is_empty() {
                Vec::new()
            } else {
                filter_rows(&loaded.file, &self.search)
            };
        }

        let query_active = !self.search.trim().is_empty();
        let rows: &[Row] = if query_active {
            &self.filtered_rows
        } else {
            &loaded.all_rows
        };

        if rows.is_empty() {
            ui.label(if query_active {
                "No channels match."
            } else {
                "This file has no channels."
            });
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for row in &rows[range] {
                    show_row(ui, row, plotted, selected);
                }
            });
    }
}

fn show_row(
    ui: &mut egui::Ui,
    row: &Row,
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
            let plotted_index = plotted.iter().position(|p| p.loc == *loc);
            ui.horizontal(|ui| {
                // The visibility checkbox and the color swatch only exist
                // once the channel is plotted; before that there is nothing
                // to show or hide.
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
                            plotted.push(PlottedChannel::new(*loc, name.clone(), plotted.len()))
                        }
                    }
                    // The detail pane follows the last channel the user
                    // touched, whether the click plotted or un-plotted it.
                    *selected = Some(*loc);
                }
            });
        }
    }
}

/// Filters using the library's own name index rather than a hand-rolled one:
/// `channel_names()` gives the universe of unique names, substring-matched
/// here since the library only offers exact lookup; `find_channels` then
/// resolves each surviving name back to every location it appears at.
fn filter_rows(file: &Mf4File, query: &str) -> Vec<Row> {
    let query_lower = query.to_lowercase();
    // `channel_names()` is documented to return names sorted lexicographically,
    // and filtering preserves that order, so no re-sort is needed here.
    let names: Vec<&str> = file
        .channel_names()
        .into_iter()
        .filter(|name| name.to_lowercase().contains(&query_lower))
        .collect();

    let mut rows = Vec::new();
    for name in names {
        for ch in file.find_channels(name) {
            let sample_count = file.data_groups()[ch.data_group_index].channel_groups
                [ch.channel_group_index]
                .sample_count;
            rows.push(Row::Channel {
                loc: ChannelLoc {
                    data_group_index: ch.data_group_index,
                    channel_group_index: ch.channel_group_index,
                    channel_index: ch.index,
                },
                name: ch.name.clone(),
                unit: ch.unit.clone(),
                sample_count,
                unreadable: ch.unreadable().map(|r| r.to_string()),
            });
        }
    }
    rows
}
