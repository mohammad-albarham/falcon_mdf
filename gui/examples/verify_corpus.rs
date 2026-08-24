//! Opens every MF4 file in the test corpora through the same code path the
//! GUI uses (`Mf4File::open_buffered`, then a walk of channels/statistics)
//! and reports pass/fail per file. This is the acceptance test for G1: no
//! crash, no hang, and any failure must show its actual error text.
//!
//! Every file that opens is also read as a file rather than as a
//! measurement: the block map walk (`Mf4File::block_map`) reports its block
//! count, the share of the file its blocks cover, and anything the walk
//! could not make sense of. A walk that warns is still a pass — warnings
//! are information, not failure.
//!
//! Usage: cargo run -p falcon_mdf_gui --example verify_corpus [corpus_dir...]
//! Defaults to `test_data/reference` and `test_data/mf4-sample-data-v2.1`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use falcon_mdf::Mf4File;

fn main() {
    let roots: Vec<PathBuf> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            vec![
                PathBuf::from("test_data/reference"),
                PathBuf::from("test_data/mf4-sample-data-v2.1"),
            ]
        } else {
            args.into_iter().map(PathBuf::from).collect()
        }
    };

    let mut files = Vec::new();
    for root in &roots {
        collect_files(root, &mut files);
    }
    files.sort();

    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut total_blocks = 0usize;
    let mut files_with_warnings = 0usize;
    // Distinct warning texts mapped to the files they came from; both sides
    // ordered, so the summary reads the same on every run.
    let mut warnings_by_text: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();

    for path in &files {
        match open_and_walk(path) {
            Ok(report) => {
                ok += 1;
                total_blocks += report.blocks;
                if !report.warnings.is_empty() {
                    files_with_warnings += 1;
                }
                for warning in &report.warnings {
                    warnings_by_text
                        .entry(warning.clone())
                        .or_default()
                        .insert(path.clone());
                }
                println!(
                    "PASS  {}  {}  blocks={} coverage={:.1}% warnings={}",
                    path.display(),
                    report.summary,
                    report.blocks,
                    report.coverage,
                    report.warnings.len()
                );
            }
            Err(message) => {
                failed += 1;
                println!("FAIL  {}  {message}", path.display());
            }
        }
    }

    println!();
    println!("--- block map summary ---");
    println!("files: {}", files.len());
    println!("blocks: {total_blocks}");
    println!("files with warnings: {files_with_warnings}");
    for (warning, sources) in &warnings_by_text {
        println!("warning: {warning}");
        for source in sources {
            println!("  in {}", source.display());
        }
    }

    println!();
    println!("{ok} passed, {failed} failed, {} total", ok + failed);

    if failed > 0 {
        std::process::exit(1);
    }
}

/// Recursively collects files whose extension is `mf4`/`MF4`/`dat`/`DAT`,
/// which is what the sample-data corpus uses instead of `.mf4`.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mf4") || ext.eq_ignore_ascii_case("dat"))
        {
            out.push(path);
        }
    }
}

/// What one file's verification produced.
struct FileReport {
    /// The per-file fields: version, group, channel and sample counts.
    summary: String,
    /// How many blocks the block map walk found.
    blocks: usize,
    /// Percentage of the file's bytes that its blocks cover.
    coverage: f64,
    /// The block map walk's warnings. Information, not failure.
    warnings: Vec<String>,
}

/// Opens the file and touches the same accessors the GUI does on load
/// (channel names, data-group walk, statistics), so a failure here is a
/// failure the GUI would also hit. Then walks the file's block map, which
/// reads the file as it sits on disk rather than as a measurement.
fn open_and_walk(path: &Path) -> Result<FileReport, String> {
    let file = Mf4File::open_buffered(path).map_err(|e| e.to_string())?;

    let stats = file.statistics();
    let _ = file.channel_names();
    let mut unreadable = 0usize;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            for ch in &cg.channels {
                let _ = &ch.name;
                let _ = &ch.unit;
                // The channel list asks every channel why it cannot be read
                // (G3), so the walk must touch that path too.
                if ch.unreadable().is_some() {
                    unreadable += 1;
                }
            }
        }
    }

    let map = file.block_map();
    let coverage = if map.file_size == 0 {
        0.0
    } else {
        100.0 * map.covered_bytes as f64 / map.file_size as f64
    };

    Ok(FileReport {
        summary: format!(
            "version={} data_groups={} channel_groups={} channels={} samples={} unreadable={}",
            file.version(),
            stats.data_group_count,
            stats.channel_group_count,
            stats.channel_count,
            stats.total_sample_count,
            unreadable,
        ),
        blocks: map.blocks.len(),
        coverage,
        warnings: map.warnings,
    })
}
