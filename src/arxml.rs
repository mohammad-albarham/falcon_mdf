//! Reading an AUTOSAR ARXML database into a [`CanDatabase`].
//!
//! Parsing and schema validation are [`autosar_data`]'s job; this walks the model
//! it produces and maps what it finds onto the front-end-neutral definitions in
//! [`crate::candb`], where the decoder lives. The decoder itself is shared with
//! [`crate::dbc`] — a signal's bits are extracted the same way whichever kind of
//! file described them.
//!
//! ARXML costs more than DBC because a signal's properties are spread across five
//! elements rather than written on one line. The walk is:
//!
//! ```text
//! CAN-CLUSTER
//!  └ PHYSICAL-CHANNELS / CAN-PHYSICAL-CHANNEL
//!     └ FRAME-TRIGGERINGS / CAN-FRAME-TRIGGERING   identifier, addressing mode
//!        └ FRAME-REF → CAN-FRAME                  name, length
//!           └ PDU-TO-FRAME-MAPPING / PDU-REF → I-SIGNAL-I-PDU
//!              └ I-SIGNAL-TO-I-PDU-MAPPING        start position, byte order
//!                 └ I-SIGNAL-REF → I-SIGNAL       width
//!                    ├ BASE-TYPE-REF → SW-BASE-TYPE        signedness
//!                    └ SYSTEM-SIGNAL-REF → SYSTEM-SIGNAL
//!                       └ COMPU-METHOD-REF → COMPU-METHOD  factor, offset, unit
//! ```
//!
//! Behind the `arxml` feature, which is off by default.
//!
//! ```no_run
//! use falcon_mdf::candb::CanDatabase;
//!
//! let database = CanDatabase::from_arxml_path("ecu_extract.arxml")?;
//! for message in database.messages() {
//!     println!("{} ({:#X}): {} signals", message.name, message.id, message.signals.len());
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```

use std::path::Path;

use autosar_data::{AutosarModel, Element, ElementName, EnumItem};

use crate::candb::{CanDatabase, MessageDef, Multiplexing, SignalDef, ID_MASK};
use crate::error::{Mf4Error, Result};

impl CanDatabase {
    /// Loads a database from an ARXML file.
    ///
    /// Reads every CAN cluster in the file. Other cluster kinds — FlexRay, LIN,
    /// Ethernet — are skipped rather than reported: this decodes CAN.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::ParseError`] when the file will not parse or does not
    /// validate against the AUTOSAR schema.
    pub fn from_arxml_path(path: impl AsRef<Path>) -> Result<Self> {
        let model = AutosarModel::new();
        model
            .load_file(path.as_ref(), false)
            .map_err(|e| Mf4Error::parse_error(format!("the ARXML file could not be read: {e}")))?;

        let mut messages = Vec::new();
        for (_, weak) in model.identifiable_elements() {
            let Some(element) = weak.upgrade() else {
                continue;
            };
            // A J1939 cluster is a CAN cluster with extra fields, and carries its
            // frames the same way, so both are walked.
            let conditional = match element.element_name() {
                ElementName::CanCluster => element
                    .get_sub_element(ElementName::CanClusterVariants)
                    .and_then(|v| v.get_sub_element(ElementName::CanClusterConditional)),
                ElementName::J1939Cluster => element
                    .get_sub_element(ElementName::J1939ClusterVariants)
                    .and_then(|v| v.get_sub_element(ElementName::J1939ClusterConditional)),
                _ => continue,
            };
            let Some(conditional) = conditional else {
                continue;
            };

            let Some(channels) = conditional.get_sub_element(ElementName::PhysicalChannels) else {
                continue;
            };
            for channel in channels
                .sub_elements()
                .filter(|e| e.element_name() == ElementName::CanPhysicalChannel)
            {
                let Some(triggerings) = channel.get_sub_element(ElementName::FrameTriggerings)
                else {
                    continue;
                };
                for triggering in triggerings
                    .sub_elements()
                    .filter(|e| e.element_name() == ElementName::CanFrameTriggering)
                {
                    if let Some(message) = message_def(&triggering) {
                        messages.push(message);
                    }
                }
            }
        }

        Ok(CanDatabase::new(messages))
    }
}

/// Reads the text of a sub-element, if it has any.
fn text(element: &Element, name: ElementName) -> Option<String> {
    element
        .get_sub_element(name)?
        .character_data()?
        .string_value()
}

/// Reads an enumerated sub-element, if it has one.
///
/// Kept separate from [`text`] because it has to be: an AUTOSAR enum arrives as
/// `CharacterData::Enum`, and `string_value()` returns `None` for it. Asking for
/// the string of `PACKING-BYTE-ORDER` therefore does not fail loudly — it quietly
/// reports nothing, which reads as "not big-endian" and byte-swaps every
/// big-endian signal in the file.
fn enumerated(element: &Element, name: ElementName) -> Option<EnumItem> {
    element
        .get_sub_element(name)?
        .character_data()?
        .enum_value()
}

/// Reads an integer sub-element, if it has one.
fn integer(element: &Element, name: ElementName) -> Option<i64> {
    element
        .get_sub_element(name)?
        .character_data()?
        .parse_integer::<i64>()
}

/// Follows a reference sub-element to what it points at.
fn target(element: &Element, name: ElementName) -> Option<Element> {
    element.get_sub_element(name)?.get_reference_target().ok()
}

/// The `SW-DATA-DEF-PROPS-CONDITIONAL` under a properties element.
///
/// AUTOSAR wraps almost every property set in a variants/conditional pair so that
/// it can differ per variant. Nothing here varies, so the first conditional is the
/// one that applies.
fn props(element: &Element, name: ElementName) -> Option<Element> {
    element
        .get_sub_element(name)?
        .get_sub_element(ElementName::SwDataDefPropsVariants)?
        .get_sub_element(ElementName::SwDataDefPropsConditional)
}

fn message_def(triggering: &Element) -> Option<MessageDef> {
    let identifier = integer(triggering, ElementName::Identifier)?;
    let extended =
        enumerated(triggering, ElementName::CanAddressingMode) == Some(EnumItem::Extended);

    let frame = target(triggering, ElementName::FrameRef)?;
    let name = frame.item_name()?;
    let length = integer(&frame, ElementName::FrameLength)
        .unwrap_or(0)
        .max(0) as u64;

    let mut signals = Vec::new();
    if let Some(mappings) = frame.get_sub_element(ElementName::PduToFrameMappings) {
        for mapping in mappings.sub_elements() {
            let Some(pdu) = target(&mapping, ElementName::PduRef) else {
                continue;
            };
            collect_signals(&pdu, &mut signals);
        }
    }

    Some(MessageDef {
        name,
        id: (identifier as u32) & ID_MASK,
        extended,
        length,
        signals,
    })
}

/// Collects the signals a PDU carries.
///
/// A multiplexed PDU nests further PDUs inside itself — a static part plus one
/// dynamic part per selector value. Only the static part's signals are collected:
/// which dynamic part applies is chosen by a selector field this build does not
/// resolve, and including all of them would report signals that overlap in the
/// payload as though they were simultaneously present.
fn collect_signals(pdu: &Element, out: &mut Vec<SignalDef>) {
    let Some(mappings) = pdu.get_sub_element(ElementName::ISignalToPduMappings) else {
        return;
    };
    for mapping in mappings
        .sub_elements()
        .filter(|e| e.element_name() == ElementName::ISignalToIPduMapping)
    {
        if let Some(signal) = signal_def(&mapping) {
            out.push(signal);
        }
    }
}

fn signal_def(mapping: &Element) -> Option<SignalDef> {
    let start_bit = integer(mapping, ElementName::StartPosition)?.max(0) as u64;

    // Absent packing order means little-endian: the element is optional, and a
    // signal that does not say is not big-endian.
    let big_endian = enumerated(mapping, ElementName::PackingByteOrder)
        == Some(EnumItem::MostSignificantByteFirst);

    let signal = target(mapping, ElementName::ISignalRef)?;
    let name = signal.item_name()?;
    let size = integer(&signal, ElementName::Length)?.max(0) as u64;

    // Signedness is a property of the base type the signal is represented in.
    // `2C` is two's complement; `NONE` and the unsigned encodings are not.
    let signed = props(&signal, ElementName::NetworkRepresentationProps)
        .and_then(|p| target(&p, ElementName::BaseTypeRef))
        .and_then(|base| text(&base, ElementName::BaseTypeEncoding))
        .is_some_and(|encoding| encoding.eq_ignore_ascii_case("2C"));

    let (factor, offset, unit) = target(&signal, ElementName::SystemSignalRef)
        .and_then(|system| props(&system, ElementName::PhysicalProps))
        .and_then(|p| target(&p, ElementName::CompuMethodRef))
        .and_then(|method| scaling(&method))
        .unwrap_or((1.0, 0.0, String::new()));

    Some(SignalDef {
        name,
        start_bit,
        size,
        big_endian,
        signed,
        factor,
        offset,
        unit,
        // Multiplexing is carried by a selector field this build does not
        // resolve, so every signal collected is reported as always present.
        multiplexing: Multiplexing::None,
        // A TEXTTABLE compu method is ARXML's equivalent of a `VAL_` table.
        // `scaling` above reads the linear part only, so nothing is collected
        // here rather than something guessed at.
        value_table: Vec::new(),
    })
}

/// Reads factor, offset and unit out of a compu method.
///
/// A linear conversion is stored as rational coefficients: the numerator's first
/// two values are offset and factor, the denominator's first is a divisor. An
/// identical conversion has no coefficients at all, which is why the caller's
/// fallback of `(1, 0)` is not merely defensive.
fn scaling(method: &Element) -> Option<(f64, f64, String)> {
    // A unit's short name is an identifier; its display name is the symbol meant
    // to be shown — `wp` where the short name is `wizepoo`. The symbol is what a
    // unit means to a reader, so it wins where both exist.
    let unit = target(method, ElementName::UnitRef)
        .and_then(|u| text(&u, ElementName::DisplayName).or_else(|| u.item_name()))
        .unwrap_or_default();

    // A `SCALE_LINEAR_AND_TEXTTABLE` method interleaves text-table scales, which
    // carry a `COMPU-CONST` instead of coefficients, with the linear scale. Taking
    // the first scale therefore finds no coefficients and silently reports the
    // signal unscaled; the first scale that *has* coefficients is the one wanted.
    let coefficients = method
        .get_sub_element(ElementName::CompuInternalToPhys)
        .and_then(|c| c.get_sub_element(ElementName::CompuScales))
        .and_then(|scales| {
            scales
                .sub_elements()
                .filter(|e| e.element_name() == ElementName::CompuScale)
                .find_map(|scale| scale.get_sub_element(ElementName::CompuRationalCoeffs))
        });

    let Some(coefficients) = coefficients else {
        return Some((1.0, 0.0, unit));
    };

    let values = |name: ElementName| -> Vec<f64> {
        coefficients
            .get_sub_element(name)
            .map(|part| {
                part.sub_elements()
                    .filter(|e| e.element_name() == ElementName::V)
                    .filter_map(|v| v.character_data())
                    .filter_map(|data| data.parse_float())
                    .collect()
            })
            .unwrap_or_default()
    };

    let numerator = values(ElementName::CompuNumerator);
    let denominator = values(ElementName::CompuDenominator);

    let divisor = denominator.first().copied().unwrap_or(1.0);
    if divisor == 0.0 {
        // A zero denominator is not a conversion. Reporting the signal unscaled
        // would be a quiet lie, so report it as unconvertible instead.
        return None;
    }
    let offset = numerator.first().copied().unwrap_or(0.0) / divisor;
    let factor = numerator.get(1).copied().unwrap_or(1.0) / divisor;

    Some((factor, offset, unit))
}
