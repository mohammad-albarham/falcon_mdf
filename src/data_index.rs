//! Data block indexing for lazy data access.
//!
//! This module provides `DataBlockInfo` and related types for efficient
//! lazy loading of measurement data. Instead of parsing or decompressing
//! data blocks upfront, we store minimal metadata needed to access them
//! on demand.
//!
//! ## Design Philosophy (inspired by asammdf)
//!
//! asammdf stores data block metadata in a `data_blocks` list per group:
//! - Block type (DT, DZ, etc.)
//! - File offset
//! - Compressed/original size
//! - Compression parameters
//!
//! This allows:
//! 1. Fast file opening (metadata only, no data parsing)
//! 2. Memory-efficient access (decompress only requested portions)
//! 3. Fragment-based iteration (stream large files without full load)

use crate::blocks::CompressionType;

/// Metadata about a single data block, enabling lazy access.
///
/// This struct stores the minimal information needed to read and decompress
/// a data block without actually loading its contents. This enables:
/// - Fast file opening (parse structure, skip data)
/// - Memory efficiency (load only requested data)
/// - Streaming access (iterate fragments without full materialization)
#[derive(Debug, Clone)]
pub struct DataBlockInfo {
    /// File offset of the data block.
    pub offset: u64,

    /// Block type identifier ("DT", "DZ", "SD", etc.).
    pub block_type: DataBlockType,

    /// Original (uncompressed) data size in bytes.
    pub original_size: u64,

    /// Compressed data size in bytes (same as original_size if uncompressed).
    pub compressed_size: u64,

    /// Compression type (if compressed).
    pub compression: Option<CompressionInfo>,
}

/// Type of data block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBlockType {
    /// Plain data block (##DT).
    Data,
    /// Sorted data block (##SD).
    SortedData,
    /// Compressed data block (##DZ).
    Compressed,
    /// Reduction data block (##RD).
    Reduction,
}

impl DataBlockType {
    /// Creates from the 2-byte block ID (excluding ##).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"DT" => Some(DataBlockType::Data),
            b"SD" => Some(DataBlockType::SortedData),
            b"DZ" => Some(DataBlockType::Compressed),
            b"RD" => Some(DataBlockType::Reduction),
            _ => None,
        }
    }

    /// Returns the 2-byte block ID.
    pub fn as_bytes(&self) -> &'static [u8; 2] {
        match self {
            DataBlockType::Data => b"DT",
            DataBlockType::SortedData => b"SD",
            DataBlockType::Compressed => b"DZ",
            DataBlockType::Reduction => b"RD",
        }
    }
}

/// Compression information for compressed data blocks.
#[derive(Debug, Clone, Copy)]
pub struct CompressionInfo {
    /// Compression algorithm.
    pub algorithm: CompressionType,

    /// Compression parameter (e.g., transposition column count).
    pub parameter: u32,

    /// Offset to compressed data within the file.
    pub data_offset: u64,
}

impl DataBlockInfo {
    /// Creates info for an uncompressed data block.
    pub fn uncompressed(offset: u64, block_type: DataBlockType, size: u64) -> Self {
        Self {
            offset,
            block_type,
            original_size: size,
            compressed_size: size,
            compression: None,
        }
    }

    /// Creates info for a compressed data block.
    pub fn compressed(
        offset: u64,
        original_size: u64,
        compressed_size: u64,
        compression: CompressionInfo,
    ) -> Self {
        Self {
            offset,
            block_type: DataBlockType::Compressed,
            original_size,
            compressed_size,
            compression: Some(compression),
        }
    }

    /// Returns true if this block is compressed.
    pub fn is_compressed(&self) -> bool {
        self.compression.is_some()
    }

    /// Returns the compression ratio (compressed/original).
    /// Returns 1.0 for uncompressed blocks.
    pub fn compression_ratio(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.compressed_size as f64 / self.original_size as f64
        }
    }
}

/// Index of all data blocks for a channel group.
///
/// This structure enables efficient access to data spread across multiple
/// blocks (common in large MDF4 files using DL/HL block chains).
#[derive(Debug, Clone, Default)]
pub struct DataBlockIndex {
    /// List of data blocks in order.
    blocks: Vec<DataBlockInfo>,

    /// Cumulative byte offsets for each block.
    /// `cumulative_offsets[i]` is the byte offset where block `i` starts
    /// in the conceptual concatenated data stream.
    cumulative_offsets: Vec<u64>,

    /// Total uncompressed data size.
    total_size: u64,
}

impl DataBlockIndex {
    /// Creates an empty data block index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an index with pre-allocated capacity.
    pub fn with_capacity(block_count: usize) -> Self {
        Self {
            blocks: Vec::with_capacity(block_count),
            cumulative_offsets: Vec::with_capacity(block_count),
            total_size: 0,
        }
    }

    /// Adds a data block to the index.
    pub fn push(&mut self, info: DataBlockInfo) {
        self.cumulative_offsets.push(self.total_size);
        self.total_size += info.original_size;
        self.blocks.push(info);
    }

    /// Returns the number of data blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns true if there are no data blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Returns the total uncompressed data size.
    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    /// Returns all data blocks.
    pub fn blocks(&self) -> &[DataBlockInfo] {
        &self.blocks
    }

    /// Returns the data block and local offset for a given global byte offset.
    ///
    /// This is useful for random access into the data stream.
    pub fn block_for_offset(&self, global_offset: u64) -> Option<(usize, &DataBlockInfo, u64)> {
        if global_offset >= self.total_size {
            return None;
        }

        // Binary search for the block containing this offset
        let block_idx = match self.cumulative_offsets.binary_search(&global_offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };

        let block = &self.blocks[block_idx];
        let local_offset = global_offset - self.cumulative_offsets[block_idx];

        Some((block_idx, block, local_offset))
    }

    /// Returns an iterator over data blocks with their global offsets.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &DataBlockInfo)> {
        self.cumulative_offsets
            .iter()
            .zip(self.blocks.iter())
            .map(|(&offset, info)| (offset, info))
    }

    /// Computes the average compression ratio across all blocks.
    pub fn average_compression_ratio(&self) -> f64 {
        if self.blocks.is_empty() {
            return 1.0;
        }

        let total_compressed: u64 = self.blocks.iter().map(|b| b.compressed_size).sum();
        let total_original: u64 = self.blocks.iter().map(|b| b.original_size).sum();

        if total_original == 0 {
            1.0
        } else {
            total_compressed as f64 / total_original as f64
        }
    }
}

/// Record layout information for a channel within a record.
///
/// Pre-computed layout enables efficient extraction of channel values
/// from records without repeated calculations.
#[derive(Debug, Clone, Copy)]
pub struct ChannelLayout {
    /// Byte offset within the record.
    pub byte_offset: usize,

    /// Number of bytes occupied by the value.
    pub byte_size: usize,

    /// Bit offset within the first byte.
    pub bit_offset: u8,

    /// Bit mask for extracting the value (for non-byte-aligned channels).
    pub bit_count: u32,

    /// Whether the value is signed.
    pub is_signed: bool,

    /// Whether the value is floating-point.
    pub is_float: bool,

    /// Whether the value uses little-endian byte order.
    pub little_endian: bool,
}

impl ChannelLayout {
    /// Creates a new channel layout.
    pub fn new(
        byte_offset: u32,
        bit_offset: u8,
        bit_count: u32,
        is_signed: bool,
        is_float: bool,
        little_endian: bool,
    ) -> Self {
        Self {
            byte_offset: byte_offset as usize,
            byte_size: (bit_offset as u32 + bit_count).div_ceil(8) as usize,
            bit_offset,
            bit_count,
            is_signed,
            is_float,
            little_endian,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_block_info_uncompressed() {
        let info = DataBlockInfo::uncompressed(1000, DataBlockType::Data, 4096);

        assert_eq!(info.offset, 1000);
        assert_eq!(info.original_size, 4096);
        assert_eq!(info.compressed_size, 4096);
        assert!(!info.is_compressed());
        assert_eq!(info.compression_ratio(), 1.0);
    }

    #[test]
    fn test_data_block_info_compressed() {
        let compression = CompressionInfo {
            algorithm: CompressionType::Deflate,
            parameter: 0,
            data_offset: 1024,
        };
        let info = DataBlockInfo::compressed(1000, 4096, 1024, compression);

        assert!(info.is_compressed());
        assert_eq!(info.compression_ratio(), 0.25);
    }

    #[test]
    fn test_data_block_index() {
        let mut index = DataBlockIndex::new();

        index.push(DataBlockInfo::uncompressed(100, DataBlockType::Data, 1000));
        index.push(DataBlockInfo::uncompressed(200, DataBlockType::Data, 500));
        index.push(DataBlockInfo::uncompressed(300, DataBlockType::Data, 500));

        assert_eq!(index.block_count(), 3);
        assert_eq!(index.total_size(), 2000);

        // Test offset lookup
        let (idx, _, local) = index.block_for_offset(0).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(local, 0);

        let (idx, _, local) = index.block_for_offset(1000).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(local, 0);

        let (idx, _, local) = index.block_for_offset(1200).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(local, 200);

        let (idx, _, local) = index.block_for_offset(1500).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(local, 0);

        // Out of bounds
        assert!(index.block_for_offset(2000).is_none());
    }

    #[test]
    fn test_data_block_type() {
        assert_eq!(DataBlockType::from_bytes(b"DT"), Some(DataBlockType::Data));
        assert_eq!(
            DataBlockType::from_bytes(b"SD"),
            Some(DataBlockType::SortedData)
        );
        assert_eq!(
            DataBlockType::from_bytes(b"DZ"),
            Some(DataBlockType::Compressed)
        );
        assert_eq!(DataBlockType::from_bytes(b"XX"), None);
    }
}

/// Positions of each channel group's records inside an unsorted data group.
///
/// In an unsorted data group, records belonging to different channel groups are
/// interleaved in one byte stream and each record is prefixed with a record ID
/// identifying its channel group. Records for different channel groups have
/// different sizes, so the stream cannot be addressed by a single stride: it has
/// to be walked once, and the position of every record remembered.
///
/// Offsets are relative to the data group's concatenated (and, where relevant,
/// decompressed) payload, and point at the record ID itself — a record's own
/// bytes begin `rec_id_size` bytes later.
#[derive(Debug, Clone, Default)]
pub struct RecordIndex {
    /// Record offsets per channel group, indexed by position within the data group.
    per_group: Vec<Vec<u64>>,
}

impl RecordIndex {
    /// Creates an index with room for `group_count` channel groups.
    pub fn with_groups(group_count: usize) -> Self {
        Self {
            per_group: vec![Vec::new(); group_count],
        }
    }

    /// Records that a channel group has a record starting at `offset`.
    pub fn push(&mut self, group: usize, offset: u64) {
        if let Some(slot) = self.per_group.get_mut(group) {
            slot.push(offset);
        }
    }

    /// Returns the record offsets for a channel group, or an empty slice.
    pub fn offsets(&self, group: usize) -> &[u64] {
        self.per_group.get(group).map_or(&[], |v| v.as_slice())
    }

    /// Returns the number of records found for a channel group.
    pub fn count(&self, group: usize) -> usize {
        self.per_group.get(group).map_or(0, |v| v.len())
    }

    /// Returns the number of channel groups covered by this index.
    pub fn group_count(&self) -> usize {
        self.per_group.len()
    }
}
