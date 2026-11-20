//! Bounded viewer reads. Only a record chunk and the requested output are
//! retained; compressed input additionally needs one inflated MDF data block.

use crate::{Channel, Mf4Error, Mf4File, Result, SignalValues};

/// Numeric view of one element per sample. Missing dynamic-array elements
/// and invalid elements are gaps, never additional timestamps.
pub fn element_values(
    values: &SignalValues,
    validity: Option<&[bool]>,
    element: usize,
) -> Result<Vec<f64>> {
    let flat = values.to_f64();
    let n = values.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let index = match values {
            SignalValues::Array {
                elements_per_sample,
                ..
            } => {
                if element >= *elements_per_sample {
                    return Err(Mf4Error::parse_error("array element out of range"));
                }
                Some(i * elements_per_sample + element)
            }
            SignalValues::ArrayVarLen { starts, .. } => {
                let a = starts.get(i).copied().unwrap_or(0);
                let b = starts.get(i + 1).copied().unwrap_or(a);
                (element < b.saturating_sub(a)).then_some(a + element)
            }
            _ if element == 0 => Some(i),
            _ => {
                return Err(Mf4Error::parse_error(
                    "scalar channel has only element zero",
                ))
            }
        };
        let valid = index.is_some_and(|j| {
            validity.is_none_or(|v| {
                let per_element = matches!(
                    values,
                    SignalValues::Array { .. } | SignalValues::ArrayVarLen { .. }
                ) && v.len() == flat.len()
                    && v.len() != n;
                if per_element {
                    v.get(j).copied().unwrap_or(false)
                } else {
                    v.get(i).copied().unwrap_or(false)
                }
            })
        });
        out.push(if valid {
            index.and_then(|j| flat.get(j)).copied().unwrap_or(f64::NAN)
        } else {
            f64::NAN
        });
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
struct Point {
    index: usize,
    time: f64,
    value: f64,
}

#[derive(Default)]
struct Bucket {
    first: Option<Point>,
    last: Option<Point>,
    min: Option<Point>,
    max: Option<Point>,
    gap: Option<Point>,
}
impl Bucket {
    fn push(&mut self, point: Point) {
        self.first.get_or_insert(point);
        self.last = Some(point);
        if point.value.is_finite() {
            if self.min.is_none_or(|p| point.value < p.value) {
                self.min = Some(point);
            }
            if self.max.is_none_or(|p| point.value > p.value) {
                self.max = Some(point);
            }
        } else {
            self.gap.get_or_insert(point);
        }
    }
}

impl Mf4File {
    /// Visits typed samples without retaining an entire decoded channel.
    /// Return false to stop. Raw table callers set `validate_axis` to false;
    /// invalid master coordinates then become NaN while values remain visible.
    pub fn visit_samples<F>(
        &self,
        channel: &Channel,
        validate_axis: bool,
        mut visit: F,
    ) -> Result<()>
    where
        F: FnMut(usize, &[f64], &SignalValues, Option<&[bool]>) -> Result<bool>,
    {
        let master = self.master_channel(channel.data_group_index, channel.channel_group_index);
        let mut selected = vec![channel];
        if let Some(master) = master {
            selected.push(master);
        }
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
                    crate::time_ops::validate_master_axis(
                        &master.name,
                        &times,
                        validity.as_deref(),
                        offset,
                        previous,
                    )?;
                } else if let Some(validity) = validity {
                    for (t, valid) in times.iter_mut().zip(validity) {
                        if !valid {
                            *t = f64::NAN;
                        }
                    }
                }
                times
            } else {
                (offset..offset + signal.len()).map(|i| i as f64).collect()
            };
            if times.len() != signal.len() {
                return Err(Mf4Error::parse_error("master/sample length mismatch"));
            }
            previous = times.last().copied().or(previous);
            if !visit(
                offset,
                &times,
                &signal.values()?,
                signal.validity().as_deref(),
            )? {
                break;
            }
            offset += times.len();
            times.clear();
        }
        Ok(())
    }

    /// Peak-preserving bounded numeric plot data for an inclusive window.
    /// At most `max_points` samples are returned (minimum budget five).
    /// Each column preserves endpoints, finite extrema and an invalid gap.
    /// Both passes validate master coordinates across chunk boundaries.
    pub fn view_window(
        &self,
        channel: &Channel,
        element: usize,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<ViewWindow> {
        if !(5..=1_000_000).contains(&max_points) {
            return Err(Mf4Error::parse_error(
                "point budget must be between 5 and 1000000",
            ));
        }
        let mut result = ViewWindow::default();
        self.visit_samples(channel, true, |_, times, _, _| {
            if let (Some(&a), Some(&b)) = (times.first(), times.last()) {
                result.extent = Some((result.extent.map_or(a, |e| e.0), b));
            }
            result.total += times.len();
            Ok(true)
        })?;
        let Some((a, b)) = result.extent else {
            return Ok(result);
        };
        if t0.is_nan() || t1.is_nan() || t0 > t1 || t0 > b || t1 < a {
            return Ok(result);
        }
        let lo = if t0.is_finite() { t0.max(a) } else { a };
        let hi = if t1.is_finite() { t1.min(b) } else { b };
        if lo > hi {
            return Ok(result);
        }
        let columns = max_points / 5;
        let mut buckets: Vec<Bucket> = (0..columns).map(|_| Bucket::default()).collect();
        self.visit_samples(channel, true, |offset, times, values, validity| {
            let values = element_values(values, validity, element)?;
            for (i, (&time, &value)) in times.iter().zip(&values).enumerate() {
                if time < lo || time > hi {
                    continue;
                }
                let column = if hi > lo {
                    (((time - lo) / (hi - lo)) * columns as f64) as usize
                } else {
                    0
                }
                .min(columns - 1);
                buckets[column].push(Point {
                    index: offset + i,
                    time,
                    value,
                });
            }
            Ok(true)
        })?;
        for bucket in buckets {
            let mut points: Vec<Point> = [
                bucket.first,
                bucket.min,
                bucket.max,
                bucket.gap,
                bucket.last,
            ]
            .into_iter()
            .flatten()
            .collect();
            points.sort_by_key(|p| p.index);
            points.dedup_by_key(|p| p.index);
            for point in points {
                result.timestamps.push(point.time);
                result.values.push(point.value);
            }
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
    pub fn view_text_window(
        &self,
        channel: &Channel,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<TextWindow> {
        if !(2..=1_000_000).contains(&max_points) {
            return Err(Mf4Error::parse_error(
                "text point budget must be between 2 and 1000000",
            ));
        }
        let mut result = TextWindow::default();
        if t0.is_nan() || t1.is_nan() || t0 > t1 {
            return Ok(result);
        }
        let mut transitions = 0usize;
        let mut previous: Option<Option<String>> = None;
        self.visit_samples(channel, true, |_, times, values, validity| {
            let SignalValues::Str(labels) = values else {
                return Err(Mf4Error::parse_error("text window requires a text channel"));
            };
            if let (Some(&a), Some(&b)) = (times.first(), times.last()) {
                result.extent = Some((result.extent.map_or(a, |e| e.0), b));
            }
            for (i, (&time, label)) in times.iter().zip(labels).enumerate() {
                if time < t0 || time > t1 {
                    continue;
                }
                let label = validity
                    .is_none_or(|v| v.get(i).copied().unwrap_or(false))
                    .then(|| label.clone());
                if previous.as_ref() != Some(&label) {
                    transitions += 1;
                    previous = Some(label);
                }
            }
            Ok(true)
        })?;
        let stride = transitions
            .div_ceil(max_points.saturating_sub(1).max(1))
            .max(1);
        result.truncated = transitions > max_points;
        previous = None;
        let mut transition = 0usize;
        let mut last = None;
        self.visit_samples(channel, true, |_, times, values, validity| {
            let SignalValues::Str(labels) = values else {
                return Err(Mf4Error::parse_error("text window requires a text channel"));
            };
            for (i, (&time, label)) in times.iter().zip(labels).enumerate() {
                if time < t0 || time > t1 {
                    continue;
                }
                let label = validity
                    .is_none_or(|v| v.get(i).copied().unwrap_or(false))
                    .then(|| label.clone());
                if previous.as_ref() != Some(&label) {
                    if transition.is_multiple_of(stride) {
                        result.timestamps.push(time);
                        result.labels.push(label.clone());
                    }
                    transition += 1;
                    previous = Some(label.clone());
                }
                last = Some((time, label));
            }
            Ok(true)
        })?;
        if let Some((time, label)) = last {
            if result.timestamps.last() != Some(&time) || result.labels.last() != Some(&label) {
                result.timestamps.push(time);
                result.labels.push(label);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Conversion, Mf4File};

    #[test]
    fn view_window_and_text_window_handle_infinite_and_nan_bounds() {
        // Virtual master channel over 20,000 samples (>16,384 stream chunk) also checks CORE-04 offset continuity.
        let n = 20_000usize;
        let mut bytes = Vec::new();
        let mut hd = [0u8; 64 + 24 + 80];
        hd[0..8].copy_from_slice(b"MDF     ");
        hd[8..16].copy_from_slice(b"4.10    ");
        hd[16..24].copy_from_slice(b"test    ");
        hd[28..30].copy_from_slice(&410u16.to_le_bytes());

        let hd_off = 64usize;
        hd[hd_off..hd_off + 4].copy_from_slice(b"##HD");
        hd[hd_off + 8..hd_off + 16].copy_from_slice(&104u64.to_le_bytes());
        hd[hd_off + 16..hd_off + 24].copy_from_slice(&6u64.to_le_bytes());
        bytes.extend_from_slice(&hd[..64 + 104]);

        let dg_off = bytes.len() as u64;
        let mut dg = vec![0u8; 24 + 32 + 8];
        dg[0..4].copy_from_slice(b"##DG");
        dg[8..16].copy_from_slice(&64u64.to_le_bytes());
        dg[16..24].copy_from_slice(&4u64.to_le_bytes());
        let cg_off = dg_off + 64;
        dg[24 + 8..24 + 16].copy_from_slice(&cg_off.to_le_bytes());
        bytes.extend_from_slice(&dg);

        let mut cg = vec![0u8; 24 + 48 + 32];
        cg[0..4].copy_from_slice(b"##CG");
        cg[8..16].copy_from_slice(&104u64.to_le_bytes());
        cg[16..24].copy_from_slice(&6u64.to_le_bytes());
        let cn_master_off = cg_off + 104;
        cg[24 + 8..24 + 16].copy_from_slice(&cn_master_off.to_le_bytes());
        cg[24 + 48 + 8..24 + 48 + 16].copy_from_slice(&(n as u64).to_le_bytes()); // cycle_count
        cg[24 + 48 + 24..24 + 48 + 28].copy_from_slice(&1u32.to_le_bytes()); // cg_data_bytes = 1
        bytes.extend_from_slice(&cg);

        // VirtualMaster (3) + SyncType::Time (1) + UIntLe (0) with linear factor 0.001, offset 0.0
        let cc_off = cn_master_off + 160;
        let mut cn = vec![0u8; 160];
        cn[0..4].copy_from_slice(b"##CN");
        cn[8..16].copy_from_slice(&160u64.to_le_bytes());
        cn[16..24].copy_from_slice(&8u64.to_le_bytes());
        cn[24 + 4 * 8..24 + 5 * 8].copy_from_slice(&cc_off.to_le_bytes()); // cn_cc_conversion
        let data = 24 + 64;
        cn[data] = 3; // ChannelType::VirtualMaster
        cn[data + 1] = 1; // SyncType::Time
        cn[data + 2] = 0; // DataType::UIntLe
        cn[data + 5] = 0; // bit_count
        bytes.extend_from_slice(&cn);

        let mut cc = vec![0u8; 24 + 32 + 24 + 16];
        cc[0..4].copy_from_slice(b"##CC");
        let cc_len = cc.len() as u64;
        cc[8..16].copy_from_slice(&cc_len.to_le_bytes());
        cc[16..24].copy_from_slice(&4u64.to_le_bytes());
        cc[56] = 1; // Linear
        cc[62..64].copy_from_slice(&2u16.to_le_bytes()); // val_count = 2
        cc[80..88].copy_from_slice(&0.0f64.to_le_bytes());
        cc[88..96].copy_from_slice(&0.001f64.to_le_bytes());
        bytes.extend_from_slice(&cc);

        let dt_off = bytes.len() as u64;
        let mut dt = vec![0u8; 24 + n];
        dt[0..4].copy_from_slice(b"##DT");
        let dt_len = dt.len() as u64;
        dt[8..16].copy_from_slice(&dt_len.to_le_bytes());
        bytes.extend_from_slice(&dt);

        // Patch hd_dg_first and dg_data
        bytes[hd_off + 24..hd_off + 32].copy_from_slice(&dg_off.to_le_bytes());
        bytes[dg_off as usize + 24 + 16..dg_off as usize + 24 + 24]
            .copy_from_slice(&dt_off.to_le_bytes());

        let file = Mf4File::from_bytes(bytes).unwrap();
        let ch = &file.data_groups()[0].channel_groups[0].channels[0];
        assert!(matches!(ch.conversion, Conversion::Linear { .. }));

        // Infinite bounds across 20,000 samples (chunks 0 and 1) must not NaN-clamp and must preserve extent/monotone timestamps.
        let win = file
            .view_window(ch, 0, f64::NEG_INFINITY, f64::INFINITY, 50)
            .unwrap();
        assert_eq!(win.extent, Some((0.0, (n - 1) as f64 * 0.001)));
        assert!(!win.timestamps.is_empty());
        assert!(win.timestamps.windows(2).all(|w| w[0] <= w[1]));
        assert!(win.timestamps.last().copied().unwrap() > 16.384);

        let empty_nan = file.view_window(ch, 0, f64::NAN, 1.0, 50).unwrap();
        assert!(empty_nan.timestamps.is_empty());
        let empty_inf = file
            .view_window(ch, 0, f64::INFINITY, f64::INFINITY, 50)
            .unwrap();
        assert!(empty_inf.timestamps.is_empty());
    }
}
