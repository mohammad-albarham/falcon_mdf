//! MDF 3.x block structures.
//!
//! Version 3 files are laid out quite differently from version 4. Blocks are
//! identified by two ASCII characters rather than `##XX`, their fields sit at
//! fixed offsets rather than behind a link list, and the byte order for the
//! whole file is declared once in the identification block. Nothing here is
//! shared with the v4 parser, and deliberately so: bending one into the other
//! would make both harder to read than writing the second one out.

use crate::error::{Mf4Error, Result};

/// Reads a little-endian `u16` at `off`, or fails naming the block.
fn u16_at(data: &[u8], off: usize, block: &'static str) -> Result<u16> {
    let end = off + 2;
    if end > data.len() {
        return Err(short(block, off, 2, data.len()));
    }
    Ok(u16::from_le_bytes([data[off], data[off + 1]]))
}

/// Reads a little-endian `u32` at `off`, or fails naming the block.
fn u32_at(data: &[u8], off: usize, block: &'static str) -> Result<u32> {
    let end = off + 4;
    if end > data.len() {
        return Err(short(block, off, 4, data.len()));
    }
    Ok(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

/// Reads a little-endian `f64` at `off`, or fails naming the block.
fn f64_at(data: &[u8], off: usize, block: &'static str) -> Result<f64> {
    let end = off + 8;
    if end > data.len() {
        return Err(short(block, off, 8, data.len()));
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[off..end]);
    Ok(f64::from_le_bytes(b))
}

fn short(block: &'static str, off: usize, want: usize, have: usize) -> Mf4Error {
    debug_assert!(!block.is_empty());
    Mf4Error::TruncatedFile {
        offset: off as u64,
        expected: want,
        actual: have.saturating_sub(off.min(have)),
    }
}

/// A fixed-width text field, trimmed at the first NUL and of trailing spaces.
///
/// v3 pads these with NULs, but files in the wild pad with spaces too, and a
/// channel named `"Speed   "` would not match a lookup for `"Speed"`.
fn fixed_text(data: &[u8], off: usize, len: usize) -> String {
    let end = (off + len).min(data.len());
    if off >= end {
        return String::new();
    }
    let raw = &data[off..end];
    let cut = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..cut]).trim_end().to_string()
}

/// The two-character identifier every v3 block after the header carries.
fn expect_id(data: &[u8], want: &[u8; 2], offset: u64) -> Result<()> {
    if data.len() < 2 {
        return Err(Mf4Error::TruncatedFile {
            offset,
            expected: 2,
            actual: data.len(),
        });
    }
    if &data[..2] != want {
        return Err(Mf4Error::InvalidBlockId {
            offset,
            expected: String::from_utf8_lossy(want).to_string(),
            actual: String::from_utf8_lossy(&data[..2]).to_string(),
        });
    }
    Ok(())
}

/// The identification block: the first 64 bytes of every v3 file.
#[derive(Debug, Clone)]
pub struct IdBlock {
    /// The measurement format's own name for itself. Always `"MDF"`.
    pub file_identification: String,
    /// Version as written in the file, e.g. `"3.30"`.
    pub version_text: String,
    /// The program that wrote the file.
    pub program_identification: String,
    /// Version as a number, e.g. `330`.
    pub version_number: u16,
    /// True when the file's numbers are stored big-endian.
    pub big_endian: bool,
    /// The floating point format code. 0 is IEEE 754.
    pub float_format: u16,
    /// Code page for the file's text, 0 when unspecified.
    pub code_page: u16,
}

impl IdBlock {
    /// Size of the identification block, which is fixed.
    pub const SIZE: usize = 64;

    /// Parses the identification block from the start of a file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            return Err(Mf4Error::TruncatedFile {
                offset: 0,
                expected: Self::SIZE,
                actual: data.len(),
            });
        }

        let file_identification = fixed_text(data, 0, 8);
        if file_identification != "MDF" {
            return Err(Mf4Error::InvalidSignature(format!(
                "expected an MDF file identification, found {file_identification:?}"
            )));
        }

        let byte_order = u16_at(data, 24, "ID")?;
        Ok(Self {
            file_identification,
            version_text: fixed_text(data, 8, 8),
            program_identification: fixed_text(data, 16, 8),
            big_endian: byte_order != 0,
            float_format: u16_at(data, 26, "ID")?,
            version_number: u16_at(data, 28, "ID")?,
            code_page: u16_at(data, 30, "ID")?,
        })
    }
}

/// The header block: what the measurement is and when it was taken.
#[derive(Debug, Clone)]
pub struct HdBlock {
    /// Address of the first data group.
    pub first_dg_addr: u32,
    /// Address of the file comment, 0 when there is none.
    pub comment_addr: u32,
    /// Address of the program block, 0 when there is none.
    pub program_addr: u32,
    /// Number of data groups the header declares.
    pub dg_count: u16,
    /// Recording date as written, `DD:MM:YYYY`.
    pub date: String,
    /// Recording time as written, `HH:MM:SS`.
    pub time: String,
    /// Who recorded it.
    pub author: String,
    /// Which department.
    pub department: String,
    /// Which project.
    pub project: String,
    /// What was measured.
    pub subject: String,
    /// Nanoseconds since the epoch. Only written from 3.20 onwards.
    pub abs_time: Option<u64>,
    /// Timezone offset in minutes. Only written from 3.20 onwards.
    pub tz_offset_minutes: Option<i16>,
}

impl HdBlock {
    /// Size of the fields every version writes.
    pub const COMMON_SIZE: usize = 164;
    /// Size of the extra fields 3.20 and later add.
    pub const POST_320_EXTRA_SIZE: usize = 44;

    /// Parses the header block, which always sits directly after the
    /// identification block.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        expect_id(data, b"HD", offset)?;
        if data.len() < Self::COMMON_SIZE {
            return Err(short("HD", 0, Self::COMMON_SIZE, data.len()));
        }

        let block_len = u16_at(data, 2, "HD")? as usize;
        let mut hd = Self {
            first_dg_addr: u32_at(data, 4, "HD")?,
            comment_addr: u32_at(data, 8, "HD")?,
            program_addr: u32_at(data, 12, "HD")?,
            dg_count: u16_at(data, 16, "HD")?,
            date: fixed_text(data, 18, 10),
            time: fixed_text(data, 28, 8),
            author: fixed_text(data, 36, 32),
            department: fixed_text(data, 68, 32),
            project: fixed_text(data, 100, 32),
            subject: fixed_text(data, 132, 32),
            abs_time: None,
            tz_offset_minutes: None,
        };

        // The absolute timestamp arrived in 3.20. Its presence is told by the
        // block's own declared length, not by the file version: a 3.20 writer
        // is allowed to emit the shorter header, and one that does is not
        // malformed.
        if block_len >= Self::COMMON_SIZE + Self::POST_320_EXTRA_SIZE
            && data.len() >= Self::COMMON_SIZE + 10
        {
            let mut b = [0u8; 8];
            b.copy_from_slice(&data[164..172]);
            let abs = u64::from_le_bytes(b);
            if abs != 0 {
                hd.abs_time = Some(abs);
            }
            hd.tz_offset_minutes = Some(u16_at(data, 172, "HD")? as i16);
        }

        Ok(hd)
    }
}

/// A data group: one record stream and the channel groups that share it.
#[derive(Debug, Clone)]
pub struct DgBlock {
    /// Address of the next data group, 0 at the end of the chain.
    pub next_dg_addr: u32,
    /// Address of this group's first channel group.
    pub first_cg_addr: u32,
    /// Address of the trigger block, 0 when there is none.
    pub trigger_addr: u32,
    /// Address of this group's records.
    pub data_block_addr: u32,
    /// How many channel groups share the record stream.
    pub cg_count: u16,
    /// How many copies of the record identifier each record carries.
    ///
    /// This is a *count of identifiers*, not a width: the identifier itself is
    /// always one byte. Zero means the group holds a single channel group and
    /// writes no identifier, one means an identifier before each record, and
    /// two means one before and a copy after. Reading it as a byte width would
    /// mis-align every record in a group that uses the trailing copy.
    pub record_id_count: u16,
}

impl DgBlock {
    /// Size before 3.20.
    pub const SIZE_PRE_320: usize = 24;
    /// Size from 3.20 onwards, which adds a reserved word.
    pub const SIZE_POST_320: usize = 28;

    /// Parses a data group block.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        expect_id(data, b"DG", offset)?;
        if data.len() < Self::SIZE_PRE_320 {
            return Err(short("DG", 0, Self::SIZE_PRE_320, data.len()));
        }
        let record_id_count = u16_at(data, 22, "DG")?;

        // The format defines exactly three values here. Anything else is a
        // count this reader cannot turn into a record stride, and guessing one
        // would mis-align every record in the group.
        if record_id_count > 2 {
            return Err(Mf4Error::ParseError {
                message: format!(
                    "data group declares {record_id_count} record identifiers per record; \
                     the format allows 0, 1 or 2"
                ),
            });
        }

        Ok(Self {
            next_dg_addr: u32_at(data, 4, "DG")?,
            first_cg_addr: u32_at(data, 8, "DG")?,
            trigger_addr: u32_at(data, 12, "DG")?,
            data_block_addr: u32_at(data, 16, "DG")?,
            cg_count: u16_at(data, 20, "DG")?,
            record_id_count,
        })
    }
}

/// A channel group: the record layout shared by a set of channels.
#[derive(Debug, Clone)]
pub struct CgBlock {
    /// Address of the next channel group, 0 at the end of the chain.
    pub next_cg_addr: u32,
    /// Address of this group's first channel.
    pub first_ch_addr: u32,
    /// Address of the group comment, 0 when there is none.
    pub comment_addr: u32,
    /// The identifier prefixing this group's records in an unsorted group.
    pub record_id: u16,
    /// How many channels the group declares.
    pub channel_count: u16,
    /// Size of one record in bytes, excluding any record identifier.
    pub record_size: u16,
    /// How many records the group declares.
    pub cycle_count: u32,
}

impl CgBlock {
    /// Size of a channel group block.
    pub const SIZE: usize = 26;

    /// Parses a channel group block.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        expect_id(data, b"CG", offset)?;
        if data.len() < Self::SIZE {
            return Err(short("CG", 0, Self::SIZE, data.len()));
        }
        Ok(Self {
            next_cg_addr: u32_at(data, 4, "CG")?,
            first_ch_addr: u32_at(data, 8, "CG")?,
            comment_addr: u32_at(data, 12, "CG")?,
            record_id: u16_at(data, 16, "CG")?,
            channel_count: u16_at(data, 18, "CG")?,
            record_size: u16_at(data, 20, "CG")?,
            cycle_count: u32_at(data, 22, "CG")?,
        })
    }
}

/// What a v3 channel is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mdf3ChannelType {
    /// An ordinary measured channel.
    Data,
    /// The group's master, which in v3 is always time.
    Time,
    /// A type this build does not recognise, kept so it can be reported.
    Unknown(u16),
}

impl From<u16> for Mdf3ChannelType {
    fn from(v: u16) -> Self {
        match v {
            0 => Self::Data,
            1 => Self::Time,
            other => Self::Unknown(other),
        }
    }
}

/// A channel: one measured quantity within a record.
#[derive(Debug, Clone)]
pub struct CnBlock {
    /// Address of the next channel, 0 at the end of the chain.
    pub next_ch_addr: u32,
    /// Address of the conversion, 0 when the values are already physical.
    pub conversion_addr: u32,
    /// Address of the acquisition source, 0 when unrecorded.
    pub source_addr: u32,
    /// Address of the channel dependency block, 0 when there is none.
    pub dependency_addr: u32,
    /// Address of the channel comment, 0 when there is none.
    pub comment_addr: u32,
    /// Whether this is data or the group's time channel.
    pub channel_type: Mdf3ChannelType,
    /// The 32-byte name. Superseded by `long_name` when one is present.
    pub short_name: String,
    /// The channel's description.
    pub description: String,
    /// Offset in bits from the start of the record.
    pub start_offset: u16,
    /// Width of the channel in bits.
    pub bit_count: u16,
    /// The raw signal data type code.
    pub data_type: u16,
    /// Whether the min and max below were actually written.
    pub range_valid: bool,
    /// Smallest raw value seen, when the writer recorded one.
    pub min_raw_value: f64,
    /// Largest raw value seen, when the writer recorded one.
    pub max_raw_value: f64,
    /// Sampling rate in seconds, 0 when unrecorded.
    pub sampling_rate: f64,
    /// Address of the long name, present only in the longer block forms.
    pub long_name_addr: u32,
    /// Address of the display name, present only in the longest form.
    pub display_name_addr: u32,
    /// Whole bytes to add to [`Self::start_offset`] before using it.
    ///
    /// `start_offset` is 16 bits, so on its own it cannot address past byte
    /// 8191 of a record. Records longer than that carry the rest here, and a
    /// reader that ignores it decodes a channel from the wrong part of the
    /// record without noticing. Present only in the longest block form.
    pub additional_byte_offset: u16,
}

impl CnBlock {
    /// The original block, with a 32-character name.
    pub const SIZE_SHORT: usize = 218;
    /// With a link to a name longer than 32 characters.
    pub const SIZE_LONG_NAME: usize = 222;
    /// With a display name as well.
    pub const SIZE_DISPLAY_NAME: usize = 228;

    /// Parses a channel block. Which of the three forms it is is told by the
    /// block's declared length.
    pub fn parse(data: &[u8], offset: u64) -> Result<Self> {
        expect_id(data, b"CN", offset)?;
        if data.len() < Self::SIZE_SHORT {
            return Err(short("CN", 0, Self::SIZE_SHORT, data.len()));
        }
        let block_len = u16_at(data, 2, "CN")? as usize;

        let bit_count = u16_at(data, 188, "CN")?;
        // A zero-width channel has no values to decode, and a width past the
        // record cannot be read. Both are rejected here rather than producing
        // an empty or overrunning read later.
        if bit_count == 0 {
            return Err(Mf4Error::ParseError {
                message: "channel declares a width of zero bits".to_string(),
            });
        }

        let long_name_addr = if block_len >= Self::SIZE_LONG_NAME {
            u32_at(data, 218, "CN")?
        } else {
            0
        };
        let (display_name_addr, additional_byte_offset) = if block_len >= Self::SIZE_DISPLAY_NAME {
            (u32_at(data, 222, "CN")?, u16_at(data, 226, "CN")?)
        } else {
            (0, 0)
        };

        Ok(Self {
            next_ch_addr: u32_at(data, 4, "CN")?,
            conversion_addr: u32_at(data, 8, "CN")?,
            source_addr: u32_at(data, 12, "CN")?,
            dependency_addr: u32_at(data, 16, "CN")?,
            comment_addr: u32_at(data, 20, "CN")?,
            channel_type: Mdf3ChannelType::from(u16_at(data, 24, "CN")?),
            short_name: fixed_text(data, 26, 32),
            description: fixed_text(data, 58, 128),
            start_offset: u16_at(data, 186, "CN")?,
            bit_count,
            data_type: u16_at(data, 190, "CN")?,
            range_valid: u16_at(data, 192, "CN")? != 0,
            min_raw_value: f64_at(data, 194, "CN")?,
            max_raw_value: f64_at(data, 202, "CN")?,
            sampling_rate: f64_at(data, 210, "CN")?,
            long_name_addr,
            display_name_addr,
            additional_byte_offset,
        })
    }
}

/// Reads a `TX` text block's payload.
pub fn parse_tx(data: &[u8], offset: u64) -> Result<String> {
    expect_id(data, b"TX", offset)?;
    let block_len = u16_at(data, 2, "TX")? as usize;
    if block_len < 4 {
        return Err(Mf4Error::InvalidBlockSize {
            block_type: "TX".to_string(),
            size: block_len as u64,
            min_size: 4,
        });
    }
    let end = block_len.min(data.len());
    Ok(fixed_text(data, 4, end.saturating_sub(4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built identification block, so the expected bytes come from the
    /// format rather than from this module's own writer.
    fn id_bytes(version: &str, number: u16, big_endian: bool) -> Vec<u8> {
        let mut v = vec![0u8; IdBlock::SIZE];
        v[..8].copy_from_slice(b"MDF     ");
        let ver = version.as_bytes();
        v[8..8 + ver.len()].copy_from_slice(ver);
        v[16..20].copy_from_slice(b"test");
        v[24..26].copy_from_slice(&(if big_endian { 1u16 } else { 0 }).to_le_bytes());
        v[28..30].copy_from_slice(&number.to_le_bytes());
        v
    }

    #[test]
    fn an_identification_block_reports_its_version_and_byte_order() {
        let id = IdBlock::parse(&id_bytes("3.30", 330, false)).unwrap();
        assert_eq!(id.file_identification, "MDF");
        assert_eq!(id.version_text, "3.30");
        assert_eq!(id.version_number, 330);
        assert!(!id.big_endian);

        let be = IdBlock::parse(&id_bytes("2.14", 214, true)).unwrap();
        assert_eq!(be.version_number, 214);
        assert!(be.big_endian);
    }

    #[test]
    fn a_file_that_does_not_say_mdf_is_refused() {
        let mut v = id_bytes("3.30", 330, false);
        v[..8].copy_from_slice(b"NOTMDF  ");
        assert!(matches!(
            IdBlock::parse(&v),
            Err(Mf4Error::InvalidSignature(_))
        ));
    }

    #[test]
    fn a_file_shorter_than_its_identification_block_is_refused() {
        assert!(matches!(
            IdBlock::parse(&[0u8; 10]),
            Err(Mf4Error::TruncatedFile { .. })
        ));
    }

    /// Builds a data group block declaring `ids` record identifiers per record.
    fn dg_bytes(ids: u16) -> Vec<u8> {
        let mut v = vec![0u8; DgBlock::SIZE_POST_320];
        v[..2].copy_from_slice(b"DG");
        v[2..4].copy_from_slice(&(DgBlock::SIZE_POST_320 as u16).to_le_bytes());
        v[22..24].copy_from_slice(&ids.to_le_bytes());
        v
    }

    #[test]
    fn a_data_group_with_an_impossible_record_id_count_is_refused() {
        // The field counts identifiers, not bytes, and the format defines only
        // 0, 1 and 2. Three would have been accepted while it was read as a
        // width, and every record after the first would have been decoded from
        // the wrong offset.
        for ids in [3u16, 4, 9, 0xFFFF] {
            assert!(
                matches!(
                    DgBlock::parse(&dg_bytes(ids), 64),
                    Err(Mf4Error::ParseError { .. })
                ),
                "{ids} identifiers per record should be refused"
            );
        }
        for ids in [0u16, 1, 2] {
            assert_eq!(
                DgBlock::parse(&dg_bytes(ids), 64).unwrap().record_id_count,
                ids
            );
        }
    }

    #[test]
    fn a_block_carrying_the_wrong_identifier_is_refused() {
        let mut v = vec![0u8; CgBlock::SIZE];
        v[..2].copy_from_slice(b"XX");
        assert!(matches!(
            CgBlock::parse(&v, 128),
            Err(Mf4Error::InvalidBlockId { .. })
        ));
    }

    #[test]
    fn a_channel_of_zero_width_is_refused() {
        let mut v = vec![0u8; CnBlock::SIZE_SHORT];
        v[..2].copy_from_slice(b"CN");
        v[2..4].copy_from_slice(&(CnBlock::SIZE_SHORT as u16).to_le_bytes());
        // bit_count at 188 left as zero
        assert!(matches!(
            CnBlock::parse(&v, 256),
            Err(Mf4Error::ParseError { .. })
        ));
    }

    #[test]
    fn a_fixed_text_field_stops_at_its_nul_and_drops_padding() {
        let mut v = vec![b' '; 32];
        v[..5].copy_from_slice(b"Speed");
        v[10] = 0;
        assert_eq!(fixed_text(&v, 0, 32), "Speed");
    }
}
