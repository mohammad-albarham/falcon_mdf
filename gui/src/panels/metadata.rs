//! File-level metadata panel and the per-channel detail view.
//!
//! G4 extends the file-level facts G1 shows with the rest of the file: the
//! change history, attachments (with their embedded bytes savable), events
//! and the channel hierarchy. Sections collapse so a file rich in them does
//! not bury the statistics; each names its count in its header so an empty
//! one still says "there is none of this here" rather than nothing at all.

use std::sync::Arc;

use falcon_mdf::blocks::ChElement;
use falcon_mdf::Mf4File;

use crate::job::Job;
use crate::model::{ChannelLoc, LoadedFile, PlottedChannel};

/// Version, start time, comment, size and `statistics()` — the file-level
/// facts G1 asks for — plus G4's history, attachments, events and hierarchy.
///
/// `notice` carries the outcome of the last save/export-style action until
/// the next one; the panel renders it at the top so a failed save is never
/// silent. `attachment_job` is the save running on a worker thread, if one
/// is in flight; the panel both starts it and greys out its buttons while it
/// lasts, and `app.rs` collects its message into `notice`.
pub fn show_file_metadata(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    notice: &mut Option<String>,
    plotted: &mut Vec<PlottedChannel>,
    attachment_job: &mut Option<Job>,
) {
    let file = &loaded.file;

    if let Some(notice) = notice.as_deref() {
        ui.label(notice);
    }

    show_unfinalized_notice(ui, file);

    egui::Grid::new("file_metadata_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("File");
            ui.label(loaded.path.display().to_string());
            ui.end_row();

            ui.label("Version");
            ui.label(file.version().to_string());
            ui.end_row();

            ui.label("Start time");
            ui.label(file.start_time().to_iso8601());
            ui.end_row();

            ui.label("UTC offset");
            ui.label(format!("{} min", file.start_time().total_utc_offset_min()));
            ui.end_row();

            ui.label("File size");
            ui.label(format!(
                "{} bytes ({:.2} MB)",
                file.file_size(),
                file.file_size() as f64 / (1024.0 * 1024.0)
            ));
            ui.end_row();

            if !file.comment().is_empty() {
                ui.label("Comment");
                ui.label(file.comment());
                ui.end_row();
            }
        });

    ui.separator();

    let stats = file.statistics();
    egui::Grid::new("file_statistics_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Data groups");
            ui.label(stats.data_group_count.to_string());
            ui.end_row();

            ui.label("Channel groups");
            ui.label(stats.channel_group_count.to_string());
            ui.end_row();

            ui.label("Channels");
            ui.label(stats.channel_count.to_string());
            ui.end_row();

            ui.label("Total samples");
            ui.label(stats.total_sample_count.to_string());
            ui.end_row();
        });

    ui.separator();
    show_history(ui, file);
    show_attachments(ui, &loaded.file, attachment_job);
    show_events(ui, file);
    show_hierarchy(ui, file, plotted);
}

/// A file whose writer stopped before finalizing still carries its data but
/// not all of its bookkeeping. The reader compensates two of the flags on
/// its own; saying so here keeps a surprising sample count from reading
/// like a decode bug, and names the flags it does not touch.
fn show_unfinalized_notice(ui: &mut egui::Ui, file: &Mf4File) {
    let Some(flags) = file.unfinalized() else {
        return;
    };
    let warning = egui::Color32::from_rgb(200, 140, 40);
    ui.colored_label(
        warning,
        "This file was not finalized when written \u{2014} the recording ended before the writer updated its bookkeeping.",
    );
    ui.colored_label(
        warning,
        "Sample counts are taken from the data itself, and a zero-length last data block is read to the end of the file.",
    );
    let mut stale: Vec<&str> = Vec::new();
    if flags.update_sr_counters {
        stale.push("sample-reduction counters");
    }
    if flags.update_last_rd_length {
        stale.push("the last reduction block's length");
    }
    if flags.update_last_dl {
        stale.push("the last data list");
    }
    if flags.update_vlsd_bytes {
        stale.push("variable-length byte counts");
    }
    if flags.update_vlsd_offsets {
        stale.push("variable-length offsets (such payloads may not resolve)");
    }
    if !stale.is_empty() {
        ui.colored_label(
            warning,
            format!("The writer still declares stale: {}.", stale.join(", ")),
        );
    }
    ui.separator();
}

/// The file's change history, in the order the chain is walked from the
/// header — creation first, modifications after.
fn show_history(ui: &mut egui::Ui, file: &Mf4File) {
    let history = file.file_history();
    egui::CollapsingHeader::new(format!("File history ({})", history.len()))
        .default_open(false)
        .show(ui, |ui| {
            if history.is_empty() {
                ui.label("(none)");
                return;
            }
            for entry in history {
                ui.vertical(|ui| {
                    ui.strong(entry.time.to_iso8601());
                    let tool = [entry.tool_vendor(), entry.tool_id(), entry.tool_version()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !tool.is_empty() {
                        ui.label(tool);
                    }
                    if !entry.comment.is_empty() {
                        ui.label(&entry.comment);
                    }
                });
            }
        });
}

/// Attachments, with a save action for the embedded ones. External
/// attachments name a path the writer knew; only embedded bytes can be
/// handed back, so only they get the button.
fn show_attachments(ui: &mut egui::Ui, file: &Arc<Mf4File>, attachment_job: &mut Option<Job>) {
    let attachments = file.attachments();
    let save_busy = attachment_job.is_some();
    egui::CollapsingHeader::new(format!("Attachments ({})", attachments.len()))
        .default_open(false)
        .show(ui, |ui| {
            if attachments.is_empty() {
                ui.label("(none)");
                return;
            }
            if save_busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Saving attachment\u{2026}");
                });
            }
            for (index, attachment) in attachments.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&attachment.file_name);
                        if !attachment.is_embedded {
                            ui.label("(external)");
                        }
                    });
                    ui.label(format!(
                        "{} bytes",
                        if attachment.is_embedded {
                            attachment.embedded_size()
                        } else {
                            attachment.original_size
                        }
                    ));
                    if !attachment.comment.is_empty() {
                        ui.label(&attachment.comment);
                    }
                    if attachment.is_embedded {
                        let file_name = attachment.file_name.clone();
                        ui.add_enabled_ui(!save_busy, |ui| {
                            if ui.button("Save\u{2026}").clicked() {
                                start_attachment_save(ui, file, index, &file_name, attachment_job);
                            }
                        });
                    }
                });
            }
        });
}

/// Picks a path on the UI thread (the dialog has to run there), then reads
/// and writes the attachment's bytes on a worker thread — an embedded
/// attachment is decompressed in full, which for a large blob would freeze
/// the frame loop. The result message lands in the app's notice line when
/// the worker finishes.
fn start_attachment_save(
    ui: &egui::Ui,
    file: &Arc<Mf4File>,
    index: usize,
    file_name: &str,
    attachment_job: &mut Option<Job>,
) {
    let Some(path) = rfd::FileDialog::new().set_file_name(file_name).save_file() else {
        return;
    };
    let file = Arc::clone(file);
    *attachment_job = Some(Job::spawn(ui.ctx(), move || {
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

/// Events, each with its position in its own synchronisation domain. Only
/// time-synchronised events become plot markers (see the plot panel); the
/// others have no place on a time axis but are still listed here.
fn show_events(ui: &mut egui::Ui, file: &Mf4File) {
    let events = file.events();
    egui::CollapsingHeader::new(format!("Events ({})", events.len()))
        .default_open(false)
        .show(ui, |ui| {
            if events.is_empty() {
                ui.label("(none)");
                return;
            }
            for event in events {
                ui.vertical(|ui| {
                    let label = if event.name.is_empty() {
                        format!("{:?}", event.event_type)
                    } else {
                        event.name.clone()
                    };
                    ui.strong(label);
                    ui.label(format!(
                        "{:?} \u{00b7} {:?} \u{00b7} position {:.6}",
                        event.event_type,
                        event.sync_type,
                        event.position()
                    ));
                    if !event.comment.is_empty() {
                        ui.label(&event.comment);
                    }
                });
            }
        });
}

/// The channel hierarchy as a tree: node names indented by depth, their
/// channels resolved through `Mf4File::channel_at`. The accessor recurses
/// into children, so the tree draws every level the file carries. A channel
/// row plots on click, same as the channel list — the tree is a second way
/// to find a signal, not just a picture of the file's organisation.
fn show_hierarchy(ui: &mut egui::Ui, file: &Mf4File, plotted: &mut Vec<PlottedChannel>) {
    let nodes = file.channel_hierarchy();
    egui::CollapsingHeader::new(format!("Channel hierarchy ({})", nodes.len()))
        .default_open(false)
        .show(ui, |ui| {
            if nodes.is_empty() {
                ui.label("(none)");
                return;
            }
            for node in nodes {
                show_hierarchy_node(ui, file, node, 0, plotted);
            }
        });
}

fn show_hierarchy_node(
    ui: &mut egui::Ui,
    file: &Mf4File,
    node: &falcon_mdf::ChannelHierarchyNode,
    depth: usize,
    plotted: &mut Vec<PlottedChannel>,
) {
    let indent = "  ".repeat(depth);
    ui.strong(format!("{indent}{}", node.name));
    for element in &node.elements {
        match resolve_element(file, element) {
            Some((name, loc)) => {
                let is_plotted = plotted.iter().any(|p| p.loc == loc);
                if ui
                    .selectable_label(is_plotted, format!("{indent}  {name}"))
                    .clicked()
                    && !is_plotted
                {
                    plotted.push(PlottedChannel::new(loc, name, plotted.len()));
                }
            }
            None => {
                ui.label(format!("{indent}  (channel not found)"));
            }
        }
    }
    for child in &node.children {
        show_hierarchy_node(ui, file, child, depth + 1, plotted);
    }
}

fn resolve_element(file: &Mf4File, element: &ChElement) -> Option<(String, ChannelLoc)> {
    file.channel_at(element).map(|c| {
        (
            c.name.clone(),
            ChannelLoc {
                data_group_index: c.data_group_index,
                channel_group_index: c.channel_group_index,
                channel_index: c.index,
            },
        )
    })
}

/// Detail for the selected channel. This is the seam G2 extends: it already
/// knows which channel is selected, so a plot panel slots in beside (or
/// instead of) this view without any change to how selection is tracked.
pub fn show_channel_detail(ui: &mut egui::Ui, loaded: &LoadedFile, loc: ChannelLoc) {
    let dg = &loaded.file.data_groups()[loc.data_group_index];
    let cg = &dg.channel_groups[loc.channel_group_index];
    let ch = &cg.channels[loc.channel_index];

    egui::Grid::new("channel_detail_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Name");
            ui.label(&ch.name);
            ui.end_row();

            ui.label("Unit");
            ui.label(if ch.unit.is_empty() {
                "\u{2014}"
            } else {
                &ch.unit
            });
            ui.end_row();

            ui.label("Data group / group");
            ui.label(format!(
                "{} / {}",
                loc.data_group_index, loc.channel_group_index
            ));
            ui.end_row();

            ui.label("Sample count");
            ui.label(cg.sample_count.to_string());
            ui.end_row();

            ui.label("Channel type");
            ui.label(format!("{:?}", ch.channel_type));
            ui.end_row();

            ui.label("Data type");
            ui.label(format!("{:?}", ch.data_type));
            ui.end_row();

            if let Some(source) = &ch.source {
                if !source.name.is_empty() {
                    ui.label("Source");
                    ui.label(&source.name);
                    ui.end_row();
                }
                if !source.path.is_empty() {
                    ui.label("Source path");
                    ui.label(&source.path);
                    ui.end_row();
                }
                if let Some(source_type) = source.source_type {
                    ui.label("Source type");
                    ui.label(format!("{source_type:?}"));
                    ui.end_row();
                }
                if let Some(bus_type) = source.bus_type {
                    ui.label("Bus");
                    ui.label(format!("{bus_type:?}"));
                    ui.end_row();
                }
                if source.simulated {
                    ui.label("Simulated");
                    ui.label("yes");
                    ui.end_row();
                }
            }

            if !ch.comment.is_empty() {
                ui.label("Comment");
                ui.label(&ch.comment);
                ui.end_row();
            }
        });
}
