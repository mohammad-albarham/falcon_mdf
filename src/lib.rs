//! # falcon_mdf
//!
//! A high-performance Rust library for reading ASAM MDF v4 (MF4) measurement data files.
//!
//! ## Overview
//!
//! `falcon_mdf` provides a clean, ergonomic API for reading MF4 files, which are commonly
//! used in the automotive industry for storing measurement and calibration data. The library
//! focuses on:
//!
//! - **Performance**: Zero-copy access via memory mapping, lazy data decoding
//! - **Modularity**: Clear separation between I/O, parsing, and data model layers
//! - **Extensibility**: Version-aware design for easy support of different MF4 versions
//! - **Usability**: High-level API that hides format complexity
//!
//! ## Quick Start
//!
//! ```no_run
//! use falcon_mdf::Mf4File;
//!
//! // Open an MF4 file
//! let file = Mf4File::open("measurement.mf4")?;
//!
//! // Print file information
//! println!("MF4 Version: {}", file.version());
//! println!("Channels: {}", file.channel_count());
//!
//! // List all channels
//! for channel in file.channels() {
//!     println!("  {} [{}]", channel.name, channel.unit);
//! }
//!
//! // Read data from a channel
//! if let Some(channel) = file.find_channel("VehicleSpeed") {
//!     let signal = file.signal(channel)?;
//!     let values = signal.values_f64()?;
//!     println!("Speed values: {:?}", &values[..5.min(values.len())]);
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```
//!
//! ## Architecture
//!
//! The crate is organized into several layers:
//!
//! - **I/O Layer** ([`io`]): Abstraction over file access (mmap vs buffered)
//! - **Block Layer** ([`blocks`]): Low-level MF4 block structures and parsing
//! - **Parser Layer** ([`parser`]): Version-aware parsing and block iteration
//! - **Model Layer** ([`model`]): High-level, user-friendly data types
//! - **File API** ([`Mf4File`]): Main entry point for users
//!
//! ## Performance Tips
//!
//! - Use memory-mapped I/O (default with `mmap` feature) for large files
//! - Use iterators instead of `values_f64()` for very large signals
//! - The file structure is parsed eagerly, but sample data is read lazily
//!
//! ## Supported Versions
//!
//! Currently supports MF4 versions 4.0, 4.1, and 4.2. The architecture is designed
//! to easily add support for future versions.
//!
//! ## Relationship to asammdf
//!
//! This library draws design inspiration from the Python [asammdf](https://github.com/danielhrisca/asammdf)
//! library, particularly:
//!
//! - Block caching via [`cache::BlockCache`] (similar to asammdf's `cc_map`, `si_map`)
//! - Channel indexing via [`channels_db::ChannelsDB`] (similar to asammdf's `channels_db`)
//! - Lazy data loading with [`data_index::DataBlockIndex`] (similar to asammdf's `data_blocks`)
//!
//! However, the implementation is fully idiomatic Rust, leveraging:
//! - `Arc<T>` for shared block ownership
//! - Zero-copy memory mapping via `memmap2`
//! - Type-safe enums instead of magic numbers
//! - Parallel parsing with `rayon`

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod error;
pub mod io;
pub mod blocks;
pub mod parser;
pub mod model;
pub mod cache;
pub mod channels_db;
pub mod data_index;
mod file;

// Re-export main types at crate root
pub use error::{Mf4Error, Result};
pub use file::{Mf4File, OpenOptions};
pub use model::{Channel, ChannelGroup, DataGroup, Signal, FileStatistics, RecordingTime};
pub use parser::Mf4Version;
pub use channels_db::{ChannelsDB, ChannelLocation, MastersDB};
pub use cache::{BlockCache, CacheStats};

/// Prelude module for convenient imports.
///
/// ```
/// use falcon_mdf::prelude::*;
/// ```
pub mod prelude {
    pub use crate::Mf4File;
    pub use crate::error::{Mf4Error, Result};
    pub use crate::model::{Channel, ChannelGroup, DataGroup, Signal};
    pub use crate::parser::Mf4Version;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prelude_imports() {
        // Just verify that prelude types are accessible
        fn _test_types() {
            let _: Option<Mf4Version> = None;
        }
    }
}
