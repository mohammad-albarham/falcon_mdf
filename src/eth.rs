//! Ethernet frames out of bus-logged groups.
//!
//! The counterpart to [`crate::bus`] and [`crate::lin`] for Ethernet traffic:
//! a group an Ethernet logger wrote holds one composed `ETH_Frame` channel per
//! field — timestamp, source, destination, EtherType, payload — and this
//! module reads them back as frames.

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup};
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

/// Every Ethernet frame of one channel group, in logging order.
#[derive(Debug, Clone)]
pub struct EthFrames {
    /// Frame timestamps.
    pub(crate) timestamps: Vec<f64>,
    /// Source MAC addresses; `None` when the file does not record them.
    pub(crate) sources: Option<Vec<[u8; 6]>>,
    /// Destination MAC addresses; `None` when the file does not record them.
    pub(crate) destinations: Option<Vec<[u8; 6]>>,
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
            source: match &self.sources {
                Some(sources) => sources.get(index).copied(),
                None => None,
            },
            destination: match &self.destinations {
                Some(destinations) => destinations.get(index).copied(),
                None => None,
            },
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

/// Reads a scalar frame field as integers.
///
/// Every frame field is a small unsigned integer, so `f64` represents each
/// one exactly and one path covers all of their widths.
fn scalars(file: &Mf4File, channel: &Channel) -> Result<Vec<f64>> {
    Ok(file.signal(channel)?.values()?.to_f64())
}

/// Reads fixed 6-byte MAC addresses from a channel.
fn read_macs(
    file: &Mf4File,
    channel: &Channel,
    suffix: &str,
    group_name: &str,
) -> Result<Vec<[u8; 6]>> {
    let values = file.signal(channel)?.values()?;
    let mut macs = Vec::with_capacity(values.len());
    for i in 0..values.len() {
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
        macs.push(mac);
    }
    Ok(macs)
}

/// Reads every Ethernet frame `group` logged.
pub(crate) fn read_eth_frames(file: &Mf4File, group: &ChannelGroup) -> Result<EthFrames> {
    let ether_type_channel = require(group, "EtherType")?;
    let length_channel = require(group, "DataLength")?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let ether_types: Vec<u16> = scalars(file, ether_type_channel)?
        .into_iter()
        .map(|et| et as u16)
        .collect();
    let timestamps = scalars(file, master)?;
    let lengths: Vec<usize> = scalars(file, length_channel)?
        .into_iter()
        .map(|len| len as usize)
        .collect();
    let payload_values = file.signal(payload_channel)?.values()?;

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // fields with another's payload.
    if timestamps.len() != ether_types.len()
        || lengths.len() != ether_types.len()
        || payload_values.len() != ether_types.len()
    {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} EtherTypes, {} timestamps, {} lengths and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            ether_types.len(),
            timestamps.len(),
            lengths.len(),
            payload_values.len()
        )));
    }

    let sources = match field(group, "Source") {
        Some(channel) => {
            let macs = read_macs(file, channel, "Source", &group.acquisition_name)?;
            if macs.len() != ether_types.len() {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} source MACs and {} EtherTypes; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    macs.len(),
                    ether_types.len()
                )));
            }
            Some(macs)
        }
        None => None,
    };

    let destinations = match field(group, "Destination") {
        Some(channel) => {
            let macs = read_macs(file, channel, "Destination", &group.acquisition_name)?;
            if macs.len() != ether_types.len() {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} destination MACs and {} EtherTypes; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    macs.len(),
                    ether_types.len()
                )));
            }
            Some(macs)
        }
        None => None,
    };

    let bus_channels = match field(group, "BusChannel") {
        Some(channel) => {
            let buses: Vec<u8> = scalars(file, channel)?
                .into_iter()
                .map(|bus| bus as u8)
                .collect();
            if buses.len() != ether_types.len() {
                return Err(Mf4Error::parse_error(format!(
                    "bus group '{}' has {} bus channels and {} EtherTypes; \
                     frame fields must agree sample for sample",
                    group.acquisition_name,
                    buses.len(),
                    ether_types.len()
                )));
            }
            buses
        }
        None => Vec::new(),
    };

    // The payloads are flattened here rather than kept as the decoded
    // `SignalValues`: each one is trimmed to the length the record logged,
    // and a fixed-width DataBytes channel pads short frames out to its full
    // width. `starts` has one entry per frame plus a final total, like
    // `SignalValues::VarBytes`.
    let mut payloads = Vec::new();
    let mut payload_starts = Vec::with_capacity(ether_types.len() + 1);
    for (index, &len) in lengths.iter().enumerate() {
        let sample = payload_values.bytes_at(index).ok_or_else(|| {
            Mf4Error::parse_error(format!(
                "payload channel '{ETH_PREFIX}.DataBytes' of bus group '{}' did not decode to bytes",
                group.acquisition_name
            ))
        })?;
        payload_starts.push(payloads.len());
        payloads.extend_from_slice(&sample[..len.min(sample.len())]);
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
    field(group, "EtherType").is_some() && field(group, "DataBytes").is_some()
}
