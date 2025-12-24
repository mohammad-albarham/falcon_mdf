//! Data Group (DG) block parsing.
//!
//! A data group contains one or more channel groups that share the same
//! raw data block(s). Each data group links to its channel groups and data.

use crate::error::{Mf4Error, Result};
use crate::blocks::common::{BlockHeader, read_link, BLOCK_HEADER_SIZE, ParseBlock};
use byteorder::ReadBytesExt;
use std::io::Cursor;

/// The Data Group (DG) block.
///
/// Data groups are containers for related channel groups that share
/// the same underlying data storage. Iterating through all DG blocks
/// (following dg_next links) gives access to all data in the file.
#[derive(Debug, Clone)]
pub struct DgBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to next data group block (0 = none).
    pub dg_next: u64,
    /// Link to first channel group block.
    pub cg_first: u64,
    /// Link to data block (DT, DZ, DL, or HL).
    pub data: u64,
    /// Link to comment (TX or MD block).
    pub md_comment: u64,
    /// Number of record IDs in the data block.
    /// 0 = only one channel group, no record IDs needed.
    pub rec_id_size: u8,
    /// Reserved bytes.
    pub reserved: [u8; 7],
}

impl DgBlock {
    /// Minimum size of the DG block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 4 * 8 + 8;
}

impl ParseBlock for DgBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##DG", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size("DG", header.length, Self::MIN_SIZE));
        }

        // Parse links
        let links_start = BLOCK_HEADER_SIZE;
        let dg_next = read_link(data, links_start)?;
        let cg_first = read_link(data, links_start + 8)?;
        let data_link = read_link(data, links_start + 16)?;
        let md_comment = read_link(data, links_start + 24)?;

        // Parse data section
        let data_start = header.data_offset();
        let data_section = &data[data_start..];
        let mut cursor = Cursor::new(data_section);

        let rec_id_size = cursor.read_u8()?;
        let mut reserved = [0u8; 7];
        std::io::Read::read_exact(&mut cursor, &mut reserved)?;

        Ok(DgBlock {
            header,
            dg_next,
            cg_first,
            data: data_link,
            md_comment,
            rec_id_size,
            reserved,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_dg_block() -> Vec<u8> {
        let mut data = vec![0u8; 64];
        
        // Header
        data[0..4].copy_from_slice(b"##DG");
        data[8..16].copy_from_slice(&64u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&4u64.to_le_bytes()); // link_count

        // Links (4 x 8 bytes)
        data[24..32].copy_from_slice(&100u64.to_le_bytes()); // dg_next
        data[32..40].copy_from_slice(&200u64.to_le_bytes()); // cg_first
        data[40..48].copy_from_slice(&300u64.to_le_bytes()); // data
        data[48..56].copy_from_slice(&400u64.to_le_bytes()); // md_comment

        // Data section
        data[56] = 0; // rec_id_size

        data
    }

    #[test]
    fn test_dg_block_parse() {
        let data = create_test_dg_block();
        let dg = DgBlock::parse(&data, 1000).unwrap();

        assert_eq!(dg.header.block_type, *b"##DG");
        assert_eq!(dg.dg_next, 100);
        assert_eq!(dg.cg_first, 200);
        assert_eq!(dg.data, 300);
        assert_eq!(dg.md_comment, 400);
        assert_eq!(dg.rec_id_size, 0);
    }

    #[test]
    fn test_dg_block_invalid_type() {
        let mut data = create_test_dg_block();
        data[0..4].copy_from_slice(b"##XX");

        let result = DgBlock::parse(&data, 1000);
        assert!(matches!(result, Err(Mf4Error::InvalidBlockId { .. })));
    }
}
