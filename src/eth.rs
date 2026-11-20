//! Ethernet frames out of bus-logged groups.
//!
//! The counterpart to [`crate::bus`] and [`crate::lin`] for Ethernet traffic:
//! a group an Ethernet logger wrote holds one composed `ETH_Frame` channel per
//! field — timestamp, source, destination, EtherType, payload — and this
//! module reads them back as frames.

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup, SignalValues};
use crate::Mf4File;

/// The name an Ethernet frame group composes its fields under.
///
/// The frame fields appear as channels named `ETH_Frame.EtherType`,
/// `ETH_Frame.DataBytes` and so on — the same arrangement `LIN_Frame`
/// uses (see [`crate::lin`]), under its own prefix. `qualify_channel_name`
/// has already reconciled where the prefix is stored by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to
/// look for.
const ETH_PREFIX: &str = "ETH_Frame";

/// One logged Ethernet frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EthFrame<'a> {
    /// Time of the frame, in the group master's units (seconds, in practice).
    pub timestamp: f64,
    /// The source MAC address, or `None` when the file does not record one.
    pub source: Option<[u8; 6]>,
    /// The destination MAC address, or `None` when the file does not record one.
    pub destination: Option<[u8; 6]>,
    /// The EtherType or length field.
    pub ether_type: u16,
    /// Which bus the frame was logged from, or 0 when the file does not say.
    pub bus_channel: u8,
    /// The payload, trimmed to the logged length.
    pub data: &'a [u8],
}

impl<'a> EthFrame<'a> {
    /// Returns the 802.1Q / 802.1ad VLAN Tag Control Information (TCI) when
    /// `ether_type` is `0x8100` (802.1Q) or `0x88A8` (802.1ad) and the payload
    /// carries at least the 4-byte VLAN header.
    pub fn vlan_tci(&self) -> Option<u16> {
        if matches!(self.ether_type, 0x8100 | 0x88A8) && self.data.len() >= 4 {
            Some(u16::from_be_bytes([self.data[0], self.data[1]]))
        } else {
            None
        }
    }

    /// Returns the 12-bit VLAN identifier (VID, 0..=4095) when the frame carries
    /// an 802.1Q / 802.1ad VLAN tag.
    pub fn vlan_id(&self) -> Option<u16> {
        self.vlan_tci().map(|tci| tci & 0x0FFF)
    }

    /// Peels any 802.1Q / 802.1ad VLAN tags (including QinQ double tags) and
    /// returns the inner EtherType and payload slice.
    pub fn inner_ether_type_and_payload(&self) -> (u16, &'a [u8]) {
        let mut et = self.ether_type;
        let mut payload = self.data;
        while matches!(et, 0x8100 | 0x88A8) && payload.len() >= 4 {
            et = u16::from_be_bytes([payload[2], payload[3]]);
            payload = &payload[4..];
        }
        (et, payload)
    }
}

/// Every Ethernet frame of one channel group, in logging order.
#[derive(Debug, Clone)]
pub struct EthFrames {
    /// Frame timestamps.
    pub(crate) timestamps: Vec<f64>,
    /// Source MAC addresses; `None` when the file does not record them.
    pub(crate) sources: Option<Vec<Option<[u8; 6]>>>,
    /// Destination MAC addresses; `None` when the file does not record them.
    pub(crate) destinations: Option<Vec<Option<[u8; 6]>>>,
    /// Frame EtherTypes.
    pub(crate) ether_types: Vec<u16>,
    /// Bus channel per frame; empty when the file does not record one.
    pub(crate) bus_channels: Vec<u8>,
    /// Payload bytes, concatenated.
    pub(crate) payloads: Vec<u8>,
    /// Where each frame's payload starts in `payloads`, with a final entry
    /// holding the total length.
    pub(crate) payload_starts: Vec<usize>,
}

impl EthFrames {
    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.ether_types.len()
    }

    /// Returns true when the group logged no frames.
    pub fn is_empty(&self) -> bool {
        self.ether_types.is_empty()
    }

    /// Returns the frame at `index`, or `None` when it is past the end.
    pub fn get(&self, index: usize) -> Option<EthFrame<'_>> {
        let ether_type = *self.ether_types.get(index)?;
        let from = *self.payload_starts.get(index)?;
        let to = *self.payload_starts.get(index + 1)?;

        Some(EthFrame {
            timestamp: *self.timestamps.get(index)?,
            source: self
                .sources
                .as_ref()
                .and_then(|sources| sources.get(index).copied().flatten()),
            destination: self
                .destinations
                .as_ref()
                .and_then(|destinations| destinations.get(index).copied().flatten()),
            ether_type,
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            data: self.payloads.get(from..to)?,
        })
    }

    /// Returns an iterator over every frame.
    pub fn iter(&self) -> impl Iterator<Item = EthFrame<'_>> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

fn field<'a>(group: &'a ChannelGroup, suffix: &str) -> Option<&'a Channel> {
    let qualified = format!("{ETH_PREFIX}.{suffix}");
    group.find_channel(&qualified)
}

fn require<'a>(group: &'a ChannelGroup, suffix: &str) -> Result<&'a Channel> {
    field(group, suffix).ok_or_else(|| Mf4Error::ChannelNotFound {
        name: format!("{ETH_PREFIX}.{suffix}"),
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

/// Reads fixed 6-byte MAC addresses from a channel, marking invalid samples as `None`.
fn read_macs(
    file: &Mf4File,
    channel: &Channel,
    suffix: &str,
    group_name: &str,
) -> Result<Vec<Option<[u8; 6]>>> {
    let signal = file.signal(channel)?;
    let validity = signal.validity();
    let values = signal.values()?;
    if !matches!(
        values,
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
    ) {
        return Err(Mf4Error::parse_error(format!(
            "MAC address channel '{ETH_PREFIX}.{suffix}' of bus group '{group_name}' did not decode to bytes",
        )));
    }
    let mut macs = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if !is_sample_valid(&validity, i) {
            macs.push(None);
            continue;
        }
        let bytes = values.bytes_at(i).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "MAC address channel '{ETH_PREFIX}.{suffix}' of bus group '{group_name}' did not decode to bytes",
            ))
        })?;
        if bytes.len() < 6 {
            return Err(Mf4Error::parse_error(format!(
                "MAC address channel '{ETH_PREFIX}.{suffix}' of bus group '{group_name}' sample {i} has {} bytes, expected 6",
                bytes.len()
            )));
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&bytes[..6]);
        macs.push(Some(mac));
    }
    Ok(macs)
}

/// Reads every Ethernet frame `group` logged.
pub(crate) fn read_eth_frames(file: &Mf4File, group: &ChannelGroup) -> Result<EthFrames> {
    let ether_type_channel = require(group, "EtherType")?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let (raw_ether_types, et_validity) = scalars_with_validity(file, ether_type_channel)?;
    let (raw_timestamps, ts_validity) = scalars_with_validity(file, master)?;
    let payload_signal = file.signal(payload_channel)?;
    let payload_validity = payload_signal.validity();
    let payload_values = payload_signal.values()?;

    if !matches!(
        payload_values,
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
    ) {
        return Err(Mf4Error::parse_error(format!(
            "payload channel '{ETH_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
            group.acquisition_name
        )));
    }

    let count = raw_ether_types.len();

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // fields with another's payload.
    if raw_timestamps.len() != count || payload_values.len() != count {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} EtherTypes, {} timestamps and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            count,
            raw_timestamps.len(),
            payload_values.len()
        )));
    }

    let raw_lengths = match field(group, "DataLength") {
        Some(channel) => {
            let (values, validity) = scalars_with_validity(file, channel)?;
            if values.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} EtherTypes, {} timestamps, {} lengths and {} payloads; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    count,
                    raw_timestamps.len(),
                    values.len(),
                    payload_values.len()
                )));
            }
            Some((values, validity))
        }
        None => None,
    };

    let raw_sources = match field(group, "Source") {
        Some(channel) => {
            let macs = read_macs(file, channel, "Source", &group.acquisition_name)?;
            if macs.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} source MACs and {} EtherTypes; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    macs.len(),
                    count
                )));
            }
            Some(macs)
        }
        None => None,
    };

    let raw_destinations = match field(group, "Destination") {
        Some(channel) => {
            let macs = read_macs(file, channel, "Destination", &group.acquisition_name)?;
            if macs.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} destination MACs and {} EtherTypes; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    macs.len(),
                    count
                )));
            }
            Some(macs)
        }
        None => None,
    };

    let raw_bus_channels = match field(group, "BusChannel") {
        Some(channel) => {
            let (values, validity) = scalars_with_validity(file, channel)?;
            if values.len() != count {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} bus channels and {} EtherTypes; \
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

    let mut timestamps = Vec::with_capacity(count);
    let mut sources = raw_sources.as_ref().map(|_| Vec::with_capacity(count));
    let mut destinations = raw_destinations.as_ref().map(|_| Vec::with_capacity(count));
    let mut ether_types = Vec::with_capacity(count);
    let mut bus_channels = if raw_bus_channels.is_some() {
        Vec::with_capacity(count)
    } else {
        Vec::new()
    };
    let mut payloads = Vec::new();
    let mut payload_starts = Vec::with_capacity(count + 1);

    for index in 0..count {
        let len_ok = raw_lengths
            .as_ref()
            .is_none_or(|(vals, val)| is_sample_valid(val, index) && vals[index].is_finite());
        if !is_sample_valid(&ts_validity, index)
            || !raw_timestamps[index].is_finite()
            || !is_sample_valid(&et_validity, index)
            || !raw_ether_types[index].is_finite()
            || !is_sample_valid(&payload_validity, index)
            || !len_ok
        {
            continue;
        }
        let sample = payload_values.bytes_at(index).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "payload channel '{ETH_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
                group.acquisition_name
            ))
        })?;
        let slice = match &raw_lengths {
            Some((vals, _)) => {
                let len = vals[index].max(0.0) as usize;
                &sample[..len.min(sample.len())]
            }
            None => sample,
        };
        timestamps.push(raw_timestamps[index]);
        ether_types.push(raw_ether_types[index] as u16);
        if let (Some(dst_vec), Some(src_vec)) = (&mut sources, &raw_sources) {
            dst_vec.push(src_vec[index]);
        }
        if let (Some(dst_vec), Some(src_vec)) = (&mut destinations, &raw_destinations) {
            dst_vec.push(src_vec[index]);
        }
        if let Some((buses, bus_val)) = &raw_bus_channels {
            let bus = if is_sample_valid(bus_val, index) && buses[index].is_finite() {
                buses[index] as u8
            } else {
                0
            };
            bus_channels.push(bus);
        }
        payload_starts.push(payloads.len());
        payloads.extend_from_slice(slice);
    }
    payload_starts.push(payloads.len());

    Ok(EthFrames {
        timestamps,
        sources,
        destinations,
        ether_types,
        bus_channels,
        payloads,
        payload_starts,
    })
}

/// Returns true if `group` holds Ethernet frames this module can read.
pub(crate) fn is_eth_frame_group(group: &ChannelGroup) -> bool {
    field(group, "EtherType").is_some()
        && field(group, "DataBytes").is_some()
        && group.master_channel().is_some()
}
