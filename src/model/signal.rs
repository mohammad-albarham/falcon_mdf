//! Signal abstraction for reading decoded channel data.
//!
//! The Signal type provides efficient access to channel samples,
//! supporting both eager and lazy decoding strategies.

use crate::error::{Mf4Error, Result};
use crate::model::Channel;
use crate::parser::binary::bytes_to_f64;

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
    /// Record size in bytes.
    pub(crate) record_size: usize,
    /// Byte offset to start of records (after record ID if present).
    pub(crate) record_offset: usize,
    /// Number of samples.
    pub(crate) sample_count: usize,
    /// Cached decoded values (lazily populated).
    pub(crate) cached_values: Option<Vec<f64>>,
}

impl Signal {
    /// Creates a new Signal from raw data.
    pub(crate) fn new(
        channel: Channel,
        raw_data: Vec<u8>,
        record_size: usize,
        record_offset: usize,
        sample_count: usize,
    ) -> Self {
        Signal {
            channel,
            raw_data,
            record_size,
            record_offset,
            sample_count,
            cached_values: None,
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

        let record_start = index * self.record_size + self.record_offset;
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

    /// Returns all physical values as a vector of f64.
    ///
    /// This method decodes all samples and applies the channel conversion.
    /// For large signals, consider using the iterator instead.
    pub fn values_f64(&self) -> Result<Vec<f64>> {
        let mut values = Vec::with_capacity(self.sample_count);
        for i in 0..self.sample_count {
            values.push(self.value_at(i)?);
        }
        Ok(values)
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
            return Err(Mf4Error::parse_error("Cannot compute min/max of empty signal"));
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
    use crate::blocks::{ChannelType, DataType, SyncType, Conversion};

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
        let signal = Signal::new(channel, raw_data, 4, 0, 3);

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
        let signal = Signal::new(channel, raw_data, 4, 0, 3);

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

        let signal = Signal::new(channel, raw_data, 4, 0, 1);
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
        let signal = Signal::new(channel, raw_data, 4, 0, 5);

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
        let signal = Signal::new(channel, raw_data, 4, 0, 3);

        let (min, max) = signal.min_max().unwrap();
        assert!((min - (-5.0)).abs() < 0.001);
        assert!((max - 10.0).abs() < 0.001);
    }
}
