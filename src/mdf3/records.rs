//! Decoding samples out of an MDF 3.x record stream.
//!
//! A version 3 data group is a flat run of fixed-size records with no length
//! of its own: how many bytes it occupies has to be derived from the channel
//! groups that share it. That derivation is the dangerous part of this format,
//! so everything here validates before it does arithmetic — a record size, a
//! bit offset or a record identifier that does not fit what is actually in the
//! file produces a named error rather than a shifted read that decodes to
//! plausible wrong numbers.

use crate::error::{Mf4Error, Result};
use crate::io::ByteSource;
use crate::model::SignalValues;

use super::{Mdf3Channel, Mdf3DataGroup};

/// What a v3 channel's raw samples are.
///
/// The v3 data type code carries the byte order as well as the kind, and does
/// so in two ways: codes 9–12 are always big-endian and 13–16 always
/// little-endian, while codes 0–3 defer to the byte order the identification
/// block declares for the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mdf3SampleKind {
    /// An unsigned integer of `bit_count` bits.
    Unsigned,
    /// A two's-complement signed integer of `bit_count` bits.
    Signed,
    /// An IEEE 754 float, 32 or 64 bits wide.
    Float,
    /// A VAX F-floating 32-bit float.
    VaxF,
    /// A VAX D-floating 64-bit float.
    VaxD,
    /// A VAX G-floating 64-bit float.
    VaxG,
    /// Text, `bit_count / 8` bytes per sample.
    String,
    /// An opaque byte run, `bit_count / 8` bytes per sample.
    ByteArray,
}

/// A v3 signal data type code, resolved into a kind and a byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mdf3SampleFormat {
    /// What the samples are.
    pub kind: Mdf3SampleKind,
    /// Whether the sample bytes are stored most-significant first.
    pub big_endian: bool,
}

impl Mdf3SampleFormat {
    /// Resolves a `CNBLOCK` data type code against the file's declared byte
    /// order.
    pub fn from_code(code: u16, file_big_endian: bool) -> Result<Self> {
        use Mdf3SampleKind::*;
        let (kind, big_endian) = match code {
            0 => (Unsigned, file_big_endian),
            1 => (Signed, file_big_endian),
            2 | 3 => (Float, file_big_endian),
            4 => (VaxF, false),
            5 => (VaxD, false),
            6 => (VaxG, false),
            7 => (String, false),
            8 => (ByteArray, false),
            9 => (Unsigned, true),
            10 => (Signed, true),
            11 | 12 => (Float, true),
            13 => (Unsigned, false),
            14 => (Signed, false),
            15 | 16 => (Float, false),
            other => {
                return Err(Mf4Error::unsupported(
                    format!("MDF 3.x signal data type {other}"),
                    "this build decodes the integer, IEEE 754, VAX floating point, string and byte-array \
                     types; any unknown code is refused rather than guessed at",
                ))
            }
        };
        Ok(Self { kind, big_endian })
    }
}

/// Where one channel sits in a record, checked against that record's size.
#[derive(Debug, Clone, Copy)]
struct ChannelLayout {
    format: Mdf3SampleFormat,
    /// First byte of the channel within the record.
    byte_offset: usize,
    /// Bits to discard at the low end of those bytes.
    bit_offset: u32,
    /// Width of the value in bits.
    bit_count: u32,
    /// Bytes the value spans, including the discarded low bits.
    byte_span: usize,
}

impl ChannelLayout {
    /// Works out where a channel sits, refusing anything that does not fit.
    fn build(channel: &Mdf3Channel, record_size: usize, file_big_endian: bool) -> Result<Self> {
        let format = Mdf3SampleFormat::from_code(channel.data_type, file_big_endian)?;

        // `start_offset` is 16 bits, so it cannot address past byte 8191 on its
        // own; records longer than that carry the rest in
        // `additional_byte_offset`. Both are added in `usize` because their sum
        // does not fit either field.
        let start_bit = channel.start_offset as usize + channel.additional_byte_offset as usize * 8;
        let bit_count = channel.bit_count as usize;
        let end_byte = (start_bit + bit_count).div_ceil(8);

        if end_byte > record_size {
            return Err(Mf4Error::InvalidDataBlock {
                message: format!(
                    "channel {:?} claims bits {}..{} of a {}-byte record, which ends at bit {}",
                    channel.name,
                    start_bit,
                    start_bit + bit_count,
                    record_size,
                    record_size * 8
                ),
            });
        }

        let byte_offset = start_bit / 8;
        let bit_offset = (start_bit % 8) as u32;

        match format.kind {
            Mdf3SampleKind::String | Mdf3SampleKind::ByteArray => {
                // These are byte runs, not bit fields. A run that starts or
                // ends mid-byte is not something the format describes, and
                // rounding it to byte boundaries would return neighbouring
                // channels' bytes as part of the text.
                if bit_offset != 0 || !bit_count.is_multiple_of(8) {
                    return Err(Mf4Error::InvalidDataBlock {
                        message: format!(
                            "channel {:?} is a text or byte-array channel of {bit_count} bits \
                             at bit offset {bit_offset}; both must be whole bytes",
                            channel.name
                        ),
                    });
                }
            }
            Mdf3SampleKind::Float => {
                if bit_count != 32 && bit_count != 64 {
                    return Err(Mf4Error::unsupported(
                        format!("a {bit_count}-bit IEEE 754 float"),
                        format!(
                            "channel {:?} declares a floating point type of a width \
                             IEEE 754 does not define",
                            channel.name
                        ),
                    ));
                }
            }
            Mdf3SampleKind::VaxF => {
                if bit_count != 32 {
                    return Err(Mf4Error::unsupported(
                        format!("a {bit_count}-bit VAX F float"),
                        format!(
                            "channel {:?} declares a VAX F floating point type of a width \
                             other than 32 bits",
                            channel.name
                        ),
                    ));
                }
            }
            Mdf3SampleKind::VaxD => {
                if bit_count != 64 {
                    return Err(Mf4Error::unsupported(
                        format!("a {bit_count}-bit VAX D float"),
                        format!(
                            "channel {:?} declares a VAX D floating point type of a width \
                             other than 64 bits",
                            channel.name
                        ),
                    ));
                }
            }
            Mdf3SampleKind::VaxG => {
                if bit_count != 64 {
                    return Err(Mf4Error::unsupported(
                        format!("a {bit_count}-bit VAX G float"),
                        format!(
                            "channel {:?} declares a VAX G floating point type of a width \
                             other than 64 bits",
                            channel.name
                        ),
                    ));
                }
            }
            Mdf3SampleKind::Unsigned | Mdf3SampleKind::Signed => {
                if bit_count > 64 {
                    return Err(Mf4Error::unsupported(
                        format!("a {bit_count}-bit integer"),
                        format!(
                            "channel {:?} is wider than the widest integer this build \
                             decodes; its bytes are readable but its value is not",
                            channel.name
                        ),
                    ));
                }
            }
        }

        Ok(Self {
            format,
            byte_offset,
            bit_offset,
            bit_count: bit_count as u32,
            byte_span: (bit_offset as usize + bit_count).div_ceil(8),
        })
    }

    /// Pulls this channel's bits out of one record.
    ///
    /// The record has already been checked to be `record_size` bytes and the
    /// layout to fit inside it, so the slice below cannot be out of range.
    fn bits(&self, record: &[u8]) -> u128 {
        let bytes = &record[self.byte_offset..self.byte_offset + self.byte_span];
        let mut raw: u128 = 0;
        if self.format.big_endian {
            for &b in bytes {
                raw = (raw << 8) | b as u128;
            }
        } else {
            for (i, &b) in bytes.iter().enumerate() {
                raw |= (b as u128) << (8 * i);
            }
        }
        let shifted = raw >> self.bit_offset;
        if self.bit_count >= 128 {
            shifted
        } else {
            shifted & ((1u128 << self.bit_count) - 1)
        }
    }
}

/// Collects samples straight into the vector of the channel's own type, so an
/// integer channel never passes through `f64` on its way out.
enum Acc {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    VaxF(Vec<f64>),
    VaxD(Vec<f64>),
    VaxG(Vec<f64>),
    Str(Vec<String>),
    Bytes { data: Vec<u8>, width: usize },
}

impl Acc {
    /// Chooses the accumulator for a layout, rounding an integer up to the
    /// narrowest standard width that holds it.
    fn for_layout(layout: &ChannelLayout, capacity: usize) -> Self {
        let bytes = layout.bit_count.div_ceil(8);
        match layout.format.kind {
            Mdf3SampleKind::Unsigned => match bytes {
                0..=1 => Acc::U8(Vec::with_capacity(capacity)),
                2 => Acc::U16(Vec::with_capacity(capacity)),
                3..=4 => Acc::U32(Vec::with_capacity(capacity)),
                _ => Acc::U64(Vec::with_capacity(capacity)),
            },
            Mdf3SampleKind::Signed => match bytes {
                0..=1 => Acc::I8(Vec::with_capacity(capacity)),
                2 => Acc::I16(Vec::with_capacity(capacity)),
                3..=4 => Acc::I32(Vec::with_capacity(capacity)),
                _ => Acc::I64(Vec::with_capacity(capacity)),
            },
            Mdf3SampleKind::Float => {
                if layout.bit_count == 32 {
                    Acc::F32(Vec::with_capacity(capacity))
                } else {
                    Acc::F64(Vec::with_capacity(capacity))
                }
            }
            Mdf3SampleKind::VaxF => Acc::VaxF(Vec::with_capacity(capacity)),
            Mdf3SampleKind::VaxD => Acc::VaxD(Vec::with_capacity(capacity)),
            Mdf3SampleKind::VaxG => Acc::VaxG(Vec::with_capacity(capacity)),
            Mdf3SampleKind::String => Acc::Str(Vec::with_capacity(capacity)),
            Mdf3SampleKind::ByteArray => Acc::Bytes {
                data: Vec::with_capacity(capacity * (layout.bit_count as usize / 8)),
                width: layout.bit_count as usize / 8,
            },
        }
    }

    /// Adds one sample, read out of `record` according to `layout`.
    fn push(&mut self, layout: &ChannelLayout, record: &[u8]) {
        let bytes = &record[layout.byte_offset..layout.byte_offset + layout.byte_span];
        let signed = |l: &ChannelLayout| sign_extend(l.bits(record), l.bit_count);
        match self {
            Acc::U8(v) => v.push(layout.bits(record) as u8),
            Acc::U16(v) => v.push(layout.bits(record) as u16),
            Acc::U32(v) => v.push(layout.bits(record) as u32),
            Acc::U64(v) => v.push(layout.bits(record) as u64),
            Acc::I8(v) => v.push(signed(layout) as i8),
            Acc::I16(v) => v.push(signed(layout) as i16),
            Acc::I32(v) => v.push(signed(layout) as i32),
            Acc::I64(v) => v.push(signed(layout) as i64),
            Acc::F32(v) => v.push(f32::from_bits(layout.bits(record) as u32)),
            Acc::F64(v) => v.push(f64::from_bits(layout.bits(record) as u64)),
            Acc::VaxF(v) => {
                let b = (layout.bits(record) as u32).to_le_bytes();
                v.push(decode_vax_f(b));
            }
            Acc::VaxD(v) => {
                let b = (layout.bits(record) as u64).to_le_bytes();
                v.push(decode_vax_d(b));
            }
            Acc::VaxG(v) => {
                let b = (layout.bits(record) as u64).to_le_bytes();
                v.push(decode_vax_g(b));
            }
            Acc::Str(v) => {
                let cut = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                // v3 text is Latin-1, which maps byte for byte onto the first
                // 256 code points. Decoding it as UTF-8 would replace every
                // accented character with U+FFFD.
                v.push(bytes[..cut].iter().map(|&b| b as char).collect());
            }
            Acc::Bytes { data, .. } => data.extend_from_slice(bytes),
        }
    }

    fn into_values(self) -> SignalValues {
        match self {
            Acc::U8(v) => SignalValues::U8(v),
            Acc::U16(v) => SignalValues::U16(v),
            Acc::U32(v) => SignalValues::U32(v),
            Acc::U64(v) => SignalValues::U64(v),
            Acc::I8(v) => SignalValues::I8(v),
            Acc::I16(v) => SignalValues::I16(v),
            Acc::I32(v) => SignalValues::I32(v),
            Acc::I64(v) => SignalValues::I64(v),
            Acc::F32(v) => SignalValues::F32(v),
            Acc::F64(v) | Acc::VaxF(v) | Acc::VaxD(v) | Acc::VaxG(v) => SignalValues::F64(v),
            Acc::Str(v) => SignalValues::Str(v),
            Acc::Bytes { data, width } => SignalValues::Bytes { data, width },
        }
    }
}

/// Decodes a 32-bit VAX F-floating value to `f64`.
///
/// VAX F-float format (32 bits):
/// - Stored as two 16-bit little-endian words in memory (Word 0, Word 1).
/// - Word 0: bit 15 = sign, bits 14..7 = 8-bit exponent (excess-128), bits 6..0 = fraction[22..16].
/// - Word 1: bits 15..0 = fraction[15..0].
/// - Exponent 0 with sign 0 represents true zero (0.0).
/// - Exponent 0 with sign 1 is a reserved operand; decoded as NaN.
/// - Exponent > 0 represents normalized number: $(-1)^s \times 2^{e - 128} \times (0.5 + \text{fraction} / 2^{24})$.
pub fn decode_vax_f(bytes: [u8; 4]) -> f64 {
    let word0 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let word1 = u16::from_le_bytes([bytes[2], bytes[3]]);

    let sign = (word0 >> 15) & 1;
    let exponent = (word0 >> 7) & 0xFF;
    let fraction = (((word0 & 0x7F) as u32) << 16) | (word1 as u32);

    if exponent == 0 {
        if sign == 0 {
            0.0
        } else {
            // Exponent 0 with sign 1 is a VAX reserved operand; we decode it as NaN.
            f64::NAN
        }
    } else {
        let sign_mul = if sign == 1 { -1.0 } else { 1.0 };
        let mantissa = 1.0 + (fraction as f64) * 2.0f64.powi(-23);
        sign_mul * mantissa * 2.0f64.powi(exponent as i32 - 129)
    }
}

/// Decodes a 64-bit VAX D-floating value to `f64`.
///
/// VAX D-float format (64 bits):
/// - Stored as four 16-bit little-endian words in memory (Word 0, Word 1, Word 2, Word 3).
/// - Word 0: bit 15 = sign, bits 14..7 = 8-bit exponent (excess-128), bits 6..0 = fraction[54..48].
/// - Word 1: bits 15..0 = fraction[47..32].
/// - Word 2: bits 15..0 = fraction[31..16].
/// - Word 3: bits 15..0 = fraction[15..0].
/// - Exponent 0 with sign 0 represents true zero (0.0).
/// - Exponent 0 with sign 1 is a reserved operand; decoded as NaN.
/// - Exponent > 0 represents normalized number: $(-1)^s \times 2^{e - 128} \times (0.5 + \text{fraction} / 2^{56})$.
pub fn decode_vax_d(bytes: [u8; 8]) -> f64 {
    let word0 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let word1 = u16::from_le_bytes([bytes[2], bytes[3]]);
    let word2 = u16::from_le_bytes([bytes[4], bytes[5]]);
    let word3 = u16::from_le_bytes([bytes[6], bytes[7]]);

    let sign = (word0 >> 15) & 1;
    let exponent = (word0 >> 7) & 0xFF;
    let fraction = (((word0 & 0x7F) as u64) << 48)
        | ((word1 as u64) << 32)
        | ((word2 as u64) << 16)
        | (word3 as u64);

    if exponent == 0 {
        if sign == 0 {
            0.0
        } else {
            // Exponent 0 with sign 1 is a VAX reserved operand; we decode it as NaN.
            f64::NAN
        }
    } else {
        let sign_mul = if sign == 1 { -1.0 } else { 1.0 };
        let mantissa = 1.0 + (fraction as f64) * 2.0f64.powi(-55);
        sign_mul * mantissa * 2.0f64.powi(exponent as i32 - 129)
    }
}

/// Decodes a 64-bit VAX G-floating value to `f64`.
///
/// VAX G-float format (64 bits):
/// - Stored as four 16-bit little-endian words in memory (Word 0, Word 1, Word 2, Word 3).
/// - Word 0: bit 15 = sign, bits 14..4 = 11-bit exponent (excess-1024), bits 3..0 = fraction[51..48].
/// - Word 1: bits 15..0 = fraction[47..32].
/// - Word 2: bits 15..0 = fraction[31..16].
/// - Word 3: bits 15..0 = fraction[15..0].
/// - Exponent 0 with sign 0 represents true zero (0.0).
/// - Exponent 0 with sign 1 is a reserved operand; decoded as NaN.
/// - Exponent > 0 represents normalized number: $(-1)^s \times 2^{e - 1024} \times (0.5 + \text{fraction} / 2^{53})$.
pub fn decode_vax_g(bytes: [u8; 8]) -> f64 {
    let word0 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let word1 = u16::from_le_bytes([bytes[2], bytes[3]]);
    let word2 = u16::from_le_bytes([bytes[4], bytes[5]]);
    let word3 = u16::from_le_bytes([bytes[6], bytes[7]]);

    let sign = (word0 >> 15) & 1;
    let exponent = (word0 >> 4) & 0x7FF;
    let fraction = (((word0 & 0x0F) as u64) << 48)
        | ((word1 as u64) << 32)
        | ((word2 as u64) << 16)
        | (word3 as u64);

    if exponent == 0 {
        if sign == 0 {
            0.0
        } else {
            // Exponent 0 with sign 1 is a VAX reserved operand; we decode it as NaN.
            f64::NAN
        }
    } else {
        let sign_mul = if sign == 1 { -1.0 } else { 1.0 };
        let mantissa = 1.0 + (fraction as f64) * 2.0f64.powi(-52);
        sign_mul * mantissa * 2.0f64.powi(exponent as i32 - 1025)
    }
}

/// Reinterprets the low `bit_count` bits of `bits` as two's complement.
fn sign_extend(bits: u128, bit_count: u32) -> i128 {
    if bit_count == 0 || bit_count >= 128 {
        return bits as i128;
    }
    let sign = 1u128 << (bit_count - 1);
    if bits & sign != 0 {
        (bits | !((1u128 << bit_count) - 1)) as i128
    } else {
        bits as i128
    }
}

/// How a data group's records are laid out on disk.
///
/// A v3 data group has no length field. Its size is the sum over its channel
/// groups of `(record size + record identifiers) × cycles`, and that sum is
/// exactly the kind of derived byte count that has silently corrupted reads in
/// this repository before — so it is checked against the file before use.
struct RecordPlan {
    /// How many copies of the record identifier each record carries: 0 for a
    /// sorted group, 1 for an identifier before each record, 2 for one before
    /// and a copy after.
    id_count: u16,
    /// Record size in bytes for each identifier present in the group, indexed
    /// by identifier.
    sizes: Vec<Option<usize>>,
    /// Declared cycles for each identifier, for the count check afterwards.
    cycles: Vec<u64>,
    /// Total bytes the group's records occupy.
    total_size: usize,
}

impl RecordPlan {
    fn build(dg: &Mdf3DataGroup) -> Result<Self> {
        let id_count = dg.record_id_count;

        if id_count == 0 && dg.channel_groups.len() > 1 {
            return Err(Mf4Error::InvalidDataBlock {
                message: format!(
                    "data group holds {} channel groups but writes no record identifier, \
                     so its records cannot be told apart",
                    dg.channel_groups.len()
                ),
            });
        }

        // Identifiers index this table directly, so it has to span every value
        // one byte can hold.
        let mut sizes: Vec<Option<usize>> = vec![None; 256];
        let mut cycles: Vec<u64> = vec![0; 256];
        let mut total_size: usize = 0;

        for cg in &dg.channel_groups {
            let id = if id_count == 0 { 0 } else { cg.record_id };
            if id > u8::MAX as u16 {
                return Err(Mf4Error::InvalidDataBlock {
                    message: format!(
                        "channel group declares record identifier {id}, but a v3 record \
                         identifier is a single byte"
                    ),
                });
            }
            let id = id as usize;
            if sizes[id].is_some() {
                return Err(Mf4Error::InvalidDataBlock {
                    message: format!(
                        "two channel groups in one data group share record identifier {id}"
                    ),
                });
            }
            sizes[id] = Some(cg.record_size as usize);
            cycles[id] = cg.cycle_count as u64;

            // u64 throughout: cycles is 32 bits and the record can be 64KiB, so
            // the product overflows a 32-bit count and can exceed a 32-bit file
            // offset even though v3 addresses are 32 bits. Overflow here would
            // be a short read presented as a whole one.
            let per_record = cg.record_size as u64 + id_count as u64;
            let bytes = per_record
                .checked_mul(cg.cycle_count as u64)
                .and_then(|b| b.checked_add(total_size as u64))
                .ok_or_else(|| Mf4Error::InvalidDataBlock {
                    message: "data group declares more record bytes than can be addressed"
                        .to_string(),
                })?;
            total_size = usize::try_from(bytes).map_err(|_| Mf4Error::InvalidDataBlock {
                message: "data group declares more record bytes than fit in memory".to_string(),
            })?;
        }

        Ok(Self {
            id_count,
            sizes,
            cycles,
            total_size,
        })
    }
}

/// Reads one channel's raw samples out of a data group's record stream.
pub(super) fn read_channel(
    source: &dyn ByteSource,
    file_big_endian: bool,
    dg: &Mdf3DataGroup,
    cg_index: usize,
    ch_index: usize,
) -> Result<SignalValues> {
    let cg = dg
        .channel_groups
        .get(cg_index)
        .ok_or_else(|| Mf4Error::parse_error(format!("no channel group {cg_index} in this data group")))?;
    let channel = cg.channels.get(ch_index).ok_or_else(|| {
        Mf4Error::parse_error(format!("no channel {ch_index} in channel group {cg_index}"))
    })?;

    let record_size = cg.record_size as usize;
    let layout = ChannelLayout::build(channel, record_size, file_big_endian)?;
    let plan = RecordPlan::build(dg)?;
    let target_id = if plan.id_count == 0 { 0 } else { cg.record_id };

    let mut acc = Acc::for_layout(&layout, cg.cycle_count as usize);

    if plan.total_size == 0 {
        return Ok(acc.into_values());
    }

    if dg.data_block_addr == 0 {
        return Err(Mf4Error::InvalidDataBlock {
            message: format!(
                "data group declares {} bytes of records but no data block address",
                plan.total_size
            ),
        });
    }

    // The declared size against what the file actually holds. This is the check
    // that turns a corrupt count into an error instead of a read that runs off
    // the end of the last record and decodes whatever follows.
    let addr = dg.data_block_addr as u64;
    let available = source.len().saturating_sub(addr);
    if (plan.total_size as u64) > available {
        return Err(Mf4Error::TruncatedFile {
            offset: addr,
            expected: plan.total_size,
            actual: available as usize,
        });
    }

    let block = source.read_bytes(addr, plan.total_size)?;
    let trailing = if plan.id_count == 2 { 1 } else { 0 };

    let mut counts: Vec<u64> = vec![0; 256];
    let mut pos = 0usize;
    while pos < block.len() {
        let (id, start) = if plan.id_count == 0 {
            (target_id, pos)
        } else {
            (block[pos] as u16, pos + 1)
        };

        let size = plan.sizes[id as usize].ok_or_else(|| Mf4Error::InvalidDataBlock {
            message: format!(
                "record at byte {pos} of the data block carries identifier {id}, which no \
                 channel group in this data group claims; its length is therefore unknown"
            ),
        })?;

        let end = start + size;
        if end + trailing > block.len() {
            return Err(Mf4Error::TruncatedFile {
                offset: addr + start as u64,
                expected: size + trailing,
                actual: block.len().saturating_sub(start),
            });
        }

        // The trailing copy exists so a reader can confirm it is still aligned.
        // Checking it is the cheapest guard there is against decoding the rest
        // of the stream one byte out.
        if trailing == 1 && block[end] as u16 != id {
            return Err(Mf4Error::InvalidDataBlock {
                message: format!(
                    "record at byte {pos} opens with identifier {id} and closes with {}; \
                     the trailing copy must repeat the leading one",
                    block[end]
                ),
            });
        }

        if id == target_id {
            acc.push(&layout, &block[start..end]);
        }
        counts[id as usize] += 1;
        pos = end + trailing;
    }

    // The block length came from the declared cycle counts, so records that do
    // not add up to them mean the identifiers in the stream disagree with the
    // channel groups. Reporting the samples found would silently drop or
    // duplicate cycles.
    for (id, &declared) in plan.cycles.iter().enumerate() {
        if plan.sizes[id].is_some() && counts[id] != declared {
            return Err(Mf4Error::InvalidDataBlock {
                message: format!(
                    "channel group with record identifier {id} declares {declared} cycles \
                     but the data block holds {} records for it",
                    counts[id]
                ),
            });
        }
    }

    Ok(acc.into_values())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(start_bit: usize, bit_count: usize, code: u16) -> ChannelLayout {
        ChannelLayout {
            format: Mdf3SampleFormat::from_code(code, false).unwrap(),
            byte_offset: start_bit / 8,
            bit_offset: (start_bit % 8) as u32,
            bit_count: bit_count as u32,
            byte_span: ((start_bit % 8) + bit_count).div_ceil(8),
        }
    }

    #[test]
    fn a_little_endian_field_is_read_from_the_low_bits_up() {
        // 0xF1 0x2C little-endian is 0x2CF1; bits 4..16 of it are 0x2CF.
        let record = [0xF1u8, 0x2C];
        assert_eq!(layout(4, 12, 13).bits(&record), 0x2CF);
    }

    #[test]
    fn a_big_endian_field_is_read_most_significant_byte_first() {
        // 0xF1 0x2C big-endian is 0xF12C; shifted down by 4 and masked to 12
        // bits that is 0xF12.
        let record = [0xF1u8, 0x2C];
        assert_eq!(layout(4, 12, 9).bits(&record), 0xF12);
    }

    #[test]
    fn a_field_spanning_three_bytes_keeps_its_high_bits() {
        // bits 6..24 of 0x01 0x02 0x03 read little-endian (0x030201) are
        // 0x030201 >> 6 = 0xC08, masked to 18 bits.
        let record = [0x01u8, 0x02, 0x03];
        assert_eq!(layout(6, 18, 13).bits(&record), 0x030201u128 >> 6);
    }

    #[test]
    fn a_negative_value_narrower_than_its_type_sign_extends() {
        assert_eq!(sign_extend(0b1101, 4), -3);
        assert_eq!(sign_extend(0b0101, 4), 5);
        assert_eq!(sign_extend(0xFFF, 12), -1);
        assert_eq!(sign_extend(0x7FF, 12), 2047);
    }

    #[test]
    fn a_data_type_code_resolves_its_own_byte_order_before_the_files() {
        // 9 is Motorola whatever the file says, 13 is Intel whatever it says,
        // and 0 follows the file.
        assert!(Mdf3SampleFormat::from_code(9, false).unwrap().big_endian);
        assert!(!Mdf3SampleFormat::from_code(13, true).unwrap().big_endian);
        assert!(Mdf3SampleFormat::from_code(0, true).unwrap().big_endian);
        assert!(!Mdf3SampleFormat::from_code(0, false).unwrap().big_endian);
    }

    #[test]
    fn vax_float_codes_resolve_to_vax_formats() {
        assert_eq!(
            Mdf3SampleFormat::from_code(4, false).unwrap(),
            Mdf3SampleFormat {
                kind: Mdf3SampleKind::VaxF,
                big_endian: false,
            }
        );
        assert_eq!(
            Mdf3SampleFormat::from_code(5, false).unwrap(),
            Mdf3SampleFormat {
                kind: Mdf3SampleKind::VaxD,
                big_endian: false,
            }
        );
        assert_eq!(
            Mdf3SampleFormat::from_code(6, false).unwrap(),
            Mdf3SampleFormat {
                kind: Mdf3SampleKind::VaxG,
                big_endian: false,
            }
        );
    }

    #[test]
    fn an_unknown_code_is_refused_by_name() {
        for code in [99u16, 100] {
            assert!(matches!(
                Mdf3SampleFormat::from_code(code, false),
                Err(Mf4Error::Unsupported { .. })
            ));
        }
    }

    #[test]
    fn vax_f_decoding_hand_computed_values() {
        // 1.0 in VAX F: sign=0, exp=129 (0x81), fraction=0
        // Word 0 = 0x4080 (le: [0x80, 0x40]), Word 1 = 0x0000 (le: [0x00, 0x00])
        let bytes_1_0 = [0x80u8, 0x40, 0x00, 0x00];
        let vax_1_0 = decode_vax_f(bytes_1_0);
        assert_eq!(vax_1_0, 1.0);
        // Reading these bytes as little-endian IEEE 754 single precision gives a subnormal ~2.3e-41.
        let ieee_1_0 = f32::from_le_bytes(bytes_1_0);
        assert_ne!(vax_1_0, ieee_1_0 as f64);

        // 0.5 in VAX F: sign=0, exp=128 (0x80), fraction=0
        // Word 0 = 0x4000 (le: [0x00, 0x40]), Word 1 = 0x0000 (le: [0x00, 0x00])
        let bytes_0_5 = [0x00u8, 0x40, 0x00, 0x00];
        let vax_0_5 = decode_vax_f(bytes_0_5);
        assert_eq!(vax_0_5, 0.5);
        let ieee_0_5 = f32::from_le_bytes(bytes_0_5);
        assert_ne!(vax_0_5, ieee_0_5 as f64);

        // -2.5 in VAX F: sign=1, exp=130 (0x82), fraction=0x200000 (0.125 / 0.5 = 0.25 -> 2^-2 in mantissa)
        // Word 0 = 0xC120 (le: [0x20, 0xC1]), Word 1 = 0x0000 (le: [0x00, 0x00])
        let bytes_neg_2_5 = [0x20u8, 0xC1, 0x00, 0x00];
        let vax_neg_2_5 = decode_vax_f(bytes_neg_2_5);
        assert_eq!(vax_neg_2_5, -2.5);
        let ieee_neg_2_5 = f32::from_le_bytes(bytes_neg_2_5);
        assert_ne!(vax_neg_2_5, ieee_neg_2_5 as f64);

        // True zero: sign=0, exp=0
        let bytes_zero = [0x00u8, 0x00, 0x00, 0x00];
        assert_eq!(decode_vax_f(bytes_zero), 0.0);

        // Reserved operand: sign=1, exp=0 -> NaN
        let bytes_reserved = [0x00u8, 0x80, 0x00, 0x00];
        assert!(decode_vax_f(bytes_reserved).is_nan());

        // 1.5 in VAX F: sign=0, exp=129, fraction = 1 << 22 = 0x400000
        // Word 0 = 0x40C0 (le: [0xC0, 0x40]), Word 1 = 0x0000 (le: [0x00, 0x00])
        let bytes_1_5 = [0xC0u8, 0x40, 0x00, 0x00];
        assert_eq!(decode_vax_f(bytes_1_5), 1.5);
    }

    #[test]
    fn vax_d_decoding_hand_computed_values() {
        // 1.0 in VAX D: sign=0, exp=129 (0x81), fraction=0
        // Word 0 = 0x4080 (le: [0x80, 0x40]), Words 1..3 = 0
        let bytes_1_0 = [0x80u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_1_0 = decode_vax_d(bytes_1_0);
        assert_eq!(vax_1_0, 1.0);
        let ieee_1_0 = f64::from_le_bytes(bytes_1_0);
        assert_ne!(vax_1_0, ieee_1_0);

        // 0.5 in VAX D: sign=0, exp=128 (0x80), fraction=0
        // Word 0 = 0x4000 (le: [0x00, 0x40]), Words 1..3 = 0
        let bytes_0_5 = [0x00u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_0_5 = decode_vax_d(bytes_0_5);
        assert_eq!(vax_0_5, 0.5);
        let ieee_0_5 = f64::from_le_bytes(bytes_0_5);
        assert_ne!(vax_0_5, ieee_0_5);

        // -2.5 in VAX D: sign=1, exp=130 (0x82), fraction=0x0020_0000_0000_0000
        // Word 0 = 0xC120 (le: [0x20, 0xC1]), Words 1..3 = 0
        let bytes_neg_2_5 = [0x20u8, 0xC1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_neg_2_5 = decode_vax_d(bytes_neg_2_5);
        assert_eq!(vax_neg_2_5, -2.5);
        let ieee_neg_2_5 = f64::from_le_bytes(bytes_neg_2_5);
        assert_ne!(vax_neg_2_5, ieee_neg_2_5);

        // True zero: sign=0, exp=0
        let bytes_zero = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode_vax_d(bytes_zero), 0.0);

        // Reserved operand: sign=1, exp=0 -> NaN
        let bytes_reserved = [0x00u8, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(decode_vax_d(bytes_reserved).is_nan());

        // 1.5 in VAX D: sign=0, exp=129, fraction = 1 << 54 = 0x0040_0000_0000_0000
        // Word 0 = 0x40C0 (le: [0xC0, 0x40]), Words 1..3 = 0
        let bytes_1_5 = [0xC0u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode_vax_d(bytes_1_5), 1.5);
    }

    #[test]
    fn vax_g_decoding_hand_computed_values() {
        // 1.0 in VAX G: sign=0, exp=1025 (0x401), fraction=0
        // Word 0 = (0x401 << 4) = 0x4010 (le: [0x10, 0x40]), Words 1..3 = 0
        let bytes_1_0 = [0x10u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_1_0 = decode_vax_g(bytes_1_0);
        assert_eq!(vax_1_0, 1.0);
        let ieee_1_0 = f64::from_le_bytes(bytes_1_0);
        assert_ne!(vax_1_0, ieee_1_0);

        // 0.5 in VAX G: sign=0, exp=1024 (0x400), fraction=0
        // Word 0 = (0x400 << 4) = 0x4000 (le: [0x00, 0x40]), Words 1..3 = 0
        let bytes_0_5 = [0x00u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_0_5 = decode_vax_g(bytes_0_5);
        assert_eq!(vax_0_5, 0.5);
        let ieee_0_5 = f64::from_le_bytes(bytes_0_5);
        assert_ne!(vax_0_5, ieee_0_5);

        // -2.5 in VAX G: sign=1, exp=1026 (0x402), fraction=0x0004_0000_0000_0000 (4 in Word 0 bits 3..0)
        // Word 0 = (1 << 15) | (0x402 << 4) | 4 = 0xC024 (le: [0x24, 0xC0]), Words 1..3 = 0
        let bytes_neg_2_5 = [0x24u8, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let vax_neg_2_5 = decode_vax_g(bytes_neg_2_5);
        assert_eq!(vax_neg_2_5, -2.5);
        let ieee_neg_2_5 = f64::from_le_bytes(bytes_neg_2_5);
        assert_ne!(vax_neg_2_5, ieee_neg_2_5);

        // True zero: sign=0, exp=0
        let bytes_zero = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode_vax_g(bytes_zero), 0.0);

        // Reserved operand: sign=1, exp=0 -> NaN
        let bytes_reserved = [0x00u8, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(decode_vax_g(bytes_reserved).is_nan());

        // 1.5 in VAX G: sign=0, exp=1025, fraction = 1 << 51 = 0x0008_0000_0000_0000 (8 in Word 0 bits 3..0)
        // Word 0 = 0x4018 (le: [0x18, 0x40]), Words 1..3 = 0
        let bytes_1_5 = [0x18u8, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode_vax_g(bytes_1_5), 1.5);
    }

    #[test]
    fn vax_channel_decodes_into_signal_values_f64() {
        // Test Acc::push and into_values for VaxF
        let l_f = layout(0, 32, 4);
        let mut acc_f = Acc::for_layout(&l_f, 3);
        acc_f.push(&l_f, &[0x80, 0x40, 0x00, 0x00]); // 1.0
        acc_f.push(&l_f, &[0x00, 0x40, 0x00, 0x00]); // 0.5
        acc_f.push(&l_f, &[0x20, 0xC1, 0x00, 0x00]); // -2.5
        assert_eq!(
            acc_f.into_values(),
            SignalValues::F64(vec![1.0, 0.5, -2.5])
        );

        // Test Acc::push and into_values for VaxD
        let l_d = layout(0, 64, 5);
        let mut acc_d = Acc::for_layout(&l_d, 2);
        acc_d.push(&l_d, &[0x80, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // 1.0
        acc_d.push(&l_d, &[0xC0, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // 1.5
        assert_eq!(
            acc_d.into_values(),
            SignalValues::F64(vec![1.0, 1.5])
        );

        // Test Acc::push and into_values for VaxG
        let l_g = layout(0, 64, 6);
        let mut acc_g = Acc::for_layout(&l_g, 2);
        acc_g.push(&l_g, &[0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // 1.0
        acc_g.push(&l_g, &[0x24, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // -2.5
        assert_eq!(
            acc_g.into_values(),
            SignalValues::F64(vec![1.0, -2.5])
        );
    }
}
