//! ID and Header (HD) block parsing.
//!
//! The ID block is always at offset 0 and contains file identification info.
//! The HD block is the header block containing metadata and links to other blocks.

use crate::blocks::common::{read_link, BlockHeader, BLOCK_HEADER_SIZE, ID_BLOCK_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Valid file identifier strings for MF4 files.
const MF4_SIGNATURE: &[u8; 8] = b"MDF     ";
const UNFINISHED_SIGNATURE: &[u8; 8] = b"UnFinMF ";

/// The identification block at the start of every MF4 file.
///
/// This block is always 64 bytes and located at file offset 0.
#[derive(Debug, Clone)]
pub struct IdBlock {
    /// File identifier ("MDF     " or "UnFinMF ").
    pub file_id: [u8; 8],
    /// Format identifier ("4.20" etc).
    pub format_id: [u8; 8],
    /// Program identifier.
    pub program_id: [u8; 8],
    /// Reserved bytes.
    pub reserved1: [u8; 4],
    /// Version number (major * 100 + minor, e.g., 420 for v4.20).
    pub version_number: u16,
    /// Reserved bytes.
    pub reserved2: [u8; 30],
    /// Unfinalized flags (0 = finalized).
    pub unfinalized_flags: u16,
    /// Custom unfinalized flags.
    pub custom_unfinalized_flags: u16,
}

/// What a writer left undone when it stopped writing a file.
///
/// A measurement tool writes an MF4 file as it records and finalises it at the
/// end. If it stops first — a crash, a full disk, a logger losing power — the
/// file is left with the `UnFinMF` signature and these flags saying which
/// bookkeeping never got done. The data is usually all there.
///
/// This reader compensates for two of them without being asked: it takes sample
/// counts from the data rather than from `cg_cycle_count`, and it reads a last
/// data block whose declared length is zero to the end of the file. The rest
/// are reported and not acted on, so a caller can tell whether what it is about
/// to read is affected. That is deliberate — inventing the missing values would
/// be guessing, and refusing the file would withhold the channels that are fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnfinalizedFlags {
    /// Channel group cycle counters need updating (bit 0). **Compensated for:**
    /// sample counts are taken from the data the file actually holds.
    pub update_cg_counters: bool,
    /// Sample reduction cycle counters need updating (bit 1). Not compensated
    /// for — `ChannelGroup::sample_reductions` reports the declared counts.
    pub update_sr_counters: bool,
    /// The last data block's length needs updating (bit 2). **Compensated
    /// for:** a zero-length last block is read to the end of the file.
    pub update_last_dt_length: bool,
    /// The last reduction-data block's length needs updating (bit 3).
    pub update_last_rd_length: bool,
    /// The last data list needs updating (bit 4).
    pub update_last_dl: bool,
    /// A variable-length channel group's byte counts need updating (bit 5).
    pub update_vlsd_bytes: bool,
    /// A variable-length channel's offset values need updating (bit 6). Such a
    /// channel's payloads may not be resolvable at all.
    pub update_vlsd_offsets: bool,
    /// Flags the writer defined for itself, which no reader can interpret.
    pub custom: u16,
}

impl UnfinalizedFlags {
    fn from_parts(flags: u16, custom: u16) -> Self {
        UnfinalizedFlags {
            update_cg_counters: (flags & 0x01) != 0,
            update_sr_counters: (flags & 0x02) != 0,
            update_last_dt_length: (flags & 0x04) != 0,
            update_last_rd_length: (flags & 0x08) != 0,
            update_last_dl: (flags & 0x10) != 0,
            update_vlsd_bytes: (flags & 0x20) != 0,
            update_vlsd_offsets: (flags & 0x40) != 0,
            custom,
        }
    }
}

impl IdBlock {
    /// Returns what the writer left undone, or `None` for a finalized file.
    pub fn unfinalized(&self) -> Option<UnfinalizedFlags> {
        if !self.is_unfinished() {
            return None;
        }
        Some(UnfinalizedFlags::from_parts(
            self.unfinalized_flags,
            self.custom_unfinalized_flags,
        ))
    }

    /// Parses the ID block from the first 64 bytes of a file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < ID_BLOCK_SIZE {
            return Err(Mf4Error::truncated(0, ID_BLOCK_SIZE, data.len()));
        }

        let mut file_id = [0u8; 8];
        file_id.copy_from_slice(&data[0..8]);

        // Validate signature
        if &file_id != MF4_SIGNATURE && &file_id != UNFINISHED_SIGNATURE {
            return Err(Mf4Error::InvalidSignature(
                String::from_utf8_lossy(&file_id).to_string(),
            ));
        }

        let mut format_id = [0u8; 8];
        format_id.copy_from_slice(&data[8..16]);

        let mut program_id = [0u8; 8];
        program_id.copy_from_slice(&data[16..24]);

        let mut reserved1 = [0u8; 4];
        reserved1.copy_from_slice(&data[24..28]);

        let mut cursor = Cursor::new(&data[28..30]);
        let version_number = cursor.read_u16::<LittleEndian>()?;

        let mut reserved2 = [0u8; 30];
        reserved2.copy_from_slice(&data[30..60]);

        let mut cursor = Cursor::new(&data[60..62]);
        let unfinalized_flags = cursor.read_u16::<LittleEndian>()?;

        let mut cursor = Cursor::new(&data[62..64]);
        let custom_unfinalized_flags = cursor.read_u16::<LittleEndian>()?;

        Ok(IdBlock {
            file_id,
            format_id,
            program_id,
            reserved1,
            version_number,
            reserved2,
            unfinalized_flags,
            custom_unfinalized_flags,
        })
    }

    /// Returns the major version number.
    pub fn version_major(&self) -> u16 {
        self.version_number / 100
    }

    /// Returns the minor version number.
    pub fn version_minor(&self) -> u16 {
        self.version_number % 100
    }

    /// Returns the format identifier as a string.
    pub fn format_str(&self) -> &str {
        std::str::from_utf8(&self.format_id)
            .unwrap_or("????????")
            .trim_end()
    }

    /// Returns the program identifier as a string.
    pub fn program_str(&self) -> &str {
        std::str::from_utf8(&self.program_id)
            .unwrap_or("????????")
            .trim_end()
    }

    /// Returns true if the file is finalized.
    pub fn is_finalized(&self) -> bool {
        &self.file_id != UNFINISHED_SIGNATURE && self.unfinalized_flags == 0
    }

    /// Returns true if the file is unfinished (UnFinMF signature or unfinalized flags set).
    pub fn is_unfinished(&self) -> bool {
        &self.file_id == UNFINISHED_SIGNATURE || self.unfinalized_flags != 0
    }

    /// Returns the raw version number (e.g., 411 for version 4.11).
    pub fn version_raw(&self) -> u16 {
        self.version_number
    }
}

/// Header block flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdFlags {
    /// Start angle value is valid.
    pub start_angle_valid: bool,
    /// Start distance value is valid.
    pub start_distance_valid: bool,
}

impl HdFlags {
    fn from_u16(value: u16) -> Self {
        HdFlags {
            start_angle_valid: (value & 0x01) != 0,
            start_distance_valid: (value & 0x02) != 0,
        }
    }
}

/// Time quality class for the recording timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeClass {
    /// Local PC time (default).
    LocalPcTime,
    /// External time source.
    ExternalTime,
    /// External absolute time (GPS, etc.).
    ExternalAbsolute,
}

impl TimeClass {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => TimeClass::LocalPcTime,
            10 => TimeClass::ExternalTime,
            16 => TimeClass::ExternalAbsolute,
            _ => TimeClass::LocalPcTime,
        }
    }
}

/// The header (HD) block containing file-level metadata.
///
/// This block links to data groups, file history, attachments, and events.
#[derive(Debug, Clone)]
pub struct HdBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to first data group block (DG).
    pub dg_first: u64,
    /// Link to first file history block (FH).
    pub fh_first: u64,
    /// Link to first channel hierarchy block (CH).
    pub ch_first: u64,
    /// Link to first attachment block (AT).
    pub at_first: u64,
    /// Link to first event block (EV).
    pub ev_first: u64,
    /// Link to comment/metadata (TX/MD).
    pub md_comment: u64,
    /// Start time in nanoseconds since January 1, 1970 (UTC).
    pub start_time_ns: i64,
    /// Time zone offset in minutes from UTC.
    pub tz_offset_min: i16,
    /// Daylight saving time offset in minutes.
    pub dst_offset_min: i16,
    /// Time quality class.
    pub time_class: TimeClass,
    /// Header flags.
    pub flags: HdFlags,
    /// Reserved byte.
    pub reserved: u8,
    /// Start angle in radians (if valid).
    pub start_angle_rad: f64,
    /// Start distance in meters (if valid).
    pub start_distance_m: f64,
}

impl HdBlock {
    /// Minimum size of the HD block (header + 6 links + data).
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 6 * 8 + 32;

    /// Parses an HD block from raw bytes.
    ///
    /// # Arguments
    /// * `data` - The complete block data including header
    /// * `offset` - The file offset (for error reporting)
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##HD", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "HD",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        // Parse links (starting after the 24-byte header)
        let links_start = BLOCK_HEADER_SIZE;
        let dg_first = read_link(data, links_start)?;
        let fh_first = read_link(data, links_start + 8)?;
        let ch_first = read_link(data, links_start + 16)?;
        let at_first = read_link(data, links_start + 24)?;
        let ev_first = read_link(data, links_start + 32)?;
        let md_comment = read_link(data, links_start + 40)?;

        // Parse data section (after header and links)
        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let start_time_ns = cursor.read_i64::<LittleEndian>()?;
        let tz_offset_min = cursor.read_i16::<LittleEndian>()?;
        let dst_offset_min = cursor.read_i16::<LittleEndian>()?;
        let time_flags = cursor.read_u8()?;
        let time_class = TimeClass::from_u8(time_flags);
        let flags_value = cursor.read_u8()?;
        let flags = HdFlags::from_u16(flags_value as u16);
        let reserved = cursor.read_u8()?;
        let _reserved2 = cursor.read_u8()?;
        let start_angle_rad = cursor.read_f64::<LittleEndian>()?;
        let start_distance_m = cursor.read_f64::<LittleEndian>()?;

        Ok(HdBlock {
            header,
            dg_first,
            fh_first,
            ch_first,
            at_first,
            ev_first,
            md_comment,
            start_time_ns,
            tz_offset_min,
            dst_offset_min,
            time_class,
            flags,
            reserved,
            start_angle_rad,
            start_distance_m,
        })
    }

    /// Returns the recording start time as a Unix timestamp (seconds since epoch).
    pub fn start_time_unix(&self) -> f64 {
        self.start_time_ns as f64 / 1_000_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_id_block() -> Vec<u8> {
        let mut data = vec![0u8; 64];
        data[0..8].copy_from_slice(b"MDF     ");
        data[8..16].copy_from_slice(b"4.20    ");
        data[16..24].copy_from_slice(b"TestProg");
        data[28..30].copy_from_slice(&420u16.to_le_bytes()); // version 4.20
        data
    }

    #[test]
    fn test_id_block_parse() {
        let data = create_test_id_block();
        let id = IdBlock::parse(&data).unwrap();

        assert_eq!(id.version_major(), 4);
        assert_eq!(id.version_minor(), 20);
        assert_eq!(id.format_str(), "4.20");
        assert!(id.is_finalized());
    }

    #[test]
    fn test_id_block_invalid_signature() {
        let mut data = create_test_id_block();
        data[0..8].copy_from_slice(b"INVALID ");

        let result = IdBlock::parse(&data);
        assert!(matches!(result, Err(Mf4Error::InvalidSignature(_))));
    }

    #[test]
    fn test_id_block_unfinished() {
        let mut data = create_test_id_block();
        data[0..8].copy_from_slice(b"UnFinMF ");
        data[60..62].copy_from_slice(&1u16.to_le_bytes()); // unfinalized

        let id = IdBlock::parse(&data).unwrap();
        assert!(!id.is_finalized());
    }

    fn create_test_hd_block() -> Vec<u8> {
        let mut data = vec![0u8; 104];

        // Header
        data[0..4].copy_from_slice(b"##HD");
        data[8..16].copy_from_slice(&104u64.to_le_bytes()); // length
        data[16..24].copy_from_slice(&6u64.to_le_bytes()); // link_count

        // Links (6 x 8 bytes, starting at offset 24)
        data[24..32].copy_from_slice(&200u64.to_le_bytes()); // dg_first
        data[32..40].copy_from_slice(&300u64.to_le_bytes()); // fh_first
        data[40..48].copy_from_slice(&0u64.to_le_bytes()); // ch_first
        data[48..56].copy_from_slice(&0u64.to_le_bytes()); // at_first
        data[56..64].copy_from_slice(&0u64.to_le_bytes()); // ev_first
        data[64..72].copy_from_slice(&400u64.to_le_bytes()); // md_comment

        // Data section (starting at offset 72)
        let start_time_ns: i64 = 1_640_000_000_000_000_000; // some timestamp
        data[72..80].copy_from_slice(&start_time_ns.to_le_bytes());
        // Rest is zeros which is fine for defaults

        data
    }

    #[test]
    fn test_hd_block_parse() {
        let data = create_test_hd_block();
        let hd = HdBlock::parse(&data, 64).unwrap();

        assert_eq!(hd.header.block_type, *b"##HD");
        assert_eq!(hd.dg_first, 200);
        assert_eq!(hd.fh_first, 300);
        assert_eq!(hd.md_comment, 400);
    }
}
