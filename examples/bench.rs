//! Times open + full read of every channel, for comparison with asammdf.

use falcon_mdf::Mf4File;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: bench <file>");

    let t = Instant::now();
    let file = Mf4File::open(&path)?;
    let t_open = t.elapsed();

    let channels: Vec<_> = file.channels().cloned().collect();
    let t = Instant::now();
    let mut total = 0usize;
    for ch in &channels {
        if let Ok(sig) = file.signal(ch) {
            if let Ok(v) = sig.values_f64() {
                total += v.len();
            }
        }
    }
    let t_read = t.elapsed();

    println!(
        "{}: open={:.2}ms read_all={:.2}ms samples={}",
        path,
        t_open.as_secs_f64() * 1000.0,
        t_read.as_secs_f64() * 1000.0,
        total
    );
    Ok(())
}
