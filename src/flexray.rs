//! FlexRay frames out of bus-logged groups.
//!
//! The counterpart to [`crate::bus`] for FlexRay traffic: a group a FlexRay logger
//! wrote holds one composed `FlexRayFrame` channel per field — timestamp,
//! identifier, payload — and this module reads them back as frames.

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup, SignalValues};
use crate::Mf4File;

/// The name a FlexRay frame group composes its fields under.
///
/// The frame fields appear as channels named `FLX_Frame.ID`,
/// `FLX_Frame.DataBytes` and so on — the same arrangement `CAN_DataFrame`
/// uses (see [`crate::bus`]), under its own prefix. Names are the ones
/// `ihedvall/mdflib` writes, checked against its files in
/// `tests/bus_mdflib_fixtures.rs`; the names this module first guessed
/// (`FrameID`, `StartupFlag`) are still accepted as fallbacks. `qualify_channel_name`
/// has already reconciled where the prefix is stored by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to
/// look for.
const FLX_PREFIX: &str = "FLX_Frame";

/// Names the identifier channel may carry: `ID`, as the ASAM bus-logging
/// layout and mdflib spell it (the same field is `CAN_DataFrame.ID`), then
/// `FrameID`, which files written against this crate's first guess use.
const FRAME_ID_NAMES: &[&str] = &["ID", "FrameID"];

/// Names the startup-frame flag may carry, in the same order of preference.
const STARTUP_NAMES: &[&str] = &["StartUpFrameFlag", "StartupFrameFlag", "StartupFlag"];

/// The 11 bits of the ID field that hold the identifier itself.
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
    /// Cycle counter, 0..=63 (0 when the group does not log `FLX_Frame.Cycle`;
    /// use [`FlexRayFrame::cycle_opt`] to distinguish a missing cycle from
    /// cycle 0).
    pub cycle: u8,
    /// Which bus the frame was logged from, or 0 when the file does not say.
    pub bus_channel: u8,
    /// FlexRay channel mask (`FLX_Frame.ChannelMask`: bit 0 = Channel A,
    /// bit 1 = Channel B), or `None` when the file does not log it.
    pub channel_mask: Option<u8>,
    /// Whether the frame carried no payload.
    pub null_frame: bool,
    /// Whether the frame is a sync frame.
    pub sync_frame: bool,
    /// Whether the frame is a startup frame.
    pub startup: bool,
    /// The payload, trimmed to the logged length.
    pub data: &'a [u8],
    /// Whether `cycle` came from a logged valid `FLX_Frame.Cycle` sample.
    has_cycle: bool,
}

impl FlexRayFrame<'_> {
    /// Returns the cycle counter (0..=63) when the group logged a valid
    /// `FLX_Frame.Cycle` value for this frame, or `None` when absent.
    pub fn cycle_opt(&self) -> Option<u8> {
        self.has_cycle.then_some(self.cycle)
    }
}

/// Every FlexRay frame of one channel group, in logging order.
pub struct FlexRayFrames {
    /// Frame timestamps.
    pub(crate) timestamps: Vec<f64>,
    /// Frame identifiers (11-bit masked).
    pub(crate) frame_ids: Vec<u16>,
    /// Cycle counters (6-bit masked); `None` when the group has no `Cycle` channel.
    pub(crate) cycles: Option<Vec<Option<u8>>>,
    /// Bus channel per frame; empty when the file does not record one.
    pub(crate) bus_channels: Vec<u8>,
    /// Channel masks (Channel A / Channel B); `None` when absent.
    pub(crate) channel_masks: Option<Vec<Option<u8>>>,
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

    /// Returns the logged cycle counter (0..=63) for frame `index`, or `None`
    /// when `FLX_Frame.Cycle` was not recorded or invalid for that frame.
    pub fn cycle_at(&self, index: usize) -> Option<u8> {
        self.cycles.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the logged `FLX_Frame.ChannelMask` (Channel A / Channel B) for
    /// frame `index`, or `None` when absent or invalid.
    pub fn channel_mask_at(&self, index: usize) -> Option<u8> {
        self.channel_masks.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the frame at `index`, or `None` when it is past the end.
    pub fn get(&self, index: usize) -> Option<FlexRayFrame<'_>> {
        let frame_id = *self.frame_ids.get(index)?;
        let cycle_opt = self.cycle_at(index);
        let from = *self.payload_starts.get(index)?;
        let to = *self.payload_starts.get(index + 1)?;

        Some(FlexRayFrame {
            timestamp: *self.timestamps.get(index)?,
            frame_id,
            cycle: cycle_opt.unwrap_or(0),
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            channel_mask: self.channel_mask_at(index),
            null_frame: *self.null_frames.get(index)?,
            sync_frame: *self.sync_frames.get(index)?,
            startup: *self.startup_frames.get(index)?,
            data: self.payloads.get(from..to)?,
            has_cycle: cycle_opt.is_some(),
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

/// The first of `suffixes` the group has.
fn field_any<'a>(group: &'a ChannelGroup, suffixes: &[&str]) -> Option<&'a Channel> {
    suffixes.iter().find_map(|s| field(group, s))
}

type ScalarsWithValidity = (Vec<f64>, Option<Vec<bool>>);

/// Reads a scalar frame field together with its per-sample validity mask.
fn scalars_with_validity(file: &Mf4File, channel: &Channel) -> Result<ScalarsWithValidity> {
    let signal = file.signal(channel)?;
    let validity = signal.validity();
    let values = signal.values()?.to_f64();
    Ok((values, validity))
}

fn is_sample_valid(validity: &Option<Vec<bool>>, index: usize) -> bool {
    validity
        .as_ref()
        .is_none_or(|flags| flags.get(index).copied().unwrap_or(false))
}

fn read_optional_scalars(
    file: &Mf4File,
    group: &ChannelGroup,
    channel: Option<&Channel>,
    label: &str,
    expected_len: usize,
) -> Result<Option<ScalarsWithValidity>> {
    let Some(channel) = channel else {
        return Ok(None);
    };
    let (values, validity) = scalars_with_validity(file, channel)?;
    if values.len() != expected_len {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} {label} samples and {} frame IDs; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            values.len(),
            expected_len
        )));
    }
    Ok(Some((values, validity)))
}

/// Reads every FlexRay frame `group` logged.
pub(crate) fn read_flexray_frames(file: &Mf4File, group: &ChannelGroup) -> Result<FlexRayFrames> {
    let frame_id_channel =
        field_any(group, FRAME_ID_NAMES).ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("{FLX_PREFIX}.ID"),
        })?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let (raw_frame_ids, id_validity) = scalars_with_validity(file, frame_id_channel)?;
    let (raw_timestamps, ts_validity) = scalars_with_validity(file, master)?;
    let payload_signal = file.signal(payload_channel)?;
    let payload_validity = payload_signal.validity();
    let payload_values = payload_signal.values()?;

    if !matches!(
        payload_values,
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
    ) {
        return Err(Mf4Error::parse_error(format!(
            "payload channel '{FLX_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
            group.acquisition_name
        )));
    }

    let count = raw_frame_ids.len();

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // identifier with another frame's payload.
    if raw_timestamps.len() != count || payload_values.len() != count {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} frame IDs, {} timestamps and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            count,
            raw_timestamps.len(),
            payload_values.len()
        )));
    }

    let raw_lengths =
        read_optional_scalars(file, group, field(group, "DataLength"), "DataLength", count)?;
    let raw_bus_channels =
        read_optional_scalars(file, group, field(group, "BusChannel"), "BusChannel", count)?;
    let raw_cycles = read_optional_scalars(file, group, field(group, "Cycle"), "Cycle", count)?;
    let raw_channel_masks = read_optional_scalars(
        file,
        group,
        field(group, "ChannelMask"),
        "ChannelMask",
        count,
    )?;
    let raw_null_flags = read_optional_scalars(
        file,
        group,
        field(group, "NullFrameFlag"),
        "NullFrameFlag",
        count,
    )?;
    let raw_sync_flags = read_optional_scalars(
        file,
        group,
        field(group, "SyncFrameFlag"),
        "SyncFrameFlag",
        count,
    )?;
    let raw_startup_flags = read_optional_scalars(
        file,
        group,
        field_any(group, STARTUP_NAMES),
        "StartupFlag",
        count,
    )?;

    let mut timestamps = Vec::with_capacity(count);
    let mut frame_ids = Vec::with_capacity(count);
    let mut cycles = raw_cycles.as_ref().map(|_| Vec::with_capacity(count));
    let mut bus_channels = if raw_bus_channels.is_some() {
        Vec::with_capacity(count)
    } else {
        Vec::new()
    };
    let mut channel_masks = raw_channel_masks
        .as_ref()
        .map(|_| Vec::with_capacity(count));
    let mut null_frames = Vec::with_capacity(count);
    let mut sync_frames = Vec::with_capacity(count);
    let mut startup_frames = Vec::with_capacity(count);
    let mut payloads = Vec::new();
    let mut payload_starts = Vec::with_capacity(count + 1);

    for index in 0..count {
        let len_ok = raw_lengths
            .as_ref()
            .is_none_or(|(vals, val)| is_sample_valid(val, index) && vals[index].is_finite());
        if !is_sample_valid(&ts_validity, index)
            || !raw_timestamps[index].is_finite()
            || !is_sample_valid(&id_validity, index)
            || !raw_frame_ids[index].is_finite()
            || !is_sample_valid(&payload_validity, index)
            || !len_ok
        {
            continue;
        }
        let sample = payload_values.bytes_at(index).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "payload channel '{FLX_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
                group.acquisition_name
            ))
        })?;
        let slice = match &raw_lengths {
            Some((vals, _)) => {
                let len = usize::from(vals[index] as u8);
                &sample[..len.min(sample.len())]
            }
            None => sample,
        };

        timestamps.push(raw_timestamps[index]);
        frame_ids.push((raw_frame_ids[index] as u32 & FRAME_ID_MASK) as u16);

        if let (Some(dst), Some((vals, val_mask))) = (&mut cycles, &raw_cycles) {
            let cyc = if is_sample_valid(val_mask, index) && vals[index].is_finite() {
                Some((vals[index] as u32 & CYCLE_MASK) as u8)
            } else {
                None
            };
            dst.push(cyc);
        }

        if let Some((vals, val_mask)) = &raw_bus_channels {
            let bus = if is_sample_valid(val_mask, index) && vals[index].is_finite() {
                vals[index] as u8
            } else {
                0
            };
            bus_channels.push(bus);
        }

        if let (Some(dst), Some((vals, val_mask))) = (&mut channel_masks, &raw_channel_masks) {
            let mask = if is_sample_valid(val_mask, index) && vals[index].is_finite() {
                Some(vals[index] as u8)
            } else {
                None
            };
            dst.push(mask);
        }

        // FlexRay's null frame indicator is inverted: 0 marks a null frame,
        // 1 a frame carrying data.
        let is_null = match &raw_null_flags {
            Some((vals, val_mask)) => {
                is_sample_valid(val_mask, index) && vals[index].is_finite() && vals[index] == 0.0
            }
            None => false,
        };
        null_frames.push(is_null);

        let is_sync = match &raw_sync_flags {
            Some((vals, val_mask)) => {
                is_sample_valid(val_mask, index) && vals[index].is_finite() && vals[index] != 0.0
            }
            None => false,
        };
        sync_frames.push(is_sync);

        let is_startup = match &raw_startup_flags {
            Some((vals, val_mask)) => {
                is_sample_valid(val_mask, index) && vals[index].is_finite() && vals[index] != 0.0
            }
            None => false,
        };
        startup_frames.push(is_startup);

        payload_starts.push(payloads.len());
        payloads.extend_from_slice(slice);
    }
    payload_starts.push(payloads.len());

    Ok(FlexRayFrames {
        timestamps,
        frame_ids,
        cycles,
        bus_channels,
        channel_masks,
        null_frames,
        sync_frames,
        startup_frames,
        payloads,
        payload_starts,
    })
}

/// Returns true if `group` holds FlexRay frames this module can read.
pub(crate) fn is_flexray_frame_group(group: &ChannelGroup) -> bool {
    field_any(group, FRAME_ID_NAMES).is_some()
        && field(group, "DataBytes").is_some()
        && group.master_channel().is_some()
}
