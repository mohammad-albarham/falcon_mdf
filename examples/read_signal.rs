use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: read_signal <path/to/file.MF4> <channel_name>");
        std::process::exit(2);
    }
    let path = std::path::Path::new(&args[1]);
    let channel_name = &args[2];

    let mf4 = Mf4File::open(path)?;
    let ch = mf4.find_channel(channel_name).expect("channel not found");
    let sig = mf4.signal(ch)?;
    println!("Channel: {}", channel_name);
    println!("Samples: {}", sig.len());
    println!("First 10000 values:");
    for (i, v) in sig.iter().take(100).enumerate() {
        println!("[{}] {}", i, v?);
    }
    Ok(())
}
