//! Creates an MF4 file from scratch with [`Mf4Writer`]: two channels on one
//! time axis, one of them carrying an invalid range, written sorted to the
//! path given as the first argument (or `falcon_write_example.mf4`).
//!
//! Run: `cargo run --example write_mf4 -- /tmp/out.mf4`

use falcon_mdf::Mf4Writer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "falcon_write_example.mf4".to_string());

    let times: Vec<f64> = (0..100).map(|i| f64::from(i) * 0.1).collect();
    let speed: Vec<f64> = times
        .iter()
        .map(|t| 50.0 + (t * 0.5).sin() * 30.0)
        .collect();
    let boost: Vec<f64> = times.iter().map(|t| t * 0.2).collect();
    // Samples 40..60 of Boost are declared invalid: a gap, not a line.
    let boost_valid: Vec<bool> = (0..100).map(|i| !(40..60).contains(&i)).collect();

    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&times)?;
    group.add_channel("Speed", "km/h", &speed)?;
    group.add_channel_with_validity("Boost", "psi", &boost, Some(&boost_valid))?;
    writer.write_to_file(&out)?;

    println!("wrote {out}");
    Ok(())
}
