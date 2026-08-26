//! Python bindings for `falcon_mdf` via PyO3.
//!
//! Exposes a minimal, Pythonic API over the Rust MF4 reader:
//!
//! - `falcon_mdf.open(path)` opens a file and returns an `Mf4File`.
//! - `Mf4File.channels()` lists channel names.
//! - `Mf4File.get(name)` returns `(values, timestamps)` as lists of `float`.
//! - `Mf4File.info()` returns a dict with version and channel counts.

use ::falcon_mdf::{Mf4Error, Mf4File};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Converts a Rust [`Mf4Error`] into a Python `RuntimeError` carrying the
/// original message. Required explicitly because `Mf4Error` does not implement
/// `Send + Sync`, so PyO3 cannot provide the automatic `From` conversion.
fn py_err(err: Mf4Error) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(err.to_string())
}

/// Python wrapper around [`falcon_mdf::Mf4File`].
#[pyclass(name = "Mf4File")]
pub struct Mf4FilePy {
    inner: Mf4File,
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
    Ok(())
}
