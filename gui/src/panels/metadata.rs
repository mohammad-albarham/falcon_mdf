//! File-level metadata panel and the per-channel detail view.
//!
//! G4 extends the file-level facts G1 shows with the rest of the file: the
//! change history, attachments (with their embedded bytes savable), events
//! and the channel hierarchy. Sections collapse so a file rich in them does
//! not bury the statistics; each names its count in its header so an empty
//! one still says "there is none of this here" rather than nothing at all.

use falcon_mdf::blocks::ChElement;
use falcon_mdf::{Attachment, Mf4File};

use crate::model::{ChannelLoc, LoadedFile};

/// Version, start time, comment, size and `statistics()` — the file-level
/// facts G1 asks for — plus G4's history, attachments, events and hierarchy.
///
/// `notice` carries the outcome of the last save/export-style action until
/// the next one; the panel renders it at the top so a failed save is never
/// silent.
pub fn show_file_metadata(ui: &mut egui::Ui, loaded: &LoadedFile, notice: &mut Option<String>) {
    let file = &loaded.file;

    if let Some(notice) = notice.as_deref() {
        ui.label(notice);
    }

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
    show_attachments(ui, file, notice);
    show_events(ui, file);
    show_hierarchy(ui, file);
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
fn show_attachments(ui: &mut egui::Ui, file: &Mf4File, notice: &mut Option<String>) {
    let attachments = file.attachments();
    egui::CollapsingHeader::new(format!("Attachments ({})", attachments.len()))
        .default_open(false)
        .show(ui, |ui| {
            if attachments.is_empty() {
                ui.label("(none)");
                return;
            }
            for attachment in attachments {
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
                    if attachment.is_embedded && ui.button("Save\u{2026}").clicked() {
                        *notice = save_attachment(file, attachment);
                    }
                });
            }
        });
}

/// Writes an embedded attachment's bytes to a path the user picks, and
/// returns the line the panel shows about how it went.
fn save_attachment(file: &Mf4File, attachment: &Attachment) -> Option<String> {
    let path = rfd::FileDialog::new()
        .set_file_name(&attachment.file_name)
        .save_file()?;
    match file.attachment_data(attachment) {
        Ok(Some(bytes)) => match std::fs::write(&path, &bytes) {
            Ok(()) => Some(format!("saved {} bytes to {}", bytes.len(), path.display())),
            Err(e) => Some(format!("saving failed: {e}")),
        },
        Ok(None) => Some("the attachment carries no embedded data".to_string()),
        Err(e) => Some(format!("reading the attachment failed: {e}")),
    }
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

/// The channel hierarchy as far as this build reaches: node names, their
/// channels resolved through `Mf4File::channel_at`, and an honest marker
/// where a node has children the accessor cannot descend into.
fn show_hierarchy(ui: &mut egui::Ui, file: &Mf4File) {
    let nodes = file.channel_hierarchy();
    egui::CollapsingHeader::new(format!("Channel hierarchy ({})", nodes.len()))
        .default_open(false)
        .show(ui, |ui| {
            if nodes.is_empty() {
                ui.label("(none)");
                return;
            }
            for node in nodes {
                ui.vertical(|ui| {
                    ui.strong(&node.name);
                    for element in &node.elements {
                        match resolve_element(file, element) {
                            Some(name) => ui.label(format!("  {name}")),
                            None => ui.label("  (channel not found)"),
                        };
                    }
                    if node.has_children {
                        ui.label("  \u{2026} has children this build cannot reach");
                    }
                });
            }
        });
}

fn resolve_element(file: &Mf4File, element: &ChElement) -> Option<String> {
    file.channel_at(element).map(|c| c.name.clone())
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
