//! Every block in a file, in address order.
//!
//! The rest of the crate reads an MF4 file as a *measurement*: groups,
//! channels, samples. This module reads it as a *file*: the raw block graph,
//! walked from the ID block at offset 0 to the last block before EOF, with
//! each block's address, type, length, links and a one-line summary of what
//! its fields say.
//!
//! That is what a viewer needs to show a file's structure as it actually sits
//! on disk, and what a reader needs when a file does not open and the
//! question is which block is wrong. It is deliberately independent of the
//! model layer: the walk follows links out of block headers, so a block the
//! model layer skips (a data block behind an unsupported flag, a text block
//! nothing references any more) still appears.
//!
//! ```no_run
//! use falcon_mdf::Mf4File;
//!
//! let file = Mf4File::open("measurement.mf4")?;
//! let map = file.block_map();
//! for block in &map.blocks {
//!     println!("{:#010x}  {}  {} bytes  {}", block.address, block.block_type, block.length, block.summary);
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```
//!
//! # What the walk guarantees
//!
//! - Every block reachable from the header block is listed, once, whatever
//!   the shape of the graph — a block linked from several places carries
//!   every referrer in [`BlockInfo::referenced_by`] rather than appearing
//!   twice.
//! - A link that does not point at a block is reported in
//!   [`BlockMap::warnings`], not followed and not silently dropped.
//! - Regions of the file no listed block covers are reported as
//!   [`BlockMap::gaps`], so "the file is 4 MB but its blocks account for
//!   3 MB" is visible rather than invisible.
//! - Nothing in here reads a block's payload beyond a short prefix, so
//!   mapping a file with a gigabyte data block costs a header read, not a
//!   gigabyte.

use std::collections::{BTreeMap, HashMap, VecDeque};

use byteorder::{ByteOrder, LittleEndian};

use crate::blocks::{BlockHeader, BLOCK_HEADER_SIZE, ID_BLOCK_SIZE};
use crate::io::ByteSource;

/// How many bytes of a block's data section are read for its summary.
///
/// Enough for every fixed-size field of every block type this module
/// summarizes, plus a readable prefix of a text block. A data block's
/// payload is never wanted here, so the walk does not pay for one.
const SUMMARY_PREFIX: usize = 320;

/// Upper bound on the number of blocks a single walk will visit.
///
/// A file whose links form a graph larger than this is either enormous or
/// malformed; either way the walk stops and says so in
/// [`BlockMap::warnings`] rather than running until it is killed.
const MAX_BLOCKS: usize = 2_000_000;

/// One block as it sits in the file.
#[derive(Debug, Clone)]
pub struct BlockInfo {
    /// File offset of the block's first byte.
    pub address: u64,
    /// The block's four-character identifier, such as `##CN`. The ID block
    /// at offset 0 reports `MDF `, which is what it carries.
    pub block_type: String,
    /// Total length of the block in bytes, header included.
    pub length: u64,
    /// Number of links in the block's link section.
    pub link_count: u64,
    /// The links themselves, in file order. A zero means "absent", which is
    /// how MF4 spells an optional link that is not used.
    pub links: Vec<u64>,
    /// What each link is, per the format's naming — `cn_tx_name`,
    /// `dg_cg_first` and so on. Same length as [`BlockInfo::links`]; a link
    /// past the ones the spec names individually (a data list's payload
    /// links, for instance) gets the block's generic name for that tail.
    pub link_labels: Vec<String>,
    /// A one-line description built from the block's own fields: a text
    /// block's text, a channel's name and layout, a compressed block's
    /// before-and-after sizes. Empty when the block type carries nothing
    /// worth a line.
    pub summary: String,
    /// Addresses of the blocks that link to this one, in the order the walk
    /// met them. Empty for the ID and header blocks, which nothing points at.
    pub referenced_by: Vec<u64>,
    /// Size of the block's data section — its length less the header and the
    /// link section.
    pub data_size: u64,
}

/// A stretch of file that no listed block covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// File offset of the first uncovered byte.
    pub address: u64,
    /// How many bytes are uncovered.
    pub length: u64,
}

/// Every block in a file, plus what the walk could not make sense of.
#[derive(Debug, Clone)]
pub struct BlockMap {
    /// Every block found, sorted by address — the order they sit in the file.
    pub blocks: Vec<BlockInfo>,
    /// Regions no block covers, sorted by address.
    pub gaps: Vec<Gap>,
    /// Total size of the file in bytes.
    pub file_size: u64,
    /// Bytes accounted for by the blocks in [`BlockMap::blocks`].
    pub covered_bytes: u64,
    /// Links that did not point at a block, blocks whose header would not
    /// parse, and the walk's own limits when they are hit. One line each,
    /// naming the address it is about.
    pub warnings: Vec<String>,
    /// Whether the identification block declares the file unfinalized.
    ///
    /// It changes how the rest of the map reads: an unfinalized writer stops
    /// without filling in the last data block's length, so that block
    /// declares 24 bytes while its records run on to the end of the file.
    /// The bytes after it are then reported as an uncovered region, which is
    /// what they are — but the reason is here rather than left to be
    /// guessed at.
    pub unfinalized: bool,
}

impl BlockMap {
    /// Walks `source` and returns its block map.
    ///
    /// This never fails: a file too damaged to walk yields a map with few
    /// blocks and many warnings, which is the useful answer for a viewer.
    pub fn scan<S: ByteSource + ?Sized>(source: &S) -> Self {
        Walk::new(source).run()
    }

    /// Returns the block at `address`, if one was found there.
    pub fn block_at(&self, address: u64) -> Option<&BlockInfo> {
        self.blocks
            .binary_search_by_key(&address, |b| b.address)
            .ok()
            .map(|i| &self.blocks[i])
    }

    /// Counts the blocks of each type, most frequent first.
    ///
    /// A viewer shows this as the file's composition — "3,412 ##DZ, 41 ##CG"
    /// — which is the fastest way to see how a file was written.
    pub fn type_counts(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for block in &self.blocks {
            *counts.entry(block.block_type.as_str()).or_default() += 1;
        }
        let mut counts: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        // Ties broken by name so the order is stable between runs; a table
        // that reshuffles itself on every open is worse than useless.
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    }
}

/// A block found during the walk, before summaries are resolved.
struct Raw {
    address: u64,
    block_type: [u8; 4],
    length: u64,
    link_count: u64,
    links: Vec<u64>,
    data_size: u64,
    /// The first [`SUMMARY_PREFIX`] bytes of the data section, kept only
    /// until the summary is built from them.
    prefix: Vec<u8>,
    referenced_by: Vec<u64>,
}

struct Walk<'a, S: ByteSource + ?Sized> {
    source: &'a S,
    found: BTreeMap<u64, Raw>,
    queue: VecDeque<(u64, u64)>,
    warnings: Vec<String>,
    unfinalized: bool,
}

impl<'a, S: ByteSource + ?Sized> Walk<'a, S> {
    fn new(source: &'a S) -> Self {
        Self {
            source,
            found: BTreeMap::new(),
            queue: VecDeque::new(),
            warnings: Vec::new(),
            unfinalized: false,
        }
    }

    fn run(mut self) -> BlockMap {
        let file_size = self.source.len();
        self.read_id_block(file_size);

        // The header block always sits immediately after the ID block; the
        // format fixes both positions, so the walk needs no link to start.
        self.queue.push_back((ID_BLOCK_SIZE as u64, 0));
        while let Some((address, referrer)) = self.queue.pop_front() {
            if let Some(existing) = self.found.get_mut(&address) {
                if referrer != 0 && !existing.referenced_by.contains(&referrer) {
                    existing.referenced_by.push(referrer);
                }
                continue;
            }
            if self.found.len() >= MAX_BLOCKS {
                self.warnings.push(format!(
                    "stopped after {MAX_BLOCKS} blocks; the file's link graph is larger than this walk will follow"
                ));
                break;
            }
            self.visit(address, referrer);
        }

        self.finish(file_size)
    }

    /// The ID block is the one block with no header of the usual shape: 64
    /// fixed bytes at offset 0. It is listed anyway, because a map of the
    /// file that starts at byte 64 is not a map of the file.
    fn read_id_block(&mut self, file_size: u64) {
        if file_size < ID_BLOCK_SIZE as u64 {
            self.warnings.push(format!(
                "the file is {file_size} bytes, shorter than the {ID_BLOCK_SIZE}-byte identification block"
            ));
            return;
        }
        let Ok(data) = self.source.read_bytes(0, ID_BLOCK_SIZE) else {
            self.warnings
                .push("the identification block at 0x0 could not be read".to_string());
            return;
        };
        let mut block_type = [0u8; 4];
        block_type.copy_from_slice(&data[..4]);
        // Two ways a file says it was never finalized: the identifier reads
        // `UnFinMF` instead of `MDF`, and the flags at byte 60 are set. Both
        // are checked, because a writer may use either.
        self.unfinalized = &data[..8] == b"UnFinMF " || LittleEndian::read_u16(&data[60..62]) != 0;
        self.found.insert(
            0,
            Raw {
                address: 0,
                block_type,
                length: ID_BLOCK_SIZE as u64,
                link_count: 0,
                links: Vec::new(),
                data_size: ID_BLOCK_SIZE as u64,
                prefix: data[..ID_BLOCK_SIZE].to_vec(),
                referenced_by: Vec::new(),
            },
        );
    }

    fn visit(&mut self, address: u64, referrer: u64) {
        let file_size = self.source.len();
        if address.saturating_add(BLOCK_HEADER_SIZE as u64) > file_size {
            self.warnings.push(format!(
                "the link at {referrer:#x} points to {address:#x}, past the end of the {file_size}-byte file"
            ));
            return;
        }
        let raw_header = match self.source.read_bytes(address, BLOCK_HEADER_SIZE) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.warnings
                    .push(format!("the block at {address:#x} could not be read: {e}"));
                return;
            }
        };

        // Checked before the header is parsed, not after: a link into the
        // middle of a data block would otherwise be reported as a block whose
        // length and link count disagree, when the truth is simpler — there
        // is no block there at all. Following it would fill the map with
        // noise mined out of sample data.
        if &raw_header[..2] != b"##" {
            let id = String::from_utf8_lossy(&raw_header[..4]).into_owned();
            self.warnings.push(format!(
                "the link at {referrer:#x} points to {address:#x}, which starts with {id:?} rather than a block identifier"
            ));
            return;
        }

        let header = match BlockHeader::parse(&raw_header, address) {
            Ok(header) => header,
            Err(e) => {
                self.warnings
                    .push(format!("the block at {address:#x} does not parse: {e}"));
                return;
            }
        };
        if address.saturating_add(header.length) > file_size {
            self.warnings.push(format!(
                "the block at {address:#x} declares {} bytes, which runs past the end of the file",
                header.length
            ));
            return;
        }

        let links = self.read_links(address, &header);
        let prefix_len = header.data_size().min(SUMMARY_PREFIX as u64) as usize;
        let prefix = self
            .source
            .read_bytes(address + header.data_offset() as u64, prefix_len)
            .map(|b| b.to_vec())
            .unwrap_or_default();

        for (index, &link) in links.iter().enumerate() {
            if link == 0 {
                continue;
            }
            if link == address {
                self.warnings.push(format!(
                    "link {index} of the block at {address:#x} points at itself"
                ));
                continue;
            }
            self.queue.push_back((link, address));
        }

        self.found.insert(
            address,
            Raw {
                address,
                block_type: header.block_type,
                length: header.length,
                link_count: header.link_count,
                links,
                data_size: header.data_size(),
                prefix,
                referenced_by: if referrer == 0 {
                    Vec::new()
                } else {
                    vec![referrer]
                },
            },
        );
    }

    /// Reads a block's link section. `BlockHeader::parse` has already checked
    /// that the links fit inside the block's declared length, and `visit`
    /// that the length fits inside the file, so a short read here means the
    /// I/O failed rather than that the numbers disagreed.
    fn read_links(&mut self, address: u64, header: &BlockHeader) -> Vec<u64> {
        let count = header.link_count as usize;
        if count == 0 {
            return Vec::new();
        }
        let bytes = match self
            .source
            .read_bytes(address + BLOCK_HEADER_SIZE as u64, count * 8)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                self.warnings.push(format!(
                    "the {count} links of the block at {address:#x} could not be read: {e}"
                ));
                return Vec::new();
            }
        };
        (0..count)
            .map(|i| LittleEndian::read_u64(&bytes[i * 8..i * 8 + 8]))
            .collect()
    }

    fn finish(self, file_size: u64) -> BlockMap {
        let Walk {
            found,
            warnings,
            unfinalized,
            ..
        } = self;

        // Summaries are built after the walk because several of them name a
        // block that is linked, not embedded: a channel's name lives in its
        // TX block, which may sit anywhere in the file.
        let texts: HashMap<u64, String> = found
            .values()
            .filter(|raw| matches!(&raw.block_type, b"##TX" | b"##MD"))
            .map(|raw| (raw.address, block_text(&raw.prefix)))
            .collect();

        let mut covered_bytes = 0u64;
        let mut blocks: Vec<BlockInfo> = Vec::with_capacity(found.len());
        for raw in found.values() {
            covered_bytes = covered_bytes.saturating_add(raw.length);
            blocks.push(BlockInfo {
                address: raw.address,
                block_type: String::from_utf8_lossy(&raw.block_type).into_owned(),
                length: raw.length,
                link_count: raw.link_count,
                link_labels: link_labels(&raw.block_type, raw.links.len()),
                links: raw.links.clone(),
                summary: summarize(raw, &texts, unfinalized),
                referenced_by: raw.referenced_by.clone(),
                data_size: raw.data_size,
            });
        }

        // `found` is a BTreeMap keyed by address, so `blocks` is already in
        // address order — which is what a gap sweep needs.
        let mut gaps = Vec::new();
        let mut cursor = 0u64;
        for block in &blocks {
            if block.address > cursor {
                gaps.push(Gap {
                    address: cursor,
                    length: block.address - cursor,
                });
            }
            cursor = cursor.max(block.address.saturating_add(block.length));
        }
        if cursor < file_size {
            gaps.push(Gap {
                address: cursor,
                length: file_size - cursor,
            });
        }

        BlockMap {
            blocks,
            gaps,
            file_size,
            covered_bytes,
            warnings,
            unfinalized,
        }
    }
}

/// The text of a TX or MD block, as far as its prefix goes.
///
/// TX text is NUL-terminated; MD text is XML, which is left as it is —
/// unwrapping it to the `<TX>` element it carries is [`crate::model`]'s job,
/// and here the point is to show what the block holds.
fn block_text(prefix: &[u8]) -> String {
    let end = prefix.iter().position(|&b| b == 0).unwrap_or(prefix.len());
    String::from_utf8_lossy(&prefix[..end]).into_owned()
}

/// Collapses whitespace and cuts to `limit` characters, so a summary stays
/// one line however the block's text was written.
fn one_line(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(limit).collect();
    format!("{cut}\u{2026}")
}

/// The text of the block `link` names, when it is a text block that was
/// found. An absent or dangling link gives an empty string, which callers
/// render as "unnamed" rather than as a hole.
fn linked_text(links: &[u64], index: usize, texts: &HashMap<u64, String>) -> String {
    links
        .get(index)
        .and_then(|link| texts.get(link))
        .map(|text| one_line(text, 60))
        .unwrap_or_default()
}

fn quoted_or(name: &str, fallback: &str) -> String {
    if name.is_empty() {
        fallback.to_string()
    } else {
        format!("\"{name}\"")
    }
}

fn u8_at(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset + 2).map(LittleEndian::read_u16)
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4).map(LittleEndian::read_u32)
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8).map(LittleEndian::read_u64)
}

fn f64_at(data: &[u8], offset: usize) -> Option<f64> {
    data.get(offset..offset + 8).map(LittleEndian::read_f64)
}

/// One line describing what a block's own fields say.
///
/// Every field read here is bounds-checked against the prefix that was
/// actually read, so a truncated block yields a shorter summary rather than
/// a panic. Where a field is missing the summary simply omits it.
fn summarize(raw: &Raw, texts: &HashMap<u64, String>, unfinalized: bool) -> String {
    let data = &raw.prefix[..];
    let links = &raw.links[..];
    match &raw.block_type {
        b"MDF " => {
            let version = one_line(&String::from_utf8_lossy(data.get(8..16).unwrap_or(&[])), 8);
            let program = one_line(&String::from_utf8_lossy(data.get(16..24).unwrap_or(&[])), 8);
            let unfinalized = u16_at(data, 60).unwrap_or(0);
            let mut summary = format!(
                "MDF {version}, written by {}",
                quoted_or(&program, "an unnamed tool")
            );
            if unfinalized != 0 {
                summary.push_str(&format!(", unfinalized (flags {unfinalized:#06x})"));
            }
            summary
        }
        b"##HD" => {
            let start_ns = u64_at(data, 0).unwrap_or(0);
            let flags = u8_at(data, 12).unwrap_or(0);
            format!("recording starts at {start_ns} ns, time flags {flags:#04x}")
        }
        b"##FH" => {
            let time_ns = u64_at(data, 0).unwrap_or(0);
            let comment = linked_text(links, 1, texts);
            if comment.is_empty() {
                format!("history entry at {time_ns} ns")
            } else {
                format!("history entry at {time_ns} ns: {comment}")
            }
        }
        b"##DG" => {
            let rec_id_size = u8_at(data, 0).unwrap_or(0);
            if rec_id_size == 0 {
                "sorted data group (no record IDs)".to_string()
            } else {
                format!("unsorted data group, {rec_id_size}-byte record IDs")
            }
        }
        b"##CG" => {
            let name = linked_text(links, 2, texts);
            let record_id = u64_at(data, 0).unwrap_or(0);
            let cycles = u64_at(data, 8).unwrap_or(0);
            let flags = u16_at(data, 16).unwrap_or(0);
            let data_bytes = u32_at(data, 20).unwrap_or(0);
            let inval_bytes = u32_at(data, 24).unwrap_or(0);
            let mut summary = format!(
                "{} \u{2014} record {record_id}, {cycles} cycles, {data_bytes} data bytes",
                quoted_or(&name, "unnamed group")
            );
            if inval_bytes > 0 {
                summary.push_str(&format!(" + {inval_bytes} invalidation bytes"));
            }
            // Bit 0 is the VLSD flag and bit 1 the bus-event flag; both change
            // what the group's records even are, so they belong in the line.
            if flags & 0x1 != 0 {
                summary.push_str(", variable-length data");
            }
            if flags & 0x2 != 0 {
                summary.push_str(", bus events");
            }
            summary
        }
        b"##CN" => {
            let name = linked_text(links, 2, texts);
            let cn_type = u8_at(data, 0).unwrap_or(0);
            let sync_type = u8_at(data, 1).unwrap_or(0);
            let data_type = u8_at(data, 2).unwrap_or(0);
            let bit_offset = u8_at(data, 3).unwrap_or(0);
            let byte_offset = u32_at(data, 4).unwrap_or(0);
            let bit_count = u32_at(data, 8).unwrap_or(0);
            format!(
                "{} \u{2014} {}, data type {data_type}, {bit_count} bits at byte {byte_offset}+{bit_offset}",
                quoted_or(&name, "unnamed channel"),
                channel_type_name(cn_type, sync_type)
            )
        }
        b"##CC" => {
            let name = linked_text(links, 0, texts);
            let cc_type = u8_at(data, 0).unwrap_or(0);
            let ref_count = u16_at(data, 4).unwrap_or(0);
            let val_count = u16_at(data, 6).unwrap_or(0);
            let mut summary = conversion_type_name(cc_type).to_string();
            if !name.is_empty() {
                summary = format!("\"{name}\" \u{2014} {summary}");
            }
            summary.push_str(&format!(", {val_count} values, {ref_count} references"));
            summary
        }
        b"##SI" => {
            let name = linked_text(links, 0, texts);
            let path = linked_text(links, 1, texts);
            let si_type = u8_at(data, 0).unwrap_or(0);
            let bus_type = u8_at(data, 1).unwrap_or(0);
            let flags = u8_at(data, 2).unwrap_or(0);
            let mut summary = format!(
                "{} \u{2014} {} source",
                quoted_or(&name, "unnamed source"),
                source_type_name(si_type)
            );
            if bus_type != 0 {
                summary.push_str(&format!(" on {}", bus_type_name(bus_type)));
            }
            if !path.is_empty() {
                summary.push_str(&format!(", path {path}"));
            }
            if flags & 0x1 != 0 {
                summary.push_str(", simulated");
            }
            summary
        }
        b"##TX" | b"##MD" => {
            let text = one_line(&block_text(data), 100);
            // An empty summary next to an empty text block reads as "this
            // tool has nothing to say about it" rather than as the truth,
            // which is that the block holds an empty string.
            if text.is_empty() {
                "(empty)".to_string()
            } else {
                text
            }
        }
        b"##DT" | b"##SD" | b"##RD" => {
            // A writer that stopped without finalizing never went back to
            // fill this in. Reporting "0 bytes" alone would say the block is
            // empty, when what it means is that its length was never written.
            if raw.data_size == 0 && unfinalized {
                "no length recorded \u{2014} the file was never finalized, so the records run to the end of it".to_string()
            } else {
                format!("{} bytes of records", raw.data_size)
            }
        }
        b"##DZ" => {
            let original = data
                .get(..2)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let zip_type = u8_at(data, 2).unwrap_or(0);
            let original_len = u64_at(data, 8).unwrap_or(0);
            let stored_len = u64_at(data, 16).unwrap_or(0);
            let ratio = if original_len > 0 {
                stored_len as f64 / original_len as f64 * 100.0
            } else {
                0.0
            };
            format!(
                "##{original} compressed with {}: {original_len} \u{2192} {stored_len} bytes ({ratio:.0}%)",
                zip_type_name(zip_type)
            )
        }
        b"##DL" => {
            let flags = u8_at(data, 0).unwrap_or(0);
            let count = u32_at(data, 4).unwrap_or(0);
            let mut summary = format!("{count} data blocks");
            if flags & 0x1 != 0 {
                if let Some(equal_length) = u64_at(data, 8) {
                    summary.push_str(&format!(", {equal_length} bytes each"));
                }
            }
            if flags & 0x2 != 0 {
                summary.push_str(", time-indexed");
            }
            summary
        }
        b"##HL" => {
            let flags = u16_at(data, 0).unwrap_or(0);
            let zip_type = u8_at(data, 2).unwrap_or(0);
            format!(
                "header of a {} list, flags {flags:#06x}",
                zip_type_name(zip_type)
            )
        }
        b"##AT" => {
            let name = linked_text(links, 1, texts);
            let mime = linked_text(links, 2, texts);
            let flags = u16_at(data, 16).unwrap_or(0);
            let original_size = u64_at(data, 24).unwrap_or(0);
            let embedded_size = u64_at(data, 32).unwrap_or(0);
            let embedded = flags & 0x1 != 0;
            let mut summary = format!(
                "{} \u{2014} {}",
                quoted_or(&name, "unnamed attachment"),
                if embedded { "embedded" } else { "external" }
            );
            if embedded {
                summary.push_str(&format!(", {embedded_size} bytes stored"));
                if flags & 0x2 != 0 {
                    summary.push_str(&format!(" for {original_size} original"));
                }
            } else {
                summary.push_str(&format!(", {original_size} bytes at the named path"));
            }
            if !mime.is_empty() {
                summary.push_str(&format!(", {mime}"));
            }
            summary
        }
        b"##EV" => {
            let name = linked_text(links, 3, texts);
            let ev_type = u8_at(data, 0).unwrap_or(0);
            let sync_type = u8_at(data, 1).unwrap_or(0);
            let scope_count = u32_at(data, 8).unwrap_or(0);
            let base = u64_at(data, 16).unwrap_or(0) as i64;
            let factor = f64_at(data, 24).unwrap_or(0.0);
            format!(
                "{} \u{2014} {} event at {} {}, {scope_count} in scope",
                quoted_or(&name, "unnamed event"),
                event_type_name(ev_type),
                base as f64 * factor,
                sync_unit_name(sync_type)
            )
        }
        b"##CA" => {
            let ca_type = u8_at(data, 0).unwrap_or(0);
            let storage = u8_at(data, 1).unwrap_or(0);
            let ndim = u16_at(data, 2).unwrap_or(0);
            let dims: Vec<String> = (0..ndim as usize)
                .filter_map(|i| u64_at(data, 16 + i * 8))
                .map(|d| d.to_string())
                .collect();
            format!(
                "{} array, {} storage, shape [{}]",
                array_type_name(ca_type),
                array_storage_name(storage),
                dims.join(", ")
            )
        }
        b"##CH" => {
            let name = linked_text(links, 2, texts);
            let element_count = u32_at(data, 0).unwrap_or(0);
            let ch_type = u8_at(data, 4).unwrap_or(0);
            format!(
                "{} \u{2014} {} node, {element_count} channels",
                quoted_or(&name, "unnamed node"),
                hierarchy_type_name(ch_type)
            )
        }
        b"##SR" => {
            let cycles = u64_at(data, 0).unwrap_or(0);
            let interval = f64_at(data, 8).unwrap_or(0.0);
            let sync_type = u8_at(data, 16).unwrap_or(0);
            format!(
                "{cycles} reduced cycles every {interval} {}",
                sync_unit_name(sync_type)
            )
        }
        _ => String::new(),
    }
}

fn channel_type_name(cn_type: u8, sync_type: u8) -> String {
    let kind = match cn_type {
        0 => "fixed-length value",
        1 => "variable-length value",
        2 => "master",
        3 => "virtual master",
        4 => "synchronisation",
        5 => "maximum-length value",
        6 => "virtual value",
        _ => "unknown type",
    };
    // Only master and synchronisation channels give the sync type a meaning;
    // on an ordinary value channel the field is zero and saying "time" about
    // it would be an invention.
    if matches!(cn_type, 2..=4) {
        format!("{kind} ({})", sync_name(sync_type))
    } else {
        kind.to_string()
    }
}

fn sync_name(sync_type: u8) -> &'static str {
    match sync_type {
        0 => "none",
        1 => "time",
        2 => "angle",
        3 => "distance",
        4 => "index",
        _ => "unknown",
    }
}

fn sync_unit_name(sync_type: u8) -> &'static str {
    match sync_type {
        1 => "s",
        2 => "rad",
        3 => "m",
        4 => "samples",
        _ => "",
    }
}

fn conversion_type_name(cc_type: u8) -> &'static str {
    match cc_type {
        0 => "identity conversion",
        1 => "linear conversion",
        2 => "rational conversion",
        3 => "algebraic conversion",
        4 => "value-to-value table, interpolating",
        5 => "value-to-value table",
        6 => "value-range-to-value table",
        7 => "value-to-text table",
        8 => "value-range-to-text table",
        9 => "text-to-value table",
        10 => "text-to-text table",
        11 => "bitfield text table",
        _ => "unknown conversion",
    }
}

fn source_type_name(si_type: u8) -> &'static str {
    match si_type {
        0 => "other",
        1 => "ECU",
        2 => "bus",
        3 => "I/O",
        4 => "tool",
        5 => "user",
        _ => "unknown",
    }
}

fn bus_type_name(bus_type: u8) -> &'static str {
    match bus_type {
        1 => "CAN",
        2 => "LIN",
        3 => "MOST",
        4 => "FlexRay",
        5 => "K-Line",
        6 => "Ethernet",
        7 => "USB",
        _ => "an unknown bus",
    }
}

fn zip_type_name(zip_type: u8) -> &'static str {
    match zip_type {
        0 => "deflate",
        1 => "transposed deflate",
        _ => "an unknown compression",
    }
}

fn event_type_name(ev_type: u8) -> &'static str {
    match ev_type {
        0 => "recording",
        1 => "recording interrupt",
        2 => "acquisition interrupt",
        3 => "start recording trigger",
        4 => "stop recording trigger",
        5 => "trigger",
        6 => "marker",
        _ => "unknown",
    }
}

fn array_type_name(ca_type: u8) -> &'static str {
    match ca_type {
        0 => "value",
        1 => "scaling axis",
        2 => "look-up",
        3 => "interval axis",
        4 => "classification result",
        _ => "unknown",
    }
}

fn array_storage_name(storage: u8) -> &'static str {
    match storage {
        0 => "in-record",
        1 => "per-element signal data",
        2 => "per-array signal data",
        _ => "unknown",
    }
}

fn hierarchy_type_name(ch_type: u8) -> &'static str {
    match ch_type {
        0 => "group",
        1 => "function",
        2 => "structure",
        3 => "map list",
        4 => "input",
        5 => "output",
        6 => "local",
        7 => "calibration definition",
        8 => "calibration reference",
        _ => "unknown",
    }
}

/// The format's own name for each link of a block type.
///
/// Blocks whose link section ends with a repeated tail — a data list's
/// payload links, a conversion's references — name the tail generically and
/// number it, which is how the spec itself describes them.
fn link_labels(block_type: &[u8; 4], count: usize) -> Vec<String> {
    let (named, tail): (&[&str], &str) = match block_type {
        b"##HD" => (
            &[
                "hd_dg_first",
                "hd_fh_first",
                "hd_ch_first",
                "hd_at_first",
                "hd_ev_first",
                "hd_md_comment",
            ],
            "hd_link",
        ),
        b"##FH" => (&["fh_fh_next", "fh_md_comment"], "fh_link"),
        b"##DG" => (
            &["dg_dg_next", "dg_cg_first", "dg_data", "dg_md_comment"],
            "dg_link",
        ),
        b"##CG" => (
            &[
                "cg_cg_next",
                "cg_cn_first",
                "cg_tx_acq_name",
                "cg_si_acq_source",
                "cg_sr_first",
                "cg_md_comment",
            ],
            "cg_link",
        ),
        b"##CN" => (
            &[
                "cn_cn_next",
                "cn_composition",
                "cn_tx_name",
                "cn_si_source",
                "cn_cc_conversion",
                "cn_data",
                "cn_md_unit",
                "cn_md_comment",
            ],
            "cn_at_reference",
        ),
        b"##CC" => (
            &["cc_tx_name", "cc_md_unit", "cc_md_comment", "cc_cc_inverse"],
            "cc_ref",
        ),
        b"##SI" => (&["si_tx_name", "si_tx_path", "si_md_comment"], "si_link"),
        b"##CA" => (&["ca_composition"], "ca_data"),
        b"##DL" => (&["dl_dl_next"], "dl_data"),
        b"##HL" => (&["hl_dl_first"], "hl_link"),
        b"##AT" => (
            &[
                "at_at_next",
                "at_tx_filename",
                "at_tx_mimetype",
                "at_md_comment",
            ],
            "at_link",
        ),
        b"##EV" => (
            &[
                "ev_ev_next",
                "ev_ev_parent",
                "ev_ev_range",
                "ev_tx_name",
                "ev_md_comment",
            ],
            "ev_scope",
        ),
        b"##CH" => (
            &["ch_ch_next", "ch_ch_first", "ch_tx_name", "ch_md_comment"],
            "ch_element",
        ),
        b"##SR" => (&["sr_sr_next", "sr_data"], "sr_link"),
        _ => (&[], "link"),
    };

    (0..count)
        .map(|i| match named.get(i) {
            Some(name) => (*name).to_string(),
            None => format!("{tail}[{}]", i - named.len()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ByteSlice;

    /// A byte source over a buffer, so the walk can be tested against files
    /// built byte by byte rather than against whatever the corpus happens to
    /// contain.
    struct Bytes(Vec<u8>);

    impl ByteSource for Bytes {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }

        fn read_bytes(&self, offset: u64, len: usize) -> crate::error::Result<ByteSlice<'_>> {
            let start = offset as usize;
            let end = start.saturating_add(len);
            if end > self.0.len() {
                return Err(crate::error::Mf4Error::truncated(
                    offset,
                    len,
                    self.0.len().saturating_sub(start),
                ));
            }
            Ok(ByteSlice::borrowed(&self.0[start..end]))
        }
    }

    /// Builds a file: a 64-byte ID block, then the given blocks laid out in
    /// order. Each block is `(id, links, data)`.
    fn build(blocks: &[(&[u8; 4], Vec<u64>, Vec<u8>)]) -> (Bytes, Vec<u64>) {
        let mut file = vec![0u8; ID_BLOCK_SIZE];
        file[..8].copy_from_slice(b"MDF     ");
        file[8..16].copy_from_slice(b"4.10    ");

        let mut addresses = Vec::new();
        for (id, links, data) in blocks {
            addresses.push(file.len() as u64);
            let length = (BLOCK_HEADER_SIZE + links.len() * 8 + data.len()) as u64;
            file.extend_from_slice(*id);
            file.extend_from_slice(&[0u8; 4]);
            file.extend_from_slice(&length.to_le_bytes());
            file.extend_from_slice(&(links.len() as u64).to_le_bytes());
            for link in links {
                file.extend_from_slice(&link.to_le_bytes());
            }
            file.extend_from_slice(data);
        }
        (Bytes(file), addresses)
    }

    #[test]
    fn walks_from_the_id_block_to_the_last_block() {
        // ##HD at 64 linking a ##TX; the TX is only reachable through it.
        let text = b"a comment\0".to_vec();
        let hd_data = vec![0u8; 24];
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], hd_data),
            (b"##TX", vec![], text),
        ]);
        // Patch hd_md_comment (link 5) to the TX block's address.
        let mut bytes = source.0;
        let hd_links = addresses[0] as usize + BLOCK_HEADER_SIZE;
        bytes[hd_links + 5 * 8..hd_links + 6 * 8].copy_from_slice(&addresses[1].to_le_bytes());
        let map = BlockMap::scan(&Bytes(bytes));

        let types: Vec<&str> = map.blocks.iter().map(|b| b.block_type.as_str()).collect();
        assert_eq!(types, ["MDF ", "##HD", "##TX"]);
        assert_eq!(map.blocks[0].address, 0);
        assert_eq!(map.blocks[1].address, 64);
        assert_eq!(map.blocks[2].summary, "a comment");
        assert_eq!(map.blocks[2].referenced_by, vec![addresses[0]]);
        assert_eq!(map.blocks[1].link_labels[5], "hd_md_comment");
        assert!(map.warnings.is_empty(), "{:?}", map.warnings);
    }

    #[test]
    fn a_block_linked_twice_is_listed_once_with_both_referrers() {
        let hd_data = vec![0u8; 24];
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], hd_data),
            (b"##FH", vec![0, 0], vec![0u8; 16]),
            (b"##TX", vec![], b"shared\0".to_vec()),
        ]);
        let mut bytes = source.0;
        let hd_links = addresses[0] as usize + BLOCK_HEADER_SIZE;
        // hd_fh_first -> FH, hd_md_comment -> TX
        bytes[hd_links + 8..hd_links + 16].copy_from_slice(&addresses[1].to_le_bytes());
        bytes[hd_links + 5 * 8..hd_links + 6 * 8].copy_from_slice(&addresses[2].to_le_bytes());
        // fh_md_comment -> the same TX
        let fh_links = addresses[1] as usize + BLOCK_HEADER_SIZE;
        bytes[fh_links + 8..fh_links + 16].copy_from_slice(&addresses[2].to_le_bytes());

        let map = BlockMap::scan(&Bytes(bytes));
        let tx: Vec<&BlockInfo> = map
            .blocks
            .iter()
            .filter(|b| b.block_type == "##TX")
            .collect();
        assert_eq!(tx.len(), 1);
        assert_eq!(tx[0].referenced_by, vec![addresses[0], addresses[1]]);
    }

    #[test]
    fn a_link_into_nothing_is_a_warning_not_a_block() {
        let hd_data = vec![0u8; 24];
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], hd_data),
            (b"##DT", vec![], vec![0xAB; 64]),
        ]);
        let mut bytes = source.0;
        let hd_links = addresses[0] as usize + BLOCK_HEADER_SIZE;
        // hd_dg_first points into the middle of the DT payload, where there
        // is no block identifier.
        let bogus = addresses[1] + 40;
        bytes[hd_links..hd_links + 8].copy_from_slice(&bogus.to_le_bytes());

        let map = BlockMap::scan(&Bytes(bytes));
        assert!(map.blocks.iter().all(|b| b.address != bogus));
        assert_eq!(map.warnings.len(), 1, "{:?}", map.warnings);
        assert!(map.warnings[0].contains("rather than a block identifier"));
    }

    #[test]
    fn bytes_no_block_covers_are_reported_as_gaps() {
        // The DT block is never linked, so the walk cannot reach it and its
        // bytes show up as a gap at the end of the file.
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], vec![0u8; 24]),
            (b"##DT", vec![], vec![0u8; 32]),
        ]);
        let map = BlockMap::scan(&source);
        assert_eq!(map.blocks.len(), 2);
        assert_eq!(
            map.gaps,
            [Gap {
                address: addresses[1],
                length: BLOCK_HEADER_SIZE as u64 + 32,
            }]
        );
        assert_eq!(map.covered_bytes + map.gaps[0].length, map.file_size);
    }

    #[test]
    fn a_truncated_block_is_reported_rather_than_read() {
        let (source, addresses) = build(&[(b"##HD", vec![0, 0, 0, 0, 0, 0], vec![0u8; 24])]);
        let mut bytes = source.0;
        // Declare the HD block far larger than the file.
        let length_at = addresses[0] as usize + 8;
        bytes[length_at..length_at + 8].copy_from_slice(&100_000u64.to_le_bytes());

        let map = BlockMap::scan(&Bytes(bytes));
        assert_eq!(map.blocks.len(), 1, "only the ID block should be listed");
        assert!(map.warnings.iter().any(|w| w.contains("past the end")));
    }

    #[test]
    fn a_cycle_between_blocks_terminates() {
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], vec![0u8; 24]),
            (b"##FH", vec![0, 0], vec![0u8; 16]),
        ]);
        let mut bytes = source.0;
        let hd_links = addresses[0] as usize + BLOCK_HEADER_SIZE;
        bytes[hd_links + 8..hd_links + 16].copy_from_slice(&addresses[1].to_le_bytes());
        // fh_fh_next points back at the header block: a cycle.
        let fh_links = addresses[1] as usize + BLOCK_HEADER_SIZE;
        bytes[fh_links..fh_links + 8].copy_from_slice(&addresses[0].to_le_bytes());

        let map = BlockMap::scan(&Bytes(bytes));
        assert_eq!(map.blocks.len(), 3);
    }

    #[test]
    fn type_counts_are_ordered_by_frequency() {
        let (source, addresses) = build(&[
            (b"##HD", vec![0, 0, 0, 0, 0, 0], vec![0u8; 24]),
            (b"##FH", vec![0, 0], vec![0u8; 16]),
            (b"##TX", vec![], b"one\0".to_vec()),
            (b"##TX", vec![], b"two\0".to_vec()),
        ]);
        let mut bytes = source.0;
        let hd_links = addresses[0] as usize + BLOCK_HEADER_SIZE;
        bytes[hd_links + 8..hd_links + 16].copy_from_slice(&addresses[1].to_le_bytes());
        bytes[hd_links + 5 * 8..hd_links + 6 * 8].copy_from_slice(&addresses[2].to_le_bytes());
        let fh_links = addresses[1] as usize + BLOCK_HEADER_SIZE;
        bytes[fh_links + 8..fh_links + 16].copy_from_slice(&addresses[3].to_le_bytes());

        let map = BlockMap::scan(&Bytes(bytes));
        let counts = map.type_counts();
        assert_eq!(counts[0], ("##TX".to_string(), 2));
    }
}
