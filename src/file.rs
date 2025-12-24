//! High-level MF4 file interface.
//!
//! This module provides the main entry point for reading MF4 files.
//! The `Mf4File` type wraps all the complexity of the file format
//! and provides a clean, ergonomic API.
//!
//! ## Design Philosophy
//!
//! Inspired by asammdf's architecture, this implementation emphasizes:
//!
//! 1. **Lazy data loading**: Structure is parsed eagerly, data lazily
//! 2. **Block caching**: Shared blocks (CC, TX, SI) parsed once, reused via `Arc`
//! 3. **O(1) channel lookup**: `ChannelsDB` enables fast name-based access
//! 4. **Fragment-based iteration**: Large files can be streamed without full load
//! 5. **Parallel parsing**: Channel blocks parsed concurrently with rayon

use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;

use crate::cache::BlockCache;
use crate::channels_db::{ChannelLocation, ChannelsDB, MastersDB};
use crate::data_index::{
    CompressionInfo, DataBlockIndex, DataBlockInfo, DataBlockType,
};
use crate::error::{Mf4Error, Result};
use crate::io::{ByteSource, IoBackend};
use crate::blocks::{
    CgBlock, CnBlock, DgBlock, HdBlock, DzBlock, DlBlock, HlBlock,
    Conversion, ParseBlock, BLOCK_HEADER_SIZE, CompressionType,
};
use crate::model::{
    Channel, ChannelGroup, DataGroup, FileStatistics, RecordingTime, Signal,
};
use crate::parser::{self, Mf4Version, parse_id_block, parse_hd_block};

/// Configuration options for opening MF4 files.
///
/// Use this to control parsing behavior and memory usage.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Whether to build the channel name index for O(1) lookup.
    /// Enabled by default; disable if you only access channels by group/index.
    pub build_channels_db: bool,
    
    /// Whether to use parallel parsing for channel blocks.
    /// Enabled by default for files with many channels.
    pub parallel_parsing: bool,
    
    /// Minimum number of channels to trigger parallel parsing.
    /// Below this threshold, sequential parsing is used.
    pub parallel_threshold: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            build_channels_db: true,
            parallel_parsing: true,
            parallel_threshold: 100,
        }
    }
}

/// The main interface for reading MF4 files.
///
/// `Mf4File` provides access to all data in an MF4 measurement file,
/// including metadata, channel definitions, and sample data.
///
/// ## Performance Characteristics
///
/// - **Opening**: Parses file structure (fast, O(blocks))
/// - **Channel lookup by name**: O(1) via `ChannelsDB`
/// - **Channel iteration**: O(channels)
/// - **Signal access**: O(data_size), lazy decompression
///
/// ## Memory Usage
///
/// - File structure kept in memory
/// - Raw data read on demand via mmap
/// - Shared blocks (CC, TX) use `Arc` for deduplication
///
/// # Example
/// ```no_run
/// use falcon_mdf::Mf4File;
///
/// let file = Mf4File::open("measurement.mf4")?;
///
/// println!("Version: {}", file.version());
/// println!("Channels: {}", file.channel_count());
///
/// // Fast O(1) lookup by name
/// if let Some(channel) = file.find_channel("VehicleSpeed") {
///     let signal = file.signal(channel)?;
///     let values = signal.values_f64()?;
///     println!("First value: {}", values[0]);
/// }
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub struct Mf4File {
    /// I/O backend for reading file data.
    source: IoBackend,
    
    /// MF4 format version.
    version: Mf4Version,
    
    /// Recording start time.
    start_time: RecordingTime,
    
    /// File comment/description.
    comment: Arc<str>,
    
    /// Data groups containing channel groups and channels.
    data_groups: Vec<DataGroup>,
    
    /// Fast channel lookup by name.
    channels_db: ChannelsDB,
    
    /// Master channel index per channel group.
    masters_db: MastersDB,
    
    /// Total file size in bytes.
    file_size: u64,
    
    /// Block cache (for any future lookups that need shared blocks).
    /// Note: Most caching happens during parse, but cache is kept for
    /// potential future use (e.g., lazy conversion block loading).
    #[allow(dead_code)]
    cache: BlockCache,
}

impl Mf4File {
    /// Opens an MF4 file for reading with default options.
    ///
    /// This method opens the file, validates the format, and parses
    /// the structure (data groups, channel groups, channels). The actual
    /// sample data is read lazily when requested.
    ///
    /// # Arguments
    /// * `path` - Path to the MF4 file
    ///
    /// # Returns
    /// An `Mf4File` instance or an error if the file cannot be opened
    /// or parsed.
    ///
    /// # Example
    /// ```no_run
    /// use falcon_mdf::Mf4File;
    ///
    /// let file = Mf4File::open("data.mf4")?;
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_options(path, OpenOptions::default())
    }

    /// Opens an MF4 file with custom options.
    pub fn open_with_options<P: AsRef<Path>>(path: P, options: OpenOptions) -> Result<Self> {
        let source = IoBackend::open(path)?;
        Self::from_source_with_options(source, options)
    }

    /// Opens an MF4 file using memory-mapped I/O.
    ///
    /// This is the most efficient method for large files.
    #[cfg(feature = "mmap")]
    pub fn open_mmap<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = IoBackend::open_mmap(path)?;
        Self::from_source_with_options(source, OpenOptions::default())
    }

    /// Opens an MF4 file using buffered I/O.
    ///
    /// Use this method when memory-mapped I/O is not available
    /// or not desired (e.g., for network files).
    pub fn open_buffered<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = IoBackend::open_buffered(path)?;
        Self::from_source_with_options(source, OpenOptions::default())
    }

    /// Creates an Mf4File from a byte source with options.
    fn from_source_with_options(source: IoBackend, options: OpenOptions) -> Result<Self> {
        let file_size = source.len();

        // Parse ID block
        let id_block = parse_id_block(&source)?;
        let version = Mf4Version::from_id_block(&id_block);
        let is_unfinished = id_block.is_unfinished();
        
        // Validate version is supported
        version.validate()?;

        // Parse HD block (always at offset 64)
        let hd_block = parse_hd_block(&source, 64)?;

        // Initialize block cache
        let mut cache = BlockCache::new();

        // Extract start time
        let start_time = RecordingTime::new(
            hd_block.start_time_ns,
            hd_block.tz_offset_min,
            hd_block.dst_offset_min,
        );

        // Read file comment (cached)
        let comment = cache.get_or_parse_text(&source, hd_block.md_comment)?;

        // Parse data groups with caching
        let data_groups = Self::parse_data_groups(&source, &hd_block, &mut cache, &options, is_unfinished, file_size)?;

        // Build channel lookup indices
        let (channels_db, masters_db) = if options.build_channels_db {
            Self::build_channel_indices(&data_groups)
        } else {
            (ChannelsDB::new(), MastersDB::new())
        };

        Ok(Mf4File {
            source,
            version,
            start_time,
            comment,
            data_groups,
            channels_db,
            masters_db,
            file_size,
            cache,
        })
    }

    /// Builds channel name and master indices from parsed data groups.
    fn build_channel_indices(data_groups: &[DataGroup]) -> (ChannelsDB, MastersDB) {
        // Estimate capacity
        let total_channels: usize = data_groups
            .iter()
            .flat_map(|dg| &dg.channel_groups)
            .map(|cg| cg.channels.len())
            .sum();

        let mut channels_db = ChannelsDB::with_capacity(total_channels);
        let mut masters_db = MastersDB::new();

        for (dg_idx, dg) in data_groups.iter().enumerate() {
            for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
                for (ch_idx, ch) in cg.channels.iter().enumerate() {
                    // Add to channel name index
                    channels_db.insert(
                        &ch.name,
                        ChannelLocation::new(dg_idx, cg_idx, ch_idx),
                    );

                    // Track master channels
                    if ch.is_master() {
                        masters_db.insert(dg_idx, cg_idx, ch_idx);
                    }
                }
            }
        }

        (channels_db, masters_db)
    }

    /// Parses all data groups from the file.
    fn parse_data_groups(
        source: &IoBackend,
        hd: &HdBlock,
        cache: &mut BlockCache,
        options: &OpenOptions,
        is_unfinished: bool,
        file_size: u64,
    ) -> Result<Vec<DataGroup>> {
        let mut data_groups = Vec::new();
        let mut dg_offset = hd.dg_first;
        let mut dg_index = 0;

        while dg_offset != 0 {
            let dg_block = parser::parse_dg_block(source, dg_offset)?;
            
            // Parse channel groups for this data group
            let channel_groups = Self::parse_channel_groups(
                source,
                &dg_block,
                dg_index,
                cache,
                options,
            )?;

            // Read data group comment (cached)
            let comment = cache.get_or_parse_text(source, dg_block.md_comment)?;

            // Build data block index for lazy loading
            let data_block_index = if dg_block.data != 0 {
                Self::build_data_block_index(source, dg_block.data, is_unfinished, file_size)?
            } else {
                DataBlockIndex::new()
            };

            // Calculate sample counts for unfinished files or when cycle_count is 0
            let mut channel_groups = channel_groups;
            Self::calculate_sample_counts(
                source,
                &dg_block,
                &data_block_index,
                &mut channel_groups,
            )?;

            let data_group = DataGroup {
                id: dg_index,
                index: dg_index,
                channel_groups,
                comment: comment.to_string(),
                dg_offset,
                data_offset: dg_block.data,
                rec_id_size: dg_block.rec_id_size,
                data_block_index,
            };

            data_groups.push(data_group);
            dg_offset = dg_block.dg_next;
            dg_index += 1;
        }

        Ok(data_groups)
    }

    /// Calculates sample counts for channel groups when not set (unfinished files).
    ///
    /// For unsorted data with record IDs, scans the data block to count records.
    /// For sorted data or single channel group, calculates from data size.
    fn calculate_sample_counts(
        source: &IoBackend,
        dg: &DgBlock,
        data_index: &DataBlockIndex,
        channel_groups: &mut [ChannelGroup],
    ) -> Result<()> {
        if data_index.is_empty() {
            return Ok(());
        }

        // Check if any channel group needs sample count calculation
        let needs_calculation = channel_groups.iter().any(|cg| cg.sample_count == 0);
        if !needs_calculation {
            return Ok(());
        }

        let data_size = data_index.total_size();
        let rec_id_size = dg.rec_id_size;

        // Single channel group or no record IDs - simple calculation
        if channel_groups.len() == 1 || rec_id_size == 0 {
            for cg in channel_groups.iter_mut() {
                if cg.sample_count == 0 {
                    let record_size = cg.record_size(rec_id_size);
                    if record_size > 0 {
                        cg.sample_count = data_size / record_size as u64;
                    }
                }
            }
            return Ok(());
        }

        // Multiple channel groups with record IDs - need to scan data
        // Build a map of record_id -> (cg_index, record_size, is_vlsd)
        let mut record_map: std::collections::HashMap<u64, (usize, usize, bool)> = 
            std::collections::HashMap::new();
        for (idx, cg) in channel_groups.iter().enumerate() {
            let record_size = cg.record_size(rec_id_size);
            record_map.insert(cg.record_id, (idx, record_size, cg.is_vlsd));
        }

        // Initialize counters
        let mut counts: Vec<u64> = vec![0; channel_groups.len()];

        // Read and scan data blocks
        for (_offset, block_info) in data_index.iter() {
            let block_data = if let Some(compression) = &block_info.compression {
                // Read compressed data and decompress
                let compressed_data = source.read_bytes(
                    compression.data_offset,
                    block_info.compressed_size as usize,
                )?;
                Self::decompress(&compressed_data, compression, block_info.original_size as usize)?
            } else {
                // Read uncompressed data
                let data_offset = block_info.offset + BLOCK_HEADER_SIZE as u64;
                source.read_bytes(data_offset, block_info.original_size as usize)?.into_owned()
            };

            // Scan records
            let mut pos: usize = 0;
            let data: &[u8] = &block_data;
            
            while pos < data.len() {
                // Read record ID
                let rec_id = match rec_id_size {
                    1 => {
                        if pos >= data.len() { break; }
                        data[pos] as u64
                    }
                    2 => {
                        if pos + 2 > data.len() { break; }
                        u16::from_le_bytes([data[pos], data[pos + 1]]) as u64
                    }
                    4 => {
                        if pos + 4 > data.len() { break; }
                        u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as u64
                    }
                    8 => {
                        if pos + 8 > data.len() { break; }
                        u64::from_le_bytes([
                            data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
                            data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]
                        ])
                    }
                    _ => break,
                };

                // Find matching channel group and advance
                if let Some(&(cg_idx, record_size, is_vlsd)) = record_map.get(&rec_id) {
                    counts[cg_idx] += 1;
                    
                    if is_vlsd {
                        // VLSD record format: rec_id + length(4 bytes LE) + data(length bytes)
                        let length_offset = pos + rec_id_size as usize;
                        if length_offset + 4 > data.len() { break; }
                        let vlsd_len = u32::from_le_bytes([
                            data[length_offset], 
                            data[length_offset + 1], 
                            data[length_offset + 2], 
                            data[length_offset + 3]
                        ]) as usize;
                        pos = length_offset + 4 + vlsd_len;
                    } else {
                        pos += record_size;
                    }
                } else {
                    // Unknown record ID - try to continue with smallest record size
                    // This shouldn't happen with valid files
                    break;
                }
            }
        }

        // Apply calculated counts
        for (idx, cg) in channel_groups.iter_mut().enumerate() {
            if cg.sample_count == 0 {
                cg.sample_count = counts[idx];
            }
        }

        Ok(())
    }

    /// Builds a data block index for lazy data access.
    /// 
    /// For unfinished files, the DT block header may have length=24 (just header)
    /// while the actual data is stored from offset+24 to end of file.
    fn build_data_block_index(source: &IoBackend, offset: u64, is_unfinished: bool, file_size: u64) -> Result<DataBlockIndex> {
        if offset == 0 {
            return Ok(DataBlockIndex::new());
        }

        let header = parser::parse_block_header(source, offset)?;
        let block_id = source.read_bytes(offset, 4)?;

        match &block_id[..] {
            b"##DT" | b"##SD" => {
                let mut index = DataBlockIndex::new();
                let mut data_size = header.length.saturating_sub(BLOCK_HEADER_SIZE as u64);
                
                // For unfinished files with empty DT block, data extends to end of file
                if is_unfinished && data_size == 0 {
                    let data_start = offset + BLOCK_HEADER_SIZE as u64;
                    data_size = file_size.saturating_sub(data_start);
                }
                
                let block_type = if &block_id[..] == b"##SD" {
                    DataBlockType::SortedData
                } else {
                    DataBlockType::Data
                };
                index.push(DataBlockInfo::uncompressed(offset, block_type, data_size));
                Ok(index)
            }
            b"##DZ" => {
                let dz_data = source.read_bytes(offset, header.length as usize)?;
                let dz = DzBlock::parse(&dz_data, offset)?;
                
                let mut index = DataBlockIndex::new();
                let compression = CompressionInfo {
                    algorithm: dz.zip_type,
                    parameter: dz.zip_parameter,
                    data_offset: dz.compressed_data_offset,
                };
                index.push(DataBlockInfo::compressed(
                    offset,
                    dz.original_size,
                    dz.compressed_size,
                    compression,
                ));
                Ok(index)
            }
            b"##DL" => {
                Self::build_data_list_index(source, offset)
            }
            b"##HL" => {
                let hl_data = source.read_bytes(offset, header.length as usize)?;
                let hl = HlBlock::parse(&hl_data, offset)?;
                if hl.dl_first != 0 {
                    Self::build_data_list_index(source, hl.dl_first)
                } else {
                    Ok(DataBlockIndex::new())
                }
            }
            _ => Ok(DataBlockIndex::new()),
        }
    }

    /// Builds index from a data list (DL) chain.
    fn build_data_list_index(source: &IoBackend, dl_offset: u64) -> Result<DataBlockIndex> {
        let mut index = DataBlockIndex::new();
        let mut current_dl = dl_offset;

        while current_dl != 0 {
            let header = parser::parse_block_header(source, current_dl)?;
            let dl_data = source.read_bytes(current_dl, header.length as usize)?;
            let dl = DlBlock::parse(&dl_data, current_dl)?;

            for &data_link in &dl.data_links {
                if data_link == 0 {
                    continue;
                }

                let block_header = parser::parse_block_header(source, data_link)?;
                let block_id = source.read_bytes(data_link, 4)?;

                match &block_id[..] {
                    b"##DT" | b"##SD" => {
                        let data_size = block_header.length.saturating_sub(BLOCK_HEADER_SIZE as u64);
                        let block_type = if &block_id[..] == b"##SD" {
                            DataBlockType::SortedData
                        } else {
                            DataBlockType::Data
                        };
                        index.push(DataBlockInfo::uncompressed(data_link, block_type, data_size));
                    }
                    b"##DZ" => {
                        let dz_data = source.read_bytes(data_link, block_header.length as usize)?;
                        let dz = DzBlock::parse(&dz_data, data_link)?;
                        let compression = CompressionInfo {
                            algorithm: dz.zip_type,
                            parameter: dz.zip_parameter,
                            data_offset: dz.compressed_data_offset,
                        };
                        index.push(DataBlockInfo::compressed(
                            data_link,
                            dz.original_size,
                            dz.compressed_size,
                            compression,
                        ));
                    }
                    _ => {}
                }
            }

            current_dl = dl.dl_next;
        }

        Ok(index)
    }

    /// Parses all channel groups for a data group.
    fn parse_channel_groups(
        source: &IoBackend,
        dg: &DgBlock,
        dg_index: usize,
        cache: &mut BlockCache,
        options: &OpenOptions,
    ) -> Result<Vec<ChannelGroup>> {
        let mut channel_groups = Vec::new();
        let mut cg_offset = dg.cg_first;
        let mut cg_index = 0;

        while cg_offset != 0 {
            let cg_block = parser::parse_cg_block(source, cg_offset)?;

            // Parse channels for this channel group
            let channels = Self::parse_channels(
                source,
                &cg_block,
                dg_index,
                cg_index,
                cache,
                options,
            )?;

            // Read acquisition name (cached)
            let acquisition_name = cache.get_or_parse_text(source, cg_block.tx_acq_name)?;

            // Read comment (cached)
            let comment = cache.get_or_parse_text(source, cg_block.md_comment)?;

            let channel_group = ChannelGroup {
                id: cg_index,
                index: cg_index,
                data_group_index: dg_index,
                acquisition_name: acquisition_name.to_string(),
                sample_count: cg_block.cycle_count,
                channels,
                source: None, // TODO: Parse SI block if present
                comment: comment.to_string(),
                record_id: cg_block.record_id,
                data_bytes: cg_block.data_bytes,
                inval_bytes: cg_block.inval_bytes,
                cg_offset,
                is_vlsd: cg_block.flags.vlsd,
            };

            channel_groups.push(channel_group);
            cg_offset = cg_block.cg_next;
            cg_index += 1;
        }

        Ok(channel_groups)
    }

    /// Parses all channels for a channel group.
    ///
    /// Uses parallel parsing via rayon if enabled and channel count exceeds threshold.
    /// Also expands composition channels (structures) into individual channels.
    fn parse_channels(
        source: &IoBackend,
        cg: &CgBlock,
        dg_index: usize,
        cg_index: usize,
        cache: &mut BlockCache,
        options: &OpenOptions,
    ) -> Result<Vec<Channel>> {
        // First, collect all channel offsets
        let mut cn_offsets = Vec::new();
        let mut cn_offset = cg.cn_first;
        while cn_offset != 0 {
            cn_offsets.push(cn_offset);
            // Read just the header to get next link
            let header = parser::parse_block_header(source, cn_offset)?;
            let block_data = source.read_bytes(cn_offset, header.length.min(80) as usize)?;
            // cn_next is at offset 24 (after header)
            if block_data.len() >= 32 {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&block_data[24..32]);
                cn_offset = u64::from_le_bytes(bytes);
            } else {
                break;
            }
        }

        let channel_count = cn_offsets.len();
        
        // Decide whether to use parallel parsing
        let use_parallel = options.parallel_parsing && channel_count >= options.parallel_threshold;

        let mut channels = Vec::with_capacity(channel_count * 2); // Extra capacity for composition

        if use_parallel {
            // Parallel parsing - note: cache is not used in parallel section
            // We parse blocks in parallel, then do text lookups sequentially
            let parsed_blocks: Vec<Result<(CnBlock, usize)>> = cn_offsets
                .par_iter()
                .enumerate()
                .map(|(idx, &offset)| {
                    let cn_block = parser::parse_cn_block(source, offset)?;
                    Ok((cn_block, idx))
                })
                .collect();

            // Process results sequentially (to use cache for text blocks)
            for result in parsed_blocks {
                let (cn_block, cn_index) = result?;
                let parent_name = cache.get_or_parse_text(source, cn_block.tx_name)?.to_string();
                let composition_offset = cn_block.composition;
                
                let channel = Self::build_channel(
                    source, cn_block, dg_index, cg_index, channels.len(), cache,
                )?;
                channels.push(channel);
                
                // Expand composition channels
                if composition_offset != 0 {
                    Self::expand_composition_channels(
                        source,
                        composition_offset,
                        &parent_name,
                        dg_index,
                        cg_index,
                        &mut channels,
                        cache,
                    )?;
                }
            }
        } else {
            // Sequential parsing
            for (_cn_index, &cn_offset) in cn_offsets.iter().enumerate() {
                let cn_block = parser::parse_cn_block(source, cn_offset)?;
                let parent_name = cache.get_or_parse_text(source, cn_block.tx_name)?.to_string();
                let composition_offset = cn_block.composition;
                
                let channel = Self::build_channel(
                    source, cn_block, dg_index, cg_index, channels.len(), cache,
                )?;
                channels.push(channel);
                
                // Expand composition channels
                if composition_offset != 0 {
                    Self::expand_composition_channels(
                        source,
                        composition_offset,
                        &parent_name,
                        dg_index,
                        cg_index,
                        &mut channels,
                        cache,
                    )?;
                }
            }
        }

        // Re-index channels
        for (idx, ch) in channels.iter_mut().enumerate() {
            ch.id = idx;
            ch.index = idx;
        }

        Ok(channels)
    }

    /// Expands composition channels (structure channels) into individual sub-channels.
    fn expand_composition_channels(
        source: &IoBackend,
        composition_offset: u64,
        parent_name: &str,
        dg_index: usize,
        cg_index: usize,
        channels: &mut Vec<Channel>,
        cache: &mut BlockCache,
    ) -> Result<()> {
        // Read the composition block header to determine its type
        let header = parser::parse_block_header(source, composition_offset)?;
        let block_id = source.read_bytes(composition_offset, 4)?;

        match &block_id[..] {
            b"##CN" => {
                // Structure composition - chain of CN blocks
                let mut cn_offset = composition_offset;
                while cn_offset != 0 {
                    let cn_block = parser::parse_cn_block(source, cn_offset)?;
                    let child_name = cache.get_or_parse_text(source, cn_block.tx_name)?;
                    let next_offset = cn_block.cn_next;
                    let nested_composition = cn_block.composition;
                    
                    // Create qualified name: parent.child
                    let qualified_name = format!("{}.{}", parent_name, child_name);
                    
                    let mut channel = Self::build_channel_with_name(
                        source,
                        cn_block,
                        dg_index,
                        cg_index,
                        channels.len(),
                        cache,
                        qualified_name,
                    )?;
                    
                    channels.push(channel);
                    
                    // Recursively expand nested compositions
                    if nested_composition != 0 {
                        let last_name = channels.last().map(|c| c.name.clone()).unwrap_or_default();
                        Self::expand_composition_channels(
                            source,
                            nested_composition,
                            &last_name,
                            dg_index,
                            cg_index,
                            channels,
                            cache,
                        )?;
                    }
                    
                    cn_offset = next_offset;
                }
            }
            b"##CA" => {
                // Array composition - we could expand array elements here
                // For now, skip array expansion (similar to how some tools handle it)
                // TODO: Implement array expansion if needed
            }
            _ => {
                // Unknown composition type, skip
            }
        }

        Ok(())
    }

    /// Builds a Channel with a custom name (for composition channels).
    fn build_channel_with_name(
        source: &IoBackend,
        cn_block: CnBlock,
        dg_index: usize,
        cg_index: usize,
        cn_index: usize,
        cache: &mut BlockCache,
        name: String,
    ) -> Result<Channel> {
        // Read unit (cached)
        let unit = cache.get_or_parse_text(source, cn_block.md_unit)?;

        // Read comment (cached)
        let comment = cache.get_or_parse_text(source, cn_block.md_comment)?;

        // Parse conversion if present (cached)
        let conversion = if let Some(cc_arc) = cache.get_or_parse_cc(source, cn_block.cc_conversion)? {
            Conversion::from_cc_block((*cc_arc).clone())
        } else {
            Conversion::None
        };

        // Extract value range if valid
        let (min_value, max_value) = if cn_block.flags.range_valid {
            (Some(cn_block.val_range_min), Some(cn_block.val_range_max))
        } else {
            (None, None)
        };

        Ok(Channel {
            id: cn_index,
            index: cn_index,
            channel_group_index: cg_index,
            data_group_index: dg_index,
            name,
            unit: unit.to_string(),
            channel_type: cn_block.channel_type,
            sync_type: cn_block.sync_type,
            data_type: cn_block.data_type,
            conversion,
            bit_count: cn_block.bit_count,
            byte_offset: cn_block.byte_offset,
            bit_offset: cn_block.bit_offset,
            comment: comment.to_string(),
            source: None,
            min_value,
            max_value,
            cn_offset: cn_block.header.offset,
        })
    }

    /// Builds a Channel from a parsed CN block, resolving all references.
    fn build_channel(
        source: &IoBackend,
        cn_block: CnBlock,
        dg_index: usize,
        cg_index: usize,
        cn_index: usize,
        cache: &mut BlockCache,
    ) -> Result<Channel> {
        // Read channel name (cached)
        let name = cache.get_or_parse_text(source, cn_block.tx_name)?;

        // Read unit (cached)
        let unit = cache.get_or_parse_text(source, cn_block.md_unit)?;

        // Read comment (cached)
        let comment = cache.get_or_parse_text(source, cn_block.md_comment)?;

        // Parse conversion if present (cached)
        let conversion = if let Some(cc_arc) = cache.get_or_parse_cc(source, cn_block.cc_conversion)? {
            Conversion::from_cc_block((*cc_arc).clone())
        } else {
            Conversion::None
        };

        // Extract value range if valid
        let (min_value, max_value) = if cn_block.flags.range_valid {
            (Some(cn_block.val_range_min), Some(cn_block.val_range_max))
        } else {
            (None, None)
        };

        Ok(Channel {
            id: cn_index,
            index: cn_index,
            channel_group_index: cg_index,
            data_group_index: dg_index,
            name: name.to_string(),
            unit: unit.to_string(),
            channel_type: cn_block.channel_type,
            sync_type: cn_block.sync_type,
            data_type: cn_block.data_type,
            conversion,
            bit_count: cn_block.bit_count,
            byte_offset: cn_block.byte_offset,
            bit_offset: cn_block.bit_offset,
            comment: comment.to_string(),
            source: None,
            min_value,
            max_value,
            cn_offset: cn_block.header.offset,
        })
    }

    /// Returns the MF4 format version.
    pub fn version(&self) -> Mf4Version {
        self.version
    }

    /// Returns the recording start time.
    pub fn start_time(&self) -> &RecordingTime {
        &self.start_time
    }

    /// Returns the file comment/description.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// Returns all data groups.
    pub fn data_groups(&self) -> &[DataGroup] {
        &self.data_groups
    }

    /// Returns the number of data groups.
    pub fn data_group_count(&self) -> usize {
        self.data_groups.len()
    }

    /// Returns the total number of channels across all groups.
    pub fn channel_count(&self) -> usize {
        self.channels_db.total_channel_count()
    }

    /// Returns an iterator over all channels in the file.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.data_groups
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .flat_map(|cg| cg.channels.iter())
    }

    /// Finds a channel by name using O(1) lookup.
    ///
    /// If multiple channels have the same name, returns the first one found.
    pub fn find_channel(&self, name: &str) -> Option<&Channel> {
        self.channels_db.find_first(name).map(|loc| {
            &self.data_groups[loc.data_group_index]
                .channel_groups[loc.channel_group_index]
                .channels[loc.channel_index]
        })
    }

    /// Finds all channels matching the given name.
    ///
    /// This is O(1) for the lookup, O(n) for collecting results.
    pub fn find_channels(&self, name: &str) -> Vec<&Channel> {
        self.channels_db
            .find_all(name)
            .iter()
            .map(|loc| {
                &self.data_groups[loc.data_group_index]
                    .channel_groups[loc.channel_group_index]
                    .channels[loc.channel_index]
            })
            .collect()
    }

    /// Returns true if a channel with the given name exists.
    pub fn has_channel(&self, name: &str) -> bool {
        self.channels_db.contains(name)
    }

    /// Returns an iterator over all unique channel names.
    pub fn channel_names(&self) -> impl Iterator<Item = &str> {
        self.channels_db.names()
    }

    /// Returns the master channel for a channel group, if any.
    pub fn master_channel(&self, data_group_index: usize, channel_group_index: usize) -> Option<&Channel> {
        self.masters_db
            .find(data_group_index, channel_group_index)
            .map(|ch_idx| {
                &self.data_groups[data_group_index]
                    .channel_groups[channel_group_index]
                    .channels[ch_idx]
            })
    }

    /// Returns file statistics.
    pub fn statistics(&self) -> FileStatistics {
        FileStatistics::from_data_groups(&self.data_groups, self.file_size)
    }

    /// Reads signal data for a channel.
    ///
    /// This method reads the raw data for the channel and returns
    /// a `Signal` object that provides access to decoded values.
    ///
    /// # Arguments
    /// * `channel` - The channel to read data for
    ///
    /// # Returns
    /// A `Signal` providing access to the channel's sample values.
    ///
    /// # Example
    /// ```ignore
    /// let channel = file.find_channel("Speed").unwrap();
    /// let signal = file.signal(channel)?;
    ///
    /// for value in signal.iter() {
    ///     println!("{}", value?);
    /// }
    /// ```
    pub fn signal(&self, channel: &Channel) -> Result<Signal> {
        // Find the data group and channel group
        let dg = &self.data_groups[channel.data_group_index];
        let cg = &dg.channel_groups[channel.channel_group_index];

        // Read raw data using the data block index
        let raw_data = self.read_raw_data_indexed(dg)?;

        // Calculate record parameters
        let record_size = cg.record_size(dg.rec_id_size);
        let record_offset = dg.rec_id_size as usize;

        // Calculate sample count
        let sample_count = if cg.sample_count > 0 {
            cg.sample_count as usize
        } else if record_size > 0 {
            raw_data.len() / record_size
        } else {
            0
        };

        Ok(Signal::new(
            channel.clone(),
            raw_data,
            record_size,
            record_offset,
            sample_count,
        ))
    }

    /// Reads raw data using the data block index.
    fn read_raw_data_indexed(&self, dg: &DataGroup) -> Result<Vec<u8>> {
        if dg.data_block_index.is_empty() {
            return Ok(Vec::new());
        }

        let total_size = dg.data_block_index.total_size() as usize;
        let mut all_data = Vec::with_capacity(total_size);

        for (_offset, block_info) in dg.data_block_index.iter() {
            let block_data = self.read_data_block(block_info)?;
            all_data.extend(block_data);
        }

        Ok(all_data)
    }

    /// Reads and decompresses a single data block.
    fn read_data_block(&self, info: &DataBlockInfo) -> Result<Vec<u8>> {
        if let Some(compression) = &info.compression {
            // Compressed block
            let compressed_data = self.source.read_bytes(
                compression.data_offset,
                info.compressed_size as usize,
            )?;
            
            Self::decompress(&compressed_data, compression, info.original_size as usize)
        } else {
            // Uncompressed block - read from after the header
            let data_offset = info.offset + BLOCK_HEADER_SIZE as u64;
            let data = self.source.read_bytes(data_offset, info.original_size as usize)?;
            Ok(data.into_owned())
        }
    }

    /// Decompresses data according to compression info.
    fn decompress(
        compressed: &[u8],
        compression: &CompressionInfo,
        original_size: usize,
    ) -> Result<Vec<u8>> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;

        match compression.algorithm {
            CompressionType::Deflate => {
                let mut decoder = ZlibDecoder::new(compressed);
                let mut decompressed = Vec::with_capacity(original_size);
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|e| Mf4Error::Decompression(e.to_string()))?;
                Ok(decompressed)
            }
            CompressionType::TransposedDeflate => {
                // First decompress
                let mut decoder = ZlibDecoder::new(compressed);
                let mut transposed = Vec::with_capacity(original_size);
                decoder
                    .read_to_end(&mut transposed)
                    .map_err(|e| Mf4Error::Decompression(e.to_string()))?;

                // Then un-transpose
                let column_size = compression.parameter as usize;
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

    /// Returns the file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Returns channel database statistics (for debugging/profiling).
    pub fn channels_db_stats(&self) -> (usize, usize) {
        (self.channels_db.unique_name_count(), self.channels_db.total_channel_count())
    }
}

impl std::fmt::Debug for Mf4File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mf4File")
            .field("version", &self.version)
            .field("data_group_count", &self.data_groups.len())
            .field("channel_count", &self.channel_count())
            .field("unique_channel_names", &self.channels_db.unique_name_count())
            .field("file_size", &self.file_size)
            .finish()
    }
}
