//! Times open + full read of every channel, for comparison with asammdf.

use falcon_mdf::Mf4File;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: bench <file>");

    let t = Instant::now();
    let file = Mf4File::open(&path)?;
    let t_open = t.elapsed();

    let channels: Vec<_> = file.channels().cloned().collect();

    // Native types — the equivalent of reading `.samples` from the reference,
    // which also returns each channel in its own dtype.
    let t = Instant::now();
    let mut total = 0usize;
    for ch in &channels {
        if let Ok(sig) = file.signal(ch) {
            if let Ok(v) = sig.values() {
                total += v.len();
            }
        }
    }
    let t_native = t.elapsed();

    // Everything coerced to f64, which costs an extra pass and allocation for
    // any channel that is not already f64.
    let t = Instant::now();
    let mut total_f64 = 0usize;
    for ch in &channels {
        if let Ok(sig) = file.signal(ch) {
            if let Ok(v) = sig.values_f64() {
                total_f64 += v.len();
            }
        }
    }
    let t_f64 = t.elapsed();

    println!(
        "{}: open={:.2}ms read_native={:.2}ms read_f64={:.2}ms samples={}",
        path,
        t_open.as_secs_f64() * 1000.0,
        t_native.as_secs_f64() * 1000.0,
        t_f64.as_secs_f64() * 1000.0,
        total.max(total_f64)
    );
    Ok(())
}
