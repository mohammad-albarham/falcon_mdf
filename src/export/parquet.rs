//! Apache Parquet export, via Arrow.
//!
//! # What a Parquet file made here contains
//!
//! One table. A `time` column of `double`, then one column per exported
//! channel, in the order given, named after the channel. Each column keeps the
//! channel's own type — a `uint16` channel arrives in pandas or polars as
//! `uint16`, not as a float — so a reader sees the measurement's types rather
//! than a lowest common denominator.
//!
//! # Invalid samples become nulls
//!
//! An MDF invalidation bit says "this record has no value for this channel".
//! Parquet has a way to say exactly that, so a sample whose invalidation bit is
//! set is written as null rather than as whatever bit pattern happened to sit
//! in the record. This is the one place where the exported column is not a
//! transcription of the decoded samples, and it is deliberate: writing the
//! stand-in value would present a number the measurement explicitly disclaims.
//!
//! # One table means one time axis
//!
//! Every series handed to [`write_parquet`] must carry the same timestamps.
//! Channels recorded at different rates are refused rather than resampled,
//! because resampling is a choice — which raster, which interpolation — with
//! consequences for the numbers that come out, and it is not this writer's
//! choice to make. asammdf resamples silently here; we ask the caller to say
//! what they want first:
//!
//! ```no_run
//! # use falcon_mdf::{InterpolationMode, Mf4File, Raster};
//! # use falcon_mdf::export::write_parquet;
//! # let file = Mf4File::open("measurement.mf4")?;
//! # let channels: Vec<_> = file.channels().collect();
//! // Put every channel on one 10 ms raster, then export.
//! let series = file.resample(&channels, Raster::Step(0.01), InterpolationMode::Linear)?;
//! let mut out = std::fs::File::create("measurement.parquet")?;
//! write_parquet(&series, &mut out)?;
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```
//!
//! # What it does not contain
//!
//! Variable-length array channels are refused by name, with their kind in the
//! error, because per-sample length varies and they have no fixed column shape.
//! Fixed-shape arrays are flattened into one column per element (`[i]` or `[i][j]`),
//! complex channels into `.re` and `.im` columns, and CANopen date/time channels
//! into Unix epoch nanosecond timestamps. Everything scalar — the ten integer
//! and float kinds, text, and both byte-array kinds — is written natively.

use std::io::Write;

use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::{Mf4Error, Result};
use crate::export::to_record_batch;
use crate::time_ops::SignalSeries;

/// How the written Parquet file is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParquetCompression {
    /// Snappy, Parquet's de-facto default: fast, and understood by every
    /// reader without extra packages.
    #[default]
    Snappy,
    /// No compression. Largest files, fastest write.
    None,
}

/// Writes `series` to `out` as a Parquet file with Snappy compression.
///
/// See [`write_parquet_with`] to choose the compression, and the module
/// documentation for the layout, the null mapping and the one-time-axis rule.
///
/// # Errors
///
/// Returns an error if the series do not all share one time axis, or if one of
/// them holds samples this writer does not represent.
pub fn write_parquet<W: Write + Send>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    write_parquet_with(series, out, ParquetCompression::default())
}

/// Writes `series` to `out` as a Parquet file with the given compression.
pub fn write_parquet_with<W: Write + Send>(
    series: &[SignalSeries],
    out: &mut W,
    compression: ParquetCompression,
) -> Result<()> {
    let batch = to_record_batch(series)?;

    let properties = WriterProperties::builder()
        .set_compression(match compression {
            ParquetCompression::Snappy => Compression::SNAPPY,
            ParquetCompression::None => Compression::UNCOMPRESSED,
        })
        .build();

    let mut writer = ArrowWriter::try_new(out, batch.schema(), Some(properties))
        .map_err(|e| Mf4Error::write_error(format!("could not open the Parquet writer: {e}")))?;
    writer
        .write(&batch)
        .map_err(|e| Mf4Error::write_error(format!("could not write the Parquet table: {e}")))?;
    writer.close().map_err(|e| {
        Mf4Error::write_error(format!("could not finish the Parquet file: {e}"))
    })?;

    Ok(())
}
