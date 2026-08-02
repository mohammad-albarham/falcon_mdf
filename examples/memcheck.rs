//! Reports peak resident memory while reading a file, to show how the reader
//! scales with file size.
use falcon_mdf::Mf4File;

fn rss_mb() -> f64 {
    // macOS and Linux both expose this via ps.
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: memcheck <file>");
    let size = std::fs::metadata(&path)?.len() as f64 / 1e6;
    println!("file            {size:8.1} MB");
    println!("rss at start    {:8.1} MB", rss_mb());

    // The memory-mapped backend counts the file's pages in RSS as they are
    // touched, which is reclaimable page cache rather than heap. Reading the
    // same file buffered separates the two.
    let buffered = std::env::args().nth(2).as_deref() == Some("buffered");
    let file = if buffered {
        println!("backend         buffered");
        Mf4File::open_buffered(&path)?
    } else {
        println!("backend         mmap");
        Mf4File::open(&path)?
    };
    println!("rss after open  {:8.1} MB", rss_mb());

    let channels: Vec<_> = file.channels().cloned().collect();
    let mut peak: f64 = 0.0;
    let mut total = 0usize;
    for ch in &channels {
        if let Ok(sig) = file.signal(ch) {
            if let Ok(v) = sig.values() {
                total += v.len();
            }
        }
        peak = peak.max(rss_mb());
    }
    println!("rss peak read   {peak:8.1} MB");
    println!("channels {} samples {}", channels.len(), total);
    Ok(())
}
