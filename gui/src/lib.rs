//! `falcon`, a desktop viewer for MF4 measurement files, as a library.
//!
//! Everything except the launcher lives here rather than in the binary, so
//! that the parts with logic in them — how a search query matches a channel,
//! how a sample becomes the text in a cell, what a block list row says — can
//! be exercised from `gui/tests/` without an `egui::Context` and without a
//! window. The panels themselves still need a `Ui` and are still only
//! testable through their pure helpers, which is why those helpers are
//! written as free functions rather than methods wherever there was a choice.

pub mod app;
pub mod computed;
pub mod decimate;
pub mod format;
pub mod job;
pub mod loader;
pub mod model;
pub mod panels;
pub mod percentile;
pub mod recent;
pub mod search;
pub mod session;
pub mod signal_loader;
pub mod xy;
