//! Export of decoded channels to formats other tools read.
//!
//! CSV is always available. The other writers are behind their own off-by-default
//! feature, so a build that only reads MDF pulls in no writer and none of their
//! dependencies:
//!
//! | Format | Feature | Entry point |
//! |---|---|---|
//! | CSV | — | [`write_csv`] |
//! | Arrow IPC | `arrow` | `write_arrow_ipc` |
//! | Apache Parquet | `parquet` | `write_parquet` |
//! | MATLAB v4 MAT | `mat4` | `write_mat_v4` |
//! | MATLAB level 5 MAT | `mat` | `write_mat` |
//! | MATLAB v7.3 MAT | `mat73` | `write_mat73` |
//! | HDF5 | `hdf5` | `write_hdf5` |
//! | Vector CANoe ASCII | `asc` | `write_asc` |
//!
//! [`write_csv`] takes a file and the channels to read from it. The other
//! writers take `&[SignalSeries]` instead (or `&Mf4File` for ASC) — the decoded,
//! in-memory form that [`filter`](crate::Mf4File::filter), [`cut`](crate::Mf4File::cut),
//! [`resample`](crate::Mf4File::resample), [`concatenate`](crate::multi_ops::concatenate)
//! and [`stack`](crate::multi_ops::stack) all already produce, so selecting,
//! trimming, re-gridding and joining compose in front of an export instead of
//! each writer growing its own copy of them.

#[cfg(feature = "arrow")]
mod arrow;
#[cfg(feature = "asc")]
mod asc;
#[cfg(feature = "hdf5")]
mod hdf5;
#[cfg(feature = "mat")]
mod mat;
#[cfg(feature = "mat73")]
mod mat73;
#[cfg(feature = "mat4")]
mod mat_v4;
#[cfg(feature = "parquet")]
mod parquet;

#[cfg(feature = "arrow")]
pub use arrow::{to_record_batch, write_arrow_ipc};
#[cfg(feature = "asc")]
pub use asc::{write_asc, write_asc_frames};
#[cfg(feature = "hdf5")]
pub use hdf5::write_hdf5;
#[cfg(feature = "mat")]
pub use mat::write_mat;
#[cfg(feature = "mat73")]
pub use mat73::write_mat73;
#[cfg(feature = "mat4")]
pub use mat_v4::write_mat_v4;
#[cfg(feature = "parquet")]
pub use parquet::{write_parquet, write_parquet_with, ParquetCompression};

#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73", feature = "hdf5"))]
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write;
use std::sync::Arc;

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, Signal, SignalValues};
#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73", feature = "hdf5"))]
use crate::time_ops::SignalSeries;
use crate::Mf4File;

/// Compares two timestamp slices for equality, treating `NaN == NaN` as true.
pub(crate) fn timestamps_eq(a: &[f64], b: &[f64]) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    if a.len() != b.len() {
        return false;
    }
    let elem_eq = |x: f64, y: f64| x == y || (x.is_nan() && y.is_nan());
    if let (Some((&af, &bf)), Some((&al, &bl))) = (a.first().zip(b.first()), a.last().zip(b.last()))
    {
        if !elem_eq(af, bf) || !elem_eq(al, bl) {
            return false;
        }
    }
    a.iter().zip(b).all(|(&x, &y)| elem_eq(x, y))
}

/// Groups series by their time axis, in first-appearance order.
///
/// Returns one vector of indices into `series` per distinct time axis.
#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73", feature = "hdf5"))]
pub(crate) fn time_groups(series: &[SignalSeries]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, s) in series.iter().enumerate() {
        match groups
            .iter_mut()
            .rev()
            .find(|group| timestamps_eq(series[group[0]].timestamps(), s.timestamps()))
        {
            Some(group) => group.push(index),
            None => groups.push(vec![index]),
        }
    }
    groups
}

/// Hands out MATLAB-legal variable names (`<= 63` characters), never the same one twice.
///
/// Base variable names are capped at 55 characters so that a companion `_invalid`
/// mask (`+ 8` characters) always fits within MATLAB's 63-character `namelengthmax`
/// without truncating the `_invalid` suffix.
#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73"))]
#[derive(Default)]
pub(crate) struct UniqueNames {
    seen: HashSet<String>,
}

#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73"))]
impl UniqueNames {
    const MAX_BASE_LEN: usize = 55;
    const MAX_IDENT_LEN: usize = 63;

    pub(crate) fn claim(&mut self, name: &str) -> String {
        self.claim_with_limit(name, Self::MAX_BASE_LEN)
    }

    pub(crate) fn claim_invalid(&mut self, name: &str) -> String {
        self.claim_with_limit(name, Self::MAX_IDENT_LEN)
    }

    fn claim_with_limit(&mut self, name: &str, max_len: usize) -> String {
        let base = matlab_compatible_with_limit(name, max_len);
        if self.seen.insert(base.clone()) {
            return base;
        }
        let mut count = 1usize;
        loop {
            let suffix = format!("_{count}");
            let prefix_len = max_len.saturating_sub(suffix.len());
            let candidate = format!("{}{suffix}", &base[..base.len().min(prefix_len)]);
            if self.seen.insert(candidate.clone()) {
                return candidate;
            }
            count += 1;
        }
    }
}

/// Rewrites a channel name as a MATLAB identifier truncated to `max_len` ASCII characters.
#[cfg(any(feature = "mat", feature = "mat4", feature = "mat73"))]
fn matlab_compatible_with_limit(name: &str, max_len: usize) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    if !out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        out.insert_str(0, "M_");
    }

    out.truncate(max_len);
    out
}

/// Splits an interleaved fixed-size array channel into one column per element.
///
/// One sequential pass over `values`, rather than one strided walk per
/// element: each per-element walk re-touches every cache line of the input
/// `elements_per_sample` times over. Shared by every exporter that flattens
/// array channels.
pub(crate) fn element_columns(values: &[f64], elements_per_sample: usize) -> Vec<Vec<f64>> {
    if elements_per_sample == 0 {
        return Vec::new();
    }
    let mut columns =
        vec![Vec::with_capacity(values.len() / elements_per_sample); elements_per_sample];
    for chunk in values.chunks_exact(elements_per_sample) {
        for (column, &value) in chunk.iter().enumerate() {
            columns[column].push(value);
        }
    }
    columns
}

/// Generates index suffixes like `"[0]"`, `"[1]"` (1-D) or `"[0][0]"`, `"[0][1]"` (2-D)
/// in row-major order for fixed-shape arrays.
pub(crate) fn array_index_suffixes(
    shape: Option<&[u64]>,
    elements_per_sample: usize,
) -> Vec<String> {
    if elements_per_sample == 0 {
        return Vec::new();
    }

    match shape {
        Some(dims)
            if dims.len() > 1 && dims.iter().product::<u64>() == elements_per_sample as u64 =>
        {
            let mut result = Vec::with_capacity(elements_per_sample);
            let mut indices = vec![0u64; dims.len()];
            for _ in 0..elements_per_sample {
                let suffix = indices
                    .iter()
                    .map(|idx| format!("[{idx}]"))
                    .collect::<String>();
                result.push(suffix);

                for d in (0..dims.len()).rev() {
                    indices[d] += 1;
                    if indices[d] < dims[d] {
                        break;
                    }
                    indices[d] = 0;
                }
            }
            result
        }
        Some(dims) if dims.len() == 1 && dims[0] == elements_per_sample as u64 => {
            (0..elements_per_sample).map(|i| format!("[{i}]")).collect()
        }
        _ => (0..elements_per_sample).map(|i| format!("[{i}]")).collect(),
    }
}

enum CsvColumnValues {
    Float(Vec<f64>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    Str(Vec<String>),
    Bytes(Vec<Vec<u8>>),
}

struct CsvColumn {
    header: String,
    group_id: usize,
    values: CsvColumnValues,
    validity: Option<Arc<Vec<bool>>>,
}

impl CsvColumn {
    fn len(&self) -> usize {
        match &self.values {
            CsvColumnValues::Float(v) => v.len(),
            CsvColumnValues::U64(v) => v.len(),
            CsvColumnValues::I64(v) => v.len(),
            CsvColumnValues::Str(v) => v.len(),
            CsvColumnValues::Bytes(v) => v.len(),
        }
    }

    fn write_cell(&self, row_index: usize, out: &mut String) {
        if let Some(validity) = &self.validity {
            if !validity.get(row_index).copied().unwrap_or(false) {
                return;
            }
        }
        match &self.values {
            CsvColumnValues::Float(v) => {
                if let Some(&val) = v.get(row_index) {
                    write_csv_float(out, val);
                }
            }
            CsvColumnValues::U64(v) => {
                if let Some(&val) = v.get(row_index) {
                    write!(out, "{val}").expect("writing to a String cannot fail");
                }
            }
            CsvColumnValues::I64(v) => {
                if let Some(&val) = v.get(row_index) {
                    write!(out, "{val}").expect("writing to a String cannot fail");
                }
            }
            CsvColumnValues::Str(v) => {
                if let Some(val) = v.get(row_index) {
                    out.push_str(&csv_field(val));
                }
            }
            CsvColumnValues::Bytes(v) => {
                if let Some(bytes) = v.get(row_index) {
                    for b in bytes {
                        write!(out, "{b:02x}").expect("writing to a String cannot fail");
                    }
                }
            }
        }
    }
}

fn write_csv_float(out: &mut String, v: f64) {
    if !v.is_finite() {
        return;
    }
    if v == 0.0 {
        out.push_str("0.000000000");
    } else if v.abs() < 1e-4 || v.abs() >= 1e15 {
        write!(out, "{v:e}").expect("writing to a String cannot fail");
    } else {
        write!(out, "{v:.9}").expect("writing to a String cannot fail");
    }
}

/// Writes `channels` to `out` as CSV: one time column taken from the first
/// channel's master, then value columns for each channel in the order given.
///
/// Complex channels are flattened into `<channel>.re` and `<channel>.im` columns.
/// CANopen date/time channels are flattened into a single timestamp column (nanoseconds).
/// Fixed-shape arrays are flattened into one column per element (`<channel>[i]`,
/// or `[i][j]` for 2-D) in row-major order. Variable-length arrays are refused by name.
///
/// With one scalar channel this is exactly the `export_to_csv` example's format —
/// `Time [unit]` (or `Index` when the group has no master), nine-decimal
/// values — so a single-channel export is byte-identical to it. With several,
/// each row carries one cell per column; a channel with fewer samples leaves
/// its trailing cells empty rather than inventing values, and channels on
/// distinct time axes are aligned chronologically along the union of their
/// timestamps, leaving empty cells where a channel has no sample or where a
/// sample is marked invalid.
///
/// An empty slice writes nothing and succeeds: nothing was asked, and nothing
/// is the truthful answer. Callers with a user interface disable the action
/// instead; this keeps the function total.
pub fn write_csv<W: Write>(file: &Mf4File, channels: &[&Channel], out: &mut W) -> Result<()> {
    if channels.is_empty() {
        return Ok(());
    }

    let mut time_axes: Vec<Option<(Vec<f64>, String)>> = Vec::new();
    let mut cg_to_group: Vec<((usize, usize), usize)> = Vec::new();
    let mut columns = Vec::new();

    for channel in channels {
        let cg_key = (channel.data_group_index, channel.channel_group_index);
        let group_id = if let Some(&(_, gid)) = cg_to_group.iter().find(|&&(k, _)| k == cg_key) {
            gid
        } else {
            let candidate_time = time_column(file, channel);
            let existing =
                time_axes
                    .iter()
                    .position(|existing| match (existing, &candidate_time) {
                        (Some((t1, _)), Some((t2, _))) => timestamps_eq(t1, t2),
                        (None, None) => true,
                        _ => false,
                    });
            let gid = match existing {
                Some(id) => id,
                None => {
                    let id = time_axes.len();
                    time_axes.push(candidate_time);
                    id
                }
            };
            cg_to_group.push((cg_key, gid));
            gid
        };

        let signal = file.signal(channel)?;
        let validity = signal.validity().map(Arc::new);
        let col_data = csv_columns_for_signal(&signal, group_id, validity)?;
        columns.extend(col_data);
    }

    let time_header = time_axes
        .iter()
        .find_map(|axis| axis.as_ref().map(|(_, unit)| format!("Time [{unit}]")))
        .unwrap_or_else(|| "Index".to_string());

    let mut headers = Vec::with_capacity(columns.len() + 1);
    headers.push(time_header);
    for col in &columns {
        headers.push(col.header.clone());
    }

    writeln!(
        out,
        "{}",
        headers
            .iter()
            .map(|h| csv_field(h))
            .collect::<Vec<_>>()
            .join(",")
    )?;

    // One row buffer reused across rows. A fresh `String` per cell plus a
    // `join` per row is two heap allocations per cell, which for a wide
    // export adds up to tens of millions of allocations.
    let mut row = String::with_capacity(16 * (columns.len() + 1));

    if time_axes.len() <= 1 {
        let time = time_axes.first().and_then(Option::as_ref);
        let row_count = columns
            .iter()
            .map(CsvColumn::len)
            .chain(time.map(|(times, _)| times.len()))
            .max()
            .unwrap_or(0);

        for row_index in 0..row_count {
            row.clear();
            match time {
                Some((times, _)) => {
                    if let Some(&t) = times.get(row_index) {
                        write_csv_float(&mut row, t);
                    }
                }
                None => write!(row, "{row_index}").expect("writing to a String cannot fail"),
            }
            for column in &columns {
                row.push(',');
                column.write_cell(row_index, &mut row);
            }
            row.push('\n');
            out.write_all(row.as_bytes())?;
        }
    } else {
        // Multi-rate / multi-CG export: align each group's samples along the
        // chronological union of all groups' timestamps so no channel is
        // paired with another group's mismatched timestamps.
        let num_groups = time_axes.len();
        let mut group_times: Vec<Vec<f64>> = Vec::with_capacity(num_groups);
        let mut group_orders: Vec<Vec<usize>> = Vec::with_capacity(num_groups);

        for (gid, axis) in time_axes.into_iter().enumerate() {
            let times = match axis {
                Some((t, _)) => t,
                None => {
                    let max_len = columns
                        .iter()
                        .filter(|c| c.group_id == gid)
                        .map(CsvColumn::len)
                        .max()
                        .unwrap_or(0);
                    (0..max_len).map(|i| i as f64).collect()
                }
            };
            let mut order: Vec<usize> = (0..times.len()).collect();
            if !times.windows(2).all(|w| w[0].total_cmp(&w[1]).is_le()) {
                order.sort_by(|&a, &b| times[a].total_cmp(&times[b]));
            }
            group_times.push(times);
            group_orders.push(order);
        }

        let mut cursors = vec![0usize; num_groups];
        let mut active_sample = vec![None; num_groups];

        loop {
            let mut t_min: Option<f64> = None;
            for gid in 0..num_groups {
                if let Some(&sample_idx) = group_orders[gid].get(cursors[gid]) {
                    let t = group_times[gid][sample_idx];
                    t_min = Some(match t_min {
                        Some(cur) if cur.total_cmp(&t).is_le() => cur,
                        _ => t,
                    });
                }
            }
            let Some(t_now) = t_min else {
                break;
            };

            for gid in 0..num_groups {
                if let Some(&sample_idx) = group_orders[gid].get(cursors[gid]) {
                    let t = group_times[gid][sample_idx];
                    if t == t_now || (t.is_nan() && t_now.is_nan()) {
                        active_sample[gid] = Some(sample_idx);
                        cursors[gid] += 1;
                    } else {
                        active_sample[gid] = None;
                    }
                } else {
                    active_sample[gid] = None;
                }
            }

            row.clear();
            write_csv_float(&mut row, t_now);
            for column in &columns {
                row.push(',');
                if let Some(sample_idx) = active_sample[column.group_id] {
                    column.write_cell(sample_idx, &mut row);
                }
            }
            row.push('\n');
            out.write_all(row.as_bytes())?;
        }
    }

    Ok(())
}

fn csv_columns_for_signal(
    signal: &Signal,
    group_id: usize,
    validity: Option<Arc<Vec<bool>>>,
) -> Result<Vec<CsvColumn>> {
    let unit = signal.unit();
    let name = signal.name();
    let shape = signal.channel.array_shape.as_deref();

    let values = signal.values()?;
    match values {
        SignalValues::Complex { re, im } => {
            let col_re = CsvColumn {
                header: column_header_with_name(&format!("{name}.re"), unit),
                group_id,
                values: CsvColumnValues::Float(re),
                validity: validity.clone(),
            };
            let col_im = CsvColumn {
                header: column_header_with_name(&format!("{name}.im"), unit),
                group_id,
                values: CsvColumnValues::Float(im),
                validity,
            };
            Ok(vec![col_re, col_im])
        }
        SignalValues::CanopenDate(v) => {
            let nanos: Vec<i64> = v.iter().map(|d| d.to_unix_nanos()).collect();
            Ok(vec![CsvColumn {
                header: column_header_with_name(name, unit),
                group_id,
                values: CsvColumnValues::I64(nanos),
                validity,
            }])
        }
        SignalValues::CanopenTime(v) => {
            let nanos: Vec<i64> = v.iter().map(|t| t.to_unix_nanos()).collect();
            Ok(vec![CsvColumn {
                header: column_header_with_name(name, unit),
                group_id,
                values: CsvColumnValues::I64(nanos),
                validity,
            }])
        }
        SignalValues::Array {
            values,
            elements_per_sample,
        } => {
            let eps = elements_per_sample;
            let suffixes = array_index_suffixes(shape, eps);
            let element_values = element_columns(&values, eps);
            let mut cols = Vec::with_capacity(eps);
            for (elem_vals, suffix) in element_values.into_iter().zip(suffixes) {
                let col_name = format!("{name}{suffix}");
                let header = column_header_with_name(&col_name, unit);
                cols.push(CsvColumn {
                    header,
                    group_id,
                    values: CsvColumnValues::Float(elem_vals),
                    validity: validity.clone(),
                });
            }
            Ok(cols)
        }
        SignalValues::ArrayVarLen { .. } => Err(Mf4Error::unsupported(
            "CSV export",
            format!(
                "channel '{name}' holds variable-length array samples, which have no fixed column shape and cannot be exported to a tabular format"
            ),
        )),
        SignalValues::U64(v) => Ok(vec![CsvColumn {
            header: column_header_with_name(name, unit),
            group_id,
            values: CsvColumnValues::U64(v),
            validity,
        }]),
        SignalValues::I64(v) => Ok(vec![CsvColumn {
            header: column_header_with_name(name, unit),
            group_id,
            values: CsvColumnValues::I64(v),
            validity,
        }]),
        SignalValues::Str(v) => Ok(vec![CsvColumn {
            header: column_header_with_name(name, unit),
            group_id,
            values: CsvColumnValues::Str(v),
            validity,
        }]),
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => {
            let len = values.len();
            let bytes = (0..len)
                .map(|i| values.bytes_at(i).unwrap_or(&[]).to_vec())
                .collect();
            Ok(vec![CsvColumn {
                header: column_header_with_name(name, unit),
                group_id,
                values: CsvColumnValues::Bytes(bytes),
                validity,
            }])
        }
        other => {
            let vals = other.to_f64();
            Ok(vec![CsvColumn {
                header: column_header_with_name(name, unit),
                group_id,
                values: CsvColumnValues::Float(vals),
                validity,
            }])
        }
    }
}

/// The time column for an export: the first channel's master, decoded, with
/// its unit for the header. `None` when the group has no master or the master
/// does not decode, in which case the export falls back to sample indices.
fn time_column(file: &Mf4File, channel: &Channel) -> Option<(Vec<f64>, String)> {
    let dg = &file.data_groups()[channel.data_group_index];
    let cg = &dg.channel_groups[channel.channel_group_index];
    let signal = file.signal(cg.master_channel()?).ok()?;
    let unit = signal.unit().to_string();
    let times = signal.values_f64().ok()?;
    Some((times, unit))
}

/// A value column's header given custom column name and unit.
fn column_header_with_name(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} [{unit}]")
    }
}

/// A value column's header: the channel's name, with its unit in brackets
/// when it has one.
#[allow(dead_code)]
fn column_header(channel: &Channel) -> String {
    column_header_with_name(&channel.name, &channel.unit)
}

/// RFC 4180 escaping: a field containing a comma, a quote or a line break is
/// wrapped in quotes with its quotes doubled. Channel names are file-supplied
/// text, and a bus-signal name like `Boost, psi` would otherwise split into
/// two columns on its way into a spreadsheet. Plain fields pass through
/// untouched, so names without special characters export exactly as before.
fn csv_field(field: &str) -> std::borrow::Cow<'_, str> {
    if field.contains([',', '"', '\n', '\r']) {
        std::borrow::Cow::Owned(format!("\"{}\"", field.replace('"', "\"\"")))
    } else {
        std::borrow::Cow::Borrowed(field)
    }
}
