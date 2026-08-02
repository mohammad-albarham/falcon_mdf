//! Breaks a full read into its phases, to show where the time actually goes.

use falcon_mdf::{Mf4File, ValueKind};
use std::time::Instant;

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: profile <file>");
    let reps: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let mut open_total = 0.0;
    let mut first_total = 0.0;
    let mut rest_total = 0.0;
    let mut samples = 0usize;

    for _ in 0..reps {
        let t = Instant::now();
        let file = Mf4File::open(&path)?;
        open_total += ms(t.elapsed());

        let channels: Vec<_> = file.channels().cloned().collect();

        // First channel of each group pays for assembling that group's records
        // (read plus, where relevant, decompression). Later channels hit the
        // cache, so their cost is decode alone.
        let mut seen_groups = std::collections::HashSet::new();
        let mut n = 0usize;

        for ch in &channels {
            let group = (ch.data_group_index, ch.channel_group_index);
            let first = seen_groups.insert(group);

            let t = Instant::now();
            if let Ok(sig) = file.signal(ch) {
                if let Ok(v) = sig.values() {
                    n += v.len();
                }
            }
            let elapsed = ms(t.elapsed());

            if first {
                first_total += elapsed;
            } else {
                rest_total += elapsed;
            }
        }
        samples = n;
    }

    let r = reps as f64;
    println!("{}", path.rsplit('/').next().unwrap_or(&path));
    println!("  open                 {:7.3} ms", open_total / r);
    println!(
        "  assemble + decode    {:7.3} ms  (first channel per group)",
        first_total / r
    );
    println!(
        "  decode only          {:7.3} ms  (remaining channels)",
        rest_total / r
    );
    println!(
        "  total                {:7.3} ms",
        (open_total + first_total + rest_total) / r
    );
    println!("  samples              {samples}");

    // Per-kind decode cost, to show which channel types dominate.
    let file = Mf4File::open(&path)?;
    let channels: Vec<_> = file.channels().cloned().collect();
    let mut by_kind: std::collections::BTreeMap<&str, (f64, usize)> = Default::default();
    for _ in 0..reps {
        for ch in &channels {
            let Ok(sig) = file.signal(ch) else { continue };
            let kind = ch.value_kind();
            let t = Instant::now();
            let Ok(v) = sig.values() else { continue };
            let e = ms(t.elapsed());
            let entry = by_kind.entry(kind_name(kind)).or_insert((0.0, 0));
            entry.0 += e;
            entry.1 = v.len();
        }
    }
    println!("  decode by kind (per rep):");
    for (kind, (total, len)) in &by_kind {
        println!("    {kind:6} {:7.3} ms   last len={len}", total / r);
    }

    Ok(())
}

fn kind_name(k: ValueKind) -> &'static str {
    k.name()
}
