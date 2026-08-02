//! Dumps channel summaries for a single file given as argv[1].

use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: dump_one <file>");
    let file = Mf4File::open(&path)?;
    println!(
        "version={} data_groups={}",
        file.version(),
        file.data_group_count()
    );
    let channels: Vec<_> = file.channels().cloned().collect();
    for ch in &channels {
        match file.signal(ch) {
            Ok(sig) => {
                let v = sig.values_f64().unwrap_or_default();
                let (mn, mx) = v
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
                println!(
                    "  dg{} cg{} {:40} n={:<8} min={:<20} max={}",
                    ch.data_group_index,
                    ch.channel_group_index,
                    ch.name,
                    v.len(),
                    if v.is_empty() { 0.0 } else { mn },
                    if v.is_empty() { 0.0 } else { mx }
                );
            }
            Err(e) => println!("  {:40} ERROR {}", ch.name, e),
        }
    }
    Ok(())
}
