//! Several files, one operation, applied to all of them.
//!
//! The rest of the viewer is about one measurement at a time. This is the
//! other thing people do with a directory of recordings: cut every one of them
//! to the same window, keep the same handful of channels out of each, or turn
//! all of them into CSV. asammdf calls it the batch-processing tab.
//!
//! The work is split so that almost none of it needs a window. [`run_one`]
//! takes a path and an operation and returns what happened; [`run_all`] walks
//! a queue and collects one outcome per file; [`spawn`] runs `run_all` on a
//! worker thread and reports progress as it goes, the same plain
//! thread-plus-channel arrangement as [`crate::job`] and [`crate::loader`].
//! Only the panel needs a `Ui`, which is why the failure rule below can be
//! tested without one — see `gui/tests/batch_continues_past_failure.rs`.
//!
//! # The failure rule
//!
//! A batch is run over files someone did not hand-check first. One of them
//! will be truncated, or be a `.mf4` that is not one, or hold a channel type
//! the writer has no layout for. A batch that stops at the first of those has
//! done the user no favour: they wanted the other nine processed. So every
//! file is independent — a failure is recorded against that file **by name,
//! with its reason**, and the run continues.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use falcon_mdf::{Channel, Mf4File, Mf4Writer};

/// What to do to every file in the queue.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchOp {
    /// Keep only the samples inside a time range, writing a new MF4 file.
    Cut {
        /// First timestamp to keep.
        start: f64,
        /// Last timestamp to keep.
        end: f64,
    },
    /// Keep only the named channels, writing a new MF4 file. A name a file
    /// does not have is skipped; a file with none of them is a failure, since
    /// writing an empty measurement would look like success.
    Filter {
        /// Channel names to keep.
        names: Vec<String>,
    },
    /// Write every channel out as CSV.
    Export,
}

impl BatchOp {
    /// The label the tab shows, and what a saved queue stores.
    pub fn label(&self) -> &'static str {
        match self {
            BatchOp::Cut { .. } => "Cut to time range",
            BatchOp::Filter { .. } => "Keep channels",
            BatchOp::Export => "Export to CSV",
        }
    }

    /// The extension the output of this operation carries.
    fn extension(&self) -> &'static str {
        match self {
            BatchOp::Cut { .. } | BatchOp::Filter { .. } => "mf4",
            BatchOp::Export => "csv",
        }
    }

    /// The tag put into an output file's name, so a cut and a filter of the
    /// same input do not write to the same place.
    fn tag(&self) -> &'static str {
        match self {
            BatchOp::Cut { .. } => "cut",
            BatchOp::Filter { .. } => "filtered",
            BatchOp::Export => "export",
        }
    }

    /// Rejects an operation that cannot be run before any file is opened, so
    /// a bad time range is one message rather than one per file.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            BatchOp::Cut { start, end } => {
                if !start.is_finite() || !end.is_finite() {
                    Err("the time range needs two finite numbers".to_string())
                } else if end < start {
                    Err(format!("the time range ends at {end} before it starts at {start}"))
                } else {
                    Ok(())
                }
            }
            BatchOp::Filter { names } => {
                if names.iter().all(|n| n.trim().is_empty()) {
                    Err("name at least one channel to keep".to_string())
                } else {
                    Ok(())
                }
            }
            BatchOp::Export => Ok(()),
        }
    }
}

/// Where one file's output went, or why it did not.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The input file this is about.
    pub path: PathBuf,
    /// A line describing what was written, or the reason nothing was.
    pub result: Result<String, String>,
}

impl Outcome {
    /// Whether this file was processed.
    pub fn succeeded(&self) -> bool {
        self.result.is_ok()
    }

    /// The file's own name, which is what the outcome list shows: a batch is
    /// usually a directory of files whose paths differ only at the end.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Where a file's output is written: beside the input, or in a chosen folder.
///
/// Both spellings name the file after its input plus the operation, so a batch
/// never silently overwrites the measurement it read.
pub fn output_path(input: &Path, op: &BatchOp, out_dir: Option<&Path>) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());
    let name = format!("{stem}.{}.{}", op.tag(), op.extension());
    match out_dir {
        Some(dir) => dir.join(name),
        None => input.with_file_name(name),
    }
}

/// Applies `op` to one file.
///
/// Every failure is a message rather than a panic or an early exit: this is
/// the unit [`run_all`] keeps going past.
pub fn run_one(input: &Path, op: &BatchOp, out_dir: Option<&Path>) -> Result<String, String> {
    // Buffered rather than the mmap default, for the reason `loader.rs` gives:
    // a mapping over a file something else may truncate is unsound, and a
    // batch is precisely where unattended files get processed.
    let file = Mf4File::open_buffered(input).map_err(|e| e.to_string())?;
    let dst = output_path(input, op, out_dir);

    match op {
        BatchOp::Cut { start, end } => cut_to(&file, *start, *end, &dst),
        BatchOp::Filter { names } => filter_to(&file, names, &dst),
        BatchOp::Export => export_csv(&file, &dst),
    }
}

/// Writes the samples of `file` inside `[start, end]` to a new MF4 file.
fn cut_to(file: &Mf4File, start: f64, end: f64, dst: &Path) -> Result<String, String> {
    let mut writer = Mf4Writer::new();
    let mut written = 0usize;

    for (dgi, dg) in file.data_groups().iter().enumerate() {
        for (cgi, cg) in dg.channel_groups.iter().enumerate() {
            let data: Vec<&Channel> = cg.channels.iter().filter(|c| !c.is_master()).collect();
            if data.is_empty() {
                continue;
            }
            let series = file
                .cut(&data, start, end)
                .map_err(|e| format!("group {dgi}/{cgi}: {e}"))?;
            let Some(first) = series.first() else {
                continue;
            };
            // A group whose every sample fell outside the range is dropped
            // rather than written empty: an MF4 group with no records is a
            // shape readers disagree about, and it says nothing anyway.
            if first.is_empty() {
                continue;
            }
            let times = first.timestamps().to_vec();
            let group = writer
                .add_group(&times)
                .map_err(|e| format!("group {dgi}/{cgi}: {e}"))?;
            for s in &series {
                group
                    .add_channel_typed(s.name(), s.unit(), s.values().clone())
                    .map_err(|e| format!("channel '{}': {e}", s.name()))?;
                written += 1;
            }
        }
    }

    if written == 0 {
        return Err(format!(
            "no samples fall between {start} and {end}, so there was nothing to write"
        ));
    }

    writer
        .write_to_file(dst)
        .map_err(|e| format!("writing {}: {e}", dst.display()))?;
    Ok(format!(
        "cut {written} channels to {}",
        dst.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Writes only the named channels of `file` to a new MF4 file.
fn filter_to(file: &Mf4File, names: &[String], dst: &Path) -> Result<String, String> {
    let wanted: Vec<&str> = names
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .collect();

    let mut writer = Mf4Writer::new();
    let mut written = 0usize;
    let mut kept_names: Vec<String> = Vec::new();

    for (dgi, dg) in file.data_groups().iter().enumerate() {
        for (cgi, cg) in dg.channel_groups.iter().enumerate() {
            let data: Vec<&Channel> = cg
                .channels
                .iter()
                .filter(|c| !c.is_master() && wanted.iter().any(|w| *w == c.name))
                .collect();
            if data.is_empty() {
                continue;
            }
            // Cut over the full range is how a channel becomes a series
            // carrying its own timestamps; there is no separate "read whole
            // channel as a series" that also gives the group's time axis.
            let series = file
                .cut(&data, f64::NEG_INFINITY, f64::INFINITY)
                .map_err(|e| format!("group {dgi}/{cgi}: {e}"))?;
            let Some(first) = series.first() else {
                continue;
            };
            if first.is_empty() {
                continue;
            }
            let times = first.timestamps().to_vec();
            let group = writer
                .add_group(&times)
                .map_err(|e| format!("group {dgi}/{cgi}: {e}"))?;
            for s in &series {
                group
                    .add_channel_typed(s.name(), s.unit(), s.values().clone())
                    .map_err(|e| format!("channel '{}': {e}", s.name()))?;
                kept_names.push(s.name().to_string());
                written += 1;
            }
        }
    }

    if written == 0 {
        return Err(format!(
            "none of the {} named channels are in this file",
            wanted.len()
        ));
    }

    writer
        .write_to_file(dst)
        .map_err(|e| format!("writing {}: {e}", dst.display()))?;

    // Which names were missing is worth saying: a batch that silently kept
    // three of five channels looks exactly like one that kept all five.
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|w| !kept_names.iter().any(|k| k == *w))
        .copied()
        .collect();
    let mut line = format!("kept {written} channels");
    if !missing.is_empty() {
        line.push_str(&format!(" (not in this file: {})", missing.join(", ")));
    }
    Ok(line)
}

/// Writes every data channel of `file` out as CSV.
fn export_csv(file: &Mf4File, dst: &Path) -> Result<String, String> {
    let channels: Vec<&Channel> = file.channels().filter(|c| !c.is_master()).collect();
    if channels.is_empty() {
        return Err("this file has no data channels to export".to_string());
    }

    let out = std::fs::File::create(dst).map_err(|e| format!("creating {}: {e}", dst.display()))?;
    let mut out = std::io::BufWriter::new(out);
    falcon_mdf::write_csv(file, &channels, &mut out)
        .map_err(|e| format!("writing {}: {e}", dst.display()))?;
    use std::io::Write;
    out.flush().map_err(|e| e.to_string())?;

    Ok(format!(
        "exported {} channels to {}",
        channels.len(),
        dst.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Applies `op` to every path, in order, reporting each through `progress`.
///
/// A file that fails is recorded and the walk continues; the returned list has
/// exactly one entry per input path, in the order they were given, whatever
/// happened to any of them. `cancel` is checked between files, so a long batch
/// can be stopped without killing the thread mid-write.
pub fn run_all(
    paths: &[PathBuf],
    op: &BatchOp,
    out_dir: Option<&Path>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Vec<Outcome> {
    let mut outcomes = Vec::with_capacity(paths.len());

    for (index, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(Progress::Started {
            index,
            total: paths.len(),
            path: path.clone(),
        });

        let outcome = Outcome {
            path: path.clone(),
            result: run_one(path, op, out_dir),
        };
        progress(Progress::Finished {
            index,
            total: paths.len(),
            outcome: outcome.clone(),
        });
        outcomes.push(outcome);
    }

    outcomes
}

/// What a running batch reports as it goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// A file is about to be processed.
    Started {
        /// Its position in the queue.
        index: usize,
        /// How many files the batch holds.
        total: usize,
        /// The file itself.
        path: PathBuf,
    },
    /// A file is done, for better or worse.
    Finished {
        /// Its position in the queue.
        index: usize,
        /// How many files the batch holds.
        total: usize,
        /// What happened to it.
        outcome: Outcome,
    },
    /// Every file has been processed, or the run was cancelled.
    Done,
}

/// A batch running on a worker thread.
pub struct BatchRun {
    rx: Receiver<Progress>,
    cancel: Arc<AtomicBool>,
    /// The file being processed and how far through the queue it is.
    current: Option<(usize, usize, PathBuf)>,
    outcomes: Vec<Outcome>,
    finished: bool,
}

/// Starts `op` over `paths` on a worker thread and wakes the UI as each file
/// finishes, so a batch over large files never blocks a frame.
pub fn spawn(
    paths: Vec<PathBuf>,
    op: BatchOp,
    out_dir: Option<PathBuf>,
    ctx: egui::Context,
) -> BatchRun {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);

    std::thread::spawn(move || {
        run_all(
            &paths,
            &op,
            out_dir.as_deref(),
            &worker_cancel,
            |progress| {
                let _ = tx.send(progress);
                ctx.request_repaint();
            },
        );
        let _ = tx.send(Progress::Done);
        ctx.request_repaint();
    });

    BatchRun {
        rx,
        cancel,
        current: None,
        outcomes: Vec::new(),
        finished: false,
    }
}

impl BatchRun {
    /// Takes whatever the worker has reported since the last frame.
    pub fn poll(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(Progress::Started { index, total, path }) => {
                    self.current = Some((index, total, path));
                }
                Ok(Progress::Finished { outcome, .. }) => {
                    self.current = None;
                    self.outcomes.push(outcome);
                }
                Ok(Progress::Done) => {
                    self.current = None;
                    self.finished = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // A worker that died without saying so is reported rather
                    // than left spinning: the panels' rule is that a silent
                    // failure is worse than an ugly line of text.
                    self.current = None;
                    self.finished = true;
                    break;
                }
            }
        }
    }

    /// Whether the run has ended.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The file being processed, with its position in the queue.
    pub fn current(&self) -> Option<(usize, usize, &Path)> {
        self.current
            .as_ref()
            .map(|(i, total, path)| (*i, *total, path.as_path()))
    }

    /// Every file finished so far.
    pub fn outcomes(&self) -> &[Outcome] {
        &self.outcomes
    }

    /// Asks the worker to stop after the file it is on.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// How far through the queue the run is, as a fraction.
    pub fn fraction(&self, total: usize) -> f32 {
        if total == 0 {
            return 1.0;
        }
        self.outcomes.len() as f32 / total as f32
    }
}

/// A one-line summary of a finished run, which is what the panel shows once
/// the per-file list has scrolled.
pub fn summarise(outcomes: &[Outcome]) -> String {
    let failed = outcomes.iter().filter(|o| !o.succeeded()).count();
    let ok = outcomes.len() - failed;
    match (ok, failed) {
        (0, 0) => "nothing to do".to_string(),
        (ok, 0) => format!("{ok} of {ok} files processed"),
        (0, failed) => format!("all {failed} files failed"),
        (ok, failed) => format!("{ok} processed, {failed} failed", ok = ok, failed = failed),
    }
}
