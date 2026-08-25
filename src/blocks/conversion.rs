//! Channel Conversion (CC) block parsing.
//!
//! CC blocks define how to convert raw channel values to physical values.
//! They support linear scaling, polynomial, tabular lookups, and more.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::blocks::formula::Expr;
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::borrow::Cow;
use std::io::Cursor;

/// Conversion type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionType {
    /// 1:1 identity conversion (no change).
    Identity,
    /// Linear conversion: y = a*x + b.
    Linear,
    /// Rational conversion: y = (a*x^2 + b*x + c) / (d*x^2 + e*x + f).
    Rational,
    /// Algebraic/formula-based conversion (text formula).
    Algebraic,
    /// Value-to-value tabular interpolation.
    TabInterpolation,
    /// Value-to-value tabular lookup (no interpolation).
    TabLookup,
    /// Value range to value tabular lookup.
    TabRangeLookup,
    /// Value-to-text tabular lookup.
    TabValueToText,
    /// Value range to text tabular lookup.
    TabRangeToText,
    /// Text-to-value tabular lookup.
    TabTextToValue,
    /// Text-to-text tabular lookup.
    TabTextToText,
    /// Bitfield to text.
    BitfieldToText,
    /// Unknown conversion type.
    Unknown(u8),
}

impl ConversionType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => ConversionType::Identity,
            1 => ConversionType::Linear,
            2 => ConversionType::Rational,
            3 => ConversionType::Algebraic,
            4 => ConversionType::TabInterpolation,
            5 => ConversionType::TabLookup,
            6 => ConversionType::TabRangeLookup,
            7 => ConversionType::TabValueToText,
            8 => ConversionType::TabRangeToText,
            9 => ConversionType::TabTextToValue,
            10 => ConversionType::TabTextToText,
            11 => ConversionType::BitfieldToText,
            v => ConversionType::Unknown(v),
        }
    }
}

/// Conversion flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CcFlags {
    /// Precision is valid.
    pub precision_valid: bool,
    /// Physical value range is valid.
    pub range_valid: bool,
    /// Status string present.
    pub status_string: bool,
}

impl CcFlags {
    fn from_u16(value: u16) -> Self {
        CcFlags {
            precision_valid: (value & 0x01) != 0,
            range_valid: (value & 0x02) != 0,
            status_string: (value & 0x04) != 0,
        }
    }
}

/// The Channel Conversion (CC) block.
///
/// Defines the conversion from raw values to physical values.
#[derive(Debug, Clone)]
pub struct CcBlock {
    /// Common block header.
    pub header: BlockHeader,
    /// Link to name (TX block).
    pub tx_name: u64,
    /// Link to unit (TX or MD block).
    pub md_unit: u64,
    /// Link to comment (TX or MD block).
    pub md_comment: u64,
    /// Link to inverse conversion (CC block).
    pub cc_inverse: u64,
    /// Links to reference blocks (TX for formulas, CC for cascaded).
    pub references: Vec<u64>,
    /// Conversion type.
    pub conversion_type: ConversionType,
    /// Precision (decimal places) if valid.
    pub precision: u8,
    /// Conversion flags.
    pub flags: CcFlags,
    /// Number of reference links.
    pub ref_count: u16,
    /// Number of conversion values.
    pub val_count: u16,
    /// Minimum physical value (if range_valid).
    pub phy_range_min: f64,
    /// Maximum physical value (if range_valid).
    pub phy_range_max: f64,
    /// Conversion parameters/values.
    pub values: Vec<f64>,
}

// Conversions are evaluated through `Conversion`, which is built from a CC
// block with its text references resolved. `CcBlock` is the raw parsed record
// and deliberately has no `convert` method of its own: it cannot see the text
// its `cc_ref` links point at, so it could only guess at the tabular text types.
impl CcBlock {
    /// Minimum size of the CC block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 4 * 8 + 24;
}

impl ParseBlock for CcBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CC", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size(
                "CC",
                header.length,
                Self::MIN_SIZE,
            ));
        }

        // Parse fixed links (first 4)
        let links_start = BLOCK_HEADER_SIZE;
        let tx_name = read_link(data, links_start)?;
        let md_unit = read_link(data, links_start + 8)?;
        let md_comment = read_link(data, links_start + 16)?;
        let cc_inverse = read_link(data, links_start + 24)?;

        // Additional reference links
        let extra_links = header.link_count.saturating_sub(4) as usize;
        let mut references = Vec::with_capacity(extra_links);
        for i in 0..extra_links {
            references.push(read_link(data, links_start + 32 + i * 8)?);
        }

        // Parse data section
        let data_start = header.data_offset();
        if data.len() < data_start + 24 {
            return Err(Mf4Error::truncated(offset, data_start + 24, data.len()));
        }

        let data_section = data
            .get(data_start..)
            .ok_or_else(|| Mf4Error::truncated(offset, data_start, data.len()))?;
        let mut cursor = Cursor::new(data_section);

        let conversion_type_raw = cursor.read_u8()?;
        let conversion_type = ConversionType::from_u8(conversion_type_raw);
        let precision = cursor.read_u8()?;
        let flags_raw = cursor.read_u16::<LittleEndian>()?;
        let flags = CcFlags::from_u16(flags_raw);
        let ref_count = cursor.read_u16::<LittleEndian>()?;
        let val_count = cursor.read_u16::<LittleEndian>()?;
        let phy_range_min = cursor.read_f64::<LittleEndian>()?;
        let phy_range_max = cursor.read_f64::<LittleEndian>()?;

        // Read conversion values
        let mut values = Vec::with_capacity(val_count as usize);
        for _ in 0..val_count {
            values.push(cursor.read_f64::<LittleEndian>()?);
        }

        Ok(CcBlock {
            header,
            tx_name,
            md_unit,
            md_comment,
            cc_inverse,
            references,
            conversion_type,
            precision,
            flags,
            ref_count,
            val_count,
            phy_range_min,
            phy_range_max,
            values,
        })
    }
}

/// What kind of value a conversion consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConversionInput {
    /// A number read from the record.
    Numeric,
    /// A string. Conversion types 9 and 10 are keyed by the channel's text, not
    /// by a number, so their input must be decoded as text before lookup.
    Text,
}

/// What kind of value a conversion produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConversionOutput {
    /// A number.
    Numeric,
    /// Text, from one of the tabular text conversions.
    Text,
    /// Nothing this version can produce; reading must fail rather than guess.
    Unsupported,
}

/// One entry of a value-to-text or range-to-text table — MF4 types 7 and 8.
///
/// The standard calls these "value to text/**scale**" conversions, and the
/// second half is the part easy to miss: each `cc_ref` may name a text block
/// *or* another CC block. When it names a CC, the raw value is passed through
/// that conversion instead of being replaced by a label, which is how a file
/// expresses a piecewise conversion — one formula below a threshold, another
/// above it — or a mostly-numeric channel with labels for a few special values.
///
/// Vector's `Vector_PartialConversion*` reference files use nothing but nested
/// conversions in a type 8 table, and reading their references as text is what
/// made this reader refuse to open them at all.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TableEntry {
    /// A label, replacing the value.
    Text(String),
    /// A nested conversion, applied to the raw value.
    Nested(Box<Conversion>),
}

impl TableEntry {
    /// Returns the label, or `None` when this entry is a nested conversion.
    pub fn text(&self) -> Option<&str> {
        match self {
            TableEntry::Text(t) => Some(t),
            TableEntry::Nested(_) => None,
        }
    }

    /// Returns true when this entry computes a number rather than naming one.
    pub fn is_nested(&self) -> bool {
        matches!(self, TableEntry::Nested(_))
    }
}

/// One entry of a [`Conversion::Bitfield`] table.
///
/// A bitfield entry's reference is either a label or a nested table. Which one
/// a writer uses is its choice, so both are carried here.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BitfieldEntry {
    /// A flag's label, rendered when the mask selects any set bit.
    Flag(String),
    /// A nested table, applied to the masked value.
    Nested {
        /// The nested conversion's name, used to label its result. Empty when
        /// the nested block carries no name.
        name: String,
        /// The table itself, evaluated on the masked value.
        conversion: Box<Conversion>,
    },
    /// A reference this version could not resolve. Rendered as nothing, so a
    /// partly-readable table still yields the parts that are readable.
    Unresolved,
}

/// A conversion that can be applied to raw values.
///
/// Every MF4 conversion type maps to exactly one variant. Types this version
/// cannot evaluate become [`Conversion::Unsupported`] rather than silently
/// behaving as identity, so a channel is never decoded into plausible-looking
/// wrong numbers.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum Conversion {
    /// No conversion (identity), MF4 type 0.
    #[default]
    None,
    /// Linear conversion `y = factor * x + offset`, MF4 type 1.
    Linear {
        /// Additive term (`p0` in the CC block).
        offset: f64,
        /// Multiplicative term (`p1` in the CC block).
        factor: f64,
    },
    /// Rational conversion, MF4 type 2.
    Rational {
        /// Coefficients `p0..p5` of the rational polynomial.
        coefficients: [f64; 6],
    },
    /// Algebraic formula, MF4 type 3.
    Algebraic {
        /// The formula text as stored in the file, kept for diagnostics.
        formula: String,
        /// The parsed form actually evaluated.
        expr: Expr,
    },
    /// Value-to-value table with linear interpolation, MF4 type 4.
    TableInterpolated {
        /// Raw-value keys, ascending.
        keys: Vec<f64>,
        /// Physical value at each key.
        values: Vec<f64>,
    },
    /// Value-to-value table without interpolation, MF4 type 5.
    TableLookup {
        /// Raw-value keys.
        keys: Vec<f64>,
        /// Physical value at each key.
        values: Vec<f64>,
    },
    /// Value-range-to-value table, MF4 type 6.
    RangeTable {
        /// Inclusive lower bound of each range.
        lower: Vec<f64>,
        /// Exclusive upper bound of each range.
        upper: Vec<f64>,
        /// Physical value for each range.
        values: Vec<f64>,
        /// Value used when no range matches.
        default: Option<f64>,
    },
    /// Value-to-text table, MF4 type 7.
    ValueToText {
        /// Raw-value keys.
        keys: Vec<f64>,
        /// What each key maps to: a label, or a conversion to apply.
        entries: Vec<TableEntry>,
        /// Used when no key matches.
        default: Option<TableEntry>,
    },
    /// Value-range-to-text table, MF4 type 8.
    RangeToText {
        /// Inclusive lower bound of each range.
        lower: Vec<f64>,
        /// Exclusive upper bound of each range.
        upper: Vec<f64>,
        /// What each range maps to: a label, or a conversion to apply.
        entries: Vec<TableEntry>,
        /// Used when no range matches.
        default: Option<TableEntry>,
    },
    /// Text-to-value table, MF4 type 9.
    ///
    /// Unlike every conversion above it, this one consumes a *string* sample:
    /// the channel's text is matched against `keys`, and the matching entry's
    /// number is the physical value.
    TextToValue {
        /// The text of each entry, in file order.
        keys: Vec<String>,
        /// Physical value for each key.
        values: Vec<f64>,
        /// Value used when no key matches.
        default: Option<f64>,
    },
    /// Text-to-text table, MF4 type 10.
    ///
    /// Translates one string to another — a status name in the recording
    /// device's vocabulary to one in the reader's, typically.
    TextToText {
        /// The text of each entry, in file order.
        keys: Vec<String>,
        /// Replacement text for each key.
        texts: Vec<String>,
        /// Text used when no key matches.
        default: Option<String>,
    },
    /// Bitfield-to-text table, MF4 type 11.
    ///
    /// Each entry masks the raw value and renders the result, and the rendered
    /// parts are joined into one string. A status word packing several fields
    /// decodes to something like `"gear = 3 | clutch = engaged"`.
    Bitfield {
        /// The bit mask applied to the raw value for each entry.
        ///
        /// `u64` rather than `f64`: unlike every other conversion, type 11
        /// stores its `cc_val` parameters as unsigned integers.
        masks: Vec<u64>,
        /// What each entry renders its masked value as.
        entries: Vec<BitfieldEntry>,
    },
    /// A conversion this version cannot evaluate.
    ///
    /// Carries the type so the error can name it. Reading a channel with such a
    /// conversion fails; it does not fall back to raw values.
    Unsupported {
        /// The conversion type found in the file.
        kind: ConversionType,
        /// Why it could not be prepared.
        reason: String,
    },
}

impl Conversion {
    /// Returns true if this conversion leaves raw values unchanged.
    ///
    /// Channels with an identity conversion keep their raw type when decoded;
    /// any other conversion produces physical values, which are always `f64`.
    /// A linear conversion with offset 0 and factor 1 counts as identity — some
    /// writers emit that rather than omitting the conversion block.
    pub fn is_identity(&self) -> bool {
        match self {
            Conversion::None => true,
            Conversion::Linear { offset, factor } => *offset == 0.0 && *factor == 1.0,
            _ => false,
        }
    }

    /// Returns what kind of value this conversion consumes.
    ///
    /// Nearly every conversion maps a number to a number. Types 9 and 10 do not:
    /// they look their result up by the channel's *text*, so a caller must
    /// decode the samples as strings before applying them.
    pub fn input(&self) -> ConversionInput {
        match self {
            Conversion::TextToValue { .. } | Conversion::TextToText { .. } => ConversionInput::Text,
            _ => ConversionInput::Numeric,
        }
    }

    /// Returns what kind of value this conversion produces.
    pub fn output(&self) -> ConversionOutput {
        match self {
            // A type 7 or 8 table whose references are all nested conversions
            // computes a number; one with any label in it produces text. The
            // answer therefore depends on the table's contents, not on its
            // type code — which is why this cannot be a plain match arm.
            Conversion::ValueToText {
                entries, default, ..
            }
            | Conversion::RangeToText {
                entries, default, ..
            } => {
                let all_nested = entries.iter().all(TableEntry::is_nested)
                    && default.as_ref().is_none_or(TableEntry::is_nested);
                if all_nested {
                    ConversionOutput::Numeric
                } else {
                    ConversionOutput::Text
                }
            }
            Conversion::TextToText { .. } | Conversion::Bitfield { .. } => ConversionOutput::Text,
            Conversion::Unsupported { .. } => ConversionOutput::Unsupported,
            _ => ConversionOutput::Numeric,
        }
    }

    /// Applies a numeric conversion to a raw value.
    ///
    /// Text-producing and unsupported conversions have no numeric result and
    /// return `NaN`; use [`Conversion::output`] to detect them beforehand, and
    /// [`Conversion::convert_text`] to read text results.
    pub fn convert(&self, raw: f64, is_float: bool) -> f64 {
        match self {
            Conversion::None => raw,
            Conversion::Linear { offset, factor } => factor * raw + offset,
            Conversion::Rational { coefficients } => {
                let [p0, p1, p2, p3, p4, p5] = *coefficients;
                let num = p0 * raw * raw + p1 * raw + p2;
                let den = p3 * raw * raw + p4 * raw + p5;
                num / den
            }
            Conversion::Algebraic { expr, .. } => expr.eval(raw),
            Conversion::TableInterpolated { keys, values } => interpolate(keys, values, raw),
            Conversion::TableLookup { keys, values } => nearest(keys, values, raw),
            Conversion::RangeTable {
                lower,
                upper,
                values,
                default,
            } => {
                for i in (0..values.len()).rev() {
                    if in_range(raw, lower[i], upper[i], is_float) {
                        return values[i];
                    }
                }
                default.unwrap_or(f64::NAN)
            }
            // These three have no numeric result: the first two produce text,
            // the next two are keyed by text rather than by a number, and the
            // last cannot be evaluated at all.
            // Selects an entry, then evaluates it when it is a nested
            // conversion. A label has no numeric value, so it yields NaN —
            // `output` tells a caller which to expect.
            Conversion::ValueToText {
                keys,
                entries,
                default,
            } => {
                let hit = keys
                    .iter()
                    .position(|k| *k == raw)
                    .and_then(|i| entries.get(i))
                    .or(default.as_ref());
                match hit {
                    Some(TableEntry::Nested(c)) => c.convert(raw, is_float),
                    _ => f64::NAN,
                }
            }
            Conversion::RangeToText {
                lower,
                upper,
                entries,
                default,
            } => {
                let hit = (0..entries.len())
                    .rev()
                    .find(|&i| in_range(raw, lower[i], upper[i], is_float))
                    .and_then(|i| entries.get(i))
                    .or(default.as_ref());
                match hit {
                    Some(TableEntry::Nested(c)) => c.convert(raw, is_float),
                    _ => f64::NAN,
                }
            }
            Conversion::TextToValue { .. }
            | Conversion::TextToText { .. }
            | Conversion::Bitfield { .. }
            | Conversion::Unsupported { .. } => f64::NAN,
        }
    }

    /// Applies a text-producing conversion to a raw value.
    ///
    /// Returns `None` for numeric and unsupported conversions.
    ///
    /// A type 7 or 8 table may mix labels with nested conversions — a
    /// "status string table", where a few special values are named and the rest
    /// are computed. There is no single Rust type holding both, so a nested
    /// result is **rendered** as its number. That is a judgement: it keeps the
    /// labels, which the alternative of decoding the whole channel numerically
    /// would discard, and it is why this returns an owned string rather than a
    /// borrow. Use [`Conversion::output`] to learn whether a given table
    /// produces text at all — one made entirely of nested conversions does not,
    /// and is decoded as numbers.
    pub fn convert_text(&self, raw: f64, is_float: bool) -> Option<Cow<'_, str>> {
        let hit = match self {
            Conversion::ValueToText {
                keys,
                entries,
                default,
            } => keys
                .iter()
                .position(|k| *k == raw)
                .and_then(|i| entries.get(i))
                .or(default.as_ref()),
            Conversion::RangeToText {
                lower,
                upper,
                entries,
                default,
            } => (0..entries.len())
                .rev()
                .find(|&i| in_range(raw, lower[i], upper[i], is_float))
                .and_then(|i| entries.get(i))
                .or(default.as_ref()),
            _ => return None,
        };

        match hit? {
            TableEntry::Text(t) => Some(Cow::Borrowed(t.as_str())),
            TableEntry::Nested(c) => Some(Cow::Owned(format_number(c.convert(raw, is_float)))),
        }
    }

    /// Renders a raw value through a bitfield table, for MF4 type 11.
    ///
    /// Each entry masks the value and contributes a fragment; the fragments are
    /// joined with `" | "`. An entry whose nested table has a name renders as
    /// `name = text`, which is what makes a multi-field status word legible.
    ///
    /// Returns `None` for every other conversion. A value that is not a whole
    /// number cannot be masked and renders as the empty string.
    ///
    /// The two independent implementations consulted for this disagree on
    /// presentation — separator, spacing around `=`, and whether a bare label
    /// is emitted when its mask selects nothing. The rendering here treats a
    /// bare label as a flag, emitted only when the mask selects a set bit,
    /// since a label emitted regardless would make its mask meaningless.
    pub fn render_bitfield(&self, raw: f64) -> Option<String> {
        let Conversion::Bitfield { masks, entries } = self else {
            return None;
        };
        if !raw.is_finite() || raw.fract() != 0.0 || raw < 0.0 {
            return Some(String::new());
        }
        let value = raw as u64;

        let mut parts: Vec<String> = Vec::new();
        for (mask, entry) in masks.iter().zip(entries) {
            let masked = value & mask;
            match entry {
                BitfieldEntry::Flag(label) => {
                    if masked != 0 && !label.is_empty() {
                        parts.push(label.clone());
                    }
                }
                BitfieldEntry::Nested { name, conversion } => {
                    let rendered = match conversion.output() {
                        ConversionOutput::Text => conversion
                            .convert_text(masked as f64, false)
                            .unwrap_or_default()
                            .to_string(),
                        ConversionOutput::Numeric => {
                            let converted = conversion.convert(masked as f64, false);
                            if converted.is_nan() {
                                String::new()
                            } else {
                                converted.to_string()
                            }
                        }
                        ConversionOutput::Unsupported => String::new(),
                    };
                    match (name.is_empty(), rendered.is_empty()) {
                        (_, true) => {}
                        (true, false) => parts.push(rendered),
                        (false, false) => parts.push(format!("{name} = {rendered}")),
                    }
                }
                BitfieldEntry::Unresolved => {}
            }
        }
        Some(parts.join(" | "))
    }

    /// Looks a physical number up by the channel's text, for MF4 type 9.
    ///
    /// Returns `None` for every other conversion, and for a type-9 table with
    /// no default when the text matches no key.
    pub fn value_for_text(&self, text: &str) -> Option<f64> {
        let Conversion::TextToValue {
            keys,
            values,
            default,
        } = self
        else {
            return None;
        };
        keys.iter()
            .position(|k| k == text)
            .and_then(|i| values.get(i).copied())
            .or(*default)
    }

    /// Translates one string to another, for MF4 type 10.
    ///
    /// Returns `None` for every other conversion, and for a type-10 table with
    /// no default when the text matches no key.
    pub fn text_for_text(&self, text: &str) -> Option<&str> {
        let Conversion::TextToText {
            keys,
            texts,
            default,
        } = self
        else {
            return None;
        };
        keys.iter()
            .position(|k| k == text)
            .and_then(|i| texts.get(i).map(|s| s.as_str()))
            .or(default.as_deref())
    }
}

/// Renders a nested conversion's numeric result inside a text table.
///
/// Trims a trailing `.0` so a whole number reads as `7` rather than `7.0`,
/// which is what a status table sitting beside genuine labels wants to look
/// like. Nothing in the format specifies this; see `convert_text`.
fn format_number(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// Returns true when `raw` falls in `[lower, upper]`.
///
/// Integer channels use closed bounds on both ends: `raw >= lower && raw <= upper`.
/// That is what ASAM specifies for integer ranges, and it is what makes
/// single-point entries like `[100,100]` reachable — vendors write them and
/// `ASAP2_Demo_V171.mf4` has six of them in one table.
///
/// Float channels use an exclusive upper bound: `raw >= lower && raw < upper`.
/// ASAM's own List of Known Issues (issue 3545) calls out that chapters 5.17.8
/// and 5.17.10 specify different behaviour for floating-point numbers, and
/// both ihedvall/mdflib and asammdf apply the exclusive upper bound on float
/// channels.
///
/// Where ranges overlap the *last* matching one wins. The files settle both
/// halves of that rule, for conversion types 6 and 8 alike: with `[1,3]` and
/// `[3,5]`, a raw 3 belongs to two of them, and
/// `Vector_ValueRange2TextConversion.mf4` means the later — "low", not "very
/// low". Taking the first match instead would give a wrong label, silently, on
/// the one input most likely to be a table's boundary case.
fn in_range(raw: f64, lower: f64, upper: f64, is_float: bool) -> bool {
    if is_float {
        raw >= lower && raw < upper
    } else {
        raw >= lower && raw <= upper
    }
}

/// Linear interpolation between the two nearest table keys.
fn interpolate(keys: &[f64], values: &[f64], raw: f64) -> f64 {
    let n = keys.len().min(values.len());
    if n == 0 {
        return raw;
    }
    if raw <= keys[0] {
        return values[0];
    }
    if raw >= keys[n - 1] {
        return values[n - 1];
    }
    for i in 0..n - 1 {
        if raw >= keys[i] && raw <= keys[i + 1] {
            let span = keys[i + 1] - keys[i];
            if span == 0.0 {
                return values[i];
            }
            let t = (raw - keys[i]) / span;
            return values[i] + t * (values[i + 1] - values[i]);
        }
    }
    raw
}

/// Table lookup without interpolation: the value of the closest key.
fn nearest(keys: &[f64], values: &[f64], raw: f64) -> f64 {
    let n = keys.len().min(values.len());
    if n == 0 {
        return raw;
    }
    let mut best = 0usize;
    let mut best_diff = f64::MAX;
    for (i, k) in keys.iter().take(n).enumerate() {
        let diff = (raw - k).abs();
        if diff < best_diff {
            best_diff = diff;
            best = i;
        }
    }
    values[best]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_conversion_type_code() {
        assert_eq!(ConversionType::from_u8(0), ConversionType::Identity);
        assert_eq!(ConversionType::from_u8(1), ConversionType::Linear);
        assert_eq!(ConversionType::from_u8(2), ConversionType::Rational);
        assert_eq!(ConversionType::from_u8(3), ConversionType::Algebraic);
        assert_eq!(ConversionType::from_u8(4), ConversionType::TabInterpolation);
        assert_eq!(ConversionType::from_u8(5), ConversionType::TabLookup);
        assert_eq!(ConversionType::from_u8(6), ConversionType::TabRangeLookup);
        assert_eq!(ConversionType::from_u8(7), ConversionType::TabValueToText);
        assert_eq!(ConversionType::from_u8(8), ConversionType::TabRangeToText);
        assert_eq!(ConversionType::from_u8(9), ConversionType::TabTextToValue);
        assert_eq!(ConversionType::from_u8(10), ConversionType::TabTextToText);
        assert_eq!(ConversionType::from_u8(11), ConversionType::BitfieldToText);
        assert!(matches!(
            ConversionType::from_u8(99),
            ConversionType::Unknown(99)
        ));
    }

    #[test]
    fn applies_a_linear_conversion() {
        let c = Conversion::Linear {
            offset: 2.0,
            factor: 3.0,
        };
        assert_eq!(c.convert(4.0, false), 14.0, "y = factor*x + offset");
    }

    #[test]
    fn applies_the_rational_conversion_in_spec_order() {
        // ASAM MDF4: y = (P1*x^2 + P2*x + P3) / (P4*x^2 + P5*x + P6),
        // with P1..P6 stored in cc_val order.
        let c = Conversion::Rational {
            coefficients: [1.0, 2.0, 3.0, 0.0, 0.0, 2.0],
        };
        // x = 2 -> (1*4 + 2*2 + 3) / 2 = 11/2
        assert_eq!(c.convert(2.0, false), 5.5);
    }

    #[test]
    fn rational_with_a_zero_denominator_follows_ieee() {
        let c = Conversion::Rational {
            coefficients: [0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        };
        assert!(c.convert(1.0, false).is_infinite());
    }

    #[test]
    fn interpolates_between_table_keys() {
        let c = Conversion::TableInterpolated {
            keys: vec![0.0, 10.0],
            values: vec![100.0, 200.0],
        };
        assert_eq!(c.convert(5.0, false), 150.0);
        assert_eq!(c.convert(0.0, false), 100.0);
        assert_eq!(c.convert(10.0, false), 200.0);
        assert_eq!(c.convert(-5.0, false), 100.0, "clamps below the first key");
        assert_eq!(c.convert(50.0, false), 200.0, "clamps above the last key");
    }

    #[test]
    fn table_lookup_does_not_interpolate() {
        let c = Conversion::TableLookup {
            keys: vec![0.0, 10.0],
            values: vec![100.0, 200.0],
        };
        assert_eq!(c.convert(1.0, false), 100.0);
        assert_eq!(c.convert(9.0, false), 200.0);
    }

    #[test]
    fn range_table_selects_the_matching_range() {
        let c = Conversion::RangeTable {
            lower: vec![0.0, 10.0],
            upper: vec![9.0, 19.0],
            values: vec![1.0, 2.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(5.0, false), 1.0);
        assert_eq!(c.convert(15.0, false), 2.0);
        assert_eq!(c.convert(100.0, false), -1.0, "falls back to the default");
    }

    #[test]
    fn range_table_without_a_default_yields_nan() {
        let c = Conversion::RangeTable {
            lower: vec![0.0],
            upper: vec![1.0],
            values: vec![7.0],
            default: None,
        };
        assert!(c.convert(50.0, false).is_nan());
    }

    /// Builds label entries, for tables that hold no nested conversion.
    fn labels(items: &[&str]) -> Vec<TableEntry> {
        items
            .iter()
            .map(|t| TableEntry::Text((*t).into()))
            .collect()
    }

    #[test]
    fn value_to_text_matches_exact_keys() {
        let c = Conversion::ValueToText {
            keys: vec![0.0, 1.0],
            entries: labels(&["off", "on"]),
            default: Some(TableEntry::Text("unknown".into())),
        };
        assert_eq!(c.convert_text(0.0, false).as_deref(), Some("off"));
        assert_eq!(c.convert_text(1.0, false).as_deref(), Some("on"));
        assert_eq!(c.convert_text(2.0, false).as_deref(), Some("unknown"));
        assert_eq!(c.output(), ConversionOutput::Text);
    }

    #[test]
    fn range_bounds_are_closed_so_a_shared_boundary_lands_in_the_later_range() {
        // Table and expectations taken from Vector's own reference file
        // `Vector_ValueRange2TextConversion.mf4`, whose ranges are *adjacent*:
        // [1,3] [3,5] [5,7]. That is what makes the boundary meaningful — a
        // raw 3 sits in two ranges, and the file means the later one.
        //
        // The test this replaced used ranges 0-9 and 10-19 — a gap either side
        // of every bound — so it could not tell an inclusive upper bound from
        // an exclusive one, and pinned the wrong one for four phases.
        let c = Conversion::RangeToText {
            lower: vec![1.0, 3.0, 5.0],
            upper: vec![3.0, 5.0, 7.0],
            entries: labels(&["very low", "low", "medium"]),
            default: Some(TableEntry::Text("Out of range".into())),
        };

        assert_eq!(
            c.convert_text(1.0, false).as_deref(),
            Some("very low"),
            "lower is inclusive"
        );
        assert_eq!(c.convert_text(2.9, false).as_deref(), Some("very low"));
        assert_eq!(
            c.convert_text(3.0, false).as_deref(),
            Some("low"),
            "a shared boundary belongs to the later range"
        );
        assert_eq!(c.convert_text(5.0, false).as_deref(), Some("medium"));
        assert_eq!(c.convert_text(6.9, false).as_deref(), Some("medium"));
        assert_eq!(
            c.convert_text(7.0, false).as_deref(),
            Some("medium"),
            "the last range's upper bound is its own"
        );
        assert_eq!(
            c.convert_text(7.1, false).as_deref(),
            Some("Out of range"),
            "past the last"
        );
        assert_eq!(
            c.convert_text(0.0, false).as_deref(),
            Some("Out of range"),
            "before the first"
        );
    }

    #[test]
    fn a_range_table_shares_the_closed_bounds_rule() {
        // Type 6 is the numeric twin of type 8 and had the same defect, so it
        // gets the same check rather than being assumed to follow.
        let c = Conversion::RangeTable {
            lower: vec![1.0, 3.0],
            upper: vec![3.0, 5.0],
            values: vec![10.0, 20.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(1.0, false), 10.0);
        assert_eq!(c.convert(2.9, false), 10.0);
        assert_eq!(
            c.convert(3.0, false),
            20.0,
            "the boundary belongs to the next range"
        );
        assert_eq!(
            c.convert(5.0, false),
            20.0,
            "the last range owns its upper bound"
        );
        assert_eq!(c.convert(5.1, false), -1.0, "past the last range");
    }

    #[test]
    fn float_range_table_upper_bound_is_exclusive() {
        // ASAM MDF4 specification defines float ranges as [lower, upper).
        // A value on the upper boundary falls outside the range (or into the next range).
        let c = Conversion::RangeToText {
            lower: vec![0.0, 0.5, 1.0],
            upper: vec![0.5, 1.0, 2.0],
            entries: labels(&["lower range", "mid-range", "higher range"]),
            default: Some(TableEntry::Text("default".into())),
        };
        assert_eq!(c.convert_text(0.0, true).as_deref(), Some("lower range"));
        assert_eq!(c.convert_text(0.5, true).as_deref(), Some("mid-range"));
        assert_eq!(c.convert_text(1.0, true).as_deref(), Some("higher range"));
        assert_eq!(c.convert_text(1.5, true).as_deref(), Some("higher range"));
        assert_eq!(
            c.convert_text(2.0, true).as_deref(),
            Some("default"),
            "upper bound 2.0 is exclusive for float channels"
        );
    }

    #[test]
    fn integer_single_point_ranges_match_on_closed_bounds() {
        // Calibration files like ASAP2_Demo_V171 declare single point ranges [100, 100].
        let c = Conversion::RangeTable {
            lower: vec![100.0, 101.0],
            upper: vec![100.0, 101.0],
            values: vec![1.0, 2.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(100.0, false), 1.0);
        assert_eq!(c.convert(101.0, false), 2.0);
        assert_eq!(c.convert(99.0, false), -1.0);
    }

    #[test]
    fn text_conversions_have_no_numeric_result() {
        let c = Conversion::ValueToText {
            keys: vec![0.0],
            entries: labels(&["off"]),
            default: None,
        };
        assert!(c.convert(0.0, false).is_nan());
    }

    #[test]
    fn a_table_of_nested_conversions_is_piecewise_and_numeric() {
        // Types 7 and 8 are "value to text/**scale**": a reference may name a
        // CC block instead of a label, which is how a file writes a piecewise
        // conversion. Layout and expectations from Vector's
        // `Vector_PartialConversionLinearIdentityAlgebraic.mf4`.
        let c = Conversion::RangeToText {
            lower: vec![0.5, 2.2],
            upper: vec![2.2, 3.2],
            entries: vec![
                TableEntry::Nested(Box::new(Conversion::Linear {
                    offset: 5.67,
                    factor: 2.34,
                })),
                TableEntry::Nested(Box::new(Conversion::None)),
            ],
            default: Some(TableEntry::Nested(Box::new(Conversion::Linear {
                offset: -1.0,
                factor: 0.0,
            }))),
        };

        // Nothing here is a label, so the table computes numbers. Reporting it
        // as text is what made these files decode as empty strings.
        assert_eq!(c.output(), ConversionOutput::Numeric);
        assert_eq!(c.convert(1.0, false), 5.67 + 2.34);
        assert_eq!(c.convert(2.5, false), 2.5, "the identity branch");
        assert_eq!(
            c.convert(0.0, false),
            -1.0,
            "outside every range, so the default"
        );
    }

    #[test]
    fn a_table_mixing_labels_and_conversions_still_reads_as_text() {
        // A status-string table: a few values named, the rest computed. There
        // is no Rust type holding both, so the computed side is rendered — see
        // `convert_text`. Losing the labels instead would be the worse trade.
        let c = Conversion::RangeToText {
            lower: vec![9.9999],
            upper: vec![10.1001],
            entries: labels(&["Illegal value"]),
            default: Some(TableEntry::Nested(Box::new(Conversion::Linear {
                offset: 0.0,
                factor: 2.0,
            }))),
        };
        assert_eq!(c.output(), ConversionOutput::Text);
        assert_eq!(
            c.convert_text(10.0, false).as_deref(),
            Some("Illegal value")
        );
        assert_eq!(c.convert_text(3.0, false).as_deref(), Some("6"));
    }

    #[test]
    fn numeric_conversions_have_no_text_result() {
        let c = Conversion::Linear {
            offset: 0.0,
            factor: 1.0,
        };
        assert_eq!(c.convert_text(1.0, false), None);
    }

    #[test]
    fn evaluates_an_algebraic_conversion() {
        let expr = Expr::parse("2*X + 1").unwrap();
        let c = Conversion::Algebraic {
            formula: "2*X + 1".into(),
            expr,
        };
        assert_eq!(c.convert(3.0, false), 7.0);
        assert_eq!(c.output(), ConversionOutput::Numeric);
    }

    #[test]
    fn unsupported_conversions_report_themselves_as_such() {
        let c = Conversion::Unsupported {
            kind: ConversionType::BitfieldToText,
            reason: "nested conversions".into(),
        };
        assert_eq!(c.output(), ConversionOutput::Unsupported);
        assert!(!c.is_identity(), "must never be mistaken for identity");
    }

    #[test]
    fn identity_recognises_both_spellings() {
        assert!(Conversion::None.is_identity());
        assert!(Conversion::Linear {
            offset: 0.0,
            factor: 1.0
        }
        .is_identity());
        assert!(!Conversion::Linear {
            offset: 1.0,
            factor: 1.0
        }
        .is_identity());
        assert!(!Conversion::Linear {
            offset: 0.0,
            factor: 2.0
        }
        .is_identity());
    }
    #[test]
    fn a_text_to_value_table_looks_up_by_string() {
        let c = Conversion::TextToValue {
            keys: vec!["off".into(), "on".into()],
            values: vec![0.0, 1.0],
            default: Some(-1.0),
        };
        assert_eq!(c.value_for_text("off"), Some(0.0));
        assert_eq!(c.value_for_text("on"), Some(1.0));
        assert_eq!(c.value_for_text("elsewhere"), Some(-1.0));
        assert_eq!(c.input(), ConversionInput::Text);
        assert_eq!(c.output(), ConversionOutput::Numeric);
        // It has no numeric input, so the numeric path must not invent one.
        assert!(c.convert(0.0, false).is_nan());
    }

    #[test]
    fn a_text_to_value_table_without_a_default_has_no_result_for_an_unknown_key() {
        let c = Conversion::TextToValue {
            keys: vec!["on".into()],
            values: vec![1.0],
            default: None,
        };
        assert_eq!(c.value_for_text("on"), Some(1.0));
        assert_eq!(c.value_for_text("off"), None);
    }

    #[test]
    fn a_text_to_text_table_translates_and_falls_back_to_its_default() {
        let c = Conversion::TextToText {
            keys: vec!["ok".into(), "err".into()],
            texts: vec!["Healthy".into(), "Faulted".into()],
            default: Some("Unrecognised".into()),
        };
        assert_eq!(c.text_for_text("ok"), Some("Healthy"));
        assert_eq!(c.text_for_text("err"), Some("Faulted"));
        assert_eq!(c.text_for_text("???"), Some("Unrecognised"));
        assert_eq!(c.input(), ConversionInput::Text);
        assert_eq!(c.output(), ConversionOutput::Text);
    }

    #[test]
    fn the_text_lookups_ignore_conversions_that_are_not_keyed_by_text() {
        // Both return `None` rather than a plausible-looking wrong answer, so a
        // caller that reaches them by mistake gets nothing instead of a value.
        let numeric = Conversion::Linear {
            offset: 1.0,
            factor: 2.0,
        };
        assert_eq!(numeric.value_for_text("1"), None);
        assert_eq!(numeric.text_for_text("1"), None);
        assert_eq!(numeric.input(), ConversionInput::Numeric);
    }

    #[test]
    fn integer_range_tables_keep_inclusive_single_point_ranges() {
        // ASAP2_Demo_V171.mf4 has six single-point ranges [100,100]..[105,105]
        // on an integer channel. An exclusive upper bound would make them all
        // unreachable — 100 < 100 is false — and 1,100 integer samples would
        // fall to the default instead.
        let c = Conversion::RangeTable {
            lower: vec![100.0, 101.0, 102.0],
            upper: vec![100.0, 101.0, 102.0],
            values: vec![10.0, 11.0, 12.0],
            default: Some(-1.0),
        };
        assert_eq!(
            c.convert(100.0, false),
            10.0,
            "single-point lower bound is inclusive"
        );
        assert_eq!(c.convert(101.0, false), 11.0);
        assert_eq!(c.convert(102.0, false), 12.0);
        assert_eq!(c.convert(99.0, false), -1.0, "below the first range");
        assert_eq!(c.convert(103.0, false), -1.0, "above the last range");
    }

    #[test]
    fn integer_range_tables_match_an_upper_bound_followed_by_a_gap() {
        // Integer channels keep the closed bound even when the next range starts
        // after a gap. Without the inclusive upper, a raw value equal to the
        // upper bound would fall through to the default despite the file naming
        // a label for it.
        let c = Conversion::RangeTable {
            lower: vec![0.0, 10.0],
            upper: vec![5.0, 15.0],
            values: vec![1.0, 2.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(5.0, false), 1.0, "upper bound is inclusive");
        assert_eq!(c.convert(6.0, false), -1.0, "gap after the first range");
        assert_eq!(
            c.convert(10.0, false),
            2.0,
            "next range's lower bound is inclusive"
        );
    }

    #[test]
    fn float_range_tables_use_an_exclusive_upper_bound() {
        let c = Conversion::RangeTable {
            lower: vec![1.0, 3.0],
            upper: vec![3.0, 5.0],
            values: vec![10.0, 20.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(1.0, true), 10.0, "lower is inclusive");
        assert_eq!(c.convert(2.9, true), 10.0);
        assert_eq!(
            c.convert(3.0, true),
            20.0,
            "3.0 is the lower bound of the second range, which is inclusive"
        );
        assert_eq!(c.convert(4.9, true), 20.0);
        assert_eq!(
            c.convert(5.0, true),
            -1.0,
            "5.0 equals the final range's exclusive upper"
        );
        assert_eq!(c.convert(5.1, true), -1.0, "past the last range");
    }

    #[test]
    fn float_range_to_text_uses_an_exclusive_upper_bound() {
        let c = Conversion::RangeToText {
            lower: vec![1.0],
            upper: vec![3.0],
            entries: labels(&["a"]),
            default: Some(TableEntry::Text("default".into())),
        };
        assert_eq!(c.convert_text(1.0, true).as_deref(), Some("a"));
        assert_eq!(c.convert_text(2.9, true).as_deref(), Some("a"));
        assert_eq!(
            c.convert_text(3.0, true).as_deref(),
            Some("default"),
            "3.0 equals the final range's exclusive upper"
        );
    }
    /// A bitfield packing a gear in the low nibble and a flag above it.
    fn gearbox_bitfield() -> Conversion {
        Conversion::Bitfield {
            masks: vec![0x000F, 0x0010],
            entries: vec![
                BitfieldEntry::Nested {
                    name: "gear".into(),
                    conversion: Box::new(Conversion::ValueToText {
                        keys: vec![1.0, 2.0],
                        entries: labels(&["first", "second"]),
                        default: Some(TableEntry::Text("unknown".into())),
                    }),
                },
                BitfieldEntry::Flag("clutch".into()),
            ],
        }
    }

    #[test]
    fn a_bitfield_renders_each_masked_field_with_its_name() {
        let c = gearbox_bitfield();
        assert_eq!(
            c.render_bitfield(0x11 as f64).as_deref(),
            Some("gear = first | clutch")
        );
        assert_eq!(c.render_bitfield(2.0).as_deref(), Some("gear = second"));
        assert_eq!(c.output(), ConversionOutput::Text);
    }

    #[test]
    fn a_flag_is_rendered_only_when_its_mask_selects_a_set_bit() {
        // Emitting the label regardless would make the mask meaningless, and
        // would report a clutch engaged on every sample of the recording.
        let c = gearbox_bitfield();
        assert_eq!(c.render_bitfield(1.0).as_deref(), Some("gear = first"));
        assert_eq!(
            c.render_bitfield(0x10 as f64).as_deref(),
            Some("gear = unknown | clutch"),
            "gear 0 matches no key, so the nested default applies"
        );
    }

    #[test]
    fn a_bitfield_cannot_mask_a_value_that_is_not_a_whole_number() {
        let c = gearbox_bitfield();
        assert_eq!(c.render_bitfield(1.5).as_deref(), Some(""));
        assert_eq!(c.render_bitfield(f64::NAN).as_deref(), Some(""));
        assert_eq!(c.render_bitfield(-1.0).as_deref(), Some(""));
    }

    #[test]
    fn an_unresolved_bitfield_entry_contributes_nothing() {
        // A reference this build could not read drops out; the entries around
        // it are still rendered, so a partly-readable table stays useful.
        let c = Conversion::Bitfield {
            masks: vec![0xFF, 0xFF00],
            entries: vec![
                BitfieldEntry::Unresolved,
                BitfieldEntry::Flag("high".into()),
            ],
        };
        assert_eq!(c.render_bitfield(0xFFFF as f64).as_deref(), Some("high"));
    }

    #[test]
    fn render_bitfield_ignores_conversions_that_are_not_bitfields() {
        assert_eq!(Conversion::None.render_bitfield(1.0), None);
    }
}
