//! The bus view: logged CAN or LIN frames, as a frame list or — for CAN with
//! a database loaded — as decoded signals over time.
//!
//! A bus log holds millions of frames, so the frames are read on a worker
//! thread (`CanFrames` and `LinFrames` own their data and are `Send`), kept
//! for as long as the group stays selected, and drawn through
//! `ScrollArea::show_rows`. Filtering produces a list of surviving indices,
//! cached until the query changes — never a second decode. The signals view
//! decodes the same group against the DBC, also on a worker thread, and
//! caches the result the same way.

use std::io::Write;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use egui_plot::{Legend, Line, Plot};
use falcon_mdf::{BusSignals, CanDatabase, CanFrames, ChannelGroup, LinFrames, Mf4File};

use crate::job::Job;

/// Frames drawn per row height; a fixed height keeps `show_rows` honest.
const ROW_PAD: f32 = 4.0;

/// Line colours for the signals chart, cycled by signal index so a signal
/// keeps its colour as others are checked and unchecked. The length is also
/// the cap on how many signals plot at once: a database can decode hundreds
/// of signals, and all of them on one chart is not a view of anything.
const PALETTE: [egui::Color32; 8] = [
    egui::Color32::from_rgb(31, 119, 180),
    egui::Color32::from_rgb(255, 127, 14),
    egui::Color32::from_rgb(44, 160, 44),
    egui::Color32::from_rgb(214, 39, 40),
    egui::Color32::from_rgb(148, 103, 189),
    egui::Color32::from_rgb(140, 86, 75),
    egui::Color32::from_rgb(227, 119, 194),
    egui::Color32::from_rgb(127, 127, 127),
];

/// Which bus protocol a group logged. Decided from the group's channel names
/// — never by trying one reader and falling back on its error, which would
/// run a whole failing decode before the right one starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    Can,
    Lin,
}

/// How the group is being looked at: frame by frame, or decoded into
/// signals over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewMode {
    #[default]
    Frames,
    Signals,
}

enum Frames {
    Loading(Receiver<Result<FrameData, String>>),
    /// Behind an `Arc` so an export worker can hold the same frames without
    /// copying millions of them out of the panel.
    Loaded(Arc<FrameData>),
    Failed(String),
}

/// The frames of one group, whichever protocol logged them.
enum FrameData {
    Can(CanFrames),
    Lin(LinFrames),
}

/// One frame as the list and the filter use it, whichever protocol it came
/// from. A LIN identifier is six bits; it is widened to `u32` so one code
/// path handles both, and `extended` is `None` because LIN has no extended
/// frame format.
struct Row<'a> {
    timestamp: f64,
    id: u32,
    extended: Option<bool>,
    bus_channel: u8,
    data: &'a [u8],
}

impl FrameData {
    fn len(&self) -> usize {
        match self {
            FrameData::Can(frames) => frames.len(),
            FrameData::Lin(frames) => frames.len(),
        }
    }

    fn get(&self, index: usize) -> Option<Row<'_>> {
        match self {
            FrameData::Can(frames) => frames.get(index).map(|f| Row {
                timestamp: f.timestamp,
                id: f.id,
                extended: f.extended,
                bus_channel: f.bus_channel,
                data: f.data,
            }),
            FrameData::Lin(frames) => frames.get(index).map(|f| Row {
                timestamp: f.timestamp,
                id: u32::from(f.id),
                extended: None,
                bus_channel: f.bus_channel,
                data: f.data,
            }),
        }
    }
}

/// The whole-group decode behind the signals view.
enum SignalDecode {
    Loading(Receiver<Result<Vec<DecodedSeries>, String>>),
    Loaded(Vec<DecodedSeries>),
    Failed(String),
}

/// One decoded bus signal as owned data.
///
/// `BusSignals` borrows its names and value-table texts from the database,
/// so the decode worker copies out everything the chart needs and the
/// borrow ends on the worker thread.
struct DecodedSeries {
    message: String,
    name: String,
    unit: String,
    bus_channel: u8,
    timestamps: Vec<f64>,
    values: Vec<f64>,
    /// Value-table text per reading, when the signal has any. Such a signal
    /// is a state label, not a measurement: it is listed with its texts
    /// rather than plotted as a line.
    texts: Option<Vec<Option<String>>>,
}

/// A loaded CAN database, with the file name it came from for the toolbar.
/// The database itself is behind an `Arc` so the decode worker can hold the
/// same one without a second parse.
struct Database {
    name: String,
    db: Arc<CanDatabase>,
}

/// Frame list and signals chart for one bus-logged channel group.
#[derive(Default)]
pub struct BusPanel {
    /// The group the cached frames belong to; a different group means the
    /// frames are stale and are read again.
    group_key: Option<(usize, usize)>,
    /// The protocol the current group logs, decided once when the group is
    /// selected. The toolbar needs it before the frames arrive: a LIN group
    /// gets no DBC controls even while its read is still in flight.
    protocol: Option<Protocol>,
    mode: ViewMode,
    frames: Option<Frames>,
    /// The decoded series behind the signals view, cached per group and per
    /// database — loading a different DBC drops it, since every reading in
    /// it came from the old one.
    signals: Option<SignalDecode>,
    /// Checkbox state parallel to the loaded series.
    checked: Vec<bool>,
    database: Option<Database>,
    /// Message from the last database load or export, shown until the next
    /// one — a failed action must read as text, not as a button that did
    /// nothing.
    notice: Option<String>,
    /// A CSV export running on a worker thread: a bus log holds millions of
    /// frames, and writing them must not freeze the frame loop.
    export_job: Option<Job>,
    id_filter: String,
    channel_filter: String,
    /// Frame indices surviving the current filter, and the query they were
    /// computed for. `None` means "no filter": every frame is shown, and no
    /// index list is built at all.
    filtered: Option<(String, String, Vec<usize>)>,
    selected: Option<usize>,
}

impl BusPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops cached frames and any loaded database. Called on a new file.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Draws the view for channel group `cg_index` of data group `dg_index`.
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

        if !group.is_bus_event() {
            ui.label(
                "This channel group holds measurements, not logged bus traffic \u{2014} there are no frames to list.",
            );
            return;
        }

        let Some(protocol) = protocol_of(group) else {
            ui.label(
                "This bus group composes its fields under neither CAN_DataFrame nor LIN_Frame \u{2014} this build does not read its layout.",
            );
            return;
        };

        if self.group_key != Some((dg_index, cg_index)) {
            self.group_key = Some((dg_index, cg_index));
            self.protocol = Some(protocol);
            self.frames = None;
            self.signals = None;
            self.checked.clear();
            self.filtered = None;
            self.selected = None;
            if protocol != Protocol::Can {
                // The signals view decodes CAN only; a LIN group has nothing
                // to show there.
                self.mode = ViewMode::Frames;
            }
        }

        if self.frames.is_none() {
            self.frames = Some(Frames::Loading(spawn_frames(
                file.clone(),
                dg_index,
                cg_index,
                protocol,
                ui.ctx().clone(),
            )));
        }
        self.poll();

        self.toolbar(ui, &group.acquisition_name);

        match self.mode {
            ViewMode::Frames => match self.frames.as_ref() {
                Some(Frames::Loading(_)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading frames\u{2026}");
                    });
                }
                Some(Frames::Failed(message)) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        format!("The frames could not be read: {message}"),
                    );
                }
                Some(Frames::Loaded(_)) => self.show_frames(ui),
                None => {}
            },
            ViewMode::Signals => self.show_signal_mode(ui, file),
        }
    }

    fn poll(&mut self) {
        if let Some(job) = &self.export_job {
            if let Some(message) = job.poll() {
                self.notice = Some(message);
                self.export_job = None;
            }
        }
        let result = match &self.frames {
            Some(Frames::Loading(rx)) => Some(rx.try_recv()),
            _ => None,
        };
        match result {
            Some(Ok(Ok(frames))) => self.frames = Some(Frames::Loaded(Arc::new(frames))),
            Some(Ok(Err(message))) => self.frames = Some(Frames::Failed(message)),
            Some(Err(TryRecvError::Empty)) | None => {}
            Some(Err(TryRecvError::Disconnected)) => {
                self.frames = Some(Frames::Failed(
                    "the reader thread ended without a result".to_string(),
                ));
            }
        }
    }

    fn poll_signals(&mut self) {
        let result = match &self.signals {
            Some(SignalDecode::Loading(rx)) => Some(rx.try_recv()),
            _ => None,
        };
        match result {
            Some(Ok(Ok(series))) => {
                // Nothing checked by default: checking is what says which of
                // the hundreds of decoded signals the user means.
                self.checked = vec![false; series.len()];
                self.signals = Some(SignalDecode::Loaded(series));
            }
            Some(Ok(Err(message))) => self.signals = Some(SignalDecode::Failed(message)),
            Some(Err(TryRecvError::Empty)) | None => {}
            Some(Err(TryRecvError::Disconnected)) => {
                self.signals = Some(SignalDecode::Failed(
                    "the decode thread ended without a result".to_string(),
                ));
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, default_name: &str) {
        let is_can = self.protocol == Some(Protocol::Can);
        ui.horizontal_wrapped(|ui| {
            ui.label("View:");
            ui.selectable_value(&mut self.mode, ViewMode::Frames, "Frames");
            // The signals view decodes CAN frames against a DBC; for LIN the
            // toggle is disabled rather than leading to a view that cannot
            // work.
            ui.add_enabled_ui(is_can, |ui| {
                ui.selectable_value(&mut self.mode, ViewMode::Signals, "Signals");
            });
            ui.separator();
            if self.mode == ViewMode::Frames {
                ui.label("ID or name:");
                // For LIN there is no database, so only the identifier
                // matches; hinting a message name would promise a search
                // that finds nothing.
                let hint = match self.protocol {
                    Some(Protocol::Lin) => "0x3B or 59",
                    _ => "0x1F4 or EngineData",
                };
                ui.add(
                    egui::TextEdit::singleline(&mut self.id_filter)
                        .desired_width(120.0)
                        .hint_text(hint),
                );
                ui.label("Bus:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.channel_filter)
                        .desired_width(40.0)
                        .hint_text("all"),
                );
                if ui.button("Clear").clicked() {
                    self.id_filter.clear();
                    self.channel_filter.clear();
                }
                let loaded = match &self.frames {
                    Some(Frames::Loaded(frames)) => Some(Arc::clone(frames)),
                    _ => None,
                };
                let exporting = self.export_job.is_some();
                if ui
                    .add_enabled(
                        loaded.is_some() && !exporting,
                        egui::Button::new("Export to CSV\u{2026}"),
                    )
                    .clicked()
                {
                    if let Some(frames) = loaded {
                        self.start_export(ui, frames, default_name);
                    }
                }
                ui.separator();
            }
            // A LIN group gets no DBC controls: this build decodes no
            // database for LIN, so the buttons could load a file that would
            // then do nothing.
            if is_can {
                match &self.database {
                    Some(db) => {
                        ui.label(format!("{} ({} messages)", db.name, db.db.messages().len()));
                        if ui.button("Unload").clicked() {
                            self.database = None;
                            self.signals = None;
                            self.notice = None;
                            self.mode = ViewMode::Frames;
                        }
                    }
                    None => {
                        if ui.button("Load DBC\u{2026}").clicked() {
                            self.load_database();
                        }
                    }
                }
            }
        });
        // Not gated to CAN: a LIN export reports its outcome here too.
        if let Some(notice) = &self.notice {
            ui.weak(notice);
        }
    }

    /// Picks a path on the UI thread (the dialog has to run there), then
    /// writes the CSV on a worker thread so the frame loop never blocks on
    /// the write.
    fn start_export(&mut self, ui: &egui::Ui, frames: Arc<FrameData>, default_name: &str) {
        let file_name = if default_name.is_empty() {
            "bus_frames.csv".to_string()
        } else {
            format!("{default_name}.csv")
        };
        let Some(path) = rfd::FileDialog::new().set_file_name(file_name).save_file() else {
            return;
        };
        let database = self.database.as_ref().map(|d| Arc::clone(&d.db));
        let indices = self.current_indices(&frames);
        self.export_job = Some(Job::spawn(ui.ctx(), move || {
            let count = indices.as_ref().map_or(frames.len(), |v| v.len());
            let result = match &*frames {
                FrameData::Can(can) => {
                    let can_vec: Vec<falcon_mdf::CanFrame> = can.iter().collect();
                    write_can_csv(&can_vec, indices.as_deref(), database.as_deref(), &path)
                }
                FrameData::Lin(lin) => {
                    let lin_vec: Vec<falcon_mdf::LinFrame> = lin.iter().collect();
                    write_lin_csv(&lin_vec, indices.as_deref(), &path)
                }
            };
            match result {
                Ok(()) => format!("wrote {count} frames to {}", path.display()),
                Err(e) => format!("export failed: {e}"),
            }
        }));
    }

    fn load_database(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CAN database", &["dbc", "DBC"])
            .pick_file()
        else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        match CanDatabase::from_dbc_path(&path) {
            Ok(db) => {
                self.notice = Some(format!(
                    "loaded {} messages from {name}",
                    db.messages().len()
                ));
                // A new database decodes the same frames into different
                // readings, so a cached decode is stale the moment the file
                // changes.
                self.signals = None;
                self.database = Some(Database {
                    name,
                    db: Arc::new(db),
                });
            }
            Err(e) => self.notice = Some(format!("{name} could not be read: {e}")),
        }
    }

    fn current_indices(&mut self, frames: &FrameData) -> Option<Vec<usize>> {
        let database = match self.protocol {
            Some(Protocol::Can) => self.database.as_ref().map(|d| d.db.as_ref()),
            _ => None,
        };

        let query = (
            self.id_filter.trim().to_string(),
            self.channel_filter.trim().to_string(),
        );
        if query.0.is_empty() && query.1.is_empty() {
            self.filtered = None;
            None
        } else {
            if self
                .filtered
                .as_ref()
                .is_none_or(|(id, ch, _)| *id != query.0 || *ch != query.1)
            {
                let indices = filter_frames(frames, &query.0, &query.1, database);
                self.filtered = Some((query.0.clone(), query.1.clone(), indices));
            }
            self.filtered.as_ref().map(|(_, _, v)| v.clone())
        }
    }

    fn show_frames(&mut self, ui: &mut egui::Ui) {
        // The frames are taken out of `self` for the length of this method:
        // `current_indices` needs `&mut self` to cache what it computes, and
        // the borrow checker cannot allow that while `self.frames` is still
        // borrowed. `Frames::Loaded` holds an `Arc`, so this clones a handle
        // rather than the frames.
        let frames = match &self.frames {
            Some(Frames::Loaded(frames)) => Arc::clone(frames),
            _ => return,
        };
        let frames = &*frames;
        // The database decodes CAN only: for a LIN group it must not name or
        // filter identifiers, even if one was loaded for a group seen earlier.
        // Held as an owned handle rather than a reference into `self`, so that
        // `current_indices` below can still take `&mut self` to cache what it
        // computes.
        let database = match self.protocol {
            Some(Protocol::Can) => self.database.as_ref().map(|d| Arc::clone(&d.db)),
            _ => None,
        };

        let shown_indices = self.current_indices(frames);
        let database = database.as_deref();
        let count = shown_indices.as_ref().map_or(frames.len(), |v| v.len());

        let span = match (frames.get(0), frames.get(frames.len().saturating_sub(1))) {
            (Some(first), Some(last)) => {
                format!("{:.6} s \u{2013} {:.6} s", first.timestamp, last.timestamp)
            }
            _ => "no frames".to_string(),
        };
        ui.horizontal(|ui| {
            ui.strong(format!("{count} frames"));
            if shown_indices.is_some() {
                ui.weak(format!("of {}", frames.len()));
            }
            ui.separator();
            ui.weak(span);
        });

        let protocol = self.protocol;
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + ROW_PAD;
        let mut clicked = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .max_height(ui.available_height() * 0.6)
            .show_rows(ui, row_height, count, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // A frame row is one line by construction; letting a long
                // payload wrap would make it two, and `show_rows` places
                // rows on the assumption that all are `row_height`.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for row in range {
                    let index = shown_indices.as_ref().map_or(row, |v| v[row]);
                    let Some(frame) = frames.get(index) else {
                        continue;
                    };
                    let text = match protocol {
                        Some(Protocol::Lin) => {
                            // A LIN identifier is six bits, so the hex and
                            // decimal spellings are both short and both worth
                            // showing: 0x3B beside 59.
                            format!(
                                "{index:>9}  {:>12.6}  {:#04x} ({:>2})  ch{}  [{}]  {}",
                                frame.timestamp,
                                frame.id,
                                frame.id,
                                frame.bus_channel,
                                frame.data.len(),
                                hex_bytes(frame.data),
                            )
                        }
                        _ => {
                            let name = database
                                .and_then(|db| db.message_name(frame.id))
                                .unwrap_or("");
                            format!(
                                "{index:>9}  {:>12.6}  {:>10}{}  ch{}  [{}]  {}  {name}",
                                frame.timestamp,
                                format!("{:#x}", frame.id),
                                if frame.extended == Some(true) {
                                    "x"
                                } else {
                                    " "
                                },
                                frame.bus_channel,
                                frame.data.len(),
                                hex_bytes(frame.data),
                            )
                        }
                    };
                    let selected = self.selected == Some(index);
                    if ui
                        .selectable_label(selected, egui::RichText::new(text).monospace())
                        .clicked()
                    {
                        clicked = Some(index);
                    }
                }
            });
        if let Some(index) = clicked {
            // Clicking the selected row again clears it, so the signal pane
            // can be dismissed without loading a different frame.
            self.selected = if self.selected == Some(index) {
                None
            } else {
                Some(index)
            };
        }

        // The decoded-signal pane decodes against a DBC, which this build
        // supports for CAN only; for LIN there is nothing it could show.
        if self.protocol == Some(Protocol::Can) {
            self.show_signals(ui);
        }
    }

    /// The decoded signals of the selected frame. Without a database there is
    /// nothing to decode against, and the panel says so rather than showing
    /// an empty list that looks like a frame with no content.
    fn show_signals(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.selected else {
            return;
        };
        let Some(Frames::Loaded(frames)) = &self.frames else {
            return;
        };
        let Some(frame) = frames.get(index) else {
            return;
        };
        ui.separator();
        ui.strong(format!(
            "Frame {index} \u{2014} {:#x} at {:.6} s",
            frame.id, frame.timestamp
        ));
        let Some(database) = &self.database else {
            ui.weak("Load a DBC to decode this payload into named signals.");
            return;
        };
        let signals = database.db.decode(frame.id, frame.data);
        if signals.is_empty() {
            ui.weak(format!(
                "{} has no message for identifier {:#x}.",
                database.name, frame.id
            ));
            return;
        }
        egui::Grid::new("bus_signal_grid")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                for signal in signals {
                    ui.label(signal.name);
                    match signal.text {
                        Some(text) => ui.label(format!("{} ({})", text, signal.value)),
                        None => ui.label(format!("{}", signal.value)),
                    };
                    ui.label(signal.unit);
                    ui.end_row();
                }
            });
    }

    /// The signals-over-time view: one decode of the whole group, a checkbox
    /// per decoded signal, one chart for the checked ones.
    fn show_signal_mode(&mut self, ui: &mut egui::Ui, file: &Arc<Mf4File>) {
        let Some(database) = &self.database else {
            ui.weak("Load a DBC to decode this group's frames into signals over time.");
            return;
        };

        if self.signals.is_none() {
            // `decode_bus` decodes every CAN group in the file; this group's
            // bus channels — known from its frames, so the frames must be
            // here first — are what scopes the result back to this group.
            let buses = match &self.frames {
                Some(Frames::Loaded(frames)) => distinct_buses(frames),
                Some(Frames::Loading(_)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading frames\u{2026}");
                    });
                    return;
                }
                Some(Frames::Failed(message)) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        format!("The frames could not be read: {message}"),
                    );
                    return;
                }
                None => return,
            };
            self.signals = Some(SignalDecode::Loading(spawn_signal_decode(
                file.clone(),
                database.db.clone(),
                buses,
                ui.ctx().clone(),
            )));
        }
        self.poll_signals();

        match &self.signals {
            Some(SignalDecode::Loading(_)) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Decoding signals\u{2026}");
                });
            }
            Some(SignalDecode::Failed(message)) => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("The signals could not be decoded: {message}"),
                );
            }
            Some(SignalDecode::Loaded(_)) => self.show_series_list(ui),
            None => {}
        }
    }

    /// The signal list plus the chart of the checked series.
    fn show_series_list(&mut self, ui: &mut egui::Ui) {
        let Some(SignalDecode::Loaded(series)) = &self.signals else {
            return;
        };
        if series.is_empty() {
            ui.weak("The database decoded no signals out of this group's frames.");
            return;
        }

        let checked = &mut self.checked;
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + ROW_PAD;
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, s) in series.iter().enumerate() {
                    let unit = if s.unit.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", s.unit)
                    };
                    let label = format!(
                        "{}.{} ch{}{} \u{2014} {} readings",
                        s.message,
                        s.name,
                        s.bus_channel,
                        unit,
                        s.timestamps.len()
                    );
                    match &s.texts {
                        Some(texts) => {
                            // A value table turns this signal's readings
                            // into labels, not numbers; a line through them
                            // would draw the label's index as if it were a
                            // measurement. Listed with its texts instead.
                            egui::CollapsingHeader::new(format!("{label} \u{2014} text, not plotted"))
                                .id_salt(("bus_text_series", i))
                                .show(ui, |ui| {
                                    ui.weak(
                                        "A value table gives this signal text rather than a number, so there is no line to draw.",
                                    );
                                    egui::ScrollArea::vertical()
                                        .max_height(120.0)
                                        .show_rows(ui, row_height, texts.len(), |ui, range| {
                                            ui.style_mut().wrap_mode =
                                                Some(egui::TextWrapMode::Extend);
                                            for r in range {
                                                let text = match &texts[r] {
                                                    Some(text) => text.clone(),
                                                    None => format!("{}", s.values[r]),
                                                };
                                                ui.monospace(format!(
                                                    "{:>12.6}  {text}",
                                                    s.timestamps[r]
                                                ));
                                            }
                                        });
                                });
                        }
                        None => {
                            if ui.checkbox(&mut checked[i], label).changed()
                                && checked[i]
                                && checked.iter().filter(|&&c| c).count() > PALETTE.len()
                            {
                                checked[i] = false;
                                ui.weak(format!(
                                    "At most {} signals plot at once.",
                                    PALETTE.len()
                                ));
                            }
                        }
                    }
                }
            });

        if !checked.iter().any(|&c| c) {
            ui.weak("Check a signal to plot it.");
            return;
        }

        Plot::new("bus_signal_plot")
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                for (i, s) in series.iter().enumerate() {
                    if !checked[i] {
                        continue;
                    }
                    let name = format!("{}.{}", s.message, s.name);
                    let name = if s.unit.is_empty() {
                        name
                    } else {
                        format!("{name} [{}]", s.unit)
                    };
                    let points: Vec<[f64; 2]> = s
                        .timestamps
                        .iter()
                        .zip(&s.values)
                        .map(|(&t, &v)| [t, v])
                        .collect();
                    plot_ui.line(Line::new(name, points).color(PALETTE[i % PALETTE.len()]));
                }
            });
    }
}

/// Which protocol a bus-event group logged, from the names its frame fields
/// are composed under: `CAN_DataFrame.ID` and friends for CAN,
/// `LIN_Frame.ID` and friends for LIN (see `src/bus.rs` and `src/lin.rs` for
/// the names each reader looks for).
fn protocol_of(group: &ChannelGroup) -> Option<Protocol> {
    if group.find_channel("CAN_DataFrame.ID").is_some() {
        Some(Protocol::Can)
    } else if group.find_channel("LIN_Frame.ID").is_some() {
        Some(Protocol::Lin)
    } else {
        None
    }
}

/// Reads the group's frames on a worker thread. `can_frames` and
/// `lin_frames` decompress and demultiplex the whole group, which on a real
/// log is seconds of work.
fn spawn_frames(
    file: Arc<Mf4File>,
    dg_index: usize,
    cg_index: usize,
    protocol: Protocol,
    ctx: egui::Context,
) -> Receiver<Result<FrameData, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let group = &file.data_groups()[dg_index].channel_groups[cg_index];
        let result = match protocol {
            Protocol::Can => file.can_frames(group).map(FrameData::Can),
            Protocol::Lin => file.lin_frames(group).map(FrameData::Lin),
        }
        .map_err(|e| e.to_string());
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// The bus channels a group's frames came from, in first-seen order.
fn distinct_buses(frames: &FrameData) -> Vec<u8> {
    let mut buses = Vec::new();
    for i in 0..frames.len() {
        if let Some(f) = frames.get(i) {
            if !buses.contains(&f.bus_channel) {
                buses.push(f.bus_channel);
            }
        }
    }
    buses
}

/// Decodes the file's CAN traffic against the database on a worker thread
/// and copies the series of the given buses out as owned data.
fn spawn_signal_decode(
    file: Arc<Mf4File>,
    database: Arc<CanDatabase>,
    buses: Vec<u8>,
    ctx: egui::Context,
) -> Receiver<Result<Vec<DecodedSeries>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        // The result borrows the database (`BusSignals<'a>` names live in
        // it), so the copy happens here, where the borrow is still alive.
        let result = file
            .decode_bus(&database)
            .map(|signals| copy_series(&signals, &buses))
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// Copies the decoded series on the given buses into owned data, value-table
/// texts included.
fn copy_series(signals: &BusSignals, buses: &[u8]) -> Vec<DecodedSeries> {
    signals
        .iter()
        .filter(|s| buses.contains(&s.bus_channel))
        .map(|s| {
            let texts: Vec<Option<String>> = (0..s.len())
                .map(|i| s.text_at(i).map(str::to_string))
                .collect();
            DecodedSeries {
                message: s.message.to_string(),
                name: s.name.to_string(),
                unit: s.unit.to_string(),
                bus_channel: s.bus_channel,
                timestamps: s.timestamps.clone(),
                values: s.values.clone(),
                // A series whose table names no reading it has is numeric
                // for every practical purpose.
                texts: texts.iter().any(|t| t.is_some()).then_some(texts),
            }
        })
        .collect()
}

/// Frame indices matching the query. An empty part of the query matches
/// everything, so filtering on bus channel alone is possible.
fn filter_frames(
    frames: &FrameData,
    id_query: &str,
    channel_query: &str,
    database: Option<&CanDatabase>,
) -> Vec<usize> {
    let wanted_id = parse_id(id_query);
    let name_query = id_query.to_lowercase();
    let wanted_channel: Option<u8> = channel_query.parse().ok();

    (0..frames.len())
        .filter(|&i| {
            let Some(frame) = frames.get(i) else {
                return false;
            };
            if let Some(channel) = wanted_channel {
                if frame.bus_channel != channel {
                    return false;
                }
            }
            if id_query.is_empty() {
                return true;
            }
            if wanted_id == Some(frame.id) {
                return true;
            }
            // Falling through to the name means a query that is not a number
            // still finds something, rather than matching nothing at all.
            database
                .and_then(|db| db.message_name(frame.id))
                .is_some_and(|name| name.to_lowercase().contains(&name_query))
        })
        .collect()
}

/// Reads an identifier written either way round: `0x1F4` as hex, `500` as
/// decimal. A bare hex string without the prefix is ambiguous and is read as
/// decimal, which is what typing digits usually means.
fn parse_id(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Writes the CAN frames `indices` selects — the list the panel is showing,
/// after its filters — to `path` as CSV.
///
/// `indices` of `None` means no filter is active and every frame is written.
/// The time column is milliseconds and says so in its name; the logger
/// records seconds. The message column carries the name from `database`, and
/// is empty when no database is loaded or when the one loaded has no message
/// for that identifier — a name derived from the number instead would be a
/// value the user reads as coming from their database when it did not. An
/// empty selection writes the header alone.
pub fn write_can_csv(
    frames: &[falcon_mdf::CanFrame<'_>],
    indices: Option<&[usize]>,
    database: Option<&falcon_mdf::CanDatabase>,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    if database.is_some() {
        writeln!(
            out,
            "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex,message"
        )?;
    } else {
        writeln!(
            out,
            "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex"
        )?;
    }
    for index in selected(frames.len(), indices) {
        let Some(frame) = frames.get(index) else {
            continue;
        };
        let ext = if frame.extended == Some(true) {
            "true"
        } else {
            "false"
        };
        if let Some(db) = database {
            let message = db.message_name(frame.id).unwrap_or_default();
            writeln!(
                out,
                "{index},{:.6},{},{:#x},{},{},{},{},{}",
                frame.timestamp * 1000.0,
                frame.id,
                frame.id,
                ext,
                frame.bus_channel,
                frame.data.len(),
                hex_bytes(frame.data),
                csv_field(message),
            )?;
        } else {
            writeln!(
                out,
                "{index},{:.6},{},{:#x},{},{},{},{}",
                frame.timestamp * 1000.0,
                frame.id,
                frame.id,
                ext,
                frame.bus_channel,
                frame.data.len(),
                hex_bytes(frame.data),
            )?;
        }
    }
    out.into_inner().map_err(|e| e.into_error())?;
    Ok(())
}

/// Writes the LIN frames `indices` selects, the same arrangement as
/// [`write_can_csv`].
///
/// LIN carries no extended-identifier flag, so that column is always empty,
/// and this build has no LIN database, so there is no message column at all
/// rather than one that could never be filled.
pub fn write_lin_csv(
    frames: &[falcon_mdf::LinFrame<'_>],
    indices: Option<&[usize]>,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(
        out,
        "index,time_ms,id,id_hex,extended,bus_channel,length,data_hex"
    )?;
    for index in selected(frames.len(), indices) {
        let Some(frame) = frames.get(index) else {
            continue;
        };
        writeln!(
            out,
            "{index},{:.6},{},{:#x},,{},{},{}",
            frame.timestamp * 1000.0,
            frame.id,
            frame.id,
            frame.bus_channel,
            frame.data.len(),
            hex_bytes(frame.data),
        )?;
    }
    out.into_inner().map_err(|e| e.into_error())?;
    Ok(())
}

/// The rows an export covers: the filtered list when one is active, every
/// row when it is not.
fn selected(len: usize, indices: Option<&[usize]>) -> Vec<usize> {
    match indices {
        Some(list) => list.to_vec(),
        None => (0..len).collect(),
    }
}

/// One CSV field, quoted when it carries a comma, a quote or a newline, with
/// embedded quotes doubled — RFC 4180, which is what a spreadsheet expects.
pub fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}
