//! High-level MF4 file interface.
//!
//! This module provides the main entry point for reading MF4 files.
//! The `Mf4File` type wraps all the complexity of the file format
//! and provides a clean, ergonomic API.

use std::path::Path;

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSource, IoBackend};
use crate::blocks::{
    CgBlock, CnBlock, DgBlock, HdBlock, DataBlock, DtBlock, DzBlock, DlBlock,
    Conversion, ParseBlock, BLOCK_HEADER_SIZE,
};
use crate::blocks::source::SourceInfo;
use crate::model::{
    Channel, ChannelGroup, DataGroup, FileStatistics, RecordingTime, Signal,
};
use crate::parser::{
    self, Mf4Version, parse_id_block, parse_hd_block, parse_cc_block, read_text,
};

/// The main interface for reading MF4 files.
///
/// `Mf4File` provides access to all data in an MF4 measurement file,
/// including metadata, channel definitions, and sample data.
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
/// // List all channels
/// for channel in file.channels() {
///     println!("  {} [{}]", channel.name, channel.unit);
/// }
///
/// // Read data from a specific channel
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
    comment: String,
    /// Data groups containing channel groups and channels.
    data_groups: Vec<DataGroup>,
    /// Total file size in bytes.
    file_size: u64,
}

impl Mf4File {
    /// Opens an MF4 file for reading.
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
        let source = IoBackend::open(path)?;
        Self::from_source(source)
    }

    /// Opens an MF4 file using memory-mapped I/O.
    ///
    /// This is the most efficient method for large files.
    #[cfg(feature = "mmap")]
    pub fn open_mmap<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = IoBackend::open_mmap(path)?;
        Self::from_source(source)
    }

    /// Opens an MF4 file using buffered I/O.
    ///
    /// Use this method when memory-mapped I/O is not available
    /// or not desired (e.g., for network files).
    pub fn open_buffered<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = IoBackend::open_buffered(path)?;
        Self::from_source(source)
    }

    /// Creates an Mf4File from a byte source.
    fn from_source(source: IoBackend) -> Result<Self> {
        let file_size = source.len();

        // Parse ID block
        let id_block = parse_id_block(&source)?;
        let version = Mf4Version::from_id_block(&id_block);
        
        // Validate version is supported
        version.validate()?;

        // Parse HD block (always at offset 64)
        let hd_block = parse_hd_block(&source, 64)?;

        // Extract start time
        let start_time = RecordingTime::new(
            hd_block.start_time_ns,
            hd_block.tz_offset_min,
            hd_block.dst_offset_min,
        );

        // Read file comment
        let comment = read_text(&source, hd_block.md_comment).unwrap_or_default();

        // Parse data groups
        let data_groups = Self::parse_data_groups(&source, &hd_block)?;

        Ok(Mf4File {
            source,
            version,
            start_time,
            comment,
            data_groups,
            file_size,
        })
    }

    /// Parses all data groups from the file.
    fn parse_data_groups(source: &IoBackend, hd: &HdBlock) -> Result<Vec<DataGroup>> {
        let mut data_groups = Vec::new();
        let mut dg_offset = hd.dg_first;
        let mut dg_index = 0;

        while dg_offset != 0 {
            let dg_block = parser::parse_dg_block(source, dg_offset)?;
            
            // Parse channel groups for this data group
            let channel_groups = Self::parse_channel_groups(source, &dg_block, dg_index)?;

            // Read data group comment
            let comment = read_text(source, dg_block.md_comment).unwrap_or_default();

            let data_group = DataGroup {
                id: dg_index,
                index: dg_index,
                channel_groups,
                comment,
                dg_offset,
                data_offset: dg_block.data,
                rec_id_size: dg_block.rec_id_size,
            };

            data_groups.push(data_group);
            dg_offset = dg_block.dg_next;
            dg_index += 1;
        }

        Ok(data_groups)
    }

    /// Parses all channel groups for a data group.
    fn parse_channel_groups(
        source: &IoBackend,
        dg: &DgBlock,
        dg_index: usize,
    ) -> Result<Vec<ChannelGroup>> {
        let mut channel_groups = Vec::new();
        let mut cg_offset = dg.cg_first;
        let mut cg_index = 0;

        while cg_offset != 0 {
            let cg_block = parser::parse_cg_block(source, cg_offset)?;

            // Parse channels for this channel group
            let channels = Self::parse_channels(source, &cg_block, dg_index, cg_index)?;

            // Read acquisition name
            let acquisition_name = read_text(source, cg_block.tx_acq_name).unwrap_or_default();

            // Read comment
            let comment = read_text(source, cg_block.md_comment).unwrap_or_default();

            let channel_group = ChannelGroup {
                id: cg_index,
                index: cg_index,
                data_group_index: dg_index,
                acquisition_name,
                sample_count: cg_block.cycle_count,
                channels,
                source: None, // TODO: Parse SI block if present
                comment,
                record_id: cg_block.record_id,
                data_bytes: cg_block.data_bytes,
                inval_bytes: cg_block.inval_bytes,
                cg_offset,
            };

            channel_groups.push(channel_group);
            cg_offset = cg_block.cg_next;
            cg_index += 1;
        }

        Ok(channel_groups)
    }

    /// Parses all channels for a channel group.
    fn parse_channels(
        source: &IoBackend,
        cg: &CgBlock,
        dg_index: usize,
        cg_index: usize,
    ) -> Result<Vec<Channel>> {
        let mut channels = Vec::new();
        let mut cn_offset = cg.cn_first;
        let mut cn_index = 0;

        while cn_offset != 0 {
            let cn_block = parser::parse_cn_block(source, cn_offset)?;

            // Read channel name
            let name = read_text(source, cn_block.tx_name)?;

            // Read unit
            let unit = read_text(source, cn_block.md_unit).unwrap_or_default();

            // Read comment
            let comment = read_text(source, cn_block.md_comment).unwrap_or_default();

            // Parse conversion if present
            let conversion = if cn_block.cc_conversion != 0 {
                let cc_block = parse_cc_block(source, cn_block.cc_conversion)?;
                Conversion::from_cc_block(cc_block)
            } else {
                Conversion::None
            };

            // Extract value range if valid
            let (min_value, max_value) = if cn_block.flags.range_valid {
                (Some(cn_block.val_range_min), Some(cn_block.val_range_max))
            } else {
                (None, None)
            };

            let channel = Channel {
                id: cn_index,
                index: cn_index,
                channel_group_index: cg_index,
                data_group_index: dg_index,
                name,
                unit,
                channel_type: cn_block.channel_type,
                sync_type: cn_block.sync_type,
                data_type: cn_block.data_type,
                conversion,
                bit_count: cn_block.bit_count,
                byte_offset: cn_block.byte_offset,
                bit_offset: cn_block.bit_offset,
                comment,
                source: None, // TODO: Parse SI block if present
                min_value,
                max_value,
                cn_offset,
            };

            channels.push(channel);
            cn_offset = cn_block.cn_next;
            cn_index += 1;
        }

        Ok(channels)
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
        self.data_groups
            .iter()
            .flat_map(|dg| &dg.channel_groups)
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

    /// Finds a channel by name.
    ///
    /// If multiple channels have the same name, returns the first one found.
    pub fn find_channel(&self, name: &str) -> Option<&Channel> {
        self.channels().find(|ch| ch.name == name)
    }

    /// Finds all channels matching the given name.
    pub fn find_channels(&self, name: &str) -> Vec<&Channel> {
        self.channels().filter(|ch| ch.name == name).collect()
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

        // Read raw data
        let raw_data = self.read_raw_data(dg, cg)?;

        // Calculate record parameters
        let record_size = cg.record_size(dg.rec_id_size);
        let record_offset = dg.rec_id_size as usize;

        Ok(Signal::new(
            channel.clone(),
            raw_data,
            record_size,
            record_offset,
            cg.sample_count as usize,
        ))
    }

    /// Reads raw record data for a channel group.
    fn read_raw_data(&self, dg: &DataGroup, cg: &ChannelGroup) -> Result<Vec<u8>> {
        if dg.data_offset == 0 {
            // No data block
            return Ok(Vec::new());
        }

        // Parse the data block to determine type
        let header = parser::parse_block_header(&self.source, dg.data_offset)?;
        let block_data = self.source.read_bytes(dg.data_offset, header.length as usize)?;

        match &block_data[0..4] {
            b"##DT" | b"##SD" => {
                // Plain data block
                let data_start = BLOCK_HEADER_SIZE;
                Ok(block_data[data_start..].to_vec())
            }
            b"##DZ" => {
                // Compressed data block
                let dz = DzBlock::parse(&block_data, dg.data_offset)?;
                let compressed_data = self.source.read_bytes(
                    dz.compressed_data_offset,
                    dz.compressed_size as usize,
                )?;
                dz.decompress(&compressed_data)
            }
            b"##DL" => {
                // Data list - collect all referenced data blocks
                self.read_data_list(dg.data_offset, cg)
            }
            b"##HL" => {
                // Header list - read first DL and process
                let hl_data = self.source.read_bytes(dg.data_offset, header.length as usize)?;
                let dl_first_link = if hl_data.len() >= 32 {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&hl_data[24..32]);
                    u64::from_le_bytes(bytes)
                } else {
                    0
                };
                if dl_first_link != 0 {
                    self.read_data_list(dl_first_link, cg)
                } else {
                    Ok(Vec::new())
                }
            }
            _ => Err(Mf4Error::invalid_block_id(
                dg.data_offset,
                "##DT/##DZ/##DL/##HL",
                String::from_utf8_lossy(&block_data[0..4]).to_string(),
            )),
        }
    }

    /// Reads data from a data list (DL) block.
    fn read_data_list(&self, dl_offset: u64, cg: &ChannelGroup) -> Result<Vec<u8>> {
        let mut all_data = Vec::new();
        let mut current_dl = dl_offset;

        while current_dl != 0 {
            let header = parser::parse_block_header(&self.source, current_dl)?;
            let dl_data = self.source.read_bytes(current_dl, header.length as usize)?;
            let dl = DlBlock::parse(&dl_data, current_dl)?;

            // Process each data block link
            for &data_link in &dl.data_links {
                if data_link == 0 {
                    continue;
                }

                let block_header = parser::parse_block_header(&self.source, data_link)?;
                let block = self.source.read_bytes(data_link, block_header.length as usize)?;

                match &block[0..4] {
                    b"##DT" | b"##SD" => {
                        let data_start = BLOCK_HEADER_SIZE;
                        all_data.extend_from_slice(&block[data_start..]);
                    }
                    b"##DZ" => {
                        let dz = DzBlock::parse(&block, data_link)?;
                        let compressed = self.source.read_bytes(
                            dz.compressed_data_offset,
                            dz.compressed_size as usize,
                        )?;
                        let decompressed = dz.decompress(&compressed)?;
                        all_data.extend(decompressed);
                    }
                    _ => {
                        // Skip unknown block types
                    }
                }
            }

            current_dl = dl.dl_next;
        }

        Ok(all_data)
    }

    /// Returns the file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }
}

impl std::fmt::Debug for Mf4File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mf4File")
            .field("version", &self.version)
            .field("data_group_count", &self.data_groups.len())
            .field("channel_count", &self.channel_count())
            .field("file_size", &self.file_size)
            .finish()
    }
}
