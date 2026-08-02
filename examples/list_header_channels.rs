use falcon_mdf::Mf4File;
use std::env;

// Prints all channel names from the MF4 header without reading sample data.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default to the provided sample file unless a path is passed as the first argument.
    let default_path =
        "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4";
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| default_path.to_string());

    println!("MF4 file: {}", path);
    let file = Mf4File::open(&path)?;

    // Basic header info
    println!("Format Version: {}", file.version());
    println!("Channels reported in header: {}", file.channel_count());

    // Collect and sort unique channel names from the header
    let mut names: Vec<&str> = file.channel_names().collect();
    names.sort();
    names.dedup();

    println!("Unique channel names ({}):", names.len());
    for (i, name) in names.iter().enumerate() {
        println!("{:4}. {}", i + 1, name);
    }

    Ok(())
}
