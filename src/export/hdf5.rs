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
use crate::export::array_index_suffixes;
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

enum DatasetData<'a> {
    U8(&'a [u8]),
    I8(&'a [i8]),
    U16(&'a [u16]),
    I16(&'a [i16]),
    U32(&'a [u32]),
    I32(&'a [i32]),
    U64(&'a [u64]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
    OwnedI64(Vec<i64>),
    OwnedF64(Vec<f64>),
}

struct FlattenedHdf5<'a> {
    name: String,
    data: DatasetData<'a>,
    unit: &'a str,
    comment: &'a str,
    invalid_mask: Option<Vec<u8>>,
}

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
            | SignalValues::F64(_)
            | SignalValues::Complex { .. }
            | SignalValues::CanopenDate(_)
            | SignalValues::CanopenTime(_)
            | SignalValues::Array { .. } => {}
            SignalValues::ArrayVarLen { .. } => {
                return Err(Mf4Error::unsupported(
                    "HDF5 export",
                    format!(
                        "channel '{}' holds variable-length array samples, which have no fixed column shape and cannot be exported to a tabular format",
                        s.name()
                    ),
                ));
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
                let datasets = flatten_for_hdf5(s)?;
                for ds_item in datasets {
                    let ds = file_builder.create_dataset(&ds_item.name);
                    write_dataset(ds, &ds_item.data, ds_item.unit, ds_item.comment);

                    if let Some(mask) = ds_item.invalid_mask {
                        let invalid_name = format!("{}_invalid", ds_item.name);
                        let ds_inv = file_builder.create_dataset(&invalid_name);
                        ds_inv
                            .with_u8_data(&mask)
                            .with_shape(&[mask.len() as u64]);
                    }
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
                let datasets = flatten_for_hdf5(s)?;
                for ds_item in datasets {
                    let ds = group_builder.create_dataset(&ds_item.name);
                    write_dataset(ds, &ds_item.data, ds_item.unit, ds_item.comment);

                    if let Some(mask) = ds_item.invalid_mask {
                        let invalid_name = format!("{}_invalid", ds_item.name);
                        let ds_inv = group_builder.create_dataset(&invalid_name);
                        ds_inv
                            .with_u8_data(&mask)
                            .with_shape(&[mask.len() as u64]);
                    }
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

fn flatten_for_hdf5<'a>(series: &'a SignalSeries) -> Result<Vec<FlattenedHdf5<'a>>> {
    let unit = series.unit();
    let comment = series.channel.comment.as_str();
    let invalid_mask = series
        .validity()
        .map(|v| v.iter().map(|&valid| u8::from(!valid)).collect::<Vec<u8>>());

    Ok(match series.values() {
        SignalValues::U8(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::U8(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::I8(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::I8(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::U16(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::U16(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::I16(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::I16(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::U32(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::U32(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::I32(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::I32(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::U64(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::U64(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::I64(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::I64(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::F32(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::F32(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::F64(v) => vec![FlattenedHdf5 {
            name: series.name().to_string(),
            data: DatasetData::F64(v),
            unit,
            comment,
            invalid_mask,
        }],
        SignalValues::Complex { re, im } => vec![
            FlattenedHdf5 {
                name: format!("{}.re", series.name()),
                data: DatasetData::F64(re),
                unit,
                comment,
                invalid_mask: invalid_mask.clone(),
            },
            FlattenedHdf5 {
                name: format!("{}.im", series.name()),
                data: DatasetData::F64(im),
                unit,
                comment,
                invalid_mask,
            },
        ],
        SignalValues::CanopenDate(v) => {
            let nanos: Vec<i64> = v.iter().map(|d| d.to_unix_nanos()).collect();
            vec![FlattenedHdf5 {
                name: series.name().to_string(),
                data: DatasetData::OwnedI64(nanos),
                unit,
                comment,
                invalid_mask,
            }]
        }
        SignalValues::CanopenTime(v) => {
            let nanos: Vec<i64> = v.iter().map(|t| t.to_unix_nanos()).collect();
            vec![FlattenedHdf5 {
                name: series.name().to_string(),
                data: DatasetData::OwnedI64(nanos),
                unit,
                comment,
                invalid_mask,
            }]
        }
        SignalValues::Array {
            values,
            elements_per_sample,
        } => {
            let n = series.len();
            let eps = *elements_per_sample;
            let suffixes = array_index_suffixes(series.channel.array_shape.as_deref(), eps);
            let mut list = Vec::with_capacity(eps);
            for (elem_idx, suffix) in suffixes.into_iter().enumerate() {
                let name = format!("{}{suffix}", series.name());
                let elem_vals: Vec<f64> = (0..n).map(|i| values[i * eps + elem_idx]).collect();
                list.push(FlattenedHdf5 {
                    name,
                    data: DatasetData::OwnedF64(elem_vals),
                    unit,
                    comment,
                    invalid_mask: invalid_mask.clone(),
                });
            }
            list
        }
        SignalValues::ArrayVarLen { .. } => {
            return Err(Mf4Error::unsupported(
                "HDF5 export",
                format!(
                    "channel '{}' holds variable-length array samples, which have no fixed column shape and cannot be exported to a tabular format",
                    series.name()
                ),
            ));
        }
        other => {
            return Err(Mf4Error::unsupported(
                "HDF5 export",
                format!(
                    "channel '{}' holds {} samples, which a numeric HDF5 dataset cannot \
                     represent; export it to Parquet, or drop it from the selection",
                    series.name(),
                    other.kind()
                ),
            ));
        }
    })
}

fn write_dataset(
    ds: &mut hdf5_pure::DatasetBuilder,
    data: &DatasetData<'_>,
    unit: &str,
    comment: &str,
) {
    match data {
        DatasetData::U8(v) => {
            ds.with_u8_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::I8(v) => {
            ds.with_i8_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::U16(v) => {
            ds.with_u16_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::I16(v) => {
            ds.with_i16_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::U32(v) => {
            ds.with_u32_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::I32(v) => {
            ds.with_i32_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::U64(v) => {
            ds.with_u64_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::I64(v) => {
            ds.with_i64_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::F32(v) => {
            ds.with_f32_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::F64(v) => {
            ds.with_f64_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::OwnedI64(v) => {
            ds.with_i64_data(v).with_shape(&[v.len() as u64]);
        }
        DatasetData::OwnedF64(v) => {
            ds.with_f64_data(v).with_shape(&[v.len() as u64]);
        }
    }

    if !unit.is_empty() {
        ds.set_attr("unit", AttrValue::String(unit.to_string()));
    }
    if !comment.is_empty() {
        ds.set_attr("comment", AttrValue::String(comment.to_string()));
    }
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
