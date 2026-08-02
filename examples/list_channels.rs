//! Example: List all channels in an MF4 file.
//!
//! This example demonstrates how to open an MF4 file and enumerate
//! all data groups, channel groups, and channels.
//!
//! Usage: cargo run --example list_channels <file.mf4>

use std::env;
use std::process;

use falcon_mdf::Mf4File;

fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.mf4>", args[0]);
        eprintln!();
        eprintln!("Lists all channels in an MF4 measurement file.");
        process::exit(1);
    }

    let path = &args[1];

    // Open the MF4 file
    let file = match Mf4File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error opening file: {}", e);
            process::exit(1);
        }
    };

    // Print file information
    println!("═══════════════════════════════════════════════════════════════");
    println!("MF4 File: {}", path);
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Version info
    println!("Format Version: {}", file.version());
    println!(
        "File Size: {} bytes ({:.2} MB)",
        file.file_size(),
        file.file_size() as f64 / (1024.0 * 1024.0)
    );
    println!();

    // Recording time
    let start_time = file.start_time();
    println!("Recording Start: {}", start_time.to_iso8601());
    println!("UTC Offset: {} min", start_time.total_utc_offset_min());
    println!();

    // Comment
    if !file.comment().is_empty() {
        println!("Comment: {}", file.comment());
        println!();
    }

    // Statistics
    let stats = file.statistics();
    println!("Statistics:");
    println!("  Data Groups:    {}", stats.data_group_count);
    println!("  Channel Groups: {}", stats.channel_group_count);
    println!("  Channels:       {}", stats.channel_count);
    println!("  Total Samples:  {}", stats.total_sample_count);
    println!();

    // List all channels organized by data group and channel group
    println!("═══════════════════════════════════════════════════════════════");
    println!("Channel Structure");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        println!(
            "Data Group {} {}",
            dg_idx,
            if dg.comment.is_empty() {
                String::new()
            } else {
                format!("({})", dg.comment)
            }
        );

        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            let acq_name = if cg.acquisition_name.is_empty() {
                String::new()
            } else {
                format!(" \"{}\"", cg.acquisition_name)
            };

            println!("  └─ Channel Group {}{}", cg_idx, acq_name);
            println!("     Samples: {}", cg.sample_count);

            // Find master channel
            if let Some(master) = cg.master_channel() {
                println!("     Master: {} [{}]", master.name, master.unit);
            }

            println!("     Channels:");
            for channel in &cg.channels {
                let master_marker = if channel.is_master() { " (master)" } else { "" };
                let unit = if channel.unit.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", channel.unit)
                };

                println!("       • {}{}{}", channel.name, unit, master_marker);
            }
            println!();
        }
    }

    // List all unique channel names for quick reference
    println!("═══════════════════════════════════════════════════════════════");
    println!("All Channel Names (alphabetically sorted)");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    let mut channel_names: Vec<_> = file
        .channels()
        .map(|ch| {
            let unit = if ch.unit.is_empty() {
                String::new()
            } else {
                format!(" [{}]", ch.unit)
            };
            format!("{}{}", ch.name, unit)
        })
        .collect();
    channel_names.sort();
    channel_names.dedup();

    for (i, name) in channel_names.iter().enumerate() {
        println!("{:4}. {}", i + 1, name);
    }

    println!();
    println!("Total unique channels: {}", channel_names.len());
}
