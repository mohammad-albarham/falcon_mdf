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
//! - **Bus Layer** ([`bus`]): Frames out of bus-logged groups, uninterpreted
//! - **Streaming** ([`stream`]): Bounded windows of a channel, for large groups
//! - **CAN databases** ([`candb`]): Payloads decoded to named physical signals
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
//! ## Design
//!
//! Repeated lookups are kept off the hot path by three index structures:
//!
//! - Block caching via [`cache::BlockCache`] for CC/TX/SI blocks
//! - Channel indexing via [`channels_db::ChannelsDB`] for name lookups
//! - Lazy data loading via [`data_index::DataBlockIndex`]
//!
//! The implementation is idiomatic Rust throughout, leveraging:
//! - `Arc<T>` for shared block ownership
//! - Zero-copy memory mapping via `memmap2`
//! - Type-safe enums instead of magic numbers
//! - Parallel parsing with `rayon`

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]
// The crate contains exactly one `unsafe` block, in `io::mmap`, where it is
// unavoidable: creating a memory map is inherently unsound if the file changes
// underneath it. Denying by default means any new `unsafe` has to be opted into
// deliberately and justified at the site.
#![deny(unsafe_code)]

#[cfg(feature = "arxml")]
pub mod arxml;
pub mod blocks;
pub mod bus;
pub mod cache;
pub mod candb;
pub mod channels_db;
pub mod data_index;
#[cfg(feature = "dbc")]
pub mod dbc;
pub mod error;
pub mod export;
mod file;
pub mod inspect;
pub mod io;
pub mod lin;
#[cfg(feature = "mdf3")]
pub mod mdf3;
pub mod model;
pub mod parser;
pub mod stream;
pub mod write;

// Re-export main types at crate root
pub use blocks::UnfinalizedFlags;
pub use bus::{BusSignal, BusSignals, CanFrame, CanFrames};
pub use cache::{BlockCache, CacheStats};
pub use candb::{CanDatabase, DecodedSignal, IdMatching, MessageDef, Multiplexing, SignalDef};
pub use channels_db::{ChannelLocation, ChannelsDB, MastersDB};
pub use error::{Mf4Error, Result};
pub use export::write_csv;
pub use file::{Mf4File, OpenOptions};
pub use inspect::{BlockInfo, BlockMap, Gap};
pub use lin::{LinFrame, LinFrames};
pub use model::{
    Attachment, CanopenDate, CanopenTime, Channel, ChannelGroup, ChannelHierarchyNode, DataGroup,
    Event, FileStatistics, Metadata, RecordingTime, ReductionKind, SampleReduction, Signal,
    SignalValues, UnreadableReason, ValueKind,
};
pub use parser::Mf4Version;
pub use stream::SignalChunks;
pub use write::{Mf4Writer, WriteGroup};

/// Prelude module for convenient imports.
///
/// ```
/// use falcon_mdf::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{Mf4Error, Result};
    pub use crate::model::{
        Attachment, CanopenDate, CanopenTime, Channel, ChannelGroup, ChannelHierarchyNode,
        DataGroup, Event, Metadata, Signal, SignalValues, UnreadableReason, ValueKind,
    };
    pub use crate::parser::Mf4Version;
    pub use crate::Mf4File;
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
