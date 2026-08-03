//! Common block structures and traits shared across all MF4 block types.
//!
//! All MF4 blocks share a common header structure. This module provides
//! the base types and utilities for working with block headers and links.

use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// The size of a standard MF4 block header in bytes.
pub const BLOCK_HEADER_SIZE: usize = 24;

/// The size of the ID block (always at offset 0).
pub const ID_BLOCK_SIZE: usize = 64;

/// A common header present at the start of all MF4 blocks (except ID block).
///
/// The block header contains:
/// - A 4-byte block type identifier (e.g., "##HD", "##DG")
/// - Reserved bytes
/// - Block length in bytes
/// - Link count
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    /// The 4-character block type identifier.
    pub block_type: [u8; 4],
    /// Reserved bytes (should be zero).
    pub reserved: [u8; 4],
    /// Total length of the block in bytes, including header.
    pub length: u64,
    /// Number of links in the link section.
    pub link_count: u64,
    /// File offset where this block starts (not part of on-disk format).
    pub offset: u64,
}

impl BlockHeader {
    /// Parses a block header from raw bytes.
    ///
    /// # Arguments
    /// * `data` - A slice containing at least 24 bytes
    /// * `offset` - The file offset (for error reporting)
    ///
    /// # Returns
    /// The parsed `BlockHeader` or an error if parsing fails.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        if data.len() < BLOCK_HEADER_SIZE {
            return Err(Mf4Error::truncated(offset, BLOCK_HEADER_SIZE, data.len()));
        }

        let mut cursor = Cursor::new(data);

        let mut block_type = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut block_type)?;

        let mut reserved = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut reserved)?;

        let length = cursor.read_u64::<LittleEndian>()?;
        let link_count = cursor.read_u64::<LittleEndian>()?;

        // Both fields come straight off disk and drive later allocations and
        // slice bounds, so they are checked for self-consistency here — the one
        // place every block parse passes through. Without this, a corrupt
        // link_count reaches `Vec::with_capacity` and aborts the process with an
        // allocation failure, which a caller cannot catch.
        if length < BLOCK_HEADER_SIZE as u64 {
            return Err(Mf4Error::invalid_block_size(
                String::from_utf8_lossy(&block_type).to_string(),
                length,
                BLOCK_HEADER_SIZE as u64,
            ));
        }

        // A block stores its links immediately after the header, so they have to
        // fit inside the block's own declared length.
        let links_size = link_count.checked_mul(8).ok_or_else(|| {
            Mf4Error::invalid_block_size(
                String::from_utf8_lossy(&block_type).to_string(),
                length,
                u64::MAX,
            )
        })?;
        let minimum = links_size.saturating_add(BLOCK_HEADER_SIZE as u64);
        if minimum > length {
            return Err(Mf4Error::invalid_block_size(
                String::from_utf8_lossy(&block_type).to_string(),
                length,
                minimum,
            ));
        }

        Ok(BlockHeader {
            block_type,
            reserved,
            length,
            link_count,
            offset,
        })
    }

    /// Returns the block type as a string slice.
    pub fn block_type_str(&self) -> &str {
        std::str::from_utf8(&self.block_type).unwrap_or("????")
    }

    /// Validates that this header has the expected block type.
    pub fn validate_type(&self, expected: &[u8; 4], offset: u64) -> Result<()> {
        if &self.block_type != expected {
            return Err(Mf4Error::invalid_block_id(
                offset,
                String::from_utf8_lossy(expected).to_string(),
                String::from_utf8_lossy(&self.block_type).to_string(),
            ));
        }
        Ok(())
    }

    /// Returns the size of the data section (total length minus header and links).
    ///
    /// `BlockHeader::parse` has already established that the header and links
    /// fit within `length`, so this cannot underflow; the saturating operations
    /// keep it total regardless.
    pub fn data_size(&self) -> u64 {
        self.length
            .saturating_sub(BLOCK_HEADER_SIZE as u64)
            .saturating_sub(self.link_count.saturating_mul(8))
    }

    /// Returns the offset where links start within the block.
    pub fn links_offset(&self) -> usize {
        BLOCK_HEADER_SIZE
    }

    /// Returns the offset where data starts within the block (after header and links).
    pub fn data_offset(&self) -> usize {
        BLOCK_HEADER_SIZE.saturating_add((self.link_count as usize).saturating_mul(8))
    }
}

/// Block type identifiers for MF4 blocks.
pub mod block_ids {
    /// Identification block (always at file offset 0)
    pub const ID: &[u8; 4] = b"MDF ";
    /// Header block
    pub const HD: &[u8; 4] = b"##HD";
    /// File history block
    pub const FH: &[u8; 4] = b"##FH";
    /// Data group block
    pub const DG: &[u8; 4] = b"##DG";
    /// Channel group block
    pub const CG: &[u8; 4] = b"##CG";
    /// Channel block
    pub const CN: &[u8; 4] = b"##CN";
    /// Source information block
    pub const SI: &[u8; 4] = b"##SI";
    /// Channel conversion block
    pub const CC: &[u8; 4] = b"##CC";
    /// Text block
    pub const TX: &[u8; 4] = b"##TX";
    /// Metadata (XML) block
    pub const MD: &[u8; 4] = b"##MD";
    /// Data block
    pub const DT: &[u8; 4] = b"##DT";
    /// Sorted data block
    pub const SD: &[u8; 4] = b"##SD";
    /// Reduction data block  
    pub const RD: &[u8; 4] = b"##RD";
    /// Compressed data block (zlib)
    pub const DZ: &[u8; 4] = b"##DZ";
    /// Data list block
    pub const DL: &[u8; 4] = b"##DL";
    /// Header list block
    pub const HL: &[u8; 4] = b"##HL";
    /// Signal data block
    pub const SR: &[u8; 4] = b"##SR";
    /// Attachment block
    pub const AT: &[u8; 4] = b"##AT";
    /// Event block
    pub const EV: &[u8; 4] = b"##EV";
    /// Channel array block
    pub const CA: &[u8; 4] = b"##CA";
    /// Channel hierarchy block
    pub const CH: &[u8; 4] = b"##CH";
}

/// Reads a 64-bit link value from a byte slice.
///
/// # Arguments
/// * `data` - The byte slice containing the link
/// * `offset` - The offset within the slice where the link starts
pub fn read_link(data: &[u8], offset: usize) -> Result<u64> {
    if offset + 8 > data.len() {
        return Err(Mf4Error::truncated(
            offset as u64,
            8,
            data.len().saturating_sub(offset),
        ));
    }
    let mut cursor = Cursor::new(&data[offset..offset + 8]);
    Ok(cursor.read_u64::<LittleEndian>()?)
}

/// Reads multiple consecutive link values from a byte slice.
///
/// # Arguments
/// * `data` - The byte slice containing the links
/// * `offset` - The starting offset within the slice
/// * `count` - The number of links to read
pub fn read_links(data: &[u8], offset: usize, count: usize) -> Result<Vec<u64>> {
    let mut links = Vec::with_capacity(count);
    for i in 0..count {
        links.push(read_link(data, offset + i * 8)?);
    }
    Ok(links)
}

/// A trait for blocks that can be parsed from raw bytes.
pub trait ParseBlock: Sized {
    /// Parses the block from a byte slice.
    ///
    /// # Arguments
    /// * `data` - The raw bytes of the entire block (including header)
    /// * `offset` - The file offset of this block (for error reporting)
    fn parse(data: &[u8], offset: u64) -> Result<Self>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_header_parse() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(b"##HD");
        data[4..8].copy_from_slice(&[0, 0, 0, 0]); // reserved
        data[8..16].copy_from_slice(&104u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&6u64.to_le_bytes()); // link_count

        let header = BlockHeader::parse(&data, 0).unwrap();
        assert_eq!(header.block_type, *b"##HD");
        assert_eq!(header.length, 104);
        assert_eq!(header.link_count, 6);
    }

    #[test]
    fn test_block_header_too_short() {
        let data = vec![0u8; 10];
        let result = BlockHeader::parse(&data, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_link() {
        let mut data = vec![0u8; 16];
        data[0..8].copy_from_slice(&0x1234567890ABCDEFu64.to_le_bytes());
        data[8..16].copy_from_slice(&0xFEDCBA0987654321u64.to_le_bytes());

        assert_eq!(read_link(&data, 0).unwrap(), 0x1234567890ABCDEF);
        assert_eq!(read_link(&data, 8).unwrap(), 0xFEDCBA0987654321);
    }

    #[test]
    fn test_read_links() {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(&100u64.to_le_bytes());
        data[8..16].copy_from_slice(&200u64.to_le_bytes());
        data[16..24].copy_from_slice(&300u64.to_le_bytes());

        let links = read_links(&data, 0, 3).unwrap();
        assert_eq!(links, vec![100, 200, 300]);
    }
}
