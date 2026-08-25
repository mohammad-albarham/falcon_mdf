//! The content area's Details view: whatever the left-hand trees selected,
//! shown in full.
//!
//! One panel rather than one per kind, because the selection is one enum and
//! every view answers the same question — "what is this thing?" — for a
//! different kind of thing. Views that can move the selection (a block's
//! links, a channel group's block address) do so by writing to the same
//! `Selection` the trees write to, so navigation works in both directions.

use std::sync::Arc;

use falcon_mdf::{Channel, Mf4File};

use crate::job::Job;
use crate::model::{ChannelLoc, ContentTab, FileSlot, LoadedFile, PlottedChannel, Selection};
use crate::panels::blocks::{human_bytes, BlockInspector};
use crate::panels::metadata;

/// The Details view, and the state only it owns.
#[derive(Default)]
pub struct DetailsPanel {
    inspector: BlockInspector,
    /// An attachment save running on a worker thread. Embedded attachments
    /// are decompressed in full, which on a large blob would freeze the
    /// frame loop.
    attachment_job: Option<Job>,
    /// Outcome of the last save, shown until the next one. A failed save
    /// must leave text somewhere, not a closed dialog and a silence.
    notice: Option<String>,
}

impl DetailsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        // A save started for the previous file belongs to it; its result
        // would otherwise arrive into the new file's notice line. The worker
        // still finishes and writes what it was asked to.
        *self = Self::default();
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        active: FileSlot,
        selection: &mut Selection,
        plotted: &mut Vec<PlottedChannel>,
        tab: &mut ContentTab,
    ) {
        if let Some(job) = &self.attachment_job {
            if let Some(message) = job.poll() {
                self.notice = Some(message);
                self.attachment_job = None;
            }
        }
        if let Some(notice) = self.notice.clone() {
            ui.horizontal(|ui| {
                ui.label(notice);
                if ui.small_button("dismiss").clicked() {
                    self.notice = None;
                }
            });
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match *selection {
                Selection::File => show_file(ui, loaded, selection),
                Selection::DataGroup(index) => show_data_group(ui, loaded, index, selection),
                Selection::ChannelGroup {
                    data_group_index,
                    channel_group_index,
                } => show_channel_group(
                    ui,
                    loaded,
                    data_group_index,
                    channel_group_index,
                    selection,
                    tab,
                ),
                Selection::Channel(loc) => {
                    show_channel(ui, loaded, active, loc, selection, plotted, tab)
                }
                Selection::Block(address) => self.inspector.show(ui, loaded, address, selection),
                Selection::Attachment(index) => {
                    self.show_attachment(ui, loaded, index);
                }
                Selection::Event(index) => show_event(ui, loaded, index),
                Selection::HistoryEntry(index) => show_history_entry(ui, loaded, index),
            });
    }

    fn show_attachment(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile, index: usize) {
        let Some(attachment) = loaded.file.attachments().get(index) else {
            ui.label("That attachment is no longer in the file.");
            return;
        };
        ui.heading(&attachment.file_name);
        egui::Grid::new("attachment_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Storage");
                ui.label(if attachment.is_embedded {
                    "embedded in this file"
                } else {
                    "external \u{2014} the bytes sit at the named path"
                });
                ui.end_row();

                ui.label("Size");
                ui.label(format!(
                    "{} bytes",
                    if attachment.is_embedded {
                        attachment.embedded_size()
                    } else {
                        attachment.original_size
                    }
                ));
                ui.end_row();

                if !attachment.file_path.is_empty() {
                    // The same field carries a path for an external
                    // attachment and a MIME type for an embedded one, so it
                    // is labelled by what it means here.
                    ui.label(if attachment.is_embedded {
                        "Content type"
                    } else {
                        "Path"
                    });
                    ui.label(&attachment.file_path);
                    ui.end_row();
                }
                if attachment.is_embedded && attachment.is_compressed {
                    ui.label("Stored");
                    ui.label("deflate-compressed; saving writes the decompressed bytes");
                    ui.end_row();
                }
                if !attachment.comment.is_empty() {
                    ui.label("Comment");
                    ui.label(&attachment.comment);
                    ui.end_row();
                }
            });

        if !attachment.is_embedded {
            ui.weak("Only embedded attachments can be saved from here; this one's bytes are not in the MF4.");
            return;
        }
        let busy = self.attachment_job.is_some();
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Saving\u{2026}");
            });
        }
        let file_name = attachment.file_name.clone();
        ui.add_enabled_ui(!busy, |ui| {
            if ui.button("Save\u{2026}").clicked() {
                self.start_attachment_save(ui, &loaded.file, index, &file_name);
            }
        });
    }

    /// Picks a path on the UI thread (the dialog has to run there), then
    /// reads and writes the attachment's bytes on a worker thread.
    fn start_attachment_save(
        &mut self,
        ui: &egui::Ui,
        file: &Arc<Mf4File>,
        index: usize,
        file_name: &str,
    ) {
        let Some(path) = rfd::FileDialog::new().set_file_name(file_name).save_file() else {
            return;
        };
        let file = Arc::clone(file);
        self.attachment_job = Some(Job::spawn(ui.ctx(), move || {
            let attachment = &file.attachments()[index];
            match file.attachment_data(attachment) {
                Ok(Some(bytes)) => match std::fs::write(&path, &bytes) {
                    Ok(()) => format!("saved {} bytes to {}", bytes.len(), path.display()),
                    Err(e) => format!("saving failed: {e}"),
                },
                Ok(None) => "the attachment carries no embedded data".to_string(),
                Err(e) => format!("reading the attachment failed: {e}"),
            }
        }));
    }
}

fn show_file(ui: &mut egui::Ui, loaded: &LoadedFile, selection: &mut Selection) {
    ui.heading("File");
    metadata::show_file_metadata(ui, loaded);

    let map = &loaded.blocks;
    ui.separator();
    ui.strong("Blocks");
    egui::Grid::new("file_block_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Blocks found");
            ui.label(map.blocks.len().to_string());
            ui.end_row();

            ui.label("Covered by blocks");
            ui.label(format!(
                "{} of {} bytes ({:.1}%)",
                map.covered_bytes,
                map.file_size,
                map.covered_bytes as f64 / map.file_size.max(1) as f64 * 100.0
            ));
            ui.end_row();

            // Padding is expected — MF4 aligns blocks to eight bytes — so the
            // two kinds of uncovered region are counted apart. A file with a
            // large hole was written differently from one with none, and the
            // difference should not hide inside a single number.
            let (padding, holes): (Vec<&falcon_mdf::Gap>, Vec<&falcon_mdf::Gap>) =
                map.gaps.iter().partition(|gap| gap.length < 8);
            ui.label("Alignment padding");
            ui.label(format!(
                "{} places, {} bytes",
                padding.len(),
                padding.iter().map(|g| g.length).sum::<u64>()
            ));
            ui.end_row();

            ui.label("Larger uncovered regions");
            ui.label(format!(
                "{} places, {} bytes",
                holes.len(),
                holes.iter().map(|g| g.length).sum::<u64>()
            ));
            ui.end_row();
        });

    // Outside the grid, not in a cell: a grid column is as wide as its widest
    // entry and does not wrap, so a sentence in one runs off the panel.
    // On an unfinalized file the largest uncovered region is not a hole at
    // all — it is the records themselves, after a data block whose length the
    // writer never went back to fill in — and saying so stops "50% covered"
    // reading as damage.
    if map.unfinalized && map.gaps.iter().any(|gap| gap.length >= 8) {
        ui.weak(
            "This file was never finalized, so its last data block declares no length \u{2014} the records after it are what those uncovered bytes are.",
        );
    }

    egui::CollapsingHeader::new("Composition")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("file_block_types")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    for (block_type, count) in map.type_counts() {
                        ui.monospace(block_type);
                        ui.label(count.to_string());
                        ui.end_row();
                    }
                });
        });

    if !map.warnings.is_empty() {
        egui::CollapsingHeader::new(format!("Walk warnings ({})", map.warnings.len()))
            .default_open(true)
            .show(ui, |ui| {
                for warning in &map.warnings {
                    ui.colored_label(egui::Color32::from_rgb(200, 140, 40), warning);
                }
            });
    }

    let properties: Vec<(&str, &str)> = loaded.file.metadata().properties().collect();
    if !properties.is_empty() {
        egui::CollapsingHeader::new(format!("Header properties ({})", properties.len())).show(
            ui,
            |ui| {
                egui::Grid::new("file_metadata_properties")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (name, value) in properties {
                            ui.label(name);
                            ui.label(value);
                            ui.end_row();
                        }
                    });
            },
        );
    }

    ui.separator();
    if ui.button("Show the header block").clicked() {
        *selection = Selection::Block(64);
    }
}

fn show_data_group(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    index: usize,
    selection: &mut Selection,
) {
    let Some(dg) = loaded.file.data_groups().get(index) else {
        ui.label("That data group is no longer in the file.");
        return;
    };
    ui.heading(format!("Data group {index}"));
    egui::Grid::new("dg_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Channel groups");
            ui.label(dg.channel_groups.len().to_string());
            ui.end_row();

            ui.label("Record layout");
            ui.label(if dg.is_unsorted() {
                format!(
                    "unsorted \u{2014} records of several groups interleaved, {}-byte record IDs",
                    dg.rec_id_size()
                )
            } else {
                "sorted \u{2014} one group's records, one stride".to_string()
            });
            ui.end_row();

            ui.label("Channels");
            ui.label(dg.channels().count().to_string());
            ui.end_row();

            if !dg.comment.is_empty() {
                ui.label("Comment");
                ui.label(&dg.comment);
                ui.end_row();
            }
        });

    ui.separator();
    block_buttons(
        ui,
        loaded,
        selection,
        &[
            ("Show the ##DG block", dg.block_offset()),
            ("Show the data block", dg.data_block_offset()),
        ],
    );
}

fn show_channel_group(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    dg_index: usize,
    cg_index: usize,
    selection: &mut Selection,
    tab: &mut ContentTab,
) {
    let Some(cg) = loaded
        .file
        .data_groups()
        .get(dg_index)
        .and_then(|dg| dg.channel_groups.get(cg_index))
    else {
        ui.label("That channel group is no longer in the file.");
        return;
    };
    let dg = &loaded.file.data_groups()[dg_index];

    ui.heading(if cg.acquisition_name.is_empty() {
        format!("Channel group {cg_index}")
    } else {
        format!("Channel group {cg_index} \u{2014} {}", cg.acquisition_name)
    });

    egui::Grid::new("cg_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Samples");
            ui.label(cg.sample_count.to_string());
            ui.end_row();

            ui.label("Channels");
            ui.label(cg.channels.len().to_string());
            ui.end_row();

            ui.label("Record");
            ui.label(format!(
                "{} bytes of data + {} invalidation bytes ({} per record with the ID)",
                cg.data_bytes_len(),
                cg.inval_bytes_len(),
                cg.record_size(dg.rec_id_size())
            ));
            ui.end_row();

            ui.label("Total data");
            ui.label(human_bytes(
                cg.sample_count * cg.payload_size().max(1) as u64,
            ));
            ui.end_row();

            if dg.is_unsorted() {
                ui.label("Record ID");
                ui.label(cg.record_id().to_string());
                ui.end_row();
            }

            ui.label("Kind");
            ui.label(if cg.is_bus_event() {
                "logged bus traffic"
            } else if cg.is_vlsd() {
                "variable-length signal data"
            } else {
                "measurement records"
            });
            ui.end_row();

            if let Some(master) = cg.master_channel() {
                ui.label("Master");
                ui.label(format!("{} [{}]", master.name, master.unit));
                ui.end_row();
            } else {
                ui.label("Master");
                ui.label("none \u{2014} samples are indexed, not timed");
                ui.end_row();
            }

            if let Some(source) = &cg.source {
                ui.label("Acquisition source");
                ui.label(source_line(source));
                ui.end_row();
            }

            if !cg.comment.is_empty() {
                ui.label("Comment");
                ui.label(&cg.comment);
                ui.end_row();
            }
        });

    if !cg.sample_reductions().is_empty() {
        egui::CollapsingHeader::new(format!(
            "Sample reduction levels ({})",
            cg.sample_reductions().len()
        ))
        .show(ui, |ui| {
            egui::Grid::new("cg_sr_grid")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    for reduction in cg.sample_reductions() {
                        ui.label(format!("{:?}", reduction.sync_type));
                        ui.label(format!("{} cycles", reduction.cycle_count));
                        ui.label(format!("every {}", reduction.interval));
                        ui.end_row();
                    }
                });
        });
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Show the samples").clicked() {
            *tab = ContentTab::Table;
        }
        if cg.is_bus_event() && ui.button("Show the frames").clicked() {
            *tab = ContentTab::Bus;
        }
    });
    block_buttons(
        ui,
        loaded,
        selection,
        &[("Show the ##CG block", cg.block_offset())],
    );
}

fn show_channel(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    active: FileSlot,
    loc: ChannelLoc,
    selection: &mut Selection,
    plotted: &mut Vec<PlottedChannel>,
    tab: &mut ContentTab,
) {
    let Some(ch) = loaded
        .file
        .data_groups()
        .get(loc.data_group_index)
        .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
        .and_then(|cg| cg.channels.get(loc.channel_index))
    else {
        ui.label("That channel is no longer in the file.");
        return;
    };

    ui.heading(&ch.name);
    if let Some(reason) = ch.unreadable() {
        ui.colored_label(
            egui::Color32::from_rgb(200, 140, 40),
            format!("This channel cannot be decoded: {reason}"),
        );
        ui.weak(reason.detail());
    }

    egui::Grid::new("channel_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Unit");
            ui.label(if ch.unit.is_empty() {
                "\u{2014}"
            } else {
                &ch.unit
            });
            ui.end_row();

            ui.label("Location");
            ui.label(format!(
                "data group {}, channel group {}, channel {}",
                loc.data_group_index, loc.channel_group_index, loc.channel_index
            ));
            ui.end_row();

            ui.label("Samples");
            ui.label(ch.sample_count.to_string());
            ui.end_row();

            ui.label("Channel type");
            ui.label(format!("{:?}", ch.channel_type));
            ui.end_row();

            if ch.is_master() {
                ui.label("Synchronised by");
                ui.label(format!("{:?}", ch.sync_type));
                ui.end_row();
            }

            ui.label("Data type");
            ui.label(format!(
                "{:?} \u{2014} {} bits at byte {}, bit {} ({}-endian)",
                ch.data_type,
                ch.bit_count,
                ch.byte_offset,
                ch.bit_offset,
                if ch.is_little_endian() {
                    "little"
                } else {
                    "big"
                }
            ));
            ui.end_row();

            ui.label("Conversion");
            ui.label(conversion_line(ch));
            ui.end_row();

            if let Some(shape) = ch.array_shape() {
                ui.label("Array shape");
                ui.label(format!(
                    "[{}] \u{2014} {} elements per sample",
                    shape
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(" \u{00d7} "),
                    shape.iter().product::<u64>()
                ));
                ui.end_row();
            }

            if let (Some(min), Some(max)) = (ch.min_value, ch.max_value) {
                ui.label("Declared range");
                ui.label(format!("{min} \u{2026} {max}"));
                ui.end_row();
            }

            ui.label("Validity");
            ui.label(if ch.all_invalid {
                "the file declares every sample invalid".to_string()
            } else if ch.invalidation_bit {
                format!("per-sample, invalidation bit {}", ch.inval_bit_pos)
            } else {
                "every sample is valid".to_string()
            });
            ui.end_row();

            if let Some(source) = &ch.source {
                ui.label("Source");
                ui.label(source_line(source));
                ui.end_row();
            }

            if !ch.comment.is_empty() {
                ui.label("Comment");
                ui.label(&ch.comment);
                ui.end_row();
            }
        });

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        let plotted_index = plotted.iter().position(|p| p.is(active, loc));
        let label = if plotted_index.is_some() {
            "Remove from the plot"
        } else {
            "Plot this channel"
        };
        if ui.button(label).clicked() {
            match plotted_index {
                Some(i) => {
                    plotted.remove(i);
                }
                None => {
                    plotted.push(PlottedChannel::new(active, loc, ch.name.clone(), plotted.len()));
                    *tab = ContentTab::Plot;
                }
            }
        }
        if ui.button("Statistics").clicked() {
            *tab = ContentTab::Statistics;
        }
        if ui.button("Samples").clicked() {
            *tab = ContentTab::Table;
        }
    });
    block_buttons(
        ui,
        loaded,
        selection,
        &[("Show the ##CN block", ch.block_offset())],
    );
}

fn show_event(ui: &mut egui::Ui, loaded: &LoadedFile, index: usize) {
    let Some(event) = loaded.file.events().get(index) else {
        ui.label("That event is no longer in the file.");
        return;
    };
    ui.heading(if event.name.is_empty() {
        format!("{:?} event", event.event_type)
    } else {
        event.name.clone()
    });
    egui::Grid::new("event_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Type");
            ui.label(format!("{:?}", event.event_type));
            ui.end_row();

            ui.label("Synchronised by");
            ui.label(format!("{:?}", event.sync_type));
            ui.end_row();

            ui.label("Position");
            ui.label(format!("{:.9}", event.position()));
            ui.end_row();

            if !event.comment.is_empty() {
                ui.label("Comment");
                ui.label(&event.comment);
                ui.end_row();
            }
        });
}

fn show_history_entry(ui: &mut egui::Ui, loaded: &LoadedFile, index: usize) {
    let Some(entry) = loaded.file.file_history().get(index) else {
        ui.label("That history entry is no longer in the file.");
        return;
    };
    ui.heading(entry.time.to_iso8601());
    egui::Grid::new("history_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in [
                ("Tool vendor", entry.tool_vendor()),
                ("Tool", entry.tool_id()),
                ("Version", entry.tool_version()),
            ] {
                if let Some(value) = value {
                    ui.label(label);
                    ui.label(value);
                    ui.end_row();
                }
            }
            if !entry.comment.is_empty() {
                ui.label("Comment");
                ui.label(&entry.comment);
                ui.end_row();
            }
        });
}

/// Buttons that move the selection to a block, skipping any address of zero
/// — the format's way of saying the link is not used, which is not a block
/// the user could look at.
fn block_buttons(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    selection: &mut Selection,
    entries: &[(&str, u64)],
) {
    ui.horizontal_wrapped(|ui| {
        for (label, address) in entries {
            if *address == 0 {
                continue;
            }
            let Some(block) = loaded.blocks.block_at(*address) else {
                continue;
            };
            if ui
                .button(format!("{label} ({} at {address:#x})", block.block_type))
                .clicked()
            {
                *selection = Selection::Block(*address);
            }
        }
    });
}

fn source_line(source: &falcon_mdf::blocks::SourceInfo) -> String {
    let mut line = if source.name.is_empty() {
        "unnamed".to_string()
    } else {
        source.name.clone()
    };
    if let Some(source_type) = source.source_type {
        line.push_str(&format!(" \u{00b7} {source_type:?}"));
    }
    if let Some(bus_type) = source.bus_type {
        line.push_str(&format!(" on {bus_type:?}"));
    }
    if !source.path.is_empty() {
        line.push_str(&format!(" \u{00b7} {}", source.path));
    }
    if source.simulated {
        line.push_str(" \u{00b7} simulated");
    }
    line
}

/// The conversion in one line, naming what it does rather than printing the
/// enum: "linear, y = 2x + 1" says more than `Linear { .. }`.
fn conversion_line(channel: &Channel) -> String {
    use falcon_mdf::blocks::Conversion;
    match &channel.conversion {
        Conversion::None => "none \u{2014} raw values are the physical values".to_string(),
        Conversion::Linear { offset, factor } => {
            format!("linear \u{2014} y = {factor}x + {offset}")
        }
        Conversion::Rational { coefficients: c } => format!(
            "rational \u{2014} y = ({}x\u{00b2} + {}x + {}) / ({}x\u{00b2} + {}x + {})",
            c[0], c[1], c[2], c[3], c[4], c[5]
        ),
        Conversion::Algebraic { formula, .. } => format!("algebraic \u{2014} {formula}"),
        Conversion::TableInterpolated { keys, .. } => {
            format!("value table, interpolating \u{2014} {} entries", keys.len())
        }
        Conversion::TableLookup { keys, .. } => {
            format!("value table \u{2014} {} entries", keys.len())
        }
        Conversion::RangeTable { lower, .. } => {
            format!("value-range table \u{2014} {} ranges", lower.len())
        }
        Conversion::ValueToText { keys, .. } => {
            format!("value-to-text table \u{2014} {} entries", keys.len())
        }
        Conversion::RangeToText { lower, .. } => {
            format!("value-range-to-text table \u{2014} {} ranges", lower.len())
        }
        Conversion::TextToValue { keys, .. } => {
            format!("text-to-value table \u{2014} {} entries", keys.len())
        }
        Conversion::TextToText { keys, .. } => {
            format!("text-to-text table \u{2014} {} entries", keys.len())
        }
        Conversion::Bitfield { masks, .. } => {
            format!("bitfield text table \u{2014} {} fields", masks.len())
        }
        // Named rather than hidden: a channel with a conversion this build
        // cannot evaluate does not decode, and the reason belongs on screen.
        Conversion::Unsupported { kind, reason } => {
            format!("unsupported ({kind:?}) \u{2014} {reason}")
        }
        // `Conversion` is `#[non_exhaustive]`: a conversion added to the
        // library after this panel was written still gets named rather than
        // silently rendering as nothing.
        other => format!("{other:?}"),
    }
}
