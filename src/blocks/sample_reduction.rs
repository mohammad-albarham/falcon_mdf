//! Sample Reduction (SR) block parsing.
//!
//! SR blocks describe pre-computed reduced data for a channel group: minimum,
//! maximum, and mean values over fixed intervals. They allow a viewer to show
//! a downsampled overview without reading the full raw data.
//!
//! A channel group links its first SR block via `cg_sr_first`; the rest form a
//! chain via `sr_next`.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};

/// Size of the SR block's data section: a `u64` cycle count, an `f64` interval,
/// two single-byte fields and six reserved bytes.
const SR_DATA_SIZE: usize = 8 + 8 + 1 + 1 + 6;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Synchronization domain for a sample reduction block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrSyncType {
    /// Time in seconds (`sr_sync_type = 1`).
    Time,
    /// Angle in radians (`sr_sync_type = 2`).
    Angle,
    /// Distance in meters (`sr_sync_type = 3`).
    Distance,
    /// Sample index (`sr_sync_type = 4`).
    Index,
    /// Unknown sync type, including the undefined 0.
    Unknown(u8),
}

impl SrSyncType {
    /// Numbered from 1, like an event's `ev_sync_type` and unlike a channel's
    /// `cn_sync_type`, which spends 0 on "none". A reduction always condenses
    /// over some domain, so 0 is undefined. Numbering these from 0 shifted
    /// every domain by one: a reduction over seconds reported itself as angle.
    fn from_u8(value: u8) -> Self {
        match value {
            1 => SrSyncType::Time,
            2 => SrSyncType::Angle,
            3 => SrSyncType::Distance,
            4 => SrSyncType::Index,
            v => SrSyncType::Unknown(v),
        }
    }
}

/// The Sample Reduction (SR) block.
///
/// Describes pre-computed reduced data for a channel group, allowing a
/// viewer to show an overview without reading the full raw data.
#[derive(Debug, Clone)]
pub struct SrBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the next SR block (0 = none).
    pub sr_next: u64,
    /// Link to the data block (DT or RD) holding the reduced values.
    pub sr_data: u64,
    /// Number of reduced cycles (samples) in this block.
    pub sr_cycle_count: u64,
    /// Interval between reduced samples, in the sync domain's units.
    /// For time, this is seconds.
    pub sr_interval: f64,
    /// Synchronization domain.
    pub sr_sync_type: SrSyncType,
    /// Reduction flags as stored in the block.
    pub sr_flags: u8,
}

impl SrBlock {
    /// Minimum size of the SR block (header + 3 links + data section).
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 2 * 8 + SR_DATA_SIZE as u64;
}

impl ParseBlock for SrBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##SR", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "SR",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        // Two links only. A sample-reduction block carries no comment of its
        // own; reading a third link here consumed the first eight bytes of the
        // data section and shifted every field after it.
        let links_start = BLOCK_HEADER_SIZE;
        let sr_next = read_link(data, links_start)?;
        let sr_data = read_link(data, links_start + 8)?;

        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < SR_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                SR_DATA_SIZE,
                data_section.len(),
            ));
        }

        // The block records how many reduced cycles it holds, over what
        // interval, in which synchronisation domain. It carries no minimum or
        // maximum of its own — those are values in the reduced data.
        let mut cursor = Cursor::new(data_section);
        let sr_cycle_count = cursor.read_u64::<LittleEndian>()?;
        let sr_interval = cursor.read_f64::<LittleEndian>()?;
        let sr_sync_type = SrSyncType::from_u8(cursor.read_u8()?);
        let sr_flags = cursor.read_u8()?;

        Ok(SrBlock {
            header,
            sr_next,
            sr_data,
            sr_cycle_count,
            sr_interval,
            sr_sync_type,
            sr_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an SR block as the standard lays it out: two links, then a `u64`
    /// cycle count, an `f64` interval, two single-byte fields and six reserved
    /// bytes.
    fn create_test_sr_block() -> Vec<u8> {
        let links = 2usize;
        let total_len = BLOCK_HEADER_SIZE + links * 8 + SR_DATA_SIZE;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##SR");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&(links as u64).to_le_bytes());

        data[24..32].copy_from_slice(&500u64.to_le_bytes()); // sr_next
        data[32..40].copy_from_slice(&600u64.to_le_bytes()); // sr_data

        let d = BLOCK_HEADER_SIZE + links * 8;
        data[d..d + 8].copy_from_slice(&1234u64.to_le_bytes()); // cycle count
        data[d + 8..d + 16].copy_from_slice(&0.25f64.to_le_bytes()); // interval
        data[d + 16] = 1; // sync type = seconds
        data[d + 17] = 3; // flags
                          // d+18..d+24 reserved

        data
    }

    #[test]
    fn parses_every_field_at_its_specified_offset() {
        let sr = SrBlock::parse(&create_test_sr_block(), 0).unwrap();
        assert_eq!(sr.sr_next, 500);
        assert_eq!(sr.sr_data, 600);
        assert_eq!(sr.sr_cycle_count, 1234);
        assert_eq!(sr.sr_interval, 0.25);
        assert_eq!(sr.sr_sync_type, SrSyncType::Time);
        assert_eq!(sr.sr_flags, 3);
    }

    /// The numbering comes from the standard, not from `from_u8`. Taken the
    /// other way round it proves nothing: the fixture above picked whichever
    /// value the parser decoded as time, so the two agreed while both were
    /// shifted by one — a reduction over seconds reporting itself as angle.
    #[test]
    fn sync_type_is_numbered_from_one() {
        let spec = [
            (1u8, SrSyncType::Time),
            (2, SrSyncType::Angle),
            (3, SrSyncType::Distance),
            (4, SrSyncType::Index),
        ];

        for (raw, expected) in spec {
            let mut data = create_test_sr_block();
            data[BLOCK_HEADER_SIZE + 2 * 8 + 16] = raw;
            let sr = SrBlock::parse(&data, 0).unwrap();
            assert_eq!(
                sr.sr_sync_type, expected,
                "sr_sync_type {raw} decoded wrongly"
            );
        }

        let mut data = create_test_sr_block();
        data[BLOCK_HEADER_SIZE + 2 * 8 + 16] = 0;
        assert_eq!(
            SrBlock::parse(&data, 0).unwrap().sr_sync_type,
            SrSyncType::Unknown(0),
            "0 is undefined for sr_sync_type; reading it as seconds shifted \
             every domain by one"
        );
    }

    #[test]
    fn a_third_link_would_consume_the_cycle_count() {
        // The block has two links. Reading a third — as this parser once did —
        // takes the first eight bytes of the data section, so the cycle count
        // comes back as whatever followed it.
        let sr = SrBlock::parse(&create_test_sr_block(), 0).unwrap();
        assert_eq!(
            sr.sr_cycle_count, 1234,
            "cycle count is wrong, so the link section is mis-sized"
        );
        assert_eq!(sr.sr_interval, 0.25, "the fields after it are shifted too");
    }

    #[test]
    fn rejects_a_block_of_the_wrong_type() {
        let mut data = create_test_sr_block();
        data[0..4].copy_from_slice(b"##DG");
        assert!(SrBlock::parse(&data, 0).is_err());
    }

    #[test]
    fn rejects_a_block_too_short_for_its_data_section() {
        let mut data = create_test_sr_block();
        let short = (data.len() - 1) as u64;
        data[8..16].copy_from_slice(&short.to_le_bytes());
        data.truncate(data.len() - 1);
        assert!(SrBlock::parse(&data, 0).is_err());
    }
}
