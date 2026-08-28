//! MATLAB version 4 (Level 4) MAT-file export.
//!
//! Written directly against the MAT-File Format Level 4 specification.
//! The MAT v4 container is a flat sequence of records without a file-level header
//! or compression. Each record consists of a 20-byte header, followed by the
//! variable name (NUL-terminated ASCII), and the matrix data in column-major order.
//!
//! # The MAT v4 Record Header
//!
//! The 20-byte header is five 32-bit little-endian integers:
//! - `type`: decimal integer `MOPT`
//!   - `M`: numeric format / byte order (`0` for IEEE little-endian)
//!   - `O`: reserved, always `0`
//!   - `P`: precision (`0` = double, `1` = single/float, `2` = int32, `3` = int16, `4` = uint16, `5` = uint8)
//!   - `T`: matrix type (`0` = numeric full matrix, `1` = text matrix, `2` = sparse matrix)
//! - `mrows`: number of rows
//! - `ncols`: number of columns
//! - `imagf`: imaginary flag (`0` for real data, `1` for complex)
//! - `namlen`: length of variable name plus 1 (for the NUL terminator)
//!
//! # What a MAT v4 file made here contains
//!
//! One matrix per exported channel, plus one per distinct time axis.
//! Names follow the same scheme as the Level 5 and v7.3 exporters:
//!
//! | Variable | Contents |
//! |---|---|
//! | `DGM<i>_timestamps` | the time axis shared by group `i` |
//! | `DG<i>_<channel>` | one channel's samples |
//! | `DG<i>_<channel>_invalid` | that channel's invalidation mask, only when it has one |
//!
//! Series are grouped by their time axis: series `0` establishes group 0, and
//! any later series with an identical time vector joins it rather than repeating
//! the vector. Numeric time series are written as N-by-1 column vectors.
//! Text channels are written as N-by-L 2-D character matrices in column-major order.
//!
//! # Composite Channels and Unsupported Kinds
//!
//! Fixed-shape arrays are flattened into elements (`<channel>[i]`), complex channels
//! into `.re` and `.im` columns, and CANopen date/time channels into absolute
//! nanosecond timestamp floating-point values. Variable-length arrays and opaque byte
//! arrays are refused by name.

use std::collections::HashMap;
use std::io::Write;

use crate::error::{Mf4Error, Result};
use crate::export::array_index_suffixes;
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

// MAT v4 precision codes (P in MOPT).
const MI_DOUBLE: u32 = 0;
const MI_SINGLE: u32 = 1;
const MI_INT32: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_UINT8: u32 = 5;

// MAT v4 matrix types (T in MOPT).
const MX_FULL_CLASS: u32 = 0;
const MX_CHAR_CLASS: u32 = 1;

/// Serialises `v` as little-endian bytes.
macro_rules! le_bytes {
    ($v:expr) => {
        $v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
    };
}

struct FlattenedMatV4 {
    name: String,
    precision: u32,
    matrix_type: u32,
    rows: usize,
    cols: usize,
    data: Vec<u8>,
}

/// Writes `series` to `out` as a MATLAB version 4 MAT-file.
///
/// An empty slice writes a valid, 0-byte MAT-file.
///
/// # Errors
///
/// Returns an error for a channel whose samples are unrepresentable (e.g. byte arrays
/// or variable-length arrays) — see the module documentation for details.
///
/// # Example
///
/// ```no_run
/// # use falcon_mdf::Mf4File;
/// # use falcon_mdf::export::write_mat_v4;
/// let file = Mf4File::open("measurement.mf4")?;
/// let series = file.filter(&["Speed".into(), "RPM".into()])?;
/// let mut out = std::fs::File::create("measurement_v4.mat")?;
/// write_mat_v4(&series, &mut out)?;
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub fn write_mat_v4<W: Write>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    let mut names = UniqueNames::default();
    for (group_index, group) in time_groups(series).into_iter().enumerate() {
        let timestamps = series[group[0]].timestamps();
        write_matrix(
            out,
            &names.claim(&format!("DGM{group_index}_timestamps")),
            MI_DOUBLE,
            MX_FULL_CLASS,
            timestamps.len(),
            1,
            0,
            &le_bytes!(timestamps),
        )?;

        for &index in &group {
            let s = &series[index];
            let mats = flatten_for_mat_v4(s)?;
            for item in mats {
                write_matrix(
                    out,
                    &names.claim(&format!("DG{group_index}_{}", item.name)),
                    item.precision,
                    item.matrix_type,
                    item.rows,
                    item.cols,
                    0,
                    &item.data,
                )?;

                // Only when the channel actually carries invalidation bits: an
                // all-valid mask beside every channel would be noise, and the
                // absence of the variable is itself the statement that the file
                // recorded no invalidation for this channel.
                if let Some(validity) = s.validity() {
                    let mask: Vec<u8> = validity.iter().map(|&valid| u8::from(!valid)).collect();
                    write_matrix(
                        out,
                        &names.claim(&format!("DG{group_index}_{}_invalid", item.name)),
                        MI_UINT8,
                        MX_FULL_CLASS,
                        mask.len(),
                        1,
                        0,
                        &mask,
                    )?;
                }
            }
        }
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

/// Flattens one series into MATLAB v4 matrix variables, handling composites.
fn flatten_for_mat_v4(series: &SignalSeries) -> Result<Vec<FlattenedMatV4>> {
    let refuse = |kind: &str| {
        Err(Mf4Error::unsupported(
            "MAT v4 export",
            format!(
                "channel '{}' holds {kind} samples, which a numeric or text MATLAB matrix cannot \
                 represent; export it to Parquet, or drop it from the selection",
                series.name()
            ),
        ))
    };

    Ok(match series.values() {
        SignalValues::U8(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_UINT8,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: v.clone(),
        }],
        SignalValues::I8(v) => {
            // MAT v4 has no signed 8-bit integer type, so promote to double.
            let vals: Vec<f64> = v.iter().map(|&x| x as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: v.len(),
                cols: 1,
                data: le_bytes!(vals),
            }]
        }
        SignalValues::U16(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_UINT16,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: le_bytes!(v),
        }],
        SignalValues::I16(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_INT16,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: le_bytes!(v),
        }],
        SignalValues::U32(v) => {
            // MAT v4 has no unsigned 32-bit integer type (only int32), promote to double.
            let vals: Vec<f64> = v.iter().map(|&x| x as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: v.len(),
                cols: 1,
                data: le_bytes!(vals),
            }]
        }
        SignalValues::I32(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_INT32,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: le_bytes!(v),
        }],
        SignalValues::U64(v) => {
            // MAT v4 has no 64-bit integer type, promote to double.
            let vals: Vec<f64> = v.iter().map(|&x| x as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: v.len(),
                cols: 1,
                data: le_bytes!(vals),
            }]
        }
        SignalValues::I64(v) => {
            // MAT v4 has no 64-bit integer type, promote to double.
            let vals: Vec<f64> = v.iter().map(|&x| x as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: v.len(),
                cols: 1,
                data: le_bytes!(vals),
            }]
        }
        SignalValues::F32(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_SINGLE,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: le_bytes!(v),
        }],
        SignalValues::F64(v) => vec![FlattenedMatV4 {
            name: series.name().to_string(),
            precision: MI_DOUBLE,
            matrix_type: MX_FULL_CLASS,
            rows: v.len(),
            cols: 1,
            data: le_bytes!(v),
        }],
        SignalValues::Complex { re, im } => vec![
            FlattenedMatV4 {
                name: format!("{}.re", series.name()),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: re.len(),
                cols: 1,
                data: le_bytes!(re),
            },
            FlattenedMatV4 {
                name: format!("{}.im", series.name()),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: im.len(),
                cols: 1,
                data: le_bytes!(im),
            },
        ],
        SignalValues::CanopenDate(v) => {
            let nanos: Vec<f64> = v.iter().map(|d| d.to_unix_nanos() as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: nanos.len(),
                cols: 1,
                data: le_bytes!(nanos),
            }]
        }
        SignalValues::CanopenTime(v) => {
            let nanos: Vec<f64> = v.iter().map(|t| t.to_unix_nanos() as f64).collect();
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_DOUBLE,
                matrix_type: MX_FULL_CLASS,
                rows: nanos.len(),
                cols: 1,
                data: le_bytes!(nanos),
            }]
        }
        SignalValues::Array {
            values,
            elements_per_sample,
        } => {
            let n = series.len();
            let eps = *elements_per_sample;
            let suffixes = array_index_suffixes(series.channel.array_shape.as_deref(), eps);
            let mut mats = Vec::with_capacity(eps);
            for (elem_idx, suffix) in suffixes.into_iter().enumerate() {
                let elem_vals: Vec<f64> = (0..n).map(|i| values[i * eps + elem_idx]).collect();
                mats.push(FlattenedMatV4 {
                    name: format!("{}{suffix}", series.name()),
                    precision: MI_DOUBLE,
                    matrix_type: MX_FULL_CLASS,
                    rows: n,
                    cols: 1,
                    data: le_bytes!(elem_vals),
                });
            }
            mats
        }
        SignalValues::ArrayVarLen { .. } => {
            return Err(Mf4Error::unsupported(
                "MAT v4 export",
                format!(
                    "channel '{}' holds variable-length array samples, which have no fixed column shape and cannot be exported to a tabular format",
                    series.name()
                ),
            ));
        }
        SignalValues::Str(v) => {
            let rows = v.len();
            let max_len = v.iter().map(|s| s.len()).max().unwrap_or(0);
            let cols = max_len;
            let mut data = Vec::with_capacity(rows * cols);
            // Column-major order: outer loop column, inner loop row.
            // Shorter strings are space-padded up to max_len.
            for col in 0..cols {
                for s in v {
                    let b = s.as_bytes().get(col).copied().unwrap_or(b' ');
                    data.push(b);
                }
            }
            vec![FlattenedMatV4 {
                name: series.name().to_string(),
                precision: MI_UINT8,
                matrix_type: MX_CHAR_CLASS,
                rows,
                cols,
                data,
            }]
        }
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => return refuse("byte-array"),
    })
}

/// Writes one MAT v4 matrix element.
#[allow(clippy::too_many_arguments)]
fn write_matrix<W: Write>(
    out: &mut W,
    name: &str,
    precision: u32,
    matrix_type: u32,
    rows: usize,
    cols: usize,
    imagf: u32,
    data: &[u8],
) -> Result<()> {
    let rows_i32 = i32::try_from(rows).map_err(|_| {
        Mf4Error::write_error(format!(
            "channel '{name}' has more samples than a MAT v4 dimension can hold"
        ))
    })?;
    let cols_i32 = i32::try_from(cols).map_err(|_| {
        Mf4Error::write_error(format!(
            "channel '{name}' has more columns than a MAT v4 dimension can hold"
        ))
    })?;

    // MOPT encoding: M=0 (IEEE Little Endian), O=0, P=precision, T=matrix_type
    let type_val: i32 = (precision * 10 + matrix_type) as i32;
    let namlen_i32 = i32::try_from(name.len() + 1).map_err(|_| {
        Mf4Error::write_error(format!("variable name '{name}' is too long for MAT v4"))
    })?;

    let mut header = [0u8; 20];
    header[0..4].copy_from_slice(&type_val.to_le_bytes());
    header[4..8].copy_from_slice(&rows_i32.to_le_bytes());
    header[8..12].copy_from_slice(&cols_i32.to_le_bytes());
    header[12..16].copy_from_slice(&(imagf as i32).to_le_bytes());
    header[16..20].copy_from_slice(&namlen_i32.to_le_bytes());

    out.write_all(&header)?;
    out.write_all(name.as_bytes())?;
    out.write_all(b"\0")?;
    out.write_all(data)?;
    Ok(())
}

/// Hands out MATLAB-legal variable names, never the same one twice.
#[derive(Default)]
struct UniqueNames {
    seen: HashMap<String, usize>,
}

impl UniqueNames {
    fn claim(&mut self, name: &str) -> String {
        let base = matlab_compatible(name);
        match self.seen.get_mut(&base) {
            None => {
                self.seen.insert(base.clone(), 0);
                base
            }
            Some(count) => {
                *count += 1;
                format!("{base}_{count}")
            }
        }
    }
}

/// Rewrites a channel name as a MATLAB identifier.
///
/// Anything outside `[A-Za-z0-9_]` becomes an underscore, a name not starting
/// with a letter gains an `M_` prefix, and the result is cut to 60 characters —
/// MATLAB's limit is 63, and the three held back leave room for the uniquing
/// suffix. This is asammdf's `matlab_compatible` rule, matched deliberately so
/// that the two tools' exports name the same channel the same way.
fn matlab_compatible(name: &str) -> String {
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

    out.truncate(60);
    out
}
