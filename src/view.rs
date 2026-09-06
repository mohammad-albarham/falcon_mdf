//! Bounded viewer reads. Only a record chunk and the requested output are
//! retained; compressed input additionally needs one inflated MDF data block.

use crate::{Channel, Mf4Error, Mf4File, Result, SignalValues};

/// Numeric view of one element per sample. Missing dynamic-array elements
/// and invalid elements are gaps, never additional timestamps.
pub fn element_values(values: &SignalValues, validity: Option<&[bool]>, element: usize) -> Result<Vec<f64>> {
    let flat = values.to_f64();
    let n = values.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let index = match values {
            SignalValues::Array { elements_per_sample, .. } => {
                if element >= *elements_per_sample { return Err(Mf4Error::parse_error("array element out of range")); }
                Some(i * elements_per_sample + element)
            }
            SignalValues::ArrayVarLen { starts, .. } => {
                let a = starts[i]; let b = starts[i + 1];
                (element < b - a).then_some(a + element)
            }
            _ if element == 0 => Some(i),
            _ => return Err(Mf4Error::parse_error("scalar channel has only element zero")),
        };
        let valid = index.is_some_and(|j| validity.is_none_or(|v| {
            if v.len() == n { v.get(i).copied().unwrap_or(false) }
            else { v.get(j).copied().unwrap_or(false) }
        }));
        out.push(if valid { index.and_then(|j| flat.get(j)).copied().unwrap_or(f64::NAN) } else { f64::NAN });
    }
    Ok(out)
}

/// A decimated numeric window, including the full coordinate extent.
#[derive(Debug, Default)]
pub struct ViewWindow {
    /// Original coordinates of the retained samples.
    pub timestamps: Vec<f64>,
    /// Values, with NaN gaps for invalid samples.
    pub values: Vec<f64>,
    /// Full valid coordinate extent, independent of the requested window.
    pub extent: Option<(f64, f64)>,
    /// Number of source samples before decimation.
    pub total: usize,
}

#[derive(Clone, Copy)]
struct Point { index: usize, time: f64, value: f64 }

#[derive(Default)]
struct Bucket { first: Option<Point>, last: Option<Point>, min: Option<Point>, max: Option<Point>, gap: Option<Point> }
impl Bucket {
    fn push(&mut self, point: Point) {
        self.first.get_or_insert(point); self.last = Some(point);
        if point.value.is_finite() {
            if self.min.is_none_or(|p| point.value < p.value) { self.min = Some(point); }
            if self.max.is_none_or(|p| point.value > p.value) { self.max = Some(point); }
        } else { self.gap.get_or_insert(point); }
    }
}

impl Mf4File {
    /// Visits typed samples without retaining an entire decoded channel.
    /// Return false to stop. Raw table callers set `validate_axis` to false;
    /// invalid master coordinates then become NaN while values remain visible.
    pub fn visit_samples<F>(&self, channel: &Channel, validate_axis: bool, mut visit: F) -> Result<()>
    where F: FnMut(usize, &[f64], &SignalValues, Option<&[bool]>) -> Result<bool> {
        let master = self.master_channel(channel.data_group_index, channel.channel_group_index);
        let mut selected = vec![channel];
        if let Some(master) = master { selected.push(master); }
        let mut chunks = self.signals_chunks(&selected, 16_384)?;
        let mut offset = 0;
        let mut previous = None;
        for chunk in &mut chunks {
            let chunk = chunk?;
            let signal = &chunk[0];
            let mut times = if let Some(master) = master {
                let sig = &chunk[1];
                let mut times = sig.values_f64()?;
                let validity = sig.validity();
                if validate_axis {
                    crate::time_ops::validate_master_axis(&master.name, &times, validity.as_deref(), offset, previous)?;
                } else if let Some(validity) = validity {
                    for (t, valid) in times.iter_mut().zip(validity) { if !valid { *t = f64::NAN; } }
                }
                times
            } else { (offset..offset + signal.len()).map(|i| i as f64).collect() };
            if times.len() != signal.len() { return Err(Mf4Error::parse_error("master/sample length mismatch")); }
            previous = times.last().copied().or(previous);
            if !visit(offset, &times, &signal.values()?, signal.validity().as_deref())? { break; }
            offset += times.len();
            times.clear();
        }
        Ok(())
    }

    /// Peak-preserving bounded numeric plot data for an inclusive window.
    /// At most `max_points` samples are returned (minimum budget five).
    /// Each column preserves endpoints, finite extrema and an invalid gap.
    /// Both passes validate master coordinates across chunk boundaries.
    pub fn view_window(&self, channel: &Channel, element: usize, t0: f64, t1: f64, max_points: usize) -> Result<ViewWindow> {
        if max_points < 5 || max_points > 1_000_000 { return Err(Mf4Error::parse_error("point budget must be between 5 and 1000000")); }
        let mut result = ViewWindow::default();
        self.visit_samples(channel, true, |_, times, _, _| {
            if let (Some(&a), Some(&b)) = (times.first(), times.last()) {
                result.extent = Some((result.extent.map_or(a, |e| e.0), b));
            }
            result.total += times.len(); Ok(true)
        })?;
        let Some((a, b)) = result.extent else { return Ok(result) };
        if t0.is_nan() || t1.is_nan() || t0 > t1 { return Ok(result); }
        let lo = if t0.is_finite() { t0.max(a) } else { a };
        let hi = if t1.is_finite() { t1.min(b) } else { b };
        if lo > hi { return Ok(result); }
        let columns = max_points / 5;
        let mut buckets: Vec<Bucket> = (0..columns).map(|_| Bucket::default()).collect();
        self.visit_samples(channel, true, |offset, times, values, validity| {
            let values = element_values(values, validity, element)?;
            for (i, (&time, &value)) in times.iter().zip(&values).enumerate() {
                if time < lo || time > hi { continue; }
                let column = if hi > lo { (((time - lo) / (hi - lo)) * columns as f64) as usize } else { 0 }.min(columns - 1);
                buckets[column].push(Point { index: offset + i, time, value });
            }
            Ok(true)
        })?;
        for bucket in buckets {
            let mut points: Vec<Point> = [bucket.first, bucket.min, bucket.max, bucket.gap, bucket.last].into_iter().flatten().collect();
            points.sort_by_key(|p| p.index); points.dedup_by_key(|p| p.index);
            for point in points { result.timestamps.push(point.time); result.values.push(point.value); }
        }
        Ok(result)
    }
}

/// Bounded state transitions for a text plot.
#[derive(Debug, Default)]
pub struct TextWindow {
    /// Original coordinates of retained transitions.
    pub timestamps: Vec<f64>,
    /// Labels parallel to timestamps; invalid samples carry None.
    pub labels: Vec<Option<String>>,
    /// Full coordinate extent.
    pub extent: Option<(f64, f64)>,
    /// True when transitions were thinned to the requested point budget.
    pub truncated: bool,
}

impl Mf4File {
    /// Reads text transitions in two bounded passes. An exact table or cursor
    /// uses the raw samples, independently of transition thinning.
    pub fn view_text_window(&self, channel: &Channel, t0: f64, t1: f64, max_points: usize) -> Result<TextWindow> {
        if !(2..=1_000_000).contains(&max_points) { return Err(Mf4Error::parse_error("text point budget must be between 2 and 1000000")); }
        let mut result = TextWindow::default();
        let mut transitions = 0usize;
        let mut previous: Option<Option<String>> = None;
        self.visit_samples(channel, true, |_, times, values, validity| {
            let SignalValues::Str(labels) = values else { return Err(Mf4Error::parse_error("text window requires a text channel")); };
            if let (Some(&a), Some(&b)) = (times.first(), times.last()) { result.extent = Some((result.extent.map_or(a, |e| e.0), b)); }
            for (i, (&time, label)) in times.iter().zip(labels).enumerate() {
                if time < t0 || time > t1 || t0.is_nan() || t1.is_nan() { continue; }
                let label = validity.is_none_or(|v| v[i]).then(|| label.clone());
                if previous.as_ref() != Some(&label) { transitions += 1; previous = Some(label); }
            }
            Ok(true)
        })?;
        let stride = transitions.div_ceil(max_points.saturating_sub(1).max(1)).max(1);
        result.truncated = transitions + 1 > max_points;
        previous = None;
        let mut transition = 0usize;
        let mut last = None;
        self.visit_samples(channel, true, |_, times, values, validity| {
            let SignalValues::Str(labels) = values else { return Err(Mf4Error::parse_error("text window requires a text channel")); };
            for (i, (&time, label)) in times.iter().zip(labels).enumerate() {
                if time < t0 || time > t1 || t0.is_nan() || t1.is_nan() { continue; }
                let label = validity.is_none_or(|v| v[i]).then(|| label.clone());
                if previous.as_ref() != Some(&label) {
                    if transition % stride == 0 { result.timestamps.push(time); result.labels.push(label.clone()); }
                    transition += 1; previous = Some(label.clone());
                }
                last = Some((time, label));
            }
            Ok(true)
        })?;
        if let Some((time, label)) = last {
            if result.timestamps.last() != Some(&time) || result.labels.last() != Some(&label) {
                result.timestamps.push(time); result.labels.push(label);
            }
        }
        Ok(result)
    }
}
