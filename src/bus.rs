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

use crate::candb::CanDatabase;
use crate::error::{Mf4Error, Result};
use crate::model::{Channel, ChannelGroup, SignalValues};
use crate::Mf4File;

/// The name a CAN data-frame group composes its fields under.
///
/// The frame fields appear as channels named `CAN_DataFrame.ID`,
/// `CAN_DataFrame.DataBytes` and so on. Writers disagree about whether the
/// prefix is stored on the member channel or only on its composition parent;
/// `qualify_channel_name` has already reconciled that by the time a
/// [`ChannelGroup`] is handed out here, so the qualified name is what to look
/// for.
const CAN_PREFIX: &str = "CAN_DataFrame";

/// Bits of the identifier field that hold the identifier itself.
///
/// CAN identifiers are 11 bits (standard) or 29 bits (extended); nothing above
/// bit 28 is part of the number.
const ID_MASK: u32 = 0x1FFF_FFFF;

/// One logged CAN frame, borrowed from the group it was read out of.
///
/// The payload is not interpreted. Turning `data` into named physical signals
/// needs a CAN database, which this crate does not yet read.
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
/// address a frame.
#[derive(Debug, Clone)]
pub struct CanFrames {
    timestamps: Vec<f64>,
    ids: Vec<u32>,
    extended: Option<Vec<bool>>,
    bus_channels: Vec<u8>,
    payloads: SignalValues,
    /// Logged payload length per frame, present when the group records it.
    /// A fixed-width payload channel pads short frames out to its full width,
    /// and this is what trims them back.
    lengths: Option<Vec<u8>>,
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
            extended: match &self.extended {
                Some(flags) => flags.get(index).copied(),
                None => None,
            },
            bus_channel: self.bus_channels.get(index).copied().unwrap_or(0),
            data,
        })
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

        database.decode_each(frame.id, frame.data, &mut |message, signal, decoded| {
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
        });
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
/// Detection is by the frame channels being present rather than by the
/// `cg_flags` bus-event bit, because the channels are what reading actually
/// needs. The flag itself is available as
/// [`ChannelGroup::is_bus_event`].
pub(crate) fn is_can_frame_group(group: &ChannelGroup) -> bool {
    field(group, "ID").is_some() && field(group, "DataBytes").is_some()
}

fn field<'a>(group: &'a ChannelGroup, suffix: &str) -> Option<&'a Channel> {
    let qualified = format!("{CAN_PREFIX}.{suffix}");
    group.find_channel(&qualified)
}

fn require<'a>(group: &'a ChannelGroup, suffix: &str) -> Result<&'a Channel> {
    field(group, suffix).ok_or_else(|| Mf4Error::ChannelNotFound {
        name: format!("{CAN_PREFIX}.{suffix}"),
    })
}

/// Reads a scalar frame field as integers.
///
/// Every frame field is a small unsigned integer, so `f64` represents each one
/// exactly and one path covers all of their widths.
fn scalars(file: &Mf4File, channel: &Channel) -> Result<Vec<f64>> {
    Ok(file.signal(channel)?.values()?.to_f64())
}

pub(crate) fn read_can_frames(file: &Mf4File, group: &ChannelGroup) -> Result<CanFrames> {
    let id_channel = require(group, "ID")?;
    let payload_channel = require(group, "DataBytes")?;
    let master = group
        .master_channel()
        .ok_or_else(|| Mf4Error::ChannelNotFound {
            name: format!("master channel of bus group '{}'", group.acquisition_name),
        })?;

    let ids: Vec<u32> = scalars(file, id_channel)?
        .into_iter()
        .map(|id| id as u32 & ID_MASK)
        .collect();
    let timestamps = scalars(file, master)?;
    let payloads = file.signal(payload_channel)?.values()?;

    // A frame is only assembled correctly if its fields line up sample for
    // sample. Truncating to the shortest would silently pair one frame's
    // identifier with another's payload.
    if timestamps.len() != ids.len() || payloads.len() != ids.len() {
        return Err(Mf4Error::parse_error(format!(
            "bus group '{}' has {} identifiers, {} timestamps and {} payloads; \
             frame fields must agree sample for sample",
            group.acquisition_name,
            ids.len(),
            timestamps.len(),
            payloads.len()
        )));
    }

    let extended = match field(group, "IDE") {
        Some(channel) => Some(
            scalars(file, channel)?
                .into_iter()
                .map(|flag| flag != 0.0)
                .collect(),
        ),
        None => None,
    };
    let bus_channels = match field(group, "BusChannel") {
        Some(channel) => scalars(file, channel)?
            .into_iter()
            .map(|bus| bus as u8)
            .collect(),
        None => Vec::new(),
    };
    let lengths = match field(group, "DataLength") {
        Some(channel) => Some(
            scalars(file, channel)?
                .into_iter()
                .map(|len| len as u8)
                .collect(),
        ),
        None => None,
    };

    Ok(CanFrames {
        timestamps,
        ids,
        extended,
        bus_channels,
        payloads,
        lengths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candb::{MessageDef, Multiplexing, SignalDef};

    fn signal(name: &str, start_bit: u64, value_table: Vec<(i64, String)>) -> SignalDef {
        SignalDef {
            name: name.into(),
            start_bit,
            size: 8,
            big_endian: false,
            signed: false,
            factor: 1.0,
            offset: 0.0,
            unit: "u".into(),
            multiplexing: Multiplexing::None,
            value_table,
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
}
