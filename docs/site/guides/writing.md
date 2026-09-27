# Write measurements

`Mf4Writer` creates MF4 files. Start with a time master, then add typed channels to its group:

```rust
use falcon_mdf::Mf4Writer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = Mf4Writer::new();
    let group = writer.add_group(&[0.0, 0.1, 0.2])?;
    group.add_channel("Speed", "km/h", &[0.0, 5.0, 10.0])?;
    group.add_channel_with_validity(
        "Boost",
        "psi",
        &[1.0, 2.0, 3.0],
        Some(&[true, false, true]),
    )?;
    writer.write_to_file("out.mf4")?;
    Ok(())
}
```

The writer supports integer and floating-point channels, fixed-length strings and bytes, variable-length strings and bytes, conversion rules, CA arrays, and per-sample validity. A group normally gets its own data group; `add_group_in` can place several channel groups in one data group.

## Compression

By default, compressed output uses deflate. The writer can also produce transposed deflate, zstd, and LZ4 variants through `set_codec`; zstd and LZ4 require their corresponding crate features. The output uses the MDF 4 `##HL`/`##DL` structure around compressed `##DZ` blocks.

## Edit an existing file

`Mf4Writer::from_file_with_report` loads supported channels into an editable writer and reports material it could not carry over. This is a conversion path, not a lossless rewrite of every MDF metadata block. Review the report before using the result as a replacement for the source file.

For the complete writer API, see [`Mf4Writer` on docs.rs](https://docs.rs/falcon_mdf/latest/falcon_mdf/write/struct.Mf4Writer.html) and the [repository example](https://github.com/mohammad-albarham/falcon_mdf/blob/main/examples/write_mf4.rs).
