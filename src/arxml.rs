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
//!                       └ COMPU-METHOD-REF → COMPU-METHOD  factor, offset, unit, value table
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

use std::collections::HashMap;
use std::path::Path;

use autosar_data::{AttributeName, AutosarModel, Element, ElementName, EnumItem};

use crate::candb::{
    CanDatabase, ContainedPduDef, ContainerDef, ContainerHeaderType, MessageDef, Multiplexing,
    SignalDef, SignalEncoding, ID_MASK,
};
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
    /// validate against the AUTOSAR schema, or [`Mf4Error::Unsupported`] when a
    /// message uses an unsupported AUTOSAR layout (such as dynamic header-based
    /// `ContainerIPdu` or non-linear rational scaling).
    pub fn from_arxml_path(path: impl AsRef<Path>) -> Result<Self> {
        let model = AutosarModel::new();
        model
            .load_file(path.as_ref(), false)
            .map_err(|e| Mf4Error::parse_error(format!("the ARXML file could not be read: {e}")))?;

        from_arxml_model(&model)
    }

    /// Loads a database from in-memory ARXML bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::ParseError`] when the buffer will not parse or does
    /// not validate against the AUTOSAR schema, or [`Mf4Error::Unsupported`]
    /// for unsupported AUTOSAR constructs.
    pub fn from_arxml(bytes: &[u8]) -> Result<Self> {
        let model = AutosarModel::new();
        model
            .load_buffer(bytes, "memory.arxml", false)
            .map_err(|e| {
                Mf4Error::parse_error(format!("the ARXML buffer could not be parsed: {e}"))
            })?;

        from_arxml_model(&model)
    }
}

fn from_arxml_model(model: &AutosarModel) -> Result<CanDatabase> {
    // System-level container header byte order (AUTOSAR defaults to big-endian).
    let mut header_big_endian = true;
    for (_, weak) in model.identifiable_elements() {
        let Some(element) = weak.upgrade() else {
            continue;
        };
        if element.element_name() == ElementName::System {
            if let Some(order) = enumerated(&element, ElementName::ContainerIPduHeaderByteOrder) {
                header_big_endian = order != EnumItem::MostSignificantByteLast;
            }
        }
    }

    let mut messages = Vec::new();
    let mut containers = HashMap::new();
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
            let Some(triggerings) = channel.get_sub_element(ElementName::FrameTriggerings) else {
                continue;
            };
            for triggering in triggerings
                .sub_elements()
                .filter(|e| e.element_name() == ElementName::CanFrameTriggering)
            {
                if let Some((message, container)) = message_def(&triggering, header_big_endian)? {
                    let message_index = messages.len();
                    messages.push(message);
                    if let Some(container_def) = container {
                        containers.insert(message_index, container_def);
                    }
                }
            }
        }
    }

    Ok(CanDatabase::new_with_containers(messages, containers))
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

/// Shifts a signal's `start_bit` by `bit_offset` bits within a parent frame or
/// static container PDU.
fn offset_signal_start_bit(signal: &mut SignalDef, bit_offset: u64) {
    if bit_offset == 0 {
        return;
    }
    if signal.big_endian {
        let msb = (signal.start_bit / 8) * 8 + (7 - signal.start_bit % 8);
        let shifted = msb.saturating_add(bit_offset);
        signal.start_bit = (shifted / 8) * 8 + (7 - shifted % 8);
    } else {
        signal.start_bit = signal.start_bit.saturating_add(bit_offset);
    }
}

fn message_def(
    triggering: &Element,
    header_big_endian: bool,
) -> Result<Option<(MessageDef, Option<ContainerDef>)>> {
    let Some(identifier) = integer(triggering, ElementName::Identifier) else {
        return Ok(None);
    };
    let extended =
        enumerated(triggering, ElementName::CanAddressingMode) == Some(EnumItem::Extended);

    let Some(frame) = target(triggering, ElementName::FrameRef) else {
        return Ok(None);
    };
    let Some(name) = frame.item_name() else {
        return Ok(None);
    };
    let length = integer(&frame, ElementName::FrameLength)
        .unwrap_or(0)
        .max(0) as u64;

    let mut signals = Vec::new();
    let mut container_def = None;
    if let Some(mappings) = frame.get_sub_element(ElementName::PduToFrameMappings) {
        for mapping in mappings.sub_elements() {
            let Some(pdu) = target(&mapping, ElementName::PduRef) else {
                continue;
            };
            let pdu_start_bit = integer(&mapping, ElementName::StartPosition)
                .unwrap_or(0)
                .max(0) as u64;
            if pdu.element_name() == ElementName::ContainerIPdu {
                if let Some(cdef) =
                    collect_container_ipdu(&pdu, header_big_endian, pdu_start_bit, &mut signals)?
                {
                    container_def = Some(cdef);
                }
            } else {
                let mut pdu_signals = Vec::new();
                collect_signals(&pdu, &mut pdu_signals)?;
                if pdu_start_bit > 0 {
                    for signal in &mut pdu_signals {
                        offset_signal_start_bit(signal, pdu_start_bit);
                    }
                }
                signals.extend(pdu_signals);
            }
        }
    }

    Ok(Some((
        MessageDef {
            name,
            id: (identifier as u32) & ID_MASK,
            extended,
            length,
            signals,
        },
        container_def,
    )))
}

/// Collects signals from a `ContainerIPdu`.
///
/// - For static (`NO-HEADER` or absent `HEADER-TYPE`) containers, each
///   contained PDU's signals are shifted by its `CONTAINED-I-PDU-PROPS/OFFSET`
///   byte offset plus `pdu_start_bit` and appended directly to `out`.
/// - For dynamic (`SHORT-HEADER` / `LONG-HEADER`) containers, each contained
///   PDU's header ID and signal slice are collected into a [`ContainerDef`] so
///   [`CanDatabase`] can parse TLV headers and route payloads at decode time.
fn collect_container_ipdu(
    pdu: &Element,
    header_big_endian: bool,
    pdu_start_bit: u64,
    out: &mut Vec<SignalDef>,
) -> Result<Option<ContainerDef>> {
    let header_type = match enumerated(pdu, ElementName::HeaderType) {
        Some(EnumItem::ShortHeader) => Some(ContainerHeaderType::ShortHeader),
        Some(EnumItem::LongHeader) => Some(ContainerHeaderType::LongHeader),
        _ => None,
    };

    let Some(triggering_refs) = pdu.get_sub_element(ElementName::ContainedPduTriggeringRefs) else {
        return Ok(None);
    };

    let mut contained_entries = Vec::new();

    for triggering_ref in triggering_refs
        .sub_elements()
        .filter(|e| e.element_name() == ElementName::ContainedPduTriggeringRef)
    {
        let Ok(pdu_triggering) = triggering_ref.get_reference_target() else {
            continue;
        };
        let Some(contained_pdu) = target(&pdu_triggering, ElementName::IPduRef) else {
            continue;
        };

        let mut contained_signals = Vec::new();
        collect_signals(&contained_pdu, &mut contained_signals)?;

        match header_type {
            Some(htype) => {
                let header_id_elem = match htype {
                    ContainerHeaderType::ShortHeader => ElementName::HeaderIdShortHeader,
                    ContainerHeaderType::LongHeader => ElementName::HeaderIdLongHeader,
                };
                let header_id = contained_pdu
                    .get_sub_element(ElementName::ContainedIPduProps)
                    .and_then(|props| integer(&props, header_id_elem))
                    .map(|v| v as u32);
                if let Some(hid) = header_id {
                    let signal_index_offset = out.len();
                    let signal_count = contained_signals.len();
                    let has_multiplexing = contained_signals
                        .iter()
                        .any(|s| s.multiplexing != Multiplexing::None);
                    out.extend(contained_signals);
                    contained_entries.push(ContainedPduDef {
                        header_id: hid,
                        signal_index_offset,
                        signal_count,
                        has_multiplexing,
                    });
                }
            }
            None => {
                let byte_offset = contained_pdu
                    .get_sub_element(ElementName::ContainedIPduProps)
                    .and_then(|props| integer(&props, ElementName::Offset))
                    .unwrap_or(0)
                    .max(0) as u64;
                let total_bit_offset = pdu_start_bit.saturating_add(byte_offset.saturating_mul(8));
                if total_bit_offset > 0 {
                    for signal in &mut contained_signals {
                        offset_signal_start_bit(signal, total_bit_offset);
                    }
                }
                out.extend(contained_signals);
            }
        }
    }

    Ok(header_type.map(|htype| ContainerDef {
        header_type: htype,
        header_big_endian,
        contained: contained_entries,
    }))
}

/// Collects the signals a PDU carries.
///
/// Handles `ISignalIPdu`, `MultiplexedIPdu` (static and dynamic parts resolved by
/// selector field), `SecuredIPdu` (via its payload PDU), and static `ContainerIPdu`.
fn collect_signals(pdu: &Element, out: &mut Vec<SignalDef>) -> Result<()> {
    match pdu.element_name() {
        ElementName::MultiplexedIPdu => {
            collect_multiplexed_ipdu(pdu, out)?;
        }
        ElementName::ISignalIPdu => {
            collect_isignal_ipdu(pdu, None, None, Multiplexing::None, out)?;
        }
        ElementName::SecuredIPdu => {
            if let Some(payload_pdu) = target(pdu, ElementName::PayloadRef)
                .and_then(|trig| target(&trig, ElementName::IPduRef))
            {
                collect_signals(&payload_pdu, out)?;
            }
        }
        ElementName::ContainerIPdu => {
            let _ = collect_container_ipdu(pdu, true, 0, out)?;
        }
        _ => {
            collect_isignal_ipdu(pdu, None, None, Multiplexing::None, out)?;
        }
    }
    Ok(())
}

fn collect_multiplexed_ipdu(pdu: &Element, out: &mut Vec<SignalDef>) -> Result<()> {
    let selector_start = integer(pdu, ElementName::SelectorFieldStartPosition);
    let selector_len = integer(pdu, ElementName::SelectorFieldLength);
    let selector_big_endian = enumerated(pdu, ElementName::SelectorFieldByteOrder)
        == Some(EnumItem::MostSignificantByteFirst);

    if let Some(static_parts) = pdu.get_sub_element(ElementName::StaticParts) {
        for static_part in static_parts.sub_elements() {
            if let Some(ipdu) = target(&static_part, ElementName::IPduRef) {
                collect_isignal_ipdu(&ipdu, selector_start, selector_len, Multiplexing::None, out)?;
            }
        }
    }

    if let Some(dynamic_parts) = pdu.get_sub_element(ElementName::DynamicParts) {
        for dynamic_part in dynamic_parts.sub_elements() {
            if let Some(alternatives) =
                dynamic_part.get_sub_element(ElementName::DynamicPartAlternatives)
            {
                for alt in alternatives.sub_elements() {
                    let code = integer(&alt, ElementName::SelectorFieldCode).unwrap_or(0) as u64;
                    if let Some(ipdu) = target(&alt, ElementName::IPduRef) {
                        collect_isignal_ipdu(
                            &ipdu,
                            selector_start,
                            selector_len,
                            Multiplexing::Selected(code),
                            out,
                        )?;
                    }
                }
            }
        }
    }

    // If no signal in out was identified as the selector switch, synthesize one if selector info exists.
    if let (Some(s_start), Some(s_len)) = (selector_start, selector_len) {
        if !out.iter().any(|s| s.multiplexing == Multiplexing::Switch) && s_len > 0 {
            let name = format!(
                "{}_Selector",
                pdu.item_name().unwrap_or_else(|| "Multiplexed".to_string())
            );
            out.push(SignalDef {
                name,
                start_bit: s_start.max(0) as u64,
                size: s_len.max(0) as u64,
                big_endian: selector_big_endian,
                signed: false,
                encoding: SignalEncoding::Unsigned,
                factor: 1.0,
                offset: 0.0,
                unit: String::new(),
                multiplexing: Multiplexing::Switch,
                value_table: Vec::new(),
                physical_ranges: Vec::new(),
            });
        }
    }
    Ok(())
}

fn collect_isignal_ipdu(
    pdu: &Element,
    selector_start: Option<i64>,
    selector_len: Option<i64>,
    default_multiplexing: Multiplexing,
    out: &mut Vec<SignalDef>,
) -> Result<()> {
    let Some(mappings) = pdu.get_sub_element(ElementName::ISignalToPduMappings) else {
        return Ok(());
    };
    for mapping in mappings
        .sub_elements()
        .filter(|e| e.element_name() == ElementName::ISignalToIPduMapping)
    {
        let Some(mut signal) = signal_def(&mapping, default_multiplexing.clone())? else {
            continue;
        };
        if let (Some(s_start), Some(s_len)) = (selector_start, selector_len) {
            if signal.start_bit == s_start.max(0) as u64 && signal.size == s_len.max(0) as u64 {
                // If a switch signal already exists at this start bit, skip the duplicate.
                if out.iter().any(|s| {
                    s.multiplexing == Multiplexing::Switch && s.start_bit == signal.start_bit
                }) {
                    continue;
                }
                signal.multiplexing = Multiplexing::Switch;
            }
        }
        out.push(signal);
    }
    Ok(())
}

fn signal_def(mapping: &Element, multiplexing: Multiplexing) -> Result<Option<SignalDef>> {
    let Some(start_bit_raw) = integer(mapping, ElementName::StartPosition) else {
        return Ok(None);
    };
    let start_bit = start_bit_raw.max(0) as u64;

    // Absent packing order means little-endian: the element is optional, and a
    // signal that does not say is not big-endian.
    let big_endian = enumerated(mapping, ElementName::PackingByteOrder)
        == Some(EnumItem::MostSignificantByteFirst);

    let Some(signal) = target(mapping, ElementName::ISignalRef) else {
        return Ok(None);
    };
    let Some(name) = signal.item_name() else {
        return Ok(None);
    };
    let Some(size_raw) = integer(&signal, ElementName::Length) else {
        return Ok(None);
    };
    let size = size_raw.max(0) as u64;

    // Raw representation encoding comes from the base type:
    // `2C` (two's complement), `1C` (one's complement), `SM` (sign-magnitude),
    // `IEEE754` (32- or 64-bit float), or unsigned (`NONE` / absent).
    let base_encoding = props(&signal, ElementName::NetworkRepresentationProps)
        .and_then(|p| target(&p, ElementName::BaseTypeRef))
        .and_then(|base| text(&base, ElementName::BaseTypeEncoding));

    let encoding = match base_encoding.as_deref().map(str::trim) {
        Some(enc) if enc.eq_ignore_ascii_case("2C") => SignalEncoding::Signed,
        Some(enc) if enc.eq_ignore_ascii_case("1C") => SignalEncoding::OnesComplement,
        Some(enc) if enc.eq_ignore_ascii_case("SM") => SignalEncoding::SignMagnitude,
        Some(enc) if enc.eq_ignore_ascii_case("IEEE754") => match size {
            32 => SignalEncoding::Float32,
            64 => SignalEncoding::Float64,
            _ => {
                return Err(Mf4Error::unsupported(
                    "ARXML IEEE754 signal width",
                    format!(
                        "signal '{name}' declares BASE-TYPE-ENCODING IEEE754 with unsupported width {size} bits"
                    ),
                ));
            }
        },
        _ => SignalEncoding::Unsigned,
    };
    let signed = matches!(
        encoding,
        SignalEncoding::Signed | SignalEncoding::OnesComplement | SignalEncoding::SignMagnitude
    );

    let compu_method = target(&signal, ElementName::SystemSignalRef)
        .and_then(|system| props(&system, ElementName::PhysicalProps))
        .and_then(|p| target(&p, ElementName::CompuMethodRef))
        .or_else(|| {
            props(&signal, ElementName::NetworkRepresentationProps)
                .and_then(|p| target(&p, ElementName::CompuMethodRef))
        });

    let (factor, offset, unit) = match compu_method.as_ref() {
        Some(method) => scaling(method)?,
        None => (1.0, 0.0, String::new()),
    };

    let value_table = compu_method.as_ref().map(value_table).unwrap_or_default();

    Ok(Some(SignalDef {
        name,
        start_bit,
        size,
        big_endian,
        signed,
        encoding,
        factor,
        offset,
        unit,
        multiplexing,
        value_table,
        physical_ranges: Vec::new(),
    }))
}

fn find_rational_coeffs(container: &Element) -> Option<Element> {
    container
        .get_sub_element(ElementName::CompuScales)?
        .sub_elements()
        .filter(|e| e.element_name() == ElementName::CompuScale)
        .find_map(|scale| scale.get_sub_element(ElementName::CompuRationalCoeffs))
}

fn read_coeff_values(coefficients: &Element, name: ElementName) -> Vec<f64> {
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
}

/// Reads factor, offset and unit out of a compu method.
///
/// A linear conversion is stored as rational coefficients in
/// `COMPU-INTERNAL-TO-PHYS` (direct: `phys = (n0 + n1 * int) / d0`) or
/// `COMPU-PHYS-TO-INTERNAL` (inverted: `int = (p0 + p1 * phys) / d0`, giving
/// `phys = (d0 / p1) * int + (-p0 / p1)`). A single numerator coefficient `[c]`
/// represents a constant function with slope `0.0`.
fn scaling(method: &Element) -> Result<(f64, f64, String)> {
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
    if let Some(coefficients) = method
        .get_sub_element(ElementName::CompuInternalToPhys)
        .and_then(|c| find_rational_coeffs(&c))
    {
        let numerator = read_coeff_values(&coefficients, ElementName::CompuNumerator);
        let denominator = read_coeff_values(&coefficients, ElementName::CompuDenominator);

        if numerator.iter().skip(2).any(|&v| v != 0.0)
            || denominator.iter().skip(1).any(|&v| v != 0.0)
        {
            return Err(Mf4Error::unsupported(
                "ARXML non-linear COMPU-RATIONAL-COEFFS",
                format!(
                    "COMPU-METHOD '{}' has non-linear rational coefficients",
                    method
                        .item_name()
                        .unwrap_or_else(|| "<unnamed>".to_string())
                ),
            ));
        }

        let divisor = denominator.first().copied().unwrap_or(1.0);
        if divisor == 0.0 || !divisor.is_finite() {
            return Err(Mf4Error::parse_error(format!(
                "ARXML COMPU-METHOD '{}' has zero or non-finite denominator",
                method
                    .item_name()
                    .unwrap_or_else(|| "<unnamed>".to_string())
            )));
        }
        let offset = numerator.first().copied().unwrap_or(0.0) / divisor;
        // A 1-coefficient numerator `[c]` encodes the constant `c / d0` (slope 0).
        let factor = numerator.get(1).copied().unwrap_or(0.0) / divisor;
        return Ok((factor, offset, unit));
    }

    if let Some(coefficients) = method
        .get_sub_element(ElementName::CompuPhysToInternal)
        .and_then(|c| find_rational_coeffs(&c))
    {
        let numerator = read_coeff_values(&coefficients, ElementName::CompuNumerator);
        let denominator = read_coeff_values(&coefficients, ElementName::CompuDenominator);

        if numerator.iter().skip(2).any(|&v| v != 0.0)
            || denominator.iter().skip(1).any(|&v| v != 0.0)
        {
            return Err(Mf4Error::unsupported(
                "ARXML non-linear COMPU-PHYS-TO-INTERNAL",
                format!(
                    "COMPU-METHOD '{}' has non-linear physical-to-internal coefficients",
                    method
                        .item_name()
                        .unwrap_or_else(|| "<unnamed>".to_string())
                ),
            ));
        }

        let divisor = denominator.first().copied().unwrap_or(1.0);
        if divisor == 0.0 || !divisor.is_finite() {
            return Err(Mf4Error::parse_error(format!(
                "ARXML COMPU-METHOD '{}' has zero or non-finite denominator",
                method
                    .item_name()
                    .unwrap_or_else(|| "<unnamed>".to_string())
            )));
        }
        let p0 = numerator.first().copied().unwrap_or(0.0);
        let p1 = numerator.get(1).copied().unwrap_or(0.0);
        if p1 == 0.0 || !p1.is_finite() {
            return Err(Mf4Error::parse_error(format!(
                "ARXML COMPU-METHOD '{}' COMPU-PHYS-TO-INTERNAL has zero slope and cannot be inverted",
                method.item_name().unwrap_or_else(|| "<unnamed>".to_string())
            )));
        }
        let factor = divisor / p1;
        let offset = -p0 / p1;
        return Ok((factor, offset, unit));
    }

    Ok((1.0, 0.0, unit))
}

/// Reads the value table out of a compu method when it carries text-table scales.
///
/// A `TEXTTABLE` compu method maps discrete raw values to text labels via
/// `COMPU-SCALE` elements, each holding a `COMPU-CONST` with a `VT` sub-element
/// and bounded by `LOWER-LIMIT` / `UPPER-LIMIT`. A `SCALE_LINEAR_AND_TEXTTABLE`
/// compu method carries both linear coefficients and text-table scales, and this
/// collects the text-table portion into the same `(i64, String)` representation
/// used for DBC `VAL_` tables.
fn value_table(method: &Element) -> Vec<(i64, String)> {
    let mut table = Vec::new();

    let scales = method
        .get_sub_element(ElementName::CompuInternalToPhys)
        .and_then(|c| c.get_sub_element(ElementName::CompuScales));

    let Some(scales) = scales else {
        return table;
    };

    for scale in scales
        .sub_elements()
        .filter(|e| e.element_name() == ElementName::CompuScale)
    {
        let Some(compu_const) = scale.get_sub_element(ElementName::CompuConst) else {
            continue;
        };
        let Some(label) = text(&compu_const, ElementName::Vt) else {
            continue;
        };

        let lower =
            limit_value(&scale, ElementName::LowerLimit)
                .map(|(v, is_open)| if is_open { v + 1 } else { v });
        let upper =
            limit_value(&scale, ElementName::UpperLimit)
                .map(|(v, is_open)| if is_open { v - 1 } else { v });
        let mask = integer(&scale, ElementName::Mask).map(i128::from);

        let mut insert = |val: i128, text: String| {
            if let Some(m) = mask {
                if (val & m) != val {
                    return;
                }
            }
            if let Ok(v64) = i64::try_from(val) {
                table.retain(|(id, _)| *id != v64);
                table.push((v64, text));
            }
        };

        if let (Some(l), Some(u)) = (lower, upper) {
            if l > u {
                continue;
            }
            if l == u {
                insert(l, label);
            } else if (u - l) <= 1000 {
                for val in l..=u {
                    insert(val, label.clone());
                }
            } else {
                insert(l, label);
            }
        } else if let Some(val) = lower.or(upper) {
            insert(val, label);
        }
    }

    table
}

/// Reads an integer limit value and whether its `INTERVAL-TYPE` attribute is `OPEN`.
fn limit_value(scale: &Element, name: ElementName) -> Option<(i128, bool)> {
    let elem = scale.get_sub_element(name)?;
    let is_open = elem
        .attribute_value(AttributeName::IntervalType)
        .and_then(|v| v.enum_value())
        == Some(EnumItem::Open);
    let cdata = elem.character_data()?;
    let value = cdata
        .parse_integer::<i64>()
        .map(i128::from)
        .or_else(|| cdata.parse_float().map(|f| f as i128))
        .or_else(|| {
            cdata
                .string_value()
                .and_then(|s| s.trim().parse::<i128>().ok())
        })?;
    Some((value, is_open))
}
