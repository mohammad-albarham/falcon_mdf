# Features

Falcon MDF exposes a Rust API for measurement data, with Python and WebAssembly consumers built on the same core library.

## Reading and decoding

| Area | Support |
| --- | --- |
| MDF versions | MDF 4.0, 4.1, 4.2; MDF 2.00–3.30 with `mdf3` |
| Storage | Sorted and unsorted groups; mapped, buffered, and custom byte sources |
| Compression | Deflate; optional zstd and LZ4, including transposed variants |
| Channel values | Integers, floats, text, bytes, variable-length data, arrays, and per-sample validity |
| Conversions | Linear, rational, formulas, numeric tables, text tables, and bitfields |
| Metadata | Attachments, events, source information, hierarchy, and reduction levels |
| Bus logging | CAN, LIN, Ethernet, and FlexRay frame extraction |
| Database decoding | DBC, AUTOSAR ARXML, and LIN Description Files |

The reader can select channels, decode several in one pass, stream windows of a channel group, cut or resample series, and combine files by concatenating or stacking measurements.

## Writing and export

`Mf4Writer` writes MF4 with typed channels, per-sample validity, conversions, CA arrays, variable-length strings and bytes, multiple channel groups, and supported compression codecs. Decoded data can be exported to CSV. Optional features enable Parquet, Arrow IPC, MATLAB MAT, HDF5, and Vector ASC export.

## Feature flags

| Flag | Default | Purpose |
| --- | --- | --- |
| `mmap` | On | Memory-mapped I/O |
| `parallel` | On | Parallel decoding; off for `wasm32` |
| `dbc` | Off | DBC parsing |
| `arxml` | Off | AUTOSAR ARXML parsing |
| `mdf3` | Off | MDF 2.x and 3.x reader |
| `zstd`, `lz4` | Off | Additional compressed block codecs |
| `parquet`, `arrow` | Off | Columnar export |
| `mat`, `mat4`, `mat73`, `hdf5` | Off | MATLAB and HDF5 export |
| `asc` | Off | Vector ASC export |

The [Rust API reference](https://docs.rs/falcon_mdf) lists exact methods and types. The [field reference](https://mohammad-albarham.github.io/falcon_mdf/field-reference.html) tracks block-level implementation status and names the commit its claims describe.
