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

use std::io::Write;

use crate::error::{Mf4Error, Result};
use crate::model::{Channel, Signal, SignalValues};
use crate::Mf4File;

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
/// its trailing cells empty rather than inventing values, and rows past the
/// time column's last timestamp leave the time cell empty.
///
/// An empty slice writes nothing and succeeds: nothing was asked, and nothing
/// is the truthful answer. Callers with a user interface disable the action
/// instead; this keeps the function total.
pub fn write_csv<W: Write>(file: &Mf4File, channels: &[&Channel], out: &mut W) -> Result<()> {
    let Some(first) = channels.first() else {
        return Ok(());
    };

    let time = time_column(file, first);

    let mut headers = Vec::new();
    headers.push(match &time {
        Some((_, unit)) => format!("Time [{unit}]"),
        None => "Index".to_string(),
    });

    let mut columns = Vec::new();
    for channel in channels {
        let signal = file.signal(channel)?;
        let col_data = csv_columns_for_signal(&signal)?;
        for (header, values) in col_data {
            headers.push(header);
            columns.push(values);
        }
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

    let row_count = columns
        .iter()
        .map(Vec::len)
        .chain(time.as_ref().map(|(times, _)| times.len()))
        .max()
        .unwrap_or(0);

    for row in 0..row_count {
        let mut cells = Vec::with_capacity(columns.len() + 1);
        cells.push(match &time {
            Some((times, _)) => times
                .get(row)
                .map(|t| format!("{t:.9}"))
                .unwrap_or_default(),
            None => format!("{row}"),
        });
        for column in &columns {
            cells.push(
                column
                    .get(row)
                    .map(|value| format!("{value:.9}"))
                    .unwrap_or_default(),
            );
        }
        writeln!(out, "{}", cells.join(","))?;
    }

    Ok(())
}

fn csv_columns_for_signal(signal: &Signal) -> Result<Vec<(String, Vec<f64>)>> {
    let unit = signal.unit();
    let name = signal.name();
    let shape = signal.channel.array_shape.as_deref();

    let values = signal.values()?;
    match values {
        SignalValues::Complex { re, im } => {
            let col_re = (
                column_header_with_name(&format!("{name}.re"), unit),
                re,
            );
            let col_im = (
                column_header_with_name(&format!("{name}.im"), unit),
                im,
            );
            Ok(vec![col_re, col_im])
        }
        SignalValues::CanopenDate(v) => {
            let nanos: Vec<f64> = v.iter().map(|d| d.to_unix_nanos() as f64).collect();
            Ok(vec![(column_header_with_name(name, unit), nanos)])
        }
        SignalValues::CanopenTime(v) => {
            let nanos: Vec<f64> = v.iter().map(|t| t.to_unix_nanos() as f64).collect();
            Ok(vec![(column_header_with_name(name, unit), nanos)])
        }
        SignalValues::Array {
            values,
            elements_per_sample,
        } => {
            let n = signal.len();
            let eps = elements_per_sample;
            let suffixes = array_index_suffixes(shape, eps);
            let mut cols = Vec::with_capacity(eps);
            for (elem_idx, suffix) in suffixes.into_iter().enumerate() {
                let col_name = format!("{name}{suffix}");
                let header = column_header_with_name(&col_name, unit);
                let elem_vals: Vec<f64> = (0..n).map(|i| values[i * eps + elem_idx]).collect();
                cols.push((header, elem_vals));
            }
            Ok(cols)
        }
        SignalValues::ArrayVarLen { .. } => Err(Mf4Error::unsupported(
            "CSV export",
            format!(
                "channel '{name}' holds variable-length array samples, which have no fixed column shape and cannot be exported to a tabular format"
            ),
        )),
        _ => {
            let vals = signal.values_f64()?;
            Ok(vec![(column_header_with_name(name, unit), vals)])
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
