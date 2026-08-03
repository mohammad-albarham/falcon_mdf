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
use std::sync::{Arc, RwLock};

use rayon::prelude::*;

use crate::blocks::{
    CaStorage, CcBlock, CgBlock, ChannelType, CnBlock, CompressionType, Conversion, DgBlock,
    DlBlock, DzBlock, HdBlock, HlBlock, ParseBlock, TableEntry, UnfinalizedFlags,
    BLOCK_HEADER_SIZE,
};
use crate::cache::BlockCache;
use crate::channels_db::{ChannelLocation, ChannelsDB, MastersDB};
use crate::data_index::{
    CompressionInfo, DataBlockIndex, DataBlockInfo, DataBlockType, RecordIndex,
};
use crate::error::{Mf4Error, Result};
use crate::io::{ByteSource, IoBackend};
use crate::model::{
    ArrayElement, Attachment, Channel, ChannelGroup, ChannelHierarchyNode, DataGroup, Event,
    FileHistoryEntry, FileStatistics, Metadata, MlsdLength, RecordLayout, RecordingTime,
    ReductionKind, SampleReduction, Signal, UnreadableReason, VlsdPayloads,
};
use crate::parser::links::{LinkChain, MAX_COMPOSITION_DEPTH};
use crate::parser::{self, parse_hd_block, parse_id_block, Mf4Version};

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

    /// Ceiling on a single allocation sized from a number the file declares.
    ///
    /// Pre-allocating is only ever an optimisation — a buffer grows on demand —
    /// so clamping the hint costs a genuinely large file a few reallocations
    /// and costs a corrupt one nothing. Without a ceiling a mutated size field
    /// becomes an allocation of that size, which aborts the process rather than
    /// returning an error.
    ///
    /// Raising it trades that protection for fewer reallocations when reading
    /// files you trust; lowering it is only useful under a tight memory budget.
    pub max_alloc: usize,

    /// Ceiling on the expanded size of one compressed block.
    ///
    /// A compressed block declares how far it expands, but that figure is
    /// untrusted: a small block can claim — or genuinely produce — an enormous
    /// amount of data. Reads stop at this limit instead of exhausting memory.
    ///
    /// Raise it if you legitimately read blocks that expand beyond it; a read
    /// that hits the limit returns truncated data rather than failing, so a
    /// value below what your files need will quietly lose the tail.
    pub max_decompressed: u64,
}

/// The allocation ceilings in force for one file, carried to the parsing
/// helpers that would otherwise size buffers from untrusted numbers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// See [`OpenOptions::max_alloc`].
    pub max_alloc: usize,
    /// See [`OpenOptions::max_decompressed`].
    pub max_decompressed: u64,
}

impl From<&OpenOptions> for Limits {
    fn from(options: &OpenOptions) -> Self {
        Limits {
            max_alloc: options.max_alloc,
            max_decompressed: options.max_decompressed,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_alloc: DEFAULT_MAX_ALLOC,
            max_decompressed: DEFAULT_MAX_DECOMPRESSED,
        }
    }
}

/// Default ceiling on a speculative allocation, in bytes.
pub const DEFAULT_MAX_ALLOC: usize = 64 * 1024 * 1024;

/// Default ceiling on one block's expanded size, in bytes.
pub const DEFAULT_MAX_DECOMPRESSED: u64 = 1024 * 1024 * 1024;

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            build_channels_db: true,
            parallel_parsing: true,
            parallel_threshold: 100,
            max_alloc: DEFAULT_MAX_ALLOC,
            max_decompressed: DEFAULT_MAX_DECOMPRESSED,
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
    source: Arc<IoBackend>,

    /// MF4 format version.
    version: Mf4Version,

    /// What the writer left undone, for a file that was never finalized.
    unfinalized: Option<UnfinalizedFlags>,

    /// Recording start time.
    start_time: RecordingTime,

    /// File comment/description.
    comment: Arc<str>,

    /// Parsed contents of the header's metadata block.
    metadata: Metadata,

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

    /// Most recently assembled record buffer, shared by every `Signal` built
    /// from the same channel group.
    record_cache: RwLock<Option<CachedRecords>>,

    /// Allocation ceilings this file was opened with.
    limits: Limits,

    /// Most recently built payload index for a variable-length channel.
    ///
    /// Building one walks the whole payload stream, so without this every
    /// variable-length channel in a group would rebuild the same index.
    payload_cache: RwLock<Option<(u64, Arc<VlsdPayloads>)>>,

    /// File-level attachments, parsed from the HD block's AT chain.
    attachments: Vec<Attachment>,

    /// File-level events, parsed from the HD block's EV chain.
    events: Vec<Event>,

    /// Channel hierarchy nodes, parsed from the HD block's CH chain.
    hierarchy: Vec<ChannelHierarchyNode>,

    /// The file's change history, oldest entry first.
    file_history: Vec<FileHistoryEntry>,
}

/// A channel group's records, ready for a `Signal` to index.
#[derive(Clone)]
struct CachedRecords {
    /// Which channel group these records belong to.
    key: (usize, usize),
    /// The records themselves, shared rather than copied per channel.
    ///
    /// `Arc<Vec<u8>>` rather than `Arc<[u8]>`: converting a `Vec` into the
    /// latter copies every byte, because the reference count has to sit beside
    /// the data. Wrapping the `Vec` moves it instead, which for a large group
    /// is the difference between one allocation and two.
    data: Arc<Vec<u8>>,
    /// How to address a record within `data`.
    layout: RecordLayout,
    /// Number of records present.
    sample_count: usize,
}

impl Mf4File {
    /// Opens an MF4 file for reading with default options.
    ///
    /// This method opens the file, validates the format, and parses
    /// the structure (data groups, channel groups, channels). The actual
    /// sample data is read lazily when requested.
    ///
    /// # Choosing a backend
    ///
    /// With the default `mmap` feature this memory-maps the file, which is the
    /// fastest option and correct for a measurement file that is finished being
    /// written. It also means the file must not change while it is open — see
    /// [`crate::io::mmap::MmapSource::open`] for what happens if it does.
    ///
    /// For a file that another process may still be writing, replacing, or
    /// serving over a network share, use [`Mf4File::open_buffered`], which
    /// copies what it reads and carries no such requirement.
    ///
    /// # Memory on large files
    ///
    /// A data group written as a chain of blocks — which is how writers store
    /// anything large — has to be assembled into one buffer before its records
    /// can be read. Under the memory-mapped backend the data is then resident
    /// twice: once as mapped pages, once as that buffer. Measured on a 416 MB
    /// file, peak resident memory was 826 MB mapped against 434 MB buffered.
    ///
    /// For large files [`Mf4File::open_buffered`] therefore uses roughly half
    /// the memory, at the cost of the faster mapped reads.
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
        let source = Arc::new(source);
        let file_size = source.len();

        // Parse ID block
        let id_block = parse_id_block(&source)?;
        let version = Mf4Version::from_id_block(&id_block);
        let is_unfinished = id_block.is_unfinished();
        let unfinalized = id_block.unfinalized();

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

        // The header's metadata block usually carries device and tool details
        // alongside the comment; keep them rather than discarding the XML.
        let metadata = parser::read_metadata(&source, hd_block.md_comment)?.unwrap_or_default();

        // Parse data groups with caching
        let data_groups = Self::parse_data_groups(
            &source,
            &hd_block,
            &mut cache,
            &options,
            is_unfinished,
            file_size,
        )?;

        // Parse file-level attachments, events, and channel hierarchy.
        let attachments = Self::parse_attachments(&source, hd_block.at_first, &mut cache)?;
        let events = Self::parse_events(&source, hd_block.ev_first, &mut cache)?;
        let hierarchy = Self::parse_hierarchy(&source, hd_block.ch_first, &mut cache)?;
        let file_history = Self::parse_file_history(&source, hd_block.fh_first, &mut cache)?;

        // Build channel lookup indices
        let (channels_db, masters_db) = if options.build_channels_db {
            Self::build_channel_indices(&data_groups)
        } else {
            (ChannelsDB::new(), MastersDB::new())
        };

        Ok(Mf4File {
            source,
            version,
            unfinalized,
            start_time,
            comment,
            data_groups,
            channels_db,
            masters_db,
            file_size,
            cache,
            metadata,
            limits: Limits::from(&options),
            record_cache: RwLock::new(None),
            payload_cache: RwLock::new(None),
            attachments,
            events,
            hierarchy,
            file_history,
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
                    channels_db.insert(&ch.name, ChannelLocation::new(dg_idx, cg_idx, ch_idx));

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
        let mut chain = LinkChain::new();

        while dg_offset != 0 {
            chain.visit(dg_offset, "dg_next")?;
            let dg_block = parser::parse_dg_block(source, dg_offset)?;

            // Parse channel groups for this data group
            let channel_groups =
                Self::parse_channel_groups(source, &dg_block, dg_index, cache, options)?;

            // Read data group comment (cached)
            let comment = cache.get_or_parse_text(source, dg_block.md_comment)?;

            // Build data block index for lazy loading
            let data_block_index = if dg_block.data != 0 {
                Self::build_data_block_index(source, dg_block.data, is_unfinished, file_size)?
            } else {
                DataBlockIndex::new()
            };

            // Resolve sample counts, and for unsorted data groups also record
            // where every channel group's records live in the byte stream.
            let mut channel_groups = channel_groups;
            let record_index = Self::index_records(
                source,
                &dg_block,
                &data_block_index,
                &mut channel_groups,
                Limits::from(options),
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
                record_index,
            };

            data_groups.push(data_group);
            dg_offset = dg_block.dg_next;
            dg_index += 1;
        }

        Ok(data_groups)
    }

    /// Resolves sample counts and, for unsorted data groups, indexes records.
    ///
    /// A data group is *unsorted* when it holds more than one channel group and
    /// each record carries a record ID. Such records are interleaved and vary in
    /// size between channel groups, so the stream must be walked once to find
    /// where each record begins; the resulting [`RecordIndex`] is what lets
    /// [`Mf4File::signal`] gather a single channel group's records later.
    ///
    /// Sorted data groups need no walk: their records are a fixed stride, so the
    /// sample count follows from the data size and `None` is returned.
    fn index_records(
        source: &IoBackend,
        dg: &DgBlock,
        data_index: &DataBlockIndex,
        channel_groups: &mut [ChannelGroup],
        limits: Limits,
    ) -> Result<Option<RecordIndex>> {
        let rec_id_size = dg.rec_id_size;
        let unsorted = rec_id_size > 0 && channel_groups.len() > 1;

        if data_index.is_empty() {
            return Ok(unsorted.then(|| RecordIndex::with_groups(channel_groups.len())));
        }

        if !unsorted {
            // Fixed stride: the sample count follows from the data size.
            let data_size = data_index.total_size();
            for cg in channel_groups.iter_mut() {
                let record_size = cg.record_size(rec_id_size);
                if record_size == 0 {
                    cg.sample_count = 0;
                    continue;
                }

                // How many records the data can actually hold. A declared cycle
                // count above this is corrupt, and it must not be believed: the
                // count sizes the buffers every read allocates, so a wild value
                // turns into a wild allocation. The data is the authority.
                let capacity = data_size / record_size as u64;
                if cg.sample_count == 0 || cg.sample_count > capacity {
                    cg.sample_count = capacity;
                }
            }
            return Ok(None);
        }

        // record_id -> (channel group index, record size, is_vlsd)
        let mut record_map: std::collections::HashMap<u64, (usize, usize, bool)> =
            std::collections::HashMap::with_capacity(channel_groups.len());
        for (idx, cg) in channel_groups.iter().enumerate() {
            record_map.insert(cg.record_id, (idx, cg.record_size(rec_id_size), cg.is_vlsd));
        }

        let mut index = RecordIndex::with_groups(channel_groups.len());

        // Offset of the current block within the data group's concatenated
        // payload, so recorded positions address the stream `signal` will read.
        let mut base: u64 = 0;

        for (_offset, block_info) in data_index.iter() {
            let block_data = Self::read_block_payload(source, block_info, limits)?;
            let data: &[u8] = &block_data;
            let mut pos: usize = 0;

            while pos < data.len() {
                let Some(rec_id) = read_record_id(data, pos, rec_id_size) else {
                    break;
                };

                // An unrecognised record ID means the stream is no longer
                // parseable: record sizes are keyed off the ID, so there is no
                // way to know how far to skip. Stop rather than emit garbage.
                let Some(&(cg_idx, record_size, is_vlsd)) = record_map.get(&rec_id) else {
                    break;
                };

                let next = if is_vlsd {
                    // VLSD record: rec_id, then a 4-byte LE length, then payload.
                    let len_at = pos + rec_id_size as usize;
                    if len_at + 4 > data.len() {
                        break;
                    }
                    let vlsd_len = u32::from_le_bytes([
                        data[len_at],
                        data[len_at + 1],
                        data[len_at + 2],
                        data[len_at + 3],
                    ]) as usize;
                    len_at + 4 + vlsd_len
                } else {
                    if record_size == 0 || pos + record_size > data.len() {
                        break;
                    }
                    pos + record_size
                };

                index.push(cg_idx, base + pos as u64);
                pos = next;
            }

            base += block_info.original_size;
        }

        // Counts from the walk are authoritative: a declared cycle_count can be
        // stale (unfinished files) or disagree with what the stream contains.
        for (idx, cg) in channel_groups.iter_mut().enumerate() {
            cg.sample_count = index.count(idx) as u64;
        }

        Ok(Some(index))
    }

    /// Reads one data block, decompressing it if needed.
    fn read_block_payload(
        source: &IoBackend,
        info: &DataBlockInfo,
        limits: Limits,
    ) -> Result<Vec<u8>> {
        if let Some(compression) = &info.compression {
            let compressed =
                source.read_bytes(compression.data_offset, info.compressed_size as usize)?;
            Self::decompress(
                &compressed,
                compression,
                info.original_size as usize,
                limits,
            )
        } else {
            let at = info.offset + BLOCK_HEADER_SIZE as u64;
            Ok(source
                .read_bytes(at, info.original_size as usize)?
                .into_owned())
        }
    }

    /// Builds a data block index for lazy data access.
    ///
    /// For unfinished files, the DT block header may have length=24 (just header)
    /// while the actual data is stored from offset+24 to end of file.
    fn build_data_block_index(
        source: &IoBackend,
        offset: u64,
        is_unfinished: bool,
        file_size: u64,
    ) -> Result<DataBlockIndex> {
        if offset == 0 {
            return Ok(DataBlockIndex::new());
        }

        let header = parser::parse_block_header(source, offset)?;
        let block_id = source.read_bytes(offset, 4)?;

        match &block_id[..] {
            b"##DT" | b"##SD" | b"##RD" => {
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
            b"##DL" => Self::build_data_list_index(source, offset),
            b"##HL" => {
                let hl_data = source.read_bytes(offset, header.length as usize)?;
                let hl = HlBlock::parse(&hl_data, offset)?;
                if hl.dl_first != 0 {
                    Self::build_data_list_index(source, hl.dl_first)
                } else {
                    Ok(DataBlockIndex::new())
                }
            }
            // An empty index here would mean "this group has no samples",
            // which is a plausible-looking answer to a question this build
            // cannot answer — the block may hold every sample in the file. A
            // 4.2 file's `##LD` reaches exactly this arm.
            other => Err(Mf4Error::unsupported(
                "data block",
                format!(
                    "block '{}' at offset {offset} is not a data block this build can read",
                    String::from_utf8_lossy(other)
                ),
            )),
        }
    }

    /// Builds index from a data list (DL) chain.
    fn build_data_list_index(source: &IoBackend, dl_offset: u64) -> Result<DataBlockIndex> {
        let mut index = DataBlockIndex::new();
        let mut current_dl = dl_offset;
        let mut chain = LinkChain::new();

        while current_dl != 0 {
            chain.visit(current_dl, "dl_next")?;
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
                    b"##DT" | b"##SD" | b"##RD" => {
                        let data_size =
                            block_header.length.saturating_sub(BLOCK_HEADER_SIZE as u64);
                        let block_type = if &block_id[..] == b"##SD" {
                            DataBlockType::SortedData
                        } else {
                            DataBlockType::Data
                        };
                        index.push(DataBlockInfo::uncompressed(
                            data_link, block_type, data_size,
                        ));
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
                    // Worse here than at the top level: a data list holds one
                    // block per segment of a group's records, so skipping one
                    // drops a slice out of the middle of the stream and leaves
                    // the segments after it shifted.
                    other => {
                        return Err(Mf4Error::unsupported(
                            "data block",
                            format!(
                                "data list at offset {current_dl} refers to block '{}' at \
                                 offset {data_link}, which this build cannot read",
                                String::from_utf8_lossy(other)
                            ),
                        ))
                    }
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
        let mut chain = LinkChain::new();

        while cg_offset != 0 {
            chain.visit(cg_offset, "cg_next")?;
            let cg_block = parser::parse_cg_block(source, cg_offset)?;

            // Parse channels for this channel group
            let channels =
                Self::parse_channels(source, &cg_block, dg_index, cg_index, cache, options)?;

            // Read acquisition name (cached)
            let acquisition_name = cache.get_or_parse_text(source, cg_block.tx_acq_name)?;

            // Read comment (cached)
            let comment = cache.get_or_parse_text(source, cg_block.md_comment)?;

            let sample_reductions = Self::parse_sample_reductions(source, cg_block.sr_first)?;

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
                sample_reductions,
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
        let mut chain = LinkChain::new();
        while cn_offset != 0 {
            chain.visit(cn_offset, "cn_next")?;
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

        let limits = Limits::from(options);
        let mut channels =
            Vec::with_capacity(channel_count.saturating_mul(2).min(limits.max_alloc));

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
                let (cn_block, _cn_index) = result?;
                let parent_name = cache
                    .get_or_parse_text(source, cn_block.tx_name)?
                    .to_string();
                let composition_offset = cn_block.composition;

                let channel = Self::build_channel(
                    source,
                    cn_block,
                    dg_index,
                    cg_index,
                    channels.len(),
                    cache,
                )?;
                channels.push(channel);

                // Expand composition channels
                if composition_offset != 0 {
                    let parent = channels.len() - 1;
                    let outcome = Self::expand_composition_channels(
                        source,
                        composition_offset,
                        &parent_name,
                        dg_index,
                        cg_index,
                        &mut channels,
                        cache,
                        0,
                    )?;
                    if outcome == CompositionOutcome::UnsupportedArray {
                        channels[parent].unreadable = Some(UnreadableReason::ArrayComposition);
                    }
                }
            }
        } else {
            // Sequential parsing
            for &cn_offset in cn_offsets.iter() {
                let cn_block = parser::parse_cn_block(source, cn_offset)?;
                let parent_name = cache
                    .get_or_parse_text(source, cn_block.tx_name)?
                    .to_string();
                let composition_offset = cn_block.composition;

                let channel = Self::build_channel(
                    source,
                    cn_block,
                    dg_index,
                    cg_index,
                    channels.len(),
                    cache,
                )?;
                channels.push(channel);

                // Expand composition channels
                if composition_offset != 0 {
                    let parent = channels.len() - 1;
                    let outcome = Self::expand_composition_channels(
                        source,
                        composition_offset,
                        &parent_name,
                        dg_index,
                        cg_index,
                        &mut channels,
                        cache,
                        0,
                    )?;
                    if outcome == CompositionOutcome::UnsupportedArray {
                        channels[parent].unreadable = Some(UnreadableReason::ArrayComposition);
                    }
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
    #[allow(clippy::too_many_arguments)]
    fn expand_composition_channels(
        source: &IoBackend,
        composition_offset: u64,
        parent_name: &str,
        dg_index: usize,
        cg_index: usize,
        channels: &mut Vec<Channel>,
        cache: &mut BlockCache,
        depth: usize,
    ) -> Result<CompositionOutcome> {
        // Compositions nest legitimately, but only a few levels deep. A
        // composition that references an ancestor would otherwise recurse until
        // the stack overflows, which no per-chain visited set can catch because
        // each nesting level walks a fresh chain.
        if depth >= MAX_COMPOSITION_DEPTH {
            return Err(Mf4Error::CyclicLink {
                chain: format!("cn_composition (depth limit {MAX_COMPOSITION_DEPTH})"),
                offset: composition_offset,
            });
        }

        // Read the composition block header to determine its type
        let _header = parser::parse_block_header(source, composition_offset)?;
        let block_id = source.read_bytes(composition_offset, 4)?;

        match &block_id[..] {
            b"##CN" => {
                // Structure composition - chain of CN blocks
                let mut cn_offset = composition_offset;
                let mut chain = LinkChain::new();
                while cn_offset != 0 {
                    chain.visit(cn_offset, "cn_next (composition)")?;
                    let cn_block = parser::parse_cn_block(source, cn_offset)?;
                    let child_name = cache.get_or_parse_text(source, cn_block.tx_name)?;
                    let next_offset = cn_block.cn_next;
                    let nested_composition = cn_block.composition;

                    let qualified_name = qualify_channel_name(parent_name, &child_name);

                    let channel = Self::build_channel_with_name(
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
                        let _ = Self::expand_composition_channels(
                            source,
                            nested_composition,
                            &last_name,
                            dg_index,
                            cg_index,
                            channels,
                            cache,
                            depth + 1,
                        )?;
                    }

                    cn_offset = next_offset;
                }
            }
            b"##CA" => {
                // An array composition describes a channel holding many values
                // per sample. Parse the CA block to learn the array's shape and
                // element layout, then make the parent channel readable by
                // attaching that information.
                let ca_block = parser::parse_ca_block(source, composition_offset)?;

                // Only CN-template storage is decoded: all elements of one
                // sample's array are adjacent in the record, so element j is at
                // parent.byte_offset + j * element_size. The CG- and DG-template
                // forms spread one sample's elements across separate record
                // streams, which nothing here gathers, so they stay unreadable
                // rather than being decoded as though they were adjacent.
                if ca_block.ca_storage != CaStorage::CnTemplate {
                    return Ok(CompositionOutcome::UnsupportedArray);
                }

                // A dynamic-size array's `ca_dim_size` is the largest shape a
                // sample may take, not the shape any sample has; the real sizes
                // come from channels the CA block names. Decoding it as fixed
                // would hand back the unused tail of the field as though it
                // were data. This became visible only once the flags were read
                // at their standard bit positions — the old numbering read this
                // bit as "has axis", so nothing here could have known.
                if ca_block.flags.dynamic_size {
                    return Ok(CompositionOutcome::UnsupportedArray);
                }

                // A CA block need not name a template at all. When it does
                // not, the *parent* channel describes the element — its data
                // type and bit count are the element's — and
                // `ca_byte_offset_base` gives the stride. Refusing these as
                // "nothing says how wide an element is" was wrong on both
                // counts, and it is the form Vector and dSPACE actually emit
                // for look-up tables and matrices.
                let stride = if ca_block.ca_byte_offset_base > 0 {
                    ca_block.ca_byte_offset_base as usize
                } else {
                    0 // fall back to the element's own width, below
                };

                if ca_block.ca_composition == 0 {
                    if let Some(parent) = channels.last_mut() {
                        let width = (parent.bit_count as usize).div_ceil(8).max(1);
                        parent.array_shape = Some(ca_block.ca_dim_size.clone());
                        parent.array_element = Some(ArrayElement {
                            data_type: parent.data_type,
                            bit_count: parent.bit_count,
                            bit_offset: parent.bit_offset,
                            byte_offset: 0,
                            inverse_layout: ca_block.flags.inverse_layout,
                            stride: if stride > 0 { stride } else { width },
                        });
                        parent.unreadable = None;
                    }
                    return Ok(CompositionOutcome::Expanded);
                }

                // `ca_composition` normally names a CN describing one element,
                // but a look-up array composes with another CA — an array whose
                // elements are themselves arrays. Nothing here flattens that,
                // and the link carries no type, so the block has to be
                // identified before it is parsed: reading a CA as a CN failed
                // the whole *file*, where one unreadable channel is the honest
                // cost. `Vector_MeasurementArrays.mf4` is built entirely of
                // these.
                let id = source.read_bytes(ca_block.ca_composition, 4)?;
                if &id[..] != b"##CN" {
                    return Ok(CompositionOutcome::UnsupportedArray);
                }

                let template_cn = parser::parse_cn_block(source, ca_block.ca_composition)?;

                // Set the array shape and element layout on the parent channel.
                // The parent is the last channel pushed before this call.
                if let Some(parent) = channels.last_mut() {
                    parent.array_shape = Some(ca_block.ca_dim_size.clone());
                    let width = (template_cn.bit_count as usize).div_ceil(8).max(1);
                    parent.array_element = Some(ArrayElement {
                        data_type: template_cn.data_type,
                        bit_count: template_cn.bit_count,
                        bit_offset: template_cn.bit_offset,
                        byte_offset: template_cn.byte_offset,
                        inverse_layout: ca_block.flags.inverse_layout,
                        stride: if stride > 0 { stride } else { width },
                    });
                    // The channel is now readable; clear any unreadable status.
                    parent.unreadable = None;
                }

                return Ok(CompositionOutcome::Expanded);
            }
            _ => {
                // Unknown composition type, skip
            }
        }

        Ok(CompositionOutcome::Expanded)
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
        let conversion =
            if let Some(cc_arc) = cache.get_or_parse_cc(source, cn_block.cc_conversion)? {
                build_conversion(source, &cc_arc, cache)?
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
            all_invalid: cn_block.flags.all_invalid,
            invalidation_bit: cn_block.flags.invalidation_bit,
            inval_bit_pos: cn_block.inval_bit_pos,
            comment: comment.to_string(),
            source: None,
            min_value,
            max_value,
            cn_offset: cn_block.header.offset,
            data_link: cn_block.data,
            unreadable: None,
            array_shape: None,
            array_element: None,
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
        let conversion =
            if let Some(cc_arc) = cache.get_or_parse_cc(source, cn_block.cc_conversion)? {
                build_conversion(source, &cc_arc, cache)?
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
            all_invalid: cn_block.flags.all_invalid,
            invalidation_bit: cn_block.flags.invalidation_bit,
            inval_bit_pos: cn_block.inval_bit_pos,
            comment: comment.to_string(),
            source: None,
            min_value,
            max_value,
            cn_offset: cn_block.header.offset,
            data_link: cn_block.data,
            unreadable: None,
            array_shape: None,
            array_element: None,
        })
    }

    /// Returns the MF4 format version.
    pub fn version(&self) -> Mf4Version {
        self.version
    }

    /// Returns what the writer left undone, or `None` for a finalized file.
    ///
    /// A file that was never finalized still carries its data; what it lacks is
    /// bookkeeping. Two of the seven flags are compensated for automatically —
    /// see [`UnfinalizedFlags`] for which, and for what the others mean for the
    /// values this reader hands back.
    pub fn unfinalized(&self) -> Option<UnfinalizedFlags> {
        self.unfinalized
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
        // Counted from the groups themselves, not the name index. The index is
        // an accelerator that callers can switch off; the file's contents do not
        // change when they do.
        self.data_groups
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .map(|cg| cg.channels.len())
            .sum()
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
        if let Some(loc) = self.channels_db.find_first(name) {
            return Some(
                &self.data_groups[loc.data_group_index].channel_groups[loc.channel_group_index]
                    .channels[loc.channel_index],
            );
        }
        // Without the index, search directly. `build_channels_db` is meant to
        // trade lookup speed for memory, not to change which channels can be
        // found — a lookup that silently fails would make it a semantic switch.
        if self.channels_db.is_empty() {
            return self.channels().find(|ch| ch.name == name);
        }
        None
    }

    /// Finds all channels matching the given name.
    ///
    /// This is O(1) for the lookup, O(n) for collecting results.
    pub fn find_channels(&self, name: &str) -> Vec<&Channel> {
        if self.channels_db.is_empty() {
            // See `find_channel`: without the index, search directly.
            return self.channels().filter(|ch| ch.name == name).collect();
        }
        self.channels_db
            .find_all(name)
            .iter()
            .map(|loc| {
                &self.data_groups[loc.data_group_index].channel_groups[loc.channel_group_index]
                    .channels[loc.channel_index]
            })
            .collect()
    }

    /// Returns true if a channel with the given name exists.
    pub fn has_channel(&self, name: &str) -> bool {
        if self.channels_db.is_empty() {
            return self.channels().any(|ch| ch.name == name);
        }
        self.channels_db.contains(name)
    }

    /// Returns an iterator over all unique channel names.
    /// Names are unique: a name carried by several channels appears once.
    pub fn channel_names(&self) -> Vec<&str> {
        if self.channels_db.is_empty() {
            let mut names: Vec<&str> = self.channels().map(|ch| ch.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            return names;
        }
        self.channels_db.names().collect()
    }

    /// Returns the master channel for a channel group, if any.
    pub fn master_channel(
        &self,
        data_group_index: usize,
        channel_group_index: usize,
    ) -> Option<&Channel> {
        self.masters_db
            .find(data_group_index, channel_group_index)
            .map(|ch_idx| {
                &self.data_groups[data_group_index].channel_groups[channel_group_index].channels
                    [ch_idx]
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
        let key = (channel.data_group_index, channel.channel_group_index);
        let records = self.records_for(key)?;
        let mut signal = Signal::new(
            channel.clone(),
            records.data.clone(),
            records.layout,
            records.sample_count,
        );

        if channel.channel_type == ChannelType::VariableLength {
            signal.attach_payloads(self.payloads_for(channel)?);
        }

        if channel.channel_type == ChannelType::MaxLength {
            if let Some(length) = self.mlsd_length_for(channel)? {
                signal.attach_mlsd_length(length);
            }
        }

        Ok(signal)
    }

    /// Resolves the channel that holds a maximum-length channel's sample sizes.
    ///
    /// `cn_data` on an MLSD channel points at a CN block rather than a signal
    /// data block — the channel in the same group whose value counts the bytes
    /// each sample actually uses. Returns `None` when the link is absent, which
    /// leaves the channel undecodable and is reported as such when it is read.
    fn mlsd_length_for(&self, channel: &Channel) -> Result<Option<MlsdLength>> {
        if channel.data_link == 0 {
            return Ok(None);
        }

        let cn = parser::parse_cn_block(&self.source, channel.data_link)?;
        Ok(Some(MlsdLength {
            byte_offset: cn.byte_offset,
            bit_offset: cn.bit_offset,
            bit_count: cn.bit_count,
            little_endian: cn.data_type.is_little_endian(),
        }))
    }

    /// Returns the assembled record buffer for a channel group.
    ///
    /// Assembling it means reading and, for a compressed group, decompressing
    /// the whole data group. Callers almost always read several channels from
    /// the same group in succession, so the last result is kept: without it,
    /// reading N channels does that work N times over.
    ///
    /// Only the most recent group is retained. That covers sequential access —
    /// the normal pattern — while bounding memory to one group's records rather
    /// than the whole file.
    fn records_for(&self, key: (usize, usize)) -> Result<CachedRecords> {
        if let Ok(guard) = self.record_cache.read() {
            if let Some(hit) = guard.as_ref().filter(|c| c.key == key) {
                return Ok(hit.clone());
            }
        }

        let built = self.build_records(key)?;

        if let Ok(mut guard) = self.record_cache.write() {
            *guard = Some(built.clone());
        }
        Ok(built)
    }

    /// Reads and assembles one channel group's records from the file.
    fn build_records(&self, key: (usize, usize)) -> Result<CachedRecords> {
        let (dg_index, cg_index) = key;
        let dg = &self.data_groups[dg_index];
        let cg = &dg.channel_groups[cg_index];

        let raw_data = self.read_raw_data_indexed(dg)?;

        // Unsorted data group: this channel group's records are scattered
        // through a stream shared with the other groups, so collect them into a
        // contiguous buffer keyed by the index built when the file was opened.
        if let Some(index) = &dg.record_index {
            let payload = cg.payload_size();
            let (records, sample_count) = Self::gather_records(
                &raw_data,
                index.offsets(cg_index),
                dg.rec_id_size as usize,
                payload,
                self.limits,
            );
            return Ok(CachedRecords {
                key,
                data: Arc::new(records),
                layout: RecordLayout {
                    record_size: payload,
                    record_offset: 0,
                    inval_start: cg.data_bytes_len(),
                    inval_bytes: cg.inval_bytes_len(),
                },
                sample_count,
            });
        }

        // Sorted data group: records are a fixed stride in file order.
        let record_size = cg.record_size(dg.rec_id_size);
        let record_offset = dg.rec_id_size as usize;

        let sample_count = if cg.sample_count > 0 {
            cg.sample_count as usize
        } else if record_size > 0 {
            raw_data.len() / record_size
        } else {
            0
        };

        Ok(CachedRecords {
            key,
            data: Arc::new(raw_data),
            layout: RecordLayout {
                record_size,
                record_offset,
                inval_start: cg.data_bytes_len(),
                inval_bytes: cg.inval_bytes_len(),
            },
            sample_count,
        })
    }

    /// Returns the payload index for a variable-length channel, reusing the
    /// last one when several channels share a payload stream.
    fn payloads_for(&self, channel: &Channel) -> Result<Arc<VlsdPayloads>> {
        let link = channel.data_link();
        if let Ok(guard) = self.payload_cache.read() {
            if let Some((cached_link, payloads)) = guard.as_ref() {
                if *cached_link == link && link != 0 {
                    return Ok(payloads.clone());
                }
            }
        }

        let built = Arc::new(self.vlsd_payloads(channel)?);

        if let Ok(mut guard) = self.payload_cache.write() {
            *guard = Some((link, built.clone()));
        }
        Ok(built)
    }

    /// Collects the payloads a variable-length channel refers to.
    ///
    /// A variable-length channel does not store its values in the record. The
    /// record holds a byte offset into a separate stream of length-prefixed
    /// payloads, which lives either in the channel's own signal-data block or —
    /// as bus loggers write it — in a dedicated channel group alongside the
    /// records that point into it.
    ///
    /// Returns the payload stream indexed by the offsets the records carry.
    fn vlsd_payloads(&self, channel: &Channel) -> Result<VlsdPayloads> {
        let dg = &self.data_groups[channel.data_group_index];
        let link = channel.data_link();

        if link == 0 {
            return Err(Mf4Error::unsupported(
                "variable-length signal data (VLSD)",
                format!("channel '{}' has no signal-data link", channel.name),
            ));
        }

        // Form 1: the link names a channel group in this data group whose
        // records are the payloads.
        if let Some(cg_index) = dg
            .channel_groups
            .iter()
            .position(|cg| cg.matches_offset(link))
        {
            let Some(index) = &dg.record_index else {
                return Err(Mf4Error::unsupported(
                    "variable-length signal data (VLSD)",
                    format!(
                        "channel '{}' points at a channel group in a sorted data group",
                        channel.name
                    ),
                ));
            };
            let raw = self.read_raw_data_indexed(dg)?;
            return Ok(VlsdPayloads::from_records(
                &raw,
                index.offsets(cg_index),
                dg.rec_id_size as usize,
            ));
        }

        // Form 2: the link names a signal-data block holding the payloads back
        // to back.
        let block_id = self.source.read_bytes(link, 4)?;
        match &block_id[..] {
            b"##SD" | b"##DT" | b"##DZ" | b"##DL" | b"##HL" => {
                let index =
                    Self::build_data_block_index(&self.source, link, false, self.file_size)?;
                let mut stream = Vec::new();
                for (_offset, info) in index.iter() {
                    stream.extend(Self::read_block_payload(&self.source, info, self.limits)?);
                }
                Ok(VlsdPayloads::from_stream(&stream))
            }
            other => Err(Mf4Error::unsupported(
                "variable-length signal data (VLSD)",
                format!(
                    "channel '{}' points at an unexpected block '{}'",
                    channel.name,
                    String::from_utf8_lossy(other)
                ),
            )),
        }
    }

    /// Copies one channel group's records out of an interleaved stream.
    ///
    /// Each offset points at a record ID; the record's own bytes follow it. The
    /// result is a dense buffer of `payload`-sized records, so callers can index
    /// it with a plain stride. Records that would run past the end of `raw` are
    /// dropped, which is why the realised count is returned rather than assumed.
    fn gather_records(
        raw: &[u8],
        offsets: &[u64],
        rec_id_size: usize,
        payload: usize,
        limits: Limits,
    ) -> (Vec<u8>, usize) {
        if payload == 0 {
            return (Vec::new(), 0);
        }

        let mut out =
            Vec::with_capacity(offsets.len().saturating_mul(payload).min(limits.max_alloc));
        for &offset in offsets {
            let start = offset as usize + rec_id_size;
            let Some(slice) = raw.get(start..start + payload) else {
                break;
            };
            out.extend_from_slice(slice);
        }

        let count = out.len() / payload;
        (out, count)
    }

    /// Reads raw data using the data block index.
    fn read_raw_data_indexed(&self, dg: &DataGroup) -> Result<Vec<u8>> {
        if dg.data_block_index.is_empty() {
            return Ok(Vec::new());
        }

        // Reserve the whole payload up front. Growing from a clamped hint
        // instead means repeatedly reallocating and copying: assembling a
        // 400 MB group that way holds both the old and new buffers at each
        // doubling, roughly tripling peak memory.
        //
        // The total is safe to trust here because `parse_block_header` has
        // already rejected any block extending past the end of the file; the
        // bound below covers a data list that names the same block repeatedly.
        let total_size = dg.data_block_index.total_size() as usize;
        let capacity = total_size.min(self.file_size as usize);
        let mut all_data = Vec::with_capacity(capacity);

        for (_offset, block_info) in dg.data_block_index.iter() {
            self.append_data_block(block_info, &mut all_data)?;
        }

        Ok(all_data)
    }

    /// Reads and decompresses a single data block.
    /// Appends one data block's payload to `out`, decompressing if needed.
    ///
    /// Appending rather than returning a buffer matters at scale: materialising
    /// each block separately and then copying it into the group's buffer means
    /// holding both at once, which for a single large block doubles peak memory
    /// for no benefit. An uncompressed block is copied straight out of the
    /// mapping.
    fn append_data_block(&self, info: &DataBlockInfo, out: &mut Vec<u8>) -> Result<()> {
        if let Some(compression) = &info.compression {
            let compressed_data = self
                .source
                .read_bytes(compression.data_offset, info.compressed_size as usize)?;
            let payload = Self::decompress(
                &compressed_data,
                compression,
                info.original_size as usize,
                self.limits,
            )?;
            out.extend_from_slice(&payload);
        } else {
            let data_offset = info.offset + BLOCK_HEADER_SIZE as u64;
            let data = self
                .source
                .read_bytes(data_offset, info.original_size as usize)?;
            out.extend_from_slice(&data);
        }
        Ok(())
    }

    /// Decompresses data according to compression info.
    fn decompress(
        compressed: &[u8],
        compression: &CompressionInfo,
        original_size: usize,
        limits: Limits,
    ) -> Result<Vec<u8>> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;

        match compression.algorithm {
            CompressionType::Deflate => {
                let mut decoder = ZlibDecoder::new(compressed).take(limits.max_decompressed);
                let mut decompressed = Vec::with_capacity(original_size.min(limits.max_alloc));
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|e| Mf4Error::Decompression(e.to_string()))?;
                Ok(decompressed)
            }
            CompressionType::TransposedDeflate => {
                // First decompress
                let mut decoder = ZlibDecoder::new(compressed).take(limits.max_decompressed);
                let mut transposed = Vec::with_capacity(original_size.min(limits.max_alloc));
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

                let row_count = transposed.len().div_ceil(column_size);
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

    /// Reads one of a sample reduction's condensed value series for a channel.
    ///
    /// A reduction condenses the group's data into one record per interval,
    /// each holding three copies of the group's normal record: the means, then
    /// the minima, then the maxima. `kind` selects which of the three to read.
    ///
    /// # Example
    /// ```no_run
    /// # use falcon_mdf::ReductionKind;
    /// # let file = falcon_mdf::Mf4File::open("data.mf4")?;
    /// # let channel = file.channels().next().unwrap();
    /// let group = &file.data_groups()[channel.data_group_index]
    ///     .channel_groups[channel.channel_group_index];
    ///
    /// if let Some(level) = group.sample_reductions().first() {
    ///     let peaks = file.reduced_signal(channel, level, ReductionKind::Max)?;
    ///     println!("{} reduced samples", peaks.len());
    /// }
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn reduced_signal(
        &self,
        channel: &Channel,
        reduction: &SampleReduction,
        kind: ReductionKind,
    ) -> Result<Signal> {
        let dg = &self.data_groups[channel.data_group_index];
        let cg = &dg.channel_groups[channel.channel_group_index];

        if reduction.data_link == 0 {
            return Err(Mf4Error::unsupported(
                "sample reduction",
                format!(
                    "the reduction attached to '{}' names no data block",
                    channel.name
                ),
            ));
        }

        // One reduced record is three of the group's records back to back.
        let block = cg.data_bytes_len();
        if block == 0 {
            return Err(Mf4Error::unsupported(
                "sample reduction",
                format!("channel group of '{}' has no record bytes", channel.name),
            ));
        }

        let index =
            Self::build_data_block_index(&self.source, reduction.data_link, false, self.file_size)?;
        let mut records = Vec::new();
        for (_offset, info) in index.iter() {
            self.append_data_block(info, &mut records)?;
        }

        let available = records.len() / (block * 3);
        let sample_count = (reduction.cycle_count as usize).min(available);

        Ok(Signal::new(
            channel.clone(),
            Arc::new(records),
            RecordLayout {
                record_size: block * 3,
                // The chosen series sits a whole record into the triple.
                record_offset: kind.index() * block,
                inval_start: cg.data_bytes_len(),
                inval_bytes: 0,
            },
            sample_count,
        ))
    }

    /// Returns the header's metadata: the properties a writer recorded about
    /// the device, tool and measurement environment.
    ///
    /// # Example
    /// ```no_run
    /// # let file = falcon_mdf::Mf4File::open("data.mf4")?;
    /// if let Some(serial) = file.metadata().get("Device Information/serial number") {
    ///     println!("recorded by device {serial}");
    /// }
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Returns channel database statistics (for debugging/profiling).
    pub fn channels_db_stats(&self) -> (usize, usize) {
        (
            self.channels_db.unique_name_count(),
            self.channels_db.total_channel_count(),
        )
    }

    /// Returns all file-level attachments.
    ///
    /// Attachments are files embedded in or referenced by the MF4 file, such
    /// as configuration dumps, screenshots, or DBC databases. Use
    /// [`Mf4File::attachment_data`] to read the bytes of an embedded
    /// attachment.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    /// Returns all file-level events.
    ///
    /// Events mark discrete moments in the measurement: trigger points,
    /// markers, recording start/stop, or external sync events.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns all channel hierarchy nodes.
    ///
    /// The channel hierarchy groups channels into named subtrees, providing a
    /// logical organisation independent of the data-group structure.
    /// Returns the file's change history, oldest entry first.
    ///
    /// The first entry records the file's creation and names the tool that
    /// wrote it; later entries record modifications.
    ///
    /// # Example
    /// ```no_run
    /// # let file = falcon_mdf::Mf4File::open("data.mf4")?;
    /// if let Some(created) = file.file_history().first() {
    ///     println!("written by {:?}", created.tool_id());
    /// }
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn file_history(&self) -> &[FileHistoryEntry] {
        &self.file_history
    }

    /// Returns the file's channel hierarchy: a logical grouping of channels
    /// independent of how they are stored in channel groups.
    pub fn channel_hierarchy(&self) -> &[ChannelHierarchyNode] {
        &self.hierarchy
    }

    /// Reads the embedded bytes of an attachment.
    ///
    /// Returns `Ok(None)` for external attachments (whose data is not in the
    /// file), `Ok(Some(bytes))` for embedded ones, or an error if the read
    /// fails.
    pub fn attachment_data(&self, attachment: &Attachment) -> Result<Option<Vec<u8>>> {
        if !attachment.is_embedded || attachment.embedded_offset == 0 {
            return Ok(None);
        }
        let data = self.source.read_bytes(
            attachment.embedded_offset,
            attachment.embedded_size as usize,
        )?;
        Ok(Some(data.to_vec()))
    }

    /// Parses the attachment chain starting at `first_at`.
    fn parse_attachments(
        source: &IoBackend,
        first_at: u64,
        cache: &mut BlockCache,
    ) -> Result<Vec<Attachment>> {
        let mut attachments = Vec::new();
        let mut offset = first_at;
        let mut chain = LinkChain::new();

        while offset != 0 {
            chain.visit(offset, "at_next")?;
            let at_block = parser::parse_at_block(source, offset)?;

            let file_name = cache
                .get_or_parse_text(source, at_block.tx_file_name)?
                .to_string();
            let file_path = cache
                .get_or_parse_text(source, at_block.tx_file_path)?
                .to_string();
            let comment = cache
                .get_or_parse_text(source, at_block.md_comment)?
                .to_string();

            attachments.push(Attachment {
                file_name,
                file_path,
                comment,
                is_embedded: at_block.is_embedded(),
                original_size: at_block.original_size,
                md5_checksum: at_block.md5_checksum,
                embedded_offset: at_block.embedded_data_offset(),
                embedded_size: at_block.embedded_size,
            });

            offset = at_block.at_next;
        }

        Ok(attachments)
    }

    /// Parses the event chain starting at `first_ev`.
    fn parse_events(
        source: &IoBackend,
        first_ev: u64,
        cache: &mut BlockCache,
    ) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut offset = first_ev;
        let mut chain = LinkChain::new();

        while offset != 0 {
            chain.visit(offset, "ev_next")?;
            let ev_block = parser::parse_ev_block(source, offset)?;

            let comment = cache
                .get_or_parse_text(source, ev_block.md_comment)?
                .to_string();
            let name = cache
                .get_or_parse_text(source, ev_block.tx_name)?
                .to_string();

            events.push(Event {
                event_type: ev_block.ev_type,
                sync_type: ev_block.ev_sync_type,
                range_type: ev_block.ev_range_type,
                cause: ev_block.ev_cause,
                sync_base_value: ev_block.ev_sync_base_value,
                sync_factor: ev_block.ev_sync_factor,
                scope_count: ev_block.ev_scope_count,
                attachment_count: ev_block.ev_attachment_count,
                comment,
                name,
            });

            offset = ev_block.ev_next;
        }

        Ok(events)
    }

    /// Walks a channel group's sample-reduction chain.
    ///
    /// Only the descriptors are read. Each block points at reduction data whose
    /// record layout this version cannot verify, so the values it condenses are
    /// deliberately not surfaced — see [`SampleReduction`].
    fn parse_sample_reductions(source: &IoBackend, first_sr: u64) -> Result<Vec<SampleReduction>> {
        let mut levels = Vec::new();
        let mut offset = first_sr;
        let mut chain = LinkChain::new();

        while offset != 0 {
            chain.visit(offset, "sr_next")?;
            let sr = parser::parse_sr_block(source, offset)?;

            levels.push(SampleReduction {
                cycle_count: sr.sr_cycle_count,
                interval: sr.sr_interval,
                sync_type: sr.sr_sync_type,
                flags: sr.sr_flags,
                data_link: sr.sr_data,
            });

            offset = sr.sr_next;
        }

        Ok(levels)
    }

    /// Walks the file-history chain starting at `first_fh`.
    ///
    /// Every MF4 file is required to carry at least one entry — its creation —
    /// so an empty result means the file omitted something the standard
    /// mandates, not that nothing happened.
    fn parse_file_history(
        source: &IoBackend,
        first_fh: u64,
        cache: &mut BlockCache,
    ) -> Result<Vec<FileHistoryEntry>> {
        let mut entries = Vec::new();
        let mut offset = first_fh;
        let mut chain = LinkChain::new();

        while offset != 0 {
            chain.visit(offset, "fh_next")?;
            let fh = parser::parse_fh_block(source, offset)?;

            let comment = cache.get_or_parse_text(source, fh.md_comment)?.to_string();
            let metadata = parser::read_metadata(source, fh.md_comment)?.unwrap_or_default();

            entries.push(FileHistoryEntry {
                time: RecordingTime::new(fh.time_ns as i64, fh.tz_offset_min, fh.dst_offset_min),
                comment,
                metadata,
            });

            offset = fh.fh_next;
        }

        Ok(entries)
    }

    /// Parses the channel hierarchy chain starting at `first_ch`.
    fn parse_hierarchy(
        source: &IoBackend,
        first_ch: u64,
        cache: &mut BlockCache,
    ) -> Result<Vec<ChannelHierarchyNode>> {
        let mut nodes = Vec::new();
        let mut offset = first_ch;
        let mut chain = LinkChain::new();

        while offset != 0 {
            chain.visit(offset, "ch_next")?;
            let ch_block = parser::parse_ch_block(source, offset)?;

            let name = cache
                .get_or_parse_text(source, ch_block.tx_name)?
                .to_string();
            let comment = cache
                .get_or_parse_text(source, ch_block.md_comment)?
                .to_string();

            nodes.push(ChannelHierarchyNode {
                name,
                comment,
                hierarchy_type: ch_block.ch_type,
                elements: ch_block.ch_element.clone(),
                has_children: ch_block.ch_first != 0,
            });

            offset = ch_block.ch_next;
        }

        Ok(nodes)
    }
}

/// Builds a [`Conversion`] from a CC block, resolving any text it references.
///
/// Text-table conversions keep their strings in referenced TX blocks, so they
/// can only be prepared where the file and block cache are available. Anything
/// that cannot be prepared becomes [`Conversion::Unsupported`] carrying the
/// reason, never a silent fallback to raw values.
fn build_conversion(
    source: &IoBackend,
    cc: &CcBlock,
    cache: &mut BlockCache,
) -> Result<Conversion> {
    build_conversion_at_depth(source, cc, cache, 0)
}

/// How deep a chain of nested conversions may go.
///
/// Only bitfield tables nest, and one level is all any writer needs. The limit
/// exists because a `cc_ref` can point back at its own block: without it, such a
/// file would recurse until the stack ran out.
const MAX_CONVERSION_DEPTH: u32 = 4;

fn build_conversion_at_depth(
    source: &IoBackend,
    cc: &CcBlock,
    cache: &mut BlockCache,
    depth: u32,
) -> Result<Conversion> {
    use crate::blocks::{ConversionType as Ct, Expr};

    // Resolves one `cc_ref` link to text. A reference may legally point at a
    // nested CC block instead of a TX block; those are not evaluated here.
    let text_at = |cache: &mut BlockCache, idx: usize| -> Result<Option<String>> {
        let Some(&link) = cc.references.get(idx) else {
            return Ok(None);
        };
        if link == 0 {
            return Ok(None);
        }
        let s = cache.get_or_parse_text(source, link)?;
        Ok(Some(s.to_string()))
    };

    let unsupported = |kind: Ct, reason: &str| Conversion::Unsupported {
        kind,
        reason: reason.to_string(),
    };

    let v = &cc.values;
    Ok(match cc.conversion_type {
        Ct::Identity => Conversion::None,
        Ct::Linear => {
            if v.len() < 2 {
                unsupported(Ct::Linear, "linear conversion needs 2 parameters")
            } else {
                Conversion::Linear {
                    offset: v[0],
                    factor: v[1],
                }
            }
        }
        Ct::Rational => {
            if v.len() < 6 {
                unsupported(Ct::Rational, "rational conversion needs 6 parameters")
            } else {
                Conversion::Rational {
                    coefficients: [v[0], v[1], v[2], v[3], v[4], v[5]],
                }
            }
        }
        Ct::Algebraic => match text_at(cache, 0)? {
            Some(formula) => match Expr::parse(&formula) {
                Ok(expr) => Conversion::Algebraic { formula, expr },
                Err(e) => unsupported(Ct::Algebraic, &e.to_string()),
            },
            None => unsupported(Ct::Algebraic, "formula text is missing"),
        },
        Ct::TabInterpolation | Ct::TabLookup => {
            let n = v.len() / 2;
            if n == 0 {
                // With no entries there is nothing to look a value up in;
                // passing the raw value through would be a silent identity.
                unsupported(cc.conversion_type, "conversion table has no entries")
            } else {
                let keys = (0..n).map(|i| v[i * 2]).collect();
                let values = (0..n).map(|i| v[i * 2 + 1]).collect();
                if cc.conversion_type == Ct::TabInterpolation {
                    Conversion::TableInterpolated { keys, values }
                } else {
                    Conversion::TableLookup { keys, values }
                }
            }
        }
        Ct::TabRangeLookup => {
            // Triples of (lower, upper, value), optionally followed by a default.
            let n = v.len() / 3;
            let default = if v.len() % 3 == 1 {
                v.last().copied()
            } else {
                None
            };
            if n == 0 && default.is_none() {
                return Ok(unsupported(
                    Ct::TabRangeLookup,
                    "range table has no entries and no default",
                ));
            }
            Conversion::RangeTable {
                lower: (0..n).map(|i| v[i * 3]).collect(),
                upper: (0..n).map(|i| v[i * 3 + 1]).collect(),
                values: (0..n).map(|i| v[i * 3 + 2]).collect(),
                default,
            }
        }
        Ct::TabValueToText => {
            // One key per entry, one text reference per entry, plus a trailing
            // default reference.
            let n = v.len();
            let mut entries = Vec::with_capacity(n);
            for i in 0..n {
                let link = cc.references.get(i).copied().unwrap_or(0);
                entries.push(
                    table_entry(source, cache, link, depth)?
                        .unwrap_or_else(|| TableEntry::Text(String::new())),
                );
            }
            let default_link = cc.references.get(n).copied().unwrap_or(0);
            Conversion::ValueToText {
                keys: v.clone(),
                entries,
                default: table_entry(source, cache, default_link, depth)?,
            }
        }
        Ct::TabRangeToText => {
            // Pairs of (lower, upper), one text per pair, plus a default.
            let n = v.len() / 2;
            let mut entries = Vec::with_capacity(n);
            for i in 0..n {
                let link = cc.references.get(i).copied().unwrap_or(0);
                entries.push(
                    table_entry(source, cache, link, depth)?
                        .unwrap_or_else(|| TableEntry::Text(String::new())),
                );
            }
            let default_link = cc.references.get(n).copied().unwrap_or(0);
            Conversion::RangeToText {
                lower: (0..n).map(|i| v[i * 2]).collect(),
                upper: (0..n).map(|i| v[i * 2 + 1]).collect(),
                entries,
                default: table_entry(source, cache, default_link, depth)?,
            }
        }
        Ct::TabTextToValue => {
            // Every reference is a key; unlike type 7 there is no trailing
            // default *link*, because the default here is a number and lives at
            // the end of the value list instead.
            let n = cc.references.len();
            if v.len() < n {
                unsupported(
                    Ct::TabTextToValue,
                    "text-to-value table has fewer values than keys",
                )
            } else {
                let mut keys = Vec::with_capacity(n);
                for i in 0..n {
                    keys.push(text_at(cache, i)?.unwrap_or_default());
                }
                Conversion::TextToValue {
                    keys,
                    values: v[..n].to_vec(),
                    default: v.get(n).copied(),
                }
            }
        }
        Ct::TabTextToText => {
            // References alternate key, replacement, key, replacement, with a
            // single default at the end — so an even count means the file is
            // missing either a replacement or the default.
            let refs = cc.references.len();
            if refs == 0 || refs % 2 == 0 {
                unsupported(
                    Ct::TabTextToText,
                    "text-to-text table needs an odd number of references: key/text pairs plus a default",
                )
            } else {
                let n = (refs - 1) / 2;
                let mut keys = Vec::with_capacity(n);
                let mut texts = Vec::with_capacity(n);
                for i in 0..n {
                    keys.push(text_at(cache, i * 2)?.unwrap_or_default());
                    texts.push(text_at(cache, i * 2 + 1)?.unwrap_or_default());
                }
                Conversion::TextToText {
                    keys,
                    texts,
                    default: text_at(cache, refs - 1)?,
                }
            }
        }
        Ct::BitfieldToText => {
            if depth >= MAX_CONVERSION_DEPTH {
                unsupported(
                    Ct::BitfieldToText,
                    "nested conversions are more than four deep, which no valid file needs",
                )
            } else {
                // Type 11 alone stores its `cc_val` parameters as unsigned
                // integers rather than doubles. The parser reads every block's
                // values as `f64`, and `to_bits` recovers the eight bytes it
                // read, which are the mask exactly.
                let masks: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
                let n = masks.len().min(cc.references.len());
                let mut entries = Vec::with_capacity(n);

                for i in 0..n {
                    entries.push(bitfield_entry(source, cache, cc.references[i], depth + 1)?);
                }

                Conversion::Bitfield {
                    masks: masks[..n].to_vec(),
                    entries,
                }
            }
        }
        Ct::Unknown(code) => Conversion::Unsupported {
            kind: Ct::Unknown(code),
            reason: format!("unknown conversion type {code}"),
        },
    })
}

/// Resolves one `cc_ref` of a value-to-text or range-to-text table.
///
/// The standard calls types 7 and 8 "value to text/**scale**": a reference may
/// name a text block holding a label, or a CC block to apply to the raw value.
/// A file expresses a piecewise conversion that way — Vector's
/// `Vector_PartialConversion*` files use nothing but nested conversions — and
/// reading the reference as text is what made this reader refuse to open them.
///
/// Which it is has to be read from the block itself, since the link carries no
/// type. A missing or unreadable reference becomes an empty label rather than
/// an error, so one bad entry does not cost the whole channel.
fn table_entry(
    source: &IoBackend,
    cache: &mut BlockCache,
    link: u64,
    depth: u32,
) -> Result<Option<TableEntry>> {
    if link == 0 {
        return Ok(None);
    }

    let id = match source.read_bytes(link, 4) {
        Ok(bytes) => {
            let mut id = [0u8; 4];
            id.copy_from_slice(&bytes);
            id
        }
        Err(_) => return Ok(None),
    };

    match &id {
        b"##CC" => {
            if depth >= MAX_CONVERSION_DEPTH {
                return Ok(None);
            }
            let Some(nested) = cache.get_or_parse_cc(source, link)? else {
                return Ok(None);
            };
            let conversion = build_conversion_at_depth(source, &nested, cache, depth + 1)?;
            Ok(Some(TableEntry::Nested(Box::new(conversion))))
        }
        b"##TX" | b"##MD" => Ok(Some(TableEntry::Text(
            cache.get_or_parse_text(source, link)?.to_string(),
        ))),
        _ => Ok(None),
    }
}

/// Resolves one `cc_ref` of a bitfield table.
///
/// A bitfield entry's reference is a text block holding a label, or a nested
/// conversion block holding a table. Which one it is has to be read from the
/// block itself, since the link carries no type.
fn bitfield_entry(
    source: &IoBackend,
    cache: &mut BlockCache,
    link: u64,
    depth: u32,
) -> Result<crate::blocks::BitfieldEntry> {
    use crate::blocks::BitfieldEntry;

    if link == 0 {
        return Ok(BitfieldEntry::Unresolved);
    }

    let id = match source.read_bytes(link, 4) {
        Ok(bytes) => {
            let mut id = [0u8; 4];
            id.copy_from_slice(&bytes);
            id
        }
        // A link past the end of the file describes nothing. The rest of the
        // table is still readable, so drop this entry rather than the channel.
        Err(_) => return Ok(BitfieldEntry::Unresolved),
    };

    match &id {
        b"##CC" => {
            let Some(nested) = cache.get_or_parse_cc(source, link)? else {
                return Ok(BitfieldEntry::Unresolved);
            };
            let name = cache.get_or_parse_text(source, nested.tx_name)?.to_string();
            let conversion = build_conversion_at_depth(source, &nested, cache, depth)?;
            Ok(BitfieldEntry::Nested {
                name,
                conversion: Box::new(conversion),
            })
        }
        b"##TX" | b"##MD" => Ok(BitfieldEntry::Flag(
            cache.get_or_parse_text(source, link)?.to_string(),
        )),
        _ => Ok(BitfieldEntry::Unresolved),
    }
}

/// What happened when a channel's composition was expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompositionOutcome {
    /// The composition was expanded into sub-channels.
    Expanded,
    /// The composition is an array, which this version cannot expand.
    UnsupportedArray,
}

/// Reads the record ID prefixing a record in an unsorted data group.
///
/// Returns `None` if the ID would run past the end of the buffer, or if
/// `rec_id_size` is not one of the sizes the format allows (1, 2, 4, 8).
fn read_record_id(data: &[u8], pos: usize, rec_id_size: u8) -> Option<u64> {
    let end = pos.checked_add(rec_id_size as usize)?;
    let bytes = data.get(pos..end)?;
    Some(match rec_id_size {
        1 => bytes[0] as u64,
        2 => u16::from_le_bytes(bytes.try_into().ok()?) as u64,
        4 => u32::from_le_bytes(bytes.try_into().ok()?) as u64,
        8 => u64::from_le_bytes(bytes.try_into().ok()?),
        _ => return None,
    })
}

/// Builds the display name for a channel nested inside a composition.
///
/// Writers disagree on what they store in a nested channel's TX name. Some
/// write the bare member name (`BusChannel`), others write it already qualified
/// by the parent structure (`CAN_DataFrame.BusChannel`). Prefixing
/// unconditionally turns the second form into `CAN_DataFrame.CAN_DataFrame.
/// BusChannel`, so qualify only when the child is not already qualified.
fn qualify_channel_name(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        return child.to_string();
    }
    if child.is_empty() {
        return parent.to_string();
    }
    if child == parent {
        return child.to_string();
    }
    if let Some(rest) = child.strip_prefix(parent) {
        if rest.starts_with('.') {
            return child.to_string();
        }
    }
    format!("{parent}.{child}")
}

impl std::fmt::Debug for Mf4File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mf4File")
            .field("version", &self.version)
            .field("data_group_count", &self.data_groups.len())
            .field("channel_count", &self.channel_count())
            .field(
                "unique_channel_names",
                &self.channels_db.unique_name_count(),
            )
            .field("file_size", &self.file_size)
            .finish()
    }
}

#[cfg(test)]
mod name_tests {
    use super::qualify_channel_name;

    #[test]
    fn qualifies_bare_child_names() {
        assert_eq!(
            qualify_channel_name("CAN_DataFrame", "BusChannel"),
            "CAN_DataFrame.BusChannel"
        );
    }

    #[test]
    fn leaves_already_qualified_names_alone() {
        assert_eq!(
            qualify_channel_name("CAN_DataFrame", "CAN_DataFrame.BusChannel"),
            "CAN_DataFrame.BusChannel"
        );
    }

    #[test]
    fn does_not_treat_a_shared_prefix_as_qualification() {
        // "CAN_DataFrameExtra" starts with the parent but is a different name,
        // so it still needs qualifying.
        assert_eq!(
            qualify_channel_name("CAN_DataFrame", "CAN_DataFrameExtra"),
            "CAN_DataFrame.CAN_DataFrameExtra"
        );
    }

    #[test]
    fn handles_nested_qualification() {
        assert_eq!(qualify_channel_name("A.B", "A.B.C"), "A.B.C");
        assert_eq!(qualify_channel_name("A.B", "C"), "A.B.C");
    }

    #[test]
    fn handles_empty_and_identical_parts() {
        assert_eq!(qualify_channel_name("", "X"), "X");
        assert_eq!(qualify_channel_name("X", ""), "X");
        assert_eq!(qualify_channel_name("X", "X"), "X");
    }
}

#[cfg(test)]
mod demux_tests {
    use super::{read_record_id, Limits, Mf4File};

    #[test]
    fn reads_record_ids_of_each_permitted_width() {
        let data = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
        assert_eq!(read_record_id(&data, 0, 1), Some(0xAA));
        assert_eq!(read_record_id(&data, 0, 2), Some(0xBBAA));
        assert_eq!(read_record_id(&data, 0, 4), Some(0xDDCC_BBAA));
        assert_eq!(read_record_id(&data, 0, 8), Some(0x2211_FFEE_DDCC_BBAA));
    }

    #[test]
    fn reads_record_id_at_an_offset() {
        let data = [0x00, 0x00, 0x07, 0x00];
        assert_eq!(read_record_id(&data, 2, 2), Some(7));
    }

    #[test]
    fn rejects_ids_running_past_the_buffer() {
        let data = [0x01, 0x02, 0x03];
        assert_eq!(read_record_id(&data, 2, 2), None);
        assert_eq!(read_record_id(&data, 3, 1), None);
        assert_eq!(read_record_id(&data, 0, 8), None);
    }

    #[test]
    fn rejects_widths_the_format_does_not_define() {
        let data = [0u8; 16];
        assert_eq!(read_record_id(&data, 0, 0), None);
        assert_eq!(read_record_id(&data, 0, 3), None);
        assert_eq!(read_record_id(&data, 0, 255), None);
    }

    #[test]
    fn gathers_interleaved_records_into_a_dense_buffer() {
        // Two groups interleaved, 1-byte record IDs. Group 1 has a 2-byte
        // payload, group 2 a 3-byte payload.
        //   [1|aa bb][2|11 22 33][1|cc dd][2|44 55 66]
        let raw = [
            1, 0xAA, 0xBB, //
            2, 0x11, 0x22, 0x33, //
            1, 0xCC, 0xDD, //
            2, 0x44, 0x55, 0x66,
        ];
        let group1 = [0u64, 7];
        let group2 = [3u64, 10];

        let (out, n) = Mf4File::gather_records(&raw, &group1, 1, 2, Limits::default());
        assert_eq!(n, 2);
        assert_eq!(out, vec![0xAA, 0xBB, 0xCC, 0xDD]);

        let (out, n) = Mf4File::gather_records(&raw, &group2, 1, 3, Limits::default());
        assert_eq!(n, 2);
        assert_eq!(out, vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    }

    #[test]
    fn drops_a_record_truncated_by_the_end_of_the_stream() {
        // The second record claims 4 payload bytes but only 2 remain.
        let raw = [1, 0xAA, 0xBB, 0xCC, 0xDD, 1, 0x11, 0x22];
        let (out, n) = Mf4File::gather_records(&raw, &[0, 5], 1, 4, Limits::default());
        assert_eq!(
            n, 1,
            "the truncated tail record must be dropped, not padded"
        );
        assert_eq!(out, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn handles_empty_inputs() {
        assert_eq!(
            Mf4File::gather_records(&[], &[], 1, 4, Limits::default()),
            (Vec::new(), 0)
        );
        // A zero-size payload would make the record count meaningless.
        assert_eq!(
            Mf4File::gather_records(&[1, 2, 3], &[0], 1, 0, Limits::default()),
            (Vec::new(), 0)
        );
    }

    #[test]
    fn skips_the_record_id_when_gathering() {
        let raw = [0xFF, 0xFF, 0x42, 0x43];
        let (out, n) = Mf4File::gather_records(&raw, &[0], 2, 2, Limits::default());
        assert_eq!(n, 1);
        assert_eq!(out, vec![0x42, 0x43], "record ID bytes must not be copied");
    }
}
