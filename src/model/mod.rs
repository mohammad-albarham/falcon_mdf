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
use crate::blocks::{
    AxisRef, CaStorage, ChannelType, Conversion, ConversionInput, ConversionOutput, DataType,
    SyncType,
};
use crate::blocks::{ChElement, ChType, EvCause, EvRangeType, EvSyncType, EventType, SrSyncType};
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
    /// Used to confirm that a dynamic-size array's companion channel (named by
    /// a (dg, cg, cn) triple) lives in this same data group, and retained for
    /// diagnostics beyond that.
    pub(crate) dg_offset: u64,
    /// File offset of the data block.
    ///
    /// Exposed through [`DataGroup::data_block_offset`], and retained for the
    /// unsorted-record demultiplexer (plan Phase 1.1).
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

    /// Returns the file offset of this group's DG block.
    ///
    /// The address is what [`crate::inspect::BlockMap`] keys blocks by, so
    /// this is the link between the measurement view of a file and the block
    /// view of it.
    pub fn block_offset(&self) -> u64 {
        self.dg_offset
    }

    /// Returns the file offset of this group's data block, or 0 when the
    /// group carries no data.
    ///
    /// The block there may be a plain `##DT`, a compressed `##DZ`, or a
    /// `##DL`/`##HL` list heading a chain of them.
    pub fn data_block_offset(&self) -> u64 {
        self.data_offset
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
    /// Whether this group holds bus events — logged CAN, LIN or FlexRay
    /// frames — rather than measurements.
    pub(crate) bus_event: bool,
    /// Whether this group holds nothing but bus events, with no decoded signal
    /// channels alongside them.
    pub(crate) plain_bus_event: bool,
    /// Sample-reduction levels attached to this group, coarsest last.
    pub(crate) sample_reductions: Vec<SampleReduction>,
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

    /// Returns the sample-reduction levels attached to this group.
    ///
    /// Each describes a condensed view of the group's data. The reduced values
    /// themselves cannot be read — see [`SampleReduction`].
    pub fn sample_reductions(&self) -> &[SampleReduction] {
        &self.sample_reductions
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

    /// Returns true if this group holds logged bus traffic rather than
    /// measurements.
    ///
    /// The frames themselves are read with [`crate::Mf4File::can_frames`] when
    /// the traffic is CAN.
    pub fn is_bus_event(&self) -> bool {
        self.bus_event
    }

    /// Returns true if this group holds nothing but bus events.
    ///
    /// The flag says no decoded signal channels share the group with the raw
    /// frames; it does not mean the frame fields themselves are stored in some
    /// reduced form. Every CANedge log in the test corpus sets it while
    /// carrying the full set of composition channels, so it is no obstacle to
    /// [`crate::Mf4File::can_frames`] and should not be used to decide whether
    /// a group is readable.
    pub fn is_plain_bus_event(&self) -> bool {
        self.plain_bus_event
    }

    /// Returns true if this group's block sits at `offset` in the file, which is
    /// how a variable-length channel names the group holding its payloads.
    pub fn matches_offset(&self, offset: u64) -> bool {
        offset != 0 && self.cg_offset == offset
    }

    /// Returns the file offset of this group's CG block.
    ///
    /// See [`DataGroup::block_offset`] for why an address is what is offered.
    pub fn block_offset(&self) -> u64 {
        self.cg_offset
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
    /// The channel's CA block declares a storage form or dimension layout
    /// this build cannot resolve into records.
    ArrayGroupTemplate,
    /// The channel's CA block declares more than one dynamic-size dimension,
    /// or a dimension whose real per-sample size is named by a channel outside
    /// this record. `ca_dim_size` is then only the largest shape a sample may
    /// take, and this build has no honest way to learn the real one.
    ArrayDynamicSize,
    /// The channel's CA block composes with something this build cannot
    /// resolve into elements: a link naming neither a template CN nor another
    /// CA block, or a chain of composed CA blocks nested deeper than
    /// composition is ever expected to go.
    ArrayComposition,
    /// The channel is a synchronisation channel (`cn_type` 4): it indexes a
    /// media stream rather than carrying measurements, so its record bits are
    /// positions into the stream, not samples. Parsing can tell this without
    /// decoding, so it is reported before any read is attempted.
    SyncChannel,
}

impl UnreadableReason {
    /// Returns a short explanation, used in the error a read produces.
    pub fn detail(&self) -> &'static str {
        match self {
            UnreadableReason::ArrayGroupTemplate => {
                "the channel holds an array whose elements are stored one per channel \
                 group or one per data group rather than adjacently in this record"
            }
            UnreadableReason::ArrayDynamicSize => {
                "the channel holds a dynamic-size array whose real per-sample size cannot \
                 be resolved from this record alone"
            }
            UnreadableReason::ArrayComposition => {
                "the channel holds an array composed with something this build cannot \
                 resolve into elements"
            }
            UnreadableReason::SyncChannel => {
                "the channel indexes a media stream rather than carrying samples"
            }
        }
    }

    /// Returns the feature name the read error reports for this reason.
    pub fn feature(&self) -> &'static str {
        match self {
            UnreadableReason::SyncChannel => "synchronisation channel",
            UnreadableReason::ArrayGroupTemplate
            | UnreadableReason::ArrayDynamicSize
            | UnreadableReason::ArrayComposition => "channel array (CA)",
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
    /// Whether the file declares every sample of this channel invalid.
    ///
    /// `cn_flags` bit 0, and independent of everything below: a channel can
    /// carry this without an invalidation bit of its own and without its group
    /// reserving any invalidation bytes.
    pub all_invalid: bool,
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
    /// Number of samples in this channel's group.
    ///
    /// The count lives on the group — every channel in it shares it — but
    /// code holding a bare `&Channel` from `find_channel` or
    /// [`crate::Mf4File::channels_matching`] would otherwise have to index
    /// back into `data_groups()[dg][cg]` just to display it. Filled in when
    /// the file is opened, after the group's declared count is corrected
    /// against what the data actually holds, so it is the same number
    /// [`ChannelGroup::sample_count`] reports.
    pub sample_count: u64,
    /// File offset of the CN block.
    ///
    /// Exposed through [`Channel::block_offset`], and retained for
    /// diagnostics and for array support (plan Phase 4).
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
    /// Shape of this channel's array, when it is an array channel.
    ///
    /// `None` for scalar channels. For array channels, this holds the CA
    /// block's dimension sizes, e.g. `[3]` for a 3-element 1-D array or
    /// `[2, 4]` for a 2×4 matrix.
    pub array_shape: Option<Vec<u64>>,
    /// Element layout for an array channel, parsed from the CA block's
    /// template CN block. `None` for scalar channels or array channels
    /// whose element layout this build does not decode.
    pub(crate) array_element: Option<ArrayElement>,
    /// Where a dynamic-size array's one real per-sample count is stored, when
    /// `array_shape` is a maximum rather than a fixed shape. `None` for every
    /// other channel, including a dynamic-size array whose size cannot be
    /// resolved (which stays unreadable instead).
    pub(crate) array_dynamic_size: Option<AxisRef>,
}

impl Channel {
    /// Creates a synthetic channel descriptor for in-memory time series.
    pub fn synthetic(name: impl Into<String>, unit: impl Into<String>) -> Self {
        Self {
            id: 0,
            index: 0,
            channel_group_index: 0,
            data_group_index: 0,
            name: name.into(),
            unit: unit.into(),
            channel_type: ChannelType::FixedLength,
            sync_type: SyncType::None,
            data_type: DataType::FloatLe,
            conversion: Conversion::None,
            bit_count: 64,
            byte_offset: 0,
            bit_offset: 0,
            all_invalid: false,
            invalidation_bit: false,
            inval_bit_pos: 0,
            comment: String::new(),
            source: None,
            min_value: None,
            max_value: None,
            sample_count: 0,
            cn_offset: 0,
            data_link: 0,
            unreadable: None,
            array_shape: None,
            array_element: None,
            array_dynamic_size: None,
        }
    }

    /// Returns true if this is a master (time/angle/distance) channel.
    pub fn is_master(&self) -> bool {
        self.channel_type.is_master()
    }

    /// Returns true if this is a time channel.
    pub fn is_time_channel(&self) -> bool {
        self.is_master() && self.sync_type == SyncType::Time
    }

    /// Returns the file offset of this channel's CN block.
    ///
    /// See [`DataGroup::block_offset`] for why an address is what is offered.
    pub fn block_offset(&self) -> u64 {
        self.cn_offset
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
        self.conversion.convert(raw, self.is_float())
    }

    /// Returns why this channel cannot be read, or `None` when it can.
    pub fn unreadable(&self) -> Option<UnreadableReason> {
        self.unreadable
    }

    /// Returns true if this is an array channel — a channel whose record
    /// field holds multiple values per sample, described by a CA block.
    pub fn is_array(&self) -> bool {
        self.array_shape.is_some()
    }

    /// Returns the shape of this channel's array, or `None` for scalar channels.
    ///
    /// For an array channel, this is the CA block's dimension sizes. Element
    /// `j` of sample `i` in the flat values returned by [`Signal::values`] is
    /// at index `i * elements_per_sample + j`, where `elements_per_sample` is
    /// the product of all dimension sizes.
    pub fn array_shape(&self) -> Option<&[u64]> {
        self.array_shape.as_deref()
    }

    /// Returns the Rust type this channel's samples decode to.
    ///
    /// Integer widths follow the channel's bit count, so a 29-bit CAN identifier
    /// decodes to `u32` and a 2-bit bus number to `u8`. A channel carrying any
    /// non-identity conversion decodes to [`ValueKind::F64`], since conversions
    /// yield physical values.
    pub fn value_kind(&self) -> ValueKind {
        // An array channel decodes to `SignalValues::Array`, whose elements are
        // f64 whatever the element type is — so the element's own width is not
        // what a caller gets back. This went unnoticed while every array had a
        // template CN of type byte-array, which reported `Bytes`; an array
        // taking its element type from the parent channel made the two
        // disagree, and the read-path system test caught it.
        if self.array_shape.is_some() {
            return ValueKind::F64;
        }

        // The data type decides what the record holds. A conversion keyed by
        // numbers cannot consume a channel whose samples are text, so it does
        // not apply — and a writer attaching one anyway is common enough to
        // matter: `ASAP2_Demo_V171.mf4` hangs an identity *rational* on a
        // 256-byte text field. Letting that decide the kind reads the text as a
        // number and returns a plausible one.
        //
        // Only types 9 and 10 are keyed by text, and those genuinely do consume
        // such a channel — type 9 yielding a number from it.
        if self.data_type.is_string() && self.conversion.input() != ConversionInput::Text {
            return ValueKind::Str;
        }

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

        self.raw_value_kind()
    }

    /// Returns the Rust type this channel's raw (unconverted) samples decode to.
    pub fn raw_value_kind(&self) -> ValueKind {
        // A virtual channel's raw value is its sample index, so the width that
        // matters is the index's, not the zero-bit field's. Sizing it from
        // `bit_count` would report `u8` and truncate every index past 255.
        if self.channel_type.is_virtual() {
            return ValueKind::U64;
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
            DataType::StringSbc
            | DataType::StringUtf8
            | DataType::StringUtf16Le
            | DataType::StringUtf16Be => ValueKind::Str,
            DataType::ComplexLe | DataType::ComplexBe => ValueKind::Complex,
            DataType::CaNopenDate => ValueKind::CanopenDate,
            DataType::CaNopenTime => ValueKind::CanopenTime,
            // Byte arrays and MIME payloads are fixed-width blobs the format
            // gives no structure to. `Unknown` never reaches here — reading it
            // fails before the kind is asked for.
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

/// Where a maximum-length channel's per-sample byte count is stored.
///
/// An MLSD channel keeps its data in the record, sized to the longest sample it
/// will ever hold. The number of bytes actually used varies per sample, and the
/// standard puts that count in a *separate channel* of the same group, named by
/// the MLSD channel's `cn_data` link. This is that channel's field, resolved
/// once when the signal is built rather than per sample.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MlsdLength {
    /// Byte offset of the length field within the record.
    pub byte_offset: u32,
    /// Bit offset within the first byte.
    pub bit_offset: u8,
    /// Width of the length field in bits.
    pub bit_count: u32,
    /// Whether the length field is stored little-endian.
    pub little_endian: bool,
}

/// Element layout for an array channel, parsed from the CA block's template CN.
///
/// This is an internal type carried on [`Channel`] so that [`Signal`] can
/// decode array elements without re-reading the template CN block.
#[derive(Debug, Clone)]
pub(crate) struct ArrayElement {
    /// Data type of one array element.
    pub data_type: DataType,
    /// Bit count of one element.
    pub bit_count: u32,
    /// Bit offset within the first byte (usually 0 for byte-aligned elements).
    pub bit_offset: u8,
    /// Byte offset of the first element, relative to the parent channel's
    /// byte offset in the record.
    pub byte_offset: u32,
    /// Whether the file stores the dimensions in reverse order.
    ///
    /// `ca_flags` bit 6. With it set the *first* dimension varies fastest in
    /// the record, so the stored order is the transpose of the row-major order
    /// `SignalValues::Array` reports. dSPACE writes its matrices this way.
    pub inverse_layout: bool,
    /// Bytes from one element to the next.
    ///
    /// `ca_byte_offset_base`, which is what the standard uses to stride the
    /// elements — not the element's own width. The two usually agree; where a
    /// writer pads between elements they do not, and the width would read the
    /// padding as data.
    ///
    /// Ignored when `element_offsets` is `Some`: a look-up array composed with
    /// another CA block (B30) has one stride per nesting level rather than
    /// one for the whole array, so `element_offsets` carries the combined
    /// per-element byte deltas directly instead.
    pub stride: usize,
    /// Precomputed byte offset of each element, relative to the array's base,
    /// for a look-up array composed with another CA block (an array whose
    /// elements are themselves arrays — B30).
    ///
    /// `None` for every other array, which the flat `stride` (and
    /// `inverse_layout`, for a single level stored column-major) already
    /// describes. `Some` holds one entry per element in the *combined* shape
    /// on [`Channel::array_shape`], in row-major order over that shape.
    pub element_offsets: Option<Vec<usize>>,
    /// Storage form of the array (CN template, CG template, DG template).
    pub storage: CaStorage,
    /// Links to member channel groups (for CG template) or data groups (for DG template).
    pub group_links: Vec<u64>,
}

/// Encryption metadata for an encrypted attachment, extracted from its XML comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionInfo {
    /// Whether the attachment is marked as encrypted.
    pub encrypted: bool,
    /// Encryption algorithm, e.g. `"AES256"`.
    pub algorithm: String,
    /// Expected MD5 hex digest of the encrypted payload before decryption.
    pub original_md5_sum: String,
    /// Unencrypted size of the attachment payload in bytes.
    pub original_size: usize,
}

/// A file attached to the measurement.
///
/// Attachments can be embedded (their bytes are in the MF4 file) or external
/// (a path the reader resolves). Use [`Attachment::is_embedded`] to
/// distinguish, and [`crate::Mf4File::attachment_data`] to read embedded bytes.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// File name of the attachment.
    pub file_name: String,
    /// File path (for external attachments) or MIME type.
    pub file_path: String,
    /// Comment/description.
    pub comment: String,
    /// Whether the attachment data is embedded in the MF4 file.
    pub is_embedded: bool,
    /// Whether the embedded bytes are deflate-compressed.
    ///
    /// [`crate::Mf4File::attachment_data`] decompresses them, so this says how
    /// the file stores the attachment rather than what you get back.
    pub is_compressed: bool,
    /// Original size in bytes. For a compressed attachment this is the size
    /// after decompression, not the number of bytes in the file.
    pub original_size: u64,
    /// MD5 checksum of the attachment content, valid only when
    /// [`Attachment::md5_valid`] is set.
    pub md5_checksum: [u8; 16],
    /// Whether the writer computed [`Attachment::md5_checksum`]. When false,
    /// those bytes carry no meaning and must not be compared against.
    pub md5_valid: bool,
    /// File offset where embedded data begins, or 0 for external attachments.
    pub(crate) embedded_offset: u64,
    /// Embedded size in bytes (0 for external attachments).
    pub(crate) embedded_size: u64,
}

impl Attachment {
    /// Returns the embedded size in bytes. Zero for external attachments.
    pub fn embedded_size(&self) -> u64 {
        self.embedded_size
    }

    /// Extracts encryption metadata from the attachment's XML comment, if present.
    pub fn encryption_info(&self) -> Option<EncryptionInfo> {
        extract_encryption_info(&self.comment)
    }

    /// Returns whether this attachment is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.encryption_info().is_some_and(|info| info.encrypted)
    }
}

/// Parses encryption metadata from an AT block's XML comment.
fn extract_encryption_info(comment: &str) -> Option<EncryptionInfo> {
    if !comment.contains("<encrypted>") && !comment.contains("<encrypted ") {
        return None;
    }

    let mut reader = quick_xml::Reader::from_str(comment);
    reader.config_mut().trim_text(true);

    let mut in_extension = false;
    let mut current_tag = Vec::new();

    let mut encrypted = false;
    let mut algorithm = String::new();
    let mut original_md5_sum = String::new();
    let mut original_size = 0usize;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                if name == b"extension" {
                    in_extension = true;
                }
                current_tag = name;
            }
            Ok(quick_xml::events::Event::Text(t)) if in_extension => {
                if let Ok(text) = t.unescape() {
                    match current_tag.as_slice() {
                        b"encrypted" => {
                            encrypted = text.trim().eq_ignore_ascii_case("true");
                        }
                        b"algorithm" => {
                            algorithm = text.trim().to_string();
                        }
                        b"original_md5_sum" => {
                            original_md5_sum = text.trim().to_string();
                        }
                        b"original_size" => {
                            original_size = text.trim().parse().unwrap_or(0);
                        }
                        _ => {}
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                if e.name().as_ref() == b"extension" {
                    if encrypted {
                        return Some(EncryptionInfo {
                            encrypted,
                            algorithm,
                            original_md5_sum,
                            original_size,
                        });
                    }
                    in_extension = false;
                }
                current_tag.clear();
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if encrypted {
        Some(EncryptionInfo {
            encrypted,
            algorithm,
            original_md5_sum,
            original_size,
        })
    } else {
        None
    }
}

/// An event marking a point or range in the measurement.
///
/// Events carry a timestamp relative to the HD block's start time. For time
/// events, the timestamp is in nanoseconds; for angle events, in radians; for
/// distance events, in meters; for index events, a sample number.
#[derive(Debug, Clone)]
pub struct Event {
    /// Event type (trigger, marker, recording, etc.).
    pub event_type: EventType,
    /// Synchronization domain for the timestamp.
    pub sync_type: EvSyncType,
    /// Range type (point, begin, end).
    pub range_type: EvRangeType,
    /// What caused the event.
    pub cause: EvCause,
    /// Raw synchronisation value, in units of [`Event::sync_factor`].
    ///
    /// An event block records where it sits as a base value and a factor rather
    /// than as a timestamp; [`Event::position`] combines them.
    pub sync_base_value: i64,
    /// Factor converting [`Event::sync_base_value`] into the synchronisation
    /// domain named by [`Event::sync_type`].
    pub sync_factor: f64,
    /// Number of scopes — channel groups or channels — the event applies to.
    pub scope_count: u32,
    /// Number of attachments the event references.
    pub attachment_count: u16,
    /// Comment/description.
    pub comment: String,
    /// This event's name.
    pub name: String,
}

impl Event {
    /// Returns the event's position in its synchronisation domain — seconds for
    /// a time-synchronised event, radians for an angle, and so on.
    ///
    /// A range event marks only one end; the other is a separate event block.
    pub fn position(&self) -> f64 {
        self.sync_base_value as f64 * self.sync_factor
    }
}

/// One level of sample reduction attached to a channel group.
///
/// A reduction is a condensed view of the group's data — one record per
/// interval, holding the mean, minimum and maximum over that interval — meant
/// for drawing an overview without reading every sample.
///
/// # Reading reduced values
///
/// Reduced values are read via [`crate::Mf4File::reduced_signal`], which
/// selects whether to retrieve the mean, minimum, or maximum series for a
/// channel across each reduction interval.
#[derive(Debug, Clone)]
pub struct SampleReduction {
    /// Number of reduced records.
    pub cycle_count: u64,
    /// Length of the interval each record condenses, in the units of
    /// [`SampleReduction::sync_type`].
    pub interval: f64,
    /// The synchronisation domain the interval is measured in.
    pub sync_type: SrSyncType,
    /// Flags as stored in the block.
    pub flags: u8,
    /// Link to the block holding the reduced records.
    pub(crate) data_link: u64,
}

/// Which of the three values a reduced record holds for each channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionKind {
    /// The mean over the interval.
    Mean,
    /// The smallest value seen in the interval.
    Min,
    /// The largest value seen in the interval.
    Max,
}

impl ReductionKind {
    /// Position of this value within a reduced record.
    ///
    /// A reduced record holds three copies of the channel group's normal
    /// record, one after another: the means, then the minima, then the maxima.
    pub(crate) fn index(self) -> usize {
        match self {
            ReductionKind::Mean => 0,
            ReductionKind::Min => 1,
            ReductionKind::Max => 2,
        }
    }
}

/// One entry in a file's change history.
///
/// The first entry records the file's creation; any later ones record a
/// modification. Every MF4 file is required to carry at least one.
#[derive(Debug, Clone)]
pub struct FileHistoryEntry {
    /// When the entry was recorded.
    pub time: RecordingTime,
    /// The human-readable note, from the entry's metadata.
    pub comment: String,
    /// The entry's metadata, which names the tool responsible.
    pub metadata: Metadata,
}

impl FileHistoryEntry {
    /// Returns the identifier of the tool that made the change, if recorded.
    pub fn tool_id(&self) -> Option<&str> {
        self.metadata.get("tool_id")
    }

    /// Returns the vendor of that tool, if recorded.
    pub fn tool_vendor(&self) -> Option<&str> {
        self.metadata.get("tool_vendor")
    }

    /// Returns the version of that tool, if recorded.
    pub fn tool_version(&self) -> Option<&str> {
        self.metadata.get("tool_version")
    }
}

/// A node in the channel hierarchy.
///
/// The channel hierarchy groups channels into named subtrees, providing a
/// logical organisation independent of the data-group/channel-group structure.
#[derive(Debug, Clone)]
pub struct ChannelHierarchyNode {
    /// Hierarchy name.
    pub name: String,
    /// Comment/description.
    pub comment: String,
    /// Hierarchy type (tree or plain).
    pub hierarchy_type: ChType,
    /// The channels this node references, each identified by the data group,
    /// channel group and channel that locate it.
    pub elements: Vec<ChElement>,
    /// Whether this node has children of its own.
    ///
    /// A hierarchy is a tree; a node with children groups them rather than
    /// naming channels directly.
    pub has_children: bool,
    /// This node's child nodes, parsed by descending `ch_first`.
    ///
    /// Empty when `has_children` is false. A child that a corrupted file
    /// links twice is visited once: a cycle between levels would otherwise
    /// recurse forever, so the second link to any node is dropped.
    pub children: Vec<ChannelHierarchyNode>,
}
