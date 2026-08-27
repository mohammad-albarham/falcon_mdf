//! FlexRay frames out of bus-logged groups.
//!
//! The counterpart to [`crate::bus`] for FlexRay traffic: a group a FlexRay logger
//! wrote holds one composed `FlexRayFrame` channel per field — timestamp,
//! identifier, payload — and this module reads them back as frames.

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup};
use crate::Mf4File;

/// The name a FlexRay frame group composes its fields under.
///
/// The frame fields appear as channels named `FLX_Frame.FrameID`,
/// `FLX_Frame.DataBytes` and so on — the same arrangement `CAN_DataFrame`
/// uses (see [`crate::bus`]), under its own prefix. `qualify_channel_name`
/// has already reconciled where the prefix is stored by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to
/// look for.
const FLX_PREFIX: &str = "FLX_Frame";

/// The 11 bits of the FrameID field that hold the identifier itself.
///
/// A writer may log the protected identifier (CRC and other bits included)
/// in a wider container with other bits set, and those bits are not part of
/// the number.
const FRAME_ID_MASK: u32 = 0x7FF;

/// The 6 bits of the Cycle field that hold the cycle counter.
///
/// A writer may log the cycle counter in a wider container with other bits
/// set, and those bits are not part of the number.
const CYCLE_MASK: u32 = 0x3F;

/// One logged FlexRay frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlexRayFrame<'a> {
    /// Time of the frame, in the group master's units (seconds, in practice).
    pub timestamp: f64,
    /// The frame identifier, 0..=2047 (11 bits).
    pub frame_id: u16,
    /// Cycle counter, 0..=63.
    pub cycle: u8,
    /// Which bus the frame was logged from, or 0 when the file does not say.
    pub bus_channel: u8,
    /// Whether the frame carried no payload.
    pub null_frame: bool,
    /// Whether the frame is a sync frame.
    pub sync_frame: bool,
    /// Whether the frame is a startup frame.
    pub startup: bool,
    /// The payload, trimmed to the logged length.
    pub data: &'a [u8],
}

/// Every FlexRay frame of one channel group, in logging order.
pub struct FlexRayFrames {
    /// Frame timestamps.
    pub(crate) timestamps: Vec<f64>,
    /// Frame identifiers (11-bit masked).
    pub(crate) frame_ids: Vec<u16>,
    /// Cycle counters (6-bit masked).
    pub(crate) cycles: Vec<u8>,
    /// Bus channel per frame; empty when the file does not record one.
    pub(crate) bus_channels: Vec<u8>,
    /// Whether each frame was a null frame.
    pub(crate) null_frames: Vec<bool>,
    /// Whether each frame was a sync frame.
    pub(crate) sync_frames: Vec<bool>,
    /// Whether each frame was a startup frame.
    pub(crate) startup_frames: Vec<bool>,
    /// Payload bytes, concatenated.
    pub(crate) payloads: Vec<u8>,
    /// Where each frame's payload starts in `payloads`, with a final entry
    /// holding the total length.
    pub(crate) payload_starts: Vec<usize>,
}

impl FlexRayFrames {
    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.frame_ids.len()
    }

    /// Returns true when the group logged no frames.
    pub fn is_empty(&self) -> bool {
        self.frame_ids.is_empty()
    }

    /// Returns the frame at `index`, or `None` when it is past the end.
    pub fn get(&self, index: usize) -> Option<FlexRayFrame<'_>> {
        let frame_id = *self.frame_ids.get(index)?;
        let cycle = *self.cycles.get(index)?;
        let from = *self.payload_starts.get(index)?;
        let to = *self.payload_starts.get(index + 1)?;

        Some(FlexRayFrame {
            timestamp: *self.timestamps.get(index)?,
            frame_id,
            cycle,
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            null_frame: *self.null_frames.get(index)?,
            sync_frame: *self.sync_frames.get(index)?,
            startup: *self.startup_frames.get(index)?,
            data: self.payloads.get(from..to)?,
        })
    }

    /// Returns an iterator over every frame.
    pub fn iter(&self) -> impl Iterator<Item = FlexRayFrame<'_>> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

fn field<'a>(group: &'a ChannelGroup, suffix: &str) -> Option<&'a Channel> {
    let qualified = format!("{FLX_PREFIX}.{suffix}");
    group.find_channel(&qualified)
}

fn require<'a>(group: &'a ChannelGroup, suffix: &str) -> Result<&'a Channel> {
    field(group, suffix).ok_or_else(|| Mf4Error::ChannelNotFound {
        name: format!("{FLX_PREFIX}.{suffix}"),
    })
}

/// Reads a scalar frame field as integers.
///
/// Every frame field is a small unsigned integer, so `f64` represents each
/// one exactly and one path covers all of their widths.
fn scalars(file: &Mf4File, channel: &Channel) -> Result<Vec<f64>> {
    Ok(file.signal(channel)?.values()?.to_f64())
}

/// Reads every FlexRay frame `group` logged.
pub(crate) fn read_flexray_frames(file: &Mf4File, group: &ChannelGroup) -> Result<FlexRayFrames> {
    let frame_id_channel = require(group, "FrameID")?;
    let length_channel = require(group, "DataLength")?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let frame_ids: Vec<u16> = scalars(file, frame_id_channel)?
        .into_iter()
        .map(|id| (id as u32 & FRAME_ID_MASK) as u16)
        .collect();
    let timestamps = scalars(file, master)?;
    let lengths: Vec<u8> = scalars(file, length_channel)?
        .into_iter()
        .map(|len| len as u8)
        .collect();
    let payload_values = file.signal(payload_channel)?.values()?;

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // identifier with another frame's payload.
    if timestamps.len() != frame_ids.len()
        || lengths.len() != frame_ids.len()
        || payload_values.len() != frame_ids.len()
    {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} frame IDs, {} timestamps, {} lengths and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            frame_ids.len(),
            timestamps.len(),
            lengths.len(),
            payload_values.len()
        )));
    }

    // Bus channel: optional, default to 0 (real bus is never numbered zero).
    let bus_channels = match field(group, "BusChannel") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|bus| bus as u8)
            .collect(),
        None => Vec::new(),
    };

    // Cycle: optional, default to 0 (real bus is never numbered zero).
    let cycles = match field(group, "Cycle") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|val| (val as u32 & CYCLE_MASK) as u8)
            .collect(),
        None => vec![0u8; frame_ids.len()],
    };

    // NullFrameFlag: optional, false when the file does not say.
    let null_frames = match field(group, "NullFrameFlag") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|val| val != 0.0)
            .collect(),
        None => vec![false; frame_ids.len()],
    };

    // SyncFrameFlag: optional, false when the file does not say.
    let sync_frames = match field(group, "SyncFrameFlag") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|val| val != 0.0)
            .collect(),
        None => vec![false; frame_ids.len()],
    };

    // StartupFlag: optional, false when the file does not say.
    let startup_frames = match field(group, "StartupFlag") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|val| val != 0.0)
            .collect(),
        None => vec![false; frame_ids.len()],
    };

    // The payloads are flattened here rather than kept as the decoded
    // `SignalValues`: each one is trimmed to the length the record logged,
    // and a fixed-width DataBytes channel pads short frames out to its full
    // width. `starts` has one entry per frame plus a final total.
    let mut payloads = Vec::new();
    let mut payload_starts = Vec::with_capacity(frame_ids.len() + 1);
    for (index, &len) in lengths.iter().enumerate() {
        let sample = payload_values.bytes_at(index).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "payload channel '{FLX_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
                group.acquisition_name
            ))
        })?;
        payload_starts.push(payloads.len());
        payloads.extend_from_slice(&sample[..usize::from(len).min(sample.len())]);
    }
    payload_starts.push(payloads.len());

    Ok(FlexRayFrames {
        timestamps,
        frame_ids,
        cycles,
        bus_channels,
        null_frames,
        sync_frames,
        startup_frames,
        payloads,
        payload_starts,
    })
}

/// Returns true if `group` holds FlexRay frames this module can read.
pub(crate) fn is_flexray_frame_group(group: &ChannelGroup) -> bool {
    field(group, "FrameID").is_some() && field(group, "DataBytes").is_some()
}
