//! Prints every block in an MF4 file, in the order they sit on disk.
//!
//! This is the command-line form of what the viewer's block explorer shows:
//! address, type, size, links and a line describing each block's fields,
//! followed by the regions no block covers and anything the walk could not
//! make sense of.
//!
//! ```text
//! cargo run --example block_map -- measurement.mf4
//! cargo run --example block_map -- measurement.mf4 --summary
//! ```

use std::process::ExitCode;

use falcon_mdf::Mf4File;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: block_map <file.mf4> [--summary]");
        return ExitCode::FAILURE;
    };
    let summary_only = args.any(|a| a == "--summary");

    let file = match Mf4File::open(&path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let map = file.block_map();

    if !summary_only {
        println!(
            "{:>12}  {:<6} {:>12} {:>6}  SUMMARY",
            "ADDRESS", "TYPE", "LENGTH", "LINKS"
        );
        for block in &map.blocks {
            println!(
                "{:>12}  {:<6} {:>12} {:>6}  {}",
                format!("{:#x}", block.address),
                block.block_type,
                block.length,
                block.link_count,
                block.summary
            );
        }
        println!();
    }

    println!("{} blocks", map.blocks.len());
    for (block_type, count) in map.type_counts() {
        println!("  {count:>7}  {block_type}");
    }
    println!(
        "{} of {} bytes covered ({:.1}%)",
        map.covered_bytes,
        map.file_size,
        map.covered_bytes as f64 / map.file_size.max(1) as f64 * 100.0
    );
    for gap in &map.gaps {
        println!("  gap: {} bytes at {:#x}", gap.length, gap.address);
    }
    for warning in &map.warnings {
        println!("  warning: {warning}");
    }

    ExitCode::SUCCESS
}
