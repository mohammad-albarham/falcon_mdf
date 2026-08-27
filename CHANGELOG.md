# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the public API is not frozen: minor versions may contain breaking
changes, and they are listed under **Changed** with the reason.

## [Unreleased]

### Fixed

- **Variable-length string channels decode as text.** A VLSD channel — `cn_type`
  1 — declaring one of the string data types was reported as raw bytes, leaving
  the channel's own declared encoding for the caller to reapply. Worse, when the
  payloads happened to share a length the fixed-width path claimed them, so two
  samples reading `"abc"` and `"def"` came back as one six-byte blob with no
  record of where the strings ended. They now come back as `SignalValues::Str`.
  Found by `all_datatypes_test.mf4` and `asammdf_dimensional_demo.mf4`, two
  files added to the reference set in this release, and pinned by a unit test.

### Added

- **Write multiple channel groups sharing one data group.** `Mf4Writer::add_group_in(sibling, times)` adds a channel group sharing a data group with an existing `sibling` group. The writer emits an unsorted data block with records of all groups interleaved behind 1-byte record IDs (`dg_rec_id_size = 1`), with `##CG` blocks carrying unique 1-based record IDs and linked via `cg_next`. Refuses transposed codecs (`TransposedDeflate`, `TransposedLz4`) over multi-CG data groups because records of different groups have varying widths. Covered by `tests/write_multi_cg.rs`.
- **Ethernet frame extraction.** `src/eth.rs` exposes `Mf4File::eth_frames(&group) -> Result<EthFrames>`, reading an Ethernet-logged channel group back as frames: timestamp, optional source and destination MAC addresses, EtherType, optional bus channel, and a payload trimmed to `ETH_Frame.DataLength` rather than padded out to `ETH_Frame.DataBytes` width. It refuses rather than truncating when a group's frame fields do not agree sample for sample, matching the CAN and LIN readers' behaviour. Pinned by `tests/eth_frames.rs` round-tripping through `Mf4Writer` across varying payload lengths, optional MAC and bus channels, and missing channel detection.
- **Transposed deflate and LZ4 compression in Mf4Writer.** `Mf4Writer` gains `set_codec` and `codec` using `WriteCodec`, allowing files to be written with `##DZ` zip types 1 (transposed deflate), 4 (LZ4 frame) and 5 (transposed LZ4 frame) in addition to type 0 (deflate). LZ4 codecs are available under the `lz4` cargo feature. Covered by `tests/write_codecs.rs`.
- **Apache Arrow RecordBatch and Arrow IPC export.** `falcon_mdf::export` gains `to_record_batch` and `write_arrow_ipc`, behind the **`arrow`** feature, converting decoded `SignalSeries` to an Arrow `RecordBatch` and writing the Arrow IPC file format directly. Channels keep their own types (integers, floats, text, and binary bytes), samples flagged invalid by MDF invalidation bits are represented as Arrow nulls, and series must share a common time axis (e.g. via `Mf4File::resample`). The existing `parquet` feature now builds on top of the `arrow` feature. Covered by `tests/export_arrow.rs`.
- **`read_bits` differential reference oracle and fuzz target.** `fuzz/fuzz_targets/read_bits.rs` adds a differential fuzz harness driving low-level bit extraction against an independent bit-by-bit reference oracle with explicit bounds checking. Running under `cargo fuzz run --release read_bits` tests bit extraction in release mode without debug assertions to catch silent overflow or calculation bugs.
- **LIN frame extraction.** `src/lin.rs` exposes `Mf4File::lin_frames(&group) -> Result<LinFrames>`, reading a LIN-logged channel group back as frames: timestamp, the 6-bit identifier, bus channel, and a payload trimmed to the length the record logged rather than padded out to the channel's width. It refuses rather than truncating when a group's frame fields do not agree sample for sample, matching the CAN reader's behaviour — a frame is never assembled out of one frame's identifier and another's payload. Frames only: there is no LIN database decoding, and the payload is not interpreted. Pinned by `tests/lin_frames.rs` against `test_data/reference/single_lin_bus_1.MF4` and `multiple.MF4` (4 tests).
- **FlexRay frame extraction.** `src/flexray.rs` exposes `Mf4File::flexray_frames(&group) -> Result<FlexRayFrames>`, reading a FlexRay-logged channel group back as frames: timestamp, the 11-bit masked frame ID, cycle counter, bus channel, payload flags (null frame, sync frame, startup frame), and payload data trimmed to `DataLength` rather than padded out to the channel width. It refuses rather than truncating when a group's frame fields do not agree sample for sample, matching the CAN and LIN readers' behaviour — a frame is never assembled out of one frame's identifier and another's payload. Pinned by `tests/flexray_frames.rs` (7 tests).
- **The bus panel reads logged LIN traffic.** `gui/src/panels/bus.rs` decides from a group's channel names whether it composes its fields under `CAN_DataFrame` or `LIN_Frame` and reads it with the matching reader. A LIN group displays index, timestamp, identifier in hex and decimal, bus channel, length and payload, and hides the DBC controls because this build has no LIN database decoding.
- **The viewer package is a library plus a thin binary.** `gui/src/lib.rs`
  exposes `app`, `decimate`, `format`, `job`, `loader`, `model`, `panels`,
  `recent`, `search`, `session` and `signal_loader`. This makes the viewer's
  logic testable without an open window, covered by 9 integration test files in
  `gui/tests/`.
- **Channel search modes.** `gui/src/panels/channel_list.rs` and `gui/src/search.rs`
  match Substring, Wildcard (`*` and `?`) and Regex — supporting literals, `.`,
  postfix `*` `+` `?`, `[abc]`, `[^abc]`, `^` and `$`; alternation and groups
  are refused with an informative error rather than silently mismatching. A
  malformed pattern displays its parse error and leaves previous results in
  place. A "Plot all matching" button adds up to 32 matches to the plotted set
  and reports how many were left.
- **The Numeric panel.** `gui/src/panels/numeric.rs` adds the sixth content tab,
  answering what every plotted channel was doing at a single instant in time: a
  time box with "Start" and "End" buttons jumping to the bounds of the plotted
  range, one row per channel with its value and the timestamp of the sample
  used. The value is the sample at or immediately before that time, never
  interpolated; invalid samples are skipped backwards and counted.
- **Plot absolute-time axis, custom styling, and region statistics.**
  `gui/src/panels/plot.rs` adds a toggle between relative seconds and an
  absolute UTC wall-clock x axis (`YYYY-MM-DD HH:MM:SS.mmm`), per-signal color
  pickers and line width controls (1.0–4.0), and a statistics table over the
  window between cursors A and B reporting sample count, excluded count,
  minimum, maximum, mean, and delta.
- **Sample table sorting, filtering and export.** `gui/src/panels/table.rs`
  adds column header sorting (ascending, descending, then back to file order),
  a row filter matching across all cells, and "Export table…" writing the
  displayed rows to CSV. Invalid samples sort last in both directions.
- **Structure tree filtering, Expand/Collapse, and group plotting.**
  `gui/src/panels/tree.rs` adds a search filter box that hides non-matching
  channels and groups, "Expand all" and "Collapse all" buttons, and a "Plot all"
  button on each channel group header adding up to 16 readable, non-master
  channels.
- **Measurement cursors and view fitting in the plot panel.** `gui/src/panels/plot.rs` adds two placeable measurement cursors, A and B, behind a "Cursors" toolbar toggle. While active, left-clicking in the plot places cursor A and Shift-clicking places cursor B, each drawn as a labelled `VLine` in its own colour. A readout row under the plot shows time at A, time at B and delta t, followed by the value at A, value at B and the delta for every visible channel, reusing the hover readout's nearest-sample helper. Toolbar buttons provide "Clear cursors" and "Fit view", which resets plot bounds to the full time range of the plotted channels.
- **The channel search list matches more than names.** `gui/src/panels/channel_list.rs` matches queries against the channel name, its unit, its comment and its channel group acquisition name. Result rows show the group each match came from so two same-named channels in different groups are distinguishable. Filter toggles beside the search box offer "Arrays only", "Unreadable only" and "Master channels only", combining with the query and each other, and a count line reports "N of M channels". The filtered list is cached and rebuilds only when the query or toggles change.
- **A file can be read as a file, not only as a measurement.**
  `Mf4File::block_map` walks the block graph from the identification block to
  the last block before EOF and returns every block it finds, in address order,
  with its length, its links under the format's own names for them, the blocks
  that point at it, and a line built from its own fields — a channel's name and
  bit layout, a compressed block's before-and-after sizes, a text block's text.
  A link that points at something that is not a block is reported rather than
  followed, and the bytes no block covers come back as gaps, so "this file is
  5 MB and its blocks account for 7 KB" is visible rather than invisible. The
  walk reads block headers and a short prefix of each block, never a data
  block's payload, so mapping a file costs about one read per block. Paired
  with `Mf4File::read_raw`, which hands back the bytes at any offset, and with
  the `block_map` example, which prints the whole map. Over the 67-file
  reference set the walk produces no warnings.
- **The viewer shows the whole file, not only its channels.** `falcon` is now
  two panes: on the left the file — a structure tree from the identification
  block down to a single channel, the address-ordered block list with alignment
  padding and uncovered regions marked, and the searchable channel list — and
  on the right whatever is selected there: details, plot, samples, bus frames
  or statistics. A block's detail view lists its links as buttons, so the
  graph can be walked in the direction the format defines, and shows its bytes
  as a hex dump. New with it: a virtualized sample table showing decoded values
  in their own types, a CAN frame list that decodes payloads against a DBC, and
  per-channel statistics with a distribution. A channel group, a channel and a
  data group each link to the block that defines them. The plot carries two
  measurement cursors with per-channel deltas between them; the bus view reads
  LIN groups as well as CAN ones — hiding the database controls for LIN, which
  this build has none of — and charts DBC-decoded signals over time as well as
  listing frames; and the channel search matches units, comments and group
  names rather than names alone, with filters for arrays, unreadable channels
  and masters.
- **A video stream is read back from a file this crate did not write.** MDF 4
  stores video as a synchronisation channel — `cn_type` 4, whose samples index a
  media stream — plus an attachment naming that stream. No published sample set
  contains one, because the attachment is external and a real example is a
  multi-file vehicle recording, so `scripts/make_video_fixture.py` writes one
  with asammdf and `tests/sync_channel.rs` reads it. The assertions are narrow
  on purpose: asammdf writing and asammdf checking would be circular, and a sync
  channel has no values to verify anyway. What is pinned is that the refusal is
  a good one — the file opens, the master channel still reads, the error names
  the feature instead of handing back frame indices as measurements, and the
  attachment survives with its media type. CI runs it alongside the writer
  conformance test, both directions against the same outside oracle.

### Changed

- **The reference set grows to 67 files and is fully covered by ground truth.**
  Four files from CSS Electronics' converter test data extend it past the
  finalized 4.10 files it held: 4.11, three of them unfinalized, one logging
  LIN, and one pair being the same measurement before and after finalization.
  `every_reference_file_opens` now walks the directory rather than the ground
  truth, and fails when a fetched file has no recorded values — the drift that
  had left six fetched files checked by nothing.
- **The ground-truth generator no longer flattens what it cannot compare.**
  Complex channels were coerced to float, silently dropping the imaginary part;
  they are now recorded and asserted as both parts. Composed bus frames such as
  `CAN_DataFrame` were mistaken for array channels and compared against a single
  sub-field; they are now recorded as structures and left unasserted, their
  children being separate channels that are compared.

### Added

- **MDF 3.x reading.** `falcon_mdf::mdf3::Mdf3File` opens MDF 3.x files (2.14,
  3.20, 3.30), reads their structure, decodes channels in their own type, and
  applies conversions — linear, tabular, polynomial, exponential, logarithmic,
  rational, algebraic formula, and text tables. Behind the `mdf3` feature, off
  by default.
- **Linked-data, data-value and data-invalidation blocks.** The MDF4 reader now
  follows `##LD` list-data chains, reads `##DV` data-value blocks and `##DI`
  data-invalidation blocks, and records the invalidation bits a `##DI` carries.
- **Zstd and LZ4 compressed data blocks.** The six `##DZ` zip types are all
  decoded: deflate, transposed deflate, zstd, transposed zstd, LZ4 and
  transposed LZ4. Behind the `zstd` and `lz4` features, both off by default.
- **DBC extended multiplexing (`SG_MUL_VAL_`).** Extended multiplexed signals
  are mapped with range selectors, matching the selectors the DBC declares.
- **DBC global value tables (`VAL_TABLE_`).** Per-signal `VAL_` entries still
  override a global table when the same raw value appears in both.
- **J1939 source-address matching.** `IdMatching::J1939PgnAndSource` matches by
  exact `(PGN, source)` pair before falling back to PGN alone, for databases
  that carry the same parameter group for several ECUs.
- **ARXML dynamic multiplexed PDUs.** The ARXML walk resolves `MultiplexedIPdu`
  static and dynamic parts by selector field, and synthesises the selector
  switch signal when the PDU does not name one explicitly.
- **LIN Description File decoding.** `CanDatabase::from_ldf_path` parses `.ldf`
  files and decodes LIN frames into physical signals with units and value tables.
- **Typed writer.** `Mf4Writer` writes channels in their own type — integer
  width and signedness, float precision, fixed-length string, byte run — and
  can carry a conversion rule and per-sample validity into the written file.
- **Deflated writer output.** `Mf4Writer::set_compression` deflates each
  group's records into a `##DZ` block behind the `##HL`/`##DL` pair.
- **Time-domain operations.** `Mf4File::cut` slices channels to a time window;
  `Mf4File::resample` re-grids them onto a fixed raster with step-hold or
  linear interpolation.
- **Multi-channel operations.** `Mf4File::filter` picks named channels out of a
  file; `concatenate` joins measurements end to end; `stack` overlays them
  aligned by start time or recorded time.
- **Batched channel reading.** `Mf4File::signals` decodes any set of channels
  in one pass, assembling each channel group's records once.
- **Signal algebra.** `SignalSeries` implements `Add`, `Sub`, `Mul`, `Div`
  against both other series and scalars, with automatic resampling onto the
  union of their timestamps.
- **Channel search.** `find_channels` returns exact name matches;
  `search_channels` supports substring, wildcard and regex queries.
- **Anonymisation.** `scramble_file` replaces every piece of identifying text
  with random bytes of the same length, leaving sample data and decoding
  formulas untouched.
- **Parquet export.** `write_parquet` and `write_parquet_with` write decoded
  channels to Apache Parquet, preserving native column types. Behind the
  `parquet` feature, off by default.
- **MAT export.** `write_mat` writes decoded channels to MATLAB level 5
  MAT-files. Behind the `mat` feature, off by default.

### Changed

- **Documentation rewritten.** The README now describes the library as it is
  after 52 commits of feature work: MDF 3.x, linked data, all six `##DZ` zip
  types, DBC extended multiplexing and global value tables, J1939
  source-address matching, ARXML dynamic multiplexed PDUs, LDF decoding, the
  typed writer, and the time-domain and multi-channel operations added in this
  round. Every claim is verified against the source before it is written.

## [0.4.0] — 2026-08-05

### Added

- **CAN frame extraction.** `Mf4File::can_frame_groups` finds the channel
  groups a bus logger wrote, and `Mf4File::can_frames` reads them as frames:
  a timestamp, an identifier, an extended-identifier flag, a bus channel and a
  payload trimmed to the logged data length. This is phase B1 of
  `plan_bus_decoding.md`, and it needs no new dependency — the frame fields
  were already reachable as ordinary channels, and what was missing was the
  layer that recognises a group as bus-logged and assembles frames from it.
  The payload is handed back uninterpreted; decoding it into named physical
  signals still needs a CAN database, which this crate does not read.
- `ChannelGroup::is_bus_event` and `is_plain_bus_event` expose the
  channel-group flags read since 0.3.0 but consumed by nothing until now.
- **Streamed reading.** `Mf4File::signal_chunks` reads a channel a bounded window
  of the record stream at a time, so peak memory no longer scales with the
  largest data group. Each chunk is an ordinary `Signal` over that window's
  records, which means every decoding path is the one `signal()` already uses
  rather than a second implementation of it.

  Windows are bounded by bytes (4 MiB) rather than by data block, because block
  size is the writer's choice — every bus log in the test corpus is a single
  large `DT` block, and chunking per block would have held one entire and bounded
  nothing. Compressed blocks are still inflated whole, since deflate cannot be
  entered part-way.

  Three things the format allows are handled rather than approximated: records
  straddling a window or block boundary are carried across; unsorted data groups
  are demultiplexed per window, mirroring where the open-time record index stops
  so that a streamed and a whole read of the same file agree; and a
  variable-length channel whose payloads are records of a companion group — how
  bus loggers write frame payloads — has a per-chunk payload index built with
  offsets continuing across chunks.

  One shape is refused with `Mf4Error::Unsupported`: a variable-length channel
  whose payloads live in its own signal-data block, a second block chain that
  would have to be walked in lockstep. No file in the test corpus has one.

- **Bus signal decoding against a CAN database.** `CanDatabase::decode(id, payload)`
  turns a frame's bytes into named physical signals with units — bit position and
  width, Intel and Motorola byte order, signedness, factor and offset, and
  multiplexed signals. The database is described by `SignalDef`/`MessageDef` in
  `candb`, which is format-neutral, so one decoder serves both front ends and the
  bit-extraction tests cover both:

  - `CanDatabase::from_dbc` / `from_dbc_path`, behind the **`dbc`** feature, reads
    DBC files via `can-dbc` 10.x.
  - `CanDatabase::from_arxml_path`, behind the **`arxml`** feature, reads AUTOSAR
    ECU extracts via `autosar-data`, walking CAN and J1939 clusters down through
    frame triggerings, frames, PDUs, signal mappings, base types and compu methods.

  Both are off by default: reading plain measurement files should not pull in a
  database parser.

  Not covered: DBC extended multiplexing (`SG_MUL_VAL_`) and the dynamic parts of
  a multiplexed ARXML PDU — which are left out rather than reported, since
  reporting all of them would hand back signals overlapping in the payload as
  though they were simultaneously present.

- **Decoded bus signals as time series.** `Mf4File::decode_bus(&database)` reads
  every CAN frame in a file and returns `BusSignals`: one `BusSignal` per signal,
  carrying all of its readings and their timestamps. Decoding frame by frame
  already worked, but it left the accumulation loop to the caller, which is the
  difference between a building block and an answer.

  Decoded signals are a **namespace of their own** and deliberately do not appear
  in `channels()`. They are derived rather than recorded — their existence depends
  on a database the file does not contain — and folding them in would make
  `channel_count()` depend on an argument. This settles the third open question in
  `plan_bus_decoding.md` §8.

  A series is identified by bus, message and signal *together*. Neither the name
  nor the identifier is enough on its own: two messages may spell one signal name,
  and the same identifier on two buses of a multi-bus logger is two different
  signals, so keying on less would silently interleave unrelated networks.

- **J1939 parameter-group matching.** `CanDatabase::with_matching(IdMatching::J1939Pgn)`
  matches a frame by its parameter group number when no message carries the whole
  identifier, ignoring the priority and the sending ECU's source address.

  Without it a real heavy-duty log decodes to *nothing*: a J1939 DBC keys EEC1 as
  `0x0CF004FE`, while the truck in the test corpus transmits it as `0x0CF00400`
  and `0x0CF00421` — two ECUs, neither of them the null address the database
  names. Selected per database rather than inferred, because whether a 29-bit
  identifier is a parameter group is not something the bits can be asked. Exact
  matches still win, so enabling it cannot change a message that already matched.

- **DBC value tables decoded to text.** `DecodedSignal::text` and
  `BusSignal::text_at` give the label a `VAL_` table attaches to a reading —
  `Neutral` rather than `0`. Looked up against the raw value after sign extension
  and before scaling, which is what a `VAL_` entry names; a value the table does
  not cover is left unlabelled rather than mislabelled.

### Changed

- **`DecodedSignal` gained a `text` field** and **`SignalDef` gained a
  `value_table` field**, both for the value-table support above. Breaking for code
  that constructs either with a struct literal; code that reads `DecodedSignal`
  is unaffected, since `value` still holds what it always did.

- **MSRV raised from 1.80 to 1.88**, and it now means something it did not
  before. The `arxml` feature already could not be built on 1.80, so the declared
  number was a promise kept only for consumers who enabled nothing. It is now
  declared at the floor every feature combination actually reaches, and CI builds
  `--all-features` on it.

  1.88, not the 1.85 previously documented here. Edition 2024 is only what
  `autosar-data` declares; what it *uses* is let-chains, stable since 1.88.
  Neither `autosar-data` nor `autosar-data-specification` sets a `rust-version`,
  so cargo reports nothing and the real floor is visible only by building — 1.87
  fails, 1.88 succeeds.

  With 1.80 no longer being held, `can-dbc` moved from 7.x to **10.x**; the pin
  existed solely to stay under 1.83. Two things improve as a result: `VAL_` keys
  are parsed as integers rather than as `f64` and then filtered for integrality,
  and `SG_MUL_VAL_` extended multiplexing is now reachable from the parser —
  though this crate does not yet map it (see below).

- **The parsed-block cache (`BlockCache`) stops storing entries past 100,000
  per block kind**, rather than growing with the file's structure uncapped. A
  file whose structure is large enough to fill the cache now costs repeated
  parsing — lookups beyond the cap parse every time — instead of unbounded
  memory held for the life of the file.

### Removed

- **`DzBlock::decompress` is removed.** It reserved `original_size` bytes from
  an untrusted file field with no clamp, bypassing the `Limits` machinery the
  internal decompression path goes through — so a crafted block could force
  exactly the allocation the rest of the crate is written to prevent. It had no
  callers. The internal path, the one `Mf4File::open` uses, is unaffected.

### Fixed

- **A DL (data list) block's declared entry `count` was fed straight to
  `Vec::with_capacity` before any byte of it was read.** A crafted block could
  force a multi-gigabyte allocation and abort the process — an abort a caller
  cannot catch. The count is now checked against the block's actual data
  section, and the block rejected as truncated when it does not fit.

- **`Signal::value_at` on a signal with zero samples underflowed while
  formatting its out-of-range error**, which panicked in debug builds. The
  error now reports the sample count itself.

- **The docs carried two defects.** `Mf4File::file_history`'s documentation no
  longer opens with `channel_hierarchy`'s text, and `signal_chunks` had two
  `# Errors` sections plus a stale "one data block at a time" description — it
  reads bounded windows.

### Planned before 1.0

- API review and freeze. One input now on the table: `can-dbc` 10.x exposes
  `SG_MUL_VAL_`, whose selectors are *ranges* and may name their multiplexor.
  `Multiplexing::Selected(u64)` cannot express that, so if extended multiplexing
  is ever to be supported, the enum has to change before it is frozen.

## [0.3.0] — 2026-08-05

The release that gains writing (`Mf4Writer`), CSV export, a GUI, and a long
list of reader fixes from a spec audit of the block layouts.

It closes with a second audit, of a kind the first did not cover. Six enum
and flag tables decoded conformant files into something the file did not
say — event types and sync domains, sample-reduction domains, hierarchy
types, attachment flags and channel-group flags. They shared a cause: each
was guarded by a fixture written to match the parser rather than the
standard, and each asserted the fields *around* the value while never
asserting the value itself. Every discriminant table in the crate has now
been checked against an independent implementation and covered by a test
written from the standard; six of the fifteen were wrong.

Four of those change public enum variants, so this is a breaking release.
`EventType::ExternalStart`/`ExternalStop`, `ChType::Tree`/`Plain` and
`AtFlags::crc32_valid` are gone, and `CgFlags` gained two fields.

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

- **`ch_type` has nine values, and this crate knew two.** A channel-hierarchy
  node's type names its role — group, function, structure, map list, a
  function's input/output/local variables, a calibration definition or a
  calibration object. This crate read 0 and 1 as `Tree` and `Plain`, described
  as ordered versus unordered elements, a distinction the standard does not
  draw: `ch_type` 1 means *function*. Everything from 2 up decoded as
  `Unknown`. `ChType::Tree` and `ChType::Plain` are replaced by the nine roles.
  Unlike the other fixes here this one rests on a single independent
  implementation (mdflib) rather than two, since asammdf does not model the
  hierarchy's semantics at all — but the mapping it replaces described
  something the format has no concept of.

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

## [0.1.0] — 2025-12-24

Initial version: block parsing, memory-mapped and buffered I/O, DT/DZ/DL/HL
traversal, and linear conversions.
