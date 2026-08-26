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

/// How two files are aligned in time when comparison mode is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeAlignMode {
    /// Each file starts at its own t = 0.0s (relative start alignment).
    #[default]
    RelativeZero,
    /// Files are aligned by header UTC timestamps (`file.start_time().timestamp_ns`).
    AbsoluteUtc,
}

impl TimeAlignMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TimeAlignMode::RelativeZero => "rel_zero",
            TimeAlignMode::AbsoluteUtc => "abs_utc",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "abs_utc" => TimeAlignMode::AbsoluteUtc,
            _ => TimeAlignMode::RelativeZero,
        }
    }
}

/// The state remembered for one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    /// The channels that were plotted from the primary file (file_index = 0).
    pub plotted: Vec<ChannelLoc>,
    /// Which left-hand tab was showing, by its label.
    pub nav: String,
    /// Which content tab was showing, by its label.
    pub tab: String,
    /// Time position of measurement cursor A, if placed.
    pub cursor_a: Option<f64>,
    /// Time position of measurement cursor B, if placed.
    pub cursor_b: Option<f64>,
    /// Computed channels defined for this file.
    pub computed: Vec<crate::computed::ComputedDef>,
    /// Path to a second file opened for comparison, if any.
    pub second_path: Option<PathBuf>,
    /// Channels plotted from the second file (file_index = 1).
    pub second_plotted: Vec<ChannelLoc>,
    /// Time alignment mode when comparison is active.
    pub align_mode: TimeAlignMode,
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
        session.second_plotted.truncate(MAX_PLOTTED);
        self.order.retain(|p| p != &path);
        self.order.push(path.clone());
        self.files.insert(path, session);
        while self.order.len() > MAX_FILES {
            let oldest = self.order.remove(0);
            self.files.remove(&oldest);
        }
    }
}

/// Encodes a list of [`ChannelLoc`] into a comma-separated string `dg:cg:ch`.
fn encode_locs(locs: &[ChannelLoc]) -> String {
    locs.iter()
        .map(|loc| {
            format!(
                "{}:{}:{}",
                loc.data_group_index, loc.channel_group_index, loc.channel_index
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Parses a comma-separated list of `dg:cg:ch` with the given `file_index`.
fn parse_locs(s: &str, file_index: usize) -> Option<Vec<ChannelLoc>> {
    let mut plotted = Vec::new();
    if !s.is_empty() {
        for entry in s.split(',') {
            let mut parts = entry.split(':');
            let dg = parts.next()?.parse().ok()?;
            let cg = parts.next()?.parse().ok()?;
            let ch = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            plotted.push(ChannelLoc {
                file_index,
                data_group_index: dg,
                channel_group_index: cg,
                channel_index: ch,
            });
        }
    }
    plotted.truncate(MAX_PLOTTED);
    Some(plotted)
}

/// Encodes a list of [`crate::computed::ComputedDef`] into a single string for storage.
pub fn encode_computed_defs(defs: &[crate::computed::ComputedDef]) -> String {
    defs.iter()
        .map(|d| {
            let enc = |s: &str| {
                s.replace('\\', "\\\\")
                    .replace('\t', " ")
                    .replace(';', "\\;")
                    .replace('=', "\\=")
                    .replace('&', "\\&")
            };
            format!("{}={}&{}", enc(&d.name), enc(&d.expression), enc(&d.unit))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Decodes a string produced by [`encode_computed_defs`] back into [`crate::computed::ComputedDef`]s.
pub fn decode_computed_defs(s: &str) -> Vec<crate::computed::ComputedDef> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut defs = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    let mut items = Vec::new();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else if c == ';' {
            items.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        items.push(current);
    }

    for item in items {
        if let Some((name, rest)) = item.split_once('=') {
            let (expr, unit) = match rest.split_once('&') {
                Some((e, u)) => (e.to_string(), u.to_string()),
                None => (rest.to_string(), String::new()),
            };
            let name_t = name.trim().to_string();
            let expr_t = expr.trim().to_string();
            if !name_t.is_empty() && !expr_t.is_empty() {
                defs.push(crate::computed::ComputedDef {
                    name: name_t,
                    expression: expr_t,
                    unit: unit.trim().to_string(),
                });
            }
        }
    }
    defs
}

/// One stored line: the path, the plotted channels, and the two tab labels,
/// tab-separated. Paths cannot contain a tab on the platforms this runs on,
/// and a path that somehow does is dropped by [`parse_line`] rather than
/// misread.
pub fn format_line(path: &Path, session: &Session) -> String {
    let plotted = encode_locs(&session.plotted);
    let mut line = format!(
        "{}\t{}\t{}\t{}",
        path.display(),
        plotted,
        session.nav,
        session.tab
    );
    if session.cursor_a.is_some()
        || session.cursor_b.is_some()
        || !session.computed.is_empty()
        || session.second_path.is_some()
        || !session.second_plotted.is_empty()
        || session.align_mode != TimeAlignMode::RelativeZero
    {
        let a = session.cursor_a.map(|v| v.to_string()).unwrap_or_default();
        let b = session.cursor_b.map(|v| v.to_string()).unwrap_or_default();
        line.push('\t');
        line.push_str(&a);
        line.push('\t');
        line.push_str(&b);
    }
    if !session.computed.is_empty()
        || session.second_path.is_some()
        || !session.second_plotted.is_empty()
        || session.align_mode != TimeAlignMode::RelativeZero
    {
        line.push('\t');
        line.push_str(&encode_computed_defs(&session.computed));
    }
    if session.second_path.is_some()
        || !session.second_plotted.is_empty()
        || session.align_mode != TimeAlignMode::RelativeZero
    {
        let second = session
            .second_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        line.push('\t');
        line.push_str(&second);
        line.push('\t');
        line.push_str(&encode_locs(&session.second_plotted));
        line.push('\t');
        line.push_str(session.align_mode.as_str());
    }
    line
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
    let cursor_a = fields
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|x| x.is_finite());
    let cursor_b = fields
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|x| x.is_finite());
    let computed = fields
        .next()
        .map(decode_computed_defs)
        .unwrap_or_default();
    let second_path = fields
        .next()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let second_plotted = match fields.next() {
        Some(s) => parse_locs(s, 1)?,
        None => Vec::new(),
    };
    let align_mode = fields
        .next()
        .map(TimeAlignMode::from_str)
        .unwrap_or_default();

    let plotted = parse_locs(plotted_field, 0)?;

    Some((
        PathBuf::from(path),
        Session {
            plotted,
            nav,
            tab,
            cursor_a,
            cursor_b,
            computed,
            second_path,
            second_plotted,
            align_mode,
        },
    ))
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
        .filter(|loc| loc.file_index == 0)
        .filter(|loc| {
            file.data_groups()
                .get(loc.data_group_index)
                .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
                .is_some_and(|cg| loc.channel_index < cg.channels.len())
        })
        .collect()
}

pub fn prune_second_to_file(session: &Session, file: &falcon_mdf::Mf4File) -> Vec<ChannelLoc> {
    session
        .second_plotted
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
