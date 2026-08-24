//! The block explorer: every block in the file from the first byte to the
//! last, and what any one of them holds.
//!
//! The list on the left is the file as it sits on disk — the identification
//! block, then every block the walk reached, in address order, with the
//! regions no block covers shown between them. The detail view on the right
//! is one block: its header, its links (each a link the user can follow),
//! who points at it, and its raw bytes.

use std::fmt::Write as _;
use std::path::Path;

use falcon_mdf::blocks::BLOCK_HEADER_SIZE;
use falcon_mdf::inspect::{BlockInfo, Gap};

use crate::job::Job;
use crate::model::{LoadedFile, Selection};
use crate::panels::hexdump::HexView;

/// How many bytes of a block the hex view reads. A data block can be
/// hundreds of megabytes; reading all of it to show the first screen would
/// stall the frame loop for no gain.
const HEX_LIMIT: u64 = 64 * 1024;

/// A row of the block list: a block, or the bytes between two blocks.
enum Row {
    Block(usize),
    Gap(Gap),
}

/// One exported CSV row: the columns a block carries, copied out so the
/// export worker owns them and the frame loop keeps no part in the write.
struct ExportBlock {
    address: u64,
    block_type: String,
    length: u64,
    data_size: u64,
    link_count: u64,
    summary: String,
}

impl From<&BlockInfo> for ExportBlock {
    fn from(block: &BlockInfo) -> Self {
        Self {
            address: block.address,
            block_type: block.block_type.clone(),
            length: block.length,
            data_size: block.data_size,
            link_count: block.link_count,
            summary: block.summary.clone(),
        }
    }
}

/// Search and filter state for the block list, plus the rows they produce.
#[derive(Default)]
pub struct BlockBrowser {
    search: String,
    /// Which block types pass the filter. Empty means "every type" — the
    /// state the list starts in and returns to when the chips are cleared.
    types: Vec<String>,
    show_gaps: bool,
    /// Rows for the current query, rebuilt only when the query changes:
    /// egui redraws continuously, and a file can hold a hundred thousand
    /// blocks.
    rows: Vec<Row>,
    last_query: Option<(String, Vec<String>, bool)>,
    /// The map export, while one runs.
    export_job: Option<Job>,
    /// Outcome of the last export, shown until the next one starts. A failed
    /// write must leave text behind — never a closed dialog and silence.
    export_message: Option<String>,
}

impl BlockBrowser {
    pub fn new() -> Self {
        Self {
            show_gaps: true,
            ..Default::default()
        }
    }

    pub fn reset(&mut self) {
        self.search.clear();
        self.types.clear();
        self.rows.clear();
        self.last_query = None;
        // An export started for the previous file belongs to it; its message
        // would otherwise arrive into the new file's status line. The worker
        // still finishes and writes what it was asked to.
        self.export_job = None;
        self.export_message = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile, selection: &mut Selection) {
        self.poll_export();
        let map = &loaded.blocks;

        let mut export_clicked = false;
        ui.horizontal(|ui| {
            ui.label("Find:");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(140.0)
                    .hint_text("##CN, 0x4a8, name"),
            );
            if ui.button("Clear").clicked() {
                self.search.clear();
                self.types.clear();
            }
            ui.separator();
            ui.add_enabled_ui(self.export_job.is_none(), |ui| {
                if ui
                    .button("Export map\u{2026}")
                    .on_hover_text(
                        "Exports the filtered list \u{2014} search text and type chips applied \
                         \u{2014} as a CSV file.",
                    )
                    .clicked()
                {
                    export_clicked = true;
                }
            });
        });
        if export_clicked {
            self.start_map_export(ui, loaded);
        }
        if self.export_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Exporting\u{2026}");
            });
        } else if let Some(message) = &self.export_message {
            ui.label(message);
        }

        // The type chips double as the file's composition: the counts say
        // what this file is made of before any of them is clicked.
        ui.horizontal_wrapped(|ui| {
            for (block_type, count) in map.type_counts() {
                let active = self.types.contains(&block_type);
                if ui
                    .selectable_label(active, format!("{block_type} {count}"))
                    .clicked()
                {
                    if active {
                        self.types.retain(|t| t != &block_type);
                    } else {
                        self.types.push(block_type);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_gaps, "Show gaps");
            ui.weak(format!(
                "{} blocks \u{00b7} {:.1}% of {} bytes covered",
                map.blocks.len(),
                map.covered_bytes as f64 / map.file_size.max(1) as f64 * 100.0,
                map.file_size
            ));
        });
        if !map.warnings.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(200, 140, 40),
                format!("{} warnings \u{2014} see the File view", map.warnings.len()),
            );
        }
        ui.separator();

        let query = (self.search.clone(), self.types.clone(), self.show_gaps);
        if self.last_query.as_ref() != Some(&query) {
            self.rows = build_rows(loaded, &query.0, &query.1, query.2);
            self.last_query = Some(query);
        }

        if self.rows.is_empty() {
            ui.label("No block matches.");
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, self.rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // A summary long enough to wrap would make a row two lines
                // tall, and `show_rows` places every row on the assumption
                // that they are all `row_height`. Truncating keeps the list
                // aligned with where the scroll area thinks its rows are.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for row in &self.rows[range] {
                    match row {
                        Row::Block(index) => {
                            let block = &map.blocks[*index];
                            let selected = *selection == Selection::Block(block.address);
                            let label = format!(
                                "{:>10}  {}  {:>10}  {}",
                                format!("{:#x}", block.address),
                                block.block_type,
                                human_bytes(block.length),
                                block.summary
                            );
                            let response = ui
                                .selectable_label(selected, egui::RichText::new(label).monospace());
                            if response.clicked() {
                                *selection = Selection::Block(block.address);
                            }
                        }
                        Row::Gap(gap) => {
                            // Padding between blocks is normal — MF4 aligns
                            // blocks to eight bytes — so it is named as such
                            // rather than reported as if something were
                            // missing. A larger hole is a different thing and
                            // says so.
                            let what = if gap.length < 8 {
                                "alignment padding"
                            } else {
                                "not covered by any block"
                            };
                            ui.label(
                                egui::RichText::new(format!(
                                    "{:>10}  \u{2014}\u{2014}  {:>10}  {what}",
                                    format!("{:#x}", gap.address),
                                    human_bytes(gap.length)
                                ))
                                .monospace()
                                .weak(),
                            );
                        }
                    }
                }
            });
    }

    /// Collects the export worker's message when it finishes. A worker that
    /// ends without one is reported like every other failure in this panel:
    /// as text, not as silence.
    fn poll_export(&mut self) {
        if let Some(job) = &self.export_job {
            if let Some(message) = job.poll() {
                self.export_message = Some(message);
                self.export_job = None;
            }
        }
    }

    /// Picks a path on the UI thread (the dialog has to run there), then
    /// builds the CSV and writes it on a worker thread — a file can hold a
    /// hundred thousand blocks, and that much formatting and writing does
    /// not belong in the frame loop.
    fn start_map_export(&mut self, ui: &egui::Ui, loaded: &LoadedFile) {
        // Built from the query itself rather than the cached row list, so
        // the export matches what the filters show now even if the cache is
        // a frame behind. Gaps are not blocks, so they stay out.
        let rows = build_rows(loaded, &self.search, &self.types, self.show_gaps);
        let blocks: Vec<ExportBlock> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Block(index) => Some(ExportBlock::from(&loaded.blocks.blocks[*index])),
                Row::Gap(_) => None,
            })
            .collect();
        if blocks.is_empty() {
            self.export_message =
                Some("nothing to export \u{2014} the filter matches no blocks".to_string());
            return;
        }
        let stem = loaded
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .map(|stem| stem.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_"))
            .unwrap_or_else(|| "blocks".to_string());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{stem}-blocks.csv"))
            .save_file()
        else {
            return;
        };
        self.export_message = None;
        self.export_job = Some(Job::spawn(ui.ctx(), move || run_map_export(&blocks, &path)));
    }
}

/// The rows for one query. Blocks and gaps are both address-sorted, so they
/// merge in one pass and the list reads as a walk through the file.
fn build_rows(loaded: &LoadedFile, search: &str, types: &[String], show_gaps: bool) -> Vec<Row> {
    let map = &loaded.blocks;
    let needle = search.trim().to_lowercase();
    // An address typed with or without `0x` selects the block containing it,
    // which is how a link value from another block gets followed by hand.
    let address: Option<u64> = needle
        .strip_prefix("0x")
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .or_else(|| needle.parse::<u64>().ok());

    let mut rows = Vec::new();
    let mut gaps = map.gaps.iter().peekable();
    for (index, block) in map.blocks.iter().enumerate() {
        while let Some(gap) = gaps.peek() {
            if gap.address >= block.address {
                break;
            }
            let gap = *gaps.next().expect("peeked");
            if show_gaps && needle.is_empty() && types.is_empty() {
                rows.push(Row::Gap(gap));
            }
        }

        if !types.is_empty() && !types.iter().any(|t| t == &block.block_type) {
            continue;
        }
        if !needle.is_empty() {
            let matches_address = address.is_some_and(|a| a == block.address);
            let matches_text = block.block_type.to_lowercase().contains(&needle)
                || block.summary.to_lowercase().contains(&needle);
            if !matches_address && !matches_text {
                continue;
            }
        }
        rows.push(Row::Block(index));
    }
    if show_gaps && needle.is_empty() && types.is_empty() {
        rows.extend(gaps.map(|gap| Row::Gap(*gap)));
    }
    rows
}

/// The worker side of the map export: builds the CSV and writes it. The
/// message it returns becomes the line under the toolbar, so a failed write
/// ends up as text, never as silence.
fn run_map_export(blocks: &[ExportBlock], path: &Path) -> String {
    let mut csv = String::with_capacity(blocks.len() * 96);
    csv.push_str("address,type,length,data_size,link_count,summary\n");
    for block in blocks {
        let _ = writeln!(
            csv,
            "{:#x},{},{},{},{},{}",
            block.address,
            block.block_type,
            block.length,
            block.data_size,
            block.link_count,
            csv_field(&block.summary),
        );
    }
    match std::fs::write(path, csv) {
        Ok(()) => format!("exported {} block(s) to {}", blocks.len(), path.display()),
        Err(e) => format!("export failed: {e}"),
    }
}

/// Quotes a CSV field when it holds anything that would break the row's
/// shape. Summaries carry commas and quotes, which CSV escapes by doubling
/// the quotes and wrapping the field.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// The right-hand view of one block: header fields, links, referrers and
/// bytes.
///
/// Every link and referrer is a button that moves the selection there, so
/// the file's graph can be walked in the direction the format itself
/// defines — which is the part a tree view cannot show.
#[derive(Default)]
pub struct BlockInspector {
    hex: HexView,
    /// The bytes last read, and the address they came from. Without this the
    /// hex view would re-read its block on every frame — sixty reads a
    /// second of something that cannot have changed.
    bytes: Option<(u64, Vec<u8>)>,
    error: Option<String>,
}

impl BlockInspector {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        address: u64,
        selection: &mut Selection,
    ) {
        show_block_detail(ui, loaded, address, selection, self);
    }

    /// The block's bytes, read once per block rather than once per frame.
    fn bytes_for(&mut self, loaded: &LoadedFile, address: u64, len: usize) -> Option<&[u8]> {
        if self.bytes.as_ref().map(|(a, _)| *a) != Some(address) {
            match loaded.file.read_raw(address, len) {
                Ok(bytes) => {
                    self.bytes = Some((address, bytes));
                    self.error = None;
                }
                Err(e) => {
                    self.bytes = None;
                    self.error = Some(e.to_string());
                }
            }
        }
        self.bytes.as_ref().map(|(_, bytes)| bytes.as_slice())
    }
}

fn show_block_detail(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    address: u64,
    selection: &mut Selection,
    inspector: &mut BlockInspector,
) {
    let Some(block) = loaded.blocks.block_at(address) else {
        ui.heading(format!("No block at {address:#x}"));
        ui.label("The file has no block starting at that address.");
        return;
    };

    ui.heading(format!("{} at {:#x}", block.block_type, block.address));
    if !block.summary.is_empty() {
        ui.label(&block.summary);
    }

    egui::Grid::new("block_header_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Address");
            ui.horizontal(|ui| {
                ui.label(format!("{:#x} ({})", block.address, block.address));
                if ui.small_button("Copy address").clicked() {
                    ui.ctx().copy_text(format!("{:#x}", block.address));
                }
            });
            ui.end_row();

            ui.label("Length");
            ui.label(format!(
                "{} bytes ({})",
                block.length,
                human_bytes(block.length)
            ));
            ui.end_row();

            ui.label("Header / links / data");
            ui.label(format!(
                "24 + {} + {} bytes",
                block.link_count * 8,
                block.data_size
            ));
            ui.end_row();

            ui.label("Ends at");
            ui.label(format!("{:#x}", block.address + block.length));
            ui.end_row();
        });

    show_links(ui, loaded, block, selection);
    show_referrers(ui, loaded, block, selection);

    ui.separator();
    let block = block.clone();
    show_text(ui, loaded, &block, inspector);
    show_bytes(ui, loaded, &block, inspector);
}

/// A text or metadata block's contents, in full and selectable.
///
/// The summary in the list is one truncated line, and a metadata block's XML
/// is where a writer puts everything it had no field for — so the block view
/// has to be able to show the whole thing, not a preview of it.
fn show_text(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    block: &BlockInfo,
    inspector: &mut BlockInspector,
) {
    if !matches!(block.block_type.as_str(), "##TX" | "##MD") {
        return;
    }
    let shown = block.length.min(HEX_LIMIT);
    let Some(bytes) = inspector.bytes_for(loaded, block.address, shown as usize) else {
        return;
    };
    // A text block's data starts after the header; TX text is NUL-terminated,
    // MD text is XML that runs to the end of the block.
    let data = &bytes[BLOCK_HEADER_SIZE.min(bytes.len())..];
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let text = String::from_utf8_lossy(&data[..end]).into_owned();

    egui::CollapsingHeader::new("Text")
        .default_open(true)
        .show(ui, |ui| {
            if text.is_empty() {
                ui.weak("(empty)");
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("block_text")
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&text).monospace()).selectable(true),
                    );
                });
        });
}

fn show_links(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    block: &BlockInfo,
    selection: &mut Selection,
) {
    let used = block.links.iter().filter(|&&l| l != 0).count();
    egui::CollapsingHeader::new(format!("Links ({used} of {} set)", block.link_count))
        .default_open(true)
        .show(ui, |ui| {
            if block.links.is_empty() {
                ui.weak("This block type carries no links.");
                return;
            }
            egui::Grid::new("block_links_grid")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    for (index, &link) in block.links.iter().enumerate() {
                        let label = block
                            .link_labels
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| format!("link[{index}]"));
                        ui.monospace(label);
                        if link == 0 {
                            // Zero is how the format spells "not used". Saying so
                            // beats printing 0x0, which reads like an address.
                            ui.weak("not set");
                            ui.label("");
                        } else if let Some(target) = loaded.blocks.block_at(link) {
                            if ui
                                .button(egui::RichText::new(format!("{link:#x}")).monospace())
                                .clicked()
                            {
                                *selection = Selection::Block(link);
                            }
                            ui.label(format!("{} \u{2014} {}", target.block_type, target.summary));
                        } else {
                            ui.monospace(format!("{link:#x}"));
                            ui.colored_label(
                                egui::Color32::from_rgb(200, 140, 40),
                                "no block found there",
                            );
                        }
                        ui.end_row();
                    }
                });
        });
}

fn show_referrers(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    block: &BlockInfo,
    selection: &mut Selection,
) {
    egui::CollapsingHeader::new(format!("Referenced by ({})", block.referenced_by.len()))
        .default_open(false)
        .show(ui, |ui| {
            if block.referenced_by.is_empty() {
                ui.weak(
                    "Nothing links here. The identification and header blocks sit at fixed positions; anything else is unreferenced.",
                );
                return;
            }
            for &referrer in &block.referenced_by {
                let text = match loaded.blocks.block_at(referrer) {
                    Some(source) => format!("{:#x}  {}  {}", referrer, source.block_type, source.summary),
                    None => format!("{referrer:#x}"),
                };
                if ui
                    .button(egui::RichText::new(text).monospace())
                    .clicked()
                {
                    *selection = Selection::Block(referrer);
                }
            }
        });
}

fn show_bytes(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    block: &BlockInfo,
    inspector: &mut BlockInspector,
) {
    let shown = block.length.min(HEX_LIMIT);
    ui.horizontal(|ui| {
        ui.strong("Bytes");
        if shown < block.length {
            ui.weak(format!(
                "first {} of {}",
                human_bytes(shown),
                human_bytes(block.length)
            ));
        }
    });
    // The borrow of the cached bytes has to end before the error is read, so
    // the two are pulled apart rather than matched on together.
    let address = block.address;
    if inspector
        .bytes_for(loaded, address, shown as usize)
        .is_some()
    {
        let (_, bytes) = inspector.bytes.take().expect("just cached");
        inspector.hex.show(ui, &bytes, address);
        inspector.bytes = Some((address, bytes));
    } else if let Some(error) = inspector.error.clone() {
        ui.colored_label(
            egui::Color32::from_rgb(220, 80, 80),
            format!("The bytes could not be read: {error}"),
        );
    }
}

/// Sizes as a person reads them. Exact byte counts are still shown beside
/// the important ones; this is for the columns where they would not fit.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_leaves_plain_text_alone() {
        assert_eq!(csv_field("first ##CN channel"), "first ##CN channel");
    }

    #[test]
    fn csv_field_wraps_commas_and_newlines() {
        assert_eq!(csv_field("a, b"), "\"a, b\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn csv_field_doubles_inner_quotes() {
        assert_eq!(csv_field("said \"hi\""), "\"said \"\"hi\"\"\"");
        assert_eq!(csv_field("a,\"b\""), "\"a,\"\"b\"\"\"");
    }

    #[test]
    fn run_map_export_writes_one_row_per_block() {
        let blocks = vec![ExportBlock {
            address: 0x40,
            block_type: "##HD".to_string(),
            length: 104,
            data_size: 32,
            link_count: 3,
            summary: "time, 2024-01-01 \"start\"".to_string(),
        }];
        let path = std::env::temp_dir().join(format!(
            "falcon-block-export-test-{}.csv",
            std::process::id()
        ));
        let message = run_map_export(&blocks, &path);
        assert!(message.starts_with("exported 1 block(s) to "), "{message}");
        let csv = std::fs::read_to_string(&path).expect("the export was written");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            csv,
            "address,type,length,data_size,link_count,summary\n\
             0x40,##HD,104,32,3,\"time, 2024-01-01 \"\"start\"\"\"\n"
        );
    }
}
