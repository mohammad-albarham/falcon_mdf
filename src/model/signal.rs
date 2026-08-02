//! Signal abstraction for reading decoded channel data.
//!
//! The Signal type provides efficient access to channel samples,
//! supporting both eager and lazy decoding strategies.

use crate::blocks::{ChannelType, Conversion, ConversionOutput, DataType};
use crate::error::{Mf4Error, Result};
use crate::model::{Channel, SignalValues, ValueKind};
use crate::parser::binary::{bytes_to_f64, read_int, read_uint};

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
    pub(crate) raw_data: Vec<u8>,
    /// Record layout within `raw_data`.
    pub(crate) layout: RecordLayout,
    /// Number of samples.
    pub(crate) sample_count: usize,
}

impl Signal {
    /// Creates a new Signal from raw data.
    pub(crate) fn new(
        channel: Channel,
        raw_data: Vec<u8>,
        layout: RecordLayout,
        sample_count: usize,
    ) -> Self {
        Signal {
            channel,
            raw_data,
            layout,
            sample_count,
        }
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
        // A variable-length channel stores an offset into a signal-data block
        // where the real payload lives; the record itself holds no value.
        // Decoding that offset as if it were the value yields plausible-looking
        // nonsense, so refuse until VLSD reads land (plan Phase 4).
        if self.channel.channel_type == ChannelType::VariableLength {
            return Err(Mf4Error::unsupported(
                "variable-length signal data (VLSD)",
                format!(
                    "channel '{}' stores its payload in a signal-data block",
                    self.channel.name
                ),
            ));
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
                let width = self.channel.byte_size();
                let mut data = Vec::with_capacity(n * width);
                for i in 0..n {
                    data.extend_from_slice(self.sample_bytes(i, width)?);
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
        Ok(self.values()?.to_f64())
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
        let signal = Signal::new(channel, raw_data, plain(4), 3);

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
        let signal = Signal::new(channel, raw_data, plain(4), 3);

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

        let signal = Signal::new(channel, raw_data, plain(4), 1);
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
        let signal = Signal::new(channel, raw_data, plain(4), 5);

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
        let signal = Signal::new(channel, raw_data, plain(4), 3);

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
            raw,
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
            raw,
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
        let sig = Signal::new(create_test_channel(), vec![0; 12], plain(4), 3);
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
            raw,
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
}
