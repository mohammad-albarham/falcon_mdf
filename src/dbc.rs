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

use can_dbc::{AttributeValue, ByteOrder, Dbc, MessageId, MultiplexIndicator, ValueType};

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
            .map(|message| {
                let message_id = message.id;
                let mut signals: Vec<SignalDef> = message
                    .signals
                    .iter()
                    .map(|signal| signal_def(&dbc, message_id, signal))
                    .collect();
                for index in 0..signals.len() {
                    apply_extended_multiplex(&dbc, message_id, &mut signals, index)?;
                }
                validate_multiplexor_graph(message_id, &signals)?;
                Ok(MessageDef {
                    name: message.name.clone(),
                    id: message_id.raw() & ID_MASK,
                    extended: matches!(message_id, MessageId::Extended(_)),
                    length: message.size,
                    signals,
                })
            })
            .collect::<Result<Vec<_>>>()?;

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

/// Applies `SG_MUL_VAL_` extended multiplexing to `signal` when the database
/// declares it.
///
/// The named multiplexor must exist in the same message. Nested extended
/// multiplexing — a multiplexor that is itself multiplexed — is now resolved
/// at decode time by walking the multiplexor chain.
fn apply_extended_multiplex(
    dbc: &Dbc,
    message_id: MessageId,
    signals: &mut [SignalDef],
    index: usize,
) -> Result<()> {
    let signal_name = &signals[index].name;
    let Some(extended) = dbc
        .extended_multiplex
        .iter()
        .find(|em| em.message_id == message_id && em.signal_name == *signal_name)
    else {
        return Ok(());
    };

    if !signals
        .iter()
        .any(|s| s.name == extended.multiplexor_signal_name)
    {
        return Err(Mf4Error::unsupported(
            "DBC extended multiplexing (SG_MUL_VAL_)",
            format!(
                "multiplexor signal '{}' not found in message {:#X}",
                extended.multiplexor_signal_name,
                message_id.raw() & ID_MASK
            ),
        ));
    }

    signals[index].multiplexing = Multiplexing::RangeSelected {
        multiplexor: extended.multiplexor_signal_name.clone(),
        ranges: extended
            .mappings
            .iter()
            .map(|mapping| (mapping.min_value, mapping.max_value))
            .collect(),
    };
    Ok(())
}

/// Checks the multiplexor graph of one message for cycles and unbounded depth.
///
/// A signal that is transitively its own multiplexor would make the decoder
/// loop; rejecting it at parse time returns a named error instead.
fn validate_multiplexor_graph(message_id: MessageId, signals: &[SignalDef]) -> Result<()> {
    const MAX_MUX_DEPTH: usize = 64;

    for start in signals {
        let mut current = &start.multiplexing;
        let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut depth = 0;

        loop {
            if depth > MAX_MUX_DEPTH {
                return Err(Mf4Error::unsupported(
                    "DBC multiplexor cycle",
                    format!(
                        "multiplexor chain for '{}' in message {:#X} exceeds depth {MAX_MUX_DEPTH}",
                        start.name,
                        message_id.raw() & ID_MASK
                    ),
                ));
            }

            let multiplexor_name = match current {
                Multiplexing::None | Multiplexing::Switch => break,
                Multiplexing::Selected(_) => signals
                    .iter()
                    .find(|s| s.multiplexing == Multiplexing::Switch)
                    .map(|s| s.name.as_str()),
                Multiplexing::RangeSelected { multiplexor, .. } => Some(multiplexor.as_str()),
            };

            let Some(name) = multiplexor_name else {
                // No message-level switch for a Selected signal; not a cycle,
                // it simply will not match any frame.
                break;
            };

            if !visited.insert(name) {
                return Err(Mf4Error::unsupported(
                    "DBC multiplexor cycle",
                    format!(
                        "signal '{}' in message {:#X} is transitively its own multiplexor",
                        start.name,
                        message_id.raw() & ID_MASK
                    ),
                ));
            }

            let Some(signal) = signals.iter().find(|s| s.name == name) else {
                // Missing multiplexor is reported by apply_extended_multiplex
                // for RangeSelected; here we just stop walking.
                break;
            };
            current = &signal.multiplexing;
            depth += 1;
        }
    }
    Ok(())
}

/// The `VAL_` table for one signal, as raw value and label.
///
/// `can-dbc` holds these apart from the signals, keyed by message and signal
/// name, which is why this is a lookup rather than a field read. A signal may
/// also reference a global `VAL_TABLE_` through the `ValTable` or
/// `GenSigValTable` attribute; per-signal `VAL_` entries override the global
/// table when the same raw value appears in both.
fn value_table(dbc: &Dbc, message_id: MessageId, signal_name: &str) -> Vec<(i64, String)> {
    let mut table = Vec::new();

    if let Some(name) = global_value_table_name(dbc, message_id, signal_name) {
        if let Some(global) = dbc.value_tables.iter().find(|vt| vt.name == name) {
            for description in &global.descriptions {
                table.push((description.id, description.description.clone()));
            }
        }
    }

    if let Some(descriptions) = dbc.value_descriptions_for_signal(message_id, signal_name) {
        for description in descriptions {
            table.retain(|(id, _)| *id != description.id);
            table.push((description.id, description.description.clone()));
        }
    }

    table
}

/// Reads the name of the global `VAL_TABLE_` a signal references, if any.
fn global_value_table_name(dbc: &Dbc, message_id: MessageId, signal_name: &str) -> Option<String> {
    let name_from = |attr_name| {
        dbc.signal_attribute(message_id, signal_name, attr_name)
            .or_else(|| dbc.resolved_signal_attribute(message_id, signal_name, attr_name))
            .and_then(|value| match value {
                AttributeValue::String(name) if !name.is_empty() => Some(name.clone()),
                _ => None,
            })
    };
    name_from("ValTable").or_else(|| name_from("GenSigValTable"))
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

    /// Nested extended multiplexing: a multiplexor that is itself multiplexed
    /// must be resolved at decode time, so the leaf signal only appears when
    /// every level of the chain matches the frame.
    #[test]
    fn nested_extended_multiplexing_is_resolved_at_decode_time() {
        let text = format!(
            "{}\
             SG_MUL_VAL_ 100 Child Parent 1-1 ;\n\
             SG_MUL_VAL_ 100 Nested Child 7-7 ;\n",
            database(
                "SG_ Parent M : 0|8@1+ (1,0) [0|0] \"\" Tester\n \
                 SG_ Child : 8|8@1+ (1,0) [0|0] \"\" Tester\n \
                 SG_ Nested : 16|8@1+ (1,0) [0|0] \"\" Tester"
            )
        );
        let db = CanDatabase::from_dbc(text.as_bytes()).expect("must parse");

        let names = |payload: &[u8]| -> Vec<&str> {
            db.decode(100, payload).iter().map(|s| s.name).collect()
        };

        assert_eq!(names(&[0, 7, 42]), ["Parent"], "Parent != 1, Child absent");
        assert_eq!(
            names(&[1, 3, 42]),
            ["Parent", "Child"],
            "Parent == 1, Child != 7"
        );
        assert_eq!(
            names(&[1, 7, 42]),
            ["Parent", "Child", "Nested"],
            "both conditions satisfied"
        );
    }

    /// A signal that is transitively its own multiplexor must be rejected at
    /// parse time with a named error rather than allowed to loop forever.
    #[test]
    fn cyclic_multiplexor_graph_returns_named_error() {
        let text = format!(
            "{}\
             SG_MUL_VAL_ 100 A B 1-1 ;\n\
             SG_MUL_VAL_ 100 B A 1-1 ;\n",
            database(
                "SG_ A : 0|8@1+ (1,0) [0|0] \"\" Tester\n \
                 SG_ B : 8|8@1+ (1,0) [0|0] \"\" Tester"
            )
        );
        let err = CanDatabase::from_dbc(text.as_bytes()).unwrap_err();
        match err {
            Mf4Error::Unsupported { feature, detail } => {
                assert!(feature.contains("multiplexor cycle"));
                assert!(detail.contains("A") || detail.contains("B"));
            }
            other => panic!("expected Unsupported multiplexor-cycle error, got {other:?}"),
        }
    }
}
