//! The structure tree: the whole file as one outline, from the
//! identification block down to a single channel.
//!
//! Where the block list shows the file as bytes, this shows it as the format
//! means it: the header and its chains — history, attachments, events,
//! hierarchy — and then the data groups, their channel groups, and their
//! channels. Everything in it selects, so the content area on the right is
//! always about whatever was last clicked here.

use falcon_mdf::Mf4File;

use crate::model::{ChannelLoc, LoadedFile, PlottedChannel, Selection};

/// Channels drawn under one group before the tree stops and points at the
/// channel list instead. A group with ten thousand channels would otherwise
/// build ten thousand widgets the moment it is opened, and scrolling a tree
/// that deep is a worse way to find a channel than searching for it.
const MAX_TREE_CHANNELS: usize = 400;

/// Returns true if a channel matches `query` against its name or unit.
/// Matching is case-insensitive, ignores leading/trailing whitespace in `query`,
/// and an empty query matches everything.
pub fn channel_matches(name: &str, unit: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    name.to_lowercase().contains(&q) || unit.to_lowercase().contains(&q)
}

/// Returns true if a channel group matches `query` on its own name or on any
/// of its constituent channels `(name, unit)`.
pub fn group_matches(group_name: &str, channel_names: &[(String, String)], query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    if group_name.to_lowercase().contains(&q) {
        return true;
    }
    channel_names
        .iter()
        .any(|(name, unit)| channel_matches(name, unit, &q))
}

/// The structure tree. Open/closed state lives in egui's own memory, keyed
/// by the ids below, so it survives across frames without being tracked here.
#[derive(Default)]
pub struct StructureTree {
    /// Set when a selection made elsewhere should be scrolled to here. Only
    /// channels are worth chasing: they are the deep nodes.
    scroll_to: Option<ChannelLoc>,
    /// Search filter string narrowing data groups and channel groups.
    filter: String,
    /// Override state to force expand or collapse on all group headers this frame.
    forced_open: Option<bool>,
    /// Status message from the last "Plot all" action.
    plot_message: Option<String>,
}

impl StructureTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.scroll_to = None;
        self.filter.clear();
        self.forced_open = None;
        self.plot_message = None;
    }

    /// Asks the tree to scroll to `loc` on the next frame, used when the
    /// selection was made in another panel.
    pub fn reveal(&mut self, loc: ChannelLoc) {
        self.scroll_to = Some(loc);
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        selection: &mut Selection,
        plotted: &mut Vec<PlottedChannel>,
    ) {
        let file = &loaded.file;

        ui.horizontal(|ui| {
            ui.label("\u{1f50d}");
            ui.text_edit_singleline(&mut self.filter);
            if !self.filter.is_empty()
                && ui
                    .small_button("\u{2a2f}")
                    .on_hover_text("Clear filter")
                    .clicked()
            {
                self.filter.clear();
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("Expand all").clicked() {
                self.forced_open = Some(true);
            }
            if ui.small_button("Collapse all").clicked() {
                self.forced_open = Some(false);
            }
            if !self.filter.trim().is_empty() {
                ui.weak("(filtered)");
            }
        });
        let mut clear_message = false;
        if let Some(msg) = &self.plot_message {
            ui.horizontal(|ui| {
                ui.weak(msg);
                if ui.small_button("\u{2a2f}").clicked() {
                    clear_message = true;
                }
            });
        }
        if clear_message {
            self.plot_message = None;
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let name = loaded
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| loaded.path.display().to_string());

                if ui
                    .selectable_label(
                        *selection == Selection::File,
                        egui::RichText::new(format!("\u{1f5ce} {name}")).strong(),
                    )
                    .clicked()
                {
                    *selection = Selection::File;
                }
                ui.weak(format!(
                    "   MDF {} \u{00b7} {} blocks",
                    file.version(),
                    loaded.blocks.blocks.len()
                ));

                // The two blocks at fixed addresses are named here rather
                // than left to the block list: they are the file's front
                // door, and the tree is where a reader starts.
                block_row(ui, loaded, 0, "Identification block", selection);
                block_row(ui, loaded, 64, "Header block", selection);

                self.show_history(ui, file, selection);
                self.show_attachments(ui, file, selection);
                self.show_events(ui, file, selection);
                self.show_hierarchy(ui, file, selection, plotted);
                self.show_data_groups(ui, loaded, selection, plotted);
            });

        self.forced_open = None;
    }

    fn show_history(&self, ui: &mut egui::Ui, file: &Mf4File, selection: &mut Selection) {
        let history = file.file_history();
        egui::CollapsingHeader::new(format!("\u{1f552} File history ({})", history.len()))
            .id_salt("tree_history")
            .show(ui, |ui| {
                for (index, entry) in history.iter().enumerate() {
                    let tool = [entry.tool_vendor(), entry.tool_id(), entry.tool_version()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let label = if tool.is_empty() {
                        entry.time.to_iso8601()
                    } else {
                        format!("{} \u{2014} {tool}", entry.time.to_iso8601())
                    };
                    if ui
                        .selectable_label(*selection == Selection::HistoryEntry(index), label)
                        .clicked()
                    {
                        *selection = Selection::HistoryEntry(index);
                    }
                }
            });
    }

    fn show_attachments(&self, ui: &mut egui::Ui, file: &Mf4File, selection: &mut Selection) {
        let attachments = file.attachments();
        egui::CollapsingHeader::new(format!("\u{1f4ce} Attachments ({})", attachments.len()))
            .id_salt("tree_attachments")
            .show(ui, |ui| {
                for (index, attachment) in attachments.iter().enumerate() {
                    let label = format!(
                        "{} ({})",
                        attachment.file_name,
                        if attachment.is_embedded {
                            "embedded"
                        } else {
                            "external"
                        }
                    );
                    if ui
                        .selectable_label(*selection == Selection::Attachment(index), label)
                        .clicked()
                    {
                        *selection = Selection::Attachment(index);
                    }
                }
            });
    }

    fn show_events(&self, ui: &mut egui::Ui, file: &Mf4File, selection: &mut Selection) {
        let events = file.events();
        egui::CollapsingHeader::new(format!("\u{2691} Events ({})", events.len()))
            .id_salt("tree_events")
            .show(ui, |ui| {
                for (index, event) in events.iter().enumerate() {
                    let name = if event.name.is_empty() {
                        format!("{:?}", event.event_type)
                    } else {
                        event.name.clone()
                    };
                    let label = format!("{name} @ {:.6}", event.position());
                    if ui
                        .selectable_label(*selection == Selection::Event(index), label)
                        .clicked()
                    {
                        *selection = Selection::Event(index);
                    }
                }
            });
    }

    fn show_hierarchy(
        &self,
        ui: &mut egui::Ui,
        file: &Mf4File,
        selection: &mut Selection,
        plotted: &mut Vec<PlottedChannel>,
    ) {
        let nodes = file.channel_hierarchy();
        egui::CollapsingHeader::new(format!("\u{1f5c2} Channel hierarchy ({})", nodes.len()))
            .id_salt("tree_hierarchy")
            .show(ui, |ui| {
                if nodes.is_empty() {
                    ui.weak("This file declares no hierarchy.");
                }
                for (index, node) in nodes.iter().enumerate() {
                    show_hierarchy_node(ui, file, node, index, selection, plotted);
                }
            });
    }

    fn show_data_groups(
        &mut self,
        ui: &mut egui::Ui,
        loaded: &LoadedFile,
        selection: &mut Selection,
        plotted: &mut Vec<PlottedChannel>,
    ) {
        let file = &loaded.file;
        let query = self.filter.trim().to_string();
        let is_filtering = !query.is_empty();
        let forced_open = self.forced_open;
        let scroll_to = &mut self.scroll_to;
        let mut new_plot_message = None;

        let mut data_groups_header = egui::CollapsingHeader::new(format!(
            "\u{1f4c1} Data groups ({})",
            file.data_groups().len()
        ))
        .id_salt("tree_data_groups")
        .default_open(true);
        if let Some(open) = forced_open {
            data_groups_header = data_groups_header.open(Some(open));
        } else if is_filtering {
            data_groups_header = data_groups_header.open(Some(true));
        }

        data_groups_header.show(ui, |ui| {
            for (dg_index, dg) in file.data_groups().iter().enumerate() {
                let matching_cgs: Vec<(usize, &falcon_mdf::ChannelGroup)> = dg
                    .channel_groups
                    .iter()
                    .enumerate()
                    .filter(|(_, cg)| {
                        if !is_filtering {
                            return true;
                        }
                        let ch_pairs: Vec<(String, String)> = cg
                            .channels
                            .iter()
                            .map(|ch| (ch.name.clone(), ch.unit.clone()))
                            .collect();
                        group_matches(&cg.acquisition_name, &ch_pairs, &query)
                    })
                    .collect();

                if is_filtering && matching_cgs.is_empty() {
                    continue;
                }

                let sorted = if dg.is_unsorted() {
                    "unsorted"
                } else {
                    "sorted"
                };
                let header = format!(
                    "Data group {dg_index} \u{2014} {} group{}, {sorted}",
                    dg.channel_groups.len(),
                    if dg.channel_groups.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                );

                let id = ui.make_persistent_id(("tree_dg", dg_index));
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    file.data_groups().len() <= 4,
                );
                if let Some(open) = forced_open {
                    state.set_open(open);
                } else if is_filtering {
                    state.set_open(true);
                }

                state
                    .show_header(ui, |ui| {
                        ui.label(header);
                    })
                    .body(|ui| {
                        if ui
                            .selectable_label(
                                *selection == Selection::DataGroup(dg_index),
                                "\u{2139} group details",
                            )
                            .clicked()
                        {
                            *selection = Selection::DataGroup(dg_index);
                        }
                        for (cg_index, cg) in matching_cgs {
                            let name = if cg.acquisition_name.is_empty() {
                                format!("Channel group {cg_index}")
                            } else {
                                format!("Channel group {cg_index} \u{2014} {}", cg.acquisition_name)
                            };
                            let marker = if cg.is_bus_event() {
                                " \u{1f68c}"
                            } else if cg.is_vlsd() {
                                " \u{2261}"
                            } else {
                                ""
                            };

                            let cg_id = ui.make_persistent_id(("tree_cg", dg_index, cg_index));
                            let mut cg_state =
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    cg_id,
                                    false,
                                );
                            if let Some(open) = forced_open {
                                cg_state.set_open(open);
                            } else if is_filtering {
                                cg_state.set_open(true);
                            }

                            cg_state
                                .show_header(ui, |ui| {
                                    ui.label(format!(
                                        "{name}{marker} ({} samples, {} channels)",
                                        cg.sample_count,
                                        cg.channels.len()
                                    ));
                                    if ui
                                        .small_button("Plot all")
                                        .on_hover_text(
                                            "Plot readable channels in this group (up to 16)",
                                        )
                                        .clicked()
                                    {
                                        let mut added = 0;
                                        let mut skipped = 0;
                                        for (ch_i, ch) in cg.channels.iter().enumerate() {
                                            if ch.is_master() || ch.unreadable().is_some() {
                                                skipped += 1;
                                                continue;
                                            }
                                            let loc = ChannelLoc {
                                                data_group_index: dg_index,
                                                channel_group_index: cg_index,
                                                channel_index: ch_i,
                                            };
                                            if plotted.iter().any(|p| p.loc == loc) {
                                                skipped += 1;
                                                continue;
                                            }
                                            if added >= 16 {
                                                skipped += 1;
                                                continue;
                                            }
                                            plotted.push(PlottedChannel::new(
                                                loc,
                                                ch.name.clone(),
                                                plotted.len(),
                                            ));
                                            added += 1;
                                        }
                                        new_plot_message = Some(format!(
                                            "Group {cg_index}: added {added} channel(s) to plot, skipped {skipped}"
                                        ));
                                    }
                                })
                                .body(|ui| {
                                    let group_selection = Selection::ChannelGroup {
                                        data_group_index: dg_index,
                                        channel_group_index: cg_index,
                                    };
                                    if ui
                                        .selectable_label(
                                            *selection == group_selection,
                                            "\u{2139} group details",
                                        )
                                        .clicked()
                                    {
                                        *selection = group_selection;
                                    }
                                    Self::show_channels_impl(
                                        ui, scroll_to, dg_index, cg_index, cg, &query, selection, plotted,
                                    );
                                });
                        }
                    });
            }
        });

        if new_plot_message.is_some() {
            self.plot_message = new_plot_message;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_channels_impl(
        ui: &mut egui::Ui,
        scroll_to: &mut Option<ChannelLoc>,
        dg_index: usize,
        cg_index: usize,
        cg: &falcon_mdf::ChannelGroup,
        query: &str,
        selection: &mut Selection,
        plotted: &mut Vec<PlottedChannel>,
    ) {
        let is_filtering = !query.is_empty();
        let group_name_matches = is_filtering
            && cg
                .acquisition_name
                .to_lowercase()
                .contains(&query.to_lowercase());
        let channels_to_show: Vec<(usize, &falcon_mdf::Channel)> = cg
            .channels
            .iter()
            .enumerate()
            .filter(|(_, ch)| {
                if !is_filtering || group_name_matches {
                    true
                } else {
                    channel_matches(&ch.name, &ch.unit, query)
                }
            })
            .collect();

        for &(ch_index, ch) in channels_to_show.iter().take(MAX_TREE_CHANNELS) {
            let loc = ChannelLoc {
                data_group_index: dg_index,
                channel_group_index: cg_index,
                channel_index: ch_index,
            };
            let response = ui
                .horizontal(|ui| {
                    let plotted_index = plotted.iter().position(|p| p.loc == loc);
                    let mut is_plotted = plotted_index.is_some();
                    if ui.checkbox(&mut is_plotted, "").changed() {
                        match plotted_index {
                            Some(i) => {
                                plotted.remove(i);
                            }
                            None => plotted.push(PlottedChannel::new(
                                loc,
                                ch.name.clone(),
                                plotted.len(),
                            )),
                        }
                    }
                    if let Some(i) = plotted_index {
                        ui.colored_label(plotted[i].color, "\u{25cf}");
                    }
                    let mut label = if ch.unit.is_empty() {
                        ch.name.clone()
                    } else {
                        format!("{} [{}]", ch.name, ch.unit)
                    };
                    if ch.is_master() {
                        label.push_str("  (master)");
                    }
                    if ch.is_array() {
                        label.push_str("  \u{25a6}");
                    }
                    if ch.unreadable().is_some() {
                        label.push_str("  \u{26a0}");
                    }
                    let response =
                        ui.selectable_label(*selection == Selection::Channel(loc), label);
                    match ch.unreadable() {
                        Some(reason) => response.on_hover_text(reason.to_string()),
                        None => response,
                    }
                })
                .inner;
            if response.clicked() {
                *selection = Selection::Channel(loc);
            }
            if *scroll_to == Some(loc) {
                response.scroll_to_me(Some(egui::Align::Center));
                *scroll_to = None;
            }
        }
        if channels_to_show.len() > MAX_TREE_CHANNELS {
            ui.weak(format!(
                "\u{2026} and {} more \u{2014} use the Channels tab to search them",
                channels_to_show.len() - MAX_TREE_CHANNELS
            ));
        }
        if !cg.sample_reductions().is_empty() {
            egui::CollapsingHeader::new(format!(
                "Sample reduction ({})",
                cg.sample_reductions().len()
            ))
            .id_salt(("tree_sr", dg_index, cg_index))
            .show(ui, |ui| {
                for reduction in cg.sample_reductions() {
                    ui.weak(format!(
                        "{} cycles every {} ({:?})",
                        reduction.cycle_count, reduction.interval, reduction.sync_type
                    ));
                }
            });
        }
    }
}

/// A row naming a block at a fixed address, used for the two blocks the
/// format puts in known places.
fn block_row(
    ui: &mut egui::Ui,
    loaded: &LoadedFile,
    address: u64,
    label: &str,
    selection: &mut Selection,
) {
    let Some(block) = loaded.blocks.block_at(address) else {
        return;
    };
    if ui
        .selectable_label(
            *selection == Selection::Block(address),
            format!("\u{25aa} {label} ({})", block.block_type),
        )
        .clicked()
    {
        *selection = Selection::Block(address);
    }
}

fn show_hierarchy_node(
    ui: &mut egui::Ui,
    file: &Mf4File,
    node: &falcon_mdf::ChannelHierarchyNode,
    index: usize,
    selection: &mut Selection,
    plotted: &mut Vec<PlottedChannel>,
) {
    egui::CollapsingHeader::new(if node.name.is_empty() {
        format!("node {index}")
    } else {
        node.name.clone()
    })
    .id_salt(("tree_ch_node", index, node.name.as_str()))
    .show(ui, |ui| {
        for element in &node.elements {
            let Some(channel) = file.channel_at(element) else {
                ui.weak("(a channel this node names is not in the file)");
                continue;
            };
            let loc = ChannelLoc {
                data_group_index: channel.data_group_index,
                channel_group_index: channel.channel_group_index,
                channel_index: channel.index,
            };
            if ui
                .selectable_label(*selection == Selection::Channel(loc), &channel.name)
                .clicked()
            {
                *selection = Selection::Channel(loc);
                if !plotted.iter().any(|p| p.loc == loc) {
                    plotted.push(PlottedChannel::new(
                        loc,
                        channel.name.clone(),
                        plotted.len(),
                    ));
                }
            }
        }
        for (child_index, child) in node.children.iter().enumerate() {
            show_hierarchy_node(
                ui,
                file,
                child,
                index * 100 + child_index + 1,
                selection,
                plotted,
            );
        }
    });
}
