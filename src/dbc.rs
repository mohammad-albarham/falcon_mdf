//! Reading a DBC file into a [`CanDatabase`].
//!
//! Parsing is [`can_dbc`]'s job; this maps what it produces onto the
//! front-end-neutral definitions in [`crate::candb`], where the decoder lives.
//! `can-dbc` parses and does not decode, which is the division of labour that
//! makes it worth depending on.
//!
//! Behind the `dbc` feature, which is off by default: someone reading plain
//! measurement files should not pay for a CAN database parser.
//!
//! ```no_run
//! use falcon_mdf::{candb::CanDatabase, Mf4File};
//!
//! let file = Mf4File::open("bus_log.mf4")?;
//! let database = CanDatabase::from_dbc_path("engine.dbc")?;
//!
//! for group in file.can_frame_groups() {
//!     for frame in file.can_frames(group)?.iter().take(10) {
//!         for signal in database.decode(frame.id, frame.data) {
//!             println!("{} = {} {}", signal.name, signal.value, signal.unit);
//!         }
//!     }
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```

use std::path::Path;

use can_dbc::{ByteOrder, Dbc, MessageId, MultiplexIndicator, ValueType};

use crate::candb::{CanDatabase, MessageDef, Multiplexing, SignalDef, ID_MASK};
use crate::error::{Mf4Error, Result};

impl CanDatabase {
    /// Loads a database from a DBC file.
    pub fn from_dbc_path(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_dbc(&bytes)
    }

    /// Parses a database from the contents of a DBC file.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::ParseError`] when the database will not parse. The
    /// underlying error is not carried through: `can-dbc`'s error type borrows
    /// from the input, so it cannot outlive this call.
    pub fn from_dbc(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Mf4Error::parse_error(format!("the DBC database is not UTF-8: {e}")))?;
        let dbc = Dbc::try_from(text)
            .map_err(|_| Mf4Error::parse_error("the DBC database could not be parsed"))?;

        let messages = dbc
            .messages
            .iter()
            .map(|message| MessageDef {
                name: message.name.clone(),
                id: message.id.raw() & ID_MASK,
                extended: matches!(message.id, MessageId::Extended(_)),
                length: message.size,
                signals: message
                    .signals
                    .iter()
                    .map(|signal| signal_def(&dbc, message.id, signal))
                    .collect(),
            })
            .collect();

        Ok(CanDatabase::new(messages))
    }
}

fn signal_def(dbc: &Dbc, message_id: MessageId, signal: &can_dbc::Signal) -> SignalDef {
    SignalDef {
        name: signal.name.clone(),
        start_bit: signal.start_bit,
        size: signal.size,
        big_endian: signal.byte_order == ByteOrder::BigEndian,
        signed: signal.value_type == ValueType::Signed,
        factor: signal.factor,
        offset: signal.offset,
        unit: signal.unit.clone(),
        multiplexing: match signal.multiplexer_indicator {
            MultiplexIndicator::Plain => Multiplexing::None,
            MultiplexIndicator::Multiplexor => Multiplexing::Switch,
            MultiplexIndicator::MultiplexedSignal(value)
            | MultiplexIndicator::MultiplexorAndMultiplexedSignal(value) => {
                Multiplexing::Selected(value)
            }
        },
        value_table: value_table(dbc, message_id, &signal.name),
    }
}

/// The `VAL_` table for one signal, as raw value and label.
///
/// `can-dbc` holds these apart from the signals, keyed by message and signal
/// name, which is why this is a lookup rather than a field read.
fn value_table(dbc: &Dbc, message_id: MessageId, signal_name: &str) -> Vec<(i64, String)> {
    let Some(descriptions) = dbc.value_descriptions_for_signal(message_id, signal_name) else {
        return Vec::new();
    };
    descriptions
        .iter()
        .map(|description| (description.id, description.description.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps signal definitions in the smallest DBC the grammar accepts.
    ///
    /// The preamble is not decoration: `Dbc::try_from` requires the version, the
    /// new-symbol list and the bus-speed line before it will look at a message.
    fn database(signals: &str) -> String {
        format!(
            "VERSION \"1\"\n\
             NS_ :\n\
             BS_:\n\
             BU_: Tester\n\
             BO_ 100 Probe: 8 Tester\n    {signals}\n"
        )
    }

    /// The fields the decoder reads must survive the trip through `can-dbc`'s
    /// parser and this mapping — in particular the byte-order and sign flags,
    /// which are single characters in the grammar and easy to invert.
    #[test]
    fn a_dbc_signals_fields_arrive_intact() {
        let text = database(
            "SG_ Speed : 8|12@1- (0.25,-40) [0|100] \"km/h\" Tester\n \
             SG_ Flag : 4|1@0+ (1,0) [0|1] \"\" Tester",
        );
        let db = CanDatabase::from_dbc(text.as_bytes()).expect("must parse");

        let message = db.message(100).expect("message 100");
        assert_eq!(message.name, "Probe");
        assert!(!message.extended);
        assert_eq!(message.length, 8);

        let speed = &message.signals[0];
        assert_eq!(speed.name, "Speed");
        assert_eq!(speed.start_bit, 8);
        assert_eq!(speed.size, 12);
        assert!(!speed.big_endian, "@1 is little-endian");
        assert!(speed.signed, "the trailing - means signed");
        assert_eq!(speed.factor, 0.25);
        assert_eq!(speed.offset, -40.0);
        assert_eq!(speed.unit, "km/h");

        let flag = &message.signals[1];
        assert!(flag.big_endian, "@0 is big-endian");
        assert!(!flag.signed, "the trailing + means unsigned");
    }

    /// An extended message keeps its 29-bit identifier and is marked extended,
    /// rather than carrying `can-dbc`'s bit-31 marker into the identifier.
    #[test]
    fn an_extended_identifier_is_separated_from_its_flag() {
        let text = format!(
            "VERSION \"1\"\nNS_ :\nBS_:\nBU_: Tester\n\
             BO_ {} Extended: 8 Tester\n \
             SG_ Value : 0|8@1+ (1,0) [0|0] \"\" Tester\n",
            0x1FED_CBA9u32 | 1 << 31
        );
        let db = CanDatabase::from_dbc(text.as_bytes()).expect("must parse");

        let message = db.message(0x1FED_CBA9).expect("extended message");
        assert_eq!(message.id, 0x1FED_CBA9);
        assert!(message.extended);
    }

    #[test]
    fn multiplexing_is_carried_across() {
        let text = database(
            "SG_ Mode M : 0|8@1+ (1,0) [0|0] \"\" Tester\n \
             SG_ WhenTwo m2 : 8|8@1+ (1,0) [0|0] \"\" Tester",
        );
        let db = CanDatabase::from_dbc(text.as_bytes()).expect("must parse");
        let signals = &db.message(100).unwrap().signals;

        assert_eq!(signals[0].multiplexing, Multiplexing::Switch);
        assert_eq!(signals[1].multiplexing, Multiplexing::Selected(2));
    }

    /// `VAL_` tables live apart from the signals in a DBC and are keyed by
    /// message and signal name, so the risk is attaching a table to the wrong
    /// signal — which shows up as one signal carrying another's labels.
    #[test]
    fn value_tables_reach_the_signals_they_name() {
        let text = format!(
            "{}VAL_ 100 Gear -1 \"Reverse\" 0 \"Neutral\" 1 \"First\" ;\n",
            database(
                "SG_ Gear : 0|8@1- (1,0) [-1|1] \"\" Tester\n \
                 SG_ Unlabelled : 8|8@1+ (1,0) [0|255] \"\" Tester"
            )
        );
        let db = CanDatabase::from_dbc(text.as_bytes()).expect("must parse");
        let signals = &db.message(100).unwrap().signals;

        let mut table = signals[0].value_table.clone();
        table.sort();
        assert_eq!(
            table,
            [
                (-1, "Reverse".to_string()),
                (0, "Neutral".to_string()),
                (1, "First".to_string()),
            ]
        );
        assert!(
            signals[1].value_table.is_empty(),
            "a signal with no VAL_ line must not inherit its neighbour's"
        );

        // And the labels come back through decoding, against the raw value.
        let decoded = db.decode(100, &[0xFF, 0x00]);
        assert_eq!(decoded[0].text, Some("Reverse"), "0xFF is -1");
        assert_eq!(decoded[0].value, -1.0);
        assert_eq!(decoded[1].text, None);
    }

    #[test]
    fn a_malformed_database_is_refused() {
        assert!(CanDatabase::from_dbc(b"this is not a DBC file").is_err());
    }
}
