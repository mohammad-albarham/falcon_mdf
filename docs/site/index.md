# Falcon MDF

Read, write, and explore ASAM MDF measurements with Rust.

Falcon MDF handles MDF 4.x measurements and, with the `mdf3` feature, MDF 2.x and 3.x. It gives you typed channel values, timestamps, validity, conversions, bus frames, and access to file structure. A browser viewer and Python bindings use the same core reader.

## Where to start

- **New Rust project:** [Install the crate](getting-started/installation.md), then [open and read a channel](getting-started/quickstart.md).
- **Existing measurement file:** [Try the browser viewer](integrations/browser.md) without uploading your data.
- **Data analysis in Python:** [Use the local Python bindings](integrations/python.md) and convert channels to DataFrames.
- **Bus log with a database:** [Decode CAN or LIN signals](guides/bus-data.md).

## What it covers

The reader supports MDF 4.0, 4.1, and 4.2, including compressed data, conversions, invalidation, variable-length signals, arrays, attachments, and events. The optional MDF 3 reader covers versions 2.00 through 3.30. The writer creates MF4 files with typed channels, validity, conversions, arrays, and compression. See [features](reference/features.md) for the full list and [limits](reference/limits.md) before relying on a less common layout.

The core API returns an error when a requested channel cannot be decoded. It does not silently substitute raw values for physical values or return only part of a channel.

## Choose an interface

| Interface | Best for | Start here |
| --- | --- | --- |
| Rust crate | Reading, writing, and transforming MDF in an application | [Rust quickstart](getting-started/quickstart.md) |
| Python binding | Exploration with pandas or polars | [Python](integrations/python.md) |
| Browser viewer | Inspecting and plotting a local MF4 file | [Browser and WebAssembly](integrations/browser.md) |
| Desktop viewer | Exploring file structure and channels in a native app | [GUI instructions](https://github.com/mohammad-albarham/falcon_mdf/blob/main/gui/RUNNING.md) |

For exact Rust types and methods, use the [API reference](https://docs.rs/falcon_mdf). For the file format implementation map, use the [field reference](https://mohammad-albarham.github.io/falcon_mdf/field-reference.html).
