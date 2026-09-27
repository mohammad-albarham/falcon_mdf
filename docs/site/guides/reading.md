# Read channels

## Find the right channel

`find_channel(name)` returns the first exact name match. Use `find_channels(name)` when the name may occur in more than one group, or `search_channels` for substring, wildcard, or regex queries. A channel's identity includes its location in the file; a repeated name alone may not identify the intended series.

Read several channels with `signals()` when you need them together. It assembles each channel group's records once, rather than repeating the work for each channel.

## Keep values and validity aligned

`signal.values()` returns the channel's own type, including integers, text, and bytes. `signal.values_f64()` is a convenient numeric projection, but can lose precision for wide integers and cannot represent byte or text channels.

A channel with an invalidation bit still returns all samples. Check validity alongside values so they stay aligned with the time master:

```rust
use falcon_mdf::Mf4File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("measurement.mf4")?;
    let channel = file.find_channel("VehicleSpeed").ok_or("channel not found")?;
    let signal = file.signal(channel)?;
    let values = signal.values_f64()?;

    match signal.validity() {
        Some(valid) => {
            for (value, ok) in values.iter().zip(&valid) {
                if *ok {
                    println!("{value}");
                }
            }
        }
        None => println!("{} valid samples", values.len()),
    }
    Ok(())
}
```

## Choose an I/O backend

| Backend | Use when |
| --- | --- |
| `Mf4File::open` | The file is complete and stays unchanged while open |
| `Mf4File::open_buffered` | Another process may update or replace it, or it is on a network share |
| `Mf4File::from_source` | Bytes come from an object store, an HTTP range source, or another custom source |

For large groups, the streamed `signals_chunks` API reads in bounded windows. The ordinary full-channel API assembles a data group's records before decoding; its peak memory therefore depends on the largest group. See the [performance notes](../reference/performance.md#memory-and-backends).

## Handle unsupported layouts

Opening a file does not guarantee every channel can be decoded. Handle `Mf4Error::Unsupported` when processing files you did not create:

```rust
use falcon_mdf::{Mf4Error, Mf4File};

fn main() -> Result<(), Mf4Error> {
    let file = Mf4File::open("measurement.mf4")?;
    for channel in file.channels() {
        match file.signal(channel).and_then(|signal| signal.values()) {
            Ok(values) => println!("{}: {} samples", channel.name, values.len()),
            Err(Mf4Error::Unsupported { feature, .. }) => {
                eprintln!("{}: unsupported: {feature}", channel.name);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
```

See [limits and errors](../reference/limits.md) for the known format boundaries.
