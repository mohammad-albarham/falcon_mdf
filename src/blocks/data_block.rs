//! Data block (DT, DZ, DL, HL) parsing.
//!
//! Data blocks contain the actual sample data for channel groups.
//! They may be stored as plain data (DT), compressed data (DZ),
//! or organized in lists (DL, HL) for large files.

use crate::error::{Mf4Error, Result};
use crate::blocks::common::{BlockHeader, read_link, BLOCK_HEADER_SIZE, ParseBlock};
use byteorder::{LittleEndian, ReadBytesExt};
use flate2::read::ZlibDecoder;
use std::io::{Cursor, Read};

/// Type of data block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        header.validate_type(b"##DT", offset)?;

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
    /// Unknown compression type.
    Unknown(u8),
}

impl CompressionType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => CompressionType::Deflate,
            1 => CompressionType::TransposedDeflate,
            v => CompressionType::Unknown(v),
        }
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

    /// Decompresses the data block.
    ///
    /// # Arguments
    /// * `compressed_data` - The compressed bytes from the file
    ///
    /// # Returns
    /// The decompressed data or an error if decompression fails.
    pub fn decompress(&self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        match self.zip_type {
            CompressionType::Deflate => {
                let mut decoder = ZlibDecoder::new(compressed_data);
                let mut decompressed = Vec::with_capacity(self.original_size as usize);
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|e| Mf4Error::Decompression(e.to_string()))?;
                Ok(decompressed)
            }
            CompressionType::TransposedDeflate => {
                // First decompress
                let mut decoder = ZlibDecoder::new(compressed_data);
                let mut transposed = Vec::with_capacity(self.original_size as usize);
                decoder
                    .read_to_end(&mut transposed)
                    .map_err(|e| Mf4Error::Decompression(e.to_string()))?;

                // Then un-transpose
                let column_size = self.zip_parameter as usize;
                if column_size == 0 {
                    return Err(Mf4Error::Decompression(
                        "Invalid transposition parameter".to_string(),
                    ));
                }

                let row_count = (transposed.len() + column_size - 1) / column_size;
                let mut result = vec![0u8; transposed.len()];

                for (src_idx, &byte) in transposed.iter().enumerate() {
                    let col = src_idx / row_count;
                    let row = src_idx % row_count;
                    let dst_idx = row * column_size + col;
                    if dst_idx < result.len() {
                        result[dst_idx] = byte;
                    }
                }

                Ok(result)
            }
            CompressionType::Unknown(t) => Err(Mf4Error::Decompression(format!(
                "Unknown compression type: {}",
                t
            ))),
        }
    }
}

impl ParseBlock for DzBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##DZ", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size("DZ", header.length, Self::MIN_SIZE));
        }

        let data_start = BLOCK_HEADER_SIZE;
        let data_section = &data[data_start..];
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
        let data_section = &data[data_start..];
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
        let data_section = &data[data_start..];
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
        assert_eq!(CompressionType::from_u8(1), CompressionType::TransposedDeflate);
        assert!(matches!(CompressionType::from_u8(99), CompressionType::Unknown(99)));
    }
}
