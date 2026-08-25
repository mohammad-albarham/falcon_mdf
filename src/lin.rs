//! LIN frames out of bus-logged groups.
//!
//! The counterpart to [`crate::bus`] for LIN traffic: a group a LIN logger
//! wrote holds one composed `LIN_Frame` channel per field — timestamp,
//! identifier, payload — and this module reads them back as frames.

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup};
use crate::Mf4File;

/// The name a LIN frame group composes its fields under.
///
/// The frame fields appear as channels named `LIN_Frame.ID`,
/// `LIN_Frame.DataBytes` and so on — the same arrangement `CAN_DataFrame`
/// uses (see [`crate::bus`]), under its own prefix. `qualify_channel_name`
/// has already reconciled where the prefix is stored by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to
/// look for.
const LIN_PREFIX: &str = "LIN_Frame";

/// The six bits of the identifier field that hold the identifier itself.
///
/// LIN identifiers are 6 bits; a writer that logs the protected identifier
/// (parity bits included) sets bits 6 and 7, and they are not part of the
/// number.
const ID_MASK: u32 = 0x3F;

/// One logged LIN frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinFrame<'a> {
    /// Time of the frame, in the group master's units (seconds, in practice).
    pub timestamp: f64,
    /// The frame identifier, 0..=63.
    pub id: u8,
    /// Which bus the frame was logged from, or 0 when the file does not say.
    pub bus_channel: u8,
    /// The payload, trimmed to the logged length.
    pub data: &'a [u8],
}

/// Every LIN frame of one channel group, in logging order.
pub struct LinFrames {
    /// Frame timestamps.
    pub(crate) timestamps: Vec<f64>,
    /// Frame identifiers.
    pub(crate) ids: Vec<u8>,
    /// Bus channel per frame; empty when the file does not record one.
    pub(crate) bus_channels: Vec<u8>,
    /// Payload bytes, concatenated.
    pub(crate) payloads: Vec<u8>,
    /// Where each frame's payload starts in `payloads`, with a final entry
    /// holding the total length.
    pub(crate) payload_starts: Vec<usize>,
}

impl LinFrames {
    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns true when the group logged no frames.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Returns the frame at `index`, or `None` when it is past the end.
    pub fn get(&self, index: usize) -> Option<LinFrame<'_>> {
        let id = *self.ids.get(index)?;
        let from = *self.payload_starts.get(index)?;
        let to = *self.payload_starts.get(index + 1)?;

        Some(LinFrame {
            timestamp: *self.timestamps.get(index)?,
            id,
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            data: self.payloads.get(from..to)?,
        })
    }

    /// Returns an iterator over every frame.
    pub fn iter(&self) -> impl Iterator<Item = LinFrame<'_>> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

fn field<'a>(group: &'a ChannelGroup, suffix: &str) -> Option<&'a Channel> {
    let qualified = format!("{LIN_PREFIX}.{suffix}");
    group.find_channel(&qualified)
}

fn require<'a>(group: &'a ChannelGroup, suffix: &str) -> Result<&'a Channel> {
    field(group, suffix).ok_or_else(|| Mf4Error::ChannelNotFound {
        name: format!("{LIN_PREFIX}.{suffix}"),
    })
}

/// Reads a scalar frame field as integers.
///
/// Every frame field is a small unsigned integer, so `f64` represents each
/// one exactly and one path covers all of their widths.
fn scalars(file: &Mf4File, channel: &Channel) -> Result<Vec<f64>> {
    Ok(file.signal(channel)?.values()?.to_f64())
}

/// Reads every LIN frame `group` logged.
pub(crate) fn read_lin_frames(file: &Mf4File, group: &ChannelGroup) -> Result<LinFrames> {
    let id_channel = require(group, "ID")?;
    let length_channel = require(group, "DataLength")?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let ids: Vec<u8> = scalars(file, id_channel)?
        .into_iter()
        .map(|id| (id as u32 & ID_MASK) as u8)
        .collect();
    let timestamps = scalars(file, master)?;
    let lengths: Vec<u8> = scalars(file, length_channel)?
        .into_iter()
        .map(|len| len as u8)
        .collect();
    let payload_values = file.signal(payload_channel)?.values()?;

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // identifier with another's payload.
    if timestamps.len() != ids.len()
        || lengths.len() != ids.len()
        || payload_values.len() != ids.len()
    {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} identifiers, {} timestamps, {} lengths and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            ids.len(),
            timestamps.len(),
            lengths.len(),
            payload_values.len()
        )));
    }

    let bus_channels = match field(group, "BusChannel") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|bus| bus as u8)
            .collect(),
        None => Vec::new(),
    };

    // The payloads are flattened here rather than kept as the decoded
    // `SignalValues`: each one is trimmed to the length the record logged,
    // and a fixed-width DataBytes channel pads short frames out to its full
    // width. `starts` has one entry per frame plus a final total, like
    // `SignalValues::VarBytes`.
    let mut payloads = Vec::new();
    let mut payload_starts = Vec::with_capacity(ids.len() + 1);
    for (index, &len) in lengths.iter().enumerate() {
        let sample = payload_values.bytes_at(index).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "payload channel '{LIN_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
                group.acquisition_name
            ))
        })?;
        payload_starts.push(payloads.len());
        payloads.extend_from_slice(&sample[..usize::from(len).min(sample.len())]);
    }
    payload_starts.push(payloads.len());

    Ok(LinFrames {
        timestamps,
        ids,
        bus_channels,
        payloads,
        payload_starts,
    })
}

/// Returns true if `group` holds LIN frames this module can read.
pub(crate) fn is_lin_frame_group(group: &ChannelGroup) -> bool {
    field(group, "ID").is_some() && field(group, "DataBytes").is_some()
}

/// Decodes LIN traffic from a file against a bus database.
pub(crate) fn decode_lin_signals<'a>(
    file: &Mf4File,
    database: &'a crate::candb::CanDatabase,
) -> Result<crate::bus::BusSignals<'a>> {
    let mut accumulator = crate::bus::Accumulator::new(database);
    for group in file.lin_frame_groups() {
        for frame in file.lin_frames(group)?.iter() {
            accumulator.push(crate::bus::CanFrame {
                timestamp: frame.timestamp,
                id: frame.id as u32,
                extended: Some(false),
                bus_channel: frame.bus_channel,
                data: frame.data,
            });
        }
    }
    Ok(accumulator.finish())
}
