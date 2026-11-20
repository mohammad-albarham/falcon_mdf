//! Frame extraction from bus-logged channel groups.
//!
//! A bus logger writes CAN traffic into an MF4 file as raw frames: a timestamp,
//! an identifier, a bus channel and a payload of bytes. What those bytes *mean*
//! lives outside the file, in a DBC or ARXML database the user supplies. This
//! module stops at the frame. It reports what was logged and interprets no
//! payload byte, which makes a bus log inspectable without any database at all.
//!
//! ```no_run
//! use falcon_mdf::Mf4File;
//!
//! let file = Mf4File::open("bus_log.mf4")?;
//! for group in file.can_frame_groups() {
//!     let frames = file.can_frames(group)?;
//!     for frame in frames.iter().take(5) {
//!         println!("{:.6} 0x{:X} {:?}", frame.timestamp, frame.id, frame.data);
//!     }
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```

use std::collections::HashMap;

use crate::blocks::source::BusType;
use crate::candb::CanDatabase;
use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup, SignalValues};
use crate::Mf4File;

/// The default name a CAN data-frame group composes its fields under.
///
/// The frame fields appear as channels named `CAN_DataFrame.ID`,
/// `CAN_DataFrame.DataBytes` and so on. Writers disagree about whether the
/// prefix is stored on the member channel or only on its composition parent;
/// `qualify_channel_name` has already reconciled that by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to look
/// for.
const CAN_PREFIX: &str = "CAN_DataFrame";

/// Alternative prefix used by some CAN FD loggers.
const CAN_FD_PREFIX: &str = "CAN_FD_DataFrame";

/// Bits of the identifier field that hold the identifier itself.
///
/// CAN identifiers are 11 bits (standard) or 29 bits (extended); nothing above
/// bit 28 is part of the number.
const ID_MASK: u32 = 0x1FFF_FFFF;

/// Converts a CAN / CAN FD DLC value (0..=15) to its payload length in bytes.
fn dlc_to_length(dlc: u8, is_fd: bool) -> u8 {
    if is_fd {
        match dlc {
            0..=8 => dlc,
            9 => 12,
            10 => 16,
            11 => 20,
            12 => 24,
            13 => 32,
            14 => 48,
            _ => 64,
        }
    } else {
        dlc.min(8)
    }
}

/// Optional CAN / CAN FD control flags recorded alongside a [`CanFrame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CanFrameFlags {
    /// Extended Data Length (CAN FD frame) flag (`CAN_DataFrame.EDL`).
    pub edl: Option<bool>,
    /// Bit Rate Switch flag (`CAN_DataFrame.BRS`).
    pub brs: Option<bool>,
    /// Error State Indicator flag (`CAN_DataFrame.ESI`).
    pub esi: Option<bool>,
    /// Remote Transmission Request flag (`CAN_DataFrame.RTR`).
    pub rtr: Option<bool>,
    /// Frame direction (`CAN_DataFrame.Dir`): `Some(false)` = Rx, `Some(true)` = Tx.
    pub dir: Option<bool>,
}

/// One logged CAN frame, borrowed from the group it was read out of.
///
/// The payload is not interpreted. Turning `data` into named physical signals
/// needs a CAN database, which [`crate::candb::CanDatabase`] provides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanFrame<'a> {
    /// Time of the frame, in the master channel's unit — seconds, for every
    /// bus logger seen so far.
    pub timestamp: f64,
    /// The CAN identifier, masked to its 29 significant bits.
    pub id: u32,
    /// Whether this is an extended (29-bit) frame.
    ///
    /// `None` when the group has no `IDE` channel, in which case the file does
    /// not say. Deriving it from the identifier's magnitude would be a guess:
    /// an extended frame may perfectly well carry a number below `0x800`.
    pub extended: Option<bool>,
    /// Which bus of a multi-bus logger the frame came from, numbered from 1.
    ///
    /// Zero when the group has no `BusChannel` channel, since a real bus is
    /// never numbered zero.
    pub bus_channel: u8,
    /// The frame's payload, trimmed to the logged data length.
    pub data: &'a [u8],
}

/// Every CAN frame from one bus-logged channel group.
///
/// Frames are stored field-by-field rather than as a vector of structs: a bus
/// log holds millions of frames, and one allocation per payload would dominate
/// the cost of reading one. Use [`CanFrames::get`] or [`CanFrames::iter`] to
/// address a frame, and [`CanFrames::flags`] / [`CanFrames::edl`] / etc. to
/// inspect optional CAN FD control flags.
#[derive(Debug, Clone)]
pub struct CanFrames {
    timestamps: Vec<f64>,
    ids: Vec<u32>,
    extended: Option<Vec<Option<bool>>>,
    bus_channels: Vec<u8>,
    payloads: SignalValues,
    /// Logged payload length per frame, present when the group records
    /// `DataLength` or `DLC`. A fixed-width payload channel pads short frames
    /// out to its full width, and this is what trims them back.
    lengths: Option<Vec<u8>>,
    dlcs: Option<Vec<Option<u8>>>,
    edl: Option<Vec<Option<bool>>>,
    brs: Option<Vec<Option<bool>>>,
    esi: Option<Vec<Option<bool>>>,
    rtr: Option<Vec<Option<bool>>>,
    dir: Option<Vec<Option<bool>>>,
}

impl CanFrames {
    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns true if the group logged no frames.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the frame at `index`, or `None` if there is no such frame.
    pub fn get(&self, index: usize) -> Option<CanFrame<'_>> {
        let id = *self.ids.get(index)?;
        let data = self.payloads.bytes_at(index)?;
        let data = match &self.lengths {
            Some(lengths) => {
                let len = usize::from(*lengths.get(index)?);
                data.get(..len.min(data.len()))?
            }
            None => data,
        };

        Some(CanFrame {
            timestamp: *self.timestamps.get(index)?,
            id,
            extended: self
                .extended
                .as_ref()
                .and_then(|flags| flags.get(index).copied().flatten()),
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            data,
        })
    }

    /// Returns the Extended Data Length (CAN FD) flag for frame `index`, if logged.
    pub fn edl(&self, index: usize) -> Option<bool> {
        self.edl.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the Bit Rate Switch (CAN FD) flag for frame `index`, if logged.
    pub fn brs(&self, index: usize) -> Option<bool> {
        self.brs.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the Error State Indicator (CAN FD) flag for frame `index`, if logged.
    pub fn esi(&self, index: usize) -> Option<bool> {
        self.esi.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the Remote Transmission Request flag for frame `index`, if logged.
    pub fn rtr(&self, index: usize) -> Option<bool> {
        self.rtr.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the transmission direction for frame `index` (`false` = Rx,
    /// `true` = Tx), if logged.
    pub fn dir(&self, index: usize) -> Option<bool> {
        self.dir.as_ref()?.get(index).copied().flatten()
    }

    /// Returns the raw DLC (0..=15) for frame `index`, if logged.
    pub fn dlc(&self, index: usize) -> Option<u8> {
        self.dlcs.as_ref()?.get(index).copied().flatten()
    }

    /// Returns all optional CAN / CAN FD control flags for frame `index`.
    pub fn flags(&self, index: usize) -> CanFrameFlags {
        CanFrameFlags {
            edl: self.edl(index),
            brs: self.brs(index),
            esi: self.esi(index),
            rtr: self.rtr(index),
            dir: self.dir(index),
        }
    }

    /// Iterates over every frame in logging order.
    pub fn iter(&self) -> impl Iterator<Item = CanFrame<'_>> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }
}

/// One decoded signal as a time series: every reading the log holds of it.
///
/// Produced by [`Mf4File::decode_bus`]. The names borrow from the database, so a
/// series costs two vectors and no string allocation.
#[derive(Debug, Clone, PartialEq)]
pub struct BusSignal<'a> {
    /// Name of the message the signal was decoded out of.
    pub message: &'a str,
    /// Signal name as the database spells it.
    pub name: &'a str,
    /// Physical unit, empty when the database gives none.
    pub unit: &'a str,
    /// Which bus of a multi-bus logger these readings came from.
    ///
    /// Part of the signal's identity, not a label: the same identifier on two
    /// buses is two different signals, and merging them would interleave
    /// readings from unrelated networks into one series.
    pub bus_channel: u8,
    /// Time of each reading, in the master channel's unit.
    pub timestamps: Vec<f64>,
    /// Physical value of each reading, parallel to `timestamps`.
    pub values: Vec<f64>,
    /// Value-table labels, parallel to `values`.
    ///
    /// `None` when the signal has no value table at all, which is the ordinary
    /// case. Read it through [`BusSignal::text_at`].
    texts: Option<Vec<Option<&'a str>>>,
}

impl<'a> BusSignal<'a> {
    /// Returns the number of readings.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if the signal was never seen.
    ///
    /// A series is only created when a reading arrives, so this is false for
    /// every signal [`Mf4File::decode_bus`] returns.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the value table's label for the reading at `index`.
    ///
    /// `None` when the signal has no value table, when the table does not cover
    /// that reading, or when there is no such reading.
    pub fn text_at(&self, index: usize) -> Option<&'a str> {
        self.texts.as_ref()?.get(index).copied().flatten()
    }
}

/// Every signal a database decoded out of a file's bus traffic.
///
/// A signal is identified by its bus, its message and its name together. Two
/// messages may spell one signal name, so the name alone does not name a series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BusSignals<'a> {
    signals: Vec<BusSignal<'a>>,
}

impl<'a> BusSignals<'a> {
    /// Returns the number of decoded signals.
    pub fn len(&self) -> usize {
        self.signals.len()
    }

    /// Returns true if nothing decoded.
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    /// Returns every decoded signal.
    pub fn iter(&self) -> impl Iterator<Item = &BusSignal<'a>> + '_ {
        self.signals.iter()
    }

    /// Returns the signals with this name, across every message and bus.
    ///
    /// A slice rather than a single signal because the name is not unique: two
    /// messages may define `EngineSpeed`, and a multi-bus log may carry the same
    /// message twice. Returning the first would silently pick one.
    pub fn find(&self, name: &str) -> Vec<&BusSignal<'a>> {
        self.signals
            .iter()
            .filter(|signal| signal.name == name)
            .collect()
    }
}

/// Accumulates decoded readings into one series per bus, message and signal.
///
/// Separate from the file so that the keying can be tested against frames built
/// by hand: no corpus file carries one identifier on two buses, which is the
/// case where getting the key wrong silently merges two networks' readings.
pub(crate) struct Accumulator<'a> {
    database: &'a CanDatabase,
    signals: Vec<BusSignal<'a>>,
    /// (bus channel, message index, signal index) -> position in `signals`.
    series: HashMap<(u8, usize, usize), usize>,
}

impl<'a> Accumulator<'a> {
    pub(crate) fn new(database: &'a CanDatabase) -> Self {
        Accumulator {
            database,
            signals: Vec::new(),
            series: HashMap::new(),
        }
    }

    pub(crate) fn push(&mut self, frame: CanFrame<'_>) {
        let database = self.database;
        let signals = &mut self.signals;
        let series = &mut self.series;

        database.decode_each(
            frame.id,
            frame.extended,
            frame.data,
            &mut |message, signal, decoded| {
                let slot = *series
                    .entry((frame.bus_channel, message, signal))
                    .or_insert_with(|| {
                        let definition = &database.messages()[message].signals[signal];
                        signals.push(BusSignal {
                            message: &database.messages()[message].name,
                            name: decoded.name,
                            unit: decoded.unit,
                            bus_channel: frame.bus_channel,
                            timestamps: Vec::new(),
                            values: Vec::new(),
                            // Decided once, from the definition, so the labels stay
                            // parallel to the values: a signal with a table has an
                            // entry for every reading, even the ones it does not
                            // name.
                            texts: (!definition.value_table.is_empty()).then(Vec::new),
                        });
                        signals.len() - 1
                    });

                let series = &mut signals[slot];
                series.timestamps.push(frame.timestamp);
                series.values.push(decoded.value);
                if let Some(texts) = &mut series.texts {
                    texts.push(decoded.text);
                }
            },
        );
    }

    pub(crate) fn finish(self) -> BusSignals<'a> {
        BusSignals {
            signals: self.signals,
        }
    }
}

pub(crate) fn decode_bus_signals<'a>(
    file: &Mf4File,
    database: &'a CanDatabase,
) -> Result<BusSignals<'a>> {
    let mut accumulator = Accumulator::new(database);
    // One group at a time: a multi-bus log has dozens, and holding every group's
    // frames at once to chain the iterators would undo the point of the
    // field-by-field storage in `CanFrames`.
    for group in file.can_frame_groups() {
        for frame in file.can_frames(group)?.iter() {
            accumulator.push(frame);
        }
    }
    Ok(accumulator.finish())
}

/// Returns true if `group` holds CAN data frames this module can read.
///
/// Detection is by the frame channels being present rather than solely by the
/// `cg_flags` bus-event bit, because the channels are what reading actually
/// needs. The flag itself is available as [`ChannelGroup::is_bus_event`].
pub(crate) fn is_can_frame_group(group: &ChannelGroup) -> bool {
    detect_can_prefix(group).is_some() && group.master_channel().is_some()
}

/// Detects the channel name prefix used by a CAN frame group.
///
/// Tries the standard `CAN_DataFrame` and `CAN_FD_DataFrame` prefixes first,
/// then falls back to scanning `<prefix>.ID` + `<prefix>.DataBytes` when the
/// group is marked as a CAN bus-event group (handling e.g. `CAN1_DataFrame`).
fn detect_can_prefix(group: &ChannelGroup) -> Option<String> {
    for prefix in [CAN_PREFIX, CAN_FD_PREFIX] {
        if field_with_prefix(group, prefix, "ID").is_some()
            && field_with_prefix(group, prefix, "DataBytes").is_some()
        {
            return Some(prefix.to_string());
        }
    }
    let is_can_source = group
        .source
        .as_ref()
        .is_some_and(|s| s.bus_type == Some(BusType::Can));
    if is_can_source || group.is_bus_event() {
        for ch in &group.channels {
            if let Some(prefix) = ch.name.strip_suffix(".ID") {
                if (is_can_source || prefix.starts_with("CAN"))
                    && field_with_prefix(group, prefix, "DataBytes").is_some()
                {
                    return Some(prefix.to_string());
                }
            }
        }
    }
    None
}

fn field_with_prefix<'a>(
    group: &'a ChannelGroup,
    prefix: &str,
    suffix: &str,
) -> Option<&'a Channel> {
    let qualified = format!("{prefix}.{suffix}");
    group.find_channel(&qualified)
}

fn require_with_prefix<'a>(
    group: &'a ChannelGroup,
    prefix: &str,
    suffix: &str,
) -> Result<&'a Channel> {
    field_with_prefix(group, prefix, suffix).ok_or_else(|| Mf4Error::ChannelNotFound {
        name: format!("{prefix}.{suffix}"),
    })
}

/// Reads a scalar frame field together with its per-sample validity mask.
fn scalars_with_validity(
    file: &Mf4File,
    channel: &Channel,
) -> Result<(Vec<f64>, Option<Vec<bool>>)> {
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

fn read_optional_bools(
    file: &Mf4File,
    group: &ChannelGroup,
    prefix: &str,
    suffix: &str,
    expected_len: usize,
) -> Result<Option<Vec<Option<bool>>>> {
    let Some(channel) = field_with_prefix(group, prefix, suffix) else {
        return Ok(None);
    };
    let (values, validity) = scalars_with_validity(file, channel)?;
    if values.len() != expected_len {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} '{prefix}.{suffix}' samples and {} identifiers; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            values.len(),
            expected_len
        )));
    }
    Ok(Some(
        values
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                if is_sample_valid(&validity, i) && v.is_finite() {
                    Some(v != 0.0)
                } else {
                    None
                }
            })
            .collect(),
    ))
}

fn filter_opt_vec<T>(opt: Option<Vec<T>>, valid: &[bool]) -> Option<Vec<T>> {
    opt.map(|vec| {
        vec.into_iter()
            .enumerate()
            .filter_map(|(i, v)| valid[i].then_some(v))
            .collect()
    })
}

pub(crate) fn read_can_frames(file: &Mf4File, group: &ChannelGroup) -> Result<CanFrames> {
    let prefix = detect_can_prefix(group).unwrap_or_else(|| CAN_PREFIX.to_string());
    let id_channel = require_with_prefix(group, &prefix, "ID")?;
    let payload_channel = require_with_prefix(group, &prefix, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let (raw_ids, id_validity) = scalars_with_validity(file, id_channel)?;
    let (timestamps, ts_validity) = scalars_with_validity(file, master)?;
    let payload_signal = file.signal(payload_channel)?;
    let payload_validity = payload_signal.validity();
    let payloads = payload_signal.values()?;

    if !matches!(
        payloads,
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
    ) {
        return Err(Mf4Error::parse_error(format!(
            "payload channel '{prefix}.DataBytes' of bus group '{}' did not decode to bytes",
            group.acquisition_name
        )));
    }

    let count = raw_ids.len();

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // identifier with another's payload.
    if timestamps.len() != count || payloads.len() != count {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} identifiers, {} timestamps and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            count,
            timestamps.len(),
            payloads.len()
        )));
    }

    let extended = read_optional_bools(file, group, &prefix, "IDE", count)?;
    let edl = read_optional_bools(file, group, &prefix, "EDL", count)?;
    let brs = read_optional_bools(file, group, &prefix, "BRS", count)?;
    let esi = read_optional_bools(file, group, &prefix, "ESI", count)?;
    let rtr = read_optional_bools(file, group, &prefix, "RTR", count)?;
    let dir = read_optional_bools(file, group, &prefix, "Dir", count)?;

    let bus_channels = match field_with_prefix(group, &prefix, "BusChannel") {
        Some(channel) => {
            let (values, validity) = scalars_with_validity(file, channel)?;
            if values.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} bus channels and {} identifiers; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    values.len(),
                    count
                )));
            }
            values
                .into_iter()
                .enumerate()
                .map(|(i, bus)| {
                    if is_sample_valid(&validity, i) && bus.is_finite() {
                        bus as u8
                    } else {
                        0
                    }
                })
                .collect()
        }
        None => Vec::new(),
    };

    let raw_data_lengths = match field_with_prefix(group, &prefix, "DataLength") {
        Some(channel) => {
            let (values, validity) = scalars_with_validity(file, channel)?;
            if values.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} lengths and {} identifiers; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    values.len(),
                    count
                )));
            }
            Some((values, validity))
        }
        None => None,
    };

    let raw_dlcs = match field_with_prefix(group, &prefix, "DLC") {
        Some(channel) => {
            let (values, validity) = scalars_with_validity(file, channel)?;
            if values.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} DLC values and {} identifiers; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    values.len(),
                    count
                )));
            }
            Some((values, validity))
        }
        None => None,
    };

    let dlcs: Option<Vec<Option<u8>>> = raw_dlcs.as_ref().map(|(values, validity)| {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                if is_sample_valid(validity, i) && v.is_finite() {
                    Some((v as u8) & 0x0F)
                } else {
                    None
                }
            })
            .collect()
    });

    // Determine which frames are valid (all required channels valid and finite).
    let mut all_valid = true;
    let mut valid_mask = vec![true; count];
    for i in 0..count {
        let len_ok = match (&raw_data_lengths, &raw_dlcs) {
            (Some((vals, val_mask)), _) => is_sample_valid(val_mask, i) && vals[i].is_finite(),
            (None, Some((vals, val_mask))) => is_sample_valid(val_mask, i) && vals[i].is_finite(),
            (None, None) => true,
        };
        let ok = is_sample_valid(&ts_validity, i)
            && timestamps[i].is_finite()
            && is_sample_valid(&id_validity, i)
            && raw_ids[i].is_finite()
            && is_sample_valid(&payload_validity, i)
            && payloads.bytes_at(i).is_some()
            && len_ok;
        if !ok {
            valid_mask[i] = false;
            all_valid = false;
        }
    }

    let lengths: Option<Vec<u8>> = if let Some((vals, _)) = &raw_data_lengths {
        Some(vals.iter().map(|&len| len as u8).collect())
    } else if let Some((vals, _)) = &raw_dlcs {
        Some(
            vals.iter()
                .enumerate()
                .map(|(i, &dlc)| {
                    let is_fd = edl
                        .as_ref()
                        .and_then(|flags| flags.get(i).copied().flatten())
                        .unwrap_or(false);
                    dlc_to_length(dlc as u8, is_fd)
                })
                .collect(),
        )
    } else {
        None
    };

    let ids: Vec<u32> = raw_ids.into_iter().map(|id| id as u32 & ID_MASK).collect();

    if all_valid {
        return Ok(CanFrames {
            timestamps,
            ids,
            extended,
            bus_channels,
            payloads,
            lengths,
            dlcs,
            edl,
            brs,
            esi,
            rtr,
            dir,
        });
    }

    // Filter out invalid / NaN frames while preserving parallel alignment.
    let kept = valid_mask.iter().filter(|&&v| v).count();
    let mut filtered_timestamps = Vec::with_capacity(kept);
    let mut filtered_ids = Vec::with_capacity(kept);
    let mut filtered_bus = if bus_channels.is_empty() {
        Vec::new()
    } else {
        Vec::with_capacity(kept)
    };
    let mut filtered_payload_bytes = Vec::new();
    let mut filtered_starts = Vec::with_capacity(kept + 1);

    for i in 0..count {
        if !valid_mask[i] {
            continue;
        }
        filtered_timestamps.push(timestamps[i]);
        filtered_ids.push(ids[i]);
        if !bus_channels.is_empty() {
            filtered_bus.push(bus_channels[i]);
        }
        let sample = payloads.bytes_at(i).unwrap_or(&[]);
        filtered_starts.push(filtered_payload_bytes.len());
        filtered_payload_bytes.extend_from_slice(sample);
    }
    filtered_starts.push(filtered_payload_bytes.len());

    Ok(CanFrames {
        timestamps: filtered_timestamps,
        ids: filtered_ids,
        extended: filter_opt_vec(extended, &valid_mask),
        bus_channels: filtered_bus,
        payloads: SignalValues::VarBytes {
            data: filtered_payload_bytes,
            starts: filtered_starts,
        },
        lengths: filter_opt_vec(lengths, &valid_mask),
        dlcs: filter_opt_vec(dlcs, &valid_mask),
        edl: filter_opt_vec(edl, &valid_mask),
        brs: filter_opt_vec(brs, &valid_mask),
        esi: filter_opt_vec(esi, &valid_mask),
        rtr: filter_opt_vec(rtr, &valid_mask),
        dir: filter_opt_vec(dir, &valid_mask),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candb::{MessageDef, Multiplexing, SignalDef, SignalEncoding};

    fn signal(name: &str, start_bit: u64, value_table: Vec<(i64, String)>) -> SignalDef {
        SignalDef {
            name: name.into(),
            start_bit,
            size: 8,
            big_endian: false,
            signed: false,
            encoding: SignalEncoding::Unsigned,
            factor: 1.0,
            offset: 0.0,
            unit: "u".into(),
            multiplexing: Multiplexing::None,
            value_table,
            physical_ranges: Vec::new(),
        }
    }

    fn database() -> CanDatabase {
        CanDatabase::new(vec![
            MessageDef {
                name: "First".into(),
                id: 0x100,
                extended: false,
                length: 2,
                signals: vec![
                    signal("Shared", 0, Vec::new()),
                    signal("Labelled", 8, vec![(1, "On".into()), (0, "Off".into())]),
                ],
            },
            MessageDef {
                name: "Second".into(),
                id: 0x200,
                extended: false,
                length: 1,
                // Deliberately the same signal name as First's, which is why a
                // series cannot be identified by name alone.
                signals: vec![signal("Shared", 0, Vec::new())],
            },
        ])
    }

    fn frame(timestamp: f64, id: u32, bus_channel: u8, data: &[u8]) -> CanFrame<'_> {
        CanFrame {
            timestamp,
            id,
            extended: Some(false),
            bus_channel,
            data,
        }
    }

    fn collect<'a>(database: &'a CanDatabase, frames: Vec<CanFrame<'_>>) -> BusSignals<'a> {
        let mut accumulator = Accumulator::new(database);
        for frame in frames {
            accumulator.push(frame);
        }
        accumulator.finish()
    }

    /// The case no corpus file can reach: one identifier logged on two buses.
    ///
    /// Merging them would interleave readings from unrelated networks into a
    /// single series, and nothing downstream could tell that had happened.
    #[test]
    fn one_identifier_on_two_buses_is_two_series() {
        let db = database();
        let signals = collect(
            &db,
            vec![
                frame(0.0, 0x100, 1, &[10, 0]),
                frame(0.1, 0x100, 9, &[20, 0]),
                frame(0.2, 0x100, 1, &[11, 0]),
                frame(0.3, 0x100, 9, &[21, 0]),
            ],
        );

        let shared = signals.find("Shared");
        assert_eq!(shared.len(), 2, "one series per bus");

        let mut by_bus: Vec<_> = shared.iter().map(|s| (s.bus_channel, &s.values)).collect();
        by_bus.sort_by_key(|(bus, _)| *bus);
        assert_eq!(by_bus[0], (1, &vec![10.0, 11.0]));
        assert_eq!(by_bus[1], (9, &vec![20.0, 21.0]));
    }

    /// Two messages may spell one signal name, so the name does not name a
    /// series either.
    #[test]
    fn one_name_in_two_messages_is_two_series() {
        let db = database();
        let signals = collect(
            &db,
            vec![frame(0.0, 0x100, 1, &[10, 0]), frame(0.1, 0x200, 1, &[99])],
        );

        let mut shared: Vec<_> = signals
            .find("Shared")
            .iter()
            .map(|s| (s.message, s.values.clone()))
            .collect();
        shared.sort_by_key(|(message, _)| *message);
        assert_eq!(
            shared,
            [("First", vec![10.0]), ("Second", vec![99.0])],
            "the two Shared signals must not be merged"
        );
    }

    /// Timestamps and values stay parallel and in logging order, which is what
    /// makes the result a time series rather than a bag of numbers.
    #[test]
    fn readings_keep_their_timestamps_in_order() {
        let db = database();
        let signals = collect(
            &db,
            vec![
                frame(1.5, 0x100, 1, &[7, 0]),
                frame(2.5, 0x200, 1, &[0]),
                frame(3.5, 0x100, 1, &[8, 0]),
            ],
        );

        let first = signals
            .iter()
            .find(|s| s.message == "First" && s.name == "Shared")
            .unwrap();
        assert_eq!(
            first.timestamps,
            [1.5, 3.5],
            "the 0x200 frame is not First's"
        );
        assert_eq!(first.values, [7.0, 8.0]);
        assert_eq!(first.len(), 2);
    }

    /// Labels stay parallel to values, including for readings the table does not
    /// name — otherwise `text_at` would drift out of step with `values`.
    #[test]
    fn labels_stay_parallel_to_the_values_they_label() {
        let db = database();
        let signals = collect(
            &db,
            vec![
                frame(0.0, 0x100, 1, &[0, 1]),
                frame(0.1, 0x100, 1, &[0, 7]),
                frame(0.2, 0x100, 1, &[0, 0]),
            ],
        );

        let labelled = signals
            .iter()
            .find(|s| s.name == "Labelled")
            .expect("the labelled signal");
        assert_eq!(labelled.values, [1.0, 7.0, 0.0]);
        assert_eq!(labelled.text_at(0), Some("On"));
        assert_eq!(labelled.text_at(1), None, "7 is not in the table");
        assert_eq!(labelled.text_at(2), Some("Off"));
        assert_eq!(labelled.text_at(3), None, "there is no fourth reading");

        // A signal with no table reports no text at all rather than an empty one.
        let plain = signals
            .iter()
            .find(|s| s.message == "First" && s.name == "Shared")
            .unwrap();
        assert_eq!(plain.text_at(0), None);
    }

    /// Frames the database does not cover contribute nothing, and a series is
    /// only created once a reading arrives for it.
    #[test]
    fn unknown_identifiers_create_no_series() {
        let db = database();
        let signals = collect(
            &db,
            vec![frame(0.0, 0x555, 1, &[1, 2]), frame(0.1, 0x100, 1, &[3, 4])],
        );

        // First's two signals, and nothing from Second or from 0x555.
        assert_eq!(signals.len(), 2);
        assert!(signals.iter().all(|s| s.message == "First"));
        assert!(signals.iter().all(|s| !s.is_empty()));
        assert!(signals.find("Missing").is_empty());

        assert!(collect(&db, Vec::new()).is_empty());
    }

    #[test]
    fn accumulator_preserves_extended_flag_and_blocks_j1939_std_collision() {
        let db = CanDatabase::new(vec![
            MessageDef {
                name: "TSC1".into(),
                id: 0x0C00_0003,
                extended: true,
                length: 1,
                signals: vec![signal("Torque", 0, Vec::new())],
            },
            MessageDef {
                name: "Std100".into(),
                id: 0x100,
                extended: false,
                length: 1,
                signals: vec![signal("StdSig", 0, Vec::new())],
            },
            MessageDef {
                name: "Ext100".into(),
                id: 0x100,
                extended: true,
                length: 1,
                signals: vec![signal("ExtSig", 0, Vec::new())],
            },
        ])
        .with_matching(crate::candb::IdMatching::J1939Pgn);

        let frames = vec![
            CanFrame {
                timestamp: 0.0,
                id: 0x100,
                extended: Some(false),
                bus_channel: 1,
                data: &[11],
            },
            CanFrame {
                timestamp: 0.1,
                id: 0x100,
                extended: Some(true),
                bus_channel: 1,
                data: &[22],
            },
            // Standard 11-bit frame 0x123 has j1939_pgn(0x123) == 0; MUST NOT decode as TSC1!
            CanFrame {
                timestamp: 0.2,
                id: 0x123,
                extended: Some(false),
                bus_channel: 1,
                data: &[99],
            },
            // Extended J1939 TSC1 frame from another ECU MUST decode as TSC1.
            CanFrame {
                timestamp: 0.3,
                id: 0x0C00_0021,
                extended: Some(true),
                bus_channel: 1,
                data: &[55],
            },
        ];

        let signals = collect(&db, frames);
        assert_eq!(signals.find("StdSig")[0].values, vec![11.0]);
        assert_eq!(signals.find("ExtSig")[0].values, vec![22.0]);
        let torque = signals.find("Torque");
        assert_eq!(torque.len(), 1);
        assert_eq!(
            torque[0].values,
            vec![55.0],
            "standard 11-bit frame 0x123 must not match J1939 PGN 0"
        );
    }
}
