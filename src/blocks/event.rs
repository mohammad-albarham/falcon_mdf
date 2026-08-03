//! Event (EV) block parsing.
//!
//! EV blocks mark discrete moments in a measurement: trigger points, markers,
//! recording start/stop, or external sync events. They carry a timestamp
//! (relative to the HD block's start time) and optional range information.
//!
//! The HD block links the first EV block; the rest form a chain via `ev_next`.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::error::{Mf4Error, Result};

/// Size of the EV block's data section: five single-byte fields, three reserved
/// bytes, a `u32` scope count, two `u16` counts, an `i64` and an `f64`.
const EV_DATA_SIZE: usize = 5 + 3 + 4 + 2 + 2 + 8 + 8;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Event type, from the EV block's `ev_type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    /// Recording event (`ev_type = 0`).
    Recording,
    /// Recording interrupt (`ev_type = 1`).
    RecordingInterrupt,
    /// External start event (`ev_type = 2`).
    ExternalStart,
    /// External stop event (`ev_type = 3`).
    ExternalStop,
    /// Trigger event (`ev_type = 4`).
    Trigger,
    /// Marker event (`ev_type = 5`).
    Marker,
    /// Unknown event type.
    Unknown(u8),
}

impl EventType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => EventType::Recording,
            1 => EventType::RecordingInterrupt,
            2 => EventType::ExternalStart,
            3 => EventType::ExternalStop,
            4 => EventType::Trigger,
            5 => EventType::Marker,
            v => EventType::Unknown(v),
        }
    }
}

/// Synchronization domain for an event's timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvSyncType {
    /// Time in seconds (`ev_sync_type = 0`).
    Time,
    /// Angle in radians (`ev_sync_type = 1`).
    Angle,
    /// Distance in meters (`ev_sync_type = 2`).
    Distance,
    /// Sample index (`ev_sync_type = 3`).
    Index,
    /// Unknown sync type.
    Unknown(u8),
}

impl EvSyncType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => EvSyncType::Time,
            1 => EvSyncType::Angle,
            2 => EvSyncType::Distance,
            3 => EvSyncType::Index,
            v => EvSyncType::Unknown(v),
        }
    }
}

/// Range type for events that span an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvRangeType {
    /// A single point in time (`ev_range_type = 0`).
    Point,
    /// The beginning of a range (`ev_range_type = 1`).
    Begin,
    /// The end of a range (`ev_range_type = 2`).
    End,
    /// Unknown range type.
    Unknown(u8),
}

impl EvRangeType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => EvRangeType::Point,
            1 => EvRangeType::Begin,
            2 => EvRangeType::End,
            v => EvRangeType::Unknown(v),
        }
    }
}

/// What caused the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvCause {
    /// Other cause (`ev_cause = 0`).
    Other,
    /// Error condition (`ev_cause = 1`).
    Error,
    /// Tool-generated (`ev_cause = 2`).
    Tool,
    /// Script-generated (`ev_cause = 3`).
    Script,
    /// User-initiated (`ev_cause = 4`).
    User,
    /// Unknown cause.
    Unknown(u8),
}

impl EvCause {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => EvCause::Other,
            1 => EvCause::Error,
            2 => EvCause::Tool,
            3 => EvCause::Script,
            4 => EvCause::User,
            v => EvCause::Unknown(v),
        }
    }
}

/// The Event (EV) block.
///
/// Marks a discrete event in the measurement with a timestamp and optional
/// range information.
#[derive(Debug, Clone)]
pub struct EvBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to the next EV block (0 = none).
    pub ev_next: u64,
    /// Link to a parent EV block (0 = none).
    pub ev_parent: u64,
    /// Link to the EV block beginning this event's range (0 = none).
    ///
    /// An **event block**, not text. An event that marks a span points here at
    /// the event that opened it. Reading it as a name is what made this parser
    /// refuse a file whose events use ranges — the link resolves to `##EV`, and
    /// a text block was demanded.
    pub ev_range_start: u64,
    /// Link to this event's name (TX block).
    pub tx_name: u64,
    /// Link to a comment (TX or MD block).
    pub md_comment: u64,
    /// Event type.
    pub ev_type: EventType,
    /// Synchronization domain for the timestamp.
    pub ev_sync_type: EvSyncType,
    /// Range type (point, begin, end).
    pub ev_range_type: EvRangeType,
    /// What caused the event.
    pub ev_cause: EvCause,
    /// Event flags.
    pub ev_flags: u8,
    /// Number of channel-group or channel scopes this event applies to.
    pub ev_scope_count: u32,
    /// Number of attachments referenced by this event.
    pub ev_attachment_count: u16,
    /// Index of the tool that created the event.
    pub ev_creator_index: u16,
    /// Start timestamp/position, in the sync domain's units.
    /// Synchronisation base value, in units of `ev_sync_factor`.
    pub ev_sync_base_value: i64,
    /// End timestamp/position for range events. Equal to `ev_dt_start` for
    /// Factor converting the base value to the synchronisation domain.
    pub ev_sync_factor: f64,
}

impl EvBlock {
    /// Minimum size of the EV block (header + 4 links + data section).
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 4 * 8 + EV_DATA_SIZE as u64;
}

impl ParseBlock for EvBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##EV", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "EV",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        let links_start = BLOCK_HEADER_SIZE;
        let ev_next = read_link(data, links_start)?;
        let ev_parent = read_link(data, links_start + 8)?;
        let ev_range_start = read_link(data, links_start + 16)?;
        let tx_name = read_link(data, links_start + 24)?;
        let md_comment = read_link(data, links_start + 32)?;

        let data_start = header.data_offset();
        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;

        if data_section.len() < EV_DATA_SIZE {
            return Err(Mf4Error::truncated(
                offset,
                EV_DATA_SIZE,
                data_section.len(),
            ));
        }

        // Field order and widths per the standard: five single-byte fields,
        // three reserved bytes, then the counts and the synchronisation pair.
        // `ev_flags` is one byte, not two — reading it wider shifts every field
        // after it, which is how this block was previously misread.
        let mut cursor = Cursor::new(data_section);
        let ev_type = EventType::from_u8(cursor.read_u8()?);
        let ev_sync_type = EvSyncType::from_u8(cursor.read_u8()?);
        let ev_range_type = EvRangeType::from_u8(cursor.read_u8()?);
        let ev_cause = EvCause::from_u8(cursor.read_u8()?);
        let ev_flags = cursor.read_u8()?;
        let mut reserved = [0u8; 3];
        std::io::Read::read_exact(&mut cursor, &mut reserved)?;
        let ev_scope_count = cursor.read_u32::<LittleEndian>()?;
        let ev_attachment_count = cursor.read_u16::<LittleEndian>()?;
        let ev_creator_index = cursor.read_u16::<LittleEndian>()?;
        let ev_sync_base_value = cursor.read_i64::<LittleEndian>()?;
        let ev_sync_factor = cursor.read_f64::<LittleEndian>()?;

        Ok(EvBlock {
            header,
            ev_next,
            ev_parent,
            ev_range_start,
            tx_name,
            md_comment,
            ev_type,
            ev_sync_type,
            ev_range_type,
            ev_cause,
            ev_flags,
            ev_scope_count,
            ev_attachment_count,
            ev_creator_index,
            ev_sync_base_value,
            ev_sync_factor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an EV block laid out as the standard specifies: four links, then
    /// five single-byte fields, three reserved bytes, a `u32` scope count, two
    /// `u16` counts, an `i64` base value and an `f64` factor.
    fn create_test_ev_block() -> Vec<u8> {
        // Five fixed links: next, parent, range start, name, comment. The
        // fixture carried four while the parser read four, so both agreed on a
        // layout the standard does not have — and the name link was read as the
        // comment. See the field docs on `EvBlock`.
        let links = 5usize;
        let total_len = BLOCK_HEADER_SIZE + links * 8 + EV_DATA_SIZE;
        let mut data = vec![0u8; total_len];

        data[0..4].copy_from_slice(b"##EV");
        data[8..16].copy_from_slice(&(total_len as u64).to_le_bytes());
        data[16..24].copy_from_slice(&(links as u64).to_le_bytes());

        data[24..32].copy_from_slice(&100u64.to_le_bytes()); // ev_next
        data[32..40].copy_from_slice(&0u64.to_le_bytes()); // ev_parent
        data[40..48].copy_from_slice(&200u64.to_le_bytes()); // ev_range_start
        data[48..56].copy_from_slice(&300u64.to_le_bytes()); // tx_name
        data[56..64].copy_from_slice(&400u64.to_le_bytes()); // md_comment

        let d = BLOCK_HEADER_SIZE + links * 8;
        data[d] = 4; // ev_type = Trigger
        data[d + 1] = 0; // ev_sync_type = Time
        data[d + 2] = 0; // ev_range_type = Point
        data[d + 3] = 4; // ev_cause = User
        data[d + 4] = 1; // ev_flags — one byte, not two
                         // d+5..d+8 reserved
        data[d + 8..d + 12].copy_from_slice(&2u32.to_le_bytes()); // scope count
        data[d + 12..d + 14].copy_from_slice(&1u16.to_le_bytes()); // attachments
        data[d + 14..d + 16].copy_from_slice(&7u16.to_le_bytes()); // creator
        data[d + 16..d + 24].copy_from_slice(&1_500_000i64.to_le_bytes());
        data[d + 24..d + 32].copy_from_slice(&1e-9f64.to_le_bytes());

        data
    }

    #[test]
    fn parses_every_field_at_its_specified_offset() {
        let ev = EvBlock::parse(&create_test_ev_block(), 1000).unwrap();

        assert_eq!(ev.ev_next, 100);
        assert_eq!(ev.ev_parent, 0);
        assert_eq!(ev.ev_range_start, 200);
        assert_eq!(ev.tx_name, 300);
        assert_eq!(ev.md_comment, 400);

        assert_eq!(ev.ev_flags, 1);
        assert_eq!(ev.ev_scope_count, 2);
        assert_eq!(ev.ev_attachment_count, 1);
        assert_eq!(ev.ev_creator_index, 7);
        assert_eq!(ev.ev_sync_base_value, 1_500_000);
        assert_eq!(ev.ev_sync_factor, 1e-9);
    }

    #[test]
    fn a_one_byte_flags_field_does_not_shift_the_fields_after_it() {
        // Reading `ev_flags` as a `u16` — as this parser once did — leaves every
        // later field one byte out. The scope count is the first to show it.
        let ev = EvBlock::parse(&create_test_ev_block(), 0).unwrap();
        assert_eq!(
            ev.ev_scope_count, 2,
            "scope count is wrong, so the fields before it are mis-sized"
        );
        assert_eq!(
            ev.ev_sync_factor, 1e-9,
            "the tail of the block is misaligned"
        );
    }

    #[test]
    fn rejects_a_block_of_the_wrong_type() {
        let mut data = create_test_ev_block();
        data[0..4].copy_from_slice(b"##DG");
        assert!(EvBlock::parse(&data, 0).is_err());
    }

    #[test]
    fn rejects_a_block_too_short_for_its_data_section() {
        let mut data = create_test_ev_block();
        let short = (data.len() - 1) as u64;
        data[8..16].copy_from_slice(&short.to_le_bytes());
        data.truncate(data.len() - 1);
        assert!(
            EvBlock::parse(&data, 0).is_err(),
            "a block shorter than its fields must be rejected, not read past"
        );
    }
}
