//! Example: Export a channel's data to CSV.
//!
//! This example demonstrates how to read signal data from a channel
//! and export it to a CSV file with timestamps.
//!
//! Usage: cargo run --example export_to_csv <file.mf4> <channel_name> [output.csv]

use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::process;

use falcon_mdf::Mf4File;

fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <file.mf4> <channel_name> [output.csv]", args[0]);
        eprintln!();
        eprintln!("Exports a channel's data to CSV format.");
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  file.mf4      Input MF4 measurement file");
        eprintln!("  channel_name  Name of the channel to export");
        eprintln!("  output.csv    Output CSV file (default: <channel_name>.csv)");
        process::exit(1);
    }

    let mf4_path = &args[1];
    let channel_name = &args[2];
    let csv_path = if args.len() > 3 {
        args[3].clone()
    } else {
        format!(
            "{}.csv",
            channel_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
        )
    };

    // Open the MF4 file
    println!("Opening {}...", mf4_path);
    let file = match Mf4File::open(mf4_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error opening MF4 file: {}", e);
            process::exit(1);
        }
    };

    println!("MF4 Version: {}", file.version());
    println!("Total channels: {}", file.channel_count());
    println!();

    // Find the channel
    let channel = match file.find_channel(channel_name) {
        Some(ch) => ch,
        None => {
            eprintln!("Channel '{}' not found.", channel_name);
            eprintln!();
            eprintln!("Available channels:");
            for ch in file.channels().take(20) {
                eprintln!("  - {}", ch.name);
            }
            if file.channel_count() > 20 {
                eprintln!("  ... and {} more", file.channel_count() - 20);
            }
            process::exit(1);
        }
    };

    println!("Found channel: {}", channel.name);
    println!(
        "  Unit: {}",
        if channel.unit.is_empty() {
            "(none)"
        } else {
            &channel.unit
        }
    );
    println!("  Data type: {:?}", channel.data_type);
    println!("  Bits: {}", channel.bit_count);
    println!();

    // Find the channel group to get the master channel
    let dg = &file.data_groups()[channel.data_group_index];
    let cg = &dg.channel_groups[channel.channel_group_index];

    println!("Channel group: {} samples", cg.sample_count);

    // Read the signal data
    println!("Reading signal data...");
    let signal = match file.signal(channel) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading signal: {}", e);
            process::exit(1);
        }
    };

    // Try to find and read the master (time) channel
    let time_signal = cg
        .master_channel()
        .and_then(|master| file.signal(master).ok());

    // Get the values
    let values = match signal.values_f64() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error decoding values: {}", e);
            process::exit(1);
        }
    };

    let timestamps: Option<Vec<f64>> = time_signal.as_ref().and_then(|ts| ts.values_f64().ok());

    println!("Decoded {} samples", values.len());
    println!();

    // Create CSV file
    println!("Writing to {}...", csv_path);
    let csv_file = match File::create(&csv_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error creating CSV file: {}", e);
            process::exit(1);
        }
    };

    let mut writer = BufWriter::new(csv_file);

    // Write header
    let time_unit = time_signal.as_ref().map(|s| s.unit()).unwrap_or("index");
    let time_header = if timestamps.is_some() {
        format!("Time [{}]", time_unit)
    } else {
        "Index".to_string()
    };

    let value_header = if channel.unit.is_empty() {
        channel.name.clone()
    } else {
        format!("{} [{}]", channel.name, channel.unit)
    };

    if let Err(e) = writeln!(writer, "{},{}", time_header, value_header) {
        eprintln!("Error writing header: {}", e);
        process::exit(1);
    }

    // Write data rows
    for (i, &value) in values.iter().enumerate() {
        let time_str = if let Some(ref ts) = timestamps {
            format!("{:.9}", ts[i])
        } else {
            format!("{}", i)
        };

        if let Err(e) = writeln!(writer, "{},{:.9}", time_str, value) {
            eprintln!("Error writing row {}: {}", i, e);
            process::exit(1);
        }
    }

    if let Err(e) = writer.flush() {
        eprintln!("Error flushing file: {}", e);
        process::exit(1);
    }

    println!(
        "Successfully exported {} samples to {}",
        values.len(),
        csv_path
    );

    // Print some statistics
    if !values.is_empty() {
        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = values.iter().sum();
        let mean = sum / values.len() as f64;

        println!();
        println!("Statistics:");
        println!("  Min:  {:.6} {}", min, channel.unit);
        println!("  Max:  {:.6} {}", max, channel.unit);
        println!("  Mean: {:.6} {}", mean, channel.unit);

        if let Some(ref ts) = timestamps {
            if ts.len() >= 2 {
                let duration = ts.last().unwrap() - ts.first().unwrap();
                println!("  Duration: {:.3} {}", duration, time_unit);
            }
        }
    }
}
