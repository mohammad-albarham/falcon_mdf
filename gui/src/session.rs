//! What the viewer remembers about a file between runs.
//!
//! Reopening a measurement and finding an empty plot is the small tax every
//! viewer charges for closing it; this is the refund. Per file path, the
//! channels that were plotted and the two panes that were open come back.
//!
//! Stored as one line per file under a single storage key, the same
//! hand-rolled arrangement [`crate::recent`] uses and for the same reason:
//! `serde` is a large dependency to add for a handful of integers. The
//! parsing is a free function so it can be tested without a window — see
//! `gui/tests/session_roundtrip.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::ChannelLoc;

const STORAGE_KEY: &str = "file_sessions";

/// How many files are remembered. Beyond this the least recently saved is
/// dropped: the list is a convenience, not an archive.
const MAX_FILES: usize = 20;

/// Channels remembered for one file. A file with hundreds of plotted
/// channels is a plot nobody can read, and restoring it would make reopening
/// slow for no gain, so the list is capped where the plot stops being useful.
const MAX_PLOTTED: usize = 32;

/// The state remembered for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    /// The channels that were plotted, in the order they were added — which
    /// is the order that decides their colours.
    pub plotted: Vec<ChannelLoc>,
    /// Which left-hand tab was showing, by its label.
    pub nav: String,
    /// Which content tab was showing, by its label.
    pub tab: String,
}

/// Every remembered file, keyed by path.
#[derive(Default)]
pub struct Sessions {
    files: HashMap<PathBuf, Session>,
    /// Paths in save order, most recent last, so the cap drops the oldest.
    order: Vec<PathBuf>,
}

impl Sessions {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut sessions = Self::default();
        let Some(text) = storage.and_then(|s| s.get_string(STORAGE_KEY)) else {
            return sessions;
        };
        for line in text.lines() {
            // A line this version cannot read is skipped rather than
            // discarding the whole store: a stored session is a convenience,
            // and losing every file's because one line is malformed would be
            // a worse bargain than losing that one.
            if let Some((path, session)) = parse_line(line) {
                sessions.insert(path, session);
            }
        }
        sessions
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let text = self
            .order
            .iter()
            .filter_map(|path| Some((path, self.files.get(path)?)))
            .map(|(path, session)| format_line(path, session))
            .collect::<Vec<_>>()
            .join("\n");
        storage.set_string(STORAGE_KEY, text);
    }

    /// Returns what was remembered for `path`, if anything.
    pub fn get(&self, path: &Path) -> Option<&Session> {
        self.files.get(path)
    }

    /// Remembers `session` for `path`, replacing anything held for it.
    pub fn insert(&mut self, path: PathBuf, mut session: Session) {
        session.plotted.truncate(MAX_PLOTTED);
        self.order.retain(|p| p != &path);
        self.order.push(path.clone());
        self.files.insert(path, session);
        while self.order.len() > MAX_FILES {
            let oldest = self.order.remove(0);
            self.files.remove(&oldest);
        }
    }
}

/// One stored line: the path, the plotted channels, and the two tab labels,
/// tab-separated. Paths cannot contain a tab on the platforms this runs on,
/// and a path that somehow does is dropped by [`parse_line`] rather than
/// misread.
pub fn format_line(path: &Path, session: &Session) -> String {
    let plotted = session
        .plotted
        .iter()
        .map(|loc| {
            format!(
                "{}:{}:{}",
                loc.data_group_index, loc.channel_group_index, loc.channel_index
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{}\t{}\t{}\t{}",
        path.display(),
        plotted,
        session.nav,
        session.tab
    )
}

/// Reads a line written by [`format_line`]. Returns `None` for anything that
/// does not parse, which is how a store written by a different version is
/// survived.
pub fn parse_line(line: &str) -> Option<(PathBuf, Session)> {
    let mut fields = line.split('\t');
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    let plotted_field = fields.next()?;
    let nav = fields.next().unwrap_or_default().to_string();
    let tab = fields.next().unwrap_or_default().to_string();

    let mut plotted = Vec::new();
    if !plotted_field.is_empty() {
        for entry in plotted_field.split(',') {
            let mut parts = entry.split(':');
            let dg = parts.next()?.parse().ok()?;
            let cg = parts.next()?.parse().ok()?;
            let ch = parts.next()?.parse().ok()?;
            // A fourth part means the entry was written by something this
            // version does not understand; refusing the line is safer than
            // guessing which three of four numbers were meant.
            if parts.next().is_some() {
                return None;
            }
            plotted.push(ChannelLoc {
                data_group_index: dg,
                channel_group_index: cg,
                channel_index: ch,
            });
        }
    }
    plotted.truncate(MAX_PLOTTED);

    Some((PathBuf::from(path), Session { plotted, nav, tab }))
}

/// Drops remembered channels that the file no longer has.
///
/// A session is keyed by path, and the file at that path can be rewritten
/// between runs — a shorter recording, a different set of groups. Restoring
/// a location that no longer exists would index past the end of a group, so
/// what is restored is checked against the file first.
pub fn prune_to_file(session: &Session, file: &falcon_mdf::Mf4File) -> Vec<ChannelLoc> {
    session
        .plotted
        .iter()
        .copied()
        .filter(|loc| {
            file.data_groups()
                .get(loc.data_group_index)
                .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
                .is_some_and(|cg| loc.channel_index < cg.channels.len())
        })
        .collect()
}
