//! Source Information (SI) block parsing.
//!
//! SI blocks describe the source of a signal or channel group,
//! such as a CAN bus, ECU, or recording device.

use crate::error::{Mf4Error, Result};
use crate::blocks::common::{BlockHeader, read_link, BLOCK_HEADER_SIZE, ParseBlock};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Source type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// Other/unspecified source.
    Other,
    /// ECU (Electronic Control Unit).
    Ecu,
    /// Bus (CAN, LIN, FlexRay, etc.).
    Bus,
    /// I/O device.
    Io,
    /// Tool (measurement software).
    Tool,
    /// User-defined source.
    User,
    /// Unknown source type.
    Unknown(u8),
}

impl SourceType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => SourceType::Other,
            1 => SourceType::Ecu,
            2 => SourceType::Bus,
            3 => SourceType::Io,
            4 => SourceType::Tool,
            5 => SourceType::User,
            v => SourceType::Unknown(v),
        }
    }
}

/// Bus type enumeration (when source_type is Bus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusType {
    /// No bus type specified.
    None,
    /// Other bus type.
    Other,
    /// CAN bus.
    Can,
    /// LIN bus.
    Lin,
    /// MOST bus.
    Most,
    /// FlexRay bus.
    FlexRay,
    /// K-Line.
    KLine,
    /// Ethernet.
    Ethernet,
    /// USB.
    Usb,
    /// Unknown bus type.
    Unknown(u8),
}

impl BusType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => BusType::None,
            1 => BusType::Other,
            2 => BusType::Can,
            3 => BusType::Lin,
            4 => BusType::Most,
            5 => BusType::FlexRay,
            6 => BusType::KLine,
            7 => BusType::Ethernet,
            8 => BusType::Usb,
            v => BusType::Unknown(v),
        }
    }
}

/// Source information flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SiFlags {
    /// Source is simulated.
    pub simulated: bool,
}

impl SiFlags {
    fn from_u8(value: u8) -> Self {
        SiFlags {
            simulated: (value & 0x01) != 0,
        }
    }
}

/// The Source Information (SI) block.
///
/// Describes the origin of measurement data, such as a specific
/// ECU, CAN channel, or recording tool.
#[derive(Debug, Clone)]
pub struct SiBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to source name (TX block).
    pub tx_name: u64,
    /// Link to path/identifier (TX block).
    pub tx_path: u64,
    /// Link to comment (TX or MD block).
    pub md_comment: u64,
    /// Source type.
    pub source_type: SourceType,
    /// Bus type (for Bus sources).
    pub bus_type: BusType,
    /// Source flags.
    pub flags: SiFlags,
    /// Reserved bytes.
    pub reserved: [u8; 5],
}

impl SiBlock {
    /// Minimum size of the SI block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 3 * 8 + 8;
}

impl ParseBlock for SiBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##SI", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size("SI", header.length, Self::MIN_SIZE));
        }

        // Parse links
        let links_start = BLOCK_HEADER_SIZE;
        let tx_name = read_link(data, links_start)?;
        let tx_path = read_link(data, links_start + 8)?;
        let md_comment = read_link(data, links_start + 16)?;

        // Parse data section
        let data_start = header.data_offset();
        let data_section = &data[data_start..];
        let mut cursor = Cursor::new(data_section);

        let source_type_raw = cursor.read_u8()?;
        let source_type = SourceType::from_u8(source_type_raw);
        let bus_type_raw = cursor.read_u8()?;
        let bus_type = BusType::from_u8(bus_type_raw);
        let flags_raw = cursor.read_u8()?;
        let flags = SiFlags::from_u8(flags_raw);
        let mut reserved = [0u8; 5];
        std::io::Read::read_exact(&mut cursor, &mut reserved)?;

        Ok(SiBlock {
            header,
            tx_name,
            tx_path,
            md_comment,
            source_type,
            bus_type,
            flags,
            reserved,
        })
    }
}

/// High-level source information.
#[derive(Debug, Clone, Default)]
pub struct SourceInfo {
    /// Source name.
    pub name: String,
    /// Source path/identifier.
    pub path: String,
    /// Source type.
    pub source_type: Option<SourceType>,
    /// Bus type (if applicable).
    pub bus_type: Option<BusType>,
    /// Whether the source is simulated.
    pub simulated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_si_block() -> Vec<u8> {
        let mut data = vec![0u8; 56];
        
        // Header
        data[0..4].copy_from_slice(b"##SI");
        data[8..16].copy_from_slice(&56u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&3u64.to_le_bytes()); // link_count

        // Links
        data[24..32].copy_from_slice(&1000u64.to_le_bytes()); // tx_name
        data[32..40].copy_from_slice(&2000u64.to_le_bytes()); // tx_path
        data[40..48].copy_from_slice(&0u64.to_le_bytes()); // md_comment

        // Data section
        data[48] = 2; // source_type = Bus
        data[49] = 2; // bus_type = CAN
        data[50] = 0; // flags

        data
    }

    #[test]
    fn test_si_block_parse() {
        let data = create_test_si_block();
        let si = SiBlock::parse(&data, 5000).unwrap();

        assert_eq!(si.header.block_type, *b"##SI");
        assert_eq!(si.tx_name, 1000);
        assert_eq!(si.tx_path, 2000);
        assert_eq!(si.source_type, SourceType::Bus);
        assert_eq!(si.bus_type, BusType::Can);
        assert!(!si.flags.simulated);
    }

    #[test]
    fn test_source_type() {
        assert_eq!(SourceType::from_u8(0), SourceType::Other);
        assert_eq!(SourceType::from_u8(1), SourceType::Ecu);
        assert_eq!(SourceType::from_u8(2), SourceType::Bus);
        assert!(matches!(SourceType::from_u8(99), SourceType::Unknown(99)));
    }

    #[test]
    fn test_bus_type() {
        assert_eq!(BusType::from_u8(2), BusType::Can);
        assert_eq!(BusType::from_u8(3), BusType::Lin);
        assert_eq!(BusType::from_u8(7), BusType::Ethernet);
    }
}
