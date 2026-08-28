//! Reading MDF 3.x files.
//!
//! This is the structure of a version 3 file — what it is, when it was taken,
//! and which channels it holds — the raw samples of those channels, in each
//! channel's own type, and the physical values those raw samples convert to.
//!
//! Version 3 is a different format from version 4 rather than an older
//! spelling of it, so it has its own module tree. See [`blocks`] for why, and
//! [`conversions`] for where the two formats' rules genuinely differ.

pub mod blocks;
pub mod conversions;
pub mod records;

use std::path::Path;
use std::sync::Arc;

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSource, IoBackend};
use crate::model::SignalValues;

use conversions::{Mdf3Conversion, Mdf3ConversionOutput};

use blocks::{CgBlock, CnBlock, DgBlock, HdBlock, IdBlock, Mdf3ChannelType};

/// How deep a chain of blocks may run before the file is treated as
/// self-referential. A file with more channels than this in one group is not
/// something this format describes; a file that loops is.
const MAX_CHAIN: usize = 100_000;

/// One channel of an MDF 3.x file.
#[derive(Debug, Clone)]
pub struct Mdf3Channel {
    /// The channel's name — its long name where one is present, otherwise the
    /// 32-character short name.
    pub name: String,
    /// What the channel measures, as written by the recorder.
    pub description: String,
    /// The unit, taken from the conversion block where one is present.
    pub unit: String,
    /// Whether this is data or the group's time channel.
    pub channel_type: Mdf3ChannelType,
    /// Offset in bits from the start of the record.
    pub start_offset: u16,
    /// Width of the channel in bits.
    pub bit_count: u16,
    /// The raw signal data type code.
    pub data_type: u16,
    /// Sampling rate in seconds, 0 when unrecorded.
    pub sampling_rate: f64,
    /// Whole bytes to add to [`Self::start_offset`], for records longer than
    /// the 8191 bytes a 16-bit bit offset can reach.
    pub additional_byte_offset: u16,
    /// Address of the conversion block, 0 when the values are already
    /// physical. Kept so that applying conversions can be added without
    /// re-walking the file.
    pub conversion_addr: u32,
}

impl Mdf3Channel {
    /// True when this channel carries the group's timestamps.
    pub fn is_time(&self) -> bool {
        self.channel_type == Mdf3ChannelType::Time
    }
}

/// One channel group: a record layout and the channels sharing it.
#[derive(Debug, Clone)]
pub struct Mdf3ChannelGroup {
    /// The identifier prefixing this group's records when the data group holds
    /// more than one group.
    pub record_id: u16,
    /// Size of one record in bytes, excluding any record identifier.
    pub record_size: u16,
    /// How many records the group declares.
    pub cycle_count: u32,
    /// The group's comment, empty when there is none.
    pub comment: String,
    /// The group's channels, in the order the file chains them.
    pub channels: Vec<Mdf3Channel>,
}

/// One data group: a record stream and the groups that share it.
#[derive(Debug, Clone)]
pub struct Mdf3DataGroup {
    /// How many copies of the record identifier each record carries: 0 when
    /// the group holds a single channel group, 1 for an identifier before each
    /// record, 2 for one before and a copy after. A count, not a byte width —
    /// the identifier itself is always one byte.
    pub record_id_count: u16,
    /// Where this group's records begin.
    pub data_block_addr: u32,
    /// The channel groups sharing the record stream.
    pub channel_groups: Vec<Mdf3ChannelGroup>,
}

/// An open MDF 3.x file.
pub struct Mdf3File {
    source: Arc<dyn ByteSource>,
    id: IdBlock,
    header: HdBlock,
    comment: String,
    data_groups: Vec<Mdf3DataGroup>,
}

impl Mdf3File {
    /// Opens an MDF 3.x file and reads its structure.
    ///
    /// Fails if the file is version 4 — that is [`crate::Mf4File`]'s job, and
    /// silently accepting it here would give the caller the wrong reader.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = IoBackend::open(path)?;
        Self::from_source(Arc::new(source))
    }

    /// Reads an MDF 3.x file's structure from an already-open source.
    pub fn from_source(source: Arc<dyn ByteSource>) -> Result<Self> {
        let id_bytes = source.read_bytes(0, IdBlock::SIZE)?;
        let id = IdBlock::parse(&id_bytes)?;

        if id.version_number >= 400 {
            return Err(Mf4Error::UnsupportedVersion {
                major: id.version_number / 100,
                minor: id.version_number % 100,
            });
        }

        let hd_offset = IdBlock::SIZE as u64;
        let hd_len = (HdBlock::COMMON_SIZE + HdBlock::POST_320_EXTRA_SIZE)
            .min((source.len().saturating_sub(hd_offset)) as usize);
        let hd_bytes = source.read_bytes(hd_offset, hd_len)?;
        let header = HdBlock::parse(&hd_bytes, hd_offset, id.big_endian)?;

        let comment =
            read_tx(source.as_ref(), header.comment_addr, id.big_endian).unwrap_or_default();

        let data_groups = walk_data_groups(source.as_ref(), &header, id.big_endian)?;

        Ok(Self {
            source,
            id,
            header,
            comment,
            data_groups,
        })
    }

    /// The version as written in the file, e.g. `"3.30"`.
    pub fn version(&self) -> &str {
        &self.id.version_text
    }

    /// The version as a number, e.g. `330`.
    pub fn version_number(&self) -> u16 {
        self.id.version_number
    }

    /// The program that wrote the file.
    pub fn program(&self) -> &str {
        &self.id.program_identification
    }

    /// The header block, for the fields not surfaced directly.
    pub fn header(&self) -> &HdBlock {
        &self.header
    }

    /// The file comment, empty when there is none.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// Nanoseconds since the epoch, when the file records an absolute time.
    ///
    /// Only 3.20 and later write one; before that the file carries the date
    /// and time as text, available through [`Self::header`].
    pub fn start_time_ns(&self) -> Option<u64> {
        self.header.abs_time
    }

    /// The file's data groups.
    pub fn data_groups(&self) -> &[Mdf3DataGroup] {
        &self.data_groups
    }

    /// Total number of channels across every group.
    pub fn channel_count(&self) -> usize {
        self.data_groups
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .map(|cg| cg.channels.len())
            .sum()
    }

    /// Finds the first channel with the given name.
    pub fn find_channel(&self, name: &str) -> Option<&Mdf3Channel> {
        self.data_groups
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .flat_map(|cg| cg.channels.iter())
            .find(|ch| ch.name == name)
    }

    /// Every channel name in the file, in file order.
    pub fn channel_names(&self) -> Vec<&str> {
        self.data_groups
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .flat_map(|cg| cg.channels.iter())
            .map(|ch| ch.name.as_str())
            .collect()
    }

    /// The channel carrying a channel group's timestamps.
    ///
    /// In v3 the master is always time, and a group is meant to hold exactly
    /// one. Where a file holds none this returns `None` rather than picking
    /// the first channel and calling it time.
    pub fn master_channel(&self, group: usize, channel_group: usize) -> Option<&Mdf3Channel> {
        self.data_groups
            .get(group)?
            .channel_groups
            .get(channel_group)?
            .channels
            .iter()
            .find(|ch| ch.is_time())
    }

    /// Reads a channel's raw samples, in the channel's own type.
    ///
    /// The values are as stored: an integer channel comes back as an integer
    /// of its own width rather than through `f64`, and no conversion is
    /// applied, so a channel with a conversion block returns raw values rather
    /// than physical ones.
    ///
    /// # Errors
    ///
    /// Returns a named error rather than a partial or shifted read when the
    /// channel does not fit its record, when the data block is shorter than
    /// the channel groups declare, when a record carries an identifier no
    /// channel group claims, or when the data type is one this build does not
    /// decode.
    pub fn channel_values(
        &self,
        group: usize,
        channel_group: usize,
        channel: usize,
    ) -> Result<SignalValues> {
        let dg = self
            .data_groups
            .get(group)
            .ok_or_else(|| Mf4Error::parse_error(format!("no data group {group} in this file")))?;
        records::read_channel(
            self.source.as_ref(),
            self.id.big_endian,
            dg,
            channel_group,
            channel,
        )
    }

    /// Reads the raw samples of the first channel with the given name.
    pub fn values_by_name(&self, name: &str) -> Result<SignalValues> {
        let (g, c, i) = self.locate(name)?;
        self.channel_values(g, c, i)
    }

    /// Reads and parses a channel's conversion rule.
    ///
    /// Returns [`Mdf3Conversion::None`] for a channel whose values are already
    /// physical.
    pub fn channel_conversion(
        &self,
        group: usize,
        channel_group: usize,
        channel: usize,
    ) -> Result<Mdf3Conversion> {
        let ch = self.channel_at(group, channel_group, channel)?;
        Mdf3Conversion::parse(self.source.as_ref(), ch.conversion_addr, self.id.big_endian)
    }

    /// Reads a channel's physical samples: its raw samples with its conversion
    /// applied.
    ///
    /// A channel whose conversion is the identity keeps its stored type, the
    /// same values [`Self::channel_values`] returns. Every other numeric
    /// conversion produces [`SignalValues::F64`], and the two text tables
    /// produce [`SignalValues::Str`].
    ///
    /// # Errors
    ///
    /// Everything [`Self::channel_values`] can fail with, plus a named error
    /// for a conversion type this build does not evaluate. A conversion that
    /// cannot be applied is never quietly skipped: raw counts returned as
    /// physical values would be a wrong measurement rather than a missing one.
    pub fn channel_physical(
        &self,
        group: usize,
        channel_group: usize,
        channel: usize,
    ) -> Result<SignalValues> {
        let conversion = self.channel_conversion(group, channel_group, channel)?;
        let raw = self.channel_values(group, channel_group, channel)?;
        apply(&conversion, raw)
    }

    /// Reads the physical samples of the first channel with the given name.
    pub fn physical_by_name(&self, name: &str) -> Result<SignalValues> {
        let (g, c, i) = self.locate(name)?;
        self.channel_physical(g, c, i)
    }

    /// The bytes backing this file, for callers that need them directly.
    pub fn source(&self) -> &Arc<dyn ByteSource> {
        &self.source
    }

    /// Finds a channel by its three indices.
    fn channel_at(
        &self,
        group: usize,
        channel_group: usize,
        channel: usize,
    ) -> Result<&Mdf3Channel> {
        self.data_groups
            .get(group)
            .and_then(|dg| dg.channel_groups.get(channel_group))
            .and_then(|cg| cg.channels.get(channel))
            .ok_or_else(|| {
                Mf4Error::parse_error(format!(
                    "no channel {channel} of group {channel_group} in data group {group}"
                ))
            })
    }

    /// Finds the indices of the first channel with the given name.
    fn locate(&self, name: &str) -> Result<(usize, usize, usize)> {
        for (g, dg) in self.data_groups.iter().enumerate() {
            for (c, cg) in dg.channel_groups.iter().enumerate() {
                if let Some(i) = cg.channels.iter().position(|ch| ch.name == name) {
                    return Ok((g, c, i));
                }
            }
        }
        Err(Mf4Error::ChannelNotFound {
            name: name.to_string(),
        })
    }
}

/// Applies a conversion to a channel's decoded raw samples.
///
/// The rule is inspected once per channel rather than once per sample: which
/// output a conversion produces, and whether it can be applied at all, do not
/// vary between the samples of one channel.
fn apply(conversion: &Mdf3Conversion, raw: SignalValues) -> Result<SignalValues> {
    if conversion.is_identity() {
        return Ok(raw);
    }

    // A text or byte channel has no number to put through a conversion. The
    // format allows a conversion block on one, and every reader ignores it.
    if matches!(raw, SignalValues::Str(_) | SignalValues::Bytes { .. }) {
        return Ok(raw);
    }

    // Every kind the v3 decoder produces other than those two is a number.
    let numbers = raw.to_f64();

    match conversion.output() {
        Mdf3ConversionOutput::Text => Ok(SignalValues::Str(
            numbers
                .iter()
                .map(|&x| conversion.convert_text(x).unwrap_or_default().to_string())
                .collect(),
        )),
        Mdf3ConversionOutput::Numeric => {
            // A rule that cannot be applied fails on the first sample, so this
            // does not walk a whole channel before reporting it.
            let mut out = Vec::with_capacity(numbers.len());
            for x in numbers {
                out.push(conversion.convert(x)?);
            }
            Ok(SignalValues::F64(out))
        }
    }
}

/// Reads a `TX` block's text, returning `None` for a null link.
fn read_tx(source: &dyn ByteSource, addr: u32, big_endian: bool) -> Option<String> {
    if addr == 0 {
        return None;
    }
    // The length lives in the block, so read the header first rather than
    // guessing a size and reading past the end of the file.
    let head = source.read_bytes(addr as u64, 4).ok()?;
    let len = if big_endian {
        u16::from_be_bytes([head[2], head[3]])
    } else {
        u16::from_le_bytes([head[2], head[3]])
    } as usize;
    if len < 4 {
        return None;
    }
    let bytes = source.read_bytes(addr as u64, len).ok()?;
    blocks::parse_tx(&bytes, addr as u64, big_endian).ok()
}

/// Reads the unit out of a conversion block.
///
/// The unit sits at a fixed offset in every conversion type, so it can be read
/// without knowing which type this is — which is deliberate, since applying
/// the conversion is a later task.
fn read_conversion_unit(source: &dyn ByteSource, addr: u32, big_endian: bool) -> Option<String> {
    if addr == 0 {
        return None;
    }
    let head = source.read_bytes(addr as u64, 4).ok()?;
    if &head[..2] != b"CC" {
        return None;
    }
    let len = if big_endian {
        u16::from_be_bytes([head[2], head[3]])
    } else {
        u16::from_le_bytes([head[2], head[3]])
    } as usize;
    // id(2) + block_len(2) + range_flag(2) + min(8) + max(8) = 22, then unit.
    if len < 42 {
        return None;
    }
    let bytes = source.read_bytes(addr as u64, len.min(42)).ok()?;
    let raw = &bytes[22..42.min(bytes.len())];
    let cut = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Some(String::from_utf8_lossy(&raw[..cut]).trim_end().to_string())
}

/// Walks the data group chain, and each group's channel group and channel
/// chains, collecting the file's structure.
fn walk_data_groups(
    source: &dyn ByteSource,
    header: &HdBlock,
    big_endian: bool,
) -> Result<Vec<Mdf3DataGroup>> {
    let mut groups = Vec::new();
    let mut addr = header.first_dg_addr;
    let mut seen = Vec::new();

    while addr != 0 {
        // A link that points at a block already visited is a cycle. Following
        // it would run until memory ran out.
        if seen.contains(&addr) {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: "data group chain returns to a block it already visited".to_string(),
            });
        }
        seen.push(addr);
        if seen.len() > MAX_CHAIN {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: format!("data group chain exceeds {MAX_CHAIN} blocks"),
            });
        }

        let bytes = source.read_bytes(
            addr as u64,
            DgBlock::SIZE_POST_320.min((source.len().saturating_sub(addr as u64)) as usize),
        )?;
        let dg = DgBlock::parse(&bytes, addr as u64, big_endian)?;

        let channel_groups = walk_channel_groups(source, dg.first_cg_addr, big_endian)?;

        groups.push(Mdf3DataGroup {
            record_id_count: dg.record_id_count,
            data_block_addr: dg.data_block_addr,
            channel_groups,
        });

        addr = dg.next_dg_addr;
    }

    Ok(groups)
}

/// Walks one data group's channel group chain.
fn walk_channel_groups(
    source: &dyn ByteSource,
    first: u32,
    big_endian: bool,
) -> Result<Vec<Mdf3ChannelGroup>> {
    let mut groups = Vec::new();
    let mut addr = first;
    let mut seen = Vec::new();

    while addr != 0 {
        if seen.contains(&addr) {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: "channel group chain returns to a block it already visited".to_string(),
            });
        }
        seen.push(addr);
        if seen.len() > MAX_CHAIN {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: format!("channel group chain exceeds {MAX_CHAIN} blocks"),
            });
        }

        let bytes = source.read_bytes(addr as u64, CgBlock::SIZE)?;
        let cg = CgBlock::parse(&bytes, addr as u64, big_endian)?;

        let channels = walk_channels(source, cg.first_ch_addr, big_endian)?;

        groups.push(Mdf3ChannelGroup {
            record_id: cg.record_id,
            record_size: cg.record_size,
            cycle_count: cg.cycle_count,
            comment: read_tx(source, cg.comment_addr, big_endian).unwrap_or_default(),
            channels,
        });

        addr = cg.next_cg_addr;
    }

    Ok(groups)
}

/// Walks one channel group's channel chain.
fn walk_channels(
    source: &dyn ByteSource,
    first: u32,
    big_endian: bool,
) -> Result<Vec<Mdf3Channel>> {
    let mut channels = Vec::new();
    let mut addr = first;
    let mut seen = Vec::new();

    while addr != 0 {
        if seen.contains(&addr) {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: "channel chain returns to a block it already visited".to_string(),
            });
        }
        seen.push(addr);
        if seen.len() > MAX_CHAIN {
            return Err(Mf4Error::CyclicLink {
                offset: addr as u64,
                chain: format!("channel chain exceeds {MAX_CHAIN} blocks"),
            });
        }

        let want =
            CnBlock::SIZE_DISPLAY_NAME.min((source.len().saturating_sub(addr as u64)) as usize);
        let bytes = source.read_bytes(addr as u64, want)?;
        let cn = CnBlock::parse(&bytes, addr as u64, big_endian)?;

        // The 32-character short name is the fallback, not the answer: a
        // channel whose name does not fit is written to a TX block, and using
        // the truncated form would make it unfindable by its real name.
        let name = read_tx(source, cn.long_name_addr, big_endian)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cn.short_name.clone());

        channels.push(Mdf3Channel {
            name,
            description: cn.description.clone(),
            unit: read_conversion_unit(source, cn.conversion_addr, big_endian).unwrap_or_default(),
            channel_type: cn.channel_type,
            start_offset: cn.start_offset,
            bit_count: cn.bit_count,
            data_type: cn.data_type,
            sampling_rate: cn.sampling_rate,
            additional_byte_offset: cn.additional_byte_offset,
            conversion_addr: cn.conversion_addr,
        });

        addr = cn.next_ch_addr;
    }

    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ByteSlice;

    struct MemSource(Vec<u8>);

    impl ByteSource for MemSource {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }

        fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
            let start = offset as usize;
            let end = start + len;
            if start > self.0.len() || end > self.0.len() {
                return Err(Mf4Error::TruncatedFile {
                    offset,
                    expected: len,
                    actual: self.0.len().saturating_sub(start),
                });
            }
            Ok(ByteSlice::borrowed(&self.0[start..end]))
        }
    }

    struct TestCh {
        name: &'static str,
        long_name: Option<&'static str>,
        description: &'static str,
        channel_type: u16,
        start_offset: u16,
        bit_count: u16,
        data_type: u16,
        conversion: Option<TestCc>,
    }

    enum TestCc {
        Linear { a: f64, b: f64, unit: &'static str },
        Tabular(Vec<(f64, f64)>, &'static str),
        TextTable(Vec<(f64, &'static str)>),
    }

    struct TestGrp {
        record_id: u16,
        record_size: u16,
        channels: Vec<TestCh>,
        records: Vec<Vec<u8>>,
        comment: Option<&'static str>,
    }

    fn put_u16(buf: &mut [u8], at: usize, v: u16, be: bool) {
        let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
        buf[at..at + 2].copy_from_slice(&b);
    }

    fn put_u32(buf: &mut [u8], at: usize, v: u32, be: bool) {
        let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
        buf[at..at + 4].copy_from_slice(&b);
    }

    fn put_u64(buf: &mut [u8], at: usize, v: u64, be: bool) {
        let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
        buf[at..at + 8].copy_from_slice(&b);
    }

    fn put_text(buf: &mut [u8], at: usize, len: usize, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(len);
        buf[at..at + n].copy_from_slice(&b[..n]);
    }

    fn push_tx(buf: &mut Vec<u8>, text: &str, be: bool) -> u32 {
        let addr = buf.len() as u32;
        let len = 4 + text.len() + 1;
        let mut tx = vec![0u8; len];
        tx[..2].copy_from_slice(b"TX");
        put_u16(&mut tx, 2, len as u16, be);
        tx[4..4 + text.len()].copy_from_slice(text.as_bytes());
        buf.extend_from_slice(&tx);
        addr
    }

    fn push_cc(buf: &mut Vec<u8>, cc: &TestCc, be: bool) -> u32 {
        let mut params = Vec::new();
        let push_f = |p: &mut Vec<u8>, v: f64| {
            let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
            p.extend_from_slice(&b);
        };
        let (code, count, unit) = match cc {
            TestCc::Linear { a, b, unit } => {
                push_f(&mut params, *b);
                push_f(&mut params, *a);
                (0u16, 2u16, *unit)
            }
            TestCc::Tabular(pairs, unit) => {
                for (raw, phys) in pairs {
                    push_f(&mut params, *raw);
                    push_f(&mut params, *phys);
                }
                (1u16, pairs.len() as u16, *unit)
            }
            TestCc::TextTable(pairs) => {
                for (raw, label) in pairs {
                    push_f(&mut params, *raw);
                    let mut field = [0u8; 32];
                    let b = label.as_bytes();
                    let n = b.len().min(32);
                    field[..n].copy_from_slice(&b[..n]);
                    params.extend_from_slice(&field);
                }
                (11u16, pairs.len() as u16, "")
            }
        };
        let addr = buf.len() as u32;
        let len = 46 + params.len();
        let mut cc_bytes = vec![0u8; 46];
        cc_bytes[..2].copy_from_slice(b"CC");
        put_u16(&mut cc_bytes, 2, len as u16, be);
        put_text(&mut cc_bytes, 22, 20, unit);
        put_u16(&mut cc_bytes, 42, code, be);
        put_u16(&mut cc_bytes, 44, count, be);
        cc_bytes.extend_from_slice(&params);
        buf.extend_from_slice(&cc_bytes);
        addr
    }

    fn build_test_mdf3(
        big_endian: bool,
        groups: &[TestGrp],
        file_comment: Option<&str>,
        abs_time: Option<u64>,
        tz_offset: Option<i16>,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 208];

        // ID block
        put_text(&mut buf, 0, 8, "MDF     ");
        put_text(&mut buf, 8, 8, "3.20    ");
        put_text(&mut buf, 16, 8, "falcon  ");
        put_u16(&mut buf, 24, if big_endian { 1 } else { 0 }, big_endian);
        put_u16(&mut buf, 26, 0, big_endian);
        put_u16(&mut buf, 28, 320, big_endian);

        // HD block
        buf[64..66].copy_from_slice(b"HD");
        put_u16(&mut buf, 66, 208, big_endian);
        put_u16(&mut buf, 64 + 16, groups.len() as u16, big_endian);
        put_text(&mut buf, 64 + 18, 10, "01:01:2025");
        put_text(&mut buf, 64 + 28, 8, "12:30:00");
        put_text(&mut buf, 64 + 36, 32, "Author");
        put_text(&mut buf, 64 + 68, 32, "Dept");
        put_text(&mut buf, 64 + 100, 32, "Project");
        put_text(&mut buf, 64 + 132, 32, "Subject");
        if let Some(ns) = abs_time {
            put_u64(&mut buf, 64 + 164, ns, big_endian);
        }
        if let Some(tz) = tz_offset {
            put_u16(&mut buf, 64 + 172, tz as u16, big_endian);
        }

        let comment_addr = file_comment
            .map(|c| push_tx(&mut buf, c, big_endian))
            .unwrap_or(0);
        put_u32(&mut buf, 64 + 8, comment_addr, big_endian);

        // Channels
        let mut first_ch_of_group = Vec::new();
        for g in groups {
            let mut next = 0u32;
            for ch in g.channels.iter().rev() {
                let cc_addr = match &ch.conversion {
                    Some(cc) => push_cc(&mut buf, cc, big_endian),
                    None => 0,
                };
                let long_name_addr = ch
                    .long_name
                    .map(|ln| push_tx(&mut buf, ln, big_endian))
                    .unwrap_or(0);
                let addr = buf.len() as u32;
                let mut cn = vec![0u8; 228];
                cn[..2].copy_from_slice(b"CN");
                put_u16(&mut cn, 2, 228, big_endian);
                put_u32(&mut cn, 4, next, big_endian);
                put_u32(&mut cn, 8, cc_addr, big_endian);
                put_u16(&mut cn, 24, ch.channel_type, big_endian);
                put_text(&mut cn, 26, 32, ch.name);
                put_text(&mut cn, 58, 128, ch.description);
                put_u16(&mut cn, 186, ch.start_offset, big_endian);
                put_u16(&mut cn, 188, ch.bit_count, big_endian);
                put_u16(&mut cn, 190, ch.data_type, big_endian);
                put_u32(&mut cn, 218, long_name_addr, big_endian);
                buf.extend_from_slice(&cn);
                next = addr;
            }
            first_ch_of_group.push(next);
        }

        // Channel groups
        let mut next_cg = 0u32;
        for (i, g) in groups.iter().enumerate().rev() {
            let cg_comment_addr = g
                .comment
                .map(|c| push_tx(&mut buf, c, big_endian))
                .unwrap_or(0);
            let addr = buf.len() as u32;
            let mut cg = vec![0u8; 26];
            cg[..2].copy_from_slice(b"CG");
            put_u16(&mut cg, 2, 26, big_endian);
            put_u32(&mut cg, 4, next_cg, big_endian);
            put_u32(&mut cg, 8, first_ch_of_group[i], big_endian);
            put_u32(&mut cg, 12, cg_comment_addr, big_endian);
            put_u16(&mut cg, 16, g.record_id, big_endian);
            put_u16(&mut cg, 18, g.channels.len() as u16, big_endian);
            put_u16(&mut cg, 20, g.record_size, big_endian);
            put_u32(&mut cg, 22, g.records.len() as u32, big_endian);
            buf.extend_from_slice(&cg);
            next_cg = addr;
        }

        let dg_addr = buf.len() as u32;
        let mut dg = vec![0u8; 28];
        dg[..2].copy_from_slice(b"DG");
        put_u16(&mut dg, 2, 28, big_endian);
        put_u32(&mut dg, 8, next_cg, big_endian);
        put_u16(&mut dg, 20, groups.len() as u16, big_endian);
        put_u16(&mut dg, 22, 0, big_endian); // sorted
        buf.extend_from_slice(&dg);

        let data_addr = buf.len() as u32;
        for g in groups {
            for rec in &g.records {
                buf.extend_from_slice(rec);
            }
        }

        put_u32(&mut buf, dg_addr as usize + 16, data_addr, big_endian);
        put_u32(&mut buf, 64 + 4, dg_addr, big_endian);
        buf
    }

    fn sample_dataset(be: bool) -> Vec<TestGrp> {
        let rec0 = {
            let mut r = Vec::new();
            let push_u16 = |buf: &mut Vec<u8>, v: u16| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_u32 = |buf: &mut Vec<u8>, v: u32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_u64 = |buf: &mut Vec<u8>, v: u64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i16 = |buf: &mut Vec<u8>, v: i16| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i32 = |buf: &mut Vec<u8>, v: i32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i64 = |buf: &mut Vec<u8>, v: i64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_f32 = |buf: &mut Vec<u8>, v: f32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_f64 = |buf: &mut Vec<u8>, v: f64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };

            push_f64(&mut r, 0.0);
            r.push(42u8);
            push_u16(&mut r, 1000);
            push_u32(&mut r, 100_000);
            push_u64(&mut r, 5_000_000_000);
            r.push(-10i8 as u8);
            push_i16(&mut r, -500);
            push_i32(&mut r, -50_000);
            push_i64(&mut r, -2_000_000_000);
            push_f32(&mut r, 3.75);
            push_f64(&mut r, 2.625);
            push_u16(&mut r, 1234);
            push_f64(&mut r, 5.0);
            push_f64(&mut r, 5.0);
            push_f64(&mut r, 1.0);
            r
        };

        let rec1 = {
            let mut r = Vec::new();
            let push_u16 = |buf: &mut Vec<u8>, v: u16| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_u32 = |buf: &mut Vec<u8>, v: u32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_u64 = |buf: &mut Vec<u8>, v: u64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i16 = |buf: &mut Vec<u8>, v: i16| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i32 = |buf: &mut Vec<u8>, v: i32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_i64 = |buf: &mut Vec<u8>, v: i64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_f32 = |buf: &mut Vec<u8>, v: f32| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };
            let push_f64 = |buf: &mut Vec<u8>, v: f64| {
                let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                buf.extend_from_slice(&b);
            };

            push_f64(&mut r, 1.0);
            r.push(84u8);
            push_u16(&mut r, 2000);
            push_u32(&mut r, 200_000);
            push_u64(&mut r, 10_000_000_000);
            r.push(20i8 as u8);
            push_i16(&mut r, 1000);
            push_i32(&mut r, 100_000);
            push_i64(&mut r, 4_000_000_000);
            push_f32(&mut r, -1.5);
            push_f64(&mut r, 100000.5);
            push_u16(&mut r, 5678);
            push_f64(&mut r, 10.0);
            push_f64(&mut r, 10.0);
            push_f64(&mut r, 0.0);
            r
        };

        vec![TestGrp {
            record_id: 0,
            record_size: 76,
            comment: Some("Main Channel Group"),
            channels: vec![
                TestCh {
                    name: "Time",
                    long_name: None,
                    description: "Time channel",
                    channel_type: 1,
                    start_offset: 0,
                    bit_count: 64,
                    data_type: 3,
                    conversion: None,
                },
                TestCh {
                    name: "U8",
                    long_name: None,
                    description: "8-bit unsigned",
                    channel_type: 0,
                    start_offset: 64,
                    bit_count: 8,
                    data_type: 0,
                    conversion: None,
                },
                TestCh {
                    name: "U16",
                    long_name: None,
                    description: "16-bit unsigned",
                    channel_type: 0,
                    start_offset: 72,
                    bit_count: 16,
                    data_type: 0,
                    conversion: None,
                },
                TestCh {
                    name: "U32",
                    long_name: None,
                    description: "32-bit unsigned",
                    channel_type: 0,
                    start_offset: 88,
                    bit_count: 32,
                    data_type: 0,
                    conversion: None,
                },
                TestCh {
                    name: "U64",
                    long_name: None,
                    description: "64-bit unsigned",
                    channel_type: 0,
                    start_offset: 120,
                    bit_count: 64,
                    data_type: 0,
                    conversion: None,
                },
                TestCh {
                    name: "I8",
                    long_name: None,
                    description: "8-bit signed",
                    channel_type: 0,
                    start_offset: 184,
                    bit_count: 8,
                    data_type: 1,
                    conversion: None,
                },
                TestCh {
                    name: "I16",
                    long_name: None,
                    description: "16-bit signed",
                    channel_type: 0,
                    start_offset: 192,
                    bit_count: 16,
                    data_type: 1,
                    conversion: None,
                },
                TestCh {
                    name: "I32",
                    long_name: None,
                    description: "32-bit signed",
                    channel_type: 0,
                    start_offset: 208,
                    bit_count: 32,
                    data_type: 1,
                    conversion: None,
                },
                TestCh {
                    name: "I64",
                    long_name: None,
                    description: "64-bit signed",
                    channel_type: 0,
                    start_offset: 240,
                    bit_count: 64,
                    data_type: 1,
                    conversion: None,
                },
                TestCh {
                    name: "F32",
                    long_name: None,
                    description: "32-bit float",
                    channel_type: 0,
                    start_offset: 304,
                    bit_count: 32,
                    data_type: 2,
                    conversion: None,
                },
                TestCh {
                    name: "F64",
                    long_name: None,
                    description: "64-bit float",
                    channel_type: 0,
                    start_offset: 336,
                    bit_count: 64,
                    data_type: 3,
                    conversion: None,
                },
                TestCh {
                    name: "LongNameCh",
                    long_name: Some("ThisIsAChannelWithAVeryLongNameExceedingThirtyTwoCharacters"),
                    description: "Channel with long name",
                    channel_type: 0,
                    start_offset: 400,
                    bit_count: 16,
                    data_type: 0,
                    conversion: None,
                },
                TestCh {
                    name: "LinChan",
                    long_name: None,
                    description: "Linear conversion channel",
                    channel_type: 0,
                    start_offset: 416,
                    bit_count: 64,
                    data_type: 3,
                    conversion: Some(TestCc::Linear {
                        a: 2.0,
                        b: 10.0,
                        unit: "km/h",
                    }),
                },
                TestCh {
                    name: "TabChan",
                    long_name: None,
                    description: "Tabular conversion channel",
                    channel_type: 0,
                    start_offset: 480,
                    bit_count: 64,
                    data_type: 3,
                    conversion: Some(TestCc::Tabular(vec![(0.0, 0.0), (10.0, 100.0)], "m/s")),
                },
                TestCh {
                    name: "TextChan",
                    long_name: None,
                    description: "Text table conversion channel",
                    channel_type: 0,
                    start_offset: 544,
                    bit_count: 64,
                    data_type: 3,
                    conversion: Some(TestCc::TextTable(vec![(0.0, "Off"), (1.0, "On")])),
                },
            ],
            records: vec![rec0, rec1],
        }]
    }

    #[test]
    fn test_synthetic_big_endian_file_hand_stated_values() {
        let groups = sample_dataset(true);
        let bytes = build_test_mdf3(
            true,
            &groups,
            Some("Synthetic Big-Endian MDF Test File"),
            Some(1_700_000_000_000_000_000),
            Some(60),
        );

        let file = Mdf3File::from_source(Arc::new(MemSource(bytes)))
            .expect("should open synthetic big-endian file");

        assert_eq!(file.version(), "3.20");
        assert_eq!(file.version_number(), 320);
        assert_eq!(file.comment(), "Synthetic Big-Endian MDF Test File");
        assert_eq!(file.start_time_ns(), Some(1_700_000_000_000_000_000));
        assert_eq!(file.header().tz_offset_minutes, Some(60));
        assert_eq!(
            file.data_groups()[0].channel_groups[0].comment,
            "Main Channel Group"
        );

        // Raw channel values checked against hand-stated numbers
        assert_eq!(
            file.values_by_name("Time").unwrap(),
            SignalValues::F64(vec![0.0, 1.0])
        );
        assert_eq!(
            file.values_by_name("U8").unwrap(),
            SignalValues::U8(vec![42, 84])
        );
        assert_eq!(
            file.values_by_name("U16").unwrap(),
            SignalValues::U16(vec![1000, 2000])
        );
        assert_eq!(
            file.values_by_name("U32").unwrap(),
            SignalValues::U32(vec![100_000, 200_000])
        );
        assert_eq!(
            file.values_by_name("U64").unwrap(),
            SignalValues::U64(vec![5_000_000_000, 10_000_000_000])
        );
        assert_eq!(
            file.values_by_name("I8").unwrap(),
            SignalValues::I8(vec![-10, 20])
        );
        assert_eq!(
            file.values_by_name("I16").unwrap(),
            SignalValues::I16(vec![-500, 1000])
        );
        assert_eq!(
            file.values_by_name("I32").unwrap(),
            SignalValues::I32(vec![-50_000, 100_000])
        );
        assert_eq!(
            file.values_by_name("I64").unwrap(),
            SignalValues::I64(vec![-2_000_000_000, 4_000_000_000])
        );
        assert_eq!(
            file.values_by_name("F32").unwrap(),
            SignalValues::F32(vec![3.75, -1.5])
        );
        assert_eq!(
            file.values_by_name("F64").unwrap(),
            SignalValues::F64(vec![2.625, 100000.5])
        );
        assert_eq!(
            file.values_by_name("ThisIsAChannelWithAVeryLongNameExceedingThirtyTwoCharacters")
                .unwrap(),
            SignalValues::U16(vec![1234, 5678])
        );

        // Conversions and physical values
        let lin_ch = file.find_channel("LinChan").unwrap();
        assert_eq!(lin_ch.unit, "km/h");
        assert_eq!(
            file.physical_by_name("LinChan").unwrap(),
            SignalValues::F64(vec![20.0, 30.0])
        );

        let tab_ch = file.find_channel("TabChan").unwrap();
        assert_eq!(tab_ch.unit, "m/s");
        assert_eq!(
            file.physical_by_name("TabChan").unwrap(),
            SignalValues::F64(vec![50.0, 100.0])
        );

        assert_eq!(
            file.physical_by_name("TextChan").unwrap(),
            SignalValues::Str(vec!["On".to_string(), "Off".to_string()])
        );
    }

    #[test]
    fn test_identical_logical_file_le_and_be_decode_to_identical_values() {
        let groups_be = sample_dataset(true);
        let bytes_be = build_test_mdf3(
            true,
            &groups_be,
            Some("Shared Comment"),
            Some(1_234_567_890_000_000),
            Some(-120),
        );

        let groups_le = sample_dataset(false);
        let bytes_le = build_test_mdf3(
            false,
            &groups_le,
            Some("Shared Comment"),
            Some(1_234_567_890_000_000),
            Some(-120),
        );

        let file_be = Mdf3File::from_source(Arc::new(MemSource(bytes_be)))
            .expect("should open big-endian file");
        let file_le = Mdf3File::from_source(Arc::new(MemSource(bytes_le)))
            .expect("should open little-endian file");

        assert_eq!(file_be.version(), file_le.version());
        assert_eq!(file_be.version_number(), file_le.version_number());
        assert_eq!(file_be.comment(), file_le.comment());
        assert_eq!(file_be.start_time_ns(), file_le.start_time_ns());
        assert_eq!(
            file_be.header().tz_offset_minutes,
            file_le.header().tz_offset_minutes
        );
        assert_eq!(file_be.channel_count(), file_le.channel_count());
        assert_eq!(file_be.channel_names(), file_le.channel_names());

        for name in file_be.channel_names() {
            let ch_be = file_be.find_channel(name).unwrap();
            let ch_le = file_le.find_channel(name).unwrap();
            assert_eq!(ch_be.unit, ch_le.unit, "unit mismatch for {name}");
            assert_eq!(
                ch_be.description, ch_le.description,
                "desc mismatch for {name}"
            );
            assert_eq!(
                ch_be.channel_type, ch_le.channel_type,
                "type mismatch for {name}"
            );

            let raw_be = file_be.values_by_name(name).unwrap();
            let raw_le = file_le.values_by_name(name).unwrap();
            assert_eq!(raw_be, raw_le, "raw sample mismatch for {name}");

            let phys_be = file_be.physical_by_name(name).unwrap();
            let phys_le = file_le.physical_by_name(name).unwrap();
            assert_eq!(phys_be, phys_le, "phys sample mismatch for {name}");
        }
    }

    #[test]
    fn test_malformed_big_endian_files_error_by_name() {
        let groups = sample_dataset(true);
        let bytes = build_test_mdf3(
            true,
            &groups,
            Some("Test"),
            Some(1_700_000_000_000_000_000),
            Some(0),
        );

        // Corrupt HD block magic
        let mut bad_hd = bytes.clone();
        bad_hd[64] = b'X';
        bad_hd[65] = b'X';
        let err = match Mdf3File::from_source(Arc::new(MemSource(bad_hd))) {
            Ok(_) => panic!("corrupt HD block should fail"),
            Err(e) => e,
        };
        assert!(
            matches!(err, Mf4Error::InvalidBlockId { .. }),
            "should error on bad HD id: {err}"
        );
        assert!(err.to_string().contains("HD"), "{err}");

        // Truncated file in BE
        let short_bytes = bytes[..100].to_vec();
        let err_trunc = match Mdf3File::from_source(Arc::new(MemSource(short_bytes))) {
            Ok(_) => panic!("truncated file should fail"),
            Err(e) => e,
        };
        assert!(
            matches!(err_trunc, Mf4Error::TruncatedFile { .. }),
            "should error on truncated BE file: {err_trunc}"
        );
    }
}
