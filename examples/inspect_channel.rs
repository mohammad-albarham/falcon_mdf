use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: inspect_channel <path/to/file.MF4> [channel_name]");
        std::process::exit(2);
    }
    let path = std::path::Path::new(&args[1]);
    let channel_name = args.get(2).map(|s| s.as_str()).unwrap_or("CAN_DataFrame");

    let mf4 = Mf4File::open(path)?;
    let ch = mf4.find_channel(channel_name).expect("channel not found");
    let sig = mf4.signal(ch)?;
    println!("Channel: {}", channel_name);
    println!("Samples: {}", sig.len());
    println!("First 10 values:");
    for (i, v) in sig.iter().take(10).enumerate() {
        println!("[{}] {}", i, v?);
    }
    Ok(())
}
