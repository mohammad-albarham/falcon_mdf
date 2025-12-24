# falcon_mdf

A high-performance Rust library for reading ASAM MDF (Measurement Data Format) v4.x files.

[![Crates.io](https://img.shields.io/crates/v/falcon_mdf.svg)](https://crates.io/crates/falcon_mdf)
[![Documentation](https://docs.rs/falcon_mdf/badge.svg)](https://docs.rs/falcon_mdf)
[![License](https://img.shields.io/crates/l/falcon_mdf.svg)](LICENSE)

## Overview

**falcon_mdf** provides efficient, zero-copy parsing of MDF4 files commonly used in automotive and industrial data acquisition. The library is designed with:

- **High Performance**: Memory-mapped I/O for zero-copy data access
- **Low Allocations**: Careful memory management and lazy evaluation
- **Version Extensibility**: Layered architecture supporting MDF 4.0, 4.1, and 4.2
- **Idiomatic Rust**: Type-safe API with comprehensive error handling
- **Flexibility**: Multiple I/O backends (mmap, buffered)

## Features

- ✅ Read MDF 4.0, 4.1, 4.2 files
- ✅ Memory-mapped and buffered file access
- ✅ Parse all major block types (HD, DG, CG, CN, DT, DZ, TX, CC, SI)
- ✅ Compressed data block support (zlib deflate)
- ✅ Data list and hierarchy blocks (DL, HL)
- ✅ Signal conversion rules (linear, rational, polynomial, etc.)
- ✅ Channel metadata and source information
- ✅ Comprehensive error handling

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
falcon_mdf = "0.1"
```

For memory-mapped I/O (enabled by default):

```toml
[dependencies]
falcon_mdf = { version = "0.1", features = ["mmap"] }
```

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
    
    Ok(())
}
```

### Listing All Channels

```rust
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    
    for dg in file.data_groups() {
        println!("Data Group {} ({} samples)", dg.index, dg.sample_count);
        
        for cg in &dg.channel_groups {
            for ch in &cg.channels {
                println!("  - {} [{}]", ch.name, ch.unit);
            }
        }
    }
    
    Ok(())
}
```

### Reading Signal Data

```rust
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    
    // Find a channel by name
    if let Some((dg_idx, cg_idx, ch_idx)) = file.find_channel("VehicleSpeed") {
        // Read the signal with converted values
        let signal = file.signal(dg_idx, cg_idx, ch_idx)?;
        
        println!("Channel: {} [{}]", signal.channel_name(), signal.unit());
        println!("Samples: {}", signal.len());
        
        // Iterate over values with timestamps
        for point in signal.iter().take(10) {
            if let Some(ts) = point.timestamp {
                println!("  t={:.3}s: {:?}", ts, point.value);
            }
        }
    }
    
    Ok(())
}
```

### Exporting to CSV

```rust
use std::fs::File;
use std::io::Write;
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    let mut csv = File::create("output.csv")?;
    
    if let Some((dg, cg, ch)) = file.find_channel("EngineRPM") {
        let signal = file.signal(dg, cg, ch)?;
        
        writeln!(csv, "timestamp,value")?;
        for point in signal.iter() {
            if let (Some(ts), falcon_mdf::SignalValue::Float(v)) = (point.timestamp, point.value) {
                writeln!(csv, "{:.6},{:.6}", ts, v)?;
            }
        }
    }
    
    Ok(())
}
```

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
| `error.rs` | Comprehensive error types with `thiserror` |

## Performance

### I/O Strategies

The library supports multiple I/O backends optimized for different use cases:

| Strategy | Use Case | Pros | Cons |
|----------|----------|------|------|
| **Memory-mapped** (default) | Large files, random access | Zero-copy, OS caching | Requires file on disk |
| **Buffered** | Streaming, network files | Works with any `Read` | More allocations |

Memory-mapped I/O is recommended for most use cases and is enabled by default.

### Zero-Copy Design

- Block headers are parsed in-place without copying
- String data uses borrowed slices where possible
- Signal data can be accessed without full file loading

### Memory Considerations

- File is memory-mapped, not loaded entirely into RAM
- OS manages page caching automatically
- Large files (10+ GB) are handled efficiently

### Benchmarking Tips

For optimal performance:

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
    
    fn parse_header(&self, data: &[u8]) -> Result<HdBlock, Mf4Error>;
    fn parse_channel(&self, data: &[u8], offset: u64) -> Result<CnBlock, Mf4Error>;
    // ... other version-specific methods
}
```

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
- `Mf4Error::InvalidBlock` - Malformed block structure
- `Mf4Error::Decompression` - Zlib decompression failure
- `Mf4Error::ChannelNotFound` - Channel lookup failed

## Examples

The `examples/` directory contains:

- **list_channels.rs** - Enumerate all channels in a file
- **export_to_csv.rs** - Export a channel to CSV format

Run examples with:

```bash
cargo run --example list_channels -- measurement.mf4
cargo run --example export_to_csv -- measurement.mf4 VehicleSpeed output.csv
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

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## References

- [ASAM MDF Standard](https://www.asam.net/standards/detail/mdf/)
- [MDF4 Specification](https://www.asam.net/index.php?eID=dumpFile&t=f&f=4412&token=...)

## Acknowledgments

This library was designed to provide a robust, performant foundation for working with measurement data in Rust. Special thanks to the ASAM organization for the MDF specification.
