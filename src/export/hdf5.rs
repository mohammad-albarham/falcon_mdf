//! HDF5 file export.
//!
//! # What an HDF5 file made here contains
//!
//! One HDF5 dataset per exported channel, plus one per distinct time axis.
//! Channels are written in their own native numeric types — `u8`, `u16`, `u32`,
//! `u64`, `i8`, `i16`, `i32`, `i64`, `f32`, `f64` — so an analysis pipeline
//! using `h5py` receives the exact precision and width stored in the measurement file.
//!
//! Each channel dataset carries metadata attributes:
//!
//! | Attribute | Type | Description |
//! |---|---|---|
//! | `unit` | string | Physical measurement unit (e.g. `"km/h"`, `"rpm"`) |
//! | `comment` | string | Channel comment/description, when present |
//!
//! # Multi-axis grouping
//!
//! Series are grouped by their time axis:
//! - When all series share a single time vector, datasets are stored directly in
//!   the root group (`timestamps`, `<channel>`, `<channel>_invalid`).
//! - When series span multiple time axes, each distinct axis produces a
//!   `ChannelGroup_<i>` group containing `timestamps`, `<channel>`, and `<channel>_invalid`.
//!
//! # Invalidation masks
//!
//! Invalidation bits are not folded into samples: a channel that carries them
//! receives a companion `_invalid` dataset of `u8` (1 for invalid, 0 for valid),
//! preserving complete data fidelity without altering sample values.

use std::io::Write;

use hdf5_pure::{AttrValue, FileBuilder};

use crate::error::{Mf4Error, Result};
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

/// Writes `series` to `out` as an HDF5 file.
///
/// An empty slice produces a valid empty HDF5 container.
///
/// # Errors
///
/// Returns an error if any series contains non-numeric data that HDF5 numeric
/// datasets cannot represent, naming the channel and its unsupported kind.
pub fn write_hdf5<W: Write>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    // Refuse non-numeric kinds upfront before writing anything
    for s in series {
        match s.values() {
            SignalValues::U8(_)
            | SignalValues::U16(_)
            | SignalValues::U32(_)
            | SignalValues::U64(_)
            | SignalValues::I8(_)
            | SignalValues::I16(_)
            | SignalValues::I32(_)
            | SignalValues::I64(_)
            | SignalValues::F32(_)
            | SignalValues::F64(_) => {}
            other => {
                return Err(Mf4Error::unsupported(
                    "HDF5 export",
                    format!(
                        "channel '{}' holds {} samples, which a numeric HDF5 dataset cannot \
                         represent; export it to Parquet, or drop it from the selection",
                        s.name(),
                        other.kind()
                    ),
                ));
            }
        }
    }

    let groups = time_groups(series);
    let mut file_builder = FileBuilder::new();

    if groups.len() <= 1 {
        // Flat root group layout for single-time-base files
        if let Some(indices) = groups.first() {
            let first = &series[indices[0]];
            file_builder
                .create_dataset("timestamps")
                .with_f64_data(first.timestamps())
                .with_shape(&[first.len() as u64]);

            for &idx in indices {
                let s = &series[idx];
                write_channel_dataset(file_builder.create_dataset(s.name()), s)?;

                if let Some(validity) = s.validity() {
                    let invalid_bytes: Vec<u8> =
                        validity.iter().map(|&valid| u8::from(!valid)).collect();
                    let invalid_name = format!("{}_invalid", s.name());
                    file_builder
                        .create_dataset(&invalid_name)
                        .with_u8_data(&invalid_bytes)
                        .with_shape(&[invalid_bytes.len() as u64]);
                }
            }
        }
    } else {
        // Multi-group hierarchical layout
        for (i, indices) in groups.iter().enumerate() {
            let group_name = format!("ChannelGroup_{i}");
            let mut group_builder = file_builder.create_group(&group_name);

            let first = &series[indices[0]];
            group_builder
                .create_dataset("timestamps")
                .with_f64_data(first.timestamps())
                .with_shape(&[first.len() as u64]);

            for &idx in indices {
                let s = &series[idx];
                write_channel_dataset(group_builder.create_dataset(s.name()), s)?;

                if let Some(validity) = s.validity() {
                    let invalid_bytes: Vec<u8> =
                        validity.iter().map(|&valid| u8::from(!valid)).collect();
                    let invalid_name = format!("{}_invalid", s.name());
                    group_builder
                        .create_dataset(&invalid_name)
                        .with_u8_data(&invalid_bytes)
                        .with_shape(&[invalid_bytes.len() as u64]);
                }
            }

            let finished = group_builder.finish();
            file_builder.add_group(finished);
        }
    }

    let bytes = file_builder.finish().map_err(|e| {
        Mf4Error::write_error(format!("failed to serialize HDF5 file: {e:?}"))
    })?;

    out.write_all(&bytes)?;
    Ok(())
}

fn write_channel_dataset(
    ds: &mut hdf5_pure::DatasetBuilder,
    s: &SignalSeries,
) -> Result<()> {
    match s.values() {
        SignalValues::F64(v) => {
            ds.with_f64_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::F32(v) => {
            ds.with_f32_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::I64(v) => {
            ds.with_i64_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::U64(v) => {
            ds.with_u64_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::I32(v) => {
            ds.with_i32_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::U32(v) => {
            ds.with_u32_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::I16(v) => {
            ds.with_i16_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::U16(v) => {
            ds.with_u16_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::I8(v) => {
            ds.with_i8_data(v).with_shape(&[v.len() as u64]);
        }
        SignalValues::U8(v) => {
            ds.with_u8_data(v).with_shape(&[v.len() as u64]);
        }
        other => {
            return Err(Mf4Error::unsupported(
                "HDF5 export",
                format!(
                    "channel '{}' holds {} samples, which a numeric HDF5 dataset cannot \
                     represent; export it to Parquet, or drop it from the selection",
                    s.name(),
                    other.kind()
                ),
            ));
        }
    }

    if !s.unit().is_empty() {
        ds.set_attr("unit", AttrValue::String(s.unit().to_string()));
    }
    if !s.channel.comment.is_empty() {
        ds.set_attr("comment", AttrValue::String(s.channel.comment.clone()));
    }

    Ok(())
}

/// Groups series by their time axis, in first-appearance order.
fn time_groups(series: &[SignalSeries]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, s) in series.iter().enumerate() {
        match groups
            .iter_mut()
            .find(|group| series[group[0]].timestamps() == s.timestamps())
        {
            Some(group) => group.push(index),
            None => groups.push(vec![index]),
        }
    }
    groups
}
