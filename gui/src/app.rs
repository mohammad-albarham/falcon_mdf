//! Top-level application state and layout.
//!
//! The window is two panes. On the left is the file: a structure tree, the
//! block list, and the searchable channel list, all writing to one
//! [`Selection`]. On the right is the content that selection is about —
//! details, plot, samples, bus frames, statistics — chosen with the tabs
//! above it. Everything the file holds is reachable from the left; the right
//! only ever shows one thing at a time, which is what makes it readable.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::loader::{spawn_load, LoadResult};
use crate::batch_queue::BatchQueue;
use crate::panels::batch::BatchPanel;
use crate::model::{
    ChannelLoc, ContentTab, FileSlot, LoadedFile, OpenFiles, PlottedChannel, Selection,
};
use crate::panels::blocks::BlockBrowser;
use crate::panels::bus::BusPanel;
use crate::panels::channel_list::ChannelBrowser;
use crate::panels::details::DetailsPanel;
use crate::panels::numeric::NumericPanel;
use crate::panels::plot::PlotPanel;
use crate::panels::stats::StatsPanel;
use crate::panels::table::TablePanel;
use crate::panels::tree::StructureTree;
use crate::panels::xy::XyPanel;
use crate::recent::RecentFiles;
use crate::session::{prune_to_file, prune_xy, Session, Sessions};

enum LoadState {
    Idle,
    Loading {
        path: PathBuf,
        rx: Receiver<LoadResult>,
    },
    Loaded(LoadedFile),
    /// A file that failed to open. Holds the full `Mf4Error` text, not a
    /// generic "failed to open" — several corpus files exercise unusual
    /// paths, and that text is the point of showing it.
    Failed {
        path: PathBuf,
        message: String,
    },
}

/// Which of the three left-hand views is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavTab {
    Structure,
    Blocks,
    Channels,
}

impl NavTab {
    const ALL: [NavTab; 3] = [NavTab::Structure, NavTab::Blocks, NavTab::Channels];

    fn label(self) -> &'static str {
        match self {
            NavTab::Structure => "Structure",
            NavTab::Blocks => "Blocks",
            NavTab::Channels => "Channels",
        }
    }

    /// The tab a stored session names, or the default when it names
    /// something this version does not have.
    fn from_label(label: &str) -> Self {
        NavTab::ALL
            .into_iter()
            .find(|tab| tab.label() == label)
            .unwrap_or(NavTab::Structure)
    }
}

pub struct FalconApp {
    state: LoadState,
    /// The measurement opened alongside the first one to compare against, if
    /// any. `Idle` when nothing is open there, which is also what closing it
    /// puts it back to.
    second: LoadState,
    /// Which of the two open files the left-hand browser, the detail views
    /// and the computed expressions are about. The plot draws both at once;
    /// everything else is about one file at a time, and this says which.
    active: FileSlot,
    recent: RecentFiles,
    /// What was plotted and open the last time each file was closed.
    sessions: Sessions,
    /// Whether the stored session has already been applied to the file now
    /// open. Restoring is a one-shot on the frame the load lands, and this is
    /// what keeps it from running again every frame afterwards.
    restored: bool,
    /// A restored session whose second file is still opening. Its file-B
    /// channels cannot be checked against a file that is not loaded yet, so
    /// they wait here until it is.
    pending_second: Option<Session>,
    /// Set when the shortcut for the channel search runs, so the search box
    /// takes focus on the next frame — focus can only be requested against a
    /// widget that exists, and it does not exist until the panel draws.
    focus_search: bool,
    /// Whether the keyboard-shortcut list is showing.
    show_shortcuts: bool,
    nav: NavTab,
    tab: ContentTab,
    selection: Selection,
    /// Selections already visited, oldest first, and the ones stepped back
    /// out of. Following a block's links is a walk, and a walk needs a way
    /// back: without it, three links deep into a conversion chain the only
    /// route to where you came from is remembering its address.
    history: Vec<Selection>,
    future: Vec<Selection>,
    /// The selection as of the end of the last frame, so a change made
    /// anywhere — tree, block list, a link button — is what pushes history.
    last_selection: Selection,
    tree: StructureTree,
    block_browser: BlockBrowser,
    browser: ChannelBrowser,
    details: DetailsPanel,
    /// The channels the user has asked the plot panel to draw, in
    /// insertion order. Lives here rather than in `PlotPanel` because the
    /// navigation panes own the interaction that adds and removes entries
    /// (checkbox, color swatch), while the plot panel only reads them.
    plotted: Vec<PlottedChannel>,
    plot: PlotPanel,
    numeric: NumericPanel,
    table: TablePanel,
    bus: BusPanel,
    stats: StatsPanel,
    /// One plotted channel against another. Kept beside the plot rather than
    /// inside it: it draws no time axis and answers a different question.
    xy: XyPanel,
    /// Last title sent to the window, so the viewport command is only re-sent
    /// when the file changes, not every frame.
    window_title: String,
    /// The files a batch runs over. Owned here rather than by the panel so it
    /// is saved with the rest of the stored state, and so it outlives whichever
    /// measurement happens to be open.
    batch_queue: BatchQueue,
    batch: BatchPanel,
    /// Whether the batch tab is showing. It replaces the content area rather
    /// than sitting among the per-file tabs: a batch is about files that are
    /// not open, so it has to be reachable with no file open at all.
    show_batch: bool,
}

impl FalconApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        let recent = RecentFiles::load(cc.storage);
        let mut app = Self {
            state: LoadState::Idle,
            second: LoadState::Idle,
            active: FileSlot::A,
            recent,
            sessions: Sessions::load(cc.storage),
            restored: false,
            pending_second: None,
            focus_search: false,
            show_shortcuts: false,
            nav: NavTab::Structure,
            tab: ContentTab::Details,
            selection: Selection::File,
            history: Vec::new(),
            future: Vec::new(),
            last_selection: Selection::File,
            tree: StructureTree::new(),
            block_browser: BlockBrowser::new(),
            browser: ChannelBrowser::new(),
            details: DetailsPanel::new(),
            plotted: Vec::new(),
            plot: PlotPanel::new(),
            numeric: NumericPanel::new(),
            table: TablePanel::new(),
            bus: BusPanel::new(),
            stats: StatsPanel::new(),
            xy: XyPanel::new(),
            window_title: "falcon".to_string(),
            batch_queue: BatchQueue::load(cc.storage),
            batch: BatchPanel::new(),
            show_batch: false,
        };
        if let Some(path) = initial_path {
            app.start_load(path, &cc.egui_ctx);
        }
        app
    }

    /// Stores what is plotted and open for the file currently loaded, so
    /// reopening it later comes back to the same view.
    ///
    /// The session belongs to the first file and names the second, so
    /// reopening one measurement brings back the pair it was being compared
    /// against along with the channels plotted from each.
    fn remember_current(&mut self) {
        let LoadState::Loaded(loaded) = &self.state else {
            return;
        };
        let (cursor_a, cursor_b) = self.plot.cursors();
        self.sessions.insert(
            loaded.path.clone(),
            Session {
                plotted: self.plotted.iter().map(|p| (p.file, p.loc)).collect(),
                nav: self.nav.label().to_string(),
                tab: self.tab.label().to_string(),
                cursor_a,
                cursor_b,
                computed: self.plot.computed_defs().to_vec(),
                second: self.loaded_second().map(|l| l.path.clone()),
                xy: self.xy.axes(),
            },
        );
    }

    fn loaded_second(&self) -> Option<&LoadedFile> {
        match &self.second {
            LoadState::Loaded(loaded) => Some(loaded),
            _ => None,
        }
    }

    fn start_load(&mut self, path: PathBuf, ctx: &egui::Context) {
        // The outgoing file's view is remembered before it is torn down;
        // after this point `self.plotted` belongs to the incoming file.
        self.remember_current();
        // A `ChannelLoc` and a block address are both just numbers, so state
        // left over from the previous file would silently point at whatever
        // sits in the same position in the new one. Every panel that caches
        // anything keyed that way is reset on this seam.
        self.reset_views();
        self.plotted.clear();
        self.plot = PlotPanel::new();
        self.xy.reset();
        // Opening a new first file ends whatever comparison was running:
        // the second file was chosen to be compared against the old one.
        self.second = LoadState::Idle;
        self.active = FileSlot::A;
        self.restored = false;
        self.pending_second = None;
        let rx = spawn_load(path.clone(), ctx.clone());
        self.state = LoadState::Loading { path, rx };
    }

    /// Opens `path` as the comparison file, leaving the first one open.
    fn start_load_second(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.close_second();
        let rx = spawn_load(path.clone(), ctx.clone());
        self.second = LoadState::Loading { path, rx };
    }

    /// Closes the comparison file and drops everything that referred to it.
    /// A plotted channel from a file that is no longer open would otherwise
    /// keep a slot the plot cannot fill.
    fn close_second(&mut self) {
        self.second = LoadState::Idle;
        self.plotted.retain(|p| p.file == FileSlot::A);
        // An axis reading from the file just closed cannot be redrawn, and
        // half an X-Y selection is not one.
        if self.xy.axes().is_some_and(|a| {
            a.x.file == FileSlot::B || a.y.file == FileSlot::B
        }) {
            self.xy.set_axes(None);
        }
        if self.active == FileSlot::B {
            self.select_file(FileSlot::A);
        }
    }

    /// Points the left-hand browser and the detail views at `file`.
    ///
    /// Everything keyed by a location is reset: the same three indices name a
    /// different channel in the other measurement, so a selection carried
    /// across would be about a channel the user never picked.
    fn select_file(&mut self, file: FileSlot) {
        if self.active == file {
            return;
        }
        self.active = file;
        self.reset_views();
        self.history.clear();
        self.future.clear();
    }

    /// Resets every panel that caches something keyed by a location or a
    /// block address, for a change of which file those numbers are about.
    fn reset_views(&mut self) {
        self.selection = Selection::File;
        self.last_selection = Selection::File;
        self.tab = ContentTab::Details;
        self.tree.reset();
        self.block_browser.reset();
        self.browser.reset();
        self.details.reset();
        self.numeric.reset();
        self.table.reset();
        self.bus.reset();
        self.stats.reset();
    }

    fn poll_load(&mut self, ctx: &egui::Context) {
        Self::poll_one(&mut self.state);
        Self::poll_one(&mut self.second);
        // The recent list and the stored session belong to the first file
        // only, and both are applied once, on the frame it finishes loading.
        if let LoadState::Loaded(loaded) = &self.state {
            if !self.restored {
                self.restored = true;
                let path = loaded.path.clone();
                self.recent.push(&path);
                self.restore_session(ctx);
            }
        }
    }

    fn poll_one(state: &mut LoadState) {
        let LoadState::Loading { path, rx } = state else {
            return;
        };
        match rx.try_recv() {
            Ok(LoadResult::Ok(loaded)) => *state = LoadState::Loaded(loaded),
            Ok(LoadResult::Err { path, message }) => *state = LoadState::Failed { path, message },
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Name the file the failure panel is about: an empty path
                // there reads like a bug in the panel, not in the load.
                let path = path.clone();
                *state = LoadState::Failed {
                    path,
                    message: "loader thread ended without a result".to_string(),
                };
            }
        }
    }

    /// Puts back what was plotted and open for this file when it was last
    /// closed. Channels the file no longer has are dropped rather than
    /// restored blindly — the file at a path can be rewritten between runs.
    ///
    /// The remembered second file is reopened here too, and the channels
    /// plotted from it are held back until it finishes loading: its
    /// locations cannot be checked against a file that is not open yet.
    fn restore_session(&mut self, ctx: &egui::Context) {
        let LoadState::Loaded(loaded) = &self.state else {
            return;
        };
        let Some(session) = self.sessions.get(&loaded.path).cloned() else {
            return;
        };
        self.nav = NavTab::from_label(&session.nav);
        self.tab = ContentTab::from_label(&session.tab);
        self.plot.set_cursors(session.cursor_a, session.cursor_b);
        self.plot.set_computed_defs(session.computed.clone());
        for loc in prune_to_file(&session, FileSlot::A, &loaded.file) {
            let name = channel_name(&loaded.file, loc);
            self.plotted
                .push(PlottedChannel::new(FileSlot::A, loc, name, self.plotted.len()));
        }
        if let Some(second) = session.second.clone() {
            // The X-Y axes may name a channel in the second file, so they
            // wait with it: an axis can only be checked against the file it
            // says it is in.
            self.pending_second = Some(session);
            self.start_load_second(second, ctx);
        } else {
            self.xy
                .set_axes(prune_xy(&session, &[(FileSlot::A, &loaded.file)]));
        }
    }

    /// Restores the second file's plotted channels once it has finished
    /// loading, so each location is checked against the file it belongs to.
    fn restore_second_session(&mut self) {
        let Some(session) = self.pending_second.take() else {
            return;
        };
        let LoadState::Loaded(loaded) = &self.second else {
            // Still loading: put the session back and try again next frame.
            // A failed open drops it, which is the right outcome — there is
            // no file to restore channels against.
            if matches!(self.second, LoadState::Loading { .. }) {
                self.pending_second = Some(session);
            }
            return;
        };
        let restored: Vec<(ChannelLoc, String)> = prune_to_file(&session, FileSlot::B, &loaded.file)
            .into_iter()
            .map(|loc| (loc, channel_name(&loaded.file, loc)))
            .collect();
        // Both files are open now, so an X-Y selection naming either can
        // finally be checked.
        let axes = match &self.state {
            LoadState::Loaded(first) => prune_xy(
                &session,
                &[(FileSlot::A, &first.file), (FileSlot::B, &loaded.file)],
            ),
            _ => None,
        };
        for (loc, name) in restored {
            self.plotted
                .push(PlottedChannel::new(FileSlot::B, loc, name, self.plotted.len()));
        }
        self.xy.set_axes(axes);
    }

    /// Keyboard shortcuts, which only fire when no text box has focus — a
    /// viewer where typing "b" into the search box jumps to the block list
    /// is worse than one with no shortcuts at all.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key| {
            ctx.input_mut(|i| i.consume_key(modifiers, key))
        };
        let command = egui::Modifiers::COMMAND;

        if pressed(ctx, command, egui::Key::O) {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("MF4", &["mf4", "MF4"])
                .pick_file()
            {
                self.start_load(path, ctx);
            }
        }
        if pressed(ctx, command, egui::Key::F) {
            self.nav = NavTab::Channels;
            self.focus_search = true;
        }
        for (key, tab) in [
            (egui::Key::Num1, NavTab::Structure),
            (egui::Key::Num2, NavTab::Blocks),
            (egui::Key::Num3, NavTab::Channels),
        ] {
            if pressed(ctx, command, key) {
                self.nav = tab;
            }
        }
        for (index, tab) in ContentTab::ALL.into_iter().enumerate() {
            // Shift+Command+1..5 for the content tabs, so the two rows of
            // tabs are reachable without either shadowing the other.
            let key = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
            ][index];
            if pressed(ctx, command | egui::Modifiers::SHIFT, key) {
                self.tab = tab;
            }
        }
        if pressed(ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft) {
            self.go_back();
        }
        if pressed(ctx, egui::Modifiers::ALT, egui::Key::ArrowRight) {
            self.go_forward();
        }
        if pressed(ctx, egui::Modifiers::NONE, egui::Key::Questionmark)
            || pressed(ctx, egui::Modifiers::SHIFT, egui::Key::Slash)
        {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if self.show_shortcuts && pressed(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
            self.show_shortcuts = false;
        }
    }

    fn go_back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.future.push(self.selection);
            self.selection = previous;
            self.last_selection = previous;
        }
    }

    fn go_forward(&mut self) {
        if let Some(next) = self.future.pop() {
            self.history.push(self.selection);
            self.selection = next;
            self.last_selection = next;
        }
    }

    /// The shortcut list, as a window rather than a page: it is read once and
    /// dismissed, and hiding it behind a tab would make it harder to find
    /// than the shortcuts themselves.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_shortcuts;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("shortcut_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, what) in [
                            ("Cmd/Ctrl + O", "Open a file"),
                            ("Cmd/Ctrl + F", "Search channels"),
                            ("Cmd/Ctrl + 1/2/3", "Structure, Blocks, Channels"),
                            (
                                "Cmd/Ctrl + Shift + 1-7",
                                "Details, Plot, Numeric, Samples, Bus, Statistics, X-Y",
                            ),
                            // Spelled out rather than drawn as arrows: egui's
                            // default font has no glyph for them, and a
                            // shortcut list full of tofu boxes helps nobody.
                            ("Alt + Left / Right", "Back and forward through selections"),
                            ("?", "This list"),
                        ] {
                            ui.strong(keys);
                            ui.label(what);
                            ui.end_row();
                        }
                    });
            });
        self.show_shortcuts = open;
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.into_iter().find_map(|f| f.path) {
            self.start_load(path, ctx);
        }
    }

    /// The window title names the open file, so a screenshot or a window
    /// switcher says which measurement this is — a viewer whose title is
    /// always the app name makes every window look like every other.
    fn update_title(&mut self, ctx: &egui::Context) {
        let desired = match &self.state {
            LoadState::Idle => "falcon".to_string(),
            LoadState::Loading { .. } => "falcon — opening\u{2026}".to_string(),
            LoadState::Failed { .. } => "falcon — open failed".to_string(),
            LoadState::Loaded(loaded) => match self.loaded_second() {
                Some(second) => format!(
                    "falcon \u{2014} {} vs {}",
                    loaded.short_name(),
                    second.short_name()
                ),
                None => format!("falcon \u{2014} {}", loaded.short_name()),
            },
        };
        if desired != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(desired.clone()));
            self.window_title = desired;
        }
    }

    /// Top bar: open/recent-files controls and the loading spinner. Takes
    /// the outer `ui` and claims a strip off its top, same as every other
    /// panel below claims a strip of whatever `ui` is left after this one.
    fn top_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Panel::top("top_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open File\u{2026}").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("MF4", &["mf4", "MF4"])
                        .pick_file()
                    {
                        self.start_load(path, ctx);
                    }
                }

                // The batch tab is about files that are not open, so it is
                // reachable with no file open — which the per-file content
                // tabs below are not.
                if ui
                    .selectable_label(self.show_batch, "Batch")
                    .on_hover_text(
                        "Queue several measurements and apply one operation \
                         to all of them",
                    )
                    .clicked()
                {
                    self.show_batch = !self.show_batch;
                }

                ui.menu_button("Recent Files", |ui| {
                    if self.recent.paths().is_empty() {
                        ui.label("(none)");
                    }
                    for path in self.recent.paths().to_vec() {
                        if ui.button(path.display().to_string()).clicked() {
                            self.start_load(path, ui.ctx());
                            ui.close();
                        }
                    }
                });

                // The comparison file is opened alongside, never instead:
                // this button is the whole difference between an inspector
                // and something you can compare two runs in.
                if matches!(self.state, LoadState::Loaded(_)) {
                    ui.separator();
                    if ui
                        .button("Compare File\u{2026}")
                        .on_hover_text(
                            "Open a second measurement alongside this one and plot channels \
                             from both on shared axes",
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("MF4", &["mf4", "MF4"])
                            .pick_file()
                        {
                            self.start_load_second(path, ctx);
                        }
                    }
                    match &self.second {
                        LoadState::Idle => {}
                        LoadState::Loading { path, .. } => {
                            ui.spinner();
                            ui.label(format!("Opening {}\u{2026}", path.display()));
                        }
                        LoadState::Failed { path, message } => {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 80, 80),
                                format!("B: {} failed to open \u{2014} {message}", path.display()),
                            );
                            if ui.button("Dismiss").clicked() {
                                self.close_second();
                            }
                        }
                        LoadState::Loaded(loaded) => {
                            ui.label(format!("B: {}", loaded.short_name()))
                                .on_hover_text(loaded.path.display().to_string());
                            if ui.button("Close B").clicked() {
                                self.close_second();
                            }
                        }
                    }
                }

                if let LoadState::Loading { path, .. } = &self.state {
                    ui.separator();
                    ui.spinner();
                    ui.label(format!("Opening {}\u{2026}", path.display()));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::widgets::global_theme_preference_switch(ui);
                });
            });
        });
    }

    /// The strip along the bottom, naming what is open and what is selected.
    /// A viewer whose selection is only visible as a highlight somewhere in a
    /// long tree makes the user hunt for where they are.
    fn status_bar(&self, ui: &mut egui::Ui, files: &OpenFiles, loaded: &LoadedFile) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if files.has_second() {
                    ui.strong(format!("{} \u{00b7} {}", self.active.label(), loaded.short_name()));
                    ui.separator();
                }
                ui.weak(format!(
                    "MDF {} \u{00b7} {} bytes \u{00b7} {} blocks \u{00b7} {} channels",
                    loaded.file.version(),
                    loaded.file.file_size(),
                    loaded.blocks.blocks.len(),
                    loaded.file.channel_count()
                ));
                ui.separator();
                ui.weak(describe_selection(loaded, self.selection));
            });
        });
    }

    /// The row that says which file the browser below is about, shown only
    /// when a second one is open. It names both files by their own name, not
    /// just "A" and "B": two runs of the same test have the same channels,
    /// and the badge on a plotted channel is only unambiguous if this row
    /// says what the badge stands for.
    fn file_selector(&mut self, ui: &mut egui::Ui, files: &OpenFiles) {
        let Some(b) = files.b else {
            return;
        };
        let mut chosen = self.active;
        ui.horizontal_wrapped(|ui| {
            ui.label("Browsing:");
            ui.selectable_value(&mut chosen, FileSlot::A, format!("A \u{00b7} {}", files.a.short_name()))
                .on_hover_text(files.a.path.display().to_string());
            ui.selectable_value(&mut chosen, FileSlot::B, format!("B \u{00b7} {}", b.short_name()))
                .on_hover_text(b.path.display().to_string());
        });
        ui.weak("Channels plotted from either file are drawn together; the legend carries the badge.");
        self.select_file(chosen);
    }

    fn left_panel(&mut self, ui: &mut egui::Ui, files: &OpenFiles, loaded: &LoadedFile) {
        egui::Panel::left("nav_panel")
            .resizable(true)
            .default_size(420.0)
            .show(ui, |ui| {
                self.file_selector(ui, files);
                ui.horizontal(|ui| {
                    for tab in NavTab::ALL {
                        // Switching to the tree with a channel selected
                        // elsewhere scrolls to it: a selection the user made
                        // in the search list should not be somewhere off
                        // screen when they come back to the outline.
                        if ui
                            .selectable_value(&mut self.nav, tab, tab.label())
                            .clicked()
                            && tab == NavTab::Structure
                        {
                            if let Some(loc) = selected_channel(self.selection) {
                                self.tree.reveal(loc);
                            }
                        }
                    }
                });
                ui.separator();
                match self.nav {
                    NavTab::Structure => {
                        self.tree
                            .show(ui, loaded, self.active, &mut self.selection, &mut self.plotted)
                    }
                    NavTab::Blocks => self.block_browser.show(ui, loaded, &mut self.selection),
                    NavTab::Channels => {
                        // The shortcut asked for the search box; the box only
                        // exists inside this branch, so the request is handed
                        // over here rather than where the key was read.
                        if std::mem::take(&mut self.focus_search) {
                            self.browser.request_focus();
                        }
                        let mut selected = selected_channel(self.selection);
                        self.browser
                            .show(ui, loaded, self.active, &mut self.plotted, &mut selected);
                        if let Some(loc) = selected {
                            if self.selection != Selection::Channel(loc) {
                                self.selection = Selection::Channel(loc);
                            }
                        }
                    }
                }
            });
    }

    fn content_panel(&mut self, ui: &mut egui::Ui, files: &OpenFiles, loaded: &LoadedFile) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.history.is_empty(), egui::Button::new("\u{25c0}"))
                    .on_hover_text("Back to the previous selection")
                    .clicked()
                {
                    self.go_back();
                }
                if ui
                    .add_enabled(!self.future.is_empty(), egui::Button::new("\u{25b6}"))
                    .on_hover_text("Forward again")
                    .clicked()
                {
                    self.go_forward();
                }
                ui.separator();
                for tab in ContentTab::ALL {
                    ui.selectable_value(&mut self.tab, tab, tab.label());
                }
            });
            ui.separator();

            match self.tab {
                ContentTab::Details => self.details.show(
                    ui,
                    loaded,
                    self.active,
                    &mut self.selection,
                    &mut self.plotted,
                    &mut self.tab,
                ),
                ContentTab::Plot => self.plot.show(ui, files, self.active, &self.plotted),
                ContentTab::Numeric => {
                    if self.plotted.is_empty() {
                        ui.label(
                            "Plot a channel to read its value here at any instant \u{2014} the Numeric view answers what every plotted signal was doing at one time, which a plot with overlapping lines cannot.",
                        );
                    } else {
                        // The same shift the plot applies, so a value read
                        // here at time t is the value the plot draws at t.
                        let offset = self.plot.second_file_offset(files);
                        self.numeric.show(ui, files, offset, &self.plotted);
                    }
                }
                ContentTab::Table => match selected_group(self.selection) {
                    Some((dg, cg)) => self.table.show(ui, &loaded.file, dg, cg),
                    None => {
                        ui.label("Select a channel or a channel group to see its samples.");
                    }
                },
                ContentTab::Bus => match selected_group(self.selection) {
                    Some((dg, cg)) => self.bus.show(ui, &loaded.file, dg, cg),
                    None => {
                        ui.label("Select a bus-logged channel group to see its frames.");
                        let groups = bus_groups(loaded);
                        if groups.is_empty() {
                            ui.weak("This file has no bus-logged groups.");
                        }
                        for (dg, cg, name) in groups {
                            if ui.button(name).clicked() {
                                self.selection = Selection::ChannelGroup {
                                    data_group_index: dg,
                                    channel_group_index: cg,
                                };
                            }
                        }
                    }
                },
                ContentTab::Xy => {
                    // Both taken from the plot panel rather than recomputed,
                    // so the X-Y view can never disagree with the time plot
                    // about where file B sits or where the cursors are.
                    let (cursor_a, cursor_b) = self.plot.cursors();
                    self.xy.show(
                        ui,
                        files,
                        &self.plotted,
                        self.plot.second_file_offset(files),
                        self.plot.alignment_is_absolute(),
                        cursor_a,
                        cursor_b,
                    );
                }
                ContentTab::Statistics => match selected_channel(self.selection) {
                    Some(loc) => self.stats.show(ui, &loaded.file, loc),
                    None => {
                        ui.label("Select a channel to see its statistics.");
                    }
                },
            }
        });
    }
}

/// The name of the channel at `loc`. Only called for locations already
/// checked against this file by `prune_to_file`.
fn channel_name(file: &falcon_mdf::Mf4File, loc: ChannelLoc) -> String {
    file.data_groups()[loc.data_group_index].channel_groups[loc.channel_group_index].channels
        [loc.channel_index]
        .name
        .clone()
}

/// The channel a selection is about, if any. A channel group has no single
/// channel, so the views that need one say so rather than picking.
fn selected_channel(selection: Selection) -> Option<ChannelLoc> {
    match selection {
        Selection::Channel(loc) => Some(loc),
        _ => None,
    }
}

/// The channel group a selection is in — directly, or through the channel it
/// names. This is what the sample table and the bus view are about.
fn selected_group(selection: Selection) -> Option<(usize, usize)> {
    match selection {
        Selection::Channel(loc) => Some((loc.data_group_index, loc.channel_group_index)),
        Selection::ChannelGroup {
            data_group_index,
            channel_group_index,
        } => Some((data_group_index, channel_group_index)),
        _ => None,
    }
}

/// Every bus-logged group in the file, so the Bus tab can offer them rather
/// than leaving the user to find one in the tree.
fn bus_groups(loaded: &LoadedFile) -> Vec<(usize, usize, String)> {
    let mut groups = Vec::new();
    for (dg_index, dg) in loaded.file.data_groups().iter().enumerate() {
        for (cg_index, cg) in dg.channel_groups.iter().enumerate() {
            if !cg.is_bus_event() {
                continue;
            }
            let name = if cg.acquisition_name.is_empty() {
                format!("Data group {dg_index}, channel group {cg_index}")
            } else {
                format!("{} ({} samples)", cg.acquisition_name, cg.sample_count)
            };
            groups.push((dg_index, cg_index, name));
        }
    }
    groups
}

fn describe_selection(loaded: &LoadedFile, selection: Selection) -> String {
    match selection {
        Selection::File => "the file".to_string(),
        Selection::DataGroup(index) => format!("data group {index}"),
        Selection::ChannelGroup {
            data_group_index,
            channel_group_index,
        } => format!("data group {data_group_index}, channel group {channel_group_index}"),
        Selection::Channel(loc) => loaded
            .file
            .data_groups()
            .get(loc.data_group_index)
            .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
            .and_then(|cg| cg.channels.get(loc.channel_index))
            .map(|ch| format!("channel {}", ch.name))
            .unwrap_or_else(|| "a channel that is no longer there".to_string()),
        Selection::Block(address) => match loaded.blocks.block_at(address) {
            Some(block) => format!("{} at {address:#x}", block.block_type),
            None => format!("{address:#x}"),
        },
        Selection::Attachment(index) => format!("attachment {index}"),
        Selection::Event(index) => format!("event {index}"),
        Selection::HistoryEntry(index) => format!("history entry {index}"),
    }
}

impl eframe::App for FalconApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.poll_load(&ctx);
        self.restore_second_session();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.shortcuts_window(&ctx);
        self.update_title(&ctx);
        self.top_panel(ui, &ctx);

        // Drawn instead of everything below, in every load state: a batch runs
        // over the queue, not over whatever is open.
        if self.show_batch {
            egui::CentralPanel::default().show(ui, |ui| {
                self.batch.show(ui, &mut self.batch_queue);
            });
            return;
        }

        match &self.state {
            LoadState::Idle => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("No file open");
                        ui.label("Use \u{201c}Open File\u{2026}\u{201d}, drop an MF4 file onto this window, or pick a recent file.");
                    });
                });
            }
            LoadState::Loading { .. } => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.spinner();
                        ui.label("Opening file\u{2026}");
                    });
                });
            }
            LoadState::Failed { path, message } => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("Failed to open file");
                        ui.label(path.display().to_string());
                        ui.add_space(10.0);
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), message);
                    });
                });
            }
            LoadState::Loaded(_) => {
                // The loaded files are moved out of `self` for the frame:
                // every panel below takes `&LoadedFile` while also taking
                // `&mut self`, which the borrow checker cannot allow while the
                // file is still a field. They are put back before the frame
                // ends.
                let LoadState::Loaded(loaded) = std::mem::replace(&mut self.state, LoadState::Idle)
                else {
                    unreachable!("just matched Loaded");
                };
                let second = match std::mem::replace(&mut self.second, LoadState::Idle) {
                    LoadState::Loaded(second) => Some(second),
                    other => {
                        self.second = other;
                        None
                    }
                };
                let files = OpenFiles {
                    a: &loaded,
                    b: second.as_ref(),
                };
                // The browser is about the active file; when that is the
                // comparison file, that is what the panels below are handed.
                let browsed = files.get(self.active).unwrap_or(&loaded);
                self.status_bar(ui, &files, browsed);
                self.left_panel(ui, &files, browsed);
                self.content_panel(ui, &files, browsed);
                if let Some(second) = second {
                    self.second = LoadState::Loaded(second);
                }
                self.state = LoadState::Loaded(loaded);

                // A selection made anywhere this frame becomes history, and
                // ends whatever forward path was left over: stepping back and
                // then somewhere new is a new branch, not a detour on the old
                // one.
                if self.selection != self.last_selection {
                    self.history.push(self.last_selection);
                    self.future.clear();
                    self.last_selection = self.selection;
                }
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // The open file's view is only in `self` until the window closes, so
        // it is folded into the store on the way out rather than only when
        // another file is opened.
        self.remember_current();
        self.recent.save(storage);
        self.sessions.save(storage);
        self.batch_queue.save(storage);
    }
}
