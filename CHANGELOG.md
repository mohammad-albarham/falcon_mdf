# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the public API is not frozen: minor versions may contain breaking
changes, and they are listed under **Changed** with the reason.

## [Unreleased]

### Planned before 1.0

- `#[non_exhaustive]` on the public enums, so later format support does not
  force a major version
- Big-endian channel tests — that path is currently unexercised
- Block-by-block decoding, to stop memory scaling with the largest data group
- API review and freeze

## [0.2.0] — unreleased

The first release that reads real measurement files correctly. Every headline
number below was measured against an independent reference implementation over
an eight-file corpus of CAN, LIN and GPS/IMU logs.

### Fixed

Eighteen defects, of which the first is the reason this release exists.

- **Unsorted data groups produced garbage on every real logger file.** Records
  from several channel groups are interleaved in one stream, and the reader
  strided it with a single group's record size, reading across record
  boundaries. `Timestamp` came back as `-1.58e300`. Records are now indexed when
  the file opens and gathered per group. Corpus mismatches against the reference:
  1,780 → 0.
- **Sample counts were fabricated** from data size when `cycle_count` was zero,
  inventing 40,523 samples for groups that were empty. The stream is now the
  authority over a declared count.
- **Byte-array channels were forced through `f64`** — an eight-byte CAN payload
  arrived as `1.8e19`.
- **Variable-length channels returned zeros.** Their records hold an offset into
  a separate payload stream; the decoder read the offset as if it were the data.
- **Payload offsets were read with the channel's byte order.** A channel's data
  type describes its payload, not the byte order of the offset pointing at it,
  so most VLSD channels resolved a byte-reversed offset and returned nothing.
- **The rational conversion used reversed coefficients**, evaluating
  `(p0 + p1x + p2x²)/(p3 + p4x + p5x²)` instead of the specified
  `(P1x² + P2x + P3)/(P4x² + P5x + P6)`.
- **Conversion types 3 and 6–11 silently returned raw values** through a
  `_ => raw` fallthrough, so a text-table channel yielded meaningless numbers
  that looked like measurements.
- **Invalidation bits were parsed but never applied**, so samples the file marks
  invalid were returned as if they were data.
- **Composition channels were double-prefixed** — `CAN_DataFrame.CAN_DataFrame.ID`.
- **Payload positions were held in `u32`**, so a file with more than 4 GB of
  variable-length data silently addressed the wrong bytes.
- **`channel_count`, `find_channel`, `has_channel` and `channel_names` read from
  the name index rather than the data**, so opening with `build_channels_db:
  false` reported zero channels and found none.
- **`comment()` returned the raw XML** of a metadata block rather than the
  comment inside it.
- **Array channels were left readable while their CA block was skipped**, so
  reading one returned the first element while presenting as the whole channel.

Robustness, all found by mutating real files and fuzzing:

- **Panics on malformed input** — six crashes per four hundred structural
  mutations, from unchecked slicing in the block parsers.
- **Process aborts from unbounded allocation**, which a caller cannot catch.
  Three separate sources: an unvalidated link count, a block length exceeding
  the file, and a cycle count larger than the data could hold.
- **Infinite loops on self-referential links**, with unbounded memory growth.
- **A shift-overflow panic** reading a 64-bit field at a non-zero bit offset.

### Added

- `SignalValues` and `ValueKind` — samples in the channel's own type. A 29-bit
  CAN identifier decodes to `u32`, a two-bit bus number to `u8`, a frame payload
  to bytes. `values_f64()` remains as a documented-lossy convenience.
- `Signal::validity`, `is_valid`, `valid_count` — which samples the file marks
  valid. `values()` does not filter, so counts stay aligned with the master
  channel.
- Variable-length signal data, in both storage forms: payloads in a signal-data
  block, and payloads as records of a dedicated channel group.
- Conversion types 3 (algebraic, with a formula parser), 6, 7 and 8. Types 9–11
  report `Mf4Error::Unsupported` rather than guessing.
- `Metadata` and `Mf4File::metadata()` — a metadata block's comment and its
  named properties, with nested trees flattened to paths such as
  `"Device Information/serial number"`.
- `Mf4Error::Unsupported` — a channel this build cannot decode fails loudly
  instead of returning a plausible-looking wrong answer.
- `LICENSE-MIT` and `LICENSE-APACHE`, which the crate claimed but did not ship.
- CI across Linux, macOS and Windows: tests, clippy, rustfmt, rustdoc, MSRV and
  a packaging check, plus a `cargo-fuzz` target over the whole read path.

### Changed

Breaking, and listed with the reason:

- Composition channel names lose their duplicated prefix:
  `CAN_DataFrame.CAN_DataFrame.ID` becomes `CAN_DataFrame.ID`. Scripts written
  against the old spelling need updating; the old one was a bug.
- `CcBlock::convert` is removed. It carried both the `_ => raw` fallthrough and
  the reversed rational formula, and it cannot see the text its references point
  at, so it could only guess at the tabular text types. Conversions go through
  `Conversion`, built when the file is opened.
- `Mf4File::comment()` returns the comment rather than the enclosing XML.
- `Mf4File::channel_names()` returns a `Vec<&str>` rather than an iterator.
- `thiserror` 1.0 → 2.0; minimum supported Rust version is 1.80.

### Performance

Median of fifteen runs against the reference, decoding the same 326,623 samples:

| Read | Reference | falcon_mdf | |
|---|---|---|---|
| Uncompressed | 3.74 ms | **1.36 ms** | 2.7× |
| DZ-compressed | 9.34 ms | **2.38 ms** | 3.9× |

A large unsorted file went from 424 ms to 9.7 ms, most of that from fixing what
the reads were doing rather than from making them quicker.

Peak memory reading a 416 MB file fell from 1,291 MB to 826 MB memory-mapped, or
434 MB buffered. For large files the buffered backend uses roughly half the
memory of the mapped one, because mapped data ends up resident twice; this is
documented on `Mf4File::open`.

### Known limitations

- Array (CA) channels are not expanded. They are reported as unreadable rather
  than partially decoded.
- Attachment, event, channel-hierarchy and sample-reduction blocks are not
  parsed. No file available for testing contains one.
- Conversion types 9, 10 and 11 are unsupported.
- Big-endian channels are implemented but untested — no test file exercises one.
- Only MDF 4.11 has been tested; 4.0 and 4.2 are supported in principle.
- Memory scales with the largest data group, which is assembled whole before its
  records are read.
- Writing is not supported.

## [0.1.0]

Initial version: block parsing, memory-mapped and buffered I/O, DT/DZ/DL/HL
traversal, and linear conversions.
