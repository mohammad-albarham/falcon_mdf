//! The file-level facts: version, start time, size and statistics, plus the
//! warning a file that was never finalized has to carry.
//!
//! The chains that hang off the header — history, attachments, events, the
//! channel hierarchy — are not listed here any more: they are nodes in the
//! structure tree, and clicking one shows it in full through
//! [`crate::panels::details`]. What stays here is what is true of the file
//! itself.

use falcon_mdf::Mf4File;

use crate::model::LoadedFile;

/// Version, start time, comment, size and `statistics()`, under the notice a
/// file that was written but never finalized needs.
pub fn show_file_metadata(ui: &mut egui::Ui, loaded: &LoadedFile) {
    let file = &loaded.file;

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
