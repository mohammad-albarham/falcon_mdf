//! A CAN database, and the decoder that turns frame payloads into signals.
//!
//! [`crate::bus`] gets as far as the frame: an identifier and eight opaque bytes.
//! What those bytes mean is not in the MF4 file at all — it is in a database the
//! user supplies. This module holds the database in a form neither DBC nor ARXML
//! specific, and the decoder that reads a payload against it.
//!
//! The front ends live elsewhere and both produce a [`CanDatabase`]:
//!
//! - `crate::dbc`, behind the `dbc` feature, parses DBC files.
//! - `crate::arxml`, behind the `arxml` feature, reads AUTOSAR ECU extracts.
//!
//! Named rather than linked because the modules do not exist in a build that
//! does not enable them, and a link to one would not resolve there.
//!
//! Keeping the decoder here is what stops there being two of it. Bit extraction
//! and scaling are identical whichever file the definitions came out of, and a
//! second implementation is a second thing to get wrong.

use std::collections::HashMap;

/// Bits of an identifier that are the identifier rather than a flag.
///
/// CAN identifiers are 11 or 29 bits. A database may record the extended flag in
/// bit 31 of the same field; a frame carries it separately, so it is masked off
/// on both sides rather than compared.
pub(crate) const ID_MASK: u32 = 0x1FFF_FFFF;

/// One decoded signal: a name, a physical value and a unit.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSignal<'a> {
    /// Signal name as the database spells it.
    pub name: &'a str,
    /// Physical value, after `raw * factor + offset`.
    pub value: f64,
    /// Physical unit as the database spells it, empty when it gives none.
    pub unit: &'a str,
    /// The label the database's value table gives this reading, if any.
    ///
    /// Enum-valued signals — gear positions, fault codes, state machines — carry
    /// a table mapping raw values to names, and the number on its own means
    /// little. `None` when the signal has no table, or has one that does not
    /// cover this particular value; `value` is always populated either way, so a
    /// caller that ignores this field reads exactly what it read before.
    pub text: Option<&'a str>,
}

/// How a signal relates to its message's multiplexor, when it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexing {
    /// The signal is always present.
    None,
    /// The signal's value selects which multiplexed signals apply.
    Switch,
    /// The signal is present only when the switch holds this value.
    Selected(u64),
}

/// Where a signal sits in a payload and how to scale it.
///
/// Byte order decides what `start_bit` means, and it is the part most easily got
/// wrong:
///
/// - **Little-endian (Intel).** `start_bit` is the signal's least significant
///   bit, in a numbering that runs LSB-first within each byte and then to the
///   next byte. The signal grows upwards from there.
/// - **Big-endian (Motorola).** `start_bit` names the signal's *most* significant
///   bit in that same numbering, but the signal grows *downwards* through the
///   byte and continues at the top of the next one.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalDef {
    /// Signal name.
    pub name: String,
    /// Bit position, interpreted according to `big_endian`.
    pub start_bit: u64,
    /// Width in bits, at most 64.
    pub size: u64,
    /// Whether the signal is stored most-significant-byte first.
    pub big_endian: bool,
    /// Whether the raw value is two's-complement signed.
    pub signed: bool,
    /// Multiplier applied to the raw value.
    pub factor: f64,
    /// Constant added after scaling.
    pub offset: f64,
    /// Physical unit, empty when the database gives none.
    pub unit: String,
    /// The signal's place in its message's multiplexing, if any.
    pub multiplexing: Multiplexing,
    /// Labels for raw values, from a DBC `VAL_` table. Empty when there is none.
    ///
    /// Keyed by the raw value *after* sign extension but *before* scaling, which
    /// is what a `VAL_` entry names. Held as a list rather than a map because
    /// these tables run to a handful of entries, where a linear scan beats
    /// hashing and costs nothing to build.
    pub value_table: Vec<(i64, String)>,
}

/// One message: an identifier and the signals packed into its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageDef {
    /// Message name.
    pub name: String,
    /// CAN identifier, without any extended-flag bit.
    pub id: u32,
    /// Whether the message uses a 29-bit identifier.
    pub extended: bool,
    /// Payload length in bytes as the database declares it.
    pub length: u64,
    /// Signals in the payload.
    pub signals: Vec<SignalDef>,
}

/// How a frame's identifier is matched against the database's messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdMatching {
    /// Compare the whole identifier. What a plain CAN database means.
    #[default]
    Exact,
    /// Fall back to comparing J1939 parameter group numbers.
    ///
    /// A J1939 message's 29-bit identifier carries a priority and the sending
    /// ECU's source address alongside the parameter group number, and neither is
    /// part of what the message *is*. A database written against one ECU
    /// therefore matches no frame from any other under [`IdMatching::Exact`] —
    /// the low byte differs per ECU, so a real J1939 log decodes to nothing.
    ///
    /// Exact matches still win; the parameter group is only consulted when no
    /// message carries the identifier itself.
    J1939Pgn,
}

/// The J1939 parameter group number encoded in a 29-bit identifier.
///
/// Bits 24 and 25 are the data page and extended data page, bits 16..24 the PDU
/// format, bits 8..16 the PDU specific field. Priority (bits 26..29) and source
/// address (bits 0..8) are not part of the group and are dropped.
///
/// The PDU format decides what the PDU specific field means, and it is the part
/// worth stating: below 240 the message is addressed to one ECU (PDU1) and that
/// field holds the *destination*, which is no more part of the group than the
/// source is. At 240 and above it is a broadcast (PDU2) and the field is a group
/// extension, which is.
fn j1939_pgn(id: u32) -> u32 {
    let pages = (id >> 24) & 0b11;
    let pdu_format = (id >> 16) & 0xFF;
    let pdu_specific = (id >> 8) & 0xFF;

    if pdu_format < 240 {
        (pages << 16) | (pdu_format << 8)
    } else {
        (pages << 16) | (pdu_format << 8) | pdu_specific
    }
}

/// A CAN database: messages indexed by identifier.
///
/// Decoding a frame is a hash lookup rather than a scan, because a bus log holds
/// millions of frames and a database holds hundreds of messages.
#[derive(Debug, Clone, Default)]
pub struct CanDatabase {
    messages: Vec<MessageDef>,
    by_id: HashMap<u32, usize>,
    matching: IdMatching,
    /// Populated only under [`IdMatching::J1939Pgn`]; empty otherwise.
    by_pgn: HashMap<u32, usize>,
}

impl CanDatabase {
    /// Builds a database from message definitions.
    ///
    /// Public so that a caller with definitions from somewhere this crate does
    /// not parse can still use the decoder.
    pub fn new(messages: Vec<MessageDef>) -> Self {
        let by_id = messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.id & ID_MASK, index))
            .collect();
        CanDatabase {
            messages,
            by_id,
            matching: IdMatching::Exact,
            by_pgn: HashMap::new(),
        }
    }

    /// Sets how frame identifiers are matched against this database's messages.
    ///
    /// Selected per database rather than inferred: whether a 29-bit identifier is
    /// a J1939 parameter group or an ordinary extended identifier is not
    /// something the bits can be asked, and guessing would silently merge
    /// messages in a database that is not J1939 at all.
    ///
    /// ```
    /// use falcon_mdf::candb::{CanDatabase, IdMatching, MessageDef};
    ///
    /// // EEC1 as a J1939 DBC writes it: priority 3, PGN 0xF004, source 0xFE.
    /// let db = CanDatabase::new(vec![MessageDef {
    ///     name: "EEC1".into(),
    ///     id: 0x0CF0_04FE,
    ///     extended: true,
    ///     length: 8,
    ///     signals: Vec::new(),
    /// }])
    /// .with_matching(IdMatching::J1939Pgn);
    ///
    /// // The same parameter group broadcast by a different ECU, at a different
    /// // priority, still names EEC1.
    /// assert_eq!(db.message_name(0x18F0_0400), Some("EEC1"));
    /// ```
    pub fn with_matching(mut self, matching: IdMatching) -> Self {
        self.by_pgn = match matching {
            IdMatching::Exact => HashMap::new(),
            // Built in reverse so that the first message wins a collision, which
            // is what a database listing one parameter group for several source
            // addresses should decode as: the same signals either way.
            IdMatching::J1939Pgn => self
                .messages
                .iter()
                .enumerate()
                .rev()
                .map(|(index, message)| (j1939_pgn(message.id & ID_MASK), index))
                .collect(),
        };
        self.matching = matching;
        self
    }

    /// Returns how this database matches frame identifiers.
    pub fn matching(&self) -> IdMatching {
        self.matching
    }

    /// Returns every message in the database.
    pub fn messages(&self) -> &[MessageDef] {
        &self.messages
    }

    /// Returns the message an identifier names, if the database has one.
    pub fn message(&self, id: u32) -> Option<&MessageDef> {
        Some(&self.messages[self.message_index(id)?])
    }

    /// Returns the name of the message an identifier belongs to.
    pub fn message_name(&self, id: u32) -> Option<&str> {
        Some(self.message(id)?.name.as_str())
    }

    /// Decodes every signal of the message this identifier names.
    ///
    /// Returns an empty vector when the database has no message for `id`, which
    /// is the ordinary case for a log recorded on a bus the database covers only
    /// part of.
    ///
    /// A signal whose bits fall outside `payload` is skipped rather than read as
    /// zeroes: a short frame means those bits were not transmitted, and a value
    /// invented for them would be indistinguishable from one that was measured.
    pub fn decode(&self, id: u32, payload: &[u8]) -> Vec<DecodedSignal<'_>> {
        let mut out = match self.message(id) {
            Some(message) => Vec::with_capacity(message.signals.len()),
            None => return Vec::new(),
        };
        self.decode_each(id, payload, &mut |_, _, signal| out.push(signal));
        out
    }

    /// Decodes a frame, handing each signal to `sink` as it is read.
    ///
    /// [`CanDatabase::decode`] is written in terms of this. Accumulating a bus
    /// log into time series goes through it directly instead, because a log
    /// holds millions of frames and the vector `decode` returns would be one
    /// allocation per frame.
    ///
    /// `sink` receives the message's index in [`CanDatabase::messages`] and the
    /// signal's index within that message, which together identify the series a
    /// reading belongs to — a name cannot, since two messages may spell one.
    pub(crate) fn decode_each<'a>(
        &'a self,
        id: u32,
        payload: &[u8],
        sink: &mut dyn FnMut(usize, usize, DecodedSignal<'a>),
    ) {
        let Some(message_index) = self.message_index(id) else {
            return;
        };
        let message = &self.messages[message_index];

        // Which multiplexed signals apply is decided by the switch signal's own
        // value, so it has to be decoded before the rest can be filtered.
        let switch = message
            .signals
            .iter()
            .find(|signal| signal.multiplexing == Multiplexing::Switch)
            .and_then(|signal| raw_value(signal, payload));

        for (signal_index, signal) in message.signals.iter().enumerate() {
            let selected = match signal.multiplexing {
                Multiplexing::None | Multiplexing::Switch => true,
                Multiplexing::Selected(want) => switch == Some(want),
            };
            if !selected {
                continue;
            }
            let Some(raw) = raw_value(signal, payload) else {
                continue;
            };
            sink(
                message_index,
                signal_index,
                DecodedSignal {
                    name: &signal.name,
                    value: scale(signal, raw),
                    unit: &signal.unit,
                    text: label(signal, raw),
                },
            );
        }
    }

    /// Index into [`CanDatabase::messages`] of the message an identifier names.
    fn message_index(&self, id: u32) -> Option<usize> {
        let id = id & ID_MASK;
        if let Some(&index) = self.by_id.get(&id) {
            return Some(index);
        }
        if self.matching == IdMatching::J1939Pgn {
            return self.by_pgn.get(&j1939_pgn(id)).copied();
        }
        None
    }
}

/// Looks a raw reading up in the signal's value table.
fn label(signal: &SignalDef, raw: u64) -> Option<&str> {
    if signal.value_table.is_empty() {
        return None;
    }
    // A `VAL_` entry names the raw value as the signal's own type reads it, so
    // a signed signal has to be sign-extended first. The cast is lossy only for
    // an unsigned signal wider than 63 bits, which no value table describes.
    let key = if signal.signed {
        sign_extend(raw, signal.size)
    } else {
        raw as i64
    };
    signal
        .value_table
        .iter()
        .find(|(value, _)| *value == key)
        .map(|(_, text)| text.as_str())
}

/// Applies a signal's sign, factor and offset to its raw bits.
fn scale(signal: &SignalDef, raw: u64) -> f64 {
    let value = if signal.signed {
        sign_extend(raw, signal.size) as f64
    } else {
        raw as f64
    };
    value * signal.factor + signal.offset
}

/// Reinterprets the low `bits` of `raw` as a two's-complement signed value.
fn sign_extend(raw: u64, bits: u64) -> i64 {
    if bits == 0 || bits >= 64 {
        return raw as i64;
    }
    let sign_bit = 1u64 << (bits - 1);
    if raw & sign_bit == 0 {
        raw as i64
    } else {
        // Set every bit above the signal's width, which is what widening a
        // negative two's-complement number to 64 bits means.
        (raw | !((1u64 << bits) - 1)) as i64
    }
}

/// Extracts a signal's raw bits from a payload, or `None` if it does not fit.
fn raw_value(signal: &SignalDef, payload: &[u8]) -> Option<u64> {
    let size = signal.size;
    if size == 0 || size > 64 {
        return None;
    }
    let available = (payload.len() as u64).checked_mul(8)?;

    if signal.big_endian {
        // The MSB's position when bits are counted most-significant-first within
        // each byte, which is the order a big-endian signal runs in.
        let msb = (signal.start_bit / 8) * 8 + (7 - signal.start_bit % 8);
        if msb.checked_add(size)? > available {
            return None;
        }
        let mut value = 0u64;
        for i in 0..size {
            let bit = msb + i;
            let byte = payload[(bit / 8) as usize];
            let set = (byte >> (7 - bit % 8)) & 1;
            value = (value << 1) | u64::from(set);
        }
        Some(value)
    } else {
        if signal.start_bit.checked_add(size)? > available {
            return None;
        }
        let mut value = 0u64;
        for i in 0..size {
            let bit = signal.start_bit + i;
            let byte = payload[(bit / 8) as usize];
            let set = (byte >> (bit % 8)) & 1;
            value |= u64::from(set) << i;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(start_bit: u64, size: u64, big_endian: bool) -> SignalDef {
        SignalDef {
            name: "Probe".into(),
            start_bit,
            size,
            big_endian,
            signed: false,
            factor: 1.0,
            offset: 0.0,
            unit: String::new(),
            multiplexing: Multiplexing::None,
            value_table: Vec::new(),
        }
    }

    /// A byte-aligned eight-bit signal cannot tell the byte orders apart: it
    /// occupies one whole byte either way. Any error in either bit numbering
    /// shows up as the two disagreeing, which makes this a check on the numbering
    /// itself rather than on a value someone chose.
    #[test]
    fn the_byte_orders_agree_on_a_byte_aligned_signal() {
        let payload = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];

        for byte in 0..8u64 {
            // Little-endian names the LSB of the byte, big-endian the MSB.
            let little = raw_value(&probe(byte * 8, 8, false), &payload).unwrap();
            let big = raw_value(&probe(byte * 8 + 7, 8, true), &payload).unwrap();

            assert_eq!(little, big, "byte {byte}: the orders disagree");
            assert_eq!(
                little,
                u64::from(payload[byte as usize]),
                "byte {byte}: neither order read the byte itself"
            );
        }
    }

    /// The same holds for a single bit: one bit is one bit whichever way the
    /// surrounding bits are counted, and both orders name it the same way.
    #[test]
    fn the_byte_orders_agree_on_a_single_bit() {
        let payload = [0b1010_0101, 0x00, 0xFF, 0x0F];

        for bit in 0..payload.len() as u64 * 8 {
            let expected = u64::from(payload[(bit / 8) as usize] >> (bit % 8) & 1);
            assert_eq!(raw_value(&probe(bit, 1, false), &payload), Some(expected));
            assert_eq!(raw_value(&probe(bit, 1, true), &payload), Some(expected));
        }
    }

    /// A 16-bit signal spanning two bytes is where the orders finally differ, and
    /// where getting the numbering backwards yields a byte-swapped value.
    #[test]
    fn a_multi_byte_signal_distinguishes_the_orders() {
        let payload = [0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

        assert_eq!(raw_value(&probe(7, 16, true), &payload), Some(0x0102));
        assert_eq!(raw_value(&probe(0, 16, false), &payload), Some(0x0201));
    }

    #[test]
    fn a_signal_beyond_the_payload_is_not_invented() {
        let payload = [0xFF, 0xFF, 0xFF];

        assert_eq!(raw_value(&probe(16, 16, false), &payload), None);
        assert_eq!(raw_value(&probe(8, 16, false), &payload), Some(0xFFFF));
    }

    #[test]
    fn signed_signals_are_sign_extended_at_their_own_width() {
        assert_eq!(sign_extend(0b0111, 4), 7);
        assert_eq!(sign_extend(0b1111, 4), -1);
        assert_eq!(sign_extend(0b1000, 4), -8);
        assert_eq!(sign_extend(0x7F, 8), 127);
        assert_eq!(sign_extend(0x80, 8), -128);
        assert_eq!(sign_extend(u64::MAX, 64), -1);
    }

    /// Multiplexing selects the signals the switch names and no others.
    #[test]
    fn multiplexing_selects_by_the_switch_value() {
        let mut switch = probe(0, 8, false);
        switch.name = "Switch".into();
        switch.multiplexing = Multiplexing::Switch;

        let mut first = probe(8, 8, false);
        first.name = "WhenOne".into();
        first.multiplexing = Multiplexing::Selected(1);

        let mut second = probe(8, 8, false);
        second.name = "WhenTwo".into();
        second.multiplexing = Multiplexing::Selected(2);

        let db = CanDatabase::new(vec![MessageDef {
            name: "Muxed".into(),
            id: 0x100,
            extended: false,
            length: 2,
            signals: vec![switch, first, second],
        }]);

        let names = |payload: &[u8]| -> Vec<String> {
            db.decode(0x100, payload)
                .iter()
                .map(|s| s.name.to_string())
                .collect()
        };

        assert_eq!(names(&[1, 42]), ["Switch", "WhenOne"]);
        assert_eq!(names(&[2, 42]), ["Switch", "WhenTwo"]);
        assert_eq!(names(&[3, 42]), ["Switch"], "no branch matches switch 3");
    }

    /// The parameter group numbers below are published in SAE J1939-71 and
    /// J1939-21, not derived from this code. The PDU1 entries are the ones that
    /// matter: their low byte is a destination address, so a decoder that keeps
    /// it reports a different group for every ECU addressed.
    #[test]
    fn parameter_groups_match_the_published_numbers() {
        // (identifier, PGN, name) — identifiers as a logger records them.
        let cases = [
            (0x0CF0_0400u32, 0xF004u32, "EEC1, PDU2"),
            (0x18FE_E500, 0xFEE5, "engine hours, PDU2"),
            (0x18FE_CA00, 0xFECA, "DM1, PDU2"),
            (
                0x18EA_004A,
                0xEA00,
                "request, PDU1 — destination 0x00 dropped",
            ),
            (0x18EE_FF00, 0xEE00, "address claimed, PDU1"),
            (0x0CEF_0B0F, 0xEF00, "proprietary A, PDU1"),
            (0x0C00_0304, 0x0000, "TSC1, PDU1 — the group really is zero"),
        ];

        for (id, pgn, what) in cases {
            assert_eq!(j1939_pgn(id), pgn, "{what}: {id:#X}");
        }
    }

    /// Priority and source address are not part of the group, so changing either
    /// must not change the answer. The destination of a PDU1 message is not
    /// either; the group extension of a PDU2 message is.
    #[test]
    fn only_the_group_defining_bits_reach_the_parameter_group() {
        // EEC1 at every priority and from every source address.
        for priority in 0..8u32 {
            for source in [0x00u32, 0x21, 0xFE] {
                let id = (priority << 26) | 0x00F0_0400 | source;
                assert_eq!(j1939_pgn(id), 0xF004, "{id:#X}");
            }
        }

        // A PDU1 message keeps its group whichever ECU it is addressed to...
        assert_eq!(j1939_pgn(0x18EA_0000), j1939_pgn(0x18EA_FF00));
        // ...while a PDU2 message's group extension is load-bearing.
        assert_ne!(j1939_pgn(0x18FE_E500), j1939_pgn(0x18FE_E600));

        // The data page bit is part of the group and sits above the PDU format.
        assert_eq!(j1939_pgn(0x0DF0_0400), 0x1F004);
    }

    /// The defect T3 exists to fix, in miniature: a database written against one
    /// ECU matches nothing sent by another until PGN matching is asked for.
    #[test]
    fn j1939_matching_ignores_the_source_address() {
        let messages = vec![MessageDef {
            name: "EEC1".into(),
            id: 0x0CF0_04FE,
            extended: true,
            length: 8,
            signals: vec![probe(24, 16, false)],
        }];

        let exact = CanDatabase::new(messages.clone());
        assert_eq!(exact.matching(), IdMatching::Exact);
        assert_eq!(exact.message_name(0x0CF0_0400), None, "a different ECU");
        assert!(exact.decode(0x0CF0_0400, &[0xFF; 8]).is_empty());

        let j1939 = CanDatabase::new(messages).with_matching(IdMatching::J1939Pgn);
        assert_eq!(j1939.message_name(0x0CF0_0400), Some("EEC1"));
        assert_eq!(j1939.message_name(0x0CF0_0421), Some("EEC1"), "and another");
        assert_eq!(j1939.decode(0x0CF0_0400, &[0xFF; 8]).len(), 1);

        // A group the database does not cover is still not invented.
        assert_eq!(j1939.message_name(0x18FE_E500), None);
    }

    /// PGN matching must not shadow an identifier the database spells out, or
    /// enabling it would change the meaning of messages that were matching
    /// correctly before.
    #[test]
    fn an_exact_identifier_wins_over_its_parameter_group() {
        let db = CanDatabase::new(vec![
            MessageDef {
                name: "Generic".into(),
                id: 0x0CF0_04FE,
                extended: true,
                length: 8,
                signals: Vec::new(),
            },
            MessageDef {
                name: "ThisEcuExactly".into(),
                id: 0x0CF0_0421,
                extended: true,
                length: 8,
                signals: Vec::new(),
            },
        ])
        .with_matching(IdMatching::J1939Pgn);

        assert_eq!(db.message_name(0x0CF0_0421), Some("ThisEcuExactly"));
        assert_eq!(db.message_name(0x0CF0_0400), Some("Generic"));
    }

    /// A value table names a raw reading, so the label has to follow the raw
    /// bits rather than the scaled value.
    #[test]
    fn a_value_table_labels_the_raw_reading() {
        let mut gear = probe(0, 8, false);
        gear.name = "Gear".into();
        gear.signed = true;
        gear.factor = 10.0;
        gear.value_table = vec![
            (0, "Neutral".into()),
            (-1, "Reverse".into()),
            (1, "First".into()),
        ];

        let db = CanDatabase::new(vec![MessageDef {
            name: "Transmission".into(),
            id: 0x200,
            extended: false,
            length: 1,
            signals: vec![gear],
        }]);

        let decoded = |byte: u8| -> (f64, Option<String>) {
            let signal = db.decode(0x200, &[byte]).remove(0);
            (signal.value, signal.text.map(str::to_string))
        };

        assert_eq!(decoded(0), (0.0, Some("Neutral".into())));
        assert_eq!(decoded(1), (10.0, Some("First".into())));
        // Sign extension first: 0xFF is -1, whose label is Reverse, and whose
        // scaled value is -10. Reading it unsigned would look up 255 and find
        // nothing.
        assert_eq!(decoded(0xFF), (-10.0, Some("Reverse".into())));
        // A value the table does not cover is left unlabelled, not mislabelled.
        assert_eq!(decoded(2), (20.0, None));
    }

    /// A signal with no table decodes exactly as it did before tables existed.
    #[test]
    fn a_signal_without_a_table_has_no_text() {
        let db = CanDatabase::new(vec![MessageDef {
            name: "Plain".into(),
            id: 0x100,
            extended: false,
            length: 1,
            signals: vec![probe(0, 8, false)],
        }]);

        assert_eq!(db.decode(0x100, &[7])[0].text, None);
    }

    #[test]
    fn an_unknown_identifier_decodes_to_nothing() {
        let db = CanDatabase::new(vec![MessageDef {
            name: "Only".into(),
            id: 0x100,
            extended: false,
            length: 8,
            signals: vec![probe(0, 8, false)],
        }]);

        assert!(db.decode(0x101, &[0xFF; 8]).is_empty());
        assert_eq!(db.decode(0x100, &[0xFF; 8]).len(), 1);
    }
}
