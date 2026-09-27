# Architecture

The library separates byte access, MDF block parsing, domain types, and the public file API. The [interactive architecture map](https://mohammad-albarham.github.io/falcon_mdf/architecture.html) shows the full runtime flow and links its nodes to source files.

```text
Mf4File and operations         file.rs, stream.rs, write.rs, export/
          ↓
Channels and signals           model/, candb/, bus/
          ↓
Version-aware traversal        parser/, mdf3/
          ↓
MDF block decoding             blocks/
          ↓
Byte access                    io/ (mapped, buffered, custom source)
```

## Main modules

| Module | Responsibility |
| --- | --- |
| `file.rs` | Open MDF 4 files, find channels, decode signals, and expose high-level operations |
| `model/` | Channels, data groups, signals, and metadata |
| `parser/` and `blocks/` | Traverse links and decode MDF block structures |
| `io/` | Byte-source abstraction and built-in I/O backends |
| `mdf3/` | Optional MDF 2.x and 3.x reader |
| `stream.rs` | Bounded-window signal reads |
| `write.rs` and `export/` | MF4 writing and decoded-data export |
| `bus.rs`, `candb.rs`, `dbc.rs`, `arxml.rs`, `ldf.rs` | Frame extraction and database-driven signal decoding |

Consumers live in `python/`, `wasm/`, and `gui/`. Format conformance tests live in `tests/`; benchmark scripts and results live in `scripts/`, `benches/`, and `benchmarks/`.
