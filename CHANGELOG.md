# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the public API is not frozen: minor versions may contain breaking
changes, and they are listed under **Changed** with the reason.

## [Unreleased]

### Added

- CANopen date and time channels (data types 12 and 13) decode to `CanopenDate`
  and `CanopenTime` rather than opaque bytes. Both are structs, not timestamps:
  the standard defines them as records with named fields, and a date carries a
  day-of-week and a summer-time flag that no instant can represent. Each has
  `to_unix_nanos` for callers who want the instant. Note that the format records
  no time zone, so that conversion treats the fields as UTC.
- Complex channels (data types 14 and 15) decode to `SignalValues::Complex`,
  which holds the real and imaginary parts as separate vectors so that taking
  one part of a channel is a slice rather than a stride.
- `ValueKind` gains `Complex`, `CanopenDate` and `CanopenTime`. None of them is
  numeric: `to_f64` yields `NaN` for all three, since a complex number has no
  single real value and a calendar date is not a scalar.
- Conversion types 9, 10 and 11, which previously reported `Unsupported`.
  Types 9 and 10 are keyed by the sample's own *text* rather than by a number,
  so `Conversion::input()` now reports which conversions work that way and
  string channels are decoded before the lookup — from the record or from a
  variable-length payload stream. Type 11 renders a packed status word as
  labels, resolving the nested conversion each of its masks refers to.

### Fixed

- **Virtual channels were decoded as constants.** A virtual channel — `cn_type`
  3 for a master, 6 for data — occupies no bytes in the record: its raw value is
  the zero-based index of the sample, which its conversion then scales. That is
  how a file stores a regularly-spaced time base without writing a single sample
  of it. The reader had no rule for these and read the zero-bit field instead,
  yielding raw 0 for every sample, so a virtual master came back as a flat line
  rather than a time base. `ChannelType::is_virtual` is new, and such a channel
  now reads as `u64` when it carries no conversion, since its index does not fit
  the zero-width field it declares.

  This was not caught by comparing against a reference implementation over the
  corpus, which is how most defects here were found. Every one of the 543
  virtual channels in that corpus has a conversion factor of 0, which multiplies
  the index away — so a correct reader and a broken one produce identical output
  on all of them. It is pinned now by synthetic files with a non-zero factor.
- **An unrecognised channel data type decoded as a byte array.** The code says
  nothing about the value's width, signedness or byte order, so presenting it as
  bytes was a confident answer to a question the reader cannot answer. Reading
  such a channel now fails and names the code.
- **The `ca_storage` codes for array channels were inverted.** The standard
  assigns 0 to the CN template — a sample's elements adjacent in the record —
  and 1 and 2 to elements stored one per channel or data group. The reader had
  0 and 1 the other way round, so ordinary array channels were refused as
  unreadable while a CG-template array was strided as though its elements were
  adjacent, returning whatever bytes followed the field. The synthetic fixture
  encoded the same inversion, which is why its tests passed.
  `CaStorage::ColumnRow` and `CaStorage::Contiguous` become `CaStorage::CnTemplate`,
  `CgTemplate` and `DgTemplate`.
- **Synchronisation and maximum-length channels decoded as fixed-length.** An
  MLSD channel stores a per-sample length beside its data and a sync channel
  indexes a media stream, so both produced real-looking numbers read from the
  wrong bytes, silently. Both now report `Unsupported`.

### Planned before 1.0

- Block-by-block decoding, to stop memory scaling with the largest data group
- API review and freeze

## [0.2.0] — 2026-08-03

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
- **Embedded attachment bytes were read from past the payload.** A block's
  declared length already covers its embedded data, so reading from the end of
  the block returned whatever followed it — or nothing, when the attachment was
  last in the file.
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
- `#[non_exhaustive]` on the public enums whose variants will grow, so later
  format support does not force a major version. The enums mirroring a file byte
  keep exhaustive matching, since undefined codes already map to `Unknown`.
- `ChannelGroup::sample_reductions` lists the condensed views a group carries,
  and `Mf4File::reduced_signal` reads their mean, minimum or maximum series.
- Channel hierarchy: each node's referenced channels are surfaced as data-group,
  channel-group and channel triples.
- File history: `Mf4File::file_history` returns each entry's timestamp, comment
  and the tool that wrote it. Verified against the corpus — creation time and
  tool identifier match the reference exactly.
- CI across Linux, macOS and Windows: tests, clippy, rustfmt, rustdoc, MSRV and
  a packaging check, plus a `cargo-fuzz` target over the whole read path.

### Changed

Breaking, and listed with the reason:

- `Metadata::len` becomes `Metadata::property_count`. The old pair broke the
  usual contract: a block carrying only a comment reported `len() == 0` and
  `is_empty() == false`. `is_empty` now unambiguously means no comment and no
  properties.
- `VlsdPayloads` is no longer public. It is how variable-length payloads are
  indexed internally; nothing a caller does needs it, and exposing it meant
  committing to `from_stream`, `from_records` and a hint-carrying lookup as API.
- `Metadata` and `UnreadableReason` are now exported from the crate root and the
  prelude. `Mf4File::metadata` and `Channel::unreadable` return them, so naming
  what you were handed previously meant knowing the module layout.
- `Display` on `ValueKind` and `UnreadableReason`, and `Debug` on `Signal` —
  the last summarises rather than printing its samples.

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

- Array (CA) channels decode to their elements when the CA block uses the CN
  template, which stores them adjacently in the record. The CG- and DG-template
  forms, which put each element in its own channel or data group, are reported
  unreadable rather than partially decoded.
- Attachments, events, file history, arrays, channel hierarchy and sample
  reduction are all verified against synthetic files.
- Big-endian channels are covered by synthetic tests only; no available file
  contains one.
- Only MDF 4.11 has been tested; 4.0 and 4.2 are supported in principle.
- Memory scales with the largest data group, which is assembled whole before its
  records are read.
- Writing is not supported.

## [0.1.0]

Initial version: block parsing, memory-mapped and buffered I/O, DT/DZ/DL/HL
traversal, and linear conversions.
