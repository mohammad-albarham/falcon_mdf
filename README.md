# falcon_mdf

A high-performance Rust library for reading ASAM MDF (Measurement Data Format) v4.x files.

[![Crates.io](https://img.shields.io/crates/v/falcon_mdf.svg)](https://crates.io/crates/falcon_mdf)
[![Documentation](https://docs.rs/falcon_mdf/badge.svg)](https://docs.rs/falcon_mdf)
[![License](https://img.shields.io/crates/l/falcon_mdf.svg)](LICENSE)

## Overview

**falcon_mdf** reads MDF4 measurement files, the format automotive and
industrial acquisition tools record to. It aims at three things in this order:

- **Correct, or it says so.** A channel decodes to the right values, or reading
  it fails with a reason. It never returns part of the data, or a raw value in
  place of a converted one, dressed up as a measurement. Decoded output is
  checked against an independent reference implementation over a corpus of CAN,
  LIN and GPS/IMU logs.
- **Safe on files you did not write.** Malformed input produces an error, not a
  panic, an aborted process, or a loop that never ends. Verified by fuzzing the
  whole read path.
- **Fast.** Roughly 2.7× an established reference on uncompressed data and 3.9×
  on compressed, decoding the same samples.

## Features

- Read MDF 4.x files, sorted and unsorted, finished and unfinished
- Memory-mapped and buffered I/O
- HD, DG, CG, CN, DT, DZ, DL, HL, TX, MD, CC and SI blocks
- Compressed data blocks, plain and transposed deflate
- Typed samples: an integer channel decodes to an integer of its own width, a
  frame payload to bytes, a text table to text
- Variable-length signal data, in both storage forms
- Array (CA) channels whose elements sit in the record, decoded to flat values
  with the per-dimension shape available — including look-up arrays composed of
  nested CA blocks, and arrays whose length varies per sample
- The acquisition source behind a channel or group: which ECU, bus or tool it
  came from
- Conversion rules: identity, linear, rational, algebraic formulas, value and
  range tables, value-to-text, text-keyed and bitfield tables
- CAN frames out of bus-logged groups: timestamp, identifier, extended flag,
  bus channel and a payload trimmed to the logged length — with no database
  needed, and no interpretation of the payload
- Streamed reading of a channel in bounded windows, so peak memory does not
  scale with the largest data group — including unsorted groups, which are
  demultiplexed per window, and bus-log payload channels
- CAN payloads decoded against a CAN database into named physical signals: bit
  position and width, Intel and Motorola byte order, signedness, scaling,
  multiplexed signals and `VAL_` tables decoded to text — from DBC files (`dbc`
  feature) or AUTOSAR ECU extracts (`arxml` feature), both through one decoder
- Decoded signals as time series, not just frame by frame: `decode_bus` returns
  each signal with all of its readings and their timestamps, keyed by bus,
  message and name together
- J1939 parameter-group matching, so a heavy-duty database written against one
  ECU still matches frames from every other
- Per-sample validity from invalidation bits
- Metadata as a comment plus named properties, rather than raw XML
- Attachments (embedded data only), events, channel hierarchy and sample
  reduction blocks

### Not supported

Named so you can tell before you depend on it:

- **Writing anything beyond a simple subset.** `Mf4Writer` creates files from
  scratch — one data group per channel group, records sorted by time, raw
  little-endian float64 samples, an implicit `Time` master, invalidation bits
  when the caller supplies validity. No conversions, no arrays, no VLSD, no
  compression, and no read-modify-write round trip.
- **MDF 3.x.**
- **J1939 source-address matching, DBC extended multiplexing (`SG_MUL_VAL_`), DBC
  value tables, and the dynamic parts of a multiplexed ARXML PDU.** The last is
  left out rather than reported, because which part applies is chosen by a selector
  field this build does not resolve.
- **Streamed reading of a variable-length channel whose payloads sit in its own
  signal-data block.** `signal_chunks` refuses it by name rather than reading it
  wrongly; `signal` reads it, materialising the group. The companion-group form
  that bus loggers write *is* streamed.
- **Arrays stored one channel group or data group per element**
  (`ca_storage` CG- and DG-template), and arrays with more than one
  dynamically-sized dimension. No file we have access to writes either, so a
  decoder for them could not be checked against anything.
- **Sync channels** (`cn_type` 4), which index a media stream rather than
  measure something.

Each of these reports itself by name through `Mf4Error::Unsupported` when you
read such a channel — the rest of the file still opens and decodes.

### Tested against

Every claim above is exercised by the test suite. Two areas are implemented but
have no file available to test them: **big-endian channels** are covered by
synthetic tests only, and only **MDF 4.11** has been read from a real file —
4.0 and 4.2 are supported in principle. See `CHANGELOG.md` for the full list of
known limitations.

## Installation

```toml
[dependencies]
falcon_mdf = "0.3"
```

Memory mapping is on by default. For a file another process may be writing, or
one on a network share, open it buffered instead — and note that for large files
the buffered backend also uses roughly half the memory, since mapped data ends
up resident both as pages and as the assembled buffer.

```toml
[dependencies]
falcon_mdf = { version = "0.3", default-features = false }
```

Decoding CAN payloads against a database needs the `dbc` feature (DBC files) or
`arxml` (AUTOSAR ECU extracts). Both are off by default so that reading plain
measurement files does not pull in a database parser.

```toml
[dependencies]
falcon_mdf = { version = "0.3", features = ["dbc", "arxml"] }
```

The crate's MSRV is **1.88**, and it covers every feature: CI builds
`--all-features` on 1.88 on every push. The floor is set by `autosar-data`,
which the `arxml` feature pulls in; without that feature the crate builds on
considerably less, but a declared MSRV that only holds for some feature
combinations is not a number anyone can rely on.

## Quickstart

### Opening an MDF4 File

```rust
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open with memory-mapped I/O (recommended for large files)
    let file = Mf4File::open("measurement.mf4")?;
    
    // Print file version and statistics
    println!("Version: {}", file.version());
    println!("Data groups: {}", file.data_group_count());
    println!("Total channels: {}", file.channel_count());
    println!("Start time: {:?}", file.start_time());
    
    Ok(())
}
```

### Reading a channel

Samples come back in the channel's own type. A 29-bit CAN identifier is a
`u32`, a two-bit bus number a `u8`, a frame payload bytes — nothing is forced
through `f64` unless you ask for it.

```rust
use falcon_mdf::{Mf4File, SignalValues};

let file = Mf4File::open("measurement.mf4")?;

if let Some(channel) = file.find_channel("VehicleSpeed") {
    let signal = file.signal(channel)?;

    match signal.values()? {
        SignalValues::F64(v) => println!("first: {} {}", v[0], signal.unit()),
        SignalValues::U32(v) => println!("first: {}", v[0]),
        other => println!("{} samples of {}", other.len(), other.kind().name()),
    }

    // Or a uniform numeric view, lossy for wide integers and byte channels.
    let as_f64 = signal.values_f64()?;
    println!("{} samples", as_f64.len());
}
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

### Samples the file marks invalid

A channel may carry an invalidation bit. `values()` does not filter those out —
dropping them would break alignment with the master channel — so check validity
alongside the data.

```rust
# use falcon_mdf::Mf4File;
# let file = Mf4File::open("measurement.mf4")?;
# let channel = file.find_channel("VehicleSpeed").unwrap();
let signal = file.signal(channel)?;
let values = signal.values_f64()?;

match signal.validity() {
    Some(valid) => {
        for (value, ok) in values.iter().zip(&valid) {
            if *ok {
                println!("{value}");
            }
        }
    }
    // No invalidation bit: every sample is valid.
    None => println!("{} valid samples", values.len()),
}
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

### Channels this build cannot decode

Reading fails rather than returning something plausible. Handle it explicitly if
you process files you have not seen.

```rust
# use falcon_mdf::{Mf4File, error::Mf4Error};
# let file = Mf4File::open("measurement.mf4")?;
for channel in file.channels() {
    match file.signal(channel).and_then(|s| s.values()) {
        Ok(values) => println!("{}: {} samples", channel.name, values.len()),
        Err(Mf4Error::Unsupported { feature, .. }) => {
            println!("{}: skipped, needs {feature}", channel.name)
        }
        Err(e) => return Err(e),
    }
}
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

### File metadata

```rust
# use falcon_mdf::Mf4File;
# let file = Mf4File::open("measurement.mf4")?;
println!("{}", file.comment());

if let Some(serial) = file.metadata().get("Device Information/serial number") {
    println!("recorded by {serial}");
}
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

### Writing a file

`Mf4Writer` creates MF4 files from scratch: one data group per channel group,
an implicit `Time` master per group, records sorted by time, raw float64
samples. Validity can be carried over per sample, so an export keeps the gaps
the source declared.

```rust
# use falcon_mdf::Mf4Writer;
let mut writer = Mf4Writer::new();
let group = writer.add_group(&[0.0, 0.1, 0.2])?;
group.add_channel("Speed", "km/h", &[0.0, 5.0, 10.0])?;
group.add_channel_with_validity(
    "Boost", "psi", &[1.0, 2.0, 3.0], Some(&[true, false, true]),
)?;
writer.write_to_file("out.mf4")?;
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

### Decoding a bus log

A bus logger records raw frames; what the payload bytes mean lives in a DBC or
ARXML database you supply. `decode_bus` reads the whole file against one and
hands back each signal as a time series.

```rust,no_run
use falcon_mdf::{CanDatabase, IdMatching, Mf4File};

let file = Mf4File::open("truck.mf4")?;
let database = CanDatabase::from_dbc_path("j1939.dbc")?
    // A J1939 database keys messages by parameter group, while the identifier
    // on the wire also carries the sending ECU's address. Without this, a real
    // heavy-duty log decodes to nothing.
    .with_matching(IdMatching::J1939Pgn);

for signal in file.decode_bus(&database)?.iter() {
    println!(
        "{}.{}: {} readings [{}]",
        signal.message, signal.name, signal.len(), signal.unit
    );
    // Enum-valued signals carry their `VAL_` label as well as the number.
    if let Some(text) = signal.text_at(0) {
        println!("  first reading: {text}");
    }
}
# Ok::<(), falcon_mdf::error::Mf4Error>(())
```

Decoded signals are a namespace of their own — they do **not** appear in
`channels()`, because they are derived from a database the file does not
contain. A series is identified by bus, message and signal name together: two
messages may spell one signal name, and the same identifier on two buses is two
different signals.

For frame-level access with no database at all, `can_frame_groups` and
`can_frames` give the timestamp, identifier and payload directly. See
`examples/decode_bus.rs` for the whole path in one file.

## Architecture

The library is organized in layers, each with a clear responsibility:

```
┌─────────────────────────────────────────────┐
│                  file.rs                     │  User-facing API
│            (Mf4File, high-level)             │
├─────────────────────────────────────────────┤
│                  model/                      │  Domain types
│      (DataGroup, Channel, Signal, etc.)     │
├─────────────────────────────────────────────┤
│                 parser/                      │  Version-aware parsing
│    (Mf4Version, LinkedBlockIterator)        │
├─────────────────────────────────────────────┤
│                 blocks/                      │  Low-level block types
│  (HdBlock, DgBlock, CnBlock, DtBlock, etc.) │
├─────────────────────────────────────────────┤
│                   io/                        │  File access abstraction
│     (ByteSource, MmapSource, Buffered)      │
└─────────────────────────────────────────────┘
```

### Module Descriptions

| Module | Description |
|--------|-------------|
| `io/` | File I/O abstraction with mmap and buffered backends |
| `blocks/` | Low-level MDF4 block parsers following the ASAM spec |
| `parser/` | Version detection and block traversal utilities |
| `model/` | High-level types representing channels, signals, and metadata |
| `file.rs` | Main `Mf4File` API for opening and reading files |
| `write.rs` | `Mf4Writer` — creating MF4 files from scratch |
| `export.rs` | `write_csv` — decoded channels as CSV |
| `error.rs` | Comprehensive error types with `thiserror` |

## Performance

Median of fifteen runs against an established reference implementation, both
decoding the same 326,623 samples from the same files:

| Read | Reference | falcon_mdf | |
|---|---|---|---|
| Uncompressed | 3.74 ms | **1.36 ms** | 2.7× |
| DZ-compressed | 9.34 ms | **2.38 ms** | 3.9× |

Opening a file — parsing its structure without reading samples — is roughly an
order of magnitude quicker again, which matters when you only want to know what
a file contains.

### Choosing a backend

| Backend | When | Trade-off |
|---|---|---|
| Memory-mapped (default) | Files that are finished being written | Fastest reads. The file must not be modified while open: another process truncating it raises `SIGBUS`, which is not a catchable Rust error. |
| Buffered | Files still being written, on a network share, or that another user can replace. Also large files. | Copies what it reads, so it carries no such requirement — and uses roughly half the memory on large files. |

### Memory

A data group's records are assembled into one buffer before they are read, so
peak memory scales with the **largest data group**, not with the file. Under the
memory-mapped backend the data is resident twice — once as mapped pages, once as
that buffer.

Measured reading a 416 MB file:

| Backend | Peak resident |
|---|---|
| Memory-mapped | 826 MB |
| Buffered | 434 MB |

If you are reading files of that size, prefer `Mf4File::open_buffered`. Decoding
block by block, which would make memory independent of group size, is planned
but not implemented.

### Build settings

The release profile in this repository already sets these; if you vendor the
crate, they are worth keeping:

```toml
[profile.release]
lto = true
codegen-units = 1
opt-level = 3
```

## Extending Version Support

The library uses a trait-based approach for version-specific parsing:

```rust
pub trait VersionedParser {
    fn version(&self) -> Mf4Version;

    fn can_handle(&self, version: Mf4Version) -> bool;
    fn hd_block_offset(&self) -> u64;
    fn supports_sample_reduction(&self) -> bool;
    fn supports_events(&self) -> bool;
    fn supports_attachments(&self) -> bool;
}
```

Every method but `version` has a default, so a new version's parser states only
where it differs.

To add support for a new MDF version:

1. Add the version variant to `Mf4Version` in `parser/version.rs`
2. Implement version-specific parsing logic where block structures differ
3. Update `Mf4Version::is_supported()` and feature detection methods

## MDF4 Block Reference

| Block | Description |
|-------|-------------|
| **ID** | File identification (MDF signature, version) |
| **HD** | Header block (file metadata, timestamps) |
| **DG** | Data group (logical grouping of channels) |
| **CG** | Channel group (channels with shared time axis) |
| **CN** | Channel block (signal definition) |
| **DT** | Data block (raw measurement data) |
| **DZ** | Zipped data block (compressed) |
| **DL** | Data list (linked data blocks) |
| **HL** | Hierarchy list (nested block structure) |
| **TX** | Text block (strings) |
| **MD** | Metadata block (XML content) |
| **CC** | Conversion block (value transformation) |
| **SI** | Source information (ECU, tool metadata) |

## Error Handling

All operations return `Result<T, Mf4Error>`:

```rust
use falcon_mdf::{Mf4File, Mf4Error};

fn open_file(path: &str) -> Result<(), Mf4Error> {
    let file = Mf4File::open(path)?;
    // ... use file
    Ok(())
}
```

Error types include:
- `Mf4Error::Io` - File I/O errors
- `Mf4Error::InvalidSignature` - Not a valid MDF file
- `Mf4Error::UnsupportedVersion` - Unknown MDF version
- `Mf4Error::InvalidBlockId` / `InvalidBlockSize` - Malformed block structure
- `Mf4Error::Decompression` - Zlib decompression failure
- `Mf4Error::ChannelNotFound` - Channel lookup failed
- `Mf4Error::Unsupported` - A channel this build cannot decode, named by feature

## Examples

The `examples/` directory contains:

- **list_channels.rs** - Enumerate all channels in a file
- **export_to_csv.rs** - Export a channel to CSV format
- **write_mf4.rs** - Create an MF4 file with `Mf4Writer`

Run examples with:

```bash
cargo run --example list_channels -- measurement.mf4
cargo run --example export_to_csv -- measurement.mf4 VehicleSpeed output.csv
cargo run --example write_mf4 -- out.mf4
```

## Testing

Run the test suite:

```bash
cargo test
```

Run with logging:

```bash
RUST_LOG=debug cargo test
```

## GUI

`gui/` contains `falcon`, a desktop viewer built on this crate: browsing and
searching channels, overlay and stacked plots with min-max decimation,
invalid samples drawn as gaps, undecodable channels shown with their reason,
events, attachments, file history, the channel hierarchy, and CSV export.
The library itself stays GUI-free; build and packaging notes live in
[gui/PACKAGING.md](gui/PACKAGING.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## References

- [ASAM MDF Standard](https://www.asam.net/standards/detail/mdf/)

## Acknowledgments

This library was designed to provide a robust, performant foundation for working with measurement data in Rust. Special thanks to the ASAM organization for the MDF specification.
