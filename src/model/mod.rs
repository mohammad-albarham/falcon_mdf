//! High-level, version-agnostic data model.
//!
//! This module provides user-friendly types that abstract away the
//! complexities of the MF4 file format. These types are what users
//! interact with when using the library.

pub mod metadata;
pub mod signal;
pub mod time;
pub mod values;
pub(crate) mod vlsd;

pub use metadata::*;
pub use signal::*;
pub use time::*;
pub use values::*;
pub(crate) use vlsd::*;

use crate::blocks::source::SourceInfo;
use crate::blocks::{ChannelType, Conversion, ConversionOutput, DataType, SyncType};
use crate::data_index::{DataBlockIndex, RecordIndex};

/// A data group in the measurement file.
///
/// Data groups contain one or more channel groups that share the same
/// underlying data storage.
#[derive(Debug, Clone)]
pub struct DataGroup {
    /// Unique identifier for this data group.
    pub id: usize,
    /// Index of this data group in the file.
    pub index: usize,
    /// Channel groups in this data group.
    pub channel_groups: Vec<ChannelGroup>,
    /// Comment/description.
    pub comment: String,
    /// File offset of the DG block.
    ///
    /// Retained for the unsorted-record demultiplexer (plan Phase 1.1) and for
    /// diagnostics; not read on the current code path.
    #[allow(dead_code)]
    pub(crate) dg_offset: u64,
    /// File offset of the data block.
    ///
    /// Retained for the unsorted-record demultiplexer (plan Phase 1.1); not
    /// read on the current code path.
    #[allow(dead_code)]
    pub(crate) data_offset: u64,
    /// Record ID size (0 = no record IDs).
    pub(crate) rec_id_size: u8,
    /// Index of data blocks for lazy loading.
    pub(crate) data_block_index: DataBlockIndex,
    /// Positions of each channel group's records, for unsorted data groups.
    ///
    /// `None` when the data group is sorted, in which case records are a single
    /// fixed-size stride and no index is needed.
    pub(crate) record_index: Option<RecordIndex>,
}

impl DataGroup {
    /// Returns the number of channel groups in this data group.
    pub fn channel_group_count(&self) -> usize {
        self.channel_groups.len()
    }

    /// Returns an iterator over all channels in all channel groups.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channel_groups.iter().flat_map(|cg| cg.channels.iter())
    }

    /// Finds a channel by name across all channel groups.
    pub fn find_channel(&self, name: &str) -> Option<&Channel> {
        self.channels().find(|ch| ch.name == name)
    }

    /// Returns the index of the blocks holding this group's data.
    pub fn block_index(&self) -> &DataBlockIndex {
        &self.data_block_index
    }

    /// Returns true if records from several channel groups are interleaved in
    /// this group's data stream.
    pub fn is_unsorted(&self) -> bool {
        self.record_index.is_some()
    }

    /// Returns the size in bytes of the record ID prefixing each record, or 0
    /// when this data group's records carry none.
    pub fn rec_id_size(&self) -> u8 {
        self.rec_id_size
    }
}

/// A channel group containing channels sampled together.
///
/// All channels in a channel group share the same time axis and
/// have the same number of samples.
#[derive(Debug, Clone)]
pub struct ChannelGroup {
    /// Unique identifier for this channel group.
    pub id: usize,
    /// Index of this channel group within its data group.
    pub index: usize,
    /// Data group index this channel group belongs to.
    pub data_group_index: usize,
    /// Acquisition name.
    pub acquisition_name: String,
    /// Number of samples (cycles).
    pub sample_count: u64,
    /// Channels in this group.
    pub channels: Vec<Channel>,
    /// Source information.
    pub source: Option<SourceInfo>,
    /// Comment/description.
    pub comment: String,
    /// Record ID for this channel group.
    pub(crate) record_id: u64,
    /// Size of one data record in bytes.
    pub(crate) data_bytes: u32,
    /// Size of invalidation bits in bytes.
    pub(crate) inval_bytes: u32,
    /// File offset of the CG block, used to match a variable-length channel to
    /// the group holding its payloads.
    pub(crate) cg_offset: u64,
    /// Whether this is a VLSD (Variable Length Signal Data) channel group.
    pub(crate) is_vlsd: bool,
}

impl ChannelGroup {
    /// Returns the number of channels in this group.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Finds the master (time) channel in this group.
    pub fn master_channel(&self) -> Option<&Channel> {
        self.channels.iter().find(|ch| ch.is_master())
    }

    /// Finds a channel by name.
    pub fn find_channel(&self, name: &str) -> Option<&Channel> {
        self.channels.iter().find(|ch| ch.name == name)
    }

    /// Returns the total record size including record ID and invalidation bytes.
    pub fn record_size(&self, rec_id_size: u8) -> usize {
        rec_id_size as usize + self.payload_size()
    }

    /// Returns the number of channel-data bytes per record, excluding both the
    /// record ID and the invalidation bytes.
    pub fn data_bytes_len(&self) -> usize {
        self.data_bytes as usize
    }

    /// Returns the number of invalidation bytes per record.
    pub fn inval_bytes_len(&self) -> usize {
        self.inval_bytes as usize
    }

    /// Returns the record ID identifying this group within an unsorted stream.
    pub fn record_id(&self) -> u64 {
        self.record_id
    }

    /// Returns true if this group stores variable-length signal data rather
    /// than channel records.
    pub fn is_vlsd(&self) -> bool {
        self.is_vlsd
    }

    /// Returns true if this group's block sits at `offset` in the file, which is
    /// how a variable-length channel names the group holding its payloads.
    pub fn matches_offset(&self, offset: u64) -> bool {
        offset != 0 && self.cg_offset == offset
    }

    /// Returns the size of a record's own bytes, excluding any record ID.
    ///
    /// This is the stride that applies once records have been separated from an
    /// interleaved stream, where the record ID is no longer present.
    pub fn payload_size(&self) -> usize {
        self.data_bytes as usize + self.inval_bytes as usize
    }
}

/// Why a channel present in a file cannot be decoded by this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnreadableReason {
    /// The channel holds an array. Its values are described by a channel array
    /// (CA) block, which this version does not expand — and the channel's own
    /// record field is only the first element, so reading it would silently
    /// return a fraction of the data.
    ArrayComposition,
}

impl UnreadableReason {
    /// Returns a short explanation, used in the error a read produces.
    pub fn detail(&self) -> &'static str {
        match self {
            UnreadableReason::ArrayComposition => {
                "the channel holds an array described by a CA block, which is not expanded; \
                 its record field is only the first element"
            }
        }
    }
}

impl std::fmt::Display for UnreadableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.detail())
    }
}

/// A channel representing a single signal.
///
/// Channels define individual measurement signals with their
/// data type, unit, and conversion information.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Unique identifier for this channel.
    pub id: usize,
    /// Index of this channel within its channel group.
    pub index: usize,
    /// Channel group index this channel belongs to.
    pub channel_group_index: usize,
    /// Data group index this channel belongs to.
    pub data_group_index: usize,
    /// Channel name.
    pub name: String,
    /// Physical unit (e.g., "m/s", "°C").
    pub unit: String,
    /// Channel type.
    pub channel_type: ChannelType,
    /// Synchronization type (for master channels).
    pub sync_type: SyncType,
    /// Raw data type.
    pub data_type: DataType,
    /// Value conversion formula.
    pub conversion: Conversion,
    /// Number of bits in the raw value.
    pub bit_count: u32,
    /// Byte offset within the record.
    pub byte_offset: u32,
    /// Bit offset within the byte.
    pub bit_offset: u8,
    /// Whether this channel carries a per-sample invalidation bit.
    pub invalidation_bit: bool,
    /// Position of this channel's invalidation bit within the record's
    /// invalidation bytes. Only meaningful when `invalidation_bit` is set.
    pub inval_bit_pos: u32,
    /// Comment/description.
    pub comment: String,
    /// Source information.
    pub source: Option<SourceInfo>,
    /// Minimum physical value (if defined).
    pub min_value: Option<f64>,
    /// Maximum physical value (if defined).
    pub max_value: Option<f64>,
    /// File offset of the CN block.
    ///
    /// Retained for diagnostics and for array support (plan Phase 4).
    #[allow(dead_code)]
    pub(crate) cn_offset: u64,
    /// Link to this channel's own data block (`cn_data`), where a
    /// variable-length channel's payloads live. Zero when absent.
    pub(crate) data_link: u64,
    /// Why this channel cannot be read, when it cannot.
    ///
    /// A channel this build cannot decode stays in the channel list — it exists
    /// in the file and hiding it would misrepresent the file's contents — but
    /// reading it fails rather than returning part of the data.
    pub(crate) unreadable: Option<UnreadableReason>,
}

impl Channel {
    /// Returns true if this is a master (time/angle/distance) channel.
    pub fn is_master(&self) -> bool {
        self.channel_type.is_master()
    }

    /// Returns true if this is a time channel.
    pub fn is_time_channel(&self) -> bool {
        self.is_master() && self.sync_type == SyncType::Time
    }

    /// Returns the byte size of this channel's raw value.
    pub fn byte_size(&self) -> usize {
        self.bit_count.div_ceil(8) as usize
    }

    /// Returns true if this channel has numeric data.
    pub fn is_numeric(&self) -> bool {
        self.data_type.is_numeric()
    }

    /// Returns true if this channel's data is little-endian.
    pub fn is_little_endian(&self) -> bool {
        self.data_type.is_little_endian()
    }

    /// Returns true if this channel contains signed integers.
    pub fn is_signed(&self) -> bool {
        matches!(self.data_type, DataType::IntLe | DataType::IntBe)
    }

    /// Returns true if this channel contains floating-point values.
    pub fn is_float(&self) -> bool {
        matches!(self.data_type, DataType::FloatLe | DataType::FloatBe)
    }

    /// Returns the link to this channel's own data block, used by
    /// variable-length channels to locate their payloads. Zero when absent.
    pub fn data_link(&self) -> u64 {
        self.data_link
    }

    /// Converts a raw value to a physical value using this channel's conversion.
    pub fn convert(&self, raw: f64) -> f64 {
        self.conversion.convert(raw)
    }

    /// Returns why this channel cannot be read, or `None` when it can.
    pub fn unreadable(&self) -> Option<UnreadableReason> {
        self.unreadable
    }

    /// Returns the Rust type this channel's samples decode to.
    ///
    /// Integer widths follow the channel's bit count, so a 29-bit CAN identifier
    /// decodes to `u32` and a 2-bit bus number to `u8`. A channel carrying any
    /// non-identity conversion decodes to [`ValueKind::F64`], since conversions
    /// yield physical values.
    pub fn value_kind(&self) -> ValueKind {
        match self.conversion.output() {
            // A text table turns numbers into labels, so the channel reads as text
            // regardless of how its raw bits are stored.
            ConversionOutput::Text => return ValueKind::Str,
            // The kind is unknowable until the conversion can be evaluated;
            // reading such a channel fails, so the reported kind is not used.
            ConversionOutput::Unsupported => return ValueKind::F64,
            ConversionOutput::Numeric => {}
        }

        if !self.conversion.is_identity() {
            return ValueKind::F64;
        }

        let bits = self.bit_count;
        match self.data_type {
            DataType::UIntLe | DataType::UIntBe => {
                if bits <= 8 {
                    ValueKind::U8
                } else if bits <= 16 {
                    ValueKind::U16
                } else if bits <= 32 {
                    ValueKind::U32
                } else {
                    ValueKind::U64
                }
            }
            DataType::IntLe | DataType::IntBe => {
                if bits <= 8 {
                    ValueKind::I8
                } else if bits <= 16 {
                    ValueKind::I16
                } else if bits <= 32 {
                    ValueKind::I32
                } else {
                    ValueKind::I64
                }
            }
            DataType::FloatLe | DataType::FloatBe => {
                if bits <= 32 {
                    ValueKind::F32
                } else {
                    ValueKind::F64
                }
            }
            DataType::StringUtf8 | DataType::StringUtf16Le | DataType::StringUtf16Be => {
                ValueKind::Str
            }
            // Byte arrays, MIME payloads, CANopen date/time and complex numbers
            // are all fixed-width blobs with no scalar interpretation.
            _ => ValueKind::Bytes,
        }
    }
}

/// Statistics about the file structure.
#[derive(Debug, Clone, Default)]
pub struct FileStatistics {
    /// Total number of data groups.
    pub data_group_count: usize,
    /// Total number of channel groups.
    pub channel_group_count: usize,
    /// Total number of channels.
    pub channel_count: usize,
    /// Total number of samples across all channels.
    pub total_sample_count: u64,
    /// File size in bytes.
    pub file_size: u64,
}

impl FileStatistics {
    /// Computes statistics from data groups.
    pub fn from_data_groups(data_groups: &[DataGroup], file_size: u64) -> Self {
        let mut stats = FileStatistics {
            file_size,
            ..Default::default()
        };

        stats.data_group_count = data_groups.len();

        for dg in data_groups {
            stats.channel_group_count += dg.channel_groups.len();
            for cg in &dg.channel_groups {
                stats.channel_count += cg.channels.len();
                stats.total_sample_count += cg.sample_count;
            }
        }

        stats
    }
}
