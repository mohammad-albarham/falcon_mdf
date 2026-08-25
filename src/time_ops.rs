//! Time-domain operations on measurement signals: slicing (`cut`) and
//! re-gridding (`resample`).

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, SignalValues};

/// Interpolation mode for resampling time-series data onto a new raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolationMode {
    /// Carries the last known value forward (zero-order hold).
    ///
    /// For non-numeric channels (text, byte arrays), step-hold is used
    /// regardless of the requested mode.
    #[default]
    StepHold,
    /// Linearly interpolates between adjacent samples for numeric channels.
    ///
    /// Non-numeric channels (text, byte arrays, complex) fall back to step-hold.
    Linear,
}

/// The target time grid for resampling operations.
#[derive(Debug, Clone, PartialEq)]
pub enum Raster {
    /// A fixed positive time step interval (in seconds), e.g. `Raster::Step(0.01)` for 100 Hz.
    ///
    /// Generates a uniform time raster from the minimum timestamp to the maximum timestamp
    /// with the given step size.
    Step(f64),
    /// An explicit vector of target timestamps.
    Timestamps(Vec<f64>),
}

impl From<f64> for Raster {
    fn from(step: f64) -> Self {
        Raster::Step(step)
    }
}

impl From<Vec<f64>> for Raster {
    fn from(timestamps: Vec<f64>) -> Self {
        Raster::Timestamps(timestamps)
    }
}

impl From<&[f64]> for Raster {
    fn from(timestamps: &[f64]) -> Self {
        Raster::Timestamps(timestamps.to_vec())
    }
}

/// A decoded in-memory time series representing a channel's samples on a time axis.
///
/// Holds the channel metadata alongside its decoded timestamps, typed sample values,
/// and optional per-sample invalidation mask.
#[derive(Debug, Clone)]
pub struct SignalSeries {
    /// Channel descriptor and metadata.
    pub channel: Channel,
    /// Timestamps for each sample in the series (in seconds / time master units).
    pub timestamps: Vec<f64>,
    /// Decoded sample values in their native typed representation.
    pub values: SignalValues,
    /// Per-sample validity flags (true = valid, false = invalid), or None if no
    /// invalidation bits are present in the channel group.
    pub validity: Option<Vec<bool>>,
}

impl PartialEq for SignalSeries {
    fn eq(&self, other: &Self) -> bool {
        self.channel.name == other.channel.name
            && self.channel.unit == other.channel.unit
            && self.timestamps == other.timestamps
            && self.values == other.values
            && self.validity == other.validity
    }
}

impl SignalSeries {
    /// Creates a new `SignalSeries` from components, verifying length consistency.
    pub fn new(
        channel: Channel,
        timestamps: Vec<f64>,
        values: SignalValues,
        validity: Option<Vec<bool>>,
    ) -> Result<Self> {
        // Checked unconditionally: an empty timestamp vector beside non-empty
        // values used to pass here, and the mismatch surfaced later as a panic
        // in the slicer rather than as this error.
        if timestamps.len() != values.len() {
            return Err(Mf4Error::parse_error(format!(
                "timestamps length ({}) does not match values length ({}) for channel '{}'",
                timestamps.len(),
                values.len(),
                channel.name
            )));
        }
        if let Some(v) = &validity {
            if v.len() != values.len() {
                return Err(Mf4Error::parse_error(format!(
                    "validity length ({}) does not match values length ({}) for channel '{}'",
                    v.len(),
                    values.len(),
                    channel.name
                )));
            }
        }
        Ok(Self {
            channel,
            timestamps,
            values,
            validity,
        })
    }

    /// Creates a synthetic in-memory `SignalSeries` with the given channel name, unit,
    /// timestamps, typed values, and optional validity mask.
    pub fn from_samples(
        name: impl Into<String>,
        unit: impl Into<String>,
        timestamps: Vec<f64>,
        values: SignalValues,
        validity: Option<Vec<bool>>,
    ) -> Result<Self> {
        let channel = Channel::synthetic(name, unit);
        Self::new(channel, timestamps, values, validity)
    }

    /// Number of samples in the time series.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if the time series contains no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Name of the channel.
    pub fn name(&self) -> &str {
        &self.channel.name
    }

    /// Engineering unit of the channel.
    pub fn unit(&self) -> &str {
        &self.channel.unit
    }

    /// Timestamps slice.
    pub fn timestamps(&self) -> &[f64] {
        &self.timestamps
    }

    /// Decoded sample values.
    pub fn values(&self) -> &SignalValues {
        &self.values
    }

    /// Per-sample validity flags, or None if the channel has no invalidation bits.
    pub fn validity(&self) -> Option<&[bool]> {
        self.validity.as_deref()
    }

    /// Converts all sample values to a uniform `f64` representation.
    pub fn values_f64(&self) -> Vec<f64> {
        self.values.to_f64()
    }

    /// Slices the time series to include only samples whose timestamps fall within `[start, end]`.
    ///
    /// Uses a closed interval (`start <= t <= end`). If `start > end` or no samples fall
    /// within the range, returns an empty series.
    pub fn cut(&self, start: f64, end: f64) -> Self {
        if start > end || self.timestamps.is_empty() {
            return Self {
                channel: self.channel.clone(),
                timestamps: Vec::new(),
                values: slice_values(&self.values, 0..0),
                validity: self.validity.as_ref().map(|_| Vec::new()),
            };
        }

        let start_idx = self.timestamps.partition_point(|&t| t < start);
        let end_idx = self.timestamps.partition_point(|&t| t <= end);

        let cut_timestamps = self.timestamps[start_idx..end_idx].to_vec();
        let cut_values = slice_values(&self.values, start_idx..end_idx);
        let cut_validity = self.validity.as_ref().map(|v| v[start_idx..end_idx].to_vec());

        Self {
            channel: self.channel.clone(),
            timestamps: cut_timestamps,
            values: cut_values,
            validity: cut_validity,
        }
    }

    /// Resamples the time series onto a target raster.
    ///
    /// Supports `Raster::Step(dt)` (uniform step from start to end) and
    /// `Raster::Timestamps(vec)` (explicit timestamp sequence).
    ///
    /// In `InterpolationMode::StepHold`, each target timestamp holds the latest preceding sample.
    /// In `InterpolationMode::Linear`, numeric channels interpolate linearly between adjacent samples,
    /// while non-numeric channels (text, bytes) fall back to step-hold.
    pub fn resample(&self, raster: impl Into<Raster>, mode: InterpolationMode) -> Result<Self> {
        let raster = raster.into();
        let target_timestamps = match raster {
            Raster::Step(dt) => {
                if dt <= 0.0 || !dt.is_finite() {
                    return Err(Mf4Error::parse_error(format!(
                        "resample raster step must be positive and finite, got {dt}"
                    )));
                }
                if self.timestamps.is_empty() {
                    Vec::new()
                } else {
                    let t_min = self.timestamps[0];
                    let t_max = *self.timestamps.last().unwrap();
                    generate_raster_grid(t_min, t_max, dt)
                }
            }
            Raster::Timestamps(ts) => ts,
        };

        let resampled_values = resample_values(&self.values, &self.timestamps, &target_timestamps, mode);
        let resampled_validity = resample_validity(self.validity.as_deref(), &self.timestamps, &target_timestamps);

        Self::new(self.channel.clone(), target_timestamps, resampled_values, resampled_validity)
    }
}

/// Generates a uniform time raster from `t_min` to `t_max` with step `dt`.
pub(crate) fn generate_raster_grid(t_min: f64, t_max: f64, dt: f64) -> Vec<f64> {
    if t_min > t_max {
        return Vec::new();
    }
    if (t_max - t_min).abs() < 1e-15 {
        return vec![t_min];
    }
    // `t_min + i * dt` rather than repeated addition: accumulating `dt` across
    // a long recording drifts by more than the step itself, and a drifting
    // raster silently mistimes every sample placed on it.
    let eps = dt * 1e-6;
    let steps = ((t_max - t_min) / dt + eps).floor() as usize;
    (0..=steps).map(|i| t_min + i as f64 * dt).collect()
}

/// Helper to slice `SignalValues`.
pub(crate) fn slice_values(values: &SignalValues, range: std::ops::Range<usize>) -> SignalValues {
    match values {
        SignalValues::U8(v) => SignalValues::U8(v[range].to_vec()),
        SignalValues::U16(v) => SignalValues::U16(v[range].to_vec()),
        SignalValues::U32(v) => SignalValues::U32(v[range].to_vec()),
        SignalValues::U64(v) => SignalValues::U64(v[range].to_vec()),
        SignalValues::I8(v) => SignalValues::I8(v[range].to_vec()),
        SignalValues::I16(v) => SignalValues::I16(v[range].to_vec()),
        SignalValues::I32(v) => SignalValues::I32(v[range].to_vec()),
        SignalValues::I64(v) => SignalValues::I64(v[range].to_vec()),
        SignalValues::F32(v) => SignalValues::F32(v[range].to_vec()),
        SignalValues::F64(v) => SignalValues::F64(v[range].to_vec()),
        SignalValues::Str(v) => SignalValues::Str(v[range].to_vec()),
        SignalValues::Bytes { data, width } => {
            let start = range.start * *width;
            let end = range.end * *width;
            SignalValues::Bytes {
                data: data[start..end].to_vec(),
                width: *width,
            }
        }
        SignalValues::VarBytes { data, starts } => {
            if range.start >= range.end {
                return SignalValues::VarBytes {
                    data: Vec::new(),
                    starts: vec![0],
                };
            }
            let byte_start = starts[range.start];
            let byte_end = starts[range.end];
            let new_starts: Vec<usize> = starts[range.start..=range.end]
                .iter()
                .map(|&s| s - byte_start)
                .collect();
            SignalValues::VarBytes {
                data: data[byte_start..byte_end].to_vec(),
                starts: new_starts,
            }
        }
        SignalValues::Complex { re, im } => SignalValues::Complex {
            re: re[range.clone()].to_vec(),
            im: im[range].to_vec(),
        },
        SignalValues::CanopenDate(v) => SignalValues::CanopenDate(v[range].to_vec()),
        SignalValues::CanopenTime(v) => SignalValues::CanopenTime(v[range].to_vec()),
        SignalValues::Array {
            values: v,
            elements_per_sample,
        } => {
            let start = range.start * *elements_per_sample;
            let end = range.end * *elements_per_sample;
            SignalValues::Array {
                values: v[start..end].to_vec(),
                elements_per_sample: *elements_per_sample,
            }
        }
        SignalValues::ArrayVarLen { values: v, starts } => {
            if range.start >= range.end {
                return SignalValues::ArrayVarLen {
                    values: Vec::new(),
                    starts: vec![0],
                };
            }
            let val_start = starts[range.start];
            let val_end = starts[range.end];
            let new_starts: Vec<usize> = starts[range.start..=range.end]
                .iter()
                .map(|&s| s - val_start)
                .collect();
            SignalValues::ArrayVarLen {
                values: v[val_start..val_end].to_vec(),
                starts: new_starts,
            }
        }
    }
}

/// Finds the index in `src_t` for step-hold extrapolation/interpolation at time `t`.
fn step_hold_index(src_t: &[f64], t: f64) -> usize {
    if src_t.is_empty() {
        return 0;
    }
    let idx = src_t.partition_point(|&st| st <= t);
    if idx == 0 {
        0
    } else {
        idx - 1
    }
}

struct LinearWeights {
    i0: usize,
    i1: usize,
    alpha: f64,
}

fn linear_weights(src_t: &[f64], t: f64) -> LinearWeights {
    let n = src_t.len();
    if n == 0 {
        return LinearWeights {
            i0: 0,
            i1: 0,
            alpha: 0.0,
        };
    }
    if n == 1 || t <= src_t[0] {
        return LinearWeights {
            i0: 0,
            i1: 0,
            alpha: 0.0,
        };
    }
    if t >= src_t[n - 1] {
        return LinearWeights {
            i0: n - 1,
            i1: n - 1,
            alpha: 0.0,
        };
    }
    let idx = src_t.partition_point(|&st| st <= t);
    let i0 = idx - 1;
    let i1 = idx;
    let dt = src_t[i1] - src_t[i0];
    let alpha = if dt.abs() < 1e-15 {
        0.0
    } else {
        (t - src_t[i0]) / dt
    };
    LinearWeights { i0, i1, alpha }
}

/// Resamples `SignalValues` onto `target_timestamps`.
pub(crate) fn resample_values(
    values: &SignalValues,
    src_t: &[f64],
    target_t: &[f64],
    mode: InterpolationMode,
) -> SignalValues {
    let m = target_t.len();
    if src_t.is_empty() || m == 0 {
        return slice_values(values, 0..0);
    }

    match mode {
        InterpolationMode::StepHold => resample_values_step_hold(values, src_t, target_t),
        InterpolationMode::Linear => {
            match values {
                SignalValues::F64(v) => {
                    let mut out = Vec::with_capacity(m);
                    for &t in target_t {
                        let w = linear_weights(src_t, t);
                        let val = v[w.i0] * (1.0 - w.alpha) + v[w.i1] * w.alpha;
                        out.push(val);
                    }
                    SignalValues::F64(out)
                }
                SignalValues::F32(v) => {
                    let mut out = Vec::with_capacity(m);
                    for &t in target_t {
                        let w = linear_weights(src_t, t);
                        let val = (v[w.i0] as f64 * (1.0 - w.alpha) + v[w.i1] as f64 * w.alpha) as f32;
                        out.push(val);
                    }
                    SignalValues::F32(out)
                }
                // Only the float channels interpolate. An integer channel is
                // discrete — a gear, a counter, a state word — and a value
                // halfway between two of its samples was never measured and
                // frequently cannot exist. asammdf takes the same position by
                // default (`integer_interpolation_mode=0`, repeat the previous
                // sample), so a resampled integer channel compares equal
                // between the two tools; `tests/time_ops.rs` pins that against
                // asammdf rather than against this code.
                //
                // Text, byte, complex, CANopen and array channels reach the
                // same arm, for the stronger reason that there is no
                // arithmetic to interpolate them with.
                _ => resample_values_step_hold(values, src_t, target_t),
            }
        }
    }
}

fn resample_values_step_hold(
    values: &SignalValues,
    src_t: &[f64],
    target_t: &[f64],
) -> SignalValues {
    let m = target_t.len();
    let indices: Vec<usize> = target_t.iter().map(|&t| step_hold_index(src_t, t)).collect();

    match values {
        SignalValues::U8(v) => SignalValues::U8(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::U16(v) => SignalValues::U16(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::U32(v) => SignalValues::U32(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::U64(v) => SignalValues::U64(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::I8(v) => SignalValues::I8(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::I16(v) => SignalValues::I16(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::I32(v) => SignalValues::I32(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::I64(v) => SignalValues::I64(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::F32(v) => SignalValues::F32(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::F64(v) => SignalValues::F64(indices.iter().map(|&k| v[k]).collect()),
        SignalValues::Str(v) => SignalValues::Str(indices.iter().map(|&k| v[k].clone()).collect()),
        SignalValues::Bytes { data, width } => {
            let mut out = Vec::with_capacity(m * *width);
            for &k in &indices {
                let start = k * *width;
                let end = start + *width;
                out.extend_from_slice(&data[start..end]);
            }
            SignalValues::Bytes {
                data: out,
                width: *width,
            }
        }
        SignalValues::VarBytes { data, starts } => {
            let mut out_data = Vec::new();
            let mut out_starts = Vec::with_capacity(m + 1);
            out_starts.push(0);
            for &k in &indices {
                let s_start = starts[k];
                let s_end = starts[k + 1];
                out_data.extend_from_slice(&data[s_start..s_end]);
                out_starts.push(out_data.len());
            }
            SignalValues::VarBytes {
                data: out_data,
                starts: out_starts,
            }
        }
        SignalValues::Complex { re, im } => SignalValues::Complex {
            re: indices.iter().map(|&k| re[k]).collect(),
            im: indices.iter().map(|&k| im[k]).collect(),
        },
        SignalValues::CanopenDate(v) => {
            SignalValues::CanopenDate(indices.iter().map(|&k| v[k]).collect())
        }
        SignalValues::CanopenTime(v) => {
            SignalValues::CanopenTime(indices.iter().map(|&k| v[k]).collect())
        }
        SignalValues::Array {
            values: v,
            elements_per_sample,
        } => {
            let mut out = Vec::with_capacity(m * *elements_per_sample);
            for &k in &indices {
                let start = k * *elements_per_sample;
                let end = start + *elements_per_sample;
                out.extend_from_slice(&v[start..end]);
            }
            SignalValues::Array {
                values: out,
                elements_per_sample: *elements_per_sample,
            }
        }
        SignalValues::ArrayVarLen { values: v, starts } => {
            let mut out_data = Vec::new();
            let mut out_starts = Vec::with_capacity(m + 1);
            out_starts.push(0);
            for &k in &indices {
                let s_start = starts[k];
                let s_end = starts[k + 1];
                out_data.extend_from_slice(&v[s_start..s_end]);
                out_starts.push(out_data.len());
            }
            SignalValues::ArrayVarLen {
                values: out_data,
                starts: out_starts,
            }
        }
    }
}

/// Resamples the validity mask onto `target_t` using step-hold / preceding sample lookup.
pub(crate) fn resample_validity(
    validity: Option<&[bool]>,
    src_t: &[f64],
    target_t: &[f64],
) -> Option<Vec<bool>> {
    let v = validity?;
    if src_t.is_empty() || target_t.is_empty() {
        return Some(Vec::new());
    }
    let resampled = target_t
        .iter()
        .map(|&t| {
            let idx = step_hold_index(src_t, t);
            v.get(idx).copied().unwrap_or(true)
        })
        .collect();
    Some(resampled)
}
