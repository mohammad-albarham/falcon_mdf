//! MATLAB level 5 MAT-file export.
//!
//! Written directly against the published MAT-File Format specification rather
//! than through a binding, because the level 5 container is small and entirely
//! specified: a 128-byte header followed by a sequence of `miMATRIX` elements,
//! each an 8-byte tag and four sub-elements. Nothing here needs a C library,
//! and the only alternative crates either read but do not write, or bind
//! `libmatio`.
//!
//! # What a MAT file made here contains
//!
//! One numeric matrix per exported channel, plus one per distinct time axis.
//! Names follow asammdf's templates, so a MATLAB script written against an
//! asammdf export finds the same variables here:
//!
//! | Variable | Contents |
//! |---|---|
//! | `DGM<i>_timestamps` | the time axis shared by group `i` |
//! | `DG<i>_<channel>` | one channel's samples |
//! | `DG<i>_<channel>_invalid` | that channel's invalidation mask, only when it has one |
//!
//! Series are grouped by their time axis: series `0` establishes group 0, and
//! any later series with an identical time vector joins it rather than
//! repeating the vector. Every matrix is an N-by-1 column vector, which is how
//! MATLAB spells a time series.
//!
//! # What it does not contain
//!
//! Only numeric channels are written. Text, byte-array, complex, CANopen and
//! array-valued channels are refused by name, with their kind in the error,
//! rather than skipped: MATLAB spells those as cell and struct arrays, which
//! this writer does not emit, and dropping a requested channel silently is how
//! an export comes to disagree with the file it came from.
//!
//! Invalidation bits are not folded into the samples. A channel that has them
//! gets a companion `_invalid` mask instead, so no sample is overwritten with a
//! stand-in and no information is lost.

use std::collections::HashMap;
use std::io::Write;

use crate::error::{Mf4Error, Result};
use crate::model::SignalValues;
use crate::time_ops::SignalSeries;

// MAT-File data types (Table 1-1 of the specification).
const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_SINGLE: u32 = 7;
const MI_DOUBLE: u32 = 9;
const MI_INT64: u32 = 12;
const MI_UINT64: u32 = 13;
const MI_MATRIX: u32 = 14;

// MATLAB array classes (Table 1-3).
const MX_DOUBLE: u8 = 6;
const MX_SINGLE: u8 = 7;
const MX_INT8: u8 = 8;
const MX_UINT8: u8 = 9;
const MX_INT16: u8 = 10;
const MX_UINT16: u8 = 11;
const MX_INT32: u8 = 12;
const MX_UINT32: u8 = 13;
const MX_INT64: u8 = 14;
const MX_UINT64: u8 = 15;

/// Serialises `v` as little-endian bytes. Every numeric type MAT can hold has
/// `to_le_bytes`, so one macro covers all ten of them.
macro_rules! le_bytes {
    ($v:expr) => {
        $v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
    };
}

/// Writes `series` to `out` as a MATLAB level 5 MAT-file.
///
/// An empty slice writes a valid, variable-free MAT-file: the header alone is
/// a complete file, and MATLAB and scipy both load it as an empty workspace.
///
/// # Errors
///
/// Returns an error for a channel whose samples are not numeric — see the
/// module documentation for why those are refused rather than skipped.
///
/// # Example
///
/// ```no_run
/// # use falcon_mdf::Mf4File;
/// # use falcon_mdf::export::write_mat;
/// let file = Mf4File::open("measurement.mf4")?;
/// let series = file.filter(&["Speed".into(), "RPM".into()])?;
/// let mut out = std::fs::File::create("measurement.mat")?;
/// write_mat(&series, &mut out)?;
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub fn write_mat<W: Write>(series: &[SignalSeries], out: &mut W) -> Result<()> {
    write_header(out)?;

    let mut names = UniqueNames::default();
    for (group_index, group) in time_groups(series).into_iter().enumerate() {
        let timestamps = series[group[0]].timestamps();
        write_matrix(
            out,
            &names.claim(&format!("DGM{group_index}_timestamps")),
            MX_DOUBLE,
            MI_DOUBLE,
            timestamps.len(),
            &le_bytes!(timestamps),
        )?;

        for &index in &group {
            let s = &series[index];
            let (class, data_type, data) = numeric_matrix(s)?;
            write_matrix(
                out,
                &names.claim(&format!("DG{group_index}_{}", s.name())),
                class,
                data_type,
                s.len(),
                &data,
            )?;

            // Only when the channel actually carries invalidation bits: an
            // all-valid mask beside every channel would be noise, and the
            // absence of the variable is itself the statement that the file
            // recorded no invalidation for this channel.
            if let Some(validity) = s.validity() {
                let mask: Vec<u8> = validity.iter().map(|&valid| u8::from(!valid)).collect();
                write_matrix(
                    out,
                    &names.claim(&format!("DG{group_index}_{}_invalid", s.name())),
                    MX_UINT8,
                    MI_UINT8,
                    mask.len(),
                    &mask,
                )?;
            }
        }
    }

    Ok(())
}

/// Groups series by their time axis, in first-appearance order.
///
/// Returns one vector of indices into `series` per distinct time axis.
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

/// The class, element type and little-endian bytes for one series' samples.
fn numeric_matrix(series: &SignalSeries) -> Result<(u8, u32, Vec<u8>)> {
    let refuse = |kind: &str| {
        Err(Mf4Error::unsupported(
            "MAT export",
            format!(
                "channel '{}' holds {kind} samples, which a numeric MATLAB matrix cannot \
                 represent; export it to Parquet, or drop it from the selection",
                series.name()
            ),
        ))
    };

    Ok(match series.values() {
        SignalValues::U8(v) => (MX_UINT8, MI_UINT8, v.clone()),
        SignalValues::I8(v) => (MX_INT8, MI_INT8, v.iter().map(|&x| x as u8).collect()),
        SignalValues::U16(v) => (MX_UINT16, MI_UINT16, le_bytes!(v)),
        SignalValues::I16(v) => (MX_INT16, MI_INT16, le_bytes!(v)),
        SignalValues::U32(v) => (MX_UINT32, MI_UINT32, le_bytes!(v)),
        SignalValues::I32(v) => (MX_INT32, MI_INT32, le_bytes!(v)),
        SignalValues::U64(v) => (MX_UINT64, MI_UINT64, le_bytes!(v)),
        SignalValues::I64(v) => (MX_INT64, MI_INT64, le_bytes!(v)),
        SignalValues::F32(v) => (MX_SINGLE, MI_SINGLE, le_bytes!(v)),
        SignalValues::F64(v) => (MX_DOUBLE, MI_DOUBLE, le_bytes!(v)),
        SignalValues::Str(_) => return refuse("text"),
        SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => return refuse("byte-array"),
        SignalValues::Complex { .. } => return refuse("complex"),
        SignalValues::CanopenDate(_) => return refuse("CANopen date"),
        SignalValues::CanopenTime(_) => return refuse("CANopen time"),
        SignalValues::Array { .. } | SignalValues::ArrayVarLen { .. } => return refuse("array"),
    })
}

/// The 128-byte file header: 116 bytes of descriptive text, an 8-byte subsystem
/// offset, then the version and endian indicator.
fn write_header<W: Write>(out: &mut W) -> Result<()> {
    let mut header = [b' '; 128];

    let text = format!(
        "MATLAB 5.0 MAT-file, Platform: {}, Created by: falcon_mdf {}",
        std::env::consts::OS,
        env!("CARGO_PKG_VERSION")
    );
    // Truncated rather than wrapped: the field is fixed at 116 bytes and the
    // text is descriptive only. `min` on a byte length is safe here because
    // every byte of `text` is ASCII.
    let text = text.as_bytes();
    let len = text.len().min(116);
    header[..len].copy_from_slice(&text[..len]);

    // Bytes 116..124 are the subsystem data offset; all spaces means "none".
    // Bytes 124..126 are the version, 0x0100, little-endian. Bytes 126..128 are
    // the endian indicator: the characters 'I' then 'M' say the file is
    // little-endian, which is how a reader knows to byte-swap or not.
    header[124..126].copy_from_slice(&0x0100u16.to_le_bytes());
    header[126] = b'I';
    header[127] = b'M';

    out.write_all(&header)?;
    Ok(())
}

/// Writes one `miMATRIX` element holding an N-by-1 numeric column vector.
fn write_matrix<W: Write>(
    out: &mut W,
    name: &str,
    class: u8,
    data_type: u32,
    rows: usize,
    data: &[u8],
) -> Result<()> {
    let rows = i32::try_from(rows).map_err(|_| {
        Mf4Error::write_error(format!(
            "channel '{name}' has more samples than a MAT-file dimension can hold"
        ))
    })?;

    // Array flags: the class in the low byte, no flags set above it, then a
    // zero `nzmax` (used only by sparse arrays).
    let mut flags = Vec::with_capacity(8);
    flags.extend_from_slice(&(class as u32).to_le_bytes());
    flags.extend_from_slice(&0u32.to_le_bytes());

    let mut dimensions = Vec::with_capacity(8);
    dimensions.extend_from_slice(&rows.to_le_bytes());
    dimensions.extend_from_slice(&1i32.to_le_bytes());

    let mut body = Vec::new();
    push_element(&mut body, MI_UINT32, &flags);
    push_element(&mut body, MI_INT32, &dimensions);
    push_element(&mut body, MI_INT8, name.as_bytes());
    push_element(&mut body, data_type, data);

    // The matrix tag's length covers its sub-elements but not itself, and the
    // sub-elements are already padded, so `body.len()` is the figure a reader
    // needs to skip the whole variable.
    out.write_all(&MI_MATRIX.to_le_bytes())?;
    out.write_all(&(body.len() as u32).to_le_bytes())?;
    out.write_all(&body)?;
    Ok(())
}

/// Appends one tagged data element, padded to the 8-byte boundary the format
/// requires between elements.
fn push_element(buffer: &mut Vec<u8>, data_type: u32, data: &[u8]) {
    buffer.extend_from_slice(&data_type.to_le_bytes());
    buffer.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buffer.extend_from_slice(data);
    // The tag itself is 8 bytes, so only the payload needs rounding up.
    let padding = (8 - data.len() % 8) % 8;
    buffer.extend(std::iter::repeat_n(0u8, padding));
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
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();

    if !out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        out.insert_str(0, "M_");
    }

    out.truncate(60);
    out
}
