//! WebAssembly bindings for `falcon_mdf` via `wasm-bindgen`.
//!
//! Exposes a reading API over in-memory MF4 files for WebAssembly and JavaScript runtimes:
//!
//! - [`WasmMf4File::new`] reads an MF4 file from raw bytes (e.g. `Uint8Array`).
//! - [`WasmMf4File::channel_names`] lists every channel name in the file as a JSON array of strings.
//! - [`WasmMf4File::channel_count`] returns the total number of channels.
//! - [`WasmMf4File::signal`] returns a channel's samples as a JSON object with timestamps and values.
//! - [`WasmMf4File::info`] returns file metadata (version, start time, group and channel counts) as a JSON object.
//! - [`WasmMf4File::channels`] returns every channel's metadata (name, unit, group, description) in one JSON call.
//! - [`WasmMf4File::signal_arrays`] returns a channel's samples as `Float64Array`s (`NaN` stays `NaN`).
//! - [`WasmMf4File::signal_window`] returns a time window of a channel, decimated in Rust to a
//!   point budget so the browser never receives more points than it draws.
//! - [`WasmMf4File::signal_csv`] formats a time window of one channel as CSV, in Rust.
//!
//! A wasm panic would kill the whole module for every caller, so nothing here may
//! panic: no `unwrap`/`expect`, no panicking indexing, and every error crosses
//! into JS as a thrown `Error` via [`js_err`].

use std::fmt::Write;
use wasm_bindgen::prelude::*;

use falcon_mdf::error::Mf4Error;
use falcon_mdf::Mf4File;

/// Converts an error into a [`JsValue`] carrying the error message as a thrown JavaScript error.
fn js_err(err: impl std::fmt::Display) -> JsValue {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        JsError::new(&err.to_string()).into()
    }
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = err;
        JsValue::NULL
    }
}

/// Escapes a string for JSON output according to RFC 8259 and appends it to `out`.
///
/// Handles quotation marks, reverse solidi, standard escape characters (`\b`, `\f`, `\n`, `\r`, `\t`),
/// and control characters below 0x20 formatted as `\u00XX`.
pub fn escape_json_str_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Escapes a string for JSON output according to RFC 8259.
pub fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_json_str_into(s, &mut out);
    out
}

/// Appends an `f64` value to a JSON output buffer.
///
/// Finite numbers are formatted as decimal floating-point representations.
/// Non-finite numbers (`NaN`, `+inf`, `-inf`) are formatted as `null` as required by JSON.
pub fn write_f64(out: &mut String, val: f64) {
    if val.is_finite() {
        let _ = write!(out, "{}", val);
    } else {
        out.push_str("null");
    }
}

/// Appends an `f64` value to a CSV output buffer.
///
/// Unlike JSON, CSV has no `null`: a non-finite sample becomes an empty field,
/// which every spreadsheet and parser reads as a missing value.
fn write_csv_f64(out: &mut String, val: f64) {
    if val.is_finite() {
        let _ = write!(out, "{}", val);
    }
}

/// Appends a string as a single RFC 4180 CSV field, quoting it only when it
/// contains a comma, quote, or newline.
fn write_csv_field(out: &mut String, field: &str) {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        out.push('"');
        for c in field.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(field);
    }
}

/// Formats `(times[i], values[i])` pairs as a two-column CSV: a
/// `timestamp,<name>` header, then one row per sample. Non-finite values
/// become empty fields.
///
/// Pure so it can be tested natively without a JS runtime; [`WasmMf4File::signal_csv`]
/// only slices the decoded series and delegates here.
pub fn series_csv(times: &[f64], values: &[f64], name: &str) -> String {
    let mut out = String::with_capacity(times.len() * 24 + 16);
    out.push_str("timestamp,");
    write_csv_field(&mut out, name);
    out.push('\n');
    for (&t, &v) in times.iter().zip(values.iter()) {
        write_csv_f64(&mut out, t);
        out.push(',');
        write_csv_f64(&mut out, v);
        out.push('\n');
    }
    out
}

/// Decimates one run of finite samples into pixel columns, emitting at most
/// four points per column — first, min, max, last, in source order.
///
/// This is the `gui/src/decimate.rs` min/max algorithm, extended with the
/// column's first and last samples: min/max alone guarantees a spike survives,
/// and first/last additionally makes the polyline enter and exit the column at
/// real samples instead of jumping between neighbouring columns' extremes.
/// Emitting the four indices in ascending order (deduplicated) keeps the
/// output strictly time-ordered, so the canvas can stroke it as one path.
///
/// Like the reference implementation, column boundaries are recomputed from
/// the absolute timestamp of each column's first untouched sample (not a
/// running counter), so empty columns cannot desynchronize later columns from
/// their true boundaries, and the sample under the cursor is always consumed
/// before any boundary test so the loop always makes progress — a `col_end`
/// that rounds back to `x0` (a zoom narrower than the ulp of an
/// epoch-seconds master) then yields one-point columns instead of spinning.
fn decimate_run(
    times: &[f64],
    values: &[f64],
    x0: f64,
    span: f64,
    n_columns: usize,
    hard_cap: usize,
    out_t: &mut Vec<f64>,
    out_v: &mut Vec<f64>,
) {
    let col_width = span / n_columns as f64;
    // A span narrower than the representable width of a column has no
    // meaningful columns. That covers both a zero/negative width (identical
    // timestamps) and the ulp collapse of a zoom narrower than the spacing of
    // `x0` itself (an epoch-seconds master): there `x0 + col_width` rounds
    // straight back to `x0`, every column would degenerate to one sample, and
    // the budget would silently blow past itself. Keep the run's extremes
    // instead of emitting every duplicate sample.
    if !(col_width > 0.0) || !col_width.is_finite() || !(x0 + col_width > x0) {
        push_first_min_max_last(times, values, out_t, out_v);
        return;
    }

    let mut i = 0;
    while i < times.len() {
        if out_t.len() >= hard_cap {
            return;
        }
        // See the comment above: `col_index` is derived from the first
        // untouched sample, and that sample is consumed unconditionally.
        let col_index = ((times[i] - x0) / col_width) as usize;
        let col_end = x0 + (col_index as f64 + 1.0) * col_width;
        let first_i = i;
        let mut min_i = i;
        let mut max_i = i;
        i += 1;
        while i < times.len() && times[i] < col_end {
            if values[i] < values[min_i] {
                min_i = i;
            }
            if values[i] > values[max_i] {
                max_i = i;
            }
            i += 1;
        }
        let last_i = i - 1;

        // Ascending, deduplicated: at most four points, in time order.
        let mut idx = [first_i, min_i, max_i, last_i];
        idx.sort_unstable();
        for slot in 0..idx.len() {
            if slot > 0 && idx[slot] == idx[slot - 1] {
                continue;
            }
            out_t.push(times[idx[slot]]);
            out_v.push(values[idx[slot]]);
        }
    }
}

/// Pushes the first, min, max and last sample of a (finite) run, the
/// degenerate-column fallback of [`decimate_run`].
fn push_first_min_max_last(
    times: &[f64],
    values: &[f64],
    out_t: &mut Vec<f64>,
    out_v: &mut Vec<f64>,
) {
    if times.is_empty() || values.is_empty() {
        return;
    }
    let mut min_i = 0;
    let mut max_i = 0;
    for (i, &v) in values.iter().enumerate() {
        if v < values[min_i] {
            min_i = i;
        }
        if v > values[max_i] {
            max_i = i;
        }
    }
    let last_i = values.len() - 1;
    let mut idx = [0, min_i, max_i, last_i];
    idx.sort_unstable();
    for slot in 0..idx.len() {
        if slot > 0 && idx[slot] == idx[slot - 1] {
            continue;
        }
        out_t.push(times[idx[slot]]);
        out_v.push(values[idx[slot]]);
    }
}

/// Restricts `(times[i], values[i])` to `[t0, t1]` and decimates the result to
/// at most `max_points` points, so the browser never receives more points
/// than it draws.
///
/// Behaviour, in order:
///
/// - A window covering no samples (including `t0 > t1`, or either bound
///   non-finite on an empty series) yields two empty vectors.
/// - Non-finite bounds are clamped to the series' own extent, so a viewer can
///   bootstrap its initial view with `(-Infinity, Infinity)`.
/// - When the window holds `max_points` samples or fewer, the visible samples
///   are returned untouched: there is nothing to aggregate away, and returning
///   fewer points than were asked for would just be a different way of lying
///   about what the file contains.
/// - Otherwise the window is divided into `max_points / 4` columns and each
///   column keeps its first, min, max and last sample
///   ([`decimate_run`]); a single-sample spike is always a column extreme, so
///   it always survives.
/// - Non-finite values (a NaN in the data, or a sample the file's
///   invalidation bits mark invalid — the reader folds both into the value)
///   neither contribute to a column's extremes nor get bridged by a line:
///   each run of them collapses to a single `NaN` point, which the drawing
///   side turns into a gap. Finite runs share the column budget in
///   proportion to their length, so many short runs cannot blow past
///   `max_points`; a 1.5× `hard_cap` guards the duplicate-timestamp corner
///   where columns degenerate to one sample each.
///
/// `times` must be sorted ascending, as every MDF master channel is. A
/// non-monotonic master (duplicate or backwards timestamps) cannot panic or
/// hang this function — the worst case is a conservative extra column.
pub fn decimate_window(
    times: &[f64],
    values: &[f64],
    t0: f64,
    t1: f64,
    max_points: usize,
) -> (Vec<f64>, Vec<f64>) {
    if times.is_empty() || values.len() != times.len() || max_points == 0 || !(t0 <= t1) {
        return (Vec::new(), Vec::new());
    }
    // Clamp non-finite bounds to the data's extent (the initial full view).
    let x0 = if t0.is_finite() { t0 } else { times[0] };
    let x1 = if t1.is_finite() {
        t1
    } else {
        times[times.len() - 1]
    };
    if !(x0 <= x1) {
        return (Vec::new(), Vec::new());
    }

    let start = times.partition_point(|&t| t < x0);
    let end = times.partition_point(|&t| t <= x1);
    if start >= end {
        return (Vec::new(), Vec::new());
    }
    if end - start <= max_points {
        return (times[start..end].to_vec(), values[start..end].to_vec());
    }

    let n_columns = (max_points / 4).max(1);
    let hard_cap = max_points + max_points / 2;

    // One pass over the window, splitting it into runs of finite samples and
    // runs of non-finite ones; the finite runs are decimated, the non-finite
    // runs each collapse to one NaN point that breaks the drawn line.
    let mut out_t = Vec::with_capacity(max_points.min(end - start));
    let mut out_v = Vec::with_capacity(out_t.capacity());
    let mut i = start;
    let mut finite_total: usize = 0;
    while i < end {
        if values[i].is_finite() {
            let run = i;
            while i < end && values[i].is_finite() {
                i += 1;
            }
            finite_total += i - run;
        } else {
            while i < end && !values[i].is_finite() {
                i += 1;
            }
        }
    }

    i = start;
    while i < end {
        if !values[i].is_finite() {
            let gap_t = times[i];
            while i < end && !values[i].is_finite() {
                i += 1;
            }
            out_t.push(gap_t);
            out_v.push(f64::NAN);
            continue;
        }
        let run = i;
        while i < end && values[i].is_finite() {
            i += 1;
        }
        let len = i - run;
        // u64 math: `n_columns * len` overflows u32 (wasm's usize) on large
        // files long before either factor does.
        let cols = ((n_columns as u64 * len as u64) / finite_total as u64).max(1) as usize;
        decimate_run(
            &times[run..i],
            &values[run..i],
            x0,
            x1 - x0,
            cols,
            hard_cap.saturating_sub(out_t.len()),
            &mut out_t,
            &mut out_v,
        );
    }
    (out_t, out_v)
}

/// A channel's decoded samples, kept as `f64` for the typed-array and
/// decimation paths.
struct CachedSeries {
    unit: String,
    timestamps: Vec<f64>,
    values: Vec<f64>,
}

/// How many decoded channels to keep. The viewer owns its `WasmMf4File` from
/// one dedicated worker, so a plain FIFO is enough to make zoom/pan
/// re-requests reuse the previous decode; eight is the viewer's channel
/// overlay limit plus slack.
const SERIES_CACHE_CAP: usize = 10;

/// Folds the file's per-sample invalidation bits into the values: an invalid
/// sample becomes `NaN`, the same marker the data itself uses, so one code
/// path (drawing gap, decimation run split, empty CSV field) handles both.
///
/// A validity vector whose length doesn't match the samples can't be lined up
/// with them, so it is ignored rather than trusted (same stance as the GUI's
/// decimator).
fn fold_validity(values: &mut [f64], validity: Option<&[bool]>) {
    if let Some(valid) = validity {
        if valid.len() == values.len() {
            for (v, ok) in values.iter_mut().zip(valid.iter()) {
                if !ok {
                    *v = f64::NAN;
                }
            }
        }
    }
}

/// An MF4 file held in browser memory.
#[wasm_bindgen]
pub struct WasmMf4File {
    inner: Mf4File,
    series_cache: Vec<(String, CachedSeries)>,
}

#[wasm_bindgen]
impl WasmMf4File {
    /// Reads a file from bytes, e.g. a `Uint8Array` from `fetch` or a file input.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>) -> Result<WasmMf4File, JsValue> {
        let inner = Mf4File::from_bytes(bytes).map_err(js_err)?;
        Ok(WasmMf4File {
            inner,
            series_cache: Vec::new(),
        })
    }

    /// Decodes `name` once and caches it, folding the file's per-sample
    /// invalidation bits into the values (an invalid sample becomes `NaN`, the
    /// same marker the data itself uses, so one code path handles both).
    fn decoded(&mut self, name: &str) -> Result<&CachedSeries, JsValue> {
        if let Some(pos) = self.series_cache.iter().position(|(n, _)| n == name) {
            // Move-to-front: the channels a viewer keeps zooming are the ones
            // it just asked for.
            let entry = self.series_cache.remove(pos);
            self.series_cache.push(entry);
            // Just re-pushed, so last() is Some; ok_or keeps the whole crate
            // panic-free even if that invariant ever breaks.
            return self
                .series_cache
                .last()
                .map(|entry| &entry.1)
                .ok_or_else(|| js_err("series cache corrupted"));
        }

        let channel = self
            .inner
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;
        let unit = channel.unit.clone();
        let series = self.inner.time_series(channel).map_err(js_err)?;
        let mut values = series.values.to_f64();
        fold_validity(&mut values, series.validity.as_deref());

        let entry = CachedSeries {
            unit,
            timestamps: series.timestamps,
            values,
        };
        if self.series_cache.len() >= SERIES_CACHE_CAP {
            self.series_cache.remove(0);
        }
        self.series_cache.push((name.to_string(), entry));
        self.series_cache
            .last()
            .map(|entry| &entry.1)
            .ok_or_else(|| js_err("series cache corrupted"))
    }

    /// Every channel name in the file, as a JSON array of strings.
    pub fn channel_names(&self) -> Result<String, JsValue> {
        let mut out = String::from("[");
        for (i, name) in self.inner.channel_names().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_json_str_into(name, &mut out);
            out.push('"');
        }
        out.push(']');
        Ok(out)
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        self.inner.channel_count()
    }

    /// One channel's samples as a JSON object with `name`, `unit`,
    /// `timestamps` and `values` arrays.
    ///
    /// Non-finite floats (`NaN`, `+inf`, `-inf`) are not valid JSON and are emitted
    /// as `null` in both `timestamps` and `values` arrays.
    pub fn signal(&self, name: &str) -> Result<String, JsValue> {
        let channel = self
            .inner
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;

        let series = self.inner.time_series(channel).map_err(js_err)?;
        let values = series.values.to_f64();
        let timestamps = series.timestamps;

        let mut out = String::new();
        out.push_str("{\"name\":\"");
        escape_json_str_into(&channel.name, &mut out);
        out.push_str("\",\"unit\":\"");
        escape_json_str_into(&channel.unit, &mut out);
        out.push_str("\",\"timestamps\":[");
        for (i, &t) in timestamps.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, t);
        }
        out.push_str("],\"values\":[");
        for (i, &v) in values.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, v);
        }
        out.push_str("]}");
        Ok(out)
    }

    /// Version, start time, group and channel counts, as a JSON object.
    pub fn info(&self) -> Result<String, JsValue> {
        let channel_group_count: usize = self
            .inner
            .data_groups()
            .iter()
            .map(|dg| dg.channel_groups.len())
            .sum();

        let mut out = String::new();
        out.push_str("{\"version\":\"");
        escape_json_str_into(&self.inner.version().to_string(), &mut out);
        out.push_str("\",\"start_time\":\"");
        escape_json_str_into(&self.inner.start_time().to_iso8601(), &mut out);
        out.push_str("\",\"channel_group_count\":");
        let _ = write!(
            out,
            "{},\"channel_count\":{}",
            channel_group_count,
            self.inner.channel_count()
        );
        out.push('}');
        Ok(out)
    }

    /// Every channel's metadata in one JSON call, as an array of
    /// `{name, unit, group, description}` objects — one metadata round trip
    /// instead of one `signal` call per channel just to learn the unit.
    ///
    /// The list matches [`WasmMf4File::channel_names`]: same sorted order, one
    /// entry per unique name. `group` is the channel group's acquisition name,
    /// falling back to `group <dg>.<cg>` when the file carries none;
    /// `description` is the channel's comment.
    pub fn channels(&self) -> Result<String, JsValue> {
        let groups = self.inner.data_groups();
        let mut out = String::from("[");
        for (i, name) in self.inner.channel_names().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            // Same name-order as channel_names, so the pair of calls cannot
            // disagree about what the file contains.
            let Some(channel) = self.inner.find_channel(name) else {
                continue;
            };
            out.push_str("{\"name\":\"");
            escape_json_str_into(name, &mut out);
            out.push_str("\",\"unit\":\"");
            escape_json_str_into(&channel.unit, &mut out);
            out.push_str("\",\"group\":\"");
            let group = groups
                .get(channel.data_group_index)
                .and_then(|dg| dg.channel_groups.get(channel.channel_group_index))
                .map(|cg| cg.acquisition_name.trim())
                .filter(|acq| !acq.is_empty());
            match group {
                Some(acq) => escape_json_str_into(acq, &mut out),
                None => {
                    let _ = write!(
                        out,
                        "group {}.{}",
                        channel.data_group_index, channel.channel_group_index
                    );
                }
            }
            out.push_str("\",\"description\":\"");
            escape_json_str_into(&channel.comment, &mut out);
            out.push_str("\"}");
        }
        out.push(']');
        Ok(out)
    }

    /// One channel's samples as `{timestamps: Float64Array, values: Float64Array,
    /// name, unit}`.
    ///
    /// Unlike [`WasmMf4File::signal`] (JSON, where non-finite floats must
    /// become `null`), a typed array round-trips `NaN` bit-for-bit — the
    /// drawing side turns those into gaps in the line.
    pub fn signal_arrays(&mut self, name: &str) -> Result<js_sys::Object, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            values,
        } = self.decoded(name)?;
        series_object(name, unit, timestamps, values)
    }

    /// [`WasmMf4File::signal_arrays`] restricted to `[t0, t1]` and decimated in
    /// Rust to at most `max_points` points (first/min/max/last per column —
    /// see [`decimate_window`]), so a zoom or pan never ships more points to
    /// the main thread than it will draw.
    ///
    /// Non-finite bounds are clamped to the channel's extent, so a viewer can
    /// request its initial full view as `(-Infinity, Infinity)`.
    pub fn signal_window(
        &mut self,
        name: &str,
        t0: f64,
        t1: f64,
        max_points: usize,
    ) -> Result<js_sys::Object, JsValue> {
        let CachedSeries {
            unit,
            timestamps,
            values,
        } = self.decoded(name)?;
        let (ts, vs) = decimate_window(timestamps, values, t0, t1, max_points);
        series_object(name, unit, &ts, &vs)
    }

    /// One channel's samples within `[t0, t1]` as CSV (`timestamp,<name>`
    /// header, one row per sample, non-finite values as empty fields),
    /// formatted in Rust so a "Download CSV" of the visible window costs the
    /// main thread one string.
    pub fn signal_csv(&self, name: &str, t0: f64, t1: f64) -> Result<String, JsValue> {
        let channel = self
            .inner
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;
        let series = self.inner.time_series(channel).map_err(js_err)?;
        let mut values = series.values.to_f64();
        fold_validity(&mut values, series.validity.as_deref());
        let times = &series.timestamps;
        let (x0, x1) = (finite_or(t0, times.first()), finite_or(t1, times.last()));
        let (Some(x0), Some(x1)) = (x0, x1) else {
            return Ok(series_csv(&[], &[], name));
        };
        let start = times.partition_point(|&t| t < x0);
        let end = times.partition_point(|&t| t <= x1);
        Ok(series_csv(
            &times.get(start..end).unwrap_or(&[]),
            &values.get(start..end).unwrap_or(&[]),
            name,
        ))
    }
}

/// `bound` when finite, otherwise the series extent `fallback` (an empty
/// series has none, which the caller turns into an empty CSV).
fn finite_or(bound: f64, fallback: Option<&f64>) -> Option<f64> {
    if bound.is_finite() {
        Some(bound)
    } else {
        fallback.copied()
    }
}

/// Builds the `{timestamps, values, name, unit}` plain object shared by
/// [`WasmMf4File::signal_arrays`] and [`WasmMf4File::signal_window`].
///
/// The typed arrays are copied out of wasm memory (not views into it), so the
/// receiving worker can move their buffers to the main thread and they stay
/// valid whatever the module does next.
fn series_object(
    name: &str,
    unit: &str,
    timestamps: &[f64],
    values: &[f64],
) -> Result<js_sys::Object, JsValue> {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        let obj = js_sys::Object::new();
        let ts = js_sys::Float64Array::new_from_slice(timestamps);
        let vs = js_sys::Float64Array::new_from_slice(values);
        let set = |key: &str, val: JsValue| -> Result<(), JsValue> {
            js_sys::Reflect::set(obj.as_ref(), &JsValue::from_str(key), &val).map(|_| ())
        };
        set("timestamps", ts.into())?;
        set("values", vs.into())?;
        set("name", JsValue::from_str(name))?;
        set("unit", JsValue::from_str(unit))?;
        Ok(obj)
    }
    // Native builds have no JS runtime to build the object in; the logic is
    // covered by the decimate_window/series_csv tests and the browser demo.
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = (name, unit, timestamps, values);
        Err(JsValue::NULL)
    }
}
