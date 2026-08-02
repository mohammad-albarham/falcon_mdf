//! Channel Hierarchy (CH) block parsing.
//!
//! CH blocks describe a logical grouping of channels — a named subtree in the
//! channel hierarchy. Each CH block links to a set of element channels (CN
//! blocks) and may contain nested sub-hierarchies.
//!
//! The HD block links the first CH block; the rest form a chain via `ch_next`.

use crate::blocks::common::{read_links, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Hierarchy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChType {
    /// Tree hierarchy (`ch_type = 0`): elements are ordered.
    Tree,
    /// Plain hierarchy (`ch_type = 1`): elements are unordered.
    Plain,
    /// Unknown hierarchy type.
    Unknown(u8),
}

impl ChType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => ChType::Tree,
            1 => ChType::Plain,
            v => ChType::Unknown(v),
        }
    }
}

/// The Channel Hierarchy (CH) block.
///
/// Describes a named group of channels. The `ch_element` links point to the
/// CN blocks that belong to this hierarchy level.
#[derive(Debug, Clone)]
pub struct ChBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the next CH block (0 = none).
    pub ch_next: u64,
    /// Link to the hierarchy name (TX block).
    pub tx_name: u64,
    /// Link to a comment (MD block).
    pub md_comment: u64,
    /// Links to element CN blocks.
    pub ch_element: Vec<u64>,
    /// Number of elements (from the data section).
    pub ch_element_count: u32,
    /// Hierarchy type.
    pub ch_type: ChType,
}

impl ChBlock {
    /// Minimum size of the CH block data section.
    pub const MIN_DATA_SIZE: usize = 4 + 1 + 3;
}

impl ParseBlock for ChBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CH", offset)?;

        // Links: ch_next, tx_name, md_comment, then ch_element_count element links.
        // The first 3 links are fixed; the rest are element links whose count
        // comes from the data section. We read all links the header declares,
        // then split off the first 3.
        let links_start = BLOCK_HEADER_SIZE;
        let all_links = read_links(data, links_start, header.link_count as usize)?;

        if all_links.len() < 3 {
            return Err(Mf4Error::invalid_block_size(
                "CH",
                header.link_count * 8,
                3 * 8,
            ));
        }

        let ch_next = all_links[0];
        let tx_name = all_links[1];
        let md_comment = all_links[2];
        let ch_element = all_links[3..].to_vec();

        // Parse data section
        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < Self::MIN_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                Self::MIN_DATA_SIZE,
                data_section.len(),
            ));
        }

        let mut cursor = Cursor::new(data_section);
        let ch_element_count = cursor.read_u32::<LittleEndian>()?;
        let ch_type = ChType::from_u8(cursor.read_u8()?);
        let _reserved = [cursor.read_u8()?, cursor.read_u8()?, cursor.read_u8()?];

        Ok(ChBlock {
            header,
            ch_next,
            tx_name,
            md_comment,
            ch_element,
            ch_element_count,
            ch_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_ch_block(element_count: u32, ch_type: u8) -> Vec<u8> {
        let element_links: Vec<u64> = (0..element_count).map(|i| 5000 + i as u64 * 100).collect();
        let link_count = 3 + element_links.len() as u64;
        let links: Vec<u64> = vec![100, 200, 300]
            .into_iter()
            .chain(element_links)
            .collect();
        let links_bytes: Vec<u8> = links.iter().flat_map(|l| l.to_le_bytes()).collect();

        let data_section: Vec<u8> = {
            let mut v = Vec::new();
            v.extend_from_slice(&element_count.to_le_bytes());
            v.push(ch_type);
            v.extend_from_slice(&[0u8; 3]); // reserved
            v
        };

        let total_len = BLOCK_HEADER_SIZE + links_bytes.len() + data_section.len();
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##CH");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&link_count.to_le_bytes());
        data[BLOCK_HEADER_SIZE..BLOCK_HEADER_SIZE + links_bytes.len()]
            .copy_from_slice(&links_bytes);
        let ds = BLOCK_HEADER_SIZE + links_bytes.len();
        data[ds..ds + data_section.len()].copy_from_slice(&data_section);

        data
    }

    #[test]
    fn test_ch_block_parse() {
        let data = create_test_ch_block(3, 0);
        let ch = ChBlock::parse(&data, 1000).unwrap();

        assert_eq!(ch.header.block_type, *b"##CH");
        assert_eq!(ch.ch_next, 100);
        assert_eq!(ch.tx_name, 200);
        assert_eq!(ch.md_comment, 300);
        assert_eq!(ch.ch_element_count, 3);
        assert_eq!(ch.ch_type, ChType::Tree);
        assert_eq!(ch.ch_element, vec![5000, 5100, 5200]);
    }

    #[test]
    fn test_ch_block_plain_type() {
        let data = create_test_ch_block(0, 1);
        let ch = ChBlock::parse(&data, 1000).unwrap();
        assert_eq!(ch.ch_type, ChType::Plain);
        assert!(ch.ch_element.is_empty());
    }

    #[test]
    fn test_ch_block_invalid_type() {
        let mut data = create_test_ch_block(1, 0);
        data[0..4].copy_from_slice(b"##XX");
        let result = ChBlock::parse(&data, 0);
        assert!(matches!(result, Err(Mf4Error::InvalidBlockId { .. })));
    }
}
