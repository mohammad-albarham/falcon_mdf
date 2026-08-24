//! Data block (DT, DZ, DL, HL) parsing.
//!
//! Data blocks contain the actual sample data for channel groups.
//! They may be stored as plain data (DT), compressed data (DZ),
//! or organized in lists (DL, HL) for large files.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};

/// Type of data block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataBlockType {
    /// Plain data block (DT).
    Data,
    /// Sorted data block (SD).
    SortedData,
    /// Reduction data block (RD).
    ReductionData,
    /// Compressed data block (DZ).
    Compressed,
    /// Data list block (DL).
    DataList,
    /// Header list block (HL).
    HeaderList,
}

/// The Data (DT) block.
///
/// Contains raw sample data for one or more channel groups.
#[derive(Debug, Clone)]
pub struct DtBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Offset to the start of raw data within the block.
    pub data_offset: u64,
    /// Length of raw data in bytes.
    pub data_length: u64,
}

impl ParseBlock for DtBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        // DT, SD and RD are the same container; only what the records mean differs.
        if !matches!(&header.block_type, b"##DT" | b"##SD" | b"##RD") {
            header.validate_type(b"##DT", offset)?;
        }

        let data_offset = offset + BLOCK_HEADER_SIZE as u64;
        let data_length = header.length - BLOCK_HEADER_SIZE as u64;

        Ok(DtBlock {
            header,
            data_offset,
            data_length,
        })
    }
}

/// Compression type for DZ blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// Zlib (deflate) compression.
    Deflate,
    /// Transposition + deflate (for columnar data).
    TransposedDeflate,
    /// Zstandard compression.
    Zstd,
    /// Transposition + zstandard.
    TransposedZstd,
    /// LZ4 frame compression.
    Lz4,
    /// Transposition + LZ4 frame.
    TransposedLz4,
    /// Unknown compression type.
    Unknown(u8),
}

impl CompressionType {
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            0 => CompressionType::Deflate,
            1 => CompressionType::TransposedDeflate,
            2 => CompressionType::Zstd,
            3 => CompressionType::TransposedZstd,
            4 => CompressionType::Lz4,
            5 => CompressionType::TransposedLz4,
            v => CompressionType::Unknown(v),
        }
    }

    /// Returns true if this compression type applies transposition to the data.
    pub fn is_transposed(&self) -> bool {
        matches!(
            self,
            CompressionType::TransposedDeflate
                | CompressionType::TransposedZstd
                | CompressionType::TransposedLz4
        )
    }
}

/// The Compressed Data (DZ) block.
///
/// Contains zlib-compressed sample data.
#[derive(Debug, Clone)]
pub struct DzBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Original block type ("DT", "SD", "RD").
    pub original_type: [u8; 2],
    /// Compression type (0 = deflate, 1 = transposed deflate).
    pub zip_type: CompressionType,
    /// Reserved byte.
    pub reserved: u8,
    /// Compression parameter (transposition column size for type 1).
    pub zip_parameter: u32,
    /// Original (uncompressed) data length.
    pub original_size: u64,
    /// Compressed data length.
    pub compressed_size: u64,
    /// Offset to compressed data within the file.
    pub compressed_data_offset: u64,
}

impl DzBlock {
    /// Minimum size of the DZ block header.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 24;
}

impl ParseBlock for DzBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##DZ", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "DZ",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        let data_start = BLOCK_HEADER_SIZE;
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let mut original_type = [0u8; 2];
        cursor.read_exact(&mut original_type)?;
        let zip_type_raw = cursor.read_u8()?;
        let zip_type = CompressionType::from_u8(zip_type_raw);
        let reserved = cursor.read_u8()?;
        let zip_parameter = cursor.read_u32::<LittleEndian>()?;
        let original_size = cursor.read_u64::<LittleEndian>()?;
        let compressed_size = cursor.read_u64::<LittleEndian>()?;

        let compressed_data_offset = offset + Self::MIN_SIZE;

        Ok(DzBlock {
            header,
            original_type,
            zip_type,
            reserved,
            zip_parameter,
            original_size,
            compressed_size,
            compressed_data_offset,
        })
    }
}

/// The Data List (DL) block.
///
/// Links multiple data blocks together for large datasets.
#[derive(Debug, Clone)]
pub struct DlBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to next DL block (0 = none).
    pub dl_next: u64,
    /// Links to data blocks.
    pub data_links: Vec<u64>,
    /// DL flags.
    pub flags: u8,
    /// Reserved bytes.
    pub reserved: [u8; 3],
    /// Number of data blocks referenced.
    pub count: u32,
    /// Equal-length flag (if set, all referenced blocks have same length).
    pub equal_length: Option<u64>,
    /// Offset values for each data block (if not equal length).
    pub offsets: Vec<u64>,
    /// Time values for each data block (optional).
    pub time_values: Vec<i64>,
    /// Angle values for each data block (optional).
    pub angle_values: Vec<f64>,
    /// Distance values for each data block (optional).
    pub distance_values: Vec<f64>,
}

impl ParseBlock for DlBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##DL", offset)?;

        // Parse links
        let links_start = BLOCK_HEADER_SIZE;
        let dl_next = read_link(data, links_start)?;

        // Remaining links are data block references
        let data_link_count = header.link_count.saturating_sub(1) as usize;
        let mut data_links = Vec::with_capacity(data_link_count);
        for i in 0..data_link_count {
            data_links.push(read_link(data, links_start + 8 + i * 8)?);
        }

        // Parse data section
        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let flags = cursor.read_u8()?;
        let mut reserved = [0u8; 3];
        cursor.read_exact(&mut reserved)?;
        let count = cursor.read_u32::<LittleEndian>()?;

        // Equal length flag (bit 0)
        let equal_length = if (flags & 0x01) != 0 {
            Some(cursor.read_u64::<LittleEndian>()?)
        } else {
            None
        };

        // Every section below stores one fixed 8-byte entry per link, so the
        // declared count must fit the bytes the block actually carries. This is
        // checked before any `with_capacity`: `count` comes from the file, and
        // reserving count * 8 bytes per section on a crafted block would be an
        // unclamped, file-controlled allocation.
        let sections = u32::from(equal_length.is_none()) + (flags & 0x0E).count_ones();
        let needed = u64::from(count) * 8 * u64::from(sections);
        // The cursor reads within `data_section`, so its position is bounded by
        // the section's length and the subtraction cannot wrap.
        let remaining = data_section.len() - cursor.position() as usize;
        if needed > remaining as u64 {
            return Err(Mf4Error::truncated(
                offset + data_start as u64 + cursor.position(),
                usize::try_from(needed).unwrap_or(usize::MAX),
                remaining,
            ));
        }

        // Offsets (if not equal length)
        let offsets = if equal_length.is_none() {
            let mut offs = Vec::with_capacity(count as usize);
            for _ in 0..count {
                offs.push(cursor.read_u64::<LittleEndian>()?);
            }
            offs
        } else {
            Vec::new()
        };

        // Time values (bit 1)
        let time_values = if (flags & 0x02) != 0 {
            let mut times = Vec::with_capacity(count as usize);
            for _ in 0..count {
                times.push(cursor.read_i64::<LittleEndian>()?);
            }
            times
        } else {
            Vec::new()
        };

        // Angle values (bit 2)
        let angle_values = if (flags & 0x04) != 0 {
            let mut angles = Vec::with_capacity(count as usize);
            for _ in 0..count {
                angles.push(cursor.read_f64::<LittleEndian>()?);
            }
            angles
        } else {
            Vec::new()
        };

        // Distance values (bit 3)
        let distance_values = if (flags & 0x08) != 0 {
            let mut distances = Vec::with_capacity(count as usize);
            for _ in 0..count {
                distances.push(cursor.read_f64::<LittleEndian>()?);
            }
            distances
        } else {
            Vec::new()
        };

        Ok(DlBlock {
            header,
            dl_next,
            data_links,
            flags,
            reserved,
            count,
            equal_length,
            offsets,
            time_values,
            angle_values,
            distance_values,
        })
    }
}

/// The Header List (HL) block.
///
/// Organizes data blocks in a hierarchical structure for very large files.
#[derive(Debug, Clone)]
pub struct HlBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to first DL block.
    pub dl_first: u64,
    /// HL flags.
    pub flags: u16,
    /// Compression type (same as DZ).
    pub zip_type: CompressionType,
    /// Reserved bytes.
    pub reserved: [u8; 5],
}

impl ParseBlock for HlBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##HL", offset)?;

        let links_start = BLOCK_HEADER_SIZE;
        let dl_first = read_link(data, links_start)?;

        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let flags = cursor.read_u16::<LittleEndian>()?;
        let zip_type_raw = cursor.read_u8()?;
        let zip_type = CompressionType::from_u8(zip_type_raw);
        let mut reserved = [0u8; 5];
        cursor.read_exact(&mut reserved)?;

        Ok(HlBlock {
            header,
            dl_first,
            flags,
            zip_type,
            reserved,
        })
    }
}

/// Enum for different data block types.
#[derive(Debug)]
#[non_exhaustive]
pub enum DataBlock {
    /// Plain data block.
    Data(DtBlock),
    /// Compressed data block.
    Compressed(DzBlock),
    /// Data list block.
    DataList(DlBlock),
    /// Header list block.
    HeaderList(HlBlock),
}

impl DataBlock {
    /// Parses a data block, auto-detecting the type.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        if data.len() < 4 {
            return Err(Mf4Error::truncated(offset, 4, data.len()));
        }

        let block_id = &data[0..4];
        match block_id {
            b"##DT" => Ok(DataBlock::Data(DtBlock::parse(data, offset)?)),
            b"##SD" => Ok(DataBlock::Data(DtBlock::parse(data, offset)?)), // SD is like DT
            // Reduction data is a plain record container like DT; what differs
            // is the shape of the records inside it, not the block.
            b"##RD" => Ok(DataBlock::Data(DtBlock::parse(data, offset)?)),
            b"##DZ" => Ok(DataBlock::Compressed(DzBlock::parse(data, offset)?)),
            b"##DL" => Ok(DataBlock::DataList(DlBlock::parse(data, offset)?)),
            b"##HL" => Ok(DataBlock::HeaderList(HlBlock::parse(data, offset)?)),
            _ => Err(Mf4Error::invalid_block_id(
                offset,
                "##DT/DZ/DL/HL",
                String::from_utf8_lossy(block_id).to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_dt_block(data_len: usize) -> Vec<u8> {
        let total_len = BLOCK_HEADER_SIZE + data_len;
        let mut data = vec![0u8; total_len];

        // Header
        data[0..4].copy_from_slice(b"##DT");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&0u64.to_le_bytes()); // link_count

        // Fill data section with pattern
        for (i, byte) in data[BLOCK_HEADER_SIZE..].iter_mut().enumerate() {
            *byte = (i & 0xFF) as u8;
        }

        data
    }

    #[test]
    fn test_dt_block_parse() {
        let data = create_test_dt_block(100);
        let dt = DtBlock::parse(&data, 1000).unwrap();

        assert_eq!(dt.header.block_type, *b"##DT");
        assert_eq!(dt.data_length, 100);
    }

    #[test]
    fn test_data_block_auto_detect() {
        let dt_data = create_test_dt_block(50);
        let block = DataBlock::parse(&dt_data, 0).unwrap();
        assert!(matches!(block, DataBlock::Data(_)));
    }

    #[test]
    fn test_compression_type() {
        assert_eq!(CompressionType::from_u8(0), CompressionType::Deflate);
        assert_eq!(
            CompressionType::from_u8(1),
            CompressionType::TransposedDeflate
        );
        assert_eq!(CompressionType::from_u8(2), CompressionType::Zstd);
        assert_eq!(
            CompressionType::from_u8(3),
            CompressionType::TransposedZstd
        );
        assert_eq!(CompressionType::from_u8(4), CompressionType::Lz4);
        assert_eq!(
            CompressionType::from_u8(5),
            CompressionType::TransposedLz4
        );
        assert!(matches!(
            CompressionType::from_u8(99),
            CompressionType::Unknown(99)
        ));
    }

    /// Builds a DL block whose data section declares `count` entries but
    /// carries room for only `entries` of them (8 bytes each).
    fn create_test_dl_block(count: u32, entries: usize) -> Vec<u8> {
        let section_len = 8 + entries * 8; // flags/reserved/count + u64 entries
        let total_len = BLOCK_HEADER_SIZE + 8 + section_len;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##DL");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&1u64.to_le_bytes()); // dl_next only

        let section = BLOCK_HEADER_SIZE + 8;
        data[section] = 0; // flags: no equal length, no time/angle/distance
        data[section + 4..section + 8].copy_from_slice(&count.to_le_bytes());
        data
    }

    #[test]
    fn a_dl_count_beyond_the_data_section_is_an_error_not_an_allocation() {
        // count = u32::MAX asked for a ~34 GB offsets vector before any read
        // could fail; the parse must refuse the block as truncated instead.
        let data = create_test_dl_block(u32::MAX, 0);
        assert!(DlBlock::parse(&data, 0).is_err());
    }

    #[test]
    fn a_dl_count_that_fits_the_data_section_parses() {
        let mut data = create_test_dl_block(1, 1);
        let entry = BLOCK_HEADER_SIZE + 8 + 8;
        data[entry..entry + 8].copy_from_slice(&42u64.to_le_bytes());

        let dl = DlBlock::parse(&data, 0).unwrap();
        assert_eq!(dl.count, 1);
        assert_eq!(dl.offsets, vec![42]);
    }
}
