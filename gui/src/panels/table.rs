//! The tabular sample view: one row per sample, one column per selected
//! channel.
//!
//! Decoding runs on worker threads (same pattern as `signal_loader.rs` and
//! `plot.rs`): `Signal` is `Send + Sync` and owns its data, so only the
//! decoded `SignalValues` cross back to the UI. Rows are virtualized with
//! `ScrollArea::show_rows`, so a group with millions of samples only ever
//! builds the widgets for the rows on screen.
//!
//! Sorting, filtering and CSV export all work by permutation, never by
//! moving data: each produces a `Vec<usize>` of sample numbers that the
//! body reads the decoded columns through. The logic lives in free
//! functions over plain data (`sorted_indices`, `matching_indices`,
//! `csv_row`) so `gui/tests/` can exercise it without a `Ui`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use falcon_mdf::{CanopenDate, CanopenTime, ChannelGroup, Mf4File, SignalValues};

use crate::job::Job;

/// Width of the row-index column. Narrower than a data column: it only ever
/// holds a number.
const INDEX_COL_WIDTH: f32 = 80.0;
/// Width of every data column. Wide enough for a converted float, narrow
/// enough to fit several on screen; longer text (hex payloads) truncates.
const DATA_COL_WIDTH: f32 = 140.0;
/// Channels beyond the master that are selected by default. Groups can hold
/// hundreds of channels; defaulting to all of them would start hundreds of
/// decodes at once.
const DEFAULT_CHANNEL_COUNT: usize = 12;

/// One column's decoded samples, with the channel's per-sample validity.
struct ColumnData {
    values: SignalValues,
    /// `None` means the channel carries no invalidation info and every
    /// sample is valid. See `ChannelSignal::valid` in `signal_loader.rs`.
    valid: Option<Vec<bool>>,
}

enum ColumnResult {
    Ok(ColumnData),
    Err { message: String },
}

/// One channel column's decode state.
enum Slot {
    Loading(Receiver<ColumnResult>),
    Loaded(ColumnData),
    /// Decode failed — or the channel declared itself unreadable before a
    /// decode was even attempted. Either way the reason is shown in the
    /// column's cells; a failed channel is never an empty column.
    Failed(String),
}

/// Table of decoded samples for one channel group.
pub struct TablePanel {
    /// The group the cached state belongs to. When `show` is called with a
    /// different group, every slot and the column selection are stale and
    /// must be rebuilt.
    group_key: Option<(usize, usize)>,
    /// Which of the group's channels are shown as columns, by channel index.
    selected: Vec<bool>,
    /// One decode per channel index, spawned lazily as columns are selected.
    slots: HashMap<usize, Slot>,
    /// Current value of the "go to sample" box.
    goto: u64,
    /// A scroll requested by the go-to box, applied to the next frame's
    /// scroll area (the offset can only be set when the area is built).
    scroll_to_row: Option<u64>,
    /// The column sorted and its direction, if any. Clicking a header
    /// cycles ascending, then descending, then back to file order.
    sort: Option<(usize, bool)>,
    /// The filter box's contents. A row survives when any of its shown
    /// cells contains this, case-insensitively.
    filter_query: String,
    /// Bumped whenever decoded data changes (a column's decode lands or the
    /// group switches), so the view cache knows when it is stale.
    generation: u64,
    /// The filter-then-sort result plus everything it was computed from.
    /// Kept between frames and rebuilt only when one of its inputs changes,
    /// never per frame.
    view_cache: Option<ViewCache>,
    /// The CSV write running on a worker thread.
    export_job: Option<Job>,
    /// Outcome of the last export, shown until dismissed. A failed write
    /// must leave text near the button, not a closed dialog and a silence.
    notice: Option<String>,
}

/// A computed view of the rows — sample numbers in display order — plus
/// the inputs it was computed from. When any input no longer matches, the
/// view is rebuilt.
struct ViewCache {
    query: String,
    sort: Option<(usize, bool)>,
    generation: u64,
    columns: Vec<usize>,
    rows: Vec<usize>,
}

impl Default for TablePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl TablePanel {
    pub fn new() -> Self {
        Self {
            group_key: None,
            selected: Vec::new(),
            slots: HashMap::new(),
            goto: 0,
            scroll_to_row: None,
            sort: None,
            filter_query: String::new(),
            generation: 0,
            view_cache: None,
            export_job: None,
            notice: None,
        }
    }

    /// Drops every cached decode. Called when a new file is opened.
    pub fn reset(&mut self) {
        // Clearing the group key makes the next `show` treat its group as
        // new, which rebuilds the selection and drops the slots.
        self.group_key = None;
        self.selected = Vec::new();
        self.slots.clear();
        self.goto = 0;
        self.scroll_to_row = None;
        self.sort = None;
        self.filter_query.clear();
        self.generation += 1;
        self.view_cache = None;
        // An export started for the previous file belongs to it; its notice
        // would otherwise land in the new file's view. The worker still
        // finishes and writes what it was asked to.
        self.export_job = None;
        self.notice = None;
    }

    /// Draws the table for channel group `cg_index` of data group `dg_index`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        file: &Arc<Mf4File>,
        dg_index: usize,
        cg_index: usize,
    ) {
        let Some(group) = file
            .data_groups()
            .get(dg_index)
            .and_then(|dg| dg.channel_groups.get(cg_index))
        else {
            ui.label(format!(
                "data group {dg_index} has no channel group {cg_index}"
            ));
            return;
        };

        if self.group_key != Some((dg_index, cg_index)) {
            // A different group: cached decodes are keyed by channel index,
            // which means a different channel now. Dropping a Loading slot's
            // receiver just discards the worker's result when it lands. The
            // view state is the same — its rows belong to the old group.
            self.group_key = Some((dg_index, cg_index));
            self.slots.clear();
            self.selected = default_selected(group);
            self.goto = 0;
            self.scroll_to_row = None;
            self.sort = None;
            self.filter_query.clear();
            self.generation += 1;
            self.view_cache = None;
            self.export_job = None;
            self.notice = None;
        }

        let columns = column_order(group, &self.selected);
        self.sync_slots(ui, file, dg_index, cg_index, group, &columns);
        self.poll();
        if let Some(job) = &self.export_job {
            if let Some(message) = job.poll() {
                self.notice = Some(message);
                self.export_job = None;
            }
        }

        // A sort whose column is no longer shown has no header to hang its
        // arrow from; drop it rather than order by a hidden column.
        if let Some((ci, _)) = self.sort {
            if !columns.contains(&ci) {
                self.sort = None;
            }
        }

        let name = if group.acquisition_name.is_empty() {
            "(unnamed group)"
        } else {
            &group.acquisition_name
        };
        ui.strong(format!("{name} — {} samples", group.sample_count));

        self.show_column_picker(ui, group);

        let n_rows = group.sample_count as usize;
        if n_rows == 0 {
            ui.label("This group has no samples.");
            return;
        }

        self.ensure_view(&columns, n_rows);
        let visible_count = self.view_rows().len();

        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter_query)
                    .desired_width(160.0)
                    .hint_text("matches any column"),
            );
            ui.label(format!("{visible_count} of {n_rows} samples"));
        });

        ui.horizontal(|ui| {
            let busy = self.export_job.is_some();
            if busy {
                ui.spinner();
                ui.label("Exporting\u{2026}");
            }
            ui.add_enabled_ui(!busy, |ui| {
                if ui.button("Export table\u{2026}").clicked() {
                    self.start_export(ui, group, &columns, n_rows);
                }
            });
        });

        if let Some(notice) = self.notice.clone() {
            ui.horizontal(|ui| {
                ui.label(notice);
                if ui.small_button("dismiss").clicked() {
                    self.notice = None;
                }
            });
        }

        ui.horizontal(|ui| {
            ui.label("Go to sample:");
            let response = ui.add(
                egui::DragValue::new(&mut self.goto)
                    .range(0..=group.sample_count - 1)
                    .speed(1.0),
            );
            if response.changed() {
                self.scroll_to_row = Some(self.goto);
            }
        });

        let scroll_request = self.scroll_to_row.take();
        self.ensure_view(&columns, n_rows);
        let visible = self.view_rows();
        let clicked = self.show_grid(ui, group, &columns, visible, scroll_request);
        if let Some(ci) = clicked {
            // The header click cycle: ascending, descending, file order.
            self.sort = match self.sort {
                Some((prev, false)) if prev == ci => Some((ci, true)),
                Some((prev, true)) if prev == ci => None,
                _ => Some((ci, false)),
            };
        }
    }

    /// The checkboxes choosing which channels become columns.
    fn show_column_picker(&mut self, ui: &mut egui::Ui, group: &ChannelGroup) {
        let n_selected = self.selected.iter().filter(|&&s| s).count();
        egui::CollapsingHeader::new(format!(
            "Columns ({n_selected} of {} selected)",
            group.channels.len()
        ))
        .default_open(false)
        .show(ui, |ui| {
            // Not virtualized: even a few hundred checkboxes is a cheap
            // frame compared to the decodes each one gates.
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (i, ch) in group.channels.iter().enumerate() {
                        let mut label = ch.name.clone();
                        if !ch.unit.is_empty() {
                            label = format!("{label} [{}]", ch.unit);
                        }
                        if ch.is_master() {
                            label = format!("{label} (master)");
                        }
                        ui.checkbox(&mut self.selected[i], label);
                    }
                });
        });
    }

    /// Starts decodes for selected columns that have no slot yet.
    fn sync_slots(
        &mut self,
        ui: &egui::Ui,
        file: &Arc<Mf4File>,
        dg_index: usize,
        cg_index: usize,
        group: &ChannelGroup,
        columns: &[usize],
    ) {
        for &ci in columns {
            if self.slots.contains_key(&ci) {
                continue;
            }
            // A channel that already declares itself unreadable never
            // reaches the worker thread: the reason it carries *is* the
            // answer, so it becomes a failure slot directly. `unreadable()`
            // is pure metadata — no I/O.
            let slot = match group.channels[ci].unreadable() {
                Some(reason) => Slot::Failed(reason.to_string()),
                None => Slot::Loading(spawn_column_load(
                    file.clone(),
                    dg_index,
                    cg_index,
                    ci,
                    ui.ctx().clone(),
                )),
            };
            self.slots.insert(ci, slot);
        }
    }

    fn poll(&mut self) {
        let mut changed = false;
        for slot in self.slots.values_mut() {
            // The receive has to happen before the slot is overwritten, so
            // the result is moved out of the borrow first.
            let result = match slot {
                Slot::Loading(rx) => Some(rx.try_recv()),
                _ => None,
            };
            match result {
                Some(Ok(ColumnResult::Ok(data))) => {
                    *slot = Slot::Loaded(data);
                    changed = true;
                }
                Some(Ok(ColumnResult::Err { message })) => {
                    *slot = Slot::Failed(message);
                    changed = true;
                }
                Some(Err(TryRecvError::Empty)) | None => {}
                Some(Err(TryRecvError::Disconnected)) => {
                    *slot = Slot::Failed("decode thread ended without a result".to_string());
                    changed = true;
                }
            }
        }
        if changed {
            // New decoded data means any cached view was built from less.
            self.generation += 1;
        }
    }

    /// The header row plus the virtualized body. Both live in one horizontal
    /// scroll area so the header scrolls with its columns. Data columns have
    /// clickable headers; the clicked column (if any) is returned so `show`
    /// can advance the sort.
    fn show_grid(
        &self,
        ui: &mut egui::Ui,
        group: &ChannelGroup,
        columns: &[usize],
        visible_rows: &[usize],
        scroll_request: Option<u64>,
    ) -> Option<usize> {
        let mut clicked: Option<usize> = None;
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Zero vertical spacing between rows: `show_rows` assumes
                // each row consumes exactly `row_height`, and any spacing
                // would accumulate into drift over millions of rows.
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 4.0;

                ui.horizontal(|ui| {
                    header_cell(ui, INDEX_COL_WIDTH, "#");
                    for &ci in columns {
                        let ch = &group.channels[ci];
                        let label = if ch.unit.is_empty() {
                            ch.name.clone()
                        } else {
                            format!("{} [{}]", ch.name, ch.unit)
                        };
                        let arrow = match self.sort {
                            Some((sorted, descending)) if sorted == ci => {
                                if descending {
                                    " \u{25bc}"
                                } else {
                                    " \u{25b2}"
                                }
                            }
                            _ => "",
                        };
                        let tooltip = match self.sort {
                            Some((sorted, false)) if sorted == ci => "click for descending",
                            Some((sorted, true)) if sorted == ci => "click to restore file order",
                            _ => "click to sort ascending",
                        };
                        let response = ui.add_sized(
                            [DATA_COL_WIDTH, row_height],
                            egui::Label::new(
                                egui::RichText::new(format!("{label}{arrow}"))
                                    .monospace()
                                    .strong(),
                            )
                            .truncate()
                            .sense(egui::Sense::click()),
                        );
                        if response.on_hover_text(tooltip).clicked() {
                            clicked = Some(ci);
                        }
                    }
                });

                let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if let Some(sample) = scroll_request {
                    // The go-to box names a sample, not a display position;
                    // once filtered or sorted they are no longer the same.
                    if let Some(position) =
                        visible_rows.iter().position(|&row| row as u64 == sample)
                    {
                        scroll = scroll.vertical_scroll_offset(position as f32 * row_height);
                    }
                }
                let slots = &self.slots;
                scroll.show_rows(ui, row_height, visible_rows.len(), |ui, range| {
                    for position in range {
                        let row = visible_rows[position];
                        ui.horizontal(|ui| {
                            index_cell(ui, row_height, row);
                            for &ci in columns {
                                data_cell(ui, row_height, slots.get(&ci), row);
                            }
                        });
                    }
                });
            });
        clicked
    }

    /// Rebuilds the view cache when one of its inputs has changed. Split
    /// from [`Self::view_rows`] so callers can hold the row list (a shared
    /// borrow) without keeping a mutable borrow alive.
    fn ensure_view(&mut self, columns: &[usize], n_rows: usize) {
        let stale = match &self.view_cache {
            None => true,
            Some(cache) => {
                cache.query != self.filter_query
                    || cache.sort != self.sort
                    || cache.generation != self.generation
                    || cache.columns != columns
            }
        };
        if stale {
            let rows = self.build_view(columns, n_rows);
            self.view_cache = Some(ViewCache {
                query: self.filter_query.clone(),
                sort: self.sort,
                generation: self.generation,
                columns: columns.to_vec(),
                rows,
            });
        }
    }

    /// The sample numbers currently shown, in display order: the filter
    /// applied, then the sort. Only call after [`Self::ensure_view`].
    fn view_rows(&self) -> &[usize] {
        &self
            .view_cache
            .as_ref()
            .expect("the cache was just built")
            .rows
    }

    /// Filter first — keep the rows with a shown cell containing the query —
    /// then sort what survives. Neither step touches the decoded arrays; the
    /// result is a permutation of sample numbers.
    fn build_view(&self, columns: &[usize], n_rows: usize) -> Vec<usize> {
        let mut rows: Vec<usize> = if self.filter_query.is_empty() {
            (0..n_rows).collect()
        } else {
            let cells: Vec<Vec<Option<String>>> = (0..n_rows)
                .map(|row| {
                    columns
                        .iter()
                        .map(|&ci| cell_entry(self.slots.get(&ci), row))
                        .collect()
                })
                .collect();
            matching_indices(&cells, &self.filter_query)
        };

        if let Some((ci, descending)) = self.sort {
            let keys = column_sort_keys(self.slots.get(&ci), &rows);
            let order = sorted_indices(&keys, descending);
            rows = order.into_iter().map(|i| rows[i]).collect();
        }
        rows
    }

    /// Picks a path on the UI thread (the dialog has to run there),
    /// snapshots what the table currently shows, and writes the CSV on a
    /// worker thread.
    fn start_export(
        &mut self,
        ui: &egui::Ui,
        group: &ChannelGroup,
        columns: &[usize],
        n_rows: usize,
    ) {
        let default_name = if group.acquisition_name.is_empty() {
            "samples.csv".to_string()
        } else {
            format!("{}.csv", group.acquisition_name)
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };

        // The sample-number column first, then the shown columns in their
        // display order — the same cells the grid shows.
        let mut header: Vec<Option<String>> = Vec::with_capacity(columns.len() + 1);
        header.push(Some("#".to_string()));
        for &ci in columns {
            let ch = &group.channels[ci];
            header.push(Some(if ch.unit.is_empty() {
                ch.name.clone()
            } else {
                format!("{} [{}]", ch.name, ch.unit)
            }));
        }

        self.ensure_view(columns, n_rows);
        let visible = self.view_rows();
        let mut rows: Vec<Vec<Option<String>>> = Vec::with_capacity(visible.len());
        for &row in visible {
            let mut entries = Vec::with_capacity(columns.len() + 1);
            entries.push(Some(row.to_string()));
            for &ci in columns {
                entries.push(cell_entry(self.slots.get(&ci), row));
            }
            rows.push(entries);
        }
        let written = rows.len();

        self.export_job = Some(Job::spawn(ui.ctx(), move || {
            let mut text = String::new();
            text.push_str(&csv_row(&header));
            text.push('\n');
            for row in &rows {
                text.push_str(&csv_row(row));
                text.push('\n');
            }
            match std::fs::write(&path, text) {
                Ok(()) => format!("wrote {written} rows to {}", path.display()),
                Err(e) => format!("export failed: {e}"),
            }
        }));
    }
}

/// What one sample sorts by in a sorted column.
#[derive(Clone, Debug)]
pub enum SortKey {
    /// A numeric sample, compared by value.
    Number(f64),
    /// A sample that is not a number — text, a byte payload — compared
    /// case-insensitively.
    Text(String),
    /// An invalid sample. It is not a measurement, so it sorts last in both
    /// directions.
    Invalid,
}

/// The order that sorts `keys`: indices into the slice, ascending or
/// descending, stable for equal keys, and with `SortKey::Invalid` last in
/// both directions. Numbers sort before text when a column mixes the two;
/// descending reverses that grouping along with everything else.
pub fn sorted_indices(keys: &[SortKey], descending: bool) -> Vec<usize> {
    // Text keys are folded to lowercase once here, so the O(n log n)
    // comparisons never pay for case folding.
    let mut order: Vec<(usize, SortKey)> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let key = match key {
                SortKey::Text(text) => SortKey::Text(text.to_lowercase()),
                other => other.clone(),
            };
            (i, key)
        })
        .collect();
    order.sort_by(|a, b| compare_sort_keys(&a.1, &b.1, descending));
    order.into_iter().map(|(i, _)| i).collect()
}

/// The total order behind [`sorted_indices`]. `Invalid` is last regardless
/// of direction; among the rest, descending reverses the ascending order.
fn compare_sort_keys(a: &SortKey, b: &SortKey, descending: bool) -> Ordering {
    let ascending = match (a, b) {
        // Not a measurement: last in both directions, never reversed.
        (SortKey::Invalid, SortKey::Invalid) => return Ordering::Equal,
        (SortKey::Invalid, _) => return Ordering::Greater,
        (_, SortKey::Invalid) => return Ordering::Less,
        (SortKey::Number(x), SortKey::Number(y)) => x.total_cmp(y),
        (SortKey::Number(_), SortKey::Text(_)) => Ordering::Less,
        (SortKey::Text(_), SortKey::Number(_)) => Ordering::Greater,
        // Text keys arrive already lowercased; see `sorted_indices`.
        (SortKey::Text(x), SortKey::Text(y)) => x.cmp(y),
    };
    if descending {
        ascending.reverse()
    } else {
        ascending
    }
}

/// The rows any of whose cells contains `query`, case-insensitively. A
/// `None` cell carries no text and matches nothing; an empty query keeps
/// every row.
pub fn matching_indices(rows: &[Vec<Option<String>>], query: &str) -> Vec<usize> {
    let needle = query.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, cells)| {
            needle.is_empty()
                || cells.iter().any(|cell| {
                    cell.as_deref()
                        .is_some_and(|text| text.to_lowercase().contains(&needle))
                })
        })
        .map(|(i, _)| i)
        .collect()
}

/// One CSV row, RFC 4180 quoting: a field is quoted when it contains a
/// comma, a double quote or a line break, and embedded quotes are doubled.
/// `None` — an invalid sample — writes the em dash the table shows for it,
/// which keeps it apart from a genuinely empty string.
pub fn csv_row(cells: &[Option<String>]) -> String {
    cells
        .iter()
        .map(|cell| csv_field(cell.as_deref().unwrap_or("\u{2014}")))
        .collect::<Vec<_>>()
        .join(",")
}

fn csv_field(text: &str) -> String {
    if !text.contains([',', '"', '\n', '\r']) {
        return text.to_string();
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for ch in text.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

/// The text a cell carries into filtering and export: `Some(text)` for a
/// valid, decoded sample, `None` where the display would show a placeholder
/// (an invalid sample, a column still decoding, one that decoded short). A
/// failed column carries its error message — that is what its cells show.
fn cell_entry(slot: Option<&Slot>, row: usize) -> Option<String> {
    match slot {
        Some(Slot::Loaded(data)) => {
            let valid = data
                .valid
                .as_deref()
                .and_then(|v| v.get(row))
                .copied()
                .unwrap_or(true);
            if valid {
                cell_text(&data.values, row)
            } else {
                None
            }
        }
        Some(Slot::Failed(message)) => Some(message.clone()),
        Some(Slot::Loading(_)) | None => None,
    }
}

/// One sort key per row of the sorted column. Numeric cell texts sort as
/// numbers, everything else as text, and invalid samples become
/// `SortKey::Invalid` so they land last in both directions.
fn column_sort_keys(slot: Option<&Slot>, rows: &[usize]) -> Vec<SortKey> {
    match slot {
        Some(Slot::Loaded(data)) => rows
            .iter()
            .map(|&row| {
                let valid = data
                    .valid
                    .as_deref()
                    .and_then(|v| v.get(row))
                    .copied()
                    .unwrap_or(true);
                match (valid, cell_text(&data.values, row)) {
                    (true, Some(text)) => sort_key_from_text(&text),
                    _ => SortKey::Invalid,
                }
            })
            .collect(),
        // Every cell of a failed column shows the same message, so all keys
        // are equal and the stable sort keeps file order.
        Some(Slot::Failed(message)) => rows
            .iter()
            .map(|_| SortKey::Text(message.clone()))
            .collect(),
        // Still decoding: nothing to order by yet. Invalid keeps file order.
        Some(Slot::Loading(_)) | None => rows.iter().map(|_| SortKey::Invalid).collect(),
    }
}

/// A cell's sort key from the same text the cell shows: whatever parses as
/// a number sorts as one, the rest sorts as text.
fn sort_key_from_text(text: &str) -> SortKey {
    match text.parse::<f64>() {
        Ok(value) => SortKey::Number(value),
        Err(_) => SortKey::Text(text.to_string()),
    }
}

/// The default column selection: the master channel plus the first few
/// others. Channels past the default are one checkbox away.
fn default_selected(group: &ChannelGroup) -> Vec<bool> {
    let master = group.channels.iter().position(|ch| ch.is_master());
    let mut selected = vec![false; group.channels.len()];
    if let Some(m) = master {
        selected[m] = true;
    }
    let mut picked = 0;
    for (i, ch) in group.channels.iter().enumerate() {
        if picked >= DEFAULT_CHANNEL_COUNT {
            break;
        }
        if ch.is_master() {
            continue;
        }
        selected[i] = true;
        picked += 1;
    }
    selected
}

/// The selected channel indices in display order: the master first, then
/// the rest in channel order.
fn column_order(group: &ChannelGroup, selected: &[bool]) -> Vec<usize> {
    let mut columns: Vec<usize> = (0..group.channels.len())
        .filter(|&i| selected.get(i).copied().unwrap_or(false))
        .collect();
    columns.sort_by_key(|&i| (!group.channels[i].is_master(), i));
    columns
}

fn header_cell(ui: &mut egui::Ui, width: f32, text: &str) {
    ui.add_sized(
        [
            width,
            ui.text_style_height(&egui::TextStyle::Monospace) + 4.0,
        ],
        egui::Label::new(egui::RichText::new(text).monospace().strong()).truncate(),
    );
}

fn index_cell(ui: &mut egui::Ui, row_height: f32, row: usize) {
    ui.add_sized(
        [INDEX_COL_WIDTH, row_height],
        egui::Label::new(egui::RichText::new(row.to_string()).monospace()).truncate(),
    );
}

/// One body cell. A column still decoding shows a greyed ellipsis, a failed
/// column shows its reason, and an invalid sample shows a greyed em dash —
/// never a bare empty cell.
fn data_cell(ui: &mut egui::Ui, row_height: f32, slot: Option<&Slot>, row: usize) {
    let (text, grey) = match slot {
        Some(Slot::Loaded(data)) => {
            let valid = data
                .valid
                .as_deref()
                .and_then(|v| v.get(row))
                .copied()
                .unwrap_or(true);
            if !valid {
                ("\u{2014}".to_string(), true)
            } else {
                match cell_text(&data.values, row) {
                    Some(text) => (text, false),
                    None => ("\u{2014}".to_string(), true),
                }
            }
        }
        Some(Slot::Failed(message)) => (message.clone(), false),
        Some(Slot::Loading(_)) => ("\u{2026}".to_string(), true),
        None => return,
    };
    let mut rich = egui::RichText::new(text).monospace();
    if grey {
        rich = rich.color(egui::Color32::GRAY);
    }
    ui.add_sized(
        [DATA_COL_WIDTH, row_height],
        egui::Label::new(rich).truncate(),
    );
}

/// Starts decoding one channel on a new thread and returns a receiver for
/// the result. Mirrors `signal_loader::spawn_signal_load`.
fn spawn_column_load(
    file: Arc<Mf4File>,
    dg_index: usize,
    cg_index: usize,
    channel_index: usize,
    ctx: egui::Context,
) -> Receiver<ColumnResult> {
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        let result = decode_column(&file, dg_index, cg_index, channel_index);
        let _ = tx.send(result);
        ctx.request_repaint();
    });

    rx
}

/// Decodes one channel into typed values plus its validity flags. The
/// `Signal` is kept alive past `values()` so `validity()` is read from the
/// same decode rather than a second one.
fn decode_column(
    file: &Mf4File,
    dg_index: usize,
    cg_index: usize,
    channel_index: usize,
) -> ColumnResult {
    let channel = &file.data_groups()[dg_index].channel_groups[cg_index].channels[channel_index];

    let signal = match file.signal(channel) {
        Ok(s) => s,
        Err(e) => {
            return ColumnResult::Err {
                message: e.to_string(),
            }
        }
    };
    let values = match signal.values() {
        Ok(v) => v,
        Err(e) => {
            return ColumnResult::Err {
                message: e.to_string(),
            }
        }
    };
    let valid = signal.validity();

    ColumnResult::Ok(ColumnData { values, valid })
}

/// The display text for one sample, in the channel's own type: integers as
/// integers, floats at about six significant digits, byte samples as hex,
/// text as itself. `None` means the channel decoded to fewer samples than
/// the group claims.
fn cell_text(values: &SignalValues, row: usize) -> Option<String> {
    match values {
        SignalValues::U8(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::U16(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::U32(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::U64(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::I8(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::I16(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::I32(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::I64(v) => v.get(row).map(|x| x.to_string()),
        SignalValues::F32(v) => v.get(row).map(|&x| format_f64(x as f64)),
        SignalValues::F64(v) => v.get(row).map(|&x| format_f64(x)),
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => {
            values.bytes_at(row).map(hex_bytes)
        }
        SignalValues::Str(v) => v.get(row).cloned(),
        SignalValues::Complex { re, im } => match (re.get(row), im.get(row)) {
            (Some(&r), Some(&i)) => Some(format!("{} {:+}i", format_f64(r), format_f64(i))),
            _ => None,
        },
        SignalValues::CanopenDate(v) => v.get(row).map(format_canopen_date),
        SignalValues::CanopenTime(v) => v.get(row).map(format_canopen_time),
        SignalValues::Array {
            values: v,
            elements_per_sample,
        } => {
            let n = *elements_per_sample;
            if n == 0 {
                return None;
            }
            v.get(row * n..(row + 1) * n).map(format_elements)
        }
        SignalValues::ArrayVarLen { values: v, starts } => {
            let from = *starts.get(row)?;
            let to = *starts.get(row + 1)?;
            v.get(from..to).map(format_elements)
        }
        // The enum is `#[non_exhaustive]`; a variant added later should not
        // break this panel.
        _ => None,
    }
}

/// A float with about six significant digits. `format!("{:.6}")` counts
/// digits after the point instead, which would show a nanovolt-scale value
/// as `0.000000` and a million-scale value with meaningless precision.
fn format_f64(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    let exponent = x.abs().log10().floor() as i32;
    let decimals = (5 - exponent).clamp(0, 15) as usize;
    let formatted = format!("{x:.decimals$}");
    // Trailing zeros are padding from the fixed precision, not information:
    // 1.5 reads better than 1.50000.
    if formatted.contains('.') {
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        formatted
    }
}

/// One byte sample as space-separated hex, e.g. `01 af 3c`.
fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The elements of one array sample, e.g. `[1.5, -2, 3.25]`.
fn format_elements(elements: &[f64]) -> String {
    let inner = elements
        .iter()
        .map(|&x| format_f64(x))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// A CANopen date as `YYYY-MM-DD HH:MM:SS.mmm`. The `ms` field spans the
/// whole minute, so the seconds live inside it (see `CanopenDate`).
fn format_canopen_date(d: &CanopenDate) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        d.year,
        d.month,
        d.day,
        d.hour,
        d.minute,
        d.ms / 1_000,
        d.ms % 1_000
    )
}

/// A CANopen time as days plus a time of day, e.g. `42d 12:34:56.789`.
fn format_canopen_time(t: &CanopenTime) -> String {
    let ms = t.ms_since_midnight;
    format!(
        "{}d {:02}:{:02}:{:02}.{:03}",
        t.days_since_1984,
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        ms % 1_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_format_as_integers() {
        assert_eq!(
            cell_text(&SignalValues::U16(vec![7, 65535]), 1).as_deref(),
            Some("65535")
        );
        assert_eq!(
            cell_text(&SignalValues::I64(vec![-3]), 0).as_deref(),
            Some("-3")
        );
    }

    #[test]
    fn floats_keep_about_six_significant_digits() {
        assert_eq!(format_f64(0.0), "0");
        assert_eq!(format_f64(1.5), "1.5");
        assert_eq!(format_f64(1234.5678), "1234.57");
        assert_eq!(format_f64(0.000_012_345_678), "0.0000123457");
        assert_eq!(format_f64(12_345_678.0), "12345678");
    }

    #[test]
    fn byte_samples_format_as_hex() {
        let v = SignalValues::Bytes {
            data: vec![0x01, 0xAF, 0x3C, 0x00],
            width: 2,
        };
        assert_eq!(cell_text(&v, 1).as_deref(), Some("3c 00"));
    }

    #[test]
    fn text_samples_format_as_themselves() {
        let v = SignalValues::Str(vec!["hello".to_string()]);
        assert_eq!(cell_text(&v, 0).as_deref(), Some("hello"));
    }

    #[test]
    fn out_of_range_rows_report_no_text() {
        assert_eq!(cell_text(&SignalValues::U8(vec![1]), 5), None);
    }
}
