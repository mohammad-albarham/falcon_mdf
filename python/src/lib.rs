//! Python bindings for `falcon_mdf` via PyO3.
//!
//! Exposes a minimal, Pythonic API over the Rust MDF v3/v4 reader:
//!
//! - `falcon_mdf.open(path)` opens an MDF v3 or v4 file (a `str` or any
//!   `os.PathLike`) and returns an `Mf4File`, which is also a context manager.
//! - `Mf4File.channels()` lists channel names.
//! - `Mf4File.get(name)` returns `(values, timestamps)`; see [`Mf4FilePy::get`]
//!   for how each sample type and invalid sample is represented.
//! - `Mf4File.info()` returns a dict with version and channel counts.
//! - `Mf4File.to_dataframe()` returns the decoded channels as a pandas or
//!   polars DataFrame via Arrow IPC.
//! - `Mf4File.iter_to_dataframe()` streams the same data in bounded windows.
//!
//! Errors map onto Python's own exception types: a missing file is a
//! `FileNotFoundError`, an unknown channel a `KeyError`, a malformed file or
//! bad argument a `ValueError`, a valid-but-undecoded feature a
//! `NotImplementedError`, and anything else a `RuntimeError`.
//!
//! Heavy file I/O, decompression, and decoding calls release the GIL via
//! `py.detach(...)`, so other Python threads keep running concurrently.

use std::collections::{HashSet, VecDeque};
use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

use ::falcon_mdf::mdf3::Mdf3File;
use ::falcon_mdf::stream::SignalsChunks;
use ::falcon_mdf::time_ops::{InterpolationMode, Raster};
use ::falcon_mdf::{Channel, Mf4Error, Mf4File, SignalSeries, SignalValues};
use pyo3::exceptions::{
    PyImportError, PyIndexError, PyKeyError, PyNotImplementedError, PyRuntimeError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyComplex, PyDict, PyList};

/// Converts a Rust [`Mf4Error`] into the closest idiomatic Python exception.
///
/// Required explicitly because `Mf4Error` does not implement `Send + Sync`, so
/// PyO3 cannot provide the automatic `From` conversion.
///
/// - I/O failures become the matching `OSError` subclass (`FileNotFoundError`,
///   `PermissionError`, ...), carrying the OS error number.
/// - An unknown channel is a `KeyError`.
/// - A well-formed file that uses an unsupported version or feature is a
///   `NotImplementedError`.
/// - A file that is not valid MDF, or whose blocks/data are corrupt, is a
///   `ValueError`.
/// - Anything else (memory-mapping failure, write errors) is a `RuntimeError`.
fn py_err(err: Mf4Error) -> PyErr {
    match err {
        Mf4Error::Io(io) => PyErr::from(io),
        Mf4Error::ChannelNotFound { name } => PyKeyError::new_err(name),
        Mf4Error::Unsupported { .. } | Mf4Error::UnsupportedVersion { .. } => {
            PyNotImplementedError::new_err(err.to_string())
        }
        Mf4Error::InvalidSignature(_)
        | Mf4Error::MissingBlock { .. }
        | Mf4Error::InvalidBlockSize { .. }
        | Mf4Error::InvalidBlockId { .. }
        | Mf4Error::TruncatedFile { .. }
        | Mf4Error::InvalidLink { .. }
        | Mf4Error::DataTypeConversion { .. }
        | Mf4Error::Compression(_)
        | Mf4Error::Decompression(_)
        | Mf4Error::InvalidDataBlock { .. }
        | Mf4Error::InvalidConversion { .. }
        | Mf4Error::Utf8Error(_)
        | Mf4Error::ParseError { .. }
        | Mf4Error::CyclicLink { .. } => PyValueError::new_err(err.to_string()),
        _ => PyRuntimeError::new_err(err.to_string()),
    }
}

/// Imports a Python module, turning a missing dependency into a clear
/// `ImportError` that names the package to install.
fn import_required<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(name).map_err(|err| {
        if err.is_instance_of::<PyImportError>(py) {
            PyImportError::new_err(format!(
                "{name} is required for to_dataframe(); install it with `pip install {name}`"
            ))
        } else {
            err
        }
    })
}

fn check_backend(backend: &str) -> PyResult<()> {
    match backend {
        "pandas" | "pd" | "polars" | "pl" => Ok(()),
        _ => Err(PyValueError::new_err(format!(
            "unsupported backend '{backend}'; use 'pandas' or 'polars'"
        ))),
    }
}

/// Makes `names` unique while keeping the first occurrence of each name intact.
///
/// Arrow permits repeated column names, but polars refuses them with
/// `DuplicateError`, and a pandas frame with repeated columns cannot be indexed
/// unambiguously by name. MDF files routinely repeat names: channel groups
/// often share a master name (`t`, `time`, `Timestamp`), and the exported table
/// prepends its own `time` column. Each repeat becomes `name_1`, `name_2`, ...,
/// skipping any candidate already taken by another column.
fn unique_names(names: &[String]) -> Vec<String> {
    let original: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut used: HashSet<String> = HashSet::with_capacity(names.len());
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let chosen = if !used.contains(name.as_str()) {
            name.clone()
        } else {
            let mut n = 1usize;
            loop {
                let candidate = format!("{name}_{n}");
                if !used.contains(&candidate) && !original.contains(candidate.as_str()) {
                    break candidate;
                }
                n += 1;
            }
        };
        used.insert(chosen.clone());
        out.push(chosen);
    }
    out
}

/// Turns an Arrow IPC byte stream into a pandas or polars DataFrame.
fn ipc_to_dataframe<'py>(
    py: Python<'py>,
    ipc_bytes: &[u8],
    backend: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let pyarrow = import_required(py, "pyarrow")?;
    let ipc_mod = pyarrow.getattr("ipc")?;
    let reader = ipc_mod.call_method1("open_file", (PyBytes::new(py, ipc_bytes),))?;
    let mut table = reader.call_method0("read_all")?;

    let names: Vec<String> = table.getattr("column_names")?.extract()?;
    let unique = unique_names(&names);
    if unique != names {
        table = table.call_method1("rename_columns", (unique,))?;
    }

    match backend {
        "pandas" | "pd" => {
            let _pandas = import_required(py, "pandas")?;
            table.call_method0("to_pandas")
        }
        "polars" | "pl" => {
            let polars = import_required(py, "polars")?;
            polars.call_method1("from_arrow", (table,))
        }
        _ => Err(PyValueError::new_err(format!(
            "unsupported backend '{backend}'; use 'pandas' or 'polars'"
        ))),
    }
}

/// Underlying open reader for either MDF v4 or MDF v3.
#[derive(Clone)]
enum Inner {
    V4(Arc<Mf4File>),
    V3(Arc<Mdf3File>),
}

/// Python wrapper around an open MDF v3/v4 measurement file.
///
/// Marked `frozen` so concurrent Python readers do not contend on a PyO3
/// borrow lock. The underlying file sits behind `RwLock<Option<Inner>>`
/// so `close()` can release the handle through a shared reference while any
/// active `DataFrameIterator` retains its own `Arc` and keeps the memory map
/// alive until the iterator finishes or is dropped.
#[pyclass(frozen, name = "Mf4File")]
pub struct Mf4FilePy {
    inner: RwLock<Option<Inner>>,
}

impl Mf4FilePy {
    /// Returns the open file handle, or `ValueError` once closed.
    fn inner(&self) -> PyResult<Inner> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| PyValueError::new_err("I/O operation on closed file"))
    }
}

/// Resolves channel names to channels on an MF4 file; `None` selects every
/// channel, whereas `Some(&[])` selects nothing.
fn select_v4_channels<'a>(
    file: &'a Mf4File,
    names: Option<&[String]>,
) -> Result<Vec<&'a Channel>, Mf4Error> {
    match names {
        Some(names) => names
            .iter()
            .map(|name| {
                file.find_channel(name)
                    .ok_or_else(|| Mf4Error::ChannelNotFound { name: name.clone() })
            })
            .collect(),
        None => Ok(file.channels().collect()),
    }
}

/// Locates the first `(data_group, channel_group, channel)` matching `name` in
/// an MDF 3.x file.
fn find_v3_channel(file: &Mdf3File, name: &str) -> Result<(usize, usize, usize), Mf4Error> {
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if ch.name == name {
                    return Ok((dg_idx, cg_idx, ch_idx));
                }
            }
        }
    }
    Err(Mf4Error::ChannelNotFound {
        name: name.to_string(),
    })
}

/// Decodes one channel of an MDF 3.x file together with its group's time axis.
fn v3_time_series(
    file: &Mdf3File,
    dg_idx: usize,
    cg_idx: usize,
    ch_idx: usize,
) -> Result<SignalSeries, Mf4Error> {
    let cg = &file.data_groups()[dg_idx].channel_groups[cg_idx];
    let ch_meta = &cg.channels[ch_idx];
    let values = file.channel_physical(dg_idx, cg_idx, ch_idx)?;
    let timestamps = match cg.channels.iter().position(|c| c.is_time()) {
        Some(master_idx) => file.channel_physical(dg_idx, cg_idx, master_idx)?.to_f64(),
        None => (0..values.len()).map(|i| i as f64).collect(),
    };
    SignalSeries::from_samples(
        ch_meta.name.clone(),
        ch_meta.unit.clone(),
        timestamps,
        values,
        None,
    )
}

/// Builds an all-null float series on `axis` for a channel that has 0 samples
/// when other channels in the table have samples.
fn null_series(template: &SignalSeries, axis: &Arc<Vec<f64>>) -> Result<SignalSeries, Mf4Error> {
    let n = axis.len();
    SignalSeries::new(
        template.channel.clone(),
        Arc::clone(axis),
        SignalValues::F64(vec![f64::NAN; n]),
        Some(vec![false; n]),
    )
}

/// Returns the decoded MF4 channels as an Arrow IPC byte stream.
///
/// `names` is an optional slice of channel names; `None` exports every channel
/// in the file and `Some(&[])` exports none (a 0-row table with only the `time`
/// column). Channels that do not already share a single time axis are resampled
/// onto the first non-empty channel's time axis with linear interpolation,
/// because an Arrow table has one time column for the whole table. Channels
/// from 0-sample channel groups cannot be resampled onto a non-empty axis, so
/// they are emitted as all-null columns instead.
fn arrow_ipc_bytes_v4(file: &Mf4File, names: Option<&[String]>) -> Result<Vec<u8>, Mf4Error> {
    let channels = select_v4_channels(file, names)?;

    let mut series: Vec<SignalSeries> = channels
        .iter()
        .map(|&ch| file.time_series(ch))
        .collect::<Result<_, _>>()?;

    if let Some(reference) = series.iter().find(|s| !s.is_empty()) {
        let axis = reference.timestamps_shared();

        let misaligned: Vec<usize> = series
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_empty() && s.timestamps() != axis.as_slice())
            .map(|(i, _)| i)
            .collect();
        if !misaligned.is_empty() {
            let selected: Vec<&Channel> = misaligned.iter().map(|&i| channels[i]).collect();
            let resampled = file.resample(
                &selected,
                Raster::Timestamps(axis.to_vec()),
                InterpolationMode::Linear,
            )?;
            for (i, s) in misaligned.into_iter().zip(resampled) {
                series[i] = s;
            }
        }

        for s in series.iter_mut() {
            if s.is_empty() {
                *s = null_series(s, &axis)?;
            }
        }
    }

    let mut buf = Vec::new();
    ::falcon_mdf::export::write_arrow_ipc(&series, &mut buf)?;
    Ok(buf)
}

/// Returns the decoded MDF 3.x channels as an Arrow IPC byte stream.
fn arrow_ipc_bytes_v3(file: &Mdf3File, names: Option<&[String]>) -> Result<Vec<u8>, Mf4Error> {
    let coords: Vec<(usize, usize, usize)> = match names {
        Some(names) => names
            .iter()
            .map(|name| find_v3_channel(file, name))
            .collect::<Result<_, _>>()?,
        None => file
            .data_groups()
            .iter()
            .enumerate()
            .flat_map(|(dg_idx, dg)| {
                dg.channel_groups
                    .iter()
                    .enumerate()
                    .flat_map(move |(cg_idx, cg)| {
                        (0..cg.channels.len()).map(move |ch_idx| (dg_idx, cg_idx, ch_idx))
                    })
            })
            .collect(),
    };

    let mut series: Vec<SignalSeries> = coords
        .iter()
        .map(|&(dg, cg, ch)| v3_time_series(file, dg, cg, ch))
        .collect::<Result<_, _>>()?;

    if let Some(reference) = series.iter().find(|s| !s.is_empty()) {
        let axis = reference.timestamps_shared();
        for s in series.iter_mut() {
            if s.is_empty() {
                *s = null_series(s, &axis)?;
            } else if s.timestamps() != axis.as_slice() {
                *s = s.resample(Raster::Timestamps(axis.to_vec()), InterpolationMode::Linear)?;
            }
        }
    }

    let mut buf = Vec::new();
    ::falcon_mdf::export::write_arrow_ipc(&series, &mut buf)?;
    Ok(buf)
}

/// Converts a slice of integers to a Python list, masking invalid samples as `None`.
fn int_list<'py, T>(
    py: Python<'py>,
    values: &[T],
    validity: Option<&[bool]>,
) -> PyResult<Bound<'py, PyList>>
where
    T: Copy + IntoPyObject<'py>,
{
    match validity {
        None => PyList::new(py, values.iter().copied()),
        Some(ok) => PyList::new(
            py,
            values.iter().enumerate().map(|(i, &v)| ok[i].then_some(v)),
        ),
    }
}

/// Converts a slice of floats to a Python list, masking invalid samples as `NaN`.
fn float_list<'py, T: Copy + Into<f64>>(
    py: Python<'py>,
    values: &[T],
    validity: Option<&[bool]>,
) -> PyResult<Bound<'py, PyList>> {
    match validity {
        None => PyList::new(py, values.iter().map(|&v| v.into())),
        Some(ok) => PyList::new(
            py,
            values
                .iter()
                .enumerate()
                .map(|(i, &v)| if ok[i] { v.into() } else { f64::NAN }),
        ),
    }
}

/// Recursively nests a flat row-major slice of `f64` values according to `dims`.
fn nest_f64_dims<'py>(
    py: Python<'py>,
    slice: &[f64],
    dims: &[usize],
) -> PyResult<Bound<'py, PyList>> {
    if dims.len() <= 1 {
        return PyList::new(py, slice);
    }
    let outer = dims[0];
    let stride = slice.len() / outer;
    let mut rows = Vec::with_capacity(outer);
    for k in 0..outer {
        rows.push(nest_f64_dims(
            py,
            &slice[k * stride..(k + 1) * stride],
            &dims[1..],
        )?);
    }
    PyList::new(py, rows)
}

/// Validates `array_shape` against `elements_per_sample` and returns usable
/// `usize` dimensions if the product matches.
fn usable_shape(shape: Option<&[u64]>, elements_per_sample: usize) -> Option<Vec<usize>> {
    let dims = shape?;
    if dims.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(dims.len());
    let mut product = 1usize;
    for &d in dims {
        let d = usize::try_from(d).ok()?;
        if d == 0 {
            return None;
        }
        product = product.checked_mul(d)?;
        out.push(d);
    }
    (product == elements_per_sample).then_some(out)
}

/// Converts decoded samples to a Python list, one entry per sample.
///
/// Invalid samples (invalidation bit set) are masked rather than returned as
/// the stand-in bit pattern stored in the record: float scalars (and elements
/// of array samples) become `NaN`; integers, text, bytes, complex numbers, and
/// CANopen date/time samples become `None`.
fn values_to_list<'py>(py: Python<'py>, series: &SignalSeries) -> PyResult<Bound<'py, PyList>> {
    use SignalValues as V;

    let validity = series.validity();
    let valid = |i: usize| validity.is_none_or(|ok| ok[i]);

    match series.values() {
        V::U8(v) => int_list(py, v, validity),
        V::U16(v) => int_list(py, v, validity),
        V::U32(v) => int_list(py, v, validity),
        V::U64(v) => int_list(py, v, validity),
        V::I8(v) => int_list(py, v, validity),
        V::I16(v) => int_list(py, v, validity),
        V::I32(v) => int_list(py, v, validity),
        V::I64(v) => int_list(py, v, validity),
        V::F32(v) => float_list(py, v, validity),
        V::F64(v) => match validity {
            None => PyList::new(py, v),
            Some(_) => float_list(py, v, validity),
        },
        V::Str(v) => PyList::new(
            py,
            v.iter()
                .enumerate()
                .map(|(i, s)| valid(i).then_some(s.as_str())),
        ),
        V::Bytes { .. } | V::VarBytes { .. } => {
            let items = (0..series.len())
                .map(|i| {
                    valid(i)
                        .then(|| series.values().bytes_at(i))
                        .flatten()
                        .map(|b| PyBytes::new(py, b))
                })
                .collect::<Vec<_>>();
            PyList::new(py, items)
        }
        V::Complex { re, im } => {
            let items = re
                .iter()
                .zip(im)
                .enumerate()
                .map(|(i, (&re, &im))| valid(i).then(|| PyComplex::from_doubles(py, re, im)))
                .collect::<Vec<_>>();
            PyList::new(py, items)
        }
        // Calendar records have no float form; nanoseconds since the Unix
        // epoch (UTC) is the exact, unambiguous representation.
        V::CanopenDate(v) => PyList::new(
            py,
            v.iter()
                .enumerate()
                .map(|(i, d)| valid(i).then(|| d.to_unix_nanos())),
        ),
        V::CanopenTime(v) => PyList::new(
            py,
            v.iter()
                .enumerate()
                .map(|(i, t)| valid(i).then(|| t.to_unix_nanos())),
        ),
        // Array samples preserve per-sample nesting so `len(values) == len(timestamps)`
        // and multi-dimensional CA shapes stay intact.
        V::Array {
            values,
            elements_per_sample,
        } => {
            let width = *elements_per_sample;
            let dims = usable_shape(series.channel.array_shape.as_deref(), width);
            let nan_row = validity.map(|_| vec![f64::NAN; width]);
            let mut rows = Vec::with_capacity(series.len());
            for i in 0..series.len() {
                let slice = if valid(i) {
                    &values[i * width..(i + 1) * width]
                } else {
                    nan_row.as_deref().unwrap_or(&[])
                };
                let row = match dims.as_deref() {
                    Some(d) => nest_f64_dims(py, slice, d)?,
                    None => PyList::new(py, slice)?,
                };
                rows.push(row);
            }
            PyList::new(py, rows)
        }
        V::ArrayVarLen { values, starts } => {
            let mut rows = Vec::with_capacity(series.len());
            for i in 0..series.len() {
                let slice = &values[starts[i]..starts[i + 1]];
                let row = if valid(i) {
                    PyList::new(py, slice)?
                } else {
                    PyList::new(py, vec![f64::NAN; slice.len()])?
                };
                rows.push(row);
            }
            PyList::new(py, rows)
        }
        other => Err(PyNotImplementedError::new_err(format!(
            "channel '{}' has samples of a kind this binding cannot convert ({})",
            series.name(),
            other.kind()
        ))),
    }
}

/// Extracts sample range `[start..end)` from a [`SignalValues`].
fn slice_signal_values(
    values: &SignalValues,
    start: usize,
    end: usize,
) -> Result<SignalValues, Mf4Error> {
    use SignalValues as V;
    Ok(match values {
        V::U8(v) => V::U8(v[start..end].to_vec()),
        V::U16(v) => V::U16(v[start..end].to_vec()),
        V::U32(v) => V::U32(v[start..end].to_vec()),
        V::U64(v) => V::U64(v[start..end].to_vec()),
        V::I8(v) => V::I8(v[start..end].to_vec()),
        V::I16(v) => V::I16(v[start..end].to_vec()),
        V::I32(v) => V::I32(v[start..end].to_vec()),
        V::I64(v) => V::I64(v[start..end].to_vec()),
        V::F32(v) => V::F32(v[start..end].to_vec()),
        V::F64(v) => V::F64(v[start..end].to_vec()),
        V::Str(v) => V::Str(v[start..end].to_vec()),
        V::Bytes { data, width } => V::Bytes {
            data: data[start * width..end * width].to_vec(),
            width: *width,
        },
        V::VarBytes { data, starts } => {
            let base = starts[start];
            let stop = starts[end];
            V::VarBytes {
                data: data[base..stop].to_vec(),
                starts: starts[start..=end].iter().map(|&s| s - base).collect(),
            }
        }
        V::Complex { re, im } => V::Complex {
            re: re[start..end].to_vec(),
            im: im[start..end].to_vec(),
        },
        V::CanopenDate(v) => V::CanopenDate(v[start..end].to_vec()),
        V::CanopenTime(v) => V::CanopenTime(v[start..end].to_vec()),
        V::Array {
            values,
            elements_per_sample,
        } => V::Array {
            values: values[start * elements_per_sample..end * elements_per_sample].to_vec(),
            elements_per_sample: *elements_per_sample,
        },
        V::ArrayVarLen { values, starts } => {
            let base = starts[start];
            let stop = starts[end];
            V::ArrayVarLen {
                values: values[base..stop].to_vec(),
                starts: starts[start..=end].iter().map(|&s| s - base).collect(),
            }
        }
        other => {
            return Err(Mf4Error::unsupported(
                "slicing signal values",
                format!("unsupported value kind '{}'", other.kind()),
            ));
        }
    })
}

/// Slices a [`SignalSeries`] to sample range `[start..end)`.
fn slice_series(series: &SignalSeries, start: usize, end: usize) -> Result<SignalSeries, Mf4Error> {
    SignalSeries::new(
        series.channel.clone(),
        series.timestamps()[start..end].to_vec(),
        slice_signal_values(series.values(), start, end)?,
        series.validity().map(|v| v[start..end].to_vec()),
    )
}

#[pymethods]
impl Mf4FilePy {
    /// Returns a list with the name of every channel in the file.
    fn channels(&self) -> PyResult<Vec<String>> {
        match self.inner()? {
            Inner::V4(file) => Ok(file.channels().map(|ch| ch.name.clone()).collect()),
            Inner::V3(file) => Ok(file
                .channel_names()
                .into_iter()
                .map(str::to_owned)
                .collect()),
        }
    }

    /// Reads one channel by name.
    ///
    /// Returns a `(values, timestamps)` tuple of equal length: `timestamps` is
    /// a list of `float`, and `values` holds one entry per sample in the
    /// channel's native type without silent precision loss:
    ///
    /// - integers (including 64-bit `i64`/`u64`) are `int`, floats are `float`;
    /// - text is `str`, byte arrays are `bytes`, complex samples are `complex`;
    /// - CANopen date and time samples are `int` nanoseconds since the Unix
    ///   epoch (UTC);
    /// - array samples are nested `list`s of floats per sample preserving the
    ///   channel's array shape, so `len(values) == len(timestamps)` holds.
    ///
    /// Invalid samples (invalidation bit set) are masked rather than returned
    /// as raw stand-in values: `NaN` for floats (and array elements), `None`
    /// for integers, text, bytes, complex, and date/time channels.
    ///
    /// Raises `KeyError` if no channel has that name.
    fn get<'py>(&self, py: Python<'py>, name: &str) -> PyResult<(Bound<'py, PyList>, Vec<f64>)> {
        let inner = self.inner()?;
        let series = py.detach(|| -> PyResult<SignalSeries> {
            match inner {
                Inner::V4(file) => {
                    let channel = file
                        .find_channel(name)
                        .ok_or_else(|| Mf4Error::ChannelNotFound {
                            name: name.to_string(),
                        })
                        .map_err(py_err)?;
                    file.time_series(channel).map_err(py_err)
                }
                Inner::V3(file) => {
                    let (dg, cg, ch) = find_v3_channel(&file, name).map_err(py_err)?;
                    v3_time_series(&file, dg, cg, ch).map_err(py_err)
                }
            }
        })?;

        let values = values_to_list(py, &series)?;
        Ok((values, series.timestamps().to_vec()))
    }

    /// Returns the decoded channels as a pandas or polars DataFrame.
    ///
    /// `channels` is an optional list of channel names; when omitted (`None`),
    /// every channel in the file is exported; an explicit empty list (`[]`)
    /// exports an empty DataFrame (0 rows, only the `time` column). `backend`
    /// is either `"pandas"` (default) or `"polars"`.
    ///
    /// The data is handed to Python as an Arrow IPC table read with `pyarrow`,
    /// so pandas and polars both see typed, nullable columns. Duplicate column
    /// names are disambiguated (`name_1`, `name_2`, ...) so polars does not
    /// fail with `DuplicateError`.
    #[pyo3(signature = (channels=None, backend="pandas"))]
    fn to_dataframe<'py>(
        &self,
        py: Python<'py>,
        channels: Option<Vec<String>>,
        backend: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        check_backend(backend)?;
        let inner = self.inner()?;
        let ipc_bytes = py.detach(|| match inner {
            Inner::V4(file) => arrow_ipc_bytes_v4(&file, channels.as_deref()).map_err(py_err),
            Inner::V3(file) => arrow_ipc_bytes_v3(&file, channels.as_deref()).map_err(py_err),
        })?;
        ipc_to_dataframe(py, &ipc_bytes, backend)
    }

    /// Yields streaming windows of decoded channels as pandas or polars DataFrames.
    ///
    /// `chunk_size` is the maximum number of samples to yield per DataFrame
    /// window. `channels` is an optional list of channel names belonging to the
    /// same channel group; an explicit empty list (`[]`) yields nothing.
    ///
    /// When `channels` is `None` and `group` is `None`, every non-empty channel
    /// group in the file is streamed one group at a time: each group's windows
    /// are yielded in order with that group's columns and its own `time` axis.
    /// Pass `group` (0-based flat channel-group index) to stream a single group
    /// explicitly.
    ///
    /// `backend` is either `"pandas"` (default) or `"polars"`.
    #[pyo3(signature = (chunk_size, channels=None, backend="pandas", group=None))]
    fn iter_to_dataframe(
        &self,
        py: Python<'_>,
        chunk_size: usize,
        channels: Option<Vec<String>>,
        backend: &str,
        group: Option<usize>,
    ) -> PyResult<DataFrameIterator> {
        if chunk_size == 0 {
            return Err(PyValueError::new_err("chunk_size must be greater than 0"));
        }
        check_backend(backend)?;

        match self.inner()? {
            Inner::V4(file) => {
                let groups = build_v4_iter_groups(&file, channels.as_deref(), group)?;
                let mut stream = V4Stream {
                    stream: None,
                    file,
                    pending: groups,
                    chunk_size,
                    sample_offset: 0,
                };
                // Open the first group eagerly so invalid selections (e.g.
                // explicit channels spanning multiple groups) fail immediately.
                py.detach(|| stream.open_next_group().map_err(py_err))?;
                Ok(DataFrameIterator {
                    state: IterState::V4(Box::new(stream)),
                    backend: backend.to_string(),
                })
            }
            Inner::V3(file) => {
                let groups = build_v3_iter_groups(&file, channels.as_deref(), group)?;
                Ok(DataFrameIterator {
                    state: IterState::V3(V3Stream {
                        file,
                        pending: groups,
                        current: None,
                        chunk_size,
                    }),
                    backend: backend.to_string(),
                })
            }
        }
    }

    /// Returns file metadata as a dictionary.
    ///
    /// Keys: `version` (str), `channel_group_count` (int), `channel_count` (int),
    /// `sample_count` (int, the largest channel group's sample count).
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        match self.inner()? {
            Inner::V4(file) => {
                dict.set_item("version", file.version().to_string())?;
                let channel_group_count: usize = file
                    .data_groups()
                    .iter()
                    .map(|dg| dg.channel_groups.len())
                    .sum();
                dict.set_item("channel_group_count", channel_group_count)?;
                dict.set_item("channel_count", file.channel_count())?;
                let sample_count: u64 = file
                    .data_groups()
                    .iter()
                    .flat_map(|dg| dg.channel_groups.iter())
                    .map(|cg| cg.sample_count)
                    .max()
                    .unwrap_or(0);
                dict.set_item("sample_count", sample_count)?;
            }
            Inner::V3(file) => {
                dict.set_item("version", file.version())?;
                let channel_group_count: usize = file
                    .data_groups()
                    .iter()
                    .map(|dg| dg.channel_groups.len())
                    .sum();
                dict.set_item("channel_group_count", channel_group_count)?;
                dict.set_item("channel_count", file.channel_count())?;
                let sample_count: u64 = file
                    .data_groups()
                    .iter()
                    .flat_map(|dg| dg.channel_groups.iter())
                    .map(|cg| u64::from(cg.cycle_count))
                    .max()
                    .unwrap_or(0);
                dict.set_item("sample_count", sample_count)?;
            }
        }
        Ok(dict)
    }

    /// True once `close()` has been called.
    #[getter]
    fn closed(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }

    /// Releases the underlying file handle. Further reads raise `ValueError`;
    /// calling `close()` more than once is a no-op.
    ///
    /// Any `DataFrameIterator` already created from this file keeps its own
    /// reference to the file until exhausted or garbage-collected.
    fn close(&self) {
        *self.inner.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        slf.inner()?;
        Ok(slf)
    }

    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        self.close();
        false
    }
}

fn build_v4_iter_groups(
    file: &Mf4File,
    channels: Option<&[String]>,
    group: Option<usize>,
) -> PyResult<VecDeque<Vec<Channel>>> {
    if let Some(group_idx) = group {
        let all_groups: Vec<_> = file
            .data_groups()
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .collect();
        let cg = all_groups.get(group_idx).ok_or_else(|| {
            PyIndexError::new_err(format!(
                "channel group index {group_idx} out of range (file has {} channel groups)",
                all_groups.len()
            ))
        })?;
        return match channels {
            Some([]) => Ok(VecDeque::new()),
            Some(names) => {
                let selected = names
                    .iter()
                    .map(|name| {
                        cg.channels
                            .iter()
                            .find(|ch| ch.name == *name)
                            .cloned()
                            .ok_or_else(|| PyKeyError::new_err(name.clone()))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(VecDeque::from([selected]))
            }
            None => {
                if cg.sample_count == 0 {
                    Ok(VecDeque::new())
                } else {
                    let readable: Vec<Channel> = cg
                        .channels
                        .iter()
                        .filter(|ch| ch.unreadable().is_none())
                        .cloned()
                        .collect();
                    if readable.is_empty() {
                        Ok(VecDeque::new())
                    } else {
                        Ok(VecDeque::from([readable]))
                    }
                }
            }
        };
    }

    match channels {
        Some(names) => {
            let selected = select_v4_channels(file, Some(names)).map_err(py_err)?;
            if selected.is_empty() {
                Ok(VecDeque::new())
            } else {
                Ok(VecDeque::from([selected.into_iter().cloned().collect()]))
            }
        }
        None => Ok(file
            .data_groups()
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .filter(|cg| cg.sample_count > 0)
            .map(|cg| {
                cg.channels
                    .iter()
                    .filter(|ch| ch.unreadable().is_none())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .filter(|g| !g.is_empty())
            .collect()),
    }
}

fn build_v3_iter_groups(
    file: &Mdf3File,
    channels: Option<&[String]>,
    group: Option<usize>,
) -> PyResult<VecDeque<(usize, usize, Vec<usize>)>> {
    if let Some(group_idx) = group {
        let all_groups: Vec<(usize, usize, &_)> = file
            .data_groups()
            .iter()
            .enumerate()
            .flat_map(|(dg_idx, dg)| {
                dg.channel_groups
                    .iter()
                    .enumerate()
                    .map(move |(cg_idx, cg)| (dg_idx, cg_idx, cg))
            })
            .collect();
        let &(dg_idx, cg_idx, cg) = all_groups.get(group_idx).ok_or_else(|| {
            PyIndexError::new_err(format!(
                "channel group index {group_idx} out of range (file has {} channel groups)",
                all_groups.len()
            ))
        })?;
        return match channels {
            Some([]) => Ok(VecDeque::new()),
            Some(names) => {
                let indices = names
                    .iter()
                    .map(|name| {
                        cg.channels
                            .iter()
                            .position(|ch| ch.name == *name)
                            .ok_or_else(|| PyKeyError::new_err(name.clone()))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                if cg.cycle_count == 0 {
                    Ok(VecDeque::new())
                } else {
                    Ok(VecDeque::from([(dg_idx, cg_idx, indices)]))
                }
            }
            None => {
                if cg.cycle_count == 0 || cg.channels.is_empty() {
                    Ok(VecDeque::new())
                } else {
                    Ok(VecDeque::from([(
                        dg_idx,
                        cg_idx,
                        (0..cg.channels.len()).collect(),
                    )]))
                }
            }
        };
    }

    match channels {
        Some([]) => Ok(VecDeque::new()),
        Some(names) => {
            let coords: Vec<(usize, usize, usize)> = names
                .iter()
                .map(|name| find_v3_channel(file, name))
                .collect::<Result<_, _>>()
                .map_err(py_err)?;
            let (first_dg, first_cg, _) = coords[0];
            for &(dg, cg, _) in &coords[1..] {
                if dg != first_dg || cg != first_cg {
                    return Err(PyNotImplementedError::new_err(format!(
                        "Unsupported feature: aligned streaming across channel groups \
                         (groups {first_dg}:{first_cg} and {dg}:{cg} belong to different channel groups)"
                    )));
                }
            }
            let cg_meta = &file.data_groups()[first_dg].channel_groups[first_cg];
            if cg_meta.cycle_count == 0 {
                Ok(VecDeque::new())
            } else {
                let ch_indices = coords.into_iter().map(|(_, _, ch)| ch).collect();
                Ok(VecDeque::from([(first_dg, first_cg, ch_indices)]))
            }
        }
        None => Ok(file
            .data_groups()
            .iter()
            .enumerate()
            .flat_map(|(dg_idx, dg)| {
                dg.channel_groups
                    .iter()
                    .enumerate()
                    .filter(|(_, cg)| cg.cycle_count > 0 && !cg.channels.is_empty())
                    .map(move |(cg_idx, cg)| (dg_idx, cg_idx, (0..cg.channels.len()).collect()))
            })
            .collect()),
    }
}

/// Self-referential MF4 streaming state.
///
/// # Safety invariant
///
/// `stream` borrows from the `Mf4File` owned by `file: Arc<Mf4File>`, and its
/// lifetime is erased to `'static` because a `#[pyclass]` cannot carry a
/// borrow lifetime. This layout is sound because:
///
/// 1. The `Mf4File` lives at a stable heap address behind `Arc<Mf4File>`, so
///    moving `V4Stream` or `DataFrameIterator` never moves or invalidates the
///    borrowed `Mf4File` or its memory map, and calling `Mf4File.close()`
///    cannot drop the `Mf4File` while this stream holds its `Arc`.
/// 2. `stream` is wrapped in `ManuallyDrop` and dropped explicitly in
///    [`Drop::drop`] (and in [`Self::finish`] before replacing or clearing it)
///    **before** `self.file` is dropped. We never rely on implicit struct
///    field drop order.
/// 3. `stream` is never exposed outside `V4Stream`, never cloned, and only
///    accessed through `&mut self`.
struct V4Stream {
    /// Active chunk reader for the current channel group. Always dropped
    /// before `file`.
    stream: Option<ManuallyDrop<SignalsChunks<'static>>>,
    /// Heap-owned file that `stream` borrows from. Dropped strictly after
    /// `stream`.
    file: Arc<Mf4File>,
    /// Remaining channel groups to stream, each a list of channels from one group.
    pending: VecDeque<Vec<Channel>>,
    chunk_size: usize,
    sample_offset: usize,
}

impl Drop for V4Stream {
    fn drop(&mut self) {
        self.finish();
    }
}

impl V4Stream {
    /// Explicitly drops the active stream while `self.file` is still alive.
    fn finish(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            // SAFETY: `stream` was removed from `self.stream`, so it is
            // dropped at most once, and `self.file` is still alive and owns
            // the `Mf4File` that `stream` borrows from.
            unsafe { ManuallyDrop::drop(&mut stream) };
        }
    }

    /// Opens a chunk reader on the next pending channel group, if any.
    fn open_next_group(&mut self) -> Result<bool, Mf4Error> {
        self.finish();
        let Some(channels) = self.pending.pop_front() else {
            return Ok(false);
        };
        let stream = self.file.signals_chunks(&channels, self.chunk_size)?;
        // SAFETY: `stream` borrows `&Mf4File` from the stable heap allocation
        // inside `self.file: Arc<Mf4File>`. `self.stream` is always dropped via
        // `self.finish()` (including in `Drop::drop`) before `self.file` is
        // dropped or replaced, and `self.file` is never mutated or dropped
        // while `self.stream` is `Some`.
        let stream: SignalsChunks<'static> = unsafe { std::mem::transmute(stream) };
        self.stream = Some(ManuallyDrop::new(stream));
        self.sample_offset = 0;
        Ok(true)
    }

    /// Reads, decodes, and serializes the next non-empty chunk to Arrow IPC.
    fn next_ipc_chunk(&mut self) -> Result<Option<Vec<u8>>, Mf4Error> {
        loop {
            if self.stream.is_none() && !self.open_next_group()? {
                return Ok(None);
            }

            let offset = self.sample_offset;
            let Some(stream) = self.stream.as_mut() else {
                return Ok(None);
            };

            let step = match stream.next() {
                None => Ok(None),
                Some(Err(e)) => Err(e),
                Some(Ok(signals)) if signals.is_empty() => Ok(None),
                Some(Ok(signals)) => {
                    let count = signals[0].len();
                    let series = stream.signals_to_series(&signals, offset)?;
                    let mut buf = Vec::new();
                    ::falcon_mdf::export::write_arrow_ipc(&series, &mut buf)?;
                    Ok(Some((buf, count)))
                }
            };

            match step {
                Err(err) => {
                    self.finish();
                    self.pending.clear();
                    return Err(err);
                }
                Ok(None) => self.finish(),
                Ok(Some((buf, count))) => {
                    self.sample_offset += count;
                    return Ok(Some(buf));
                }
            }
        }
    }
}

/// Chunked MDF 3.x group iterator.
struct V3Stream {
    file: Arc<Mdf3File>,
    pending: VecDeque<(usize, usize, Vec<usize>)>,
    current: Option<(Vec<SignalSeries>, usize)>,
    chunk_size: usize,
}

impl V3Stream {
    fn next_ipc_chunk(&mut self) -> Result<Option<Vec<u8>>, Mf4Error> {
        loop {
            if let Some((series, offset)) = &mut self.current {
                let total = series.first().map_or(0, SignalSeries::len);
                if *offset < total {
                    let end = (*offset + self.chunk_size).min(total);
                    let window: Vec<SignalSeries> = series
                        .iter()
                        .map(|s| slice_series(s, *offset, end))
                        .collect::<Result<_, _>>()?;
                    *offset = end;
                    let mut buf = Vec::new();
                    ::falcon_mdf::export::write_arrow_ipc(&window, &mut buf)?;
                    return Ok(Some(buf));
                }
                self.current = None;
            }

            let Some((dg_idx, cg_idx, ch_indices)) = self.pending.pop_front() else {
                return Ok(None);
            };
            let series: Vec<SignalSeries> = ch_indices
                .iter()
                .map(|&ch| v3_time_series(&self.file, dg_idx, cg_idx, ch))
                .collect::<Result<_, _>>()
                .inspect_err(|_| {
                    self.current = None;
                    self.pending.clear();
                })?;
            if series.first().is_some_and(|s| !s.is_empty()) {
                self.current = Some((series, 0));
            }
        }
    }
}

enum IterState {
    V4(Box<V4Stream>),
    V3(V3Stream),
}

/// Python iterator yielding streaming DataFrames from an MDF file.
#[pyclass(name = "DataFrameIterator")]
pub struct DataFrameIterator {
    state: IterState,
    backend: String,
}

#[pymethods]
impl DataFrameIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let maybe_buf = py.detach(|| {
            match &mut self.state {
                IterState::V4(v4) => v4.next_ipc_chunk(),
                IterState::V3(v3) => v3.next_ipc_chunk(),
            }
            .map_err(py_err)
        })?;
        match maybe_buf {
            None => Ok(None),
            Some(buf) => ipc_to_dataframe(py, &buf, &self.backend).map(Some),
        }
    }
}

/// Opens an MDF (v3 or v4) file at `path` and returns an `Mf4File` object.
///
/// `path` may be a `str` or any `os.PathLike` (such as `pathlib.Path`).
/// Raises `FileNotFoundError` if the path does not exist and `ValueError` if
/// the file is not valid MDF.
#[pyfunction]
fn open(py: Python<'_>, path: PathBuf) -> PyResult<Mf4FilePy> {
    let inner = py
        .detach(|| match Mf4File::open(&path) {
            Ok(v4) => Ok(Inner::V4(Arc::new(v4))),
            Err(Mf4Error::UnsupportedVersion { major: 2 | 3, .. }) => {
                Mdf3File::open(&path).map(|v3| Inner::V3(Arc::new(v3)))
            }
            Err(e) => Err(e),
        })
        .map_err(py_err)?;
    Ok(Mf4FilePy {
        inner: RwLock::new(Some(inner)),
    })
}

/// The `falcon_mdf` extension module.
#[pymodule]
fn falcon_mdf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(open, m)?)?;
    m.add_class::<Mf4FilePy>()?;
    m.add_class::<DataFrameIterator>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|&x| x.to_string()).collect()
    }

    #[test]
    fn unique_names_leaves_distinct_columns_unchanged() {
        let names = s(&["time", "Speed", "Boost"]);
        assert_eq!(unique_names(&names), names);
    }

    #[test]
    fn unique_names_disambiguates_duplicates_without_clobbering_existing() {
        let names = s(&["time", "time", "t", "t", "t_1"]);
        assert_eq!(
            unique_names(&names),
            s(&["time", "time_1", "t", "t_2", "t_1"])
        );
    }

    #[test]
    fn usable_shape_validates_product_and_rejects_zero() {
        assert_eq!(usable_shape(Some(&[2, 3]), 6), Some(vec![2, 3]));
        assert_eq!(usable_shape(Some(&[4]), 4), Some(vec![4]));
        assert_eq!(usable_shape(Some(&[2, 3]), 5), None);
        assert_eq!(usable_shape(Some(&[0, 3]), 0), None);
        assert_eq!(usable_shape(Some(&[]), 0), None);
        assert_eq!(usable_shape(None, 4), None);
    }

    #[test]
    fn slice_signal_values_slices_varbytes_and_arrays() {
        let vb = SignalValues::VarBytes {
            data: vec![1, 2, 3, 4, 5],
            starts: vec![0, 2, 2, 5],
        };
        let sliced = slice_signal_values(&vb, 1, 3).unwrap();
        assert_eq!(sliced.len(), 2);
        assert_eq!(sliced.bytes_at(0), Some(&[][..]));
        assert_eq!(sliced.bytes_at(1), Some(&[3, 4, 5][..]));

        let arr = SignalValues::Array {
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            elements_per_sample: 2,
        };
        let sliced_arr = slice_signal_values(&arr, 1, 3).unwrap();
        assert_eq!(
            sliced_arr,
            SignalValues::Array {
                values: vec![3.0, 4.0, 5.0, 6.0],
                elements_per_sample: 2,
            }
        );
    }
}
