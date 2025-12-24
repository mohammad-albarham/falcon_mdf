//! Channel Group (CG) block parsing.
//!
//! A channel group contains channels that share the same time axis and
//! are stored together in records. Each record contains one sample from
//! each channel in the group.

use crate::error::{Mf4Error, Result};
use crate::blocks::common::{BlockHeader, read_link, BLOCK_HEADER_SIZE, ParseBlock};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Channel group flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CgFlags {
    /// Variable length signal data (VLSD) channel group.
    pub vlsd: bool,
    /// Bus event channel group.
    pub bus_event: bool,
    /// Plain bus event channel.
    pub plain_bus_event: bool,
}

impl CgFlags {
    fn from_u16(value: u16) -> Self {
        CgFlags {
            vlsd: (value & 0x01) != 0,
            bus_event: (value & 0x02) != 0,
            plain_bus_event: (value & 0x04) != 0,
        }
    }
}

/// The Channel Group (CG) block.
///
/// Channel groups organize channels that are sampled together and stored
/// in fixed-size records. The record layout defines how channel data is
/// packed within each record.
#[derive(Debug, Clone)]
pub struct CgBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to next channel group block (0 = none).
    pub cg_next: u64,
    /// Link to first channel block.
    pub cn_first: u64,
    /// Link to acquisition name (TX block).
    pub tx_acq_name: u64,
    /// Link to acquisition source (SI block).
    pub si_acq_source: u64,
    /// Link to first sample reduction block (SR, MF4 4.1+).
    pub sr_first: u64,
    /// Link to comment (TX or MD block).
    pub md_comment: u64,
    /// Record ID for this channel group (0 if only one CG in DG).
    pub record_id: u64,
    /// Number of cycles/samples in this channel group.
    pub cycle_count: u64,
    /// Channel group flags.
    pub flags: CgFlags,
    /// Path separator character (e.g., '.' or '/').
    pub path_separator: u16,
    /// Reserved bytes.
    pub reserved: [u8; 4],
    /// Size of one data record in bytes (excluding record ID).
    pub data_bytes: u32,
    /// Size of invalidation bits in bytes.
    pub inval_bytes: u32,
}

impl CgBlock {
    /// Minimum size of the CG block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 6 * 8 + 32;

    /// Returns the total record size including any record ID bytes.
    pub fn record_size(&self, rec_id_size: u8) -> usize {
        rec_id_size as usize + self.data_bytes as usize + self.inval_bytes as usize
    }
}

impl ParseBlock for CgBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CG", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size("CG", header.length, Self::MIN_SIZE));
        }

        // Parse links
        let links_start = BLOCK_HEADER_SIZE;
        let cg_next = read_link(data, links_start)?;
        let cn_first = read_link(data, links_start + 8)?;
        let tx_acq_name = read_link(data, links_start + 16)?;
        let si_acq_source = read_link(data, links_start + 24)?;
        let sr_first = read_link(data, links_start + 32)?;
        let md_comment = read_link(data, links_start + 40)?;

        // Parse data section
        let data_start = header.data_offset();
        let data_section = &data[data_start..];
        let mut cursor = Cursor::new(data_section);

        let record_id = cursor.read_u64::<LittleEndian>()?;
        let cycle_count = cursor.read_u64::<LittleEndian>()?;
        let flags_value = cursor.read_u16::<LittleEndian>()?;
        let flags = CgFlags::from_u16(flags_value);
        let path_separator = cursor.read_u16::<LittleEndian>()?;
        let mut reserved = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut reserved)?;
        let data_bytes = cursor.read_u32::<LittleEndian>()?;
        let inval_bytes = cursor.read_u32::<LittleEndian>()?;

        Ok(CgBlock {
            header,
            cg_next,
            cn_first,
            tx_acq_name,
            si_acq_source,
            sr_first,
            md_comment,
            record_id,
            cycle_count,
            flags,
            path_separator,
            reserved,
            data_bytes,
            inval_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_cg_block() -> Vec<u8> {
        let mut data = vec![0u8; 104];
        
        // Header
        data[0..4].copy_from_slice(b"##CG");
        data[8..16].copy_from_slice(&104u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&6u64.to_le_bytes()); // link_count

        // Links (6 x 8 bytes)
        data[24..32].copy_from_slice(&0u64.to_le_bytes()); // cg_next
        data[32..40].copy_from_slice(&500u64.to_le_bytes()); // cn_first
        data[40..48].copy_from_slice(&600u64.to_le_bytes()); // tx_acq_name
        data[48..56].copy_from_slice(&0u64.to_le_bytes()); // si_acq_source
        data[56..64].copy_from_slice(&0u64.to_le_bytes()); // sr_first
        data[64..72].copy_from_slice(&700u64.to_le_bytes()); // md_comment

        // Data section (starting at offset 72)
        data[72..80].copy_from_slice(&1u64.to_le_bytes()); // record_id
        data[80..88].copy_from_slice(&1000u64.to_le_bytes()); // cycle_count
        data[88..90].copy_from_slice(&0u16.to_le_bytes()); // flags
        data[90..92].copy_from_slice(&('.' as u16).to_le_bytes()); // path_separator
        // reserved: 4 bytes at 92..96
        data[96..100].copy_from_slice(&64u32.to_le_bytes()); // data_bytes
        data[100..104].copy_from_slice(&0u32.to_le_bytes()); // inval_bytes

        data
    }

    #[test]
    fn test_cg_block_parse() {
        let data = create_test_cg_block();
        let cg = CgBlock::parse(&data, 2000).unwrap();

        assert_eq!(cg.header.block_type, *b"##CG");
        assert_eq!(cg.cn_first, 500);
        assert_eq!(cg.tx_acq_name, 600);
        assert_eq!(cg.record_id, 1);
        assert_eq!(cg.cycle_count, 1000);
        assert_eq!(cg.data_bytes, 64);
    }

    #[test]
    fn test_cg_flags() {
        let flags = CgFlags::from_u16(0x03);
        assert!(flags.vlsd);
        assert!(flags.bus_event);
        assert!(!flags.plain_bus_event);
    }

    #[test]
    fn test_cg_record_size() {
        let data = create_test_cg_block();
        let cg = CgBlock::parse(&data, 2000).unwrap();
        
        // With no record ID
        assert_eq!(cg.record_size(0), 64);
        // With 1-byte record ID
        assert_eq!(cg.record_size(1), 65);
    }
}
