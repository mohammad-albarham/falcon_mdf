//! Channel (CN) block parsing.
//!
//! Channels define individual signals within a channel group.
//! Each channel specifies how to extract and decode sample values
//! from raw record data.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Channel type enumeration.
///
/// Deliberately not `#[non_exhaustive]`: the variants mirror a byte in the file,
/// and any code the standard has not defined maps to [`ChannelType::Unknown`].
/// A reader can therefore match every variant and stay correct against files
/// this version has never seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Fixed-length data channel (default).
    FixedLength,
    /// Variable-length data channel (VLSD).
    VariableLength,
    /// Master channel (time, angle, distance).
    Master,
    /// Virtual master channel (computed).
    VirtualMaster,
    /// Synchronization channel.
    Sync,
    /// Maximum length data channel.
    MaxLength,
    /// Virtual data channel (computed).
    VirtualData,
    /// Unknown channel type.
    Unknown(u8),
}

impl ChannelType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => ChannelType::FixedLength,
            1 => ChannelType::VariableLength,
            2 => ChannelType::Master,
            3 => ChannelType::VirtualMaster,
            4 => ChannelType::Sync,
            5 => ChannelType::MaxLength,
            6 => ChannelType::VirtualData,
            v => ChannelType::Unknown(v),
        }
    }

    /// Returns true if this is a master (time) channel.
    pub fn is_master(&self) -> bool {
        matches!(self, ChannelType::Master | ChannelType::VirtualMaster)
    }

    /// Returns true if this channel's samples are computed rather than stored.
    ///
    /// A virtual channel occupies no bytes in the record — `cn_bit_count` is 0.
    /// Its raw value is the zero-based index of the sample, which the channel's
    /// conversion then turns into a physical value; that is how a file stores a
    /// regularly-spaced time base without writing it out.
    pub fn is_virtual(&self) -> bool {
        matches!(self, ChannelType::VirtualMaster | ChannelType::VirtualData)
    }
}

/// Synchronization type for master channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncType {
    /// No synchronization / not a master channel.
    None,
    /// Time in seconds.
    Time,
    /// Angle in radians.
    Angle,
    /// Distance in meters.
    Distance,
    /// Index (sample number).
    Index,
    /// Unknown sync type.
    Unknown(u8),
}

impl SyncType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => SyncType::None,
            1 => SyncType::Time,
            2 => SyncType::Angle,
            3 => SyncType::Distance,
            4 => SyncType::Index,
            v => SyncType::Unknown(v),
        }
    }
}

/// Data type for channel values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Unsigned integer, little-endian.
    UIntLe,
    /// Unsigned integer, big-endian.
    UIntBe,
    /// Signed integer, little-endian.
    IntLe,
    /// Signed integer, big-endian.
    IntBe,
    /// IEEE 754 float, little-endian.
    FloatLe,
    /// IEEE 754 float, big-endian.
    FloatBe,
    /// UTF-8 string.
    StringUtf8,
    /// UTF-16 little-endian string.
    StringUtf16Le,
    /// UTF-16 big-endian string.
    StringUtf16Be,
    /// Byte array.
    ByteArray,
    /// MIME sample (image, audio, etc.).
    MimeSample,
    /// MIME stream.
    MimeStream,
    /// CANopen date.
    CaNopenDate,
    /// CANopen time.
    CaNopenTime,
    /// Complex number, little-endian (real, imag).
    ComplexLe,
    /// Complex number, big-endian.
    ComplexBe,
    /// Unknown data type.
    Unknown(u8),
}

impl DataType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => DataType::UIntLe,
            1 => DataType::UIntBe,
            2 => DataType::IntLe,
            3 => DataType::IntBe,
            4 => DataType::FloatLe,
            5 => DataType::FloatBe,
            6 => DataType::StringUtf8,
            7 => DataType::StringUtf16Le,
            8 => DataType::StringUtf16Be,
            9 => DataType::ByteArray,
            10 => DataType::MimeSample,
            11 => DataType::MimeStream,
            12 => DataType::CaNopenDate,
            13 => DataType::CaNopenTime,
            14 => DataType::ComplexLe,
            15 => DataType::ComplexBe,
            v => DataType::Unknown(v),
        }
    }

    /// Returns true if this is a numeric type (integer or float).
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            DataType::UIntLe
                | DataType::UIntBe
                | DataType::IntLe
                | DataType::IntBe
                | DataType::FloatLe
                | DataType::FloatBe
        )
    }

    /// Returns true if this is a string type.
    pub fn is_string(&self) -> bool {
        matches!(
            self,
            DataType::StringUtf8 | DataType::StringUtf16Le | DataType::StringUtf16Be
        )
    }

    /// Returns true if this type uses little-endian byte order.
    pub fn is_little_endian(&self) -> bool {
        matches!(
            self,
            DataType::UIntLe
                | DataType::IntLe
                | DataType::FloatLe
                | DataType::StringUtf16Le
                | DataType::ComplexLe
        )
    }

    /// Returns true if this is a signed integer type.
    pub fn is_signed(&self) -> bool {
        matches!(self, DataType::IntLe | DataType::IntBe)
    }

    /// Returns true if this is a floating-point type.
    pub fn is_float(&self) -> bool {
        matches!(self, DataType::FloatLe | DataType::FloatBe)
    }
}

/// Channel flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CnFlags {
    /// All values are invalid.
    pub all_invalid: bool,
    /// Invalidation bit is present.
    pub invalidation_bit: bool,
    /// Precision is valid.
    pub precision_valid: bool,
    /// Value range is valid.
    pub range_valid: bool,
    /// Limit range is valid.
    pub limit_valid: bool,
    /// Extended limit range is valid.
    pub ext_limit_valid: bool,
    /// Discrete values (enumeration).
    pub discrete: bool,
    /// Calibration possible.
    pub calibration: bool,
    /// Calculated channel.
    pub calculated: bool,
    /// Virtual channel.
    pub virtual_channel: bool,
    /// Bus event channel.
    pub bus_event: bool,
    /// Monotonous (strictly increasing or decreasing).
    pub monotonous: bool,
    /// Default X axis.
    pub default_x: bool,
    /// Signal event channel.
    pub event_signal: bool,
    /// Variable length signal data offset.
    pub vlsd_offset: bool,
}

impl CnFlags {
    fn from_u32(value: u32) -> Self {
        CnFlags {
            all_invalid: (value & 0x0001) != 0,
            invalidation_bit: (value & 0x0002) != 0,
            precision_valid: (value & 0x0004) != 0,
            range_valid: (value & 0x0008) != 0,
            limit_valid: (value & 0x0010) != 0,
            ext_limit_valid: (value & 0x0020) != 0,
            discrete: (value & 0x0040) != 0,
            calibration: (value & 0x0080) != 0,
            calculated: (value & 0x0100) != 0,
            virtual_channel: (value & 0x0200) != 0,
            bus_event: (value & 0x0400) != 0,
            monotonous: (value & 0x0800) != 0,
            default_x: (value & 0x1000) != 0,
            event_signal: (value & 0x2000) != 0,
            vlsd_offset: (value & 0x4000) != 0,
        }
    }
}

/// The Channel (CN) block.
///
/// Defines a single signal/channel within a channel group, including
/// its data type, bit position in records, conversion formula, etc.
#[derive(Debug, Clone)]
pub struct CnBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to next channel block (0 = none).
    pub cn_next: u64,
    /// Link to composition (CA for arrays, CN for structures).
    pub composition: u64,
    /// Link to channel name (TX block).
    pub tx_name: u64,
    /// Link to source information (SI block).
    pub si_source: u64,
    /// Link to conversion formula (CC block).
    pub cc_conversion: u64,
    /// Link to data block for VLSD/MLSD channels.
    pub data: u64,
    /// Link to unit (TX or MD block).
    pub md_unit: u64,
    /// Link to comment (TX or MD block).
    pub md_comment: u64,
    /// Link to attachment (AT block) for MIME channels.
    pub at_reference: u64,
    /// Link to default X channel.
    pub default_x: [u64; 3],
    /// Channel type.
    pub channel_type: ChannelType,
    /// Synchronization type (for master channels).
    pub sync_type: SyncType,
    /// Data type of raw values.
    pub data_type: DataType,
    /// Bit offset within the record byte.
    pub bit_offset: u8,
    /// Byte offset within the record (from first data byte).
    pub byte_offset: u32,
    /// Number of bits for the channel value.
    pub bit_count: u32,
    /// Channel flags.
    pub flags: CnFlags,
    /// Invalidation bit position.
    pub inval_bit_pos: u32,
    /// Precision (number of decimal places).
    pub precision: u8,
    /// Reserved byte.
    pub reserved: u8,
    /// Attachment count.
    pub attachment_count: u16,
    /// Minimum raw value (if range_valid).
    pub val_range_min: f64,
    /// Maximum raw value (if range_valid).
    pub val_range_max: f64,
    /// Minimum physical limit (if limit_valid).
    pub limit_min: f64,
    /// Maximum physical limit (if limit_valid).
    pub limit_max: f64,
    /// Extended minimum limit (if ext_limit_valid).
    pub limit_ext_min: f64,
    /// Extended maximum limit (if ext_limit_valid).
    pub limit_ext_max: f64,
}

impl CnBlock {
    /// Minimum size of the CN block (varies by version, this is for 4.2+).
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 8 * 8 + 72;

    /// Returns the size of this channel's value in bytes.
    pub fn byte_size(&self) -> usize {
        (self.bit_count as usize).div_ceil(8)
    }
}

impl ParseBlock for CnBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CN", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "CN",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        // Parse links
        let links_start = BLOCK_HEADER_SIZE;
        let cn_next = read_link(data, links_start)?;
        let composition = read_link(data, links_start + 8)?;
        let tx_name = read_link(data, links_start + 16)?;
        let si_source = read_link(data, links_start + 24)?;
        let cc_conversion = read_link(data, links_start + 32)?;
        let cn_data = read_link(data, links_start + 40)?;
        let md_unit = read_link(data, links_start + 48)?;
        let md_comment = read_link(data, links_start + 56)?;

        // Additional links depend on link_count
        let at_reference = if header.link_count > 8 {
            read_link(data, links_start + 64)?
        } else {
            0
        };

        let default_x = if header.link_count > 9 {
            [
                read_link(data, links_start + 72)?,
                read_link(data, links_start + 80)?,
                read_link(data, links_start + 88)?,
            ]
        } else {
            [0, 0, 0]
        };

        // Parse data section
        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let channel_type_raw = cursor.read_u8()?;
        let channel_type = ChannelType::from_u8(channel_type_raw);
        let sync_type_raw = cursor.read_u8()?;
        let sync_type = SyncType::from_u8(sync_type_raw);
        let data_type_raw = cursor.read_u8()?;
        let data_type = DataType::from_u8(data_type_raw);
        let bit_offset = cursor.read_u8()?;
        let byte_offset = cursor.read_u32::<LittleEndian>()?;
        let bit_count = cursor.read_u32::<LittleEndian>()?;
        let flags_raw = cursor.read_u32::<LittleEndian>()?;
        let flags = CnFlags::from_u32(flags_raw);
        let inval_bit_pos = cursor.read_u32::<LittleEndian>()?;
        let precision = cursor.read_u8()?;
        let reserved = cursor.read_u8()?;
        let attachment_count = cursor.read_u16::<LittleEndian>()?;
        let val_range_min = cursor.read_f64::<LittleEndian>()?;
        let val_range_max = cursor.read_f64::<LittleEndian>()?;
        let limit_min = cursor.read_f64::<LittleEndian>()?;
        let limit_max = cursor.read_f64::<LittleEndian>()?;
        let limit_ext_min = cursor.read_f64::<LittleEndian>()?;
        let limit_ext_max = cursor.read_f64::<LittleEndian>()?;

        Ok(CnBlock {
            header,
            cn_next,
            composition,
            tx_name,
            si_source,
            cc_conversion,
            data: cn_data,
            md_unit,
            md_comment,
            at_reference,
            default_x,
            channel_type,
            sync_type,
            data_type,
            bit_offset,
            byte_offset,
            bit_count,
            flags,
            inval_bit_pos,
            precision,
            reserved,
            attachment_count,
            val_range_min,
            val_range_max,
            limit_min,
            limit_max,
            limit_ext_min,
            limit_ext_max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_cn_block() -> Vec<u8> {
        let mut data = vec![0u8; 160];

        // Header
        data[0..4].copy_from_slice(b"##CN");
        data[8..16].copy_from_slice(&160u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&8u64.to_le_bytes()); // link_count

        // Links (8 x 8 bytes starting at offset 24)
        data[24..32].copy_from_slice(&0u64.to_le_bytes()); // cn_next
        data[32..40].copy_from_slice(&0u64.to_le_bytes()); // composition
        data[40..48].copy_from_slice(&1000u64.to_le_bytes()); // tx_name
        data[48..56].copy_from_slice(&0u64.to_le_bytes()); // si_source
        data[56..64].copy_from_slice(&2000u64.to_le_bytes()); // cc_conversion
        data[64..72].copy_from_slice(&0u64.to_le_bytes()); // data
        data[72..80].copy_from_slice(&3000u64.to_le_bytes()); // md_unit
        data[80..88].copy_from_slice(&0u64.to_le_bytes()); // md_comment

        // Data section (starting at offset 88)
        data[88] = 0; // channel_type = FixedLength
        data[89] = 1; // sync_type = Time
        data[90] = 4; // data_type = FloatLe
        data[91] = 0; // bit_offset
        data[92..96].copy_from_slice(&0u32.to_le_bytes()); // byte_offset
        data[96..100].copy_from_slice(&64u32.to_le_bytes()); // bit_count (64 bits = f64)
        data[100..104].copy_from_slice(&0u32.to_le_bytes()); // flags
        data[104..108].copy_from_slice(&0u32.to_le_bytes()); // inval_bit_pos
        data[108] = 6; // precision
        data[109] = 0; // reserved
        data[110..112].copy_from_slice(&0u16.to_le_bytes()); // attachment_count
                                                             // val_range_min, val_range_max, limits - all zeros

        data
    }

    #[test]
    fn test_cn_block_parse() {
        let data = create_test_cn_block();
        let cn = CnBlock::parse(&data, 3000).unwrap();

        assert_eq!(cn.header.block_type, *b"##CN");
        assert_eq!(cn.tx_name, 1000);
        assert_eq!(cn.cc_conversion, 2000);
        assert_eq!(cn.md_unit, 3000);
        assert_eq!(cn.channel_type, ChannelType::FixedLength);
        assert_eq!(cn.sync_type, SyncType::Time);
        assert_eq!(cn.data_type, DataType::FloatLe);
        assert_eq!(cn.bit_count, 64);
        assert_eq!(cn.precision, 6);
    }

    #[test]
    fn test_channel_type() {
        assert!(ChannelType::Master.is_master());
        assert!(ChannelType::VirtualMaster.is_master());
        assert!(!ChannelType::FixedLength.is_master());
    }

    #[test]
    fn test_data_type() {
        assert!(DataType::FloatLe.is_numeric());
        assert!(DataType::IntLe.is_numeric());
        assert!(!DataType::StringUtf8.is_numeric());
        assert!(DataType::StringUtf8.is_string());
        assert!(DataType::FloatLe.is_little_endian());
        assert!(!DataType::FloatBe.is_little_endian());
    }
}
