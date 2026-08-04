//! File-level metadata panel and the per-channel detail view.

use crate::model::{ChannelLoc, LoadedFile};

/// Version, start time, comment, size and `statistics()` — the file-level
/// facts G1 asks for.
pub fn show_file_metadata(ui: &mut egui::Ui, loaded: &LoadedFile) {
    let file = &loaded.file;

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

            if !ch.comment.is_empty() {
                ui.label("Comment");
                ui.label(&ch.comment);
                ui.end_row();
            }
        });
}
