//! Times open + one batched read of every channel, for comparison with
//! asammdf's `mdf.select()`. `bench.rs` times the per-channel path instead.

use falcon_mdf::Mf4File;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: bench_batch <file>");

    let t = Instant::now();
    let file = Mf4File::open(&path)?;
    let t_open = t.elapsed();

    let channels: Vec<_> = file.channels().cloned().collect();

    // One call, each channel group's records assembled once: the same shape
    // of work as `select()` over every channel.
    let t = Instant::now();
    let mut total = 0usize;
    for sig in file.signals(&channels)? {
        if let Ok(v) = sig.values() {
            total += v.len();
        }
    }
    let t_batch = t.elapsed();

    println!(
        "{}: open={:.2}ms read_batch={:.2}ms channels={} samples={}",
        path,
        t_open.as_secs_f64() * 1000.0,
        t_batch.as_secs_f64() * 1000.0,
        channels.len(),
        total
    );
    Ok(())
}
