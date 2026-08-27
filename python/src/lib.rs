//! Python bindings for `falcon_mdf` via PyO3.
//!
//! Exposes a minimal, Pythonic API over the Rust MF4 reader:
//!
//! - `falcon_mdf.open(path)` opens a file and returns an `Mf4File`.
//! - `Mf4File.channels()` lists channel names.
//! - `Mf4File.get(name)` returns `(values, timestamps)` as lists of `float`.
//! - `Mf4File.info()` returns a dict with version and channel counts.
//! - `Mf4File.to_dataframe()` returns the decoded channels as a pandas or
//!   polars DataFrame via Arrow IPC.

use ::falcon_mdf::{Channel, Mf4Error, Mf4File};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// Converts a Rust [`Mf4Error`] into a Python `RuntimeError` carrying the
/// original message. Required explicitly because `Mf4Error` does not implement
/// `Send + Sync`, so PyO3 cannot provide the automatic `From` conversion.
fn py_err(err: Mf4Error) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(err.to_string())
}

/// Imports a Python module, turning a missing dependency into a clear
/// `ImportError` that names the package to install.
fn import_required<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(name).map_err(|err| {
        if err.is_instance_of::<pyo3::exceptions::PyImportError>(py) {
            PyErr::new::<pyo3::exceptions::PyImportError, _>(format!(
                "{name} is required for to_dataframe(); install it with `pip install {name}`"
            ))
        } else {
            err
        }
    })
}

/// Python wrapper around [`falcon_mdf::Mf4File`].
#[pyclass(name = "Mf4File")]
pub struct Mf4FilePy {
    inner: Mf4File,
}

/// Returns the decoded channels as an Arrow IPC byte stream.
///
/// `channels` is an optional list of channel names; when omitted, every
/// channel in the file is exported. Channels that do not already share a
/// single time axis are resampled onto the first requested channel's time
/// axis with linear interpolation, because an Arrow table has one time
/// column for the whole table.
fn arrow_ipc_bytes(
    file: &Mf4File,
    channels: Option<Vec<String>>,
) -> std::result::Result<Vec<u8>, Mf4Error> {
    let channels: Vec<&Channel> = match channels.as_deref() {
        Some(names) if !names.is_empty() => names
            .iter()
            .map(|name| {
                file.find_channel(name)
                    .ok_or_else(|| Mf4Error::ChannelNotFound { name: name.clone() })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?,
        _ => file.channels().collect::<Vec<_>>(),
    };

    let mut series: Vec<::falcon_mdf::time_ops::SignalSeries> = channels
        .iter()
        .map(|&ch| file.time_series(ch))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Align channels onto a common time axis if they do not already share one.
    if series.len() > 1 {
        let first_ts = series[0].timestamps().to_vec();
        let aligned = series
            .iter()
            .skip(1)
            .all(|s| s.timestamps() == first_ts.as_slice());
        if !aligned {
            series = file.resample(
                &channels,
                ::falcon_mdf::time_ops::Raster::Timestamps(first_ts),
                ::falcon_mdf::time_ops::InterpolationMode::Linear,
            )?;
        }
    }

    let mut buf = Vec::new();
    ::falcon_mdf::export::write_arrow_ipc(&series, &mut buf)?;
    Ok(buf)
}

#[pymethods]
impl Mf4FilePy {
    /// Returns a list with the name of every channel in the file.
    fn channels(&self) -> Vec<String> {
        self.inner
            .channels()
            .map(|ch| ch.name.clone())
            .collect()
    }

    /// Reads one channel by name.
    ///
    /// Returns a `(values, timestamps)` tuple. Both are Python lists of `float`.
    /// Non-numeric channels are represented as `NaN`, matching the behaviour of
    /// the underlying `Signal::values_f64()` path.
    fn get(&self, name: &str) -> PyResult<(Vec<f64>, Vec<f64>)> {
        let channel = self
            .inner
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(py_err)?;

        let series = self.inner.time_series(channel).map_err(py_err)?;
        let values = series.values.to_f64();
        let timestamps = series.timestamps;

        Ok((values, timestamps))
    }

    /// Returns the decoded channels as a pandas or polars DataFrame.
    ///
    /// `channels` is an optional list of channel names; when omitted, every
    /// channel in the file is exported. `backend` is either `"pandas"`
    /// (default) or `"polars"`.
    ///
    /// The data is handed to Python as an Arrow IPC table read with `pyarrow`,
    /// so pandas and polars both see typed, nullable columns.
    #[pyo3(signature = (channels=None, backend="pandas"))]
    fn to_dataframe<'py>(
        &self,
        py: Python<'py>,
        channels: Option<Vec<String>>,
        backend: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ipc_bytes = arrow_ipc_bytes(&self.inner, channels).map_err(py_err)?;

        let pyarrow = import_required(py, "pyarrow")?;
        let ipc_mod = pyarrow.getattr("ipc")?;
        let reader = ipc_mod.call_method1("open_file", (PyBytes::new(py, &ipc_bytes),))?;
        let table = reader.call_method0("read_all")?;

        match backend {
            "pandas" | "pd" => {
                let _pandas = import_required(py, "pandas")?;
                table.call_method0("to_pandas")
            }
            "polars" | "pl" => {
                let polars = import_required(py, "polars")?;
                polars.call_method1("from_arrow", (table,))
            }
            _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "unsupported backend '{backend}'; use 'pandas' or 'polars'"
            ))),
        }
    }

    /// Yields streaming windows of decoded channels as pandas or polars DataFrames.
    ///
    /// `chunk_size` is the maximum number of samples to yield per DataFrame window.
    /// `channels` is an optional list of channel names belonging to the same channel group;
    /// when omitted, every channel in the file (or channel group) is exported.
    /// `backend` is either `"pandas"` (default) or `"polars"`.
    #[pyo3(signature = (chunk_size, channels=None, backend="pandas"))]
    fn iter_to_dataframe<'py>(
        slf: Bound<'py, Self>,
        chunk_size: usize,
        channels: Option<Vec<String>>,
        backend: &str,
    ) -> PyResult<DataFrameIterator> {
        if chunk_size == 0 {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "chunk_size must be greater than 0",
            ));
        }

        match backend {
            "pandas" | "pd" | "polars" | "pl" => {}
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "unsupported backend '{backend}'; use 'pandas' or 'polars'"
                )));
            }
        }

        let file = &slf.borrow().inner;
        let channels: Vec<&Channel> = match channels.as_deref() {
            Some(names) if !names.is_empty() => names
                .iter()
                .map(|name| {
                    file.find_channel(name)
                        .ok_or_else(|| Mf4Error::ChannelNotFound { name: name.clone() })
                })
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(py_err)?,
            _ => file.channels().collect::<Vec<_>>(),
        };

        let stream = file.signals_chunks(&channels, chunk_size).map_err(py_err)?;
        let static_stream: ::falcon_mdf::stream::SignalsChunks<'static> =
            unsafe { std::mem::transmute(stream) };

        Ok(DataFrameIterator {
            _parent: slf.unbind(),
            stream: static_stream,
            backend: backend.to_string(),
            sample_offset: 0,
        })
    }

    /// Returns file metadata as a dictionary.
    ///
    /// Keys: `version` (str), `channel_group_count` (int), `channel_count` (int).
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("version", self.inner.version().to_string())?;

        let channel_group_count: usize = self
            .inner
            .data_groups()
            .iter()
            .map(|dg| dg.channel_groups.len())
            .sum();
        dict.set_item("channel_group_count", channel_group_count)?;
        dict.set_item("channel_count", self.inner.channel_count())?;

        let sample_count: u64 = self
            .inner
            .data_groups()
            .iter()
            .flat_map(|dg| dg.channel_groups.iter())
            .map(|cg| cg.sample_count)
            .max()
            .unwrap_or(0);
        dict.set_item("sample_count", sample_count)?;

        Ok(dict)
    }
}

/// Python iterator yielding streaming DataFrames from an MF4 file.
#[pyclass(name = "DataFrameIterator")]
pub struct DataFrameIterator {
    _parent: Py<Mf4FilePy>,
    stream: ::falcon_mdf::stream::SignalsChunks<'static>,
    backend: String,
    sample_offset: usize,
}

#[pymethods]
impl DataFrameIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let Some(signals_res) = self.stream.next() else {
            return Ok(None);
        };
        let signals = signals_res.map_err(py_err)?;
        if signals.is_empty() {
            return Ok(None);
        }

        let series = self
            .stream
            .signals_to_series(&signals, self.sample_offset)
            .map_err(py_err)?;
        self.sample_offset += signals[0].len();

        let mut buf = Vec::new();
        ::falcon_mdf::export::write_arrow_ipc(&series, &mut buf).map_err(py_err)?;

        let pyarrow = import_required(py, "pyarrow")?;
        let ipc_mod = pyarrow.getattr("ipc")?;
        let reader = ipc_mod.call_method1("open_file", (PyBytes::new(py, &buf),))?;
        let table = reader.call_method0("read_all")?;

        let df = match self.backend.as_str() {
            "pandas" | "pd" => {
                let _pandas = import_required(py, "pandas")?;
                table.call_method0("to_pandas")?
            }
            "polars" | "pl" => {
                let polars = import_required(py, "polars")?;
                polars.call_method1("from_arrow", (table,))?
            }
            _ => unreachable!(),
        };

        Ok(Some(df))
    }
}

/// Opens an MF4 file at `path` and returns an `Mf4File` object.
#[pyfunction]
fn open(path: &str) -> PyResult<Mf4FilePy> {
    let inner = Mf4File::open(path).map_err(py_err)?;
    Ok(Mf4FilePy { inner })
}

/// The `falcon_mdf` extension module.
#[pymodule]
fn falcon_mdf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(open, m)?)?;
    m.add_class::<Mf4FilePy>()?;
    m.add_class::<DataFrameIterator>()?;
    Ok(())
}
