//! Attachment (AT) block parsing.
//!
//! AT blocks describe files attached to an MDF measurement: configuration
//! dumps, screenshots, DBC databases, or any other external artefact. An
//! attachment is either **embedded** (its bytes follow the AT block in the
//! file) or **external** (a path the reader is expected to resolve itself).
//!
//! The HD block links the first AT block; the rest form a chain via `at_next`.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};

/// Size of the AT block's data section: flags and creator index, four reserved
/// bytes, a 16-byte MD5 checksum, and the original and embedded sizes.
const AT_DATA_SIZE: usize = 2 + 2 + 4 + 16 + 8 + 8;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Attachment flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AtFlags {
    /// The attachment is embedded in the MF4 file (bit 0).
    pub embedded: bool,
    /// The embedded bytes are deflate-compressed (bit 1), and
    /// `embedded_size` counts them compressed while `original_size` gives
    /// the file's real length. Only meaningful together with `embedded`.
    pub compressed: bool,
    /// `md5_checksum` holds a checksum the writer computed (bit 2). When
    /// clear, those sixteen bytes mean nothing.
    pub md5_valid: bool,
}

impl AtFlags {
    /// Bit 1 is compression, not a second checksum. Reading it as one left
    /// compressed attachments to be handed back as a raw deflate stream.
    fn from_u16(value: u16) -> Self {
        AtFlags {
            embedded: (value & 0x0001) != 0,
            compressed: (value & 0x0002) != 0,
            md5_valid: (value & 0x0004) != 0,
        }
    }
}

/// The Attachment (AT) block.
///
/// Describes a file attached to the measurement. The attachment data is
/// embedded in the file when `embedded_size > 0` (the bytes follow the
/// block); otherwise the attachment is external and `tx_file_path` names
/// its location.
#[derive(Debug, Clone)]
pub struct AtBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the next AT block (0 = none).
    pub at_next: u64,
    /// Link to the file name (TX block).
    pub tx_file_name: u64,
    /// Link to the file path / MIME type (TX block).
    pub tx_file_path: u64,
    /// Link to a comment (MD block).
    pub md_comment: u64,
    /// Attachment flags.
    pub flags: AtFlags,
    /// Index of the creator (FH block), or 0.
    pub creator_index: u16,
    /// MD5 checksum of the attachment content (16 bytes).
    pub md5_checksum: [u8; 16],
    /// Original (uncompressed) size of the attachment in bytes.
    pub original_size: u64,
    /// Embedded size in bytes. Non-zero means the attachment data follows
    /// the block; zero means it is external.
    pub embedded_size: u64,
}

impl AtBlock {
    /// Minimum size of the AT block (header + 4 links + data section).
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 4 * 8 + AT_DATA_SIZE as u64;

    /// Returns true if the attachment data is embedded in the file.
    pub fn is_embedded(&self) -> bool {
        self.embedded_size > 0
    }

    /// Returns the file offset where embedded data begins, or 0 when external.
    ///
    /// The bytes sit *inside* the block, after its links and fixed fields — the
    /// block's declared length covers them. Reading from the end of the block
    /// instead lands past the payload and returns whatever follows, or nothing
    /// at all when the attachment is the last thing in the file.
    pub fn embedded_data_offset(&self) -> u64 {
        if self.is_embedded() {
            self.header.offset + self.header.data_offset() as u64 + AT_DATA_SIZE as u64
        } else {
            0
        }
    }
}

impl ParseBlock for AtBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##AT", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "AT",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        let links_start = BLOCK_HEADER_SIZE;
        let at_next = read_link(data, links_start)?;
        let tx_file_name = read_link(data, links_start + 8)?;
        let tx_file_path = read_link(data, links_start + 16)?;
        let md_comment = read_link(data, links_start + 24)?;

        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < AT_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                AT_DATA_SIZE,
                data_section.len(),
            ));
        }

        let mut cursor = Cursor::new(data_section);
        let flags_raw = cursor.read_u16::<LittleEndian>()?;
        let flags = AtFlags::from_u16(flags_raw);
        let creator_index = cursor.read_u16::<LittleEndian>()?;
        // Four reserved bytes, not two. Reading two leaves the checksum and both
        // sizes short by two bytes, so every one of them is wrong.
        let _reserved = cursor.read_u32::<LittleEndian>()?;

        let mut md5_checksum = [0u8; 16];
        std::io::Read::read_exact(&mut cursor, &mut md5_checksum)?;

        let original_size = cursor.read_u64::<LittleEndian>()?;
        let embedded_size = cursor.read_u64::<LittleEndian>()?;

        Ok(AtBlock {
            header,
            at_next,
            tx_file_name,
            tx_file_path,
            md_comment,
            flags,
            creator_index,
            md5_checksum,
            original_size,
            embedded_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an AT block at the offsets the standard specifies: four links,
    /// then flags, creator index, four reserved bytes, a 16-byte checksum, and
    /// the original and embedded sizes.
    fn create_test_at_block(embedded_size: u64, flags: u16) -> Vec<u8> {
        let links = 4usize;
        let total_len = BLOCK_HEADER_SIZE + links * 8 + AT_DATA_SIZE;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##AT");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&(links as u64).to_le_bytes());

        data[24..32].copy_from_slice(&100u64.to_le_bytes()); // at_next
        data[32..40].copy_from_slice(&200u64.to_le_bytes()); // tx_file_name
        data[40..48].copy_from_slice(&300u64.to_le_bytes()); // tx_file_path
        data[48..56].copy_from_slice(&400u64.to_le_bytes()); // md_comment

        let d = BLOCK_HEADER_SIZE + links * 8;
        data[d..d + 2].copy_from_slice(&flags.to_le_bytes());
        data[d + 2..d + 4].copy_from_slice(&9u16.to_le_bytes()); // creator index
                                                                 // d+4..d+8 reserved
        for (k, byte) in data[d + 8..d + 24].iter_mut().enumerate() {
            *byte = k as u8; // a recognisable checksum
        }
        data[d + 24..d + 32].copy_from_slice(&1024u64.to_le_bytes()); // original
        data[d + 32..d + 40].copy_from_slice(&embedded_size.to_le_bytes());

        data
    }

    #[test]
    fn test_at_block_parse_external() {
        let data = create_test_at_block(0, 0);
        let at = AtBlock::parse(&data, 5000).unwrap();

        assert_eq!(at.header.block_type, *b"##AT");
        assert_eq!(at.at_next, 100);
        assert_eq!(at.tx_file_name, 200);
        assert!(!at.is_embedded());
        assert_eq!(at.original_size, 1024);
        assert_eq!(at.embedded_data_offset(), 0);
    }

    #[test]
    fn test_at_block_parse_embedded() {
        let data = create_test_at_block(512, 0x0001);
        let at = AtBlock::parse(&data, 5000).unwrap();

        assert!(at.flags.embedded);
        assert!(at.is_embedded());
        assert_eq!(at.embedded_size, 512);
        assert_eq!(
            at.embedded_data_offset(),
            5000 + (BLOCK_HEADER_SIZE + 4 * 8 + 40) as u64
        );
    }

    /// Each bit is set on its own, so a bit read at the wrong position shows up
    /// as the wrong field rather than hiding behind a neighbour. Bit 1 was read
    /// as a checksum-valid flag for a long time; it is compression, and getting
    /// it wrong meant a compressed attachment was handed back still deflated.
    #[test]
    fn flag_bits_sit_where_the_standard_puts_them() {
        let only_embedded = AtBlock::parse(&create_test_at_block(512, 0x0001), 0).unwrap();
        assert!(only_embedded.flags.embedded);
        assert!(!only_embedded.flags.compressed);
        assert!(!only_embedded.flags.md5_valid);

        let compressed = AtBlock::parse(&create_test_at_block(512, 0x0002), 0).unwrap();
        assert!(!compressed.flags.embedded);
        assert!(
            compressed.flags.compressed,
            "bit 1 is compressed embedded data"
        );
        assert!(!compressed.flags.md5_valid);

        let md5 = AtBlock::parse(&create_test_at_block(512, 0x0004), 0).unwrap();
        assert!(!md5.flags.embedded);
        assert!(!md5.flags.compressed);
        assert!(md5.flags.md5_valid, "bit 2 marks the checksum as valid");

        let all = AtBlock::parse(&create_test_at_block(512, 0x0007), 0).unwrap();
        assert!(all.flags.embedded && all.flags.compressed && all.flags.md5_valid);
    }

    #[test]
    fn test_at_block_invalid_type() {
        let mut data = create_test_at_block(0, 0);
        data[0..4].copy_from_slice(b"##XX");
        let result = AtBlock::parse(&data, 0);
        assert!(matches!(result, Err(Mf4Error::InvalidBlockId { .. })));
    }
}
