//! Detailed MF4 file analysis example.
//!
//! This example reads an MF4 file and displays detailed information
//! for comparison with asammdf.

use falcon_mdf::Mf4File;
use std::env;
use std::time::Instant;

fn main() -> falcon_mdf::error::Result<()> {
    let args: Vec<String> = env::args().collect();
    
    let path = if args.len() > 1 {
        &args[1]
    } else {
        "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
    };

    println!("{}", "=".repeat(65));
    println!("MF4 File Analysis: {}", path);
    println!("{}", "=".repeat(65));
    println!();

    // Measure file open time
    let start = Instant::now();
    let file = Mf4File::open_mmap(path)?;
    let open_time = start.elapsed();

    // Basic info
    println!("Format Version: {}", file.version());
    println!("File Size: {} bytes ({:.2} MB)", 
        file.file_size(),
        file.file_size() as f64 / 1024.0 / 1024.0);
    println!();

    // Header info
    println!("Recording Start: {:?}", file.start_time());
    println!();

    // Comment
    let comment = file.comment();
    if !comment.is_empty() {
        let truncated = if comment.len() > 500 { 
            format!("{}...", &comment[..500]) 
        } else { 
            comment.to_string() 
        };
        println!("Comment: {}", truncated);
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

    // ChannelsDB stats
    let (unique_names, total_indexed) = file.channels_db_stats();
    println!("ChannelsDB Statistics:");
    println!("  Unique names:   {}", unique_names);
    println!("  Total indexed:  {}", total_indexed);
    println!();

    // Channel structure with detail
    println!("{}", "=".repeat(65));
    println!("Channel Structure (Detailed)");
    println!("{}", "=".repeat(65));
    println!();

    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        println!("Data Group {}", dg_idx);
        
        for cg in dg.channel_groups.iter() {
            let acq_name = if cg.acquisition_name.is_empty() {
                String::new()
            } else {
                format!(" \"{}\"", cg.acquisition_name)
            };
            
            println!("  └─ Channel Group {}{}", cg.index, acq_name);
            println!("     Sample Count: {}", cg.sample_count);
            println!("     Channels: {}", cg.channels.len());
            
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                let master_str = if ch.is_master() { " (master)" } else { "" };
                println!("       [{:2}] {} [{}]{}", 
                    ch_idx,
                    ch.name, 
                    ch.unit,
                    master_str);
                println!("           Type: {:?}, DataType: {:?}", 
                    ch.channel_type, ch.data_type);
                println!("           BitOffset: {}, BitCount: {}, ByteOffset: {}", 
                    ch.bit_offset, ch.bit_count, ch.byte_offset);
                
                // Show conversion info
                println!("           Conversion: {:?}", ch.conversion);
            }
            println!();
        }
    }

    // All channels from ChannelsDB
    println!("{}", "=".repeat(65));
    println!("All Channel Names (from ChannelsDB)");
    println!("{}", "=".repeat(65));
    println!();

    let mut names: Vec<&str> = file.channel_names().collect();
    names.sort();
    
    for (i, name) in names.iter().enumerate() {
        let channels = file.find_channels(name);
        println!("   {:2}. {} (found in {} location(s))", 
            i + 1, name, channels.len());
    }
    
    println!("\nTotal unique channels: {}", names.len());
    println!();

    // Performance
    println!("{}", "=".repeat(65));
    println!("Performance");
    println!("{}", "=".repeat(65));
    println!("File open time: {:.2} ms", open_time.as_secs_f64() * 1000.0);

    // Try to read signal data
    println!();
    println!("{}", "=".repeat(65));
    println!("Signal Data Sample");
    println!("{}", "=".repeat(65));

    // Find a channel with data
    for name in names.iter().take(5) {
        if let Some(channel) = file.find_channel(name) {
            let start = Instant::now();
            match file.signal(channel) {
                Ok(signal) => {
                    let read_time = start.elapsed();
                    println!("\nChannel: {}", name);
                    println!("  Samples: {}", signal.len());
                    println!("  Unit: {}", signal.unit());
                    println!("  Read time: {:.2} ms", read_time.as_secs_f64() * 1000.0);
                    
                    if !signal.is_empty() {
                        // Get first few values
                        let values: Result<Vec<f64>, _> = signal.iter().take(5).collect();
                        match values {
                            Ok(vals) => {
                                println!("  First 5 values: {:?}", vals);
                                
                                // Get min/max
                                match signal.min_max() {
                                    Ok((min, max)) => {
                                        println!("  Min: {:.6}, Max: {:.6}", min, max);
                                    }
                                    Err(e) => println!("  Min/Max error: {}", e),
                                }
                            }
                            Err(e) => println!("  Value read error: {}", e),
                        }
                    }
                }
                Err(e) => {
                    println!("\nChannel: {}", name);
                    println!("  Error: {}", e);
                }
            }
        }
    }

    println!();
    Ok(())
}
