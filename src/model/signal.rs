//! Signal abstraction for reading decoded channel data.
//!
//! The Signal type provides efficient access to channel samples,
//! supporting both eager and lazy decoding strategies.

use crate::blocks::{ChannelType, Conversion, ConversionOutput, DataType};
use crate::error::{Mf4Error, Result};
use crate::model::{Channel, SignalValues, ValueKind, VlsdPayloads};
use crate::parser::binary::{bytes_to_f64, read_int, read_uint};
use std::sync::Arc;

/// Byte offset of a record's invalidation area for sample `index`.
fn i_offset(layout: &RecordLayout, index: usize) -> usize {
    index * layout.record_size + layout.record_offset + layout.inval_start
}

/// Decodes one text sample, honouring the channel's declared encoding.
///
/// MF4 pads fixed-width text fields with NUL bytes, so trailing NULs are part of
/// the container rather than the value and are trimmed.
fn decode_string(bytes: &[u8], data_type: DataType) -> String {
    match data_type {
        DataType::StringUtf16Le | DataType::StringUtf16Be => {
            let big_endian = data_type == DataType::StringUtf16Be;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| {
                    if big_endian {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        }
    }
}

/// How records are laid out in a signal's raw buffer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordLayout {
    /// Stride from one record to the next.
    pub record_size: usize,
    /// Offset from a record's start to its payload, skipping any record ID.
    pub record_offset: usize,
    /// Offset from the payload start to the invalidation bytes, which the
    /// format places immediately after the channel data.
    pub inval_start: usize,
    /// Number of invalidation bytes per record; zero when the group has none.
    pub inval_bytes: usize,
}

/// Where a channel's field sits within a record, for strided reading.
#[derive(Debug, Clone, Copy)]
struct StridedField {
    /// Byte offset of the field from the start of the buffer.
    offset: usize,
    /// Bytes the field touches, including those its bit offset spills into.
    span: usize,
    /// Bits to shift right after reading, to bring the field to bit zero.
    bit_offset: u32,
    /// Width of the field in bits.
    bits: u32,
    /// Whether the field is whole bytes on a byte boundary, which needs no
    /// shifting or masking.
    aligned: bool,
}

impl StridedField {
    /// Reads this field from the start of one record.
    #[inline]
    fn read(&self, record: &[u8]) -> Option<u64> {
        let bytes = record.get(..self.span)?;

        // Assemble the touched bytes little-endian. `span` is at most 8, so the
        // shift never exceeds 56.
        let mut raw = 0u64;
        for (i, &b) in bytes.iter().enumerate() {
            raw |= (b as u64) << (i * 8);
        }

        if self.aligned {
            return Some(raw);
        }

        raw >>= self.bit_offset;
        // A 64-bit field masks to all ones; shifting by 64 would overflow.
        let mask = if self.bits >= 64 {
            u64::MAX
        } else {
            (1u64 << self.bits) - 1
        };
        Some(raw & mask)
    }
}

/// A signal view for accessing decoded channel samples.
///
/// This type provides efficient access to the physical values of a channel,
/// handling raw data extraction, conversion, and optional caching.
///
/// # Example
/// ```ignore
/// let signal = file.signal(&channel)?;
/// println!("Sample count: {}", signal.len());
///
/// // Get all values as f64
/// let values = signal.values_f64()?;
/// for (i, value) in values.iter().enumerate() {
///     println!("Sample {}: {}", i, value);
/// }
/// ```
pub struct Signal {
    /// Channel metadata.
    pub(crate) channel: Channel,
    /// Raw record data for all samples.
    ///
    /// Shared rather than owned: every channel in a group reads the same
    /// records, and a group's records can be tens of megabytes.
    pub(crate) raw_data: Arc<[u8]>,
    /// Record layout within `raw_data`.
    pub(crate) layout: RecordLayout,
    /// Number of samples.
    pub(crate) sample_count: usize,
    /// Payloads for a variable-length channel, absent for every other kind.
    pub(crate) payloads: Option<Arc<VlsdPayloads>>,
    /// Test-only switch forcing the general decode path.
    #[cfg(test)]
    pub(crate) force_general: bool,
}

impl Signal {
    /// Creates a new Signal from raw data.
    pub(crate) fn new(
        channel: Channel,
        raw_data: Arc<[u8]>,
        layout: RecordLayout,
        sample_count: usize,
    ) -> Self {
        Signal {
            channel,
            raw_data,
            layout,
            sample_count,
            payloads: None,
            #[cfg(test)]
            force_general: false,
        }
    }

    /// Supplies the payloads a variable-length channel refers to.
    pub(crate) fn attach_payloads(&mut self, payloads: Arc<VlsdPayloads>) {
        self.payloads = Some(payloads);
    }

    /// Returns which samples are valid, or `None` if the channel has no
    /// invalidation bit and every sample is therefore valid.
    ///
    /// `true` means the sample is valid. The file stores the opposite polarity —
    /// a set bit marks a sample *invalid* — which this method inverts so the
    /// result reads the way callers expect.
    ///
    /// Invalid samples are still present in [`Signal::values`]: they hold
    /// whatever bits the record contained, which is not a measurement. Check
    /// this before treating a channel's samples as data.
    pub fn validity(&self) -> Option<Vec<bool>> {
        if !self.channel.invalidation_bit || self.layout.inval_bytes == 0 {
            return None;
        }

        let byte = (self.channel.inval_bit_pos / 8) as usize;
        let bit = self.channel.inval_bit_pos % 8;
        if byte >= self.layout.inval_bytes {
            // The declared bit lies outside the invalidation area; treating that
            // as "all valid" would invent data, so report no validity info.
            return None;
        }

        let mut out = Vec::with_capacity(self.sample_count);
        for i in 0..self.sample_count {
            let at = i * self.layout.record_size
                + self.layout.record_offset
                + self.layout.inval_start
                + byte;
            match self.raw_data.get(at) {
                Some(b) => out.push((b >> bit) & 1 == 0),
                None => out.push(false),
            }
        }
        Some(out)
    }

    /// Returns whether one sample is valid.
    ///
    /// Samples of a channel without an invalidation bit are always valid.
    pub fn is_valid(&self, index: usize) -> bool {
        if !self.channel.invalidation_bit || self.layout.inval_bytes == 0 {
            return true;
        }
        let byte = (self.channel.inval_bit_pos / 8) as usize;
        let bit = self.channel.inval_bit_pos % 8;
        if byte >= self.layout.inval_bytes || index >= self.sample_count {
            return true;
        }
        let at = i_offset(&self.layout, index) + byte;
        match self.raw_data.get(at) {
            Some(b) => (b >> bit) & 1 == 0,
            None => false,
        }
    }

    /// Returns the number of valid samples.
    pub fn valid_count(&self) -> usize {
        match self.validity() {
            Some(v) => v.iter().filter(|ok| **ok).count(),
            None => self.sample_count,
        }
    }

    /// Returns the channel name.
    pub fn name(&self) -> &str {
        &self.channel.name
    }

    /// Returns the physical unit.
    pub fn unit(&self) -> &str {
        &self.channel.unit
    }

    /// Returns the number of samples.
    pub fn len(&self) -> usize {
        self.sample_count
    }

    /// Returns true if there are no samples.
    pub fn is_empty(&self) -> bool {
        self.sample_count == 0
    }

    /// Returns the channel metadata.
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    /// Reads a single raw value at the given sample index.
    fn read_raw_value(&self, index: usize) -> Result<f64> {
        if index >= self.sample_count {
            return Err(Mf4Error::parse_error(format!(
                "Sample index {} out of range (max: {})",
                index,
                self.sample_count - 1
            )));
        }

        let record_start = index * self.layout.record_size + self.layout.record_offset;
        let value_start = record_start + self.channel.byte_offset as usize;

        if value_start + self.channel.byte_size() > self.raw_data.len() {
            return Err(Mf4Error::truncated(
                value_start as u64,
                self.channel.byte_size(),
                self.raw_data.len() - value_start,
            ));
        }

        let raw = bytes_to_f64(
            &self.raw_data,
            value_start,
            self.channel.bit_offset,
            self.channel.bit_count,
            self.channel.is_signed(),
            self.channel.is_float(),
            self.channel.is_little_endian(),
        );

        Ok(raw)
    }

    /// Returns the physical value at the given sample index.
    ///
    /// This method reads the raw value and applies the channel conversion.
    pub fn value_at(&self, index: usize) -> Result<f64> {
        let raw = self.read_raw_value(index)?;
        Ok(self.channel.convert(raw))
    }

    /// Whether to skip the strided fast path.
    ///
    /// Always false outside tests; the differential test flips it so the two
    /// decode paths can be compared on the same input.
    #[cfg(not(test))]
    fn force_general_path(&self) -> bool {
        false
    }

    /// See the non-test version.
    #[cfg(test)]
    fn force_general_path(&self) -> bool {
        self.force_general
    }

    /// How a channel's field sits inside a record, when it can be read by
    /// striding rather than by general bit extraction.
    ///
    /// Requires the field to be little-endian and to fit within eight bytes
    /// once its bit offset is included, and every record to be present.
    fn strided_offset(&self) -> Option<StridedField> {
        // Big-endian fields need their bytes reversed, which the strided
        // reader does not do; they take the general path.
        if !self.channel.is_little_endian() {
            return None;
        }

        let bits = self.channel.bit_count;
        let bit_offset = self.channel.bit_offset as u32;
        if bits == 0 || bit_offset + bits > 64 {
            return None;
        }

        // Bytes the field touches, including the ones its bit offset pushes it
        // into.
        let span = (bit_offset + bits).div_ceil(8) as usize;
        let offset = self.layout.record_offset + self.channel.byte_offset as usize;

        // The last record must be complete, or the strided read would run off
        // the end; fall back rather than truncate silently.
        let end = self
            .sample_count
            .checked_sub(1)?
            .checked_mul(self.layout.record_size)?
            .checked_add(offset)?
            .checked_add(span)?;
        if self.sample_count > 0 && end > self.raw_data.len() {
            return None;
        }

        Some(StridedField {
            offset,
            span,
            bit_offset,
            bits,
            aligned: bit_offset == 0 && bits % 8 == 0,
        })
    }

    /// Reads every sample as a strided field.
    ///
    /// Two shapes are handled. A field occupying whole aligned bytes is a
    /// direct copy. A field at a bit offset, or one not a whole number of bytes
    /// — a two-bit bus number or a twenty-nine-bit identifier, which is most of
    /// a bus log — is read as the bytes it touches, then shifted and masked.
    /// Both keep the per-sample work to a fixed sequence over a strided buffer.
    fn decode_strided(&self, kind: ValueKind, field: StridedField) -> Option<SignalValues> {
        let stride = self.layout.record_size;
        let n = self.sample_count;
        if n == 0 || stride < field.span {
            return None;
        }
        let body = self.raw_data.get(field.offset..)?;
        let whole = n.checked_mul(stride).and_then(|len| body.get(..len));

        /// Applies `$f` to each record's field, read as a `u64`.
        macro_rules! map_records {
            ($f:expr) => {{
                let mut out = Vec::with_capacity(n);
                match whole {
                    Some(whole) => {
                        for record in whole.chunks_exact(stride) {
                            out.push($f(field.read(record)?));
                        }
                    }
                    // The final record is short of its full stride but its field
                    // is present; read those individually.
                    None => {
                        for i in 0..n {
                            let at = i * stride;
                            out.push($f(field.read(body.get(at..)?)?));
                        }
                    }
                }
                out
            }};
        }

        /// Sign-extends from the field's width, then narrows.
        macro_rules! signed {
            ($ty:ty) => {{
                let shift = 64 - field.bits;
                map_records!(|v: u64| (((v << shift) as i64) >> shift) as $ty)
            }};
        }

        Some(match kind {
            ValueKind::U8 => SignalValues::U8(map_records!(|v| v as u8)),
            ValueKind::U16 => SignalValues::U16(map_records!(|v| v as u16)),
            ValueKind::U32 => SignalValues::U32(map_records!(|v| v as u32)),
            ValueKind::U64 => SignalValues::U64(map_records!(|v| v)),
            ValueKind::I8 => SignalValues::I8(signed!(i8)),
            ValueKind::I16 => SignalValues::I16(signed!(i16)),
            ValueKind::I32 => SignalValues::I32(signed!(i32)),
            ValueKind::I64 => SignalValues::I64(signed!(i64)),
            ValueKind::F32 => {
                // A float's bits are only a float when whole and aligned.
                if !field.aligned || field.bits != 32 {
                    return None;
                }
                SignalValues::F32(map_records!(|v| f32::from_bits(v as u32)))
            }
            ValueKind::F64 => {
                if !field.aligned || field.bits != 64 || !self.channel.is_float() {
                    return None;
                }
                match &self.channel.conversion {
                    Conversion::None => SignalValues::F64(map_records!(f64::from_bits)),
                    // Linear is overwhelmingly the most common conversion;
                    // specialising it keeps the multiply-add inside the loop
                    // instead of dispatching on the conversion per sample.
                    Conversion::Linear {
                        offset: o,
                        factor: f,
                    } => {
                        let (o, f) = (*o, *f);
                        SignalValues::F64(map_records!(|v| f * f64::from_bits(v) + o))
                    }
                    _ => return None,
                }
            }
            ValueKind::Bytes | ValueKind::Str => return None,
        })
    }

    /// Returns all samples in the channel's own type.
    ///
    /// Integer channels stay integers at their natural width, byte-array and
    /// MIME channels stay bytes, and text stays text. Channels carrying a
    /// non-identity conversion decode to [`SignalValues::F64`], since a
    /// conversion produces physical values.
    ///
    /// # Example
    /// ```ignore
    /// match file.signal(&channel)?.values()? {
    ///     SignalValues::U32(ids) => println!("first id: {}", ids[0]),
    ///     SignalValues::Bytes { .. } => println!("opaque payload"),
    ///     other => println!("{} samples of {}", other.len(), other.kind().name()),
    /// }
    /// ```
    pub fn values(&self) -> Result<SignalValues> {
        // A channel this build cannot decode must fail rather than return the
        // part of it that happens to be readable.
        if let Some(reason) = self.channel.unreadable {
            return Err(Mf4Error::unsupported(
                "channel array (CA)",
                format!("channel '{}': {}", self.channel.name, reason.detail()),
            ));
        }

        // A variable-length channel stores an offset into a separate payload
        // stream; the record itself holds no value.
        if self.channel.channel_type == ChannelType::VariableLength {
            return self.variable_length_values();
        }

        // A conversion this build cannot evaluate makes every sample of the
        // channel meaningless. Fail rather than fall back to raw values, which
        // would look like plausible measurements.
        if let Conversion::Unsupported { kind, reason } = &self.channel.conversion {
            return Err(Mf4Error::unsupported(
                format!("conversion type {kind:?}"),
                format!("channel '{}': {reason}", self.channel.name),
            ));
        }

        let kind = self.channel.value_kind();
        let n = self.sample_count;

        // Fast path: a channel whose value is a whole number of bytes, aligned
        // to a byte boundary and stored little-endian, is a plain strided read.
        // That covers nearly every channel in practice, and lets the loop become
        // a chunked copy the compiler can vectorise, instead of per-sample bit
        // extraction with a bounds check each time.
        if !self.force_general_path() {
            if let Some(field) = self.strided_offset() {
                if let Some(values) = self.decode_strided(kind, field) {
                    return Ok(values);
                }
            }
        }

        // Text tables map each raw value to a label.
        if self.channel.conversion.output() == ConversionOutput::Text {
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let raw = self.read_raw_value(i)?;
                out.push(
                    self.channel
                        .conversion
                        .convert_text(raw)
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            return Ok(SignalValues::Str(out));
        }

        // Integer and float channels share one raw extraction; only the final
        // narrowing differs, so pull the raw words out once per kind.
        macro_rules! unsigned {
            ($variant:ident, $ty:ty) => {{
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(self.raw_uint(i)? as $ty);
                }
                Ok(SignalValues::$variant(out))
            }};
        }
        macro_rules! signed {
            ($variant:ident, $ty:ty) => {{
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(self.raw_int(i)? as $ty);
                }
                Ok(SignalValues::$variant(out))
            }};
        }

        match kind {
            ValueKind::U8 => unsigned!(U8, u8),
            ValueKind::U16 => unsigned!(U16, u16),
            ValueKind::U32 => unsigned!(U32, u32),
            ValueKind::U64 => unsigned!(U64, u64),
            ValueKind::I8 => signed!(I8, i8),
            ValueKind::I16 => signed!(I16, i16),
            ValueKind::I32 => signed!(I32, i32),
            ValueKind::I64 => signed!(I64, i64),
            ValueKind::F32 => {
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(self.read_raw_value(i)? as f32);
                }
                Ok(SignalValues::F32(out))
            }
            ValueKind::F64 => {
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(self.value_at(i)?);
                }
                Ok(SignalValues::F64(out))
            }
            ValueKind::Bytes => {
                // A fixed-width blob — a bus frame, a MIME sample — is a
                // strided copy. Writing into a buffer sized up front turns the
                // loop into one fixed-size move per record, rather than growing
                // a vector and bounds-checking each sample separately.
                let width = self.channel.byte_size();
                let start = self.layout.record_offset + self.channel.byte_offset as usize;
                let stride = self.layout.record_size;
                let mut data = vec![0u8; n.saturating_mul(width)];

                for (i, slot) in data.chunks_exact_mut(width).enumerate() {
                    let at = start + i * stride;
                    let Some(src) = self.raw_data.get(at..at + width) else {
                        return Err(Mf4Error::truncated(at as u64, width, self.raw_data.len()));
                    };
                    slot.copy_from_slice(src);
                }
                Ok(SignalValues::Bytes { data, width })
            }
            ValueKind::Str => {
                let width = self.channel.byte_size();
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(decode_string(
                        self.sample_bytes(i, width)?,
                        self.channel.data_type,
                    ));
                }
                Ok(SignalValues::Str(out))
            }
        }
    }

    /// Resolves each record's offset against the channel's payload stream.
    ///
    /// Payloads of a single size come back as [`SignalValues::Bytes`], which is
    /// the common case for bus logs and matches what other readers report;
    /// mixed sizes come back as [`SignalValues::VarBytes`].
    fn variable_length_values(&self) -> Result<SignalValues> {
        let Some(payloads) = &self.payloads else {
            return Err(Mf4Error::unsupported(
                "variable-length signal data (VLSD)",
                format!("channel '{}' has no payloads attached", self.channel.name),
            ));
        };

        let n = self.sample_count;
        let offsets = self.vlsd_offsets()?;

        // When every payload is the same size — which a bus log almost always
        // is — the output is a plain fixed-width buffer. Filling it directly
        // avoids both the per-sample offset table and growing the buffer a
        // payload at a time.
        if let Some(width) = payloads.uniform_len() {
            let mut data = vec![0u8; n.saturating_mul(width)];
            let mut hint = 0usize;
            let mut all_resolved = true;

            for (slot, &offset) in data.chunks_exact_mut(width).zip(offsets.iter()) {
                match payloads.get_from(offset, hint) {
                    Some((payload, at)) if payload.len() == width => {
                        slot.copy_from_slice(payload);
                        hint = at + 1;
                    }
                    _ => {
                        all_resolved = false;
                        break;
                    }
                }
            }

            // A fixed-width buffer cannot express "this sample has no payload":
            // leaving the slot zeroed would report bytes the file never
            // contained. If any offset failed to resolve, fall through to the
            // variable-width path, where a missing payload is genuinely empty.
            if all_resolved {
                return Ok(SignalValues::Bytes { data, width });
            }
        }

        // Mixed sizes: track where each sample begins. Size the output from the
        // payload stream, so growing it does not dominate; the exact total would
        // mean resolving every offset twice.
        let mut data = Vec::with_capacity(payloads.total_bytes());
        let mut starts = Vec::with_capacity(n + 1);

        // Records reference payloads in the order they were written, so carry
        // the last position forward as a hint rather than searching the whole
        // table for every sample.
        let mut hint = 0usize;
        for &offset in &offsets {
            starts.push(data.len() as u32);
            // A missing payload means the file is inconsistent, which is
            // represented as an empty sample rather than failing the channel.
            if let Some((payload, at)) = payloads.get_from(offset, hint) {
                data.extend_from_slice(payload);
                hint = at + 1;
            }
        }
        starts.push(data.len() as u32);

        // Uniform lengths collapse to a fixed width.
        let uniform = starts
            .windows(2)
            .map(|w| w[1] - w[0])
            .try_fold(None::<u32>, |acc, len| match acc {
                None => Some(Some(len)),
                Some(first) if first == len => Some(Some(first)),
                Some(_) => None,
            })
            .flatten();

        match uniform {
            Some(width) if n > 0 => Ok(SignalValues::Bytes {
                data,
                width: width as usize,
            }),
            _ => Ok(SignalValues::VarBytes { data, starts }),
        }
    }

    /// Reads every record's payload offset.
    ///
    /// The offsets are byte-aligned 64-bit fields, so they can be read by
    /// striding even though the channel's own data type is not a little-endian
    /// numeric one — see [`Signal::vlsd_offset`] for why the channel's
    /// endianness does not apply here.
    fn vlsd_offsets(&self) -> Result<Vec<u64>> {
        let n = self.sample_count;
        let bits = self.channel.bit_count;
        let bit_offset = self.channel.bit_offset as u32;

        if bit_offset == 0 && bits % 8 == 0 && bits <= 64 && n > 0 {
            let field = StridedField {
                offset: self.layout.record_offset + self.channel.byte_offset as usize,
                span: (bits / 8) as usize,
                bit_offset: 0,
                bits,
                aligned: true,
            };
            let stride = self.layout.record_size;
            if stride >= field.span {
                if let Some(body) = self.raw_data.get(field.offset..) {
                    if let Some(whole) = n.checked_mul(stride).and_then(|len| body.get(..len)) {
                        let mut out = Vec::with_capacity(n);
                        for record in whole.chunks_exact(stride) {
                            match field.read(record) {
                                Some(v) => out.push(v),
                                None => break,
                            }
                        }
                        if out.len() == n {
                            return Ok(out);
                        }
                    }
                }
            }
        }

        (0..n).map(|i| self.vlsd_offset(i)).collect()
    }

    /// Reads the payload offset a variable-length record carries.
    ///
    /// The offset is a plain little-endian integer, independent of the
    /// channel's declared data type: a variable-length channel's type describes
    /// its *payload* — `MimeSample` for a bus frame, say — which says nothing
    /// about the byte order of the offset pointing at it. Reading it through the
    /// channel's endianness gives a byte-reversed offset that resolves to
    /// nothing.
    fn vlsd_offset(&self, index: usize) -> Result<u64> {
        self.bounds_check(index)?;
        Ok(read_uint(
            &self.raw_data,
            self.value_offset(index),
            self.channel.bit_offset,
            self.channel.bit_count,
            true,
        ))
    }

    /// Returns the raw bytes of one sample.
    fn sample_bytes(&self, index: usize, width: usize) -> Result<&[u8]> {
        let start = self.value_offset(index);
        self.raw_data
            .get(start..start + width)
            .ok_or_else(|| Mf4Error::truncated(start as u64, width, self.raw_data.len()))
    }

    /// Byte offset of a sample's value within the raw record buffer.
    fn value_offset(&self, index: usize) -> usize {
        index * self.layout.record_size
            + self.layout.record_offset
            + self.channel.byte_offset as usize
    }

    /// Extracts one sample's raw bit field as an unsigned integer.
    fn raw_uint(&self, index: usize) -> Result<u64> {
        self.bounds_check(index)?;
        Ok(read_uint(
            &self.raw_data,
            self.value_offset(index),
            self.channel.bit_offset,
            self.channel.bit_count,
            self.channel.is_little_endian(),
        ))
    }

    /// Extracts one sample's raw bit field as a sign-extended integer.
    fn raw_int(&self, index: usize) -> Result<i64> {
        self.bounds_check(index)?;
        Ok(read_int(
            &self.raw_data,
            self.value_offset(index),
            self.channel.bit_offset,
            self.channel.bit_count,
            self.channel.is_little_endian(),
        ))
    }

    /// Fails if `index` is past the end, or if the sample's bytes are not present.
    fn bounds_check(&self, index: usize) -> Result<()> {
        if index >= self.sample_count {
            return Err(Mf4Error::parse_error(format!(
                "Sample index {} out of range (sample count: {})",
                index, self.sample_count
            )));
        }
        let start = self.value_offset(index);
        let end = start + self.channel.byte_size();
        if end > self.raw_data.len() {
            return Err(Mf4Error::truncated(
                start as u64,
                self.channel.byte_size(),
                self.raw_data.len().saturating_sub(start),
            ));
        }
        Ok(())
    }

    /// Returns all physical values as a vector of f64.
    ///
    /// A uniform numeric view over any channel. This is lossy where the channel
    /// is not naturally an `f64`: integers beyond 2^53 lose precision, and
    /// byte-array or text channels yield `NaN`, since they have no numeric
    /// meaning. Use [`Signal::values`] to get samples in their own type.
    pub fn values_f64(&self) -> Result<Vec<f64>> {
        match self.values()? {
            // Already the right type: hand it over instead of copying it.
            SignalValues::F64(v) => Ok(v),
            other => Ok(other.to_f64()),
        }
    }

    /// Returns an iterator over physical values.
    ///
    /// This is more memory-efficient than `values_f64()` for large signals
    /// as it decodes values on demand.
    pub fn iter(&self) -> SignalIterator<'_> {
        SignalIterator {
            signal: self,
            index: 0,
        }
    }

    /// Returns the minimum and maximum physical values.
    ///
    /// This scans all samples to find the actual min/max.
    pub fn min_max(&self) -> Result<(f64, f64)> {
        if self.sample_count == 0 {
            return Err(Mf4Error::parse_error(
                "Cannot compute min/max of empty signal",
            ));
        }

        let mut min = f64::MAX;
        let mut max = f64::MIN;

        for i in 0..self.sample_count {
            let value = self.value_at(i)?;
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }

        Ok((min, max))
    }

    /// Returns the mean (average) physical value.
    pub fn mean(&self) -> Result<f64> {
        if self.sample_count == 0 {
            return Err(Mf4Error::parse_error("Cannot compute mean of empty signal"));
        }

        let sum: f64 = self.iter().map(|r| r.unwrap_or(0.0)).sum();
        Ok(sum / self.sample_count as f64)
    }
}

/// Iterator over signal values.
pub struct SignalIterator<'a> {
    signal: &'a Signal,
    index: usize,
}

impl<'a> Iterator for SignalIterator<'a> {
    type Item = Result<f64>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.signal.sample_count {
            return None;
        }
        let value = self.signal.value_at(self.index);
        self.index += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.signal.sample_count - self.index;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for SignalIterator<'a> {}

/// A pair of time and value signals for time-series data.
pub struct TimeSeries {
    /// Time/master channel signal.
    pub time: Signal,
    /// Value channel signal.
    pub values: Signal,
}

impl TimeSeries {
    /// Creates a new time series from time and value signals.
    pub fn new(time: Signal, values: Signal) -> Result<Self> {
        if time.len() != values.len() {
            return Err(Mf4Error::parse_error(format!(
                "Time and value signal lengths don't match: {} vs {}",
                time.len(),
                values.len()
            )));
        }
        Ok(TimeSeries { time, values })
    }

    /// Returns the number of samples.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if there are no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns an iterator over (time, value) pairs.
    pub fn iter(&self) -> TimeSeriesIterator<'_> {
        TimeSeriesIterator {
            time_iter: self.time.iter(),
            value_iter: self.values.iter(),
        }
    }

    /// Returns all data as vectors of (timestamps, values).
    pub fn to_vectors(&self) -> Result<(Vec<f64>, Vec<f64>)> {
        let timestamps = self.time.values_f64()?;
        let values = self.values.values_f64()?;
        Ok((timestamps, values))
    }
}

/// Iterator over time series (time, value) pairs.
pub struct TimeSeriesIterator<'a> {
    time_iter: SignalIterator<'a>,
    value_iter: SignalIterator<'a>,
}

impl<'a> Iterator for TimeSeriesIterator<'a> {
    type Item = Result<(f64, f64)>;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.time_iter.next(), self.value_iter.next()) {
            (Some(Ok(t)), Some(Ok(v))) => Some(Ok((t, v))),
            (Some(Err(e)), _) | (_, Some(Err(e))) => Some(Err(e)),
            _ => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.value_iter.size_hint()
    }
}

impl<'a> ExactSizeIterator for TimeSeriesIterator<'a> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{ChannelType, Conversion, DataType, SyncType};

    /// A plain layout: fixed stride, no record ID, no invalidation bytes.
    fn plain(record_size: usize) -> RecordLayout {
        RecordLayout {
            record_size,
            record_offset: 0,
            inval_start: record_size,
            inval_bytes: 0,
        }
    }

    fn create_test_channel() -> Channel {
        Channel {
            id: 0,
            index: 0,
            channel_group_index: 0,
            data_group_index: 0,
            name: "TestChannel".to_string(),
            unit: "V".to_string(),
            channel_type: ChannelType::FixedLength,
            sync_type: SyncType::None,
            data_type: DataType::FloatLe,
            conversion: Conversion::Linear {
                offset: 0.0,
                factor: 1.0,
            },
            bit_count: 32,
            byte_offset: 0,
            bit_offset: 0,
            invalidation_bit: false,
            inval_bit_pos: 0,
            comment: String::new(),
            source: None,
            min_value: None,
            max_value: None,
            cn_offset: 0,
            data_link: 0,
            unreadable: None,
        }
    }

    #[test]
    fn test_signal_basic() {
        // Create raw data: 3 x f32 values [1.0, 2.0, 3.0]
        let mut raw_data = Vec::new();
        raw_data.extend_from_slice(&1.0f32.to_le_bytes());
        raw_data.extend_from_slice(&2.0f32.to_le_bytes());
        raw_data.extend_from_slice(&3.0f32.to_le_bytes());

        let channel = create_test_channel();
        let signal = Signal::new(channel, raw_data.into(), plain(4), 3);

        assert_eq!(signal.len(), 3);
        assert_eq!(signal.name(), "TestChannel");
        assert_eq!(signal.unit(), "V");
    }

    #[test]
    fn test_signal_values() {
        let mut raw_data = Vec::new();
        raw_data.extend_from_slice(&1.0f32.to_le_bytes());
        raw_data.extend_from_slice(&2.0f32.to_le_bytes());
        raw_data.extend_from_slice(&3.0f32.to_le_bytes());

        let channel = create_test_channel();
        let signal = Signal::new(channel, raw_data.into(), plain(4), 3);

        let values = signal.values_f64().unwrap();
        assert_eq!(values.len(), 3);
        assert!((values[0] - 1.0).abs() < 0.001);
        assert!((values[1] - 2.0).abs() < 0.001);
        assert!((values[2] - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_signal_with_conversion() {
        let mut raw_data = Vec::new();
        raw_data.extend_from_slice(&10.0f32.to_le_bytes());

        let mut channel = create_test_channel();
        channel.conversion = Conversion::Linear {
            offset: 5.0,
            factor: 2.0,
        };

        let signal = Signal::new(channel, raw_data.into(), plain(4), 1);
        let value = signal.value_at(0).unwrap();

        // 2.0 * 10.0 + 5.0 = 25.0
        assert!((value - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_signal_iterator() {
        let mut raw_data = Vec::new();
        for i in 0..5 {
            raw_data.extend_from_slice(&(i as f32).to_le_bytes());
        }

        let channel = create_test_channel();
        let signal = Signal::new(channel, raw_data.into(), plain(4), 5);

        let values: Vec<f64> = signal.iter().map(|r| r.unwrap()).collect();
        assert_eq!(values.len(), 5);
        for (i, &v) in values.iter().enumerate() {
            assert!((v - i as f64).abs() < 0.001);
        }
    }

    #[test]
    fn test_signal_min_max() {
        let mut raw_data = Vec::new();
        raw_data.extend_from_slice(&(-5.0f32).to_le_bytes());
        raw_data.extend_from_slice(&3.0f32.to_le_bytes());
        raw_data.extend_from_slice(&10.0f32.to_le_bytes());

        let channel = create_test_channel();
        let signal = Signal::new(channel, raw_data.into(), plain(4), 3);

        let (min, max) = signal.min_max().unwrap();
        assert!((min - (-5.0)).abs() < 0.001);
        assert!((max - 10.0).abs() < 0.001);
    }
    /// Builds a signal whose records are `[u8 value][inval byte]`, with the
    /// channel's invalidation bit at `bit_pos`.
    fn signal_with_invalidation(values: &[u8], inval: &[u8], bit_pos: u32) -> Signal {
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 8;
        ch.byte_offset = 0;
        ch.conversion = Conversion::None;
        ch.invalidation_bit = true;
        ch.inval_bit_pos = bit_pos;

        let mut raw = Vec::new();
        for (v, i) in values.iter().zip(inval) {
            raw.push(*v);
            raw.push(*i);
        }
        Signal::new(
            ch,
            raw.into(),
            RecordLayout {
                record_size: 2,
                record_offset: 0,
                inval_start: 1,
                inval_bytes: 1,
            },
            values.len(),
        )
    }

    #[test]
    fn a_set_invalidation_bit_marks_a_sample_invalid() {
        // Bit 0 set on the middle sample.
        let sig =
            signal_with_invalidation(&[10, 20, 30], &[0b0000_0000, 0b0000_0001, 0b0000_0000], 0);
        assert_eq!(sig.validity(), Some(vec![true, false, true]));
        assert!(sig.is_valid(0));
        assert!(!sig.is_valid(1));
        assert!(sig.is_valid(2));
        assert_eq!(sig.valid_count(), 2);
    }

    #[test]
    fn reads_the_invalidation_bit_at_its_declared_position() {
        // Bit 5 is this channel's; bit 0 belongs to some other channel and must
        // not be mistaken for it.
        let sig = signal_with_invalidation(&[1, 2], &[0b0000_0001, 0b0010_0000], 5);
        assert_eq!(sig.validity(), Some(vec![true, false]));
    }

    #[test]
    fn invalidation_bits_can_span_several_bytes() {
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 8;
        ch.conversion = Conversion::None;
        ch.invalidation_bit = true;
        ch.inval_bit_pos = 9; // second invalidation byte, bit 1

        // record = [value][inval0][inval1]
        let raw = vec![1, 0, 0b0000_0010, 2, 0, 0b0000_0000];
        let sig = Signal::new(
            ch,
            raw.into(),
            RecordLayout {
                record_size: 3,
                record_offset: 0,
                inval_start: 1,
                inval_bytes: 2,
            },
            2,
        );
        assert_eq!(sig.validity(), Some(vec![false, true]));
    }

    #[test]
    fn a_channel_without_an_invalidation_bit_reports_no_validity_info() {
        let sig = Signal::new(create_test_channel(), vec![0; 12].into(), plain(4), 3);
        assert_eq!(sig.validity(), None);
        assert!(sig.is_valid(0));
        assert_eq!(sig.valid_count(), 3, "all samples count as valid");
    }

    #[test]
    fn a_bit_position_outside_the_invalidation_area_is_not_treated_as_valid() {
        // Declares bit 64 but only one invalidation byte exists. Reporting
        // "all valid" would invent information, so no validity is reported.
        let sig = signal_with_invalidation(&[1, 2], &[0xFF, 0xFF], 64);
        assert_eq!(sig.validity(), None);
    }

    #[test]
    fn validity_accounts_for_the_record_id_offset() {
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 8;
        ch.conversion = Conversion::None;
        ch.invalidation_bit = true;
        ch.inval_bit_pos = 0;

        // record = [rec_id][value][inval]
        let raw = vec![7, 10, 0b0000_0001, 7, 20, 0b0000_0000];
        let sig = Signal::new(
            ch,
            raw.into(),
            RecordLayout {
                record_size: 3,
                record_offset: 1,
                inval_start: 1,
                inval_bytes: 1,
            },
            2,
        );
        assert_eq!(sig.validity(), Some(vec![false, true]));
    }

    #[test]
    fn invalid_samples_are_still_returned_by_values() {
        // Documented behaviour: values() does not filter. Callers combine it
        // with validity() themselves.
        let sig = signal_with_invalidation(&[10, 20], &[0, 1], 0);
        let v = sig.values().unwrap();
        assert_eq!(v.len(), 2, "invalid samples are present, not dropped");
        assert_eq!(v.to_f64(), vec![10.0, 20.0]);
    }
    /// Deterministic byte source, so a disagreement is reproducible.
    fn pseudo_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 33) as u8
            })
            .collect()
    }

    /// Builds the same signal twice, once decoded each way.
    #[allow(clippy::too_many_arguments)]
    fn both_paths(
        data_type: DataType,
        bit_count: u32,
        bit_offset: u8,
        byte_offset: u32,
        record_size: usize,
        conversion: Conversion,
        raw: Vec<u8>,
        samples: usize,
    ) -> (SignalValues, SignalValues) {
        let mut ch = create_test_channel();
        ch.data_type = data_type;
        ch.bit_count = bit_count;
        ch.bit_offset = bit_offset;
        ch.byte_offset = byte_offset;
        ch.conversion = conversion;

        let layout = RecordLayout {
            record_size,
            record_offset: 0,
            inval_start: record_size,
            inval_bytes: 0,
        };

        let fast = Signal::new(ch.clone(), raw.clone().into(), layout, samples);
        let mut slow = Signal::new(ch, raw.into(), layout, samples);
        slow.force_general = true;

        (fast.values().unwrap(), slow.values().unwrap())
    }

    #[test]
    fn the_fast_path_agrees_with_the_general_path() {
        // Every width and signedness the strided path claims to handle, over
        // pseudo-random bytes. A disagreement here means one of the two decoders
        // is wrong, which no amount of speed would make acceptable.
        let cases: &[(DataType, u32)] = &[
            (DataType::UIntLe, 8),
            (DataType::UIntLe, 16),
            (DataType::UIntLe, 32),
            (DataType::UIntLe, 64),
            (DataType::IntLe, 8),
            (DataType::IntLe, 16),
            (DataType::IntLe, 32),
            (DataType::IntLe, 64),
            (DataType::FloatLe, 32),
            (DataType::FloatLe, 64),
        ];

        for (seed, &(dt, bits)) in cases.iter().enumerate() {
            let width = (bits / 8) as usize;
            let record_size = width + 3; // deliberately not tightly packed
            let samples = 64;
            let raw = pseudo_bytes(record_size * samples, seed as u64 + 1);

            let (fast, slow) =
                both_paths(dt, bits, 0, 0, record_size, Conversion::None, raw, samples);
            assert_eq!(fast, slow, "paths disagree for {dt:?} at {bits} bits");
        }
    }

    #[test]
    fn the_fast_path_agrees_with_the_general_path_for_packed_bitfields() {
        // The widths and offsets a CAN frame actually uses: a 2-bit bus number,
        // a 29-bit identifier at bit 2, single flag bits, a 4-bit length.
        let cases: &[(DataType, u32, u8)] = &[
            (DataType::UIntLe, 1, 0),
            (DataType::UIntLe, 1, 7),
            (DataType::UIntLe, 2, 0),
            (DataType::UIntLe, 4, 2),
            (DataType::UIntLe, 7, 1),
            (DataType::UIntLe, 29, 2),
            (DataType::UIntLe, 12, 4),
            (DataType::UIntLe, 33, 3),
            (DataType::IntLe, 5, 1),
            (DataType::IntLe, 12, 3),
            (DataType::IntLe, 20, 6),
        ];

        for (seed, &(dt, bits, bit_offset)) in cases.iter().enumerate() {
            let record_size = 16;
            let samples = 64;
            let raw = pseudo_bytes(record_size * samples, seed as u64 + 500);

            let (fast, slow) = both_paths(
                dt,
                bits,
                bit_offset,
                0,
                record_size,
                Conversion::None,
                raw,
                samples,
            );
            assert_eq!(
                fast, slow,
                "paths disagree for {dt:?} at {bits} bits, offset {bit_offset}"
            );
        }
    }

    #[test]
    fn the_fast_path_agrees_when_the_field_is_offset_within_the_record() {
        let record_size = 16;
        let samples = 32;
        let raw = pseudo_bytes(record_size * samples, 99);
        for byte_offset in [0u32, 1, 3, 8] {
            let (fast, slow) = both_paths(
                DataType::UIntLe,
                32,
                0,
                byte_offset,
                record_size,
                Conversion::None,
                raw.clone(),
                samples,
            );
            assert_eq!(fast, slow, "paths disagree at byte offset {byte_offset}");
        }
    }

    #[test]
    fn the_fast_path_agrees_with_a_linear_conversion() {
        let record_size = 8;
        let samples = 48;
        let raw = pseudo_bytes(record_size * samples, 7);
        let (fast, slow) = both_paths(
            DataType::FloatLe,
            64,
            0,
            0,
            record_size,
            Conversion::Linear {
                offset: -3.5,
                factor: 1e-9,
            },
            raw,
            samples,
        );
        assert_eq!(fast, slow, "linear conversion differs between paths");
    }

    #[test]
    fn packed_bitfields_are_read_strided() {
        // A 29-bit identifier starting at bit 2 is the shape most of a bus log
        // takes, so it must not fall back to per-sample bit extraction.
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 29;
        ch.bit_offset = 2;
        ch.conversion = Conversion::None;
        let sig = Signal::new(ch, vec![0xFFu8; 64].into(), plain(8), 4);
        assert!(sig.strided_offset().is_some());
    }

    #[test]
    fn fields_the_strided_reader_cannot_handle_use_the_general_path() {
        // Big-endian needs its bytes reversed, which the strided reader does
        // not do.
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntBe;
        ch.bit_count = 32;
        ch.conversion = Conversion::None;
        let sig = Signal::new(ch, vec![0u8; 64].into(), plain(8), 4);
        assert!(sig.strided_offset().is_none(), "big-endian is not strided");

        // A field whose bit offset pushes it past eight bytes cannot be
        // assembled into a u64.
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 64;
        ch.bit_offset = 4;
        ch.conversion = Conversion::None;
        let sig = Signal::new(ch, vec![0u8; 128].into(), plain(16), 4);
        assert!(
            sig.strided_offset().is_none(),
            "68 bits do not fit in a u64"
        );
    }

    #[test]
    fn a_truncated_final_record_falls_back_rather_than_reading_past_the_end() {
        let mut ch = create_test_channel();
        ch.data_type = DataType::UIntLe;
        ch.bit_count = 32;
        ch.conversion = Conversion::None;
        // 4 samples at stride 8 read up to byte 28, so 30 bytes would in fact
        // be enough; 26 is genuinely short of the final record.
        let sig = Signal::new(ch.clone(), vec![0u8; 26].into(), plain(8), 4);
        assert!(
            sig.strided_offset().is_none(),
            "a buffer too short for the last record must not be read strided"
        );

        // The boundary case: exactly enough for the last field.
        let sig = Signal::new(ch, vec![0u8; 28].into(), plain(8), 4);
        assert!(
            sig.strided_offset().is_some(),
            "a buffer ending exactly at the last field is fine"
        );
    }
    /// A variable-length channel: an 8-byte offset per record, with payloads
    /// supplied separately.
    fn vlsd_signal(offsets: &[u64], payload_stream: &[u8]) -> Signal {
        let mut ch = create_test_channel();
        ch.channel_type = ChannelType::VariableLength;
        ch.data_type = DataType::MimeSample;
        ch.bit_count = 64;
        ch.byte_offset = 0;
        ch.conversion = Conversion::None;

        let mut raw = Vec::new();
        for o in offsets {
            raw.extend_from_slice(&o.to_le_bytes());
        }

        let mut sig = Signal::new(ch, raw.into(), plain(8), offsets.len());
        sig.attach_payloads(Arc::new(VlsdPayloads::from_stream(payload_stream)));
        sig
    }

    fn payload_stream(payloads: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in payloads {
            out.extend_from_slice(&(p.len() as u32).to_le_bytes());
            out.extend_from_slice(p);
        }
        out
    }

    #[test]
    fn variable_length_payloads_of_one_size_become_fixed_width_bytes() {
        let stream = payload_stream(&[&[1, 2, 3, 4], &[5, 6, 7, 8]]);
        let sig = vlsd_signal(&[0, 8], &stream);

        match sig.values().unwrap() {
            SignalValues::Bytes { data, width } => {
                assert_eq!(width, 4);
                assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
            }
            other => panic!("expected fixed-width Bytes, got {}", other.kind().name()),
        }
    }

    #[test]
    fn variable_length_payloads_of_mixed_sizes_stay_variable() {
        // Padding the short one out would invent bytes the file does not have.
        let stream = payload_stream(&[&[1, 2, 3], &[4, 5, 6, 7, 8]]);
        let sig = vlsd_signal(&[0, 7], &stream);

        let values = sig.values().unwrap();
        assert!(matches!(values, SignalValues::VarBytes { .. }));
        assert_eq!(values.len(), 2);
        assert_eq!(values.bytes_at(0), Some(&[1, 2, 3][..]));
        assert_eq!(values.bytes_at(1), Some(&[4, 5, 6, 7, 8][..]));
    }

    #[test]
    fn the_payload_offset_is_read_little_endian_whatever_the_channel_type_says() {
        // MimeSample is not a little-endian type, but the offset pointing at the
        // payload is still stored little-endian. Reading it through the
        // channel's endianness yields a byte-reversed offset that resolves to
        // nothing — which is exactly what happened before this was fixed.
        let stream = payload_stream(&[&[0xAA], &[0xBB]]);
        let sig = vlsd_signal(&[0, 5], &stream);

        let values = sig.values().unwrap();
        assert_eq!(values.bytes_at(0), Some(&[0xAA][..]));
        assert_eq!(values.bytes_at(1), Some(&[0xBB][..]));
    }

    #[test]
    fn an_offset_pointing_nowhere_yields_an_empty_sample() {
        let stream = payload_stream(&[&[1, 2]]);
        let sig = vlsd_signal(&[0, 9999], &stream);

        let values = sig.values().unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values.bytes_at(0), Some(&[1, 2][..]));
        assert_eq!(
            values.bytes_at(1),
            Some(&[][..]),
            "an unresolvable offset must not drop the sample or shift the rest"
        );
    }

    #[test]
    fn a_variable_length_channel_without_payloads_fails_rather_than_guessing() {
        let mut ch = create_test_channel();
        ch.channel_type = ChannelType::VariableLength;
        ch.bit_count = 64;
        let sig = Signal::new(ch, vec![0u8; 16].into(), plain(8), 2);
        assert!(sig.values().is_err());
    }
}
