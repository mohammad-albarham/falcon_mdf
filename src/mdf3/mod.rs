//! Reading MDF 3.x files.
//!
//! This is the structure of a version 3 file — what it is, when it was taken,
//! and which channels it holds — and the raw samples of those channels, in
//! each channel's own type. Applying a channel's conversion to turn raw
//! samples into physical ones is not here yet.
//!
//! Version 3 is a different format from version 4 rather than an older
//! spelling of it, so it has its own module tree. See [`blocks`] for why.

pub mod blocks;
pub mod records;

use std::path::Path;
use std::sync::Arc;

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSource, IoBackend};
use crate::model::SignalValues;

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

        // Big-endian v3 files exist, but every offset and every sample in one
        // is read differently. Saying so is honest; reading it little-endian
        // would return numbers that look like measurements.
        if id.big_endian {
            return Err(Mf4Error::unsupported(
                "big-endian MDF 3.x",
                "this file declares big-endian storage, which this build does not read",
            ));
        }

        let hd_offset = IdBlock::SIZE as u64;
        let hd_len = (HdBlock::COMMON_SIZE + HdBlock::POST_320_EXTRA_SIZE)
            .min((source.len().saturating_sub(hd_offset)) as usize);
        let hd_bytes = source.read_bytes(hd_offset, hd_len)?;
        let header = HdBlock::parse(&hd_bytes, hd_offset)?;

        let comment = read_tx(source.as_ref(), header.comment_addr).unwrap_or_default();

        let data_groups = walk_data_groups(source.as_ref(), &header)?;

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
        for (g, dg) in self.data_groups.iter().enumerate() {
            for (c, cg) in dg.channel_groups.iter().enumerate() {
                if let Some(i) = cg.channels.iter().position(|ch| ch.name == name) {
                    return self.channel_values(g, c, i);
                }
            }
        }
        Err(Mf4Error::ChannelNotFound {
            name: name.to_string(),
        })
    }

    /// The bytes backing this file, for callers that need them directly.
    pub fn source(&self) -> &Arc<dyn ByteSource> {
        &self.source
    }
}

/// Reads a `TX` block's text, returning `None` for a null link.
fn read_tx(source: &dyn ByteSource, addr: u32) -> Option<String> {
    if addr == 0 {
        return None;
    }
    // The length lives in the block, so read the header first rather than
    // guessing a size and reading past the end of the file.
    let head = source.read_bytes(addr as u64, 4).ok()?;
    let len = u16::from_le_bytes([head[2], head[3]]) as usize;
    if len < 4 {
        return None;
    }
    let bytes = source.read_bytes(addr as u64, len).ok()?;
    blocks::parse_tx(&bytes, addr as u64).ok()
}

/// Reads the unit out of a conversion block.
///
/// The unit sits at a fixed offset in every conversion type, so it can be read
/// without knowing which type this is — which is deliberate, since applying
/// the conversion is a later task.
fn read_conversion_unit(source: &dyn ByteSource, addr: u32) -> Option<String> {
    if addr == 0 {
        return None;
    }
    let head = source.read_bytes(addr as u64, 4).ok()?;
    if &head[..2] != b"CC" {
        return None;
    }
    let len = u16::from_le_bytes([head[2], head[3]]) as usize;
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
fn walk_data_groups(source: &dyn ByteSource, header: &HdBlock) -> Result<Vec<Mdf3DataGroup>> {
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

        let bytes = source.read_bytes(addr as u64, DgBlock::SIZE_POST_320.min(
            (source.len().saturating_sub(addr as u64)) as usize,
        ))?;
        let dg = DgBlock::parse(&bytes, addr as u64)?;

        let channel_groups = walk_channel_groups(source, dg.first_cg_addr)?;

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
fn walk_channel_groups(source: &dyn ByteSource, first: u32) -> Result<Vec<Mdf3ChannelGroup>> {
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
        let cg = CgBlock::parse(&bytes, addr as u64)?;

        let channels = walk_channels(source, cg.first_ch_addr)?;

        groups.push(Mdf3ChannelGroup {
            record_id: cg.record_id,
            record_size: cg.record_size,
            cycle_count: cg.cycle_count,
            comment: read_tx(source, cg.comment_addr).unwrap_or_default(),
            channels,
        });

        addr = cg.next_cg_addr;
    }

    Ok(groups)
}

/// Walks one channel group's channel chain.
fn walk_channels(source: &dyn ByteSource, first: u32) -> Result<Vec<Mdf3Channel>> {
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

        let want = CnBlock::SIZE_DISPLAY_NAME
            .min((source.len().saturating_sub(addr as u64)) as usize);
        let bytes = source.read_bytes(addr as u64, want)?;
        let cn = CnBlock::parse(&bytes, addr as u64)?;

        // The 32-character short name is the fallback, not the answer: a
        // channel whose name does not fit is written to a TX block, and using
        // the truncated form would make it unfindable by its real name.
        let name = read_tx(source, cn.long_name_addr)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cn.short_name.clone());

        channels.push(Mdf3Channel {
            name,
            description: cn.description.clone(),
            unit: read_conversion_unit(source, cn.conversion_addr).unwrap_or_default(),
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
