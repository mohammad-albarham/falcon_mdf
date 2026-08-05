# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the public API is not frozen: minor versions may contain breaking
changes, and they are listed under **Changed** with the reason.

## [Unreleased]

### Fixed

- **Event sync domains were shifted by one.** `ev_sync_type` is numbered from
  1 (seconds, radians, meters, index); this crate numbered it from 0, so an
  event recorded in seconds read back as `Angle`, one in radians as
  `Distance`, and an index event fell off the end as `Unknown(4)`. Nothing in
  a conformant file ever decoded to `EvSyncType::Time`, which silently emptied
  every consumer that selects time-domain events — including the GUI's plot
  markers, whose event flags therefore never appeared on any real file.
- **Event types 2 and up named the wrong thing.** `ev_type` 2–6 are
  acquisition interrupt, start recording trigger, stop recording trigger,
  trigger and marker. This crate read 2 and 3 as external start/stop, which
  shifted trigger and marker down by one and left a real marker (6) decoding
  to `Unknown(6)`. `EventType::ExternalStart` and `ExternalStop` are replaced
  by `AcquisitionInterrupt`, `StartRecordingTrigger` and
  `StopRecordingTrigger`.
- **Compressed attachments were handed back still compressed.** `AT_FL` bit 1
  marks the embedded bytes as deflate-compressed; it was read as a
  checksum-valid flag, so `attachment_data` returned a raw deflate stream as
  though it were the attached file. It now decompresses, under the same
  expansion limit that guards compressed measurement data. Bit 2 — whether
  `md5_checksum` means anything at all — was never read, so those sixteen
  bytes could not be told apart from sixteen zeros.
- **Channel-group flags stopped at bit 2.** MDF 4.2's bit 3 (remote master,
  where the group's time base lives in another channel group) and bit 4 (event
  signal group) were not read, so a remote-master group was indistinguishable
  from an ordinary one. Both are now reported. Resolving `cg_cg_master` to
  read such a group's master channel is still not implemented; the flag says
  the group is one.
- **Sample-reduction sync domains were shifted by one**, the same defect as
  the event one above and found by looking for it: `sr_sync_type` is numbered
  from 1, so a reduction condensing over seconds reported itself as `Angle`
  and its `interval` was attributed to the wrong domain. Confirmed against
  mdflib, whose `SrSyncType` spells out `Undefined = 0, Time = 1, Angle = 2,
  Distance = 3, Index = 4`, before being changed.

  These went unnoticed because the fixtures covering them were written to
  match the parser rather than the standard, and asserted only the fields
  around the enums — never the enums themselves. Each is now covered by a
  table test spelling out every value the standard defines, written from the
  standard.

- `write_csv` now escapes header fields per RFC 4180: a channel name or unit
  containing a comma, quote or line break is wrapped in quotes with its
  quotes doubled, instead of splitting into extra columns in whatever opens
  the file. Names without special characters export exactly as before.

### Added

- `Mf4Writer` creates MF4 files from scratch — Phase 5's first item: one data
  group per channel group, records sorted by time, raw little-endian float64
  samples in DT blocks, an implicit `Time` master per group, and invalidation
  bits when the caller hands over per-sample validity (`add_channel_with_validity`).
  Mismatched sample counts and NaN timestamps are refused with
  `Mf4Error::WriteError` rather than guessed at. Verified three ways: read
  back through this crate's spec-audited reader (with a mutation check on the
  record layout), byte-pinned header fields, and asammdf as an independent
  oracle — which drops exactly the samples marked invalid, confirming the
  bit's polarity against a second implementation. The GUI's plot panel gains
  `Export MF4…` beside `Export CSV…`: each plotted channel becomes a group in
  a new file, validity carried over so the export keeps the gaps the source
  declared, and the start time inherited for provenance.
- `falcon_mdf::write_csv` exports decoded channels as CSV: one time column
  taken from the first channel's master, one value column per channel, nine
  decimals, `Time [unit]`/`Index` header exactly as the `export_to_csv`
  example always produced — the example now writes through the same function,
  so a single-channel export is byte-identical to it and the GUI's export
  action cannot drift from the example's format.
- `Mf4File::channel_at` resolves a `ChElement`'s (data group, channel group,
  channel) block offsets to the `Channel` they locate. The hierarchy accessor
  hands back offsets no caller could map to a channel on its own, since
  channels do not publish their offsets; a dangling triple resolves to `None`
  rather than a guess.
- `UnreadableReason::SyncChannel`: a synchronisation channel (`cn_type` 4)
  now reports itself unreadable at parse time, where a channel list can show
  the reason before any read is attempted, instead of only failing on the
  first decode. The decode-time refusal remains as the backstop for signals
  assembled by hand.
- `Mf4File::channels_matching(predicate)` returns every channel whose name
  the predicate accepts, sorted by name with position as the tie-break — the
  substring-search primitive a channel list needs, which exact-match
  `find_channels` used to force callers to rebuild by hand.
- `Channel::sample_count`: the channel group's corrected sample count,
  copied onto each channel when the file opens, so a bare `&Channel` carries
  its own count instead of indexing back into the groups.
- `ChannelHierarchyNode::children`: `channel_hierarchy()` now descends
  `ch_first` and returns the whole tree. A node a corrupted file links twice
  is visited once — a cycle between levels truncates rather than recursing
  forever.

- `Mf4File::unfinalized()` reports what a writer left undone when it stopped
  before finalising a file — the seven `id_unfin_flags` bits as a typed
  `UnfinalizedFlags`, plus the writer's own custom flags word, or `None` for a
  finalized file. Two of the seven this reader already compensates for: sample
  counts come from the data rather than from `cg_cycle_count`, and a last data
  block whose declared length is zero is read to the end of the file. The rest
  are reported and not acted on, so a caller can tell a file with stale counts
  from one whose variable-length offsets were never written. Inventing the
  missing values would be guessing, and refusing the file would withhold the
  channels that are fine.
- Maximum-length channels (`cn_type` 5) decode instead of reporting
  `Unsupported`. Such a channel keeps its data in the record, sized to the
  longest sample it will hold, and its `cn_data` link names another channel of
  the same group whose value counts the bytes each sample actually uses — where
  the same link on a variable-length channel points at a signal data block.
  Samples come back as `VarBytes` even when they happen to be uniform, since a
  fixed width would erase the difference between the bytes used and the bytes
  available. A sample claiming more bytes than the field holds is rejected
  rather than clamped, and a channel naming no length channel remains
  unreadable, now with an accurate reason.
- CANopen date and time channels (data types 13 and 14) decode to `CanopenDate`
  and `CanopenTime` rather than opaque bytes. Both are structs, not timestamps:
  the standard defines them as records with named fields, and a date carries a
  day-of-week and a summer-time flag that no instant can represent. Each has
  `to_unix_nanos` for callers who want the instant. Note that the format records
  no time zone, so that conversion treats the fields as UTC.
- Complex channels (data types 15 and 16) decode to `SignalValues::Complex`,
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

### Changed

- `Conversion::ValueToText` and `Conversion::RangeToText` carry `TableEntry`
  values instead of `String`, since a reference may name a nested conversion.
  `Event::range_start_name` becomes `Event::name`, and `EvBlock` gains
  `ev_range_start` and `tx_name`.
- `Channel` gains an `all_invalid` field. `Channel` is not `#[non_exhaustive]`,
  so callers building one literally must add it — worth settling before the API
  freeze rather than after.
- `CaFlags` is renumbered onto the standard's bit positions and gains
  `left_open_interval` and `standard_axis`; `has_axis` becomes `axis`, and
  `axis_conversion` and `axis_name` are gone — the latter was never a flag the
  format defines. `CaBlock::ca_scale_axis`, `ca_axis_cc`, `ca_precomputed_min`
  and `ca_precomputed_max` are replaced by `ca_axis` (a `Vec<AxisRef>`, since
  the standard locates an axis with a data-group / channel-group / channel
  triple) and `ca_axis_conversion`. `CaArrayType::TypeTemplate` becomes
  `Lookup` and `FixedLength` is removed, neither being what code 2 and 3 mean.
- `DataType` gains a `StringSbc` variant for single-byte-coded (ISO-8859-1)
  text, which the enum had no representation for. This is a breaking change:
  `DataType` is deliberately not `#[non_exhaustive]`, so callers matching it
  exhaustively must add an arm. That is the reason it is being done now rather
  than after 1.0 freezes the surface.

### Fixed

- **An array whose CA block names no element template was refused.** The
  template is optional: without it the parent channel's own data type and bit
  count describe one element, and `ca_byte_offset_base` gives the stride.
  Vector and dSPACE both emit look-up tables and matrices this way.

- **The inverse-layout flag was parsed and ignored.** `ca_flags` bit 6 says the
  first dimension varies fastest in the record, so the stored order is the
  transpose of the row-major order `SignalValues::Array` reports. A matrix came
  back transposed — the right values in the wrong positions.

- `Channel::value_kind` reported an array channel's element type where such a
  channel decodes to `SignalValues::Array`, whose elements are always f64.

- **Range conversions treated the upper bound as inclusive.** MF4 partitions
  types 6 and 8 on half-open ranges `[lower, upper)`, so a sample landing
  exactly on a boundary belongs to the *next* range. Both sites tested
  `raw <= upper`, giving every boundary value the previous range's label or
  physical value — silently. With Vector's table of `[1,3)` `[3,5)` `[5,7)`, a
  raw 3 read as "very low" where the file means "low".

- **A `cc_ref` naming a nested conversion was rejected as malformed.** Types 7
  and 8 are "value to text/**scale**": a reference may name a CC block instead
  of a label, applying that conversion to the raw value. It is how a file writes
  a piecewise conversion — one formula below a threshold, another above — or a
  mostly-numeric channel with labels for a few special values. Three of Vector's
  reference files could not be opened at all. `Conversion::ValueToText` and
  `RangeToText` now hold `TableEntry` values, and a table of nothing but nested
  conversions reports itself numeric.

- **The EV block's links were off by one from index 2.** Link 2 is
  `ev_ev_range`, pointing at the *event* that opened a range, not at text; 3 is
  the name and 4 the comment. The parser read 2 as text — refusing any file
  whose events use ranges — read the name as the comment, and never read the
  comment. `Event::range_start_name` becomes `Event::name`, which is what it
  always held.

- **A look-up array whose elements are arrays failed the whole file.** Such an
  array composes CA with CA, and the reader parsed the target as a CN
  unconditionally. One unreadable channel is the honest cost; the rest of the
  file is not.

- **A numeric conversion turned a text channel into numbers.** `value_kind`
  consulted the conversion before the data type, so a string channel carrying
  any non-identity numeric conversion was decoded as a number — its text read
  as an integer and pushed through the conversion. The data type decides what
  the record holds; a conversion keyed by numbers cannot consume text, so it no
  longer applies. Conversion types 9 and 10 are keyed by text and still do.

  Found on the first real file from another tool to carry a string channel:
  `ASAP2_Demo_V171.mf4` hangs an identity *rational* conversion on a 256-byte
  ISO-8859-1 field, which this reader returned as 0.0 for every sample.

- **A data block this build cannot read was reported as no samples at all.**
  A data group pointing at an unrecognised block fell through to an empty
  index, so the file opened, its channels appeared, and every sample silently
  became zero samples — a plausible answer to "what does this file contain"
  from a reader that could not answer it. The same fallthrough inside a data
  list was worse: a list holds one block per segment of a group's records, so
  skipping an entry dropped a slice out of the middle of the stream and shifted
  every segment after it, leaving real values at the wrong times. Both now fail
  and name the block. A 4.2 file's `##LD` reaches exactly this path.

- **A channel the file declares wholly invalid reported every sample valid.**
  `cn_flags` bit 0 was parsed and then dropped on the way to `Channel`, so
  `validity()` returned `None` — "no invalidation information, everything is a
  measurement" — for a channel saying the opposite, and handed its bytes back
  as data. The flag stands alone: it needs no per-sample invalidation bit and
  no invalidation bytes in the group, so it is now answered before the record
  is consulted. Such a channel reports every sample invalid, and its
  neighbours in the same record are unaffected.

- **The CA block's flags, link partition and data section were all misread.**
  `ca_flags` bit 0 was read as "has axis" where the standard has dynamic size,
  a flag for axis names was invented outright, and so was a precomputed
  minimum/maximum region in the data section. The links each flag introduces
  are (data group, channel group, channel) *triples*, not one link per
  dimension, and the composition link is the first link whatever `ca_type`
  says. An array declaring a fixed axis therefore came back with its axis
  values dropped, one of them reported as a precomputed minimum, and its axis
  *conversion* link reported as a scale axis. Element values were unaffected,
  since the composition link and the dimension sizes both precede everything
  optional.

  As with the array storage codes before it, the fixture encoded the same
  misreading — it derived every optional section from `CaFlags` — which is why
  six tests passed against a block no writer would emit. It now takes its bit
  values and its layout from the standard.

- **Arrays whose size varies per sample were decoded at their maximum.** The
  dimension sizes of a dynamic-size array are the largest shape a sample may
  take; the size each sample uses lives in another channel. Decoding it as a
  fixed shape returned the unused tail of the field as though it were data.
  Such a channel is now reported unreadable. This was invisible until the flags
  above were read at their standard positions.

- **`cn_data_type` was read one code too low from 6 upwards.** The standard
  assigns 6 to a single-byte-coded string (ISO-8859-1), a type this reader had
  no variant for at all, so every code above it shifted: a UTF-8 channel was
  decoded as UTF-16 and a UTF-16LE channel byte-swapped — both silent garbage
  text — a UTF-16BE channel came back as raw bytes rather than a string, a MIME
  stream was decoded as a CANopen date, a CANopen date as a CANopen time, and a
  CANopen time as a complex number. The codes are now 6 SBC, 7 UTF-8,
  8 UTF-16LE, 9 UTF-16BE, 10 byte array, 11 MIME sample, 12 MIME stream,
  13 CANopen date, 14 CANopen time, 15 and 16 complex.

  The sample corpus could not have caught this: it carries only types 0, 4 and
  10, and code 10 landed on `MimeSample`, which produces byte-for-byte the same
  output as `ByteArray`. The whole table is now pinned by a test that names
  every code, and a synthetic file carries one channel per string encoding in
  text that is non-ASCII on purpose — ASCII survives all four encodings, so a
  fixture written in it would pass against the shifted table.

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
