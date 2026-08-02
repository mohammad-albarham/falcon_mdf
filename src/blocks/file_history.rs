//! File History (FH) block parsing.
//!
//! Every MF4 file records who wrote it and when, as a chain of FH blocks hanging
//! off the header. The first entry is the file's creation; later ones describe
//! each modification. Each carries a timestamp and a metadata block naming the
//! tool responsible.
//!
//! The standard requires at least one entry, so a file with none is malformed
//! rather than merely uninformative.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Size of the FH block's data section: a `u64` timestamp, two `i16` offsets, a
/// flags byte and three reserved bytes.
const FH_DATA_SIZE: usize = 8 + 2 + 2 + 1 + 3;

/// The File History (FH) block.
#[derive(Debug, Clone)]
pub struct FhBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the next entry in the history chain, or 0 at the end.
    pub fh_next: u64,
    /// Link to the metadata block describing this entry.
    pub md_comment: u64,
    /// When the entry was recorded, in nanoseconds since the epoch.
    pub time_ns: u64,
    /// Timezone offset in minutes, when the time flags say it is valid.
    pub tz_offset_min: i16,
    /// Daylight-saving offset in minutes, when the time flags say it is valid.
    pub dst_offset_min: i16,
    /// Flags describing which parts of the timestamp are meaningful.
    pub time_flags: u8,
}

impl FhBlock {
    /// Minimum size of the FH block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 2 * 8 + FH_DATA_SIZE as u64;
}

impl ParseBlock for FhBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##FH", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "FH",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        let links_start = BLOCK_HEADER_SIZE;
        let fh_next = read_link(data, links_start)?;
        let md_comment = read_link(data, links_start + 8)?;

        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < FH_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                FH_DATA_SIZE,
                data_section.len(),
            ));
        }

        let mut cursor = Cursor::new(data_section);
        let time_ns = cursor.read_u64::<LittleEndian>()?;
        let tz_offset_min = cursor.read_i16::<LittleEndian>()?;
        let dst_offset_min = cursor.read_i16::<LittleEndian>()?;
        let time_flags = cursor.read_u8()?;

        Ok(FhBlock {
            header,
            fh_next,
            md_comment,
            time_ns,
            tz_offset_min,
            dst_offset_min,
            time_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an FH block at the offsets the standard specifies.
    fn create_test_fh_block() -> Vec<u8> {
        let links = 2usize;
        let total_len = BLOCK_HEADER_SIZE + links * 8 + FH_DATA_SIZE;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##FH");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&(links as u64).to_le_bytes());

        data[24..32].copy_from_slice(&700u64.to_le_bytes()); // fh_next
        data[32..40].copy_from_slice(&800u64.to_le_bytes()); // md_comment

        let d = BLOCK_HEADER_SIZE + links * 8;
        data[d..d + 8].copy_from_slice(&1_689_868_948_000_000_000u64.to_le_bytes());
        data[d + 8..d + 10].copy_from_slice(&60i16.to_le_bytes()); // tz
        data[d + 10..d + 12].copy_from_slice(&(-30i16).to_le_bytes()); // dst
        data[d + 12] = 1; // time flags
                          // d+13..d+16 reserved

        data
    }

    #[test]
    fn parses_every_field_at_its_specified_offset() {
        let fh = FhBlock::parse(&create_test_fh_block(), 0).unwrap();
        assert_eq!(fh.fh_next, 700);
        assert_eq!(fh.md_comment, 800);
        assert_eq!(fh.time_ns, 1_689_868_948_000_000_000);
        assert_eq!(fh.tz_offset_min, 60);
        assert_eq!(fh.dst_offset_min, -30);
        assert_eq!(fh.time_flags, 1);
    }

    #[test]
    fn offsets_are_signed() {
        // A timezone west of UTC is negative; reading these unsigned would turn
        // an hour behind into eighteen hours ahead.
        let mut data = create_test_fh_block();
        let d = BLOCK_HEADER_SIZE + 2 * 8;
        data[d + 8..d + 10].copy_from_slice(&(-300i16).to_le_bytes());
        let fh = FhBlock::parse(&data, 0).unwrap();
        assert_eq!(fh.tz_offset_min, -300);
    }

    #[test]
    fn rejects_a_block_of_the_wrong_type() {
        let mut data = create_test_fh_block();
        data[0..4].copy_from_slice(b"##DG");
        assert!(FhBlock::parse(&data, 0).is_err());
    }

    #[test]
    fn rejects_a_block_too_short_for_its_data_section() {
        let mut data = create_test_fh_block();
        let short = (data.len() - 1) as u64;
        data[8..16].copy_from_slice(&short.to_le_bytes());
        data.truncate(data.len() - 1);
        assert!(FhBlock::parse(&data, 0).is_err());
    }
}
