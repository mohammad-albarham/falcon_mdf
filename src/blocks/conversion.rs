//! Channel Conversion (CC) block parsing.
//!
//! CC blocks define how to convert raw channel values to physical values.
//! They support linear scaling, polynomial, tabular lookups, and more.

use crate::blocks::common::{read_link, BlockHeader, ParseBlock, BLOCK_HEADER_SIZE};
use crate::blocks::formula::Expr;
use crate::error::{Mf4Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};
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

/// What kind of value a conversion produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionOutput {
    /// A number.
    Numeric,
    /// Text, from one of the tabular text conversions.
    Text,
    /// Nothing this version can produce; reading must fail rather than guess.
    Unsupported,
}

/// A conversion that can be applied to raw values.
///
/// Every MF4 conversion type maps to exactly one variant. Types this version
/// cannot evaluate become [`Conversion::Unsupported`] rather than silently
/// behaving as identity, so a channel is never decoded into plausible-looking
/// wrong numbers.
#[derive(Debug, Clone, Default)]
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
        /// Upper bound of each range.
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
        /// Text for each key.
        texts: Vec<String>,
        /// Text used when no key matches.
        default: Option<String>,
    },
    /// Value-range-to-text table, MF4 type 8.
    RangeToText {
        /// Inclusive lower bound of each range.
        lower: Vec<f64>,
        /// Upper bound of each range.
        upper: Vec<f64>,
        /// Text for each range.
        texts: Vec<String>,
        /// Text used when no range matches.
        default: Option<String>,
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

    /// Returns what kind of value this conversion produces.
    pub fn output(&self) -> ConversionOutput {
        match self {
            Conversion::ValueToText { .. } | Conversion::RangeToText { .. } => {
                ConversionOutput::Text
            }
            Conversion::Unsupported { .. } => ConversionOutput::Unsupported,
            _ => ConversionOutput::Numeric,
        }
    }

    /// Applies a numeric conversion to a raw value.
    ///
    /// Text-producing and unsupported conversions have no numeric result and
    /// return `NaN`; use [`Conversion::output`] to detect them beforehand, and
    /// [`Conversion::convert_text`] to read text results.
    pub fn convert(&self, raw: f64) -> f64 {
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
                for i in 0..values.len() {
                    if raw >= lower[i] && raw <= upper[i] {
                        return values[i];
                    }
                }
                default.unwrap_or(f64::NAN)
            }
            Conversion::ValueToText { .. }
            | Conversion::RangeToText { .. }
            | Conversion::Unsupported { .. } => f64::NAN,
        }
    }

    /// Applies a text-producing conversion to a raw value.
    ///
    /// Returns `None` for numeric and unsupported conversions.
    pub fn convert_text(&self, raw: f64) -> Option<&str> {
        match self {
            Conversion::ValueToText {
                keys,
                texts,
                default,
            } => {
                for (i, k) in keys.iter().enumerate() {
                    if *k == raw {
                        return texts.get(i).map(|s| s.as_str());
                    }
                }
                default.as_deref()
            }
            Conversion::RangeToText {
                lower,
                upper,
                texts,
                default,
            } => {
                for i in 0..texts.len() {
                    if raw >= lower[i] && raw <= upper[i] {
                        return texts.get(i).map(|s| s.as_str());
                    }
                }
                default.as_deref()
            }
            _ => None,
        }
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
        assert_eq!(c.convert(4.0), 14.0, "y = factor*x + offset");
    }

    #[test]
    fn applies_the_rational_conversion_in_spec_order() {
        // ASAM MDF4: y = (P1*x^2 + P2*x + P3) / (P4*x^2 + P5*x + P6),
        // with P1..P6 stored in cc_val order.
        let c = Conversion::Rational {
            coefficients: [1.0, 2.0, 3.0, 0.0, 0.0, 2.0],
        };
        // x = 2 -> (1*4 + 2*2 + 3) / 2 = 11/2
        assert_eq!(c.convert(2.0), 5.5);
    }

    #[test]
    fn rational_with_a_zero_denominator_follows_ieee() {
        let c = Conversion::Rational {
            coefficients: [0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        };
        assert!(c.convert(1.0).is_infinite());
    }

    #[test]
    fn interpolates_between_table_keys() {
        let c = Conversion::TableInterpolated {
            keys: vec![0.0, 10.0],
            values: vec![100.0, 200.0],
        };
        assert_eq!(c.convert(5.0), 150.0);
        assert_eq!(c.convert(0.0), 100.0);
        assert_eq!(c.convert(10.0), 200.0);
        assert_eq!(c.convert(-5.0), 100.0, "clamps below the first key");
        assert_eq!(c.convert(50.0), 200.0, "clamps above the last key");
    }

    #[test]
    fn table_lookup_does_not_interpolate() {
        let c = Conversion::TableLookup {
            keys: vec![0.0, 10.0],
            values: vec![100.0, 200.0],
        };
        assert_eq!(c.convert(1.0), 100.0);
        assert_eq!(c.convert(9.0), 200.0);
    }

    #[test]
    fn range_table_selects_the_matching_range() {
        let c = Conversion::RangeTable {
            lower: vec![0.0, 10.0],
            upper: vec![9.0, 19.0],
            values: vec![1.0, 2.0],
            default: Some(-1.0),
        };
        assert_eq!(c.convert(5.0), 1.0);
        assert_eq!(c.convert(15.0), 2.0);
        assert_eq!(c.convert(100.0), -1.0, "falls back to the default");
    }

    #[test]
    fn range_table_without_a_default_yields_nan() {
        let c = Conversion::RangeTable {
            lower: vec![0.0],
            upper: vec![1.0],
            values: vec![7.0],
            default: None,
        };
        assert!(c.convert(50.0).is_nan());
    }

    #[test]
    fn value_to_text_matches_exact_keys() {
        let c = Conversion::ValueToText {
            keys: vec![0.0, 1.0],
            texts: vec!["off".into(), "on".into()],
            default: Some("unknown".into()),
        };
        assert_eq!(c.convert_text(0.0), Some("off"));
        assert_eq!(c.convert_text(1.0), Some("on"));
        assert_eq!(c.convert_text(2.0), Some("unknown"));
        assert_eq!(c.output(), ConversionOutput::Text);
    }

    #[test]
    fn range_to_text_matches_inclusive_bounds() {
        let c = Conversion::RangeToText {
            lower: vec![0.0, 10.0],
            upper: vec![9.0, 19.0],
            texts: vec!["low".into(), "high".into()],
            default: None,
        };
        assert_eq!(c.convert_text(0.0), Some("low"));
        assert_eq!(c.convert_text(9.0), Some("low"));
        assert_eq!(c.convert_text(10.0), Some("high"));
        assert_eq!(c.convert_text(99.0), None);
    }

    #[test]
    fn text_conversions_have_no_numeric_result() {
        let c = Conversion::ValueToText {
            keys: vec![0.0],
            texts: vec!["off".into()],
            default: None,
        };
        assert!(c.convert(0.0).is_nan());
    }

    #[test]
    fn numeric_conversions_have_no_text_result() {
        let c = Conversion::Linear {
            offset: 0.0,
            factor: 1.0,
        };
        assert_eq!(c.convert_text(1.0), None);
    }

    #[test]
    fn evaluates_an_algebraic_conversion() {
        let expr = Expr::parse("2*X + 1").unwrap();
        let c = Conversion::Algebraic {
            formula: "2*X + 1".into(),
            expr,
        };
        assert_eq!(c.convert(3.0), 7.0);
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
}
