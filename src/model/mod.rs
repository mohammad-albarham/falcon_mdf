//! High-level, version-agnostic data model.
//!
//! This module provides user-friendly types that abstract away the
//! complexities of the MF4 file format. These types are what users
//! interact with when using the library.

pub mod signal;
pub mod time;

pub use signal::*;
pub use time::*;

use crate::blocks::{ChannelType, DataType, SyncType, Conversion};
use crate::blocks::source::SourceInfo;

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
    pub(crate) dg_offset: u64,
    /// File offset of the data block.
    pub(crate) data_offset: u64,
    /// Record ID size (0 = no record IDs).
    pub(crate) rec_id_size: u8,
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
    /// File offset of the CG block.
    pub(crate) cg_offset: u64,
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
        rec_id_size as usize + self.data_bytes as usize + self.inval_bytes as usize
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
    /// Comment/description.
    pub comment: String,
    /// Source information.
    pub source: Option<SourceInfo>,
    /// Minimum physical value (if defined).
    pub min_value: Option<f64>,
    /// Maximum physical value (if defined).
    pub max_value: Option<f64>,
    /// File offset of the CN block.
    pub(crate) cn_offset: u64,
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
        ((self.bit_count + 7) / 8) as usize
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

    /// Converts a raw value to a physical value using this channel's conversion.
    pub fn convert(&self, raw: f64) -> f64 {
        self.conversion.convert(raw)
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
