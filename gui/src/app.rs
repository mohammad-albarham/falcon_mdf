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
use crate::model::{ChannelLoc, ContentTab, LoadedFile, PlottedChannel, Selection};
use crate::panels::blocks::BlockBrowser;
use crate::panels::bus::BusPanel;
use crate::panels::channel_list::ChannelBrowser;
use crate::panels::details::DetailsPanel;
use crate::panels::numeric::NumericPanel;
use crate::panels::plot::PlotPanel;
use crate::panels::stats::StatsPanel;
use crate::panels::table::TablePanel;
use crate::panels::tree::StructureTree;
use crate::recent::RecentFiles;
use crate::session::{prune_to_file, Session, Sessions};

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
    recent: RecentFiles,
    /// What was plotted and open the last time each file was closed.
    sessions: Sessions,
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
    /// Last title sent to the window, so the viewport command is only re-sent
    /// when the file changes, not every frame.
    window_title: String,
}

impl FalconApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        let recent = RecentFiles::load(cc.storage);
        let mut app = Self {
            state: LoadState::Idle,
            recent,
            sessions: Sessions::load(cc.storage),
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
            window_title: "falcon".to_string(),
        };
        if let Some(path) = initial_path {
            app.start_load(path, &cc.egui_ctx);
        }
        app
    }

    /// Stores what is plotted and open for the file currently loaded, so
    /// reopening it later comes back to the same view.
    fn remember_current(&mut self) {
        let LoadState::Loaded(loaded) = &self.state else {
            return;
        };
        self.sessions.insert(
            loaded.path.clone(),
            Session {
                plotted: self.plotted.iter().map(|p| p.loc).collect(),
                nav: self.nav.label().to_string(),
                tab: self.tab.label().to_string(),
            },
        );
    }

    fn start_load(&mut self, path: PathBuf, ctx: &egui::Context) {
        // The outgoing file's view is remembered before it is torn down;
        // after this point `self.plotted` belongs to the incoming file.
        self.remember_current();
        // A `ChannelLoc` and a block address are both just numbers, so state
        // left over from the previous file would silently point at whatever
        // sits in the same position in the new one. Every panel that caches
        // anything keyed that way is reset on this seam.
        self.selection = Selection::File;
        self.last_selection = Selection::File;
        self.tab = ContentTab::Details;
        self.tree.reset();
        self.block_browser.reset();
        self.browser.reset();
        self.details.reset();
        self.plotted.clear();
        self.plot = PlotPanel::new();
        self.numeric.reset();
        self.table.reset();
        self.bus.reset();
        self.stats.reset();
        let rx = spawn_load(path.clone(), ctx.clone());
        self.state = LoadState::Loading { path, rx };
    }

    fn poll_load(&mut self) {
        let LoadState::Loading { path, rx } = &self.state else {
            return;
        };
        match rx.try_recv() {
            Ok(LoadResult::Ok(loaded)) => {
                self.recent.push(&loaded.path);
                self.restore_session(&loaded);
                self.state = LoadState::Loaded(loaded);
            }
            Ok(LoadResult::Err { path, message }) => {
                self.state = LoadState::Failed { path, message };
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Name the file the failure panel is about: an empty path
                // there reads like a bug in the panel, not in the load.
                let path = path.clone();
                self.state = LoadState::Failed {
                    path,
                    message: "loader thread ended without a result".to_string(),
                };
            }
        }
    }

    /// Puts back what was plotted and open for this file when it was last
    /// closed. Channels the file no longer has are dropped rather than
    /// restored blindly — the file at a path can be rewritten between runs.
    fn restore_session(&mut self, loaded: &LoadedFile) {
        let Some(session) = self.sessions.get(&loaded.path) else {
            return;
        };
        self.nav = NavTab::from_label(&session.nav);
        self.tab = ContentTab::from_label(&session.tab);
        for loc in prune_to_file(session, &loaded.file) {
            let name = loaded.file.data_groups()[loc.data_group_index].channel_groups
                [loc.channel_group_index]
                .channels[loc.channel_index]
                .name
                .clone();
            self.plotted
                .push(PlottedChannel::new(loc, name, self.plotted.len()));
        }
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
                                "Cmd/Ctrl + Shift + 1-6",
                                "Details, Plot, Numeric, Samples, Bus, Statistics",
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
            LoadState::Loaded(loaded) => {
                let name = loaded
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| loaded.path.display().to_string());
                format!("falcon — {name}")
            }
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
    fn status_bar(&self, ui: &mut egui::Ui, loaded: &LoadedFile) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
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

    fn left_panel(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile) {
        egui::Panel::left("nav_panel")
            .resizable(true)
            .default_size(420.0)
            .show(ui, |ui| {
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
                            .show(ui, loaded, &mut self.selection, &mut self.plotted)
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
                            .show(ui, loaded, &mut self.plotted, &mut selected);
                        if let Some(loc) = selected {
                            if self.selection != Selection::Channel(loc) {
                                self.selection = Selection::Channel(loc);
                            }
                        }
                    }
                }
            });
    }

    fn content_panel(&mut self, ui: &mut egui::Ui, loaded: &LoadedFile) {
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
                    &mut self.selection,
                    &mut self.plotted,
                    &mut self.tab,
                ),
                ContentTab::Plot => self.plot.show(ui, loaded, &self.plotted),
                ContentTab::Numeric => {
                    if self.plotted.is_empty() {
                        ui.label(
                            "Plot a channel to read its value here at any instant \u{2014} the Numeric view answers what every plotted signal was doing at one time, which a plot with overlapping lines cannot.",
                        );
                    } else {
                        self.numeric.show(ui, &loaded.file, &self.plotted);
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

        self.poll_load();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.shortcuts_window(&ctx);
        self.update_title(&ctx);
        self.top_panel(ui, &ctx);

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
                // The loaded file is moved out of `self.state` for the frame:
                // every panel below takes `&LoadedFile` while also taking
                // `&mut self`, which the borrow checker cannot allow while the
                // file is still a field. It is put back before the frame ends.
                let LoadState::Loaded(loaded) = std::mem::replace(&mut self.state, LoadState::Idle)
                else {
                    unreachable!("just matched Loaded");
                };
                self.status_bar(ui, &loaded);
                self.left_panel(ui, &loaded);
                self.content_panel(ui, &loaded);
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
    }
}
