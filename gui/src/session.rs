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

use crate::model::{ChannelLoc, FileSlot};

const STORAGE_KEY: &str = "file_sessions";

/// How many files are remembered. Beyond this the least recently saved is
/// dropped: the list is a convenience, not an archive.
const MAX_FILES: usize = 20;

/// Channels remembered for one file. A file with hundreds of plotted
/// channels is a plot nobody can read, and restoring it would make reopening
/// slow for no gain, so the list is capped where the plot stops being useful.
const MAX_PLOTTED: usize = 32;

/// Computed definitions remembered for one file, capped like the plotted
/// channels: restoring more than this buys nothing but cost.
const MAX_COMPUTED_DEFS: usize = 32;

/// Longest name, expression, or unit restored for a computed definition.
/// Session files are external input; a crafted line could otherwise ship
/// megabytes of expression at the parser. Definitions past the bound are
/// dropped whole rather than truncated.
const MAX_COMPUTED_FIELD_LEN: usize = 4096;

/// The state remembered for one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    /// The channels that were plotted, in the order they were added — which
    /// is the order that decides their colours. Each carries the file it came
    /// from: a session can hold channels from the compared pair, and three
    /// indices on their own do not say which measurement they are in.
    pub plotted: Vec<(FileSlot, ChannelLoc)>,
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
    /// The measurement this one was being compared against, if any. Stored
    /// as a path rather than anything derived from it, so reopening the pair
    /// is the same act as opening the second file by hand.
    pub second: Option<PathBuf>,
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
/// Encodes a list of [`crate::computed::ComputedDef`] into a single string for storage.
///
/// The format is `name=expression&unit&flag` per definition, joined with `;`,
/// where `flag` is `1` for a plotted definition and `0` for a hidden one.
/// Every field is escaped so its content cannot be mistaken for a separator.
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
            format!(
                "{}={}&{}&{}",
                enc(&d.name),
                enc(&d.expression),
                enc(&d.unit),
                u8::from(d.visible)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Decodes a string produced by [`encode_computed_defs`] back into [`crate::computed::ComputedDef`]s.
///
/// Session files are external input: a crafted line may carry megabytes of
/// expression or thousands of definitions, and the parser would otherwise
/// take all of it. Fields longer than [`MAX_COMPUTED_FIELD_LEN`] drop their
/// whole definition (truncating would evaluate something the user never
/// wrote), and at most [`MAX_COMPUTED_DEFS`] definitions are restored.
/// Lines written before the visibility flag existed restore every
/// definition as visible, which is what they meant at the time.
pub fn decode_computed_defs(s: &str) -> Vec<crate::computed::ComputedDef> {
    if s.is_empty() {
        return Vec::new();
    }

    // Splits on `delim` while treating `\x` escapes as literal, so an
    // escaped separator inside a field survives the split. Unescaping first
    // would let a `;` inside a unit tear the line apart.
    fn split_unescaped(s: &str, delim: char) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == delim {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        parts.push(current);
        parts
    }

    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    let mut defs = Vec::new();
    for item in split_unescaped(s, ';') {
        // A well-formed item has exactly one unescaped '=': the name/
        // expression separator. Anything else is a line this encoder never
        // wrote, and guessing at it would only misread it.
        let mut kv = split_unescaped(&item, '=');
        if kv.len() != 2 {
            continue;
        }
        let rest = kv.pop().unwrap_or_default();
        let name = kv.pop().unwrap_or_default();
        let fields = split_unescaped(&rest, '&');
        let expr = fields.first().map(String::as_str).unwrap_or("");
        let unit = fields.get(1).map(String::as_str).unwrap_or("");
        let flag = fields.get(2).map(String::as_str);

        let name_t = unescape(&name).trim().to_string();
        let expr_t = unescape(expr).trim().to_string();
        let unit_t = unescape(unit).trim().to_string();
        if name_t.is_empty() || expr_t.is_empty() {
            continue;
        }
        if name_t.len() > MAX_COMPUTED_FIELD_LEN
            || expr_t.len() > MAX_COMPUTED_FIELD_LEN
            || unit_t.len() > MAX_COMPUTED_FIELD_LEN
        {
            continue;
        }
        if defs.len() >= MAX_COMPUTED_DEFS {
            break;
        }
        defs.push(crate::computed::ComputedDef {
            name: name_t,
            expression: expr_t,
            unit: unit_t,
            // A missing flag is a line from before the flag existed; those
            // definitions were all plotted.
            visible: flag.is_none_or(|f| f.trim() != "0"),
        });
    }
    defs
}

/// One stored line: the path, the plotted channels, and the two tab labels,
/// tab-separated. Paths cannot contain a tab on the platforms this runs on,
/// and a path that somehow does is dropped by [`parse_line`] rather than
/// misread.
/// A channel from the compared file is written with a `B` in front of its
/// indices; one from the file the session is keyed by is written bare, which
/// is exactly what a line written before there was a second file says.
pub fn format_line(path: &Path, session: &Session) -> String {
    let plotted = session
        .plotted
        .iter()
        .map(|(file, loc)| {
            let prefix = match file {
                FileSlot::A => "",
                FileSlot::B => "B",
            };
            format!(
                "{prefix}{}:{}:{}",
                loc.data_group_index, loc.channel_group_index, loc.channel_index
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut line = format!(
        "{}\t{}\t{}\t{}",
        path.display(),
        plotted,
        session.nav,
        session.tab
    );
    // The trailing fields are positional, so a later one being present makes
    // every earlier one present too, empty if it has nothing to say.
    let has_second = session.second.is_some();
    if session.cursor_a.is_some()
        || session.cursor_b.is_some()
        || !session.computed.is_empty()
        || has_second
    {
        let a = session.cursor_a.map(|v| v.to_string()).unwrap_or_default();
        let b = session.cursor_b.map(|v| v.to_string()).unwrap_or_default();
        line.push('\t');
        line.push_str(&a);
        line.push('\t');
        line.push_str(&b);
    }
    if !session.computed.is_empty() || has_second {
        line.push('\t');
        line.push_str(&encode_computed_defs(&session.computed));
    }
    if let Some(second) = &session.second {
        line.push('\t');
        line.push_str(&second.display().to_string());
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
    // An empty field is a line that had nothing to put there, not a file
    // whose path is the empty string.
    let second = fields
        .next()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);

    let mut plotted = Vec::new();
    if !plotted_field.is_empty() {
        for entry in plotted_field.split(',') {
            // A `B` in front names the compared file; without it the entry is
            // in the file this line is keyed by, which is what every line
            // written before there was a second file means.
            let (file, entry) = match entry.strip_prefix('B') {
                Some(rest) => (FileSlot::B, rest),
                None => (FileSlot::A, entry),
            };
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
            plotted.push((
                file,
                ChannelLoc {
                    data_group_index: dg,
                    channel_group_index: cg,
                    channel_index: ch,
                },
            ));
        }
    }
    plotted.truncate(MAX_PLOTTED);

    Some((
        PathBuf::from(path),
        Session {
            plotted,
            nav,
            tab,
            cursor_a,
            cursor_b,
            computed,
            second,
        },
    ))
}

/// The channels remembered for `slot` that `file` still has.
///
/// A session is keyed by path, and the file at that path can be rewritten
/// between runs — a shorter recording, a different set of groups. Restoring
/// a location that no longer exists would index past the end of a group, so
/// what is restored is checked against the file first. Each slot is checked
/// against its own file: the compared file's locations mean nothing in the
/// first one.
pub fn prune_to_file(
    session: &Session,
    slot: FileSlot,
    file: &falcon_mdf::Mf4File,
) -> Vec<ChannelLoc> {
    session
        .plotted
        .iter()
        .filter(|(entry_slot, _)| *entry_slot == slot)
        .map(|(_, loc)| *loc)
        .filter(|loc| {
            file.data_groups()
                .get(loc.data_group_index)
                .and_then(|dg| dg.channel_groups.get(loc.channel_group_index))
                .is_some_and(|cg| loc.channel_index < cg.channels.len())
        })
        .collect()
}
