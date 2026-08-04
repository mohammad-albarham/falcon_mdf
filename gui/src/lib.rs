//! The one piece of `falcon`'s logic that needs to be reachable from a test
//! without an `egui::Context`: decimation. Everything else (app state,
//! panels, loaders) lives in the `falcon` binary itself, since it is UI code
//! through and through.

pub mod decimate;
