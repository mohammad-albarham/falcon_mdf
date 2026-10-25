# Optimization iterations

A log of the performance work done on the core reader/writer (`src/`) and the
GUI (`gui/src/`). Each iteration lists the change, why it is faster, and how it
was measured. Verification follows the AGENTS.md suite (`cargo fmt --check`,
`cargo clippy` on both feature axes, `cargo test --all-targets --all-features`,
doc tests, GUI tests); measured numbers come from the tracked benchmark
harness described in `.agents/skills/perf-benchmark/SKILL.md` unless noted
otherwise.

Timing methodology for the per-iteration numbers: `examples/bench`
(`target/release/examples/bench <file>`), release profile (`lto`, `cg=1`,
`opt=3`), warm page cache, median of 3 runs. `open` = structure parse,
`read_native` = decode every channel to its native type, `read_f64` = second
pass coerced to `f64`. Before/after binaries are kept so both sides run on the
same machine state.

Fixtures:

- `Vector_DataList_Deflate.mf4` (128 KB, DZ-deflate, transposed blocks)
- `J1939 (truck)/.../00002082.MF4` (5.0 MB bus log, 1,600,885 samples, unsorted)
- `large_deflate.mf4` (122 MB, DZ-deflate) / `large_uncompressed.mf4` (480 MB)

Baseline (before any change, median of 3):

| Fixture | open | read_native | read_f64 |
|---|---|---|---|
| Vector_DataList_Deflate.mf4 | 0.11 ms | 1.37 ms | 0.26 ms |
| J1939 00002082.MF4 | 4.36 ms | 6.98 ms | 4.26 ms |
| large_deflate.mf4 | 0.23 ms | 1821 ms | 1855 ms |
| large_uncompressed.mf4 | 0.22 ms | 475 ms | 425 ms |

---

## Iteration 1 — core read path: remove double copies and per-byte div/mod

Status: done. Tests: golden, read_system, batched_read, bus_frames,
stream_chunks, asammdf_untranspose_conformance, write_codecs, write_ca — all
pass (49 tests).

1. **`append_data_block` fast path** (`src/file.rs`): an uncompressed block
   with no invalidation bytes was materialised in a temporary `Vec` by
   `read_block_payload` (`into_owned()`) and then copied again by
   `extend_from_slice` — two copies and one allocation per DT block on the
   dominant whole-group read path. Now the payload is borrowed straight from
   the source (zero-copy under mmap/memory backends) and appended once.
2. **`un_transpose` loop restructure** (`src/file.rs`): the flat loop paid one
   integer division, one modulo and one multiply per byte of every
   transposed DZ block — the layout asammdf writes by default. Replaced with
   a column-major walk whose destination index advances by addition; the
   byte mapping is identical (verified by hand and by
   `tests/asammdf_untranspose_conformance.rs`).
3. **`gather_records` capacity** (`src/file.rs`): the unsorted-gather output
   `Vec` clamped its capacity hint at the `max_alloc` ceiling, so any group
   larger than the ceiling grew through repeated doublings (each a full copy
   of what was gathered). The gathered bytes are a subset of the input
   stream, so the clamp is now `raw.len()` — never below what is needed. The
   now-unused `Limits` parameter was dropped (call sites + unit tests
   updated).
4. **Case-insensitive search lowercases the pattern once**
   (`src/file.rs::search_channels`, `src/channels_db.rs::search`): the
   pattern was re-lowercased (a fresh `String` allocation plus scan) for
   every channel name per query.

## Iteration 2 — core read path: cache the assembled data-group stream

Status: done. Tests: as iteration 1, all pass.

`build_records` read and — for a DZ-compressed file — decompressed the whole
data group for *every* channel group, because only the gathered per-CG
records were cached. A bus log is one unsorted data group with many channel
groups, so reading channels from K groups decompressed the same stream K
times. New `raw_stream_cache` (`BoundedLru<usize, Arc<Vec<u8>>>`, keyed by
data-group index, bounded by the same `CACHE_ENTRIES`/`max_alloc` budget as
the existing caches) holds the assembled stream; the first reader pays, the
rest reuse it. The sorted-group path moves the cached `Arc` straight into
`CachedRecords`, so its record-cache entry now shares bytes with the stream
cache instead of owning a second copy. `vlsd_payloads` (Form 1) reuses the
same cache.

Peak-memory note: worst case now holds the raw stream (≤ `max_alloc`) plus
gathered records (≤ `max_alloc`) where before only the gathered records were
retained; for sorted groups memory is unchanged (same `Arc`). RSS on the
large fixtures is re-measured in the final benchmark run below.

## Iteration 3 — writer and exporters

Status: done. Tests: `export_csv`, `export_formats` (43), `write_conformance`,
`write_typed_conformance`, `write_codecs`, `write_ca`, plus the crate's 344
lib unit tests — all pass.

1. **`transpose` loop restructure** (`src/write.rs`): the writer's forward
   transposition paid div/mod/multiply per byte, mirroring the read-side
   `un_transpose`. Now a row-major walk with the destination index advancing
   by addition (round-trip covered by the codec conformance tests).
2. **Sorted-axis pre-check in `record_bytes`** (`src/write.rs`): callers
   almost always append samples in time order; one linear
   `is_sorted_by(total_cmp)` scan now skips both the O(n log n) sort and the
   per-sample indirection of gathering through an index vector. A genuinely
   shuffled axis still pays the sort.
3. **Compressed-write peak memory** (`src/write.rs`): the deflate encoder's
   output `Vec` starts with half the input as capacity (it grows into it
   incrementally, re-copying at every doubling when started empty), and the
   transposed copy is dropped as soon as the encoder has consumed it, so
   raw + transposed + compressed no longer coexist until function exit.
4. **CSV export row buffer** (`src/export/mod.rs`): rows were built as a
   `Vec<String>` of per-cell `format!`s, then `join`ed, then `writeln!`ed —
   two allocations per cell plus the join. One reused `String` per row is
   byte-identical output with a handful of allocations total.
5. **ASC export frame buffer** (`src/export/asc.rs`): the timestamp padding
   loop reallocated the string once per leading space (up to 9 per frame),
   and each data byte became its own `String` before a `join`. Now one
   reused row buffer per frame; padding happens in place. Byte-identical
   output (pinned by `tests/export_formats.rs` against asammdf reference
   output).
6. **`time_groups` cheap gate** (`src/export/mat.rs`, `mat_v4.rs`,
   `mat73.rs`, `hdf5.rs`): grouping series by time axis did a full
   element-by-element compare of both timestamps vectors for every series
   against every existing group — O(series × groups × samples) before any
   I/O. A (length, first, last) gate rejects foreign axes in O(1), and
   trying the most recently filled group first (`rev`) makes a run of
   same-axis series stop at the first candidate.
7. **Single-pass array-column split** (`src/export/mod.rs::element_columns`,
   used by CSV/MAT/MAT4/MAT73/HDF5/Arrow): each array element column was
   gathered by its own strided walk over the whole values vector, touching
   every cache line `elements_per_sample` times over. One shared helper now
   splits the columns in a single sequential pass.

## Iteration 4 — GUI

Status: done. Tests: `cargo test -p falcon_mdf_gui` — 24 test binaries, all
green (including the new pathological-wildcard regression test).

1. **Decimation cache stopped deep-cloning every frame**
   (`gui/src/panels/plot.rs`): on the common path — an unmoved view — every
   series' full `Vec<Vec<[f64; 2]>>` was cloned out of the cache per frame,
   then `Line::new` converted each segment into a fresh `Vec<PlotPoint>`.
   The cache now stores the points as `PlotPoint`s behind an `Arc`,
   converted once per view change, and the lines draw from
   `PlotPoints::Borrowed` slices. The borrowed slices must outlive the
   `Plot::show` closure (items are kept until tessellation), so each draw
   site keeps a small `segment_store` outside the closure; the overlay plot
   fills it in one pass and borrows in a second, so no borrow overlaps a
   later push.
2. **X-Y panel pairing cache** (`gui/src/panels/xy.rs`): `pair_xy` — an
   O(n) walk with resampling allocations — ran every frame, and the result
   was cloned twice more for the line and the sample markers. The paired
   `XySeries` and its points (pre-converted to `PlotPoint`) are now cached
   and rebuilt only when an axis, the file-B offset or the alignment mode
   changes; cursor lookups still answer from the undecimated series.
3. **Sample-table filter streams instead of materialising**
   (`gui/src/panels/table.rs`): every keystroke built a
   `Vec<Vec<Option<String>>>` of every cell of every row (a million-sample
   group × its columns as formatted `String`s), then lowercased each cell
   again inside `matching_indices`. The filter now formats each cell into
   one reused buffer and matches in place (`contains_ascii_ci`: an
   in-place ASCII case-insensitive scan, with the Unicode-aware comparison
   as fallback for non-ASCII text, so results are identical to
   `matching_indices`). `cell_text` became a thin wrapper over the new
   buffer-writing core.
4. **Bus panel stops cloning the filter result per frame**
   (`gui/src/panels/bus.rs`): the filtered index `Vec<usize>` was cloned
   every repaint while a filter was active, and the two filter `String`s
   were rebuilt per frame just to compare them. The cached list is now
   behind an `Arc` (a handle clone per frame) and the queries are compared
   as `&str` without allocating.
5. **Linear wildcard matcher** (`gui/src/search.rs`): the recursive
   backtracker retries exponentially for patterns like `*a*a*a*a*a*b` — a
   paste into the search box would hang the UI thread. Replaced with the
   classic single-pass greedy star-restart algorithm; pinned by
   `wildcard_pathological_pattern_stays_linear` in
   `gui/tests/channel_search.rs`. The per-call `Vec<char>` allocations in
   `matches` were left alone deliberately — results are cached per query,
   and the file's header states the clarity-over-cleverness tradeoff.
6. **Dropped-file check** (`gui/src/app.rs`): the dropped-files `Vec` was
   cloned every frame; the empty check now runs first.

## Iteration 5 — remaining core hot spots, one correctness fix

Status: done. Tests: `mdf3_records`, `mdf3_conformance`, `mdf3_conversions`,
`write_ca`, `write_multi_cg` (the `raw_values` consumer), `model_tests`, and
the new signal unit test — all pass.

1. **`decode_strided_raw` read every sample one record id late**
   (`src/model/signal.rs`) — a correctness fix found during the performance
   review. `StridedField::offset` already folds in the record offset (it is
   computed as `record_offset + byte_offset` in `strided_offset`), but
   `decode_strided_raw` added `record_offset` on top of it, so in a sorted
   data group carrying record ids, `raw_values()` decoded from a shifted
   window. `values()` and the general path were correct, which is why the
   golden tests never fired — all their layouts have `record_offset == 0`,
   as did the fast-vs-general differential tests. Pinned by
   `raw_values_honours_the_record_offset_once`, which checks exact expected
   values under `record_offset = 1` and that the fast and general paths
   agree.
2. **CA-template member resolution is linear, not quadratic**
   (`src/file.rs::resolve_array_group_members`): each array-element link
   rescanned every data group and channel group; with one link per array
   element that is quadratic in the file's group count. The offsets are now
   indexed into a `HashMap` once per call (first occurrence winning, exactly
   like the scan), and each link looks up in O(1).
3. **MDF3 byte-aligned fast path** (`src/mdf3/records.rs::aligned_bits`):
   every sample of every channel walked a byte-at-a-time `u128` loop plus
   shift and mask. A field that is byte-aligned and a whole standard width
   (1/2/4/8 bytes) is now a single `from_le_bytes`/`from_be_bytes` load —
   the shape of nearly every channel a v3 file holds; bit-packed fields
   keep the generic walk.

## Measured results

### Per-binary comparison (interleaved, same machine state)

The obvious before/after comparison — run the bench binary, apply the
changes, run it again — produced misleading numbers on this machine: absolute
times drift by 20–30 % between sessions (thermal / cache state after long
build and test runs), in both directions. The numbers below therefore come
from keeping the **baseline binary** (`/tmp/bench_baseline`, built at the
pre-optimization tree) and interleaving it with the new binary in the same
shell loop, so both sides see the same machine state:

| Fixture (`examples/bench`) | baseline binary | optimized binary |
|---|---|---|
| J1939 00002082.MF4 (5 MB, unsorted bus log) | 8.2–8.7 ms | **7.4–7.9 ms** |
| Vector_DataList_Deflate.mf4 (transposed DZ) | ~1.05 ms | ~1.04 ms |
| large_uncompressed.mf4 (480 MB) | ~613 ms | ~612 ms |
| large_deflate.mf4 (122 MB) | ~2330 ms | ~2300 ms |

The J1939 log is the file the raw-stream cache targets (one unsorted data
group, many channel groups): a consistent ~8 % faster whole-read. The
transposed-deflate and large fixtures are at parity — their runtime is
dominated by zlib inflate and record decoding, which these iterations did not
target; the removals (double copies, div/mod loops, per-cell allocations)
sit below measurement noise there.

### Tracked comparison (benchmarks/, regenerated after all iterations)

Full run per the perf-benchmark skill: corpus (78 files, 5 runs, `--select`)
plus the separate large-fixture pass (3 runs, `--tag large`).
`benchmarks/COMPARISON.md` was refreshed by hand from the four generated
files and the sync check printed `PASS`. Headline rows from
`latest_report.md` / `large_report.md`:

| Workload | vs `get()` | vs `select()` |
|---|---|---|
| Corpus files > 1 MB (8 reference + 2 fixtures) | 5.0× | 3.3× |
| 480 MB uncompressed | 3.0× | 1.5× |
| 122 MB transposed-deflate | 4.9× | 1.1× |

falcon is faster on 78/78 files. Memory: the new raw-stream cache did **not**
move RSS — the fixtures measure 1371 MB (deflate) and 1672 MB
(uncompressed) peak, the same as before (the cache stores the same `Arc` the
record cache holds; sorted groups share one copy). The 5 MB J1939 log's
falcon RSS improved from 36.2 MB to 29.9 MB.

One corpus-composition note, recorded in COMPARISON.md's header: the harness
glob now picks up `test_data/large/`, so the corpus is 78 files where earlier
runs said 76 — the two generated fixtures are inside the main run (and still
measured separately for the fixture report).

GUI changes are not exercised by this harness (they are per-frame costs);
their effect is structural — allocation counts per frame/keystroke — and is
reasoned about in iteration 4 rather than quoted as a timing.

## Verification

The full AGENTS.md suite was run after all iterations:

- `cargo fmt --all --check` — clean
- `cargo clippy --all-targets -- -D warnings` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test --all-targets --all-features` — 63 test binaries, 776 passed,
  0 failed (corpus present, so the reference tests ran rather than skipping)
- `cargo test --doc --all-features` — 35 passed
- `cargo test -p falcon_mdf_gui` — 24 test binaries, all green
- `.venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py`
  — `PASS`

## Iteration 6 — the deferred list from iteration 5, closed out

Status: done. All six "candidates for a next round" items. Tests: the full
AGENTS.md suite (see Verification below), plus the new
`one_channel_group_walk_serves_every_channel_of_the_group`,
`a_definition_whose_operands_are_still_decoding_waits_instead_of_failing`,
`a_failed_operand_names_the_channel_and_is_cached_as_an_error`, and the five
tests in `gui/tests/curve_decimation.rs`.

1. **Computed channels decode their operands off the UI thread**
   (`gui/src/computed.rs`, `gui/src/panels/plot.rs`) — `evaluate_visible_defs`
   no longer decodes anything. It takes a map of
   `OperandStatus::Ready/Failed` per channel location; a definition whose
   operands have not landed comes back `ComputedState::Waiting` naming them,
   uncached, and is drawn as a spinner row instead of a line. The plot panel
   keeps operands in `computed_operand_slots` (the same three-state slots
   plotted channels use, spawned through `spawn_signal_load`) and moves
   landed decodes into `computed_operands`, which the fingerprints in
   `CachedComputed` invalidate against as before. A failed operand decode is
   terminal (the file does not change while open), so it reaches the
   definition once as a cached error naming the channel. The interactive
   states are unit-pinned; eyes on a real window remain desirable.
2. **`SignalSeries::timestamps` is `Arc<Vec<f64>>`** (`src/time_ops.rs`) —
   the breaking field change the last round deferred. `series_for` builds
   one axis per channel group and hands every series a clone of one
   allocation; `stack` shifts the axis once per group and re-shares it;
   `new` takes `impl Into<Arc<Vec<f64>>>` so `Vec` callers compile
   unchanged, and `timestamps()` still returns `&[f64]`. Version bumped
   0.4.0 → 0.5.0 with a CHANGELOG entry.
3. **X-Y/GPS point-count decimation** (`gui/src/decimate.rs::decimate_curve`,
   `panels/xy.rs`, `panels/gps.rs`) — a curve's x is not sorted, so the time
   plot's min-then-max would reorder it and invent crossings. Each pixel
   column keeps its first, lowest, highest and last point *in the curve's
   own traversal order* (emitted sorted by visit index, deduplicated):
   spikes and reversals survive, only repetition inside a column is dropped.
   Both panels cache the pairing and re-decimate only when the view moves or
   resizes — the GPS panel had no cache at all and re-paired plus cloned its
   points every frame. Cursor lookups still answer from the undecimated
   curve, and the "N points" caption still reports the pairing's count.
4. **The open-time record walk seeds `raw_stream_cache`** (`src/file.rs`) —
   `index_records` assembles the stream it already had to read instead of
   dropping it, and `with_seeded_streams` inserts the seeds after
   construction, making the first `signal()` of an unsorted group a cache
   hit. The tradeoff the last round flagged is made explicit and bounded:
   a group is only kept when its stream fits `max_alloc`, and cumulative
   seeding stops at one budget's worth, so a file of many unsorted groups
   holds at most one budget from open — the same bound the cache already
   enforces after reads. Over budget, the walk degrades to the old
   per-block scratch behaviour. A/B on the J1939 log via `max_alloc = 1`
   (one-off temporary example, since deleted): total open+read 12.5–13.7 ms
   both ways — on an uncompressed log the saved pass is a single mmap copy.
   The win case is a DZ-compressed unsorted group under the budget; the
   corpus has none (its unsorted logs are CANedge-written uncompressed, its
   DZ files are sorted), so this is structural, benchmark-neutral.
5. **MDF 3 channel reads decode their channel group once**
   (`src/mdf3/records.rs`, `src/mdf3/mod.rs`) — `read_channel_group` decodes
   every channel of a channel group inside a single walk of the data
   group's records, and `Mdf3File::channel_values` serves channels from a
   `BoundedLru` keyed `(data group, channel group)` (the v4 reader's own
   LRU, now `pub(crate)`). The requested channel's layout is validated
   before any records are read, so a mis-placed channel is refused by name
   with exactly the old error even when the block is also truncated; a
   *sibling's* bad layout decodes to `None` and never blocks the group.
   Pinned by a counting source: 15 channels of the synthetic group cost one
   read of the record stream, and a repeat read is served from the cache.
6. **Arrow export writes nulls without a `Vec<Option<T>>` per column**
   (`src/export/arrow.rs`) — values go straight into the array buffer
   (`from_iter_values`), or alongside a bit-wise `NullBufferBuilder`
   (`from_iter_values_with_nulls`) when the channel carries invalidation
   bits. A null slot holds the type's default — what `From<Vec<Option<_>>>`
   wrote before, a detail my first (dense-values) attempt got wrong and
   the existing export test caught.

### Measured effect

The tracked harness was re-run after all six changes (corpus 78 files × 5
runs `--select`, large fixtures × 3 `--tag large`); `COMPARISON.md` was
refreshed by hand and the sync check printed `PASS`. Against the
iteration-5 numbers:

| Workload | vs `select()`, iteration 5 | vs `select()`, now |
|---|---|---|
| Corpus > 1 MB (10 files) | 3.3× | 3.2× |
| 480 MB uncompressed | 1.5× | 1.5× |
| 122 MB transposed-deflate | 1.1× | 1.1× |

All three moved within run-to-run noise; nothing regressed. Peak RSS is
unchanged where the previous round quoted it: 1371.5 MB (deflate) and
1672.0 MB (uncompressed) — the seeding budget keeps open-time retention at
or below what the first read would have held anyway — and the J1939 log
improved from 29.9 to 28.2 MB. The MDF3, GUI and Arrow changes are not
exercised by this harness (no v3 files in the corpus; GUI costs are
per-frame; Arrow is feature-gated export); their effect is structural and
pinned by the tests named above.

## Verification

The full AGENTS.md suite was run after all six items:

- `cargo fmt --all --check` — clean
- `cargo clippy --all-targets -- -D warnings` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo clippy -p falcon_mdf_gui --all-targets -- -D warnings` — clean on
  this toolchain except the pre-existing `chunks_exact_to_as_chunks` allow,
  which local clippy 1.97 does not know and CI's does (checked with
  `-A unknown-lints`; the skew predates this round)
- `cargo test --all-targets --all-features` — 63 test binaries, 777 passed,
  0 failed (corpus present, reference tests ran)
- `cargo test --doc --all-features` — 35 passed
- `cargo test -p falcon_mdf_gui` — 25 test binaries, 249 passed, 0 failed
- `.venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py`
  — `PASS`

## Not done, and why (candidates for a next round)

1. **Interactive verification of the computed-channel waiting state** — the
   state machine is unit-pinned (waiting is not cached, not an error; a
   landed operand evaluates on the next frame), but nobody has watched a
   large operand decode behind a spinner in a live window. Needs a human.
2. **MDF3 cross-group redundancy** — channels of *different* channel groups
   in one unsorted v3 data group still walk the stream once per group. The
   per-group cache closed the K-channels-per-group case; the cross-group
   case would need a whole-data-group decode with per-CG accumulators.
3. **The criterion suite (`cargo bench`) was not re-run** — it is optional
   in the skill and the tracked harness covered the read paths; a criterion
   pass would give finer per-phase deltas for the seeding change.
4. **The Python bindings remain unmeasured** (known gap in
   `benchmarks/COMPARISON.md`): `import falcon_mdf` still fails in `.venv`,
   so the PyO3/Arrow-IPC overhead a Python user pays is still not quantified.

