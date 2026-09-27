# Rust quickstart

## Open a measurement

```rust
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    println!("MDF version: {}", file.version());
    println!("Channels: {}", file.channel_count());
    Ok(())
}
```

`Mf4File::open` uses memory mapping by default. Use `Mf4File::open_buffered` when another process may change or replace the file while you read it.

## Read a channel

Channel values retain their recorded type. Inspect the returned variant before processing it:

```rust
use falcon_mdf::{Mf4File, SignalValues};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    if let Some(channel) = file.find_channel("VehicleSpeed") {
        let signal = file.signal(channel)?;
        match signal.values()? {
            SignalValues::F64(values) => println!("first: {:?} {}", values.first(), signal.unit()),
            other => println!("{} samples of {}", other.len(), other.kind().name()),
        }
    }
    Ok(())
}
```

A channel can have zero samples. Use `first()` rather than indexing at zero. For a uniform numeric view, `signal.values_f64()` converts numeric samples to `f64`; wide integers may lose precision.

## Create an MF4 file

```rust
use falcon_mdf::Mf4Writer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 0.1, 0.2])?;
    group.add_channel("Speed", "km/h", &[0.0, 5.0, 10.0])?;
    writer.write_to_file("out.mf4")?;
    Ok(())
}
```

The timestamps passed to `add_group` become the group's time master. For validity bits, conversions, arrays, and compression, continue to [write measurements](../guides/writing.md).

## Try the repository examples

From a checkout of the project:

```sh
cargo run --example list_channels -- measurement.mf4
cargo run --example export_to_csv -- measurement.mf4 VehicleSpeed output.csv
cargo run --example write_mf4 -- out.mf4
```

For typed values, validity, channel selection, and large files, see [read channels](../guides/reading.md).
