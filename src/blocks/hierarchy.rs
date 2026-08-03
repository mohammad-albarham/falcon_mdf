//! Channel Hierarchy (CH) block parsing.
//!
//! CH blocks describe a logical grouping of channels — a named subtree in the
//! channel hierarchy. Each CH block links to a set of element channels (CN
//! blocks) and may contain nested sub-hierarchies.
//!
//! The HD block links the first CH block; the rest form a chain via `ch_next`.

use crate::blocks::common::{read_links, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};

/// Links a CH block carries before its element triples: next sibling, first
/// child, name, comment.
const FIXED_LINKS: usize = 4;

/// One channel referenced by a hierarchy node.
///
/// The standard identifies a channel by the three blocks it takes to locate it,
/// not by a single link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChElement {
    /// Offset of the data group holding the channel.
    pub data_group: u64,
    /// Offset of the channel group within that data group.
    pub channel_group: u64,
    /// Offset of the channel itself.
    pub channel: u64,
}
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
    /// Link to this node's first child, or 0 when it has none.
    pub ch_first: u64,
    /// Link to the hierarchy name (TX block).
    pub tx_name: u64,
    /// Link to a comment (MD block).
    pub md_comment: u64,
    /// Links to element CN blocks.
    pub ch_element: Vec<ChElement>,
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

        // Four fixed links — next sibling, first child, name, comment — then
        // three per element: the data group, channel group and channel that
        // together identify one referenced channel.
        //
        // A hierarchy is a tree, so the child link matters: without it the
        // structure collapses to a flat list of siblings. And an element is a
        // triple, not a single link, so treating it as one both loses two
        // thirds of the references and mistakes the remainder for the wrong
        // blocks.
        let links_start = BLOCK_HEADER_SIZE;
        let all_links = read_links(data, links_start, header.link_count as usize)?;

        if all_links.len() < FIXED_LINKS {
            return Err(Mf4Error::invalid_block_size(
                "CH",
                header.link_count * 8,
                (FIXED_LINKS * 8) as u64,
            ));
        }

        let ch_next = all_links[0];
        let ch_first = all_links[1];
        let tx_name = all_links[2];
        let md_comment = all_links[3];

        let ch_element: Vec<ChElement> = all_links[FIXED_LINKS..]
            .chunks_exact(3)
            .map(|t| ChElement {
                data_group: t[0],
                channel_group: t[1],
                channel: t[2],
            })
            .collect();

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
            ch_first,
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

    /// Builds a CH block at the layout the standard specifies: four fixed links
    /// then three per element, and an eight-byte data section.
    fn create_test_ch_block(
        elements: &[(u64, u64, u64)],
        ch_type: u8,
        first_child: u64,
    ) -> Vec<u8> {
        let mut links = vec![100u64, first_child, 200, 300];
        for (dg, cg, cn) in elements {
            links.extend_from_slice(&[*dg, *cg, *cn]);
        }

        let total_len = BLOCK_HEADER_SIZE + links.len() * 8 + 8;
        let mut data = vec![0u8; total_len];
        data[0..4].copy_from_slice(b"##CH");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());

        for (i, link) in links.iter().enumerate() {
            let at = BLOCK_HEADER_SIZE + i * 8;
            data[at..at + 8].copy_from_slice(&link.to_le_bytes());
        }

        let d = BLOCK_HEADER_SIZE + links.len() * 8;
        data[d..d + 4].copy_from_slice(&(elements.len() as u32).to_le_bytes());
        data[d + 4] = ch_type;

        data
    }

    #[test]
    fn elements_are_read_as_triples_not_single_links() {
        // Each element names the data group, channel group and channel needed to
        // locate one channel. Reading them as single links loses two thirds of
        // the references and mistakes the rest for the wrong blocks.
        let data = create_test_ch_block(&[(10, 20, 30), (40, 50, 60)], 0, 0);
        let ch = ChBlock::parse(&data, 0).unwrap();

        assert_eq!(ch.ch_element_count, 2);
        assert_eq!(
            ch.ch_element,
            vec![
                ChElement {
                    data_group: 10,
                    channel_group: 20,
                    channel: 30
                },
                ChElement {
                    data_group: 40,
                    channel_group: 50,
                    channel: 60
                },
            ]
        );
    }

    #[test]
    fn the_child_link_is_read_and_does_not_displace_the_name() {
        // A hierarchy is a tree. Omitting the child link both loses the nesting
        // and shifts the name and comment links by one.
        let data = create_test_ch_block(&[], 0, 999);
        let ch = ChBlock::parse(&data, 0).unwrap();

        assert_eq!(ch.ch_next, 100);
        assert_eq!(ch.ch_first, 999, "the child link should be read");
        assert_eq!(ch.tx_name, 200, "the name link should not be displaced");
        assert_eq!(ch.md_comment, 300);
    }

    #[test]
    fn a_node_with_no_elements_parses() {
        let data = create_test_ch_block(&[], 1, 0);
        let ch = ChBlock::parse(&data, 0).unwrap();
        assert_eq!(ch.ch_element_count, 0);
        assert!(ch.ch_element.is_empty());
        assert_eq!(ch.ch_first, 0);
    }

    #[test]
    fn rejects_a_block_of_the_wrong_type() {
        let mut data = create_test_ch_block(&[(1, 2, 3)], 0, 0);
        data[0..4].copy_from_slice(b"##DG");
        assert!(ChBlock::parse(&data, 0).is_err());
    }

    #[test]
    fn rejects_a_block_without_its_fixed_links() {
        let mut data = create_test_ch_block(&[], 0, 0);
        data[16..24].copy_from_slice(&2u64.to_le_bytes()); // claims only two links
        assert!(ChBlock::parse(&data, 0).is_err());
    }
}
