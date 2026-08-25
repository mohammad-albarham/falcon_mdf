//! Example: Decode a bus-logged MF4 file against a CAN database.
//!
//! This is the whole path in one place: open a log, load a DBC, and print the
//! named physical signals the frames decode to.
//!
//! Usage: cargo run --features dbc --example decode_bus <file.mf4> <database.dbc> [--j1939]
//!
//! Pass `--j1939` for a heavy-duty database. A J1939 DBC keys its messages by
//! parameter group number while the identifier on the wire also carries the
//! sending ECU's source address, so without it a real J1939 log decodes to
//! nothing.

use std::env;
use std::process;

use falcon_mdf::{CanDatabase, IdMatching, Mf4File};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <file.mf4> <database.dbc> [--j1939]", args[0]);
        eprintln!();
        eprintln!("Decodes the CAN traffic in an MF4 bus log against a DBC.");
        process::exit(1);
    }

    let (path, dbc_path) = (&args[1], &args[2]);
    let matching = if args.iter().any(|arg| arg == "--j1939") {
        IdMatching::J1939Pgn
    } else {
        IdMatching::Exact
    };

    let file = match Mf4File::open(path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error opening {path}: {e}");
            process::exit(1);
        }
    };
    let database = match CanDatabase::from_dbc_path(dbc_path) {
        Ok(database) => database.with_matching(matching),
        Err(e) => {
            eprintln!("Error reading {dbc_path}: {e}");
            process::exit(1);
        }
    };

    println!("{path}");
    println!(
        "{} messages in {dbc_path}, matched {}",
        database.messages().len(),
        match matching {
            IdMatching::Exact => "by identifier",
            IdMatching::J1939Pgn => "by J1939 parameter group",
            IdMatching::J1939PgnAndSource => "by J1939 parameter group and source address",
        }
    );

    // Frames first, with no interpretation at all — this much needs no database.
    let frames: usize = file
        .can_frame_groups()
        .iter()
        .filter_map(|group| file.can_frames(group).ok())
        .map(|frames| frames.len())
        .sum();
    println!("{frames} CAN frames logged\n");

    // Then the same traffic as named signals over time.
    let signals = match file.decode_bus(&database) {
        Ok(signals) => signals,
        Err(e) => {
            eprintln!("Error decoding: {e}");
            process::exit(1);
        }
    };

    if signals.is_empty() {
        println!("No frame matched any message in the database.");
        if matching == IdMatching::Exact {
            println!("If this is a J1939 log, try again with --j1939.");
        }
        return;
    }

    println!("{:<34} {:>9}  {:>13}  RANGE", "SIGNAL", "READINGS", "FIRST");
    println!("{}", "─".repeat(78));

    let mut sorted: Vec<_> = signals.iter().collect();
    sorted.sort_by_key(|signal| (signal.bus_channel, signal.message, signal.name));

    for signal in sorted {
        let low = signal.values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = signal
            .values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);

        // A value table's label is more use than the number it stands for.
        let first = match signal.text_at(0) {
            Some(text) => text.to_string(),
            None => format!("{:.3}", signal.values[0]),
        };

        println!(
            "{:<34} {:>9}  {:>13}  {low:.3} .. {high:.3} {}",
            format!("{}.{}", signal.message, signal.name),
            signal.len(),
            first,
            signal.unit,
        );
    }
}
