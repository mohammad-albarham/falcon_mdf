//! MDF 3.x conversions: raw samples to physical values.
//!
//! A v3 `CCBLOCK` names one of eleven rules for turning a channel's stored
//! numbers into the quantity it measures. The rules are not the ones version 4
//! defines — v3 has polynomial, exponential and logarithmic forms that v4 has
//! no equivalent for, and its tables have their own boundary behaviour — so
//! they are evaluated here rather than through [`crate::blocks::conversion`].
//! The one piece genuinely shared is the algebraic formula parser
//! ([`crate::blocks::formula`]), which is about arithmetic rather than about
//! either format.
//!
//! A conversion this build does not recognise is carried as
//! [`Mdf3Conversion::Unsupported`] and fails by name when applied. It never
//! falls back to the identity: raw counts presented as physical values are the
//! one thing this crate promises not to return.

use std::cmp::Ordering;

use crate::blocks::formula::Expr;
use crate::error::{Mf4Error, Result};
use crate::io::ByteSource;

/// Bytes before the first parameter of any conversion block.
const HEADER_SIZE: usize = 46;

/// What a conversion produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mdf3ConversionOutput {
    /// A number. The channel's physical samples are `f64`.
    Numeric,
    /// A label. The channel's physical samples are text.
    Text,
}

/// One `CCBLOCK` rule.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Mdf3Conversion {
    /// No conversion; the raw values are already physical. Type 65535.
    None,
    /// `y = a·x + b`. Type 0.
    ///
    /// The block stores `b` before `a`, which is the opposite of the order the
    /// formula reads in.
    Linear {
        /// Multiplicative term.
        a: f64,
        /// Additive term.
        b: f64,
    },
    /// Tabular with linear interpolation between points. Type 1.
    TabularInterpolated {
        /// Raw-value keys, ascending.
        raw: Vec<f64>,
        /// Physical value at each key.
        phys: Vec<f64>,
    },
    /// Tabular without interpolation: the value of the nearest key. Type 2.
    Tabular {
        /// Raw-value keys, ascending.
        raw: Vec<f64>,
        /// Physical value at each key.
        phys: Vec<f64>,
    },
    /// Polynomial. Type 6.
    Polynomial {
        /// `P1` to `P6`.
        p: [f64; 6],
    },
    /// Exponential. Type 7.
    Exponential {
        /// `P1` to `P7`.
        p: [f64; 7],
    },
    /// Logarithmic. Type 8.
    Logarithmic {
        /// `P1` to `P7`.
        p: [f64; 7],
    },
    /// Rational. Type 9.
    Rational {
        /// `P1` to `P6`.
        p: [f64; 6],
    },
    /// An algebraic formula in the block's own text. Type 10.
    Formula {
        /// The text as stored, kept for diagnostics.
        text: String,
        /// The parsed form actually evaluated.
        expr: Expr,
    },
    /// Value to text. Type 11.
    TextTable {
        /// Raw-value keys, ascending.
        raw: Vec<f64>,
        /// The label each key maps to.
        text: Vec<String>,
    },
    /// Value range to text. Type 12.
    TextRangeTable {
        /// Lower bound of each range, ascending.
        lower: Vec<f64>,
        /// Upper bound of each range, ascending.
        upper: Vec<f64>,
        /// The label each range maps to.
        text: Vec<String>,
        /// The label for a value in no range.
        default: String,
    },
    /// A conversion type this build does not evaluate.
    ///
    /// Carried rather than dropped so that applying it can name what was
    /// found.
    Unsupported {
        /// The type code read from the block.
        code: u16,
        /// Why it could not be prepared.
        reason: String,
    },
}

impl Mdf3Conversion {
    /// Reads the conversion block at `addr`, or [`Self::None`] for a null
    /// link.
    pub fn parse(source: &dyn ByteSource, addr: u32) -> Result<Self> {
        if addr == 0 {
            return Ok(Self::None);
        }
        let head = source.read_bytes(addr as u64, 4)?;
        if &head[..2] != b"CC" {
            return Err(Mf4Error::InvalidBlockId {
                offset: addr as u64,
                expected: "CC".to_string(),
                actual: String::from_utf8_lossy(&head[..2]).to_string(),
            });
        }
        let declared = u16::from_le_bytes([head[2], head[3]]) as usize;

        // The type and the parameter count decide how long the block really
        // is, and they sit inside the first 46 bytes, so read those first.
        let header = read_at(source, addr, HEADER_SIZE, declared.max(HEADER_SIZE))?;
        let code = u16::from_le_bytes([header[42], header[43]]);
        let count = u16::from_le_bytes([header[44], header[45]]) as usize;

        // A block whose real length passes 65535 cannot say so in a 16-bit
        // field, and writers emit the saturated value rather than truncating
        // the block. asammdf handles the same case at
        // `v2_v3_blocks.py:1096-1108`. Believing the 65535 would cut a long
        // text table short and silently drop its last entries.
        let needed = params_len(code, count).map(|p| HEADER_SIZE + p);
        let len = match (declared == u16::MAX as usize, needed) {
            (true, Some(n)) => n,
            _ => declared,
        };
        if len < HEADER_SIZE {
            return Err(Mf4Error::InvalidBlockSize {
                block_type: "CC".to_string(),
                size: len as u64,
                min_size: HEADER_SIZE as u64,
            });
        }
        if let Some(n) = needed {
            if len < n {
                return Err(Mf4Error::InvalidBlockSize {
                    block_type: format!("CC (conversion type {code})"),
                    size: len as u64,
                    min_size: n as u64,
                });
            }
        }

        let data = read_at(source, addr, len, len)?;
        Self::from_block(source, &data, code, count)
    }

    /// Builds a conversion from a whole `CCBLOCK`.
    fn from_block(
        source: &dyn ByteSource,
        data: &[u8],
        code: u16,
        count: usize,
    ) -> Result<Self> {
        let f = |i: usize| f64_at(data, HEADER_SIZE + 8 * i);
        match code {
            65535 => Ok(Self::None),
            0 => Ok(Self::Linear {
                // Stored offset-first. Reading them in the order the formula
                // is written would scale by the offset and shift by the gain.
                b: f(0),
                a: f(1),
            }),
            1 | 2 => {
                let mut raw = Vec::with_capacity(count);
                let mut phys = Vec::with_capacity(count);
                for i in 0..count {
                    raw.push(f(2 * i));
                    phys.push(f(2 * i + 1));
                }
                ascending(&raw, "tabular conversion raw values")?;
                if code == 1 {
                    Ok(Self::TabularInterpolated { raw, phys })
                } else {
                    Ok(Self::Tabular { raw, phys })
                }
            }
            6 => Ok(Self::Polynomial {
                p: [f(0), f(1), f(2), f(3), f(4), f(5)],
            }),
            7 | 8 => {
                let p = [f(0), f(1), f(2), f(3), f(4), f(5), f(6)];
                // The two forms are selected by which of P1 and P4 is zero.
                // With neither zero the block names no formula at all, and
                // picking one would invent a rule the file does not state.
                if p[0] != 0.0 && p[3] != 0.0 {
                    return Err(Mf4Error::InvalidConversion {
                        message: format!(
                            "conversion type {code} needs P1 or P4 to be zero to say which \
                             of its two forms applies; this block has P1 = {} and P4 = {}",
                            p[0], p[3]
                        ),
                    });
                }
                if code == 7 {
                    Ok(Self::Exponential { p })
                } else {
                    Ok(Self::Logarithmic { p })
                }
            }
            9 => Ok(Self::Rational {
                p: [f(0), f(1), f(2), f(3), f(4), f(5)],
            }),
            10 => {
                let text = latin1(&data[HEADER_SIZE..]);
                let expr = Expr::parse(&text).map_err(|e| Mf4Error::InvalidConversion {
                    message: format!("conversion formula {text:?} could not be parsed: {e}"),
                })?;
                Ok(Self::Formula { text, expr })
            }
            11 => {
                // Each entry is a double and a 32-byte label, in the block
                // itself rather than behind a link.
                let mut pairs: Vec<(f64, String)> = (0..count)
                    .map(|i| {
                        let at = HEADER_SIZE + 40 * i;
                        (f64_at(data, at), latin1(&data[at + 8..at + 40]))
                    })
                    .collect();
                pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
                let (raw, text) = pairs.into_iter().unzip();
                Ok(Self::TextTable { raw, text })
            }
            12 => {
                // The first entry is the default; the rest are the ranges.
                if count == 0 {
                    return Err(Mf4Error::InvalidConversion {
                        message: "a value-range text table declares no entries, not even the \
                                  default one every such block carries"
                            .to_string(),
                    });
                }
                let default_addr = u32_at(data, HEADER_SIZE + 16);
                let default = read_tx(source, default_addr).unwrap_or_default();
                if default.contains("{X}") {
                    // asammdf reads a `{X}` default as an embedded `a*X+b`
                    // formula (`v2_v3_blocks.py:1568-1591`), which turns a text
                    // table into a numeric one for the unmatched values only.
                    // That is a vendor extension, not something the format
                    // states; refusing is better than guessing which half of a
                    // mixed result a caller wanted.
                    return Err(Mf4Error::unsupported(
                        "a value-range text table with a formula in its default entry",
                        format!("the default text {default:?} embeds {{X}}"),
                    ));
                }

                let n = count - 1;
                let mut lower = Vec::with_capacity(n);
                let mut upper = Vec::with_capacity(n);
                let mut text = Vec::with_capacity(n);
                for i in 0..n {
                    let at = HEADER_SIZE + 20 + 20 * i;
                    lower.push(f64_at(data, at));
                    upper.push(f64_at(data, at + 8));
                    text.push(read_tx(source, u32_at(data, at + 16)).unwrap_or_default());
                }
                ascending(&lower, "value-range table lower bounds")?;
                ascending(&upper, "value-range table upper bounds")?;
                Ok(Self::TextRangeTable {
                    lower,
                    upper,
                    text,
                    default,
                })
            }
            other => Ok(Self::Unsupported {
                code: other,
                reason: format!("conversion type {other} is not one this build evaluates"),
            }),
        }
    }

    /// Whether this rule leaves raw values as they are.
    ///
    /// A linear conversion of gain 1 and offset 0 counts, because writers emit
    /// one rather than omitting the block. A channel with an identity
    /// conversion keeps its stored type; every other rule produces `f64` or
    /// text.
    pub fn is_identity(&self) -> bool {
        match self {
            Self::None => true,
            Self::Linear { a, b } => *a == 1.0 && *b == 0.0,
            _ => false,
        }
    }

    /// Whether this rule produces numbers or labels.
    pub fn output(&self) -> Mdf3ConversionOutput {
        match self {
            Self::TextTable { .. } | Self::TextRangeTable { .. } => Mdf3ConversionOutput::Text,
            _ => Mdf3ConversionOutput::Numeric,
        }
    }

    /// Applies the rule to one raw value.
    ///
    /// # Errors
    ///
    /// Fails, naming the type, for a conversion this build does not evaluate
    /// and for one that produces text. Both are checked once per channel by
    /// the caller rather than once per sample.
    pub fn convert(&self, x: f64) -> Result<f64> {
        Ok(match self {
            Self::None => x,
            Self::Linear { a, b } => x * a + b,
            Self::TabularInterpolated { raw, phys } => interpolate(raw, phys, x),
            Self::Tabular { raw, phys } => nearest(raw, phys, x),
            Self::Polynomial { p } => {
                let [p1, p2, p3, p4, p5, p6] = *p;
                if (p2, p3, p5, p6) == (0.0, 0.0, 0.0, 0.0) {
                    // The short form. asammdf leaves the value alone when
                    // P1 == P4 (`v2_v3_blocks.py:1664-1669`), which agrees with
                    // the formula except at P1 == P4 == 0, where the formula is
                    // 0/0.
                    if p1 == p4 {
                        x
                    } else {
                        p4 * x / p1
                    }
                } else {
                    let s = x - p5 - p6;
                    (p2 - p4 * s) / (p3 * s - p1)
                }
            }
            Self::Exponential { p } => transcendental(p, x, f64::exp),
            Self::Logarithmic { p } => transcendental(p, x, f64::ln),
            Self::Rational { p } => {
                let [p1, p2, p3, p4, p5, p6] = *p;
                // The two degenerate forms are not just the general formula
                // simplified: at x = 0 the general one divides zero by zero.
                if (p1, p3, p4, p5) == (0.0, 0.0, 0.0, 0.0) {
                    if p2 == p6 {
                        x
                    } else {
                        x * (p2 / p6)
                    }
                } else if (p2, p3, p4, p6) == (0.0, 0.0, 0.0, 0.0) {
                    if p1 == p5 {
                        x
                    } else {
                        x * (p1 / p5)
                    }
                } else {
                    (p1 * x * x + p2 * x + p3) / (p4 * x * x + p5 * x + p6)
                }
            }
            Self::Formula { expr, .. } => expr.eval(x),
            Self::TextTable { .. } | Self::TextRangeTable { .. } => {
                return Err(Mf4Error::InvalidConversion {
                    message: "this channel's conversion produces text, not numbers".to_string(),
                })
            }
            Self::Unsupported { code, reason } => {
                return Err(Mf4Error::unsupported(
                    format!("MDF 3.x conversion type {code}"),
                    reason.clone(),
                ))
            }
        })
    }

    /// Applies a text-producing rule to one raw value.
    ///
    /// Returns `None` for a rule that produces numbers.
    pub fn convert_text(&self, x: f64) -> Option<&str> {
        match self {
            Self::TextTable { raw, text } => {
                // Exact match only. A raw value between two keys names no entry
                // and gets the empty label, which is what the format says and
                // what asammdf returns.
                match raw.binary_search_by(|k| k.total_cmp(&x)) {
                    Ok(i) => Some(text[i].as_str()),
                    Err(_) => Some(""),
                }
            }
            Self::TextRangeTable {
                lower,
                upper,
                text,
                default,
            } => {
                let i1 = lower.partition_point(|&l| l <= x);
                let i2 = upper.partition_point(|&u| u < x);
                // `i1 - 1` is the last range that starts at or before the
                // value; `i2` is the first that ends at or after it. They agree
                // only when exactly one range contains it — so a value on a
                // boundary two neighbouring ranges share belongs to neither and
                // takes the default. That is v3's rule, and it is not v4's; see
                // the module docs of `crate::blocks::conversion`.
                if i1 > 0 && i1 - 1 == i2 && i2 < text.len() {
                    Some(text[i2].as_str())
                } else {
                    Some(default.as_str())
                }
            }
            _ => None,
        }
    }
}

/// Both exponential and logarithmic conversions, which differ only in the
/// function applied.
fn transcendental(p: &[f64; 7], x: f64, func: fn(f64) -> f64) -> f64 {
    let [p1, p2, p3, p4, p5, p6, p7] = *p;
    if p4 == 0.0 {
        func(((x - p7) * p6 - p3) / p1) / p2
    } else {
        func((p3 / (x - p7) - p6) / p4) / p5
    }
}

/// Linear interpolation between the two nearest keys, clamped at both ends.
///
/// Written as `slope · (x − x₀) + y₀` rather than `y₀ + t · (y₁ − y₀)`, which
/// is the same value in exact arithmetic and not always the same `f64`. This
/// is the form `numpy.interp` uses, and asammdf's v3 tabular conversion is
/// `numpy.interp` — matching it is what makes the conformance test an equality
/// rather than a tolerance.
fn interpolate(keys: &[f64], values: &[f64], x: f64) -> f64 {
    let n = keys.len().min(values.len());
    if n == 0 {
        return x;
    }
    if x <= keys[0] {
        return values[0];
    }
    if x >= keys[n - 1] {
        return values[n - 1];
    }
    let hi = keys.partition_point(|&k| k <= x).min(n - 1);
    let lo = hi - 1;
    let slope = (values[hi] - values[lo]) / (keys[hi] - keys[lo]);
    slope * (x - keys[lo]) + values[lo]
}

/// The value of the nearest key, taking the lower one when two are equally
/// near — which is what asammdf's v3 comparison does at
/// `v2_v3_blocks.py:1518-1529`.
fn nearest(keys: &[f64], values: &[f64], x: f64) -> f64 {
    let n = keys.len().min(values.len());
    if n == 0 {
        return x;
    }
    let hi = keys.partition_point(|&k| k < x).min(n - 1);
    let lo = hi.saturating_sub(1);
    if (x - keys[hi]).abs() >= (x - keys[lo]).abs() {
        values[lo]
    } else {
        values[hi]
    }
}

/// How many bytes of parameters a conversion of this type and count carries,
/// where that is fixed by the type.
fn params_len(code: u16, count: usize) -> Option<usize> {
    match code {
        65535 => Some(0),
        0 => Some(16),
        1 | 2 => Some(16 * count),
        6 | 9 => Some(48),
        7 | 8 => Some(56),
        11 => Some(40 * count),
        12 => Some(20 * count),
        // A formula's length is the block's, and an unknown type's parameters
        // are not ours to size.
        _ => None,
    }
}

/// Reads `len` bytes at `addr`, requiring at least `min` of them.
fn read_at<'a>(
    source: &'a dyn ByteSource,
    addr: u32,
    min: usize,
    len: usize,
) -> Result<crate::io::ByteSlice<'a>> {
    let available = source.len().saturating_sub(addr as u64) as usize;
    if available < min {
        return Err(Mf4Error::TruncatedFile {
            offset: addr as u64,
            expected: min,
            actual: available,
        });
    }
    source.read_bytes(addr as u64, len.min(available))
}

/// Reads a `TX` block's text, returning `None` for a null link.
fn read_tx(source: &dyn ByteSource, addr: u32) -> Option<String> {
    if addr == 0 {
        return None;
    }
    let head = source.read_bytes(addr as u64, 4).ok()?;
    let len = u16::from_le_bytes([head[2], head[3]]) as usize;
    if len < 4 {
        return None;
    }
    let bytes = source.read_bytes(addr as u64, len).ok()?;
    super::blocks::parse_tx(&bytes, addr as u64).ok()
}

/// Refuses a table whose keys do not ascend.
///
/// Every lookup here is a binary search, and a binary search over unsorted keys
/// returns a wrong entry rather than no entry. asammdf has the same
/// requirement — `numpy.searchsorted` and `numpy.interp` both assume it — but
/// does not check it, so such a file decodes there to plausible wrong numbers.
fn ascending(keys: &[f64], what: &str) -> Result<()> {
    if let Some(i) = (1..keys.len()).find(|&i| {
        matches!(
            keys[i - 1].partial_cmp(&keys[i]),
            None | Some(Ordering::Greater)
        )
    }) {
        return Err(Mf4Error::InvalidConversion {
            message: format!(
                "{what} are not in ascending order: entry {} is {} after {}",
                i,
                keys[i],
                keys[i - 1]
            ),
        });
    }
    Ok(())
}

/// Reads a little-endian `f64`, or zero past the end of the block.
///
/// Past the end cannot happen: `parse` checks the block is long enough for the
/// parameters its type and count declare before any of this runs.
fn f64_at(data: &[u8], off: usize) -> f64 {
    match data.get(off..off + 8) {
        Some(b) => f64::from_le_bytes(b.try_into().expect("an eight-byte slice")),
        None => 0.0,
    }
}

/// Reads a little-endian `u32`, or zero past the end of the block.
fn u32_at(data: &[u8], off: usize) -> u32 {
    match data.get(off..off + 4) {
        Some(b) => u32::from_le_bytes(b.try_into().expect("a four-byte slice")),
        None => 0,
    }
}

/// Decodes a fixed field as Latin-1, cut at the first NUL and trimmed.
fn latin1(bytes: &[u8]) -> String {
    let cut = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..cut]
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linear_conversion_scales_then_offsets() {
        let c = Mdf3Conversion::Linear { a: 2.5, b: -1.25 };
        assert_eq!(c.convert(0.0).unwrap(), -1.25);
        assert_eq!(c.convert(2.0).unwrap(), 3.75);
    }

    #[test]
    fn a_linear_conversion_of_gain_one_is_the_identity() {
        assert!(Mdf3Conversion::Linear { a: 1.0, b: 0.0 }.is_identity());
        assert!(!Mdf3Conversion::Linear { a: 1.0, b: 0.5 }.is_identity());
        assert!(Mdf3Conversion::None.is_identity());
    }

    #[test]
    fn an_interpolated_table_clamps_outside_its_keys() {
        let c = Mdf3Conversion::TabularInterpolated {
            raw: vec![0.0, 3.0, 9.0],
            phys: vec![0.0, 30.0, 45.0],
        };
        assert_eq!(c.convert(-5.0).unwrap(), 0.0);
        assert_eq!(c.convert(0.0).unwrap(), 0.0);
        assert_eq!(c.convert(1.0).unwrap(), 10.0);
        assert_eq!(c.convert(6.0).unwrap(), 37.5);
        assert_eq!(c.convert(99.0).unwrap(), 45.0);
    }

    #[test]
    fn a_plain_table_takes_the_nearer_key_and_the_lower_one_on_a_tie() {
        let c = Mdf3Conversion::Tabular {
            raw: vec![0.0, 10.0, 20.0],
            phys: vec![1.0, 2.0, 3.0],
        };
        assert_eq!(c.convert(4.0).unwrap(), 1.0);
        assert_eq!(c.convert(6.0).unwrap(), 2.0);
        // Exactly between two keys: the lower one wins.
        assert_eq!(c.convert(5.0).unwrap(), 1.0);
        assert_eq!(c.convert(15.0).unwrap(), 2.0);
        assert_eq!(c.convert(-1.0).unwrap(), 1.0);
        assert_eq!(c.convert(100.0).unwrap(), 3.0);
    }

    #[test]
    fn a_table_whose_keys_do_not_ascend_is_refused() {
        assert!(matches!(
            ascending(&[0.0, 5.0, 2.0], "keys"),
            Err(Mf4Error::InvalidConversion { .. })
        ));
        assert!(ascending(&[0.0, 2.0, 2.0, 5.0], "keys").is_ok());
    }

    #[test]
    fn a_range_table_gives_the_default_on_a_shared_boundary() {
        let c = Mdf3Conversion::TextRangeTable {
            lower: vec![0.0, 2.0, 5.0],
            upper: vec![2.0, 5.0, 9.0],
            text: vec!["low".into(), "mid".into(), "high".into()],
            default: "none".into(),
        };
        assert_eq!(c.convert_text(0.0), Some("low"));
        assert_eq!(c.convert_text(1.0), Some("low"));
        // 2 ends one range and starts the next, so it belongs to neither.
        assert_eq!(c.convert_text(2.0), Some("none"));
        assert_eq!(c.convert_text(3.0), Some("mid"));
        assert_eq!(c.convert_text(5.0), Some("none"));
        assert_eq!(c.convert_text(6.0), Some("high"));
        // The last upper bound is not shared, so it is inclusive.
        assert_eq!(c.convert_text(9.0), Some("high"));
        assert_eq!(c.convert_text(9.5), Some("none"));
        assert_eq!(c.convert_text(-1.0), Some("none"));
    }

    #[test]
    fn a_text_table_matches_exactly_or_not_at_all() {
        let c = Mdf3Conversion::TextTable {
            raw: vec![1.0, 3.0, 5.0],
            text: vec!["one".into(), "three".into(), "five".into()],
        };
        assert_eq!(c.convert_text(1.0), Some("one"));
        assert_eq!(c.convert_text(5.0), Some("five"));
        assert_eq!(c.convert_text(2.0), Some(""));
        assert_eq!(c.convert_text(0.0), Some(""));
    }

    #[test]
    fn an_unsupported_conversion_fails_by_name_rather_than_passing_values_through() {
        let c = Mdf3Conversion::Unsupported {
            code: 42,
            reason: "made up".into(),
        };
        let err = c.convert(7.0).unwrap_err();
        assert!(matches!(err, Mf4Error::Unsupported { .. }));
        assert!(err.to_string().contains("42"), "{err}");
    }

    #[test]
    fn an_exponential_conversion_uses_the_form_its_zero_parameter_selects() {
        // P4 == 0: exp(((x - P7) * P6 - P3) / P1) / P2.
        let c = Mdf3Conversion::Exponential {
            p: [2.0, 4.0, 1.0, 0.0, 0.0, 3.0, 1.0],
        };
        let x = 5.0f64;
        assert_eq!(c.convert(x).unwrap(), (((x - 1.0) * 3.0 - 1.0) / 2.0).exp() / 4.0);

        // P1 == 0: log((P3 / (x - P7) - P6) / P4) / P5.
        let l = Mdf3Conversion::Logarithmic {
            p: [0.0, 0.0, 6.0, 2.0, 3.0, 1.0, 0.5],
        };
        assert_eq!(l.convert(x).unwrap(), ((6.0 / (x - 0.5) - 1.0) / 2.0).ln() / 3.0);
    }
}
