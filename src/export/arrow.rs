//! Apache Arrow export and Arrow IPC file generation.
//!
//! # What an Arrow table made here contains
//!
//! One [`RecordBatch`]. A `time` column of `Float64`, then one column per exported
//! channel, in the order given, named after the channel. Each column keeps the
//! channel's own type — a `uint16` channel arrives in pandas or polars as
//! `uint16`, not as a float — so a reader sees the measurement's types rather
//! than a lowest common denominator.
//!
//! # Invalid samples become nulls
//!
//! An MDF invalidation bit says "this record has no value for this channel".
//! Arrow has a way to say exactly that, so a sample whose invalidation bit is
//! set is written as null rather than as whatever bit pattern happened to sit
//! in the record. This is the one place where the exported column is not a
//! transcription of the decoded samples, and it is deliberate: writing the
//! stand-in value would present a number the measurement explicitly disclaims.
//!
//! # One table means one time axis
//!
//! Every series handed to [`to_record_batch`] or [`write_arrow_ipc`] must carry
//! the same timestamps. Channels recorded at different rates are refused rather
//! than resampled, because resampling is a choice — which raster, which
//! interpolation — with consequences for the numbers that come out, and it is
//! not this writer's choice to make. Callers resample explicitly first:
//!
//! ```no_run
//! # use falcon_mdf::{InterpolationMode, Mf4File, Raster};
//! # use falcon_mdf::export::write_arrow_ipc;
//! # let file = Mf4File::open("measurement.mf4")?;
//! # let channels: Vec<_> = file.channels().collect();
//! // Put every channel on one 10 ms raster, then export.
//! let series = file.resample(&channels, Raster::Step(0.01), InterpolationMode::Linear)?;
//! let mut out = std::fs::File::create("measurement.arrow")?;
//! write_arrow_ipc(&series, &mut out)?;
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```
//!
//! # What it does not contain
//!
//! Complex, CANopen and array-valued channels are refused by name, with their
//! kind in the error. Arrow can express all three, but each needs its own
//! nested layout and its own proof that a reader gets the same numbers back;
//! claiming support without that proof is how an exporter comes to disagree
//! with the file it read. Everything scalar — the ten integer and float kinds,
//! text, and both byte-array kinds — is written natively.

use std::io::Write;
use std::sync::Arc;

use arrow_array::builder::{BinaryBuilder, StringBuilder};
use arrow_array::{
    ArrayRef, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
    RecordBatch, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};

use crate::error::{Mf4Error, Result};
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

/// Converts `series` into an Arrow [`RecordBatch`].
///
/// See the module documentation for the table layout, the null mapping and the
/// one-time-axis rule.
///
/// # Errors
///
/// Returns an error if the series do not all share one time axis, or if one of
/// them holds samples this writer does not represent.
pub fn to_record_batch(series: &[SignalSeries]) -> Result<RecordBatch> {
    let timestamps = shared_timestamps(series)?;

    let mut fields = Vec::with_capacity(series.len() + 1);
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(series.len() + 1);

    fields.push(Field::new("time", DataType::Float64, false));
    columns.push(Arc::new(Float64Array::from(timestamps.to_vec())));

    for s in series {
        let column = column_for(s)?;
        // Nullable exactly when the channel carries invalidation bits, so the
        // schema itself records whether the measurement could disclaim a
        // sample — a reader can tell "never invalid" from "invalid nowhere in
        // this file".
        fields.push(Field::new(
            s.name(),
            column.data_type().clone(),
            s.validity().is_some(),
        ));
        columns.push(column);
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, columns)
        .map_err(|e| Mf4Error::write_error(format!("could not assemble the Arrow table: {e}")))
}

/// Writes `series` to `out` as an Arrow IPC file format stream.
///
/// This format is directly readable by `pandas.read_feather`, `polars.read_ipc`,
/// and `pyarrow.ipc.open_file`.
///
/// # Errors
///
/// Returns an error if the series do not all share one time axis, if one of
/// them holds samples this writer does not represent, or if writing fails.
pub fn write_arrow_ipc<W: Write>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    let batch = to_record_batch(series)?;
    let mut writer = FileWriter::try_new(out, &batch.schema())
        .map_err(|e| Mf4Error::write_error(format!("could not open the Arrow IPC writer: {e}")))?;
    writer
        .write(&batch)
        .map_err(|e| Mf4Error::write_error(format!("could not write the Arrow IPC table: {e}")))?;
    writer
        .finish()
        .map_err(|e| Mf4Error::write_error(format!("could not finish the Arrow IPC file: {e}")))?;
    Ok(())
}

/// The one time axis every series shares, or an error naming the first that
/// does not.
///
/// An empty selection yields an empty axis, so exporting nothing writes a table
/// with a `time` column and no rows rather than failing.
fn shared_timestamps(series: &[SignalSeries]) -> Result<&[f64]> {
    let Some(first) = series.first() else {
        return Ok(&[]);
    };
    for s in &series[1..] {
        if s.timestamps() != first.timestamps() {
            return Err(Mf4Error::write_error(format!(
                "Arrow export needs one time axis for the whole table, but '{}' has {} \
                 samples and '{}' has {}; put them on a common raster with `Mf4File::resample` \
                 first",
                first.name(),
                first.len(),
                s.name(),
                s.len()
            )));
        }
    }
    Ok(first.timestamps())
}

/// Builds one Arrow column from a series, with invalid samples as nulls.
fn column_for(series: &SignalSeries) -> Result<ArrayRef> {
    // `None` for every sample the channel disclaims, `Some` otherwise. Built
    // once here so each arm below reads the same way.
    let keep = |i: usize| series.validity().is_none_or(|v| v[i]);

    macro_rules! numeric {
        ($array:ty, $values:expr) => {{
            let taken: Vec<Option<_>> = $values
                .iter()
                .enumerate()
                .map(|(i, &x)| keep(i).then_some(x))
                .collect();
            Arc::new(<$array>::from(taken)) as ArrayRef
        }};
    }

    let refuse = |kind: &str| {
        Err(Mf4Error::unsupported(
            "Arrow export",
            format!(
                "channel '{}' holds {kind} samples, which this writer does not represent",
                series.name()
            ),
        ))
    };

    Ok(match series.values() {
        SignalValues::U8(v) => numeric!(UInt8Array, v),
        SignalValues::U16(v) => numeric!(UInt16Array, v),
        SignalValues::U32(v) => numeric!(UInt32Array, v),
        SignalValues::U64(v) => numeric!(UInt64Array, v),
        SignalValues::I8(v) => numeric!(Int8Array, v),
        SignalValues::I16(v) => numeric!(Int16Array, v),
        SignalValues::I32(v) => numeric!(Int32Array, v),
        SignalValues::I64(v) => numeric!(Int64Array, v),
        SignalValues::F32(v) => numeric!(Float32Array, v),
        SignalValues::F64(v) => numeric!(Float64Array, v),
        SignalValues::Str(v) => {
            let mut builder = StringBuilder::new();
            for (i, s) in v.iter().enumerate() {
                if keep(i) {
                    builder.append_value(s);
                } else {
                    builder.append_null();
                }
            }
            Arc::new(builder.finish()) as ArrayRef
        }
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => {
            let mut builder = BinaryBuilder::new();
            for i in 0..series.len() {
                match series.values().bytes_at(i) {
                    Some(bytes) if keep(i) => builder.append_value(bytes),
                    // A sample the channel disclaims, and — defensively — one
                    // the accessor cannot address. Both are "no value here",
                    // which is what null means; neither invents bytes.
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish()) as ArrayRef
        }
        SignalValues::Complex { .. } => return refuse("complex"),
        SignalValues::CanopenDate(_) => return refuse("CANopen date"),
        SignalValues::CanopenTime(_) => return refuse("CANopen time"),
        SignalValues::Array { .. } | SignalValues::ArrayVarLen { .. } => return refuse("array"),
    })
}
