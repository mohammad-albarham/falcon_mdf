//! Channel Conversion (CC) block parsing.
//!
//! CC blocks define how to convert raw channel values to physical values.
//! They support linear scaling, polynomial, tabular lookups, and more.

use crate::error::{Mf4Error, Result};
use crate::blocks::common::{BlockHeader, read_link, BLOCK_HEADER_SIZE, ParseBlock};
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

impl CcBlock {
    /// Minimum size of the CC block.
    pub const MIN_SIZE: u64 = BLOCK_HEADER_SIZE as u64 + 4 * 8 + 24;

    /// Applies the conversion to a raw value.
    ///
    /// # Arguments
    /// * `raw` - The raw value to convert
    ///
    /// # Returns
    /// The converted physical value.
    pub fn convert(&self, raw: f64) -> f64 {
        match self.conversion_type {
            ConversionType::Identity => raw,
            ConversionType::Linear => {
                if self.values.len() >= 2 {
                    // y = p1 * x + p0
                    self.values[1] * raw + self.values[0]
                } else {
                    raw
                }
            }
            ConversionType::Rational => {
                if self.values.len() >= 6 {
                    let p0 = self.values[0];
                    let p1 = self.values[1];
                    let p2 = self.values[2];
                    let p3 = self.values[3];
                    let p4 = self.values[4];
                    let p5 = self.values[5];
                    let num = p0 + p1 * raw + p2 * raw * raw;
                    let den = p3 + p4 * raw + p5 * raw * raw;
                    if den.abs() > f64::EPSILON {
                        num / den
                    } else {
                        f64::NAN
                    }
                } else {
                    raw
                }
            }
            ConversionType::TabInterpolation => {
                self.interpolate_table(raw)
            }
            ConversionType::TabLookup => {
                self.lookup_table(raw)
            }
            _ => {
                // For other conversion types, return raw for now
                // A full implementation would handle all types
                raw
            }
        }
    }

    /// Performs table interpolation.
    fn interpolate_table(&self, raw: f64) -> f64 {
        let n = self.val_count as usize / 2;
        if n == 0 || self.values.len() < n * 2 {
            return raw;
        }

        // Values are stored as [x0, y0, x1, y1, ...]
        let keys: Vec<f64> = (0..n).map(|i| self.values[i * 2]).collect();
        let vals: Vec<f64> = (0..n).map(|i| self.values[i * 2 + 1]).collect();

        // Find the interpolation segment
        if raw <= keys[0] {
            return vals[0];
        }
        if raw >= keys[n - 1] {
            return vals[n - 1];
        }

        for i in 0..n - 1 {
            if raw >= keys[i] && raw <= keys[i + 1] {
                let t = (raw - keys[i]) / (keys[i + 1] - keys[i]);
                return vals[i] + t * (vals[i + 1] - vals[i]);
            }
        }

        raw
    }

    /// Performs table lookup (nearest value).
    fn lookup_table(&self, raw: f64) -> f64 {
        let n = self.val_count as usize / 2;
        if n == 0 || self.values.len() < n * 2 {
            return raw;
        }

        // Find nearest key
        let mut best_idx = 0;
        let mut best_diff = f64::MAX;

        for i in 0..n {
            let key = self.values[i * 2];
            let diff = (raw - key).abs();
            if diff < best_diff {
                best_diff = diff;
                best_idx = i;
            }
        }

        self.values[best_idx * 2 + 1]
    }
}

impl ParseBlock for CcBlock {
    fn parse(data: &[u8], offset: u64) -> Result<Self> {
        let header = BlockHeader::parse(data, offset)?;
        header.validate_type(b"##CC", offset)?;

        if header.length < Self::MIN_SIZE {
            return Err(Mf4Error::invalid_block_size("CC", header.length, Self::MIN_SIZE));
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

        let data_section = &data[data_start..];
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

/// A conversion that can be applied to raw values.
#[derive(Debug, Clone)]
pub enum Conversion {
    /// No conversion (identity).
    None,
    /// Linear conversion: y = factor * x + offset.
    Linear { offset: f64, factor: f64 },
    /// Rational conversion.
    Rational { coefficients: [f64; 6] },
    /// Table interpolation.
    Table { keys: Vec<f64>, values: Vec<f64> },
    /// Full CC block for complex conversions.
    Full(CcBlock),
}

impl Conversion {
    /// Creates a conversion from a CC block.
    pub fn from_cc_block(cc: CcBlock) -> Self {
        match cc.conversion_type {
            ConversionType::Identity => Conversion::None,
            ConversionType::Linear if cc.values.len() >= 2 => Conversion::Linear {
                offset: cc.values[0],
                factor: cc.values[1],
            },
            _ => Conversion::Full(cc),
        }
    }

    /// Converts a raw value to a physical value.
    pub fn convert(&self, raw: f64) -> f64 {
        match self {
            Conversion::None => raw,
            Conversion::Linear { offset, factor } => factor * raw + offset,
            Conversion::Rational { coefficients } => {
                let [p0, p1, p2, p3, p4, p5] = *coefficients;
                let num = p0 + p1 * raw + p2 * raw * raw;
                let den = p3 + p4 * raw + p5 * raw * raw;
                if den.abs() > f64::EPSILON {
                    num / den
                } else {
                    f64::NAN
                }
            }
            Conversion::Table { keys, values } => {
                if keys.is_empty() {
                    return raw;
                }
                // Simple linear interpolation
                for i in 0..keys.len() - 1 {
                    if raw >= keys[i] && raw <= keys[i + 1] {
                        let t = (raw - keys[i]) / (keys[i + 1] - keys[i]);
                        return values[i] + t * (values[i + 1] - values[i]);
                    }
                }
                if raw <= keys[0] {
                    values[0]
                } else {
                    *values.last().unwrap_or(&raw)
                }
            }
            Conversion::Full(cc) => cc.convert(raw),
        }
    }
}

impl Default for Conversion {
    fn default() -> Self {
        Conversion::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversion_identity() {
        let conv = Conversion::None;
        assert_eq!(conv.convert(42.0), 42.0);
        assert_eq!(conv.convert(-1.5), -1.5);
    }

    #[test]
    fn test_conversion_linear() {
        let conv = Conversion::Linear {
            offset: 10.0,
            factor: 2.0,
        };
        assert_eq!(conv.convert(0.0), 10.0);
        assert_eq!(conv.convert(5.0), 20.0);
        assert_eq!(conv.convert(-5.0), 0.0);
    }

    #[test]
    fn test_conversion_type_enum() {
        assert_eq!(ConversionType::from_u8(0), ConversionType::Identity);
        assert_eq!(ConversionType::from_u8(1), ConversionType::Linear);
        assert_eq!(ConversionType::from_u8(4), ConversionType::TabInterpolation);
        assert!(matches!(ConversionType::from_u8(99), ConversionType::Unknown(99)));
    }

    #[test]
    fn test_conversion_rational() {
        // y = (1 + 2*x) / (1 + 0*x) = 1 + 2*x
        let conv = Conversion::Rational {
            coefficients: [1.0, 2.0, 0.0, 1.0, 0.0, 0.0],
        };
        assert!((conv.convert(0.0) - 1.0).abs() < 0.001);
        assert!((conv.convert(1.0) - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_conversion_table() {
        let conv = Conversion::Table {
            keys: vec![0.0, 10.0, 20.0],
            values: vec![100.0, 200.0, 300.0],
        };
        // Exact points
        assert!((conv.convert(0.0) - 100.0).abs() < 0.001);
        assert!((conv.convert(10.0) - 200.0).abs() < 0.001);
        // Interpolated
        assert!((conv.convert(5.0) - 150.0).abs() < 0.001);
        // Extrapolated (clamped)
        assert!((conv.convert(-5.0) - 100.0).abs() < 0.001);
        assert!((conv.convert(25.0) - 300.0).abs() < 0.001);
    }
}
