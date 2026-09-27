# Installation

Add the crate to your Rust project:

```toml
[dependencies]
falcon_mdf = "0.7"
```

This includes the memory-mapped I/O backend, parallel decoding, and deflate compression. The crate declares Rust **1.89** as its minimum supported version.

If the file may change while it is open, use `Mf4File::open_buffered` instead of the default memory-mapped backend. A file being truncated while memory mapped can raise `SIGBUS`.

## Select features

Enable only the formats and integrations you need:

```toml
[dependencies]
falcon_mdf = { version = "0.7", features = ["dbc", "zstd", "mdf3"] }
```

| Feature | Adds |
| --- | --- |
| `mdf3` | MDF 2.x and 3.x reader |
| `dbc` | CAN database decoding from DBC |
| `arxml` | CAN database decoding from AUTOSAR ARXML |
| `zstd`, `lz4` | Additional MDF 4 compressed block codecs |
| `parquet`, `arrow` | Columnar export and Arrow IPC |
| `mat`, `mat4`, `mat73`, `hdf5` | MATLAB and HDF5 export |
| `asc` | Vector ASC trace export |

`mmap` and `parallel` are enabled by default. Disable default features if you need a smaller build:

```toml
[dependencies]
falcon_mdf = { version = "0.7", default-features = false }
```

See the [complete feature table](../reference/features.md#feature-flags) for details. A block compressed with a codec that is not enabled produces a named unsupported-feature error.

## Next step

Continue with the [Rust quickstart](quickstart.md), or [open a file in the browser](../integrations/browser.md).
