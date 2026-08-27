//! MATLAB v7.3 MAT-file export.
//!
//! A MAT v7.3 file is an HDF5 file with a 512-byte MATLAB userblock and a small
//! set of conventions: each variable is a root dataset named for the variable,
//! and each dataset carries a `MATLAB_class` attribute (e.g. `double`, `uint8`).
//! This module arranges the existing HDF5 writer (`src/export/hdf5.rs`) to
//! produce that layout.
//!
//! # What a MAT v7.3 file made here contains
//!
//! One numeric matrix per exported channel, plus one per distinct time axis,
//! following the same naming scheme as the level-5 MAT writer:
//!
//! | Variable | Contents |
//! |---|---|
//! | `DGM<i>_timestamps` | the time axis shared by group `i` |
//! | `DG<i>_<channel>` | one channel's samples |
//! | `DG<i>_<channel>_invalid` | that channel's invalidation mask, only when it has one |
//!
//! Every matrix is an N-by-1 column vector in MATLAB's workspace. HDF5 stores
//! arrays row-major, so on disk the same data is written with shape `[1, N]`;
//! MATLAB reads it as `[N, 1]` because it views HDF5 datasets in column-major
//! order. See the HDF5-pure `mat::dims` module for the shape translation rule.
//!
//! # What it does not contain
//!
//! Only numeric channels are written. Text, byte-array, complex, CANopen and
//! array-valued channels are refused by name, with their kind in the error,
//! rather than skipped.
//!
//! Invalidation bits are not folded into the samples. A channel that has them
//! gets a companion `_invalid` mask instead.

use std::io::Write;

use hdf5_pure::{AttrValue, FileBuilder};

use crate::error::{Mf4Error, Result};
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

/// Size of the MATLAB v7.3 userblock that precedes the HDF5 superblock.
const USERBLOCK_SIZE: u64 = 512;

/// Version tag written in the userblock at bytes 124..126 (little-endian).
const MAT73_VERSION: u16 = 0x0200;

/// Writes `series` to `out` as a MATLAB v7.3 MAT-file.
///
/// An empty slice writes a valid file containing only the userblock and an
/// empty HDF5 container.
///
/// # Errors
///
/// Returns an error for a channel whose samples are not numeric — see the
/// module documentation for why those are refused rather than skipped.
pub fn write_mat73<W: Write>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    let mut file_builder = FileBuilder::new();
    file_builder.with_userblock(USERBLOCK_SIZE);
    file_builder.with_userblock_content(&userblock());

    for (group_index, group) in time_groups(series).into_iter().enumerate() {
        let timestamps = series[group[0]].timestamps();
        write_f64_dataset(
            &mut file_builder,
            &format!("DGM{group_index}_timestamps"),
            timestamps,
        );

        for &index in &group {
            let s = &series[index];
            write_channel_dataset(
                &mut file_builder,
                &format!("DG{group_index}_{}", matlab_compatible(s.name())),
                s,
            )?;

            if let Some(validity) = s.validity() {
                let mask: Vec<u8> = validity.iter().map(|&valid| u8::from(!valid)).collect();
                write_u8_dataset(
                    &mut file_builder,
                    &format!("DG{group_index}_{}_invalid", matlab_compatible(s.name())),
                    &mask,
                );
            }
        }
    }

    let bytes = file_builder.finish().map_err(|e| {
        Mf4Error::write_error(format!("failed to serialize MAT v7.3 file: {e:?}"))
    })?;
    out.write_all(&bytes)?;
    Ok(())
}

/// MATLAB class names carried by the `MATLAB_class` attribute.
enum MatClass {
    UInt8,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Single,
    Double,
}

impl MatClass {
    fn as_str(&self) -> &'static str {
        match self {
            MatClass::UInt8 => "uint8",
            MatClass::Int8 => "int8",
            MatClass::UInt16 => "uint16",
            MatClass::Int16 => "int16",
            MatClass::UInt32 => "uint32",
            MatClass::Int32 => "int32",
            MatClass::UInt64 => "uint64",
            MatClass::Int64 => "int64",
            MatClass::Single => "single",
            MatClass::Double => "double",
        }
    }
}

/// Returns the MATLAB class for a numeric `SignalValues` kind.
fn mat_class(values: &SignalValues) -> Result<MatClass> {
    let refuse = |kind: &str| {
        Err(Mf4Error::unsupported(
            "MAT v7.3 export",
            format!(
                "channel holds {kind} samples, which a numeric MATLAB matrix cannot \
                 represent; export it to Parquet, or drop it from the selection"
            ),
        ))
    };

    Ok(match values {
        SignalValues::U8(_) => MatClass::UInt8,
        SignalValues::I8(_) => MatClass::Int8,
        SignalValues::U16(_) => MatClass::UInt16,
        SignalValues::I16(_) => MatClass::Int16,
        SignalValues::U32(_) => MatClass::UInt32,
        SignalValues::I32(_) => MatClass::Int32,
        SignalValues::U64(_) => MatClass::UInt64,
        SignalValues::I64(_) => MatClass::Int64,
        SignalValues::F32(_) => MatClass::Single,
        SignalValues::F64(_) => MatClass::Double,
        SignalValues::Str(_) => return refuse("text"),
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => return refuse("byte-array"),
        SignalValues::Complex { .. } => return refuse("complex"),
        SignalValues::CanopenDate(_) => return refuse("CANopen date"),
        SignalValues::CanopenTime(_) => return refuse("CANopen time"),
        SignalValues::Array { .. } | SignalValues::ArrayVarLen { .. } => return refuse("array"),
    })
}

/// Writes a 1-D numeric dataset at the root of the file.
///
/// MATLAB sees the dataset as an N-by-1 column vector. HDF5 stores arrays in
/// row-major order, so the on-disk shape is `[1, N]`; the byte layout is the
/// native order of the source slice because a single-row dataset is contiguous.
fn write_channel_dataset(
    file_builder: &mut FileBuilder,
    name: &str,
    s: &SignalSeries,
) -> Result<()> {
    let n = s.len().max(1) as u64;
    let ds = file_builder.create_dataset(name);
    let class = mat_class(s.values())?;

    match s.values() {
        SignalValues::F64(v) => ds.with_f64_data(v),
        SignalValues::F32(v) => ds.with_f32_data(v),
        SignalValues::I64(v) => ds.with_i64_data(v),
        SignalValues::U64(v) => ds.with_u64_data(v),
        SignalValues::I32(v) => ds.with_i32_data(v),
        SignalValues::U32(v) => ds.with_u32_data(v),
        SignalValues::I16(v) => ds.with_i16_data(v),
        SignalValues::U16(v) => ds.with_u16_data(v),
        SignalValues::I8(v) => ds.with_i8_data(v),
        SignalValues::U8(v) => ds.with_u8_data(v),
        other => {
            return Err(Mf4Error::unsupported(
                "MAT v7.3 export",
                format!(
                    "channel '{}' holds {} samples, which a numeric MATLAB matrix cannot \
                     represent; export it to Parquet, or drop it from the selection",
                    s.name(),
                    other.kind()
                ),
            ));
        }
    }
    .with_shape(&[1, n]);

    ds.set_attr("MATLAB_class", AttrValue::String(class.as_str().to_string()));
    Ok(())
}

/// Writes a 1-D `u8` dataset at the root of the file (used for invalidation masks).
fn write_u8_dataset(file_builder: &mut FileBuilder, name: &str, data: &[u8]) {
    let n = data.len().max(1) as u64;
    let ds = file_builder.create_dataset(name);
    ds.with_u8_data(data).with_shape(&[1, n]);
    ds.set_attr("MATLAB_class", AttrValue::String("uint8".to_string()));
}

/// Writes a 1-D `f64` dataset at the root of the file (used for timestamps).
fn write_f64_dataset(file_builder: &mut FileBuilder, name: &str, data: &[f64]) {
    let n = data.len().max(1) as u64;
    let ds = file_builder.create_dataset(name);
    ds.with_f64_data(data).with_shape(&[1, n]);
    ds.set_attr("MATLAB_class", AttrValue::String("double".to_string()));
}

/// Builds the 512-byte MATLAB v7.3 userblock.
///
/// Layout verified against hdf5-pure's `mat::userblock` module and the
/// MathWorks MAT-File Format documentation:
/// - `[0..124]`: description text, ASCII, space-padded
/// - `[124..126]`: version tag `0x0200` (little-endian)
/// - `[126..128]`: endian indicator `"IM"` (little-endian)
/// - `[128..512]`: zero-filled padding; HDF5 superblock follows at `[512..]`
fn userblock() -> [u8; 512] {
    let mut block = [0u8; 512];

    for b in block[..124].iter_mut() {
        *b = b' ';
    }

    let text = format!(
        "MATLAB 7.3 MAT-file, Platform: {}, Created by: falcon_mdf {}",
        std::env::consts::OS,
        env!("CARGO_PKG_VERSION")
    );
    let text_bytes = text.as_bytes();
    let len = text_bytes.len().min(124);
    block[..len].copy_from_slice(&text_bytes[..len]);

    block[124..126].copy_from_slice(&MAT73_VERSION.to_le_bytes());
    block[126] = b'I';
    block[127] = b'M';

    block
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

/// Rewrites a channel name as a MATLAB identifier.
///
/// Mirrors the level-5 MAT writer so the two exporters produce the same variable
/// names for the same channel.
fn matlab_compatible(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();

    if !out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        out.insert_str(0, "M_");
    }

    out.truncate(60);
    out
}
