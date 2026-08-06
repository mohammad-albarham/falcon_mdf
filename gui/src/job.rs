//! One-shot background jobs whose completion message lands in a panel's
//! notice line. Same arrangement as `loader.rs` and `signal_loader.rs` —
//! plain OS thread plus mpsc plus `request_repaint` — factored out because
//! exports and attachment saves all need it.

use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// A job running on a worker thread. Polled each frame; once it finishes,
/// [`Job::poll`] yields the message the panel shows and the job is dropped.
pub struct Job {
    rx: Receiver<String>,
}

impl Job {
    /// Runs `work` on a new thread and wakes the UI when it finishes, so
    /// eframe repaints without waiting for user input.
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> String + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let message = work();
            let _ = tx.send(message);
            ctx.request_repaint();
        });
        Self { rx }
    }

    /// Returns the completion message once the worker finishes. `None` means
    /// still running. A worker that dies without sending is reported as such
    /// rather than swallowed — the panels' rule is that a silent failure is
    /// worse than an ugly line of text.
    pub fn poll(&self) -> Option<String> {
        match self.rx.try_recv() {
            Ok(message) => Some(message),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some("the worker thread ended without a result".to_string())
            }
        }
    }
}
