//! MF4 block type definitions and parsers.
//!
//! This module contains types and parsing logic for all MF4 block types.
//! Each block type is defined in its own submodule with parsing functions
//! that read from raw byte slices.

pub mod attachment;
pub mod channel;
pub mod channel_array;
pub mod channel_group;
pub mod common;
pub mod conversion;
pub mod data_block;
pub mod data_group;
pub mod event;
pub mod formula;
pub mod header;
pub mod hierarchy;
pub mod sample_reduction;
pub mod source;
pub mod text;

pub use attachment::*;
pub use channel::*;
pub use channel_array::*;
pub use channel_group::*;
pub use common::*;
pub use conversion::*;
pub use data_block::*;
pub use data_group::*;
pub use event::*;
pub use formula::*;
pub use header::*;
pub use hierarchy::*;
pub use sample_reduction::*;
pub use source::*;
pub use text::*;
