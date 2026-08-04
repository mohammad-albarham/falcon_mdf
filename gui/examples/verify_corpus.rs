//! Opens every MF4 file in the test corpora through the same code path the
//! GUI uses (`Mf4File::open_buffered`, then a walk of channels/statistics)
//! and reports pass/fail per file. This is the acceptance test for G1: no
//! crash, no hang, and any failure must show its actual error text.
//!
//! Usage: cargo run -p falcon_mdf_gui --example verify_corpus [corpus_dir...]
//! Defaults to `test_data/reference` and `test_data/mf4-sample-data-v2.1`.

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

    for path in &files {
        match open_and_walk(path) {
            Ok(summary) => {
                ok += 1;
                println!("PASS  {}  {summary}", path.display());
            }
            Err(message) => {
                failed += 1;
                println!("FAIL  {}  {message}", path.display());
            }
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

/// Opens the file and touches the same accessors the GUI does on load
/// (channel names, data-group walk, statistics), so a failure here is a
/// failure the GUI would also hit.
fn open_and_walk(path: &Path) -> Result<String, String> {
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

    Ok(format!(
        "version={} data_groups={} channel_groups={} channels={} samples={} unreadable={}",
        file.version(),
        stats.data_group_count,
        stats.channel_group_count,
        stats.channel_count,
        stats.total_sample_count,
        unreadable,
    ))
}
