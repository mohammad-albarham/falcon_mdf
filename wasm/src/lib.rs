//! WebAssembly bindings for `falcon_mdf` via `wasm-bindgen`.
//!
//! Exposes a reading API over in-memory MF4 files for WebAssembly and JavaScript runtimes:
//!
//! - [`WasmMf4File::new`] reads an MF4 file from raw bytes (e.g. `Uint8Array`).
//! - [`WasmMf4File::channel_names`] lists every channel name in the file as a JSON array of strings.
//! - [`WasmMf4File::channel_count`] returns the total number of channels.
//! - [`WasmMf4File::signal`] returns a channel's samples as a JSON object with timestamps and values.
//! - [`WasmMf4File::info`] returns file metadata (version, start time, group and channel counts) as a JSON object.

use std::fmt::Write;
use wasm_bindgen::prelude::*;

use falcon_mdf::error::Mf4Error;
use falcon_mdf::Mf4File;

/// Converts an error into a [`JsValue`] carrying the error message as a thrown JavaScript error.
fn js_err(err: impl std::fmt::Display) -> JsValue {
    #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
    {
        JsError::new(&err.to_string()).into()
    }
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    {
        let _ = err;
        JsValue::NULL
    }
}

/// Escapes a string for JSON output according to RFC 8259 and appends it to `out`.
///
/// Handles quotation marks, reverse solidi, standard escape characters (`\b`, `\f`, `\n`, `\r`, `\t`),
/// and control characters below 0x20 formatted as `\u00XX`.
pub fn escape_json_str_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Escapes a string for JSON output according to RFC 8259.
pub fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_json_str_into(s, &mut out);
    out
}

/// Appends an `f64` value to a JSON output buffer.
///
/// Finite numbers are formatted as decimal floating-point representations.
/// Non-finite numbers (`NaN`, `+inf`, `-inf`) are formatted as `null` as required by JSON.
pub fn write_f64(out: &mut String, val: f64) {
    if val.is_finite() {
        let _ = write!(out, "{}", val);
    } else {
        out.push_str("null");
    }
}

/// An MF4 file held in browser memory.
#[wasm_bindgen]
pub struct WasmMf4File {
    inner: Mf4File,
}

#[wasm_bindgen]
impl WasmMf4File {
    /// Reads a file from bytes, e.g. a `Uint8Array` from `fetch` or a file input.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>) -> Result<WasmMf4File, JsValue> {
        let inner = Mf4File::from_bytes(bytes).map_err(js_err)?;
        Ok(WasmMf4File { inner })
    }

    /// Every channel name in the file, as a JSON array of strings.
    pub fn channel_names(&self) -> Result<String, JsValue> {
        let mut out = String::from("[");
        for (i, name) in self.inner.channel_names().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_json_str_into(name, &mut out);
            out.push('"');
        }
        out.push(']');
        Ok(out)
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        self.inner.channel_count()
    }

    /// One channel's samples as a JSON object with `name`, `unit`,
    /// `timestamps` and `values` arrays.
    ///
    /// Non-finite floats (`NaN`, `+inf`, `-inf`) are not valid JSON and are emitted
    /// as `null` in both `timestamps` and `values` arrays.
    pub fn signal(&self, name: &str) -> Result<String, JsValue> {
        let channel = self
            .inner
            .find_channel(name)
            .ok_or_else(|| Mf4Error::ChannelNotFound {
                name: name.to_string(),
            })
            .map_err(js_err)?;

        let series = self.inner.time_series(channel).map_err(js_err)?;
        let values = series.values.to_f64();
        let timestamps = series.timestamps;

        let mut out = String::new();
        out.push_str("{\"name\":\"");
        escape_json_str_into(&channel.name, &mut out);
        out.push_str("\",\"unit\":\"");
        escape_json_str_into(&channel.unit, &mut out);
        out.push_str("\",\"timestamps\":[");
        for (i, &t) in timestamps.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, t);
        }
        out.push_str("],\"values\":[");
        for (i, &v) in values.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_f64(&mut out, v);
        }
        out.push_str("]}");
        Ok(out)
    }

    /// Version, start time, group and channel counts, as a JSON object.
    pub fn info(&self) -> Result<String, JsValue> {
        let channel_group_count: usize = self
            .inner
            .data_groups()
            .iter()
            .map(|dg| dg.channel_groups.len())
            .sum();

        let mut out = String::new();
        out.push_str("{\"version\":\"");
        escape_json_str_into(&self.inner.version().to_string(), &mut out);
        out.push_str("\",\"start_time\":\"");
        escape_json_str_into(&self.inner.start_time().to_iso8601(), &mut out);
        out.push_str("\",\"channel_group_count\":");
        let _ = write!(
            out,
            "{},\"channel_count\":{}",
            channel_group_count,
            self.inner.channel_count()
        );
        out.push('}');
        Ok(out)
    }
}
