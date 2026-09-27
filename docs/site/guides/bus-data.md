# Decode bus data

Bus-logged groups contain frames. `can_frame_groups` and `can_frames` expose CAN timestamps, identifiers, bus channels, and payload bytes without a database. To obtain named physical signals, provide a database that describes the payload.

## CAN with a DBC

Enable the `dbc` feature in your dependency, then decode the file:

```rust
use falcon_mdf::{CanDatabase, IdMatching, Mf4File};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = Mf4File::open("truck.mf4")?;
    let database = CanDatabase::from_dbc_path("j1939.dbc")?
        .with_matching(IdMatching::J1939Pgn);

    for signal in file.decode_bus(&database)?.iter() {
        println!("{}.{}: {} readings", signal.message, signal.name, signal.len());
    }
    Ok(())
}
```

J1939 PGN matching ignores the source address when looking up the database message. Use `IdMatching::J1939PgnAndSource` if the database has source-specific definitions.

## ARXML and LIN

The `arxml` feature enables `CanDatabase::from_arxml_path` for AUTOSAR ECU extracts. LIN Description File parsing is available without an extra feature; use `CanDatabase::from_ldf_path` with `file.decode_lin(&database)` for LIN signals.

Decoded signals are derived from the supplied database. They are separate from the channels recorded in the MDF file and do not appear in `file.channels()`. Identify a decoded series by bus, message, and signal name together.

For a runnable CLI example, see [`examples/decode_bus.rs`](https://github.com/mohammad-albarham/falcon_mdf/blob/main/examples/decode_bus.rs).
