# falcon_mdf performance comparison

- **Last run:** 2026-09-27
- **falcon_mdf:** git `4902572`
- **asammdf:** 8.7.2 on CPython 3.14.7
- **Machine:** macOS-27.0-arm64-arm-64bit-Mach-O
- **Corpus:** 87 files (the 81 of the previous run plus six written by
  `ihedvall/mdflib`); warm-up plus median of three measured runs per file
- **Build:** release, LTO, one codegen unit, optimization level 3

[HTML review](performance-review.html) includes changes, verification results,
all paired measurements and limitations. `latest_report.md` / `latest_results.json`
are the generated full-corpus report. `large_report.md` / `large_results.json`
contain the large-fixture subset of **the same measurements**, avoiding a
redundant benchmark pass. Both fixtures are asammdf-written repetitions of
J1939 logs: 121.9 MiB uses transposed DZ deflate; 479.7 MiB is uncompressed.

## What changed since the 2026-09-06 run

- **The sample counts were wrong, not the decoders.** `examples/bench.rs`
  reported `max(len(), values_f64().len())`, and `values_f64()` flattens array
  channels, so four files with array channels counted elements as samples and
  were excluded as "disagreements". Counted as samples (566aca5) they match
  asammdf exactly, and all four rejoin the equal-work tables. Two of them are
  the ~1 MiB dSPACE HIL files, which lifts the `> 1 MB` bucket from 3.6×
  (10 files) to 4.5× (12 files): a change in which files count, not in speed.
  Every file measured both times moved by a few percent at most.
- **The large fixtures read the same:** 1.79× and 1.78× against `select()`
  (1.76× and 1.72× before).
- **falcon's peak memory on `large_deflate.mf4` reads 1,802.5 MiB, up from
  1,371.4 MiB, and no code change caused it.** `bench` built at `2f17e70`,
  the commit the old figure was measured at, peaks at the same 1,802.5 MiB on
  this machine today, identically across four runs. The machine moved from
  macOS 26.6.2 to 27.0 between the runs; the old figure is not reproducible
  here and should not be quoted.

## The 2026-09-06 perf round, paired before/after

The following is an explicitly separate paired-binary experiment from
`scripts/bench_before_after.py`, recorded in `before_after_results.json`.
One warm-up and five measured runs per binary per file, alternating order;
median **open + native read** in milliseconds. Baseline corpus measurements
are preserved under `baseline/`; raw paired runs include binary SHA-256 hashes.

| Fixture | MiB | Before, ms | After, ms | Speedup |
|---|---|---|---|---|
| large_deflate.mf4 | 121.9 | 1732.57 | 1078.39 | 1.61× |
| large_uncompressed.mf4 | 479.7 | 470.23 | 411.01 | 1.14× |

The changes select flate2's zlib-rs backend, read compressed slices without an
extra input buffer, tile untransposition for cache locality, and load standard
numeric widths directly. `Signal::mean()` also propagates read errors instead
of substituting zero. Repeated timing loops were removed from integration tests
and replaced with correctness assertions. Independent transpose tests cover
cache boundaries and retain external writer read-back.

Sample counts agree between the two Rust binaries on all 81 files. Improvements
are workload-dependent: the four real 5 MiB J1939 logs improve roughly 3–5%,
while `00000013-64BB9AA0.MF4` and `00000014-64BBA8AF.MF4` take roughly 3–5%
longer. Tiny-file timings are quantized to 0.01 ms and should not be used for
headline claims. The standalone after run below was measured at a different
time from the paired experiment, so absolute medians can differ.

## Equal-work size buckets versus asammdf

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 65 | 60.7× | 60.4× | 9.6× |
| 100 KB – 1 MB | 7 | 14.4× | 12.8× | 5.3× |
| > 1 MB | 12 | 6.5× | 4.5× | 1.8× |


## Sample-count agreement versus asammdf

falcon and asammdf decoded identical sample counts on **84/87** files.

These files are excluded from the equal-work aggregates above, because a ratio between different amounts of work is not a speedup:

| File | Size | falcon samples | asammdf samples | Why |
|---|---|---|---|---|
| Vector_MeasurementArrays.mf4 | 12.2 KB | 82 | 78 | asammdf's `get()` raises on `KF4` ("array-shape mismatch"); falcon's `KF4` values are pinned against hand-read bytes in `tests/reference.rs` |
| mdflib_mixed_compressed.mf4 | 119.4 KB | 35,000 | 34,285 | asammdf's `get()` drops the 715 samples marked invalid (`i % 7 == 0`); falcon returns them with a validity mask |
| mdflib_mixed.mf4 | 353.1 KB | 35,000 | 34,285 | the same file, uncompressed |

Neither reason is a decoding disagreement. The four array files the previous
run listed here were a counting error in the harness (see above).

## Whole-read results above 1 MiB

Seconds, three measured runs. Rows with unequal counts are excluded from
size-bucket aggregates. Compare with `select()`, which batches channel reads.

| File | MiB | falcon | get() | select() | vs select() |
|---|---|---|---|---|---|
| dSPACE_HILAPITimeout.mf4 | 1.01 | 0.00039 | 0.00510 | 0.00491 | 12.59× |
| dSPACE_HILAPITrigger.mf4 | 1.01 | 0.00036 | 0.00492 | 0.00501 | 13.93× |
| 00000002.MF4 | 1.03 | 0.00195 | 0.01055 | 0.00816 | 4.18× |
| ASAP2_Demo_V171.mf4 | 1.15 | 0.00254 | 0.01111 | 0.00948 | 3.73× |
| 00000013-64BB9AA0.MF4 | 1.66 | 0.00564 | 0.03259 | 0.02589 | 4.59× |
| 00000014-64BBA8AF.MF4 | 2.15 | 0.00727 | 0.03880 | 0.03060 | 4.21× |
| 00002081.MF4 | 5.00 | 0.00902 | 0.05667 | 0.04053 | 4.49× |
| 00002082.MF4 | 5.00 | 0.00914 | 0.05776 | 0.04072 | 4.46× |
| 00002083.MF4 | 5.00 | 0.00918 | 0.05681 | 0.04015 | 4.37× |
| 00002084.MF4 | 5.00 | 0.00900 | 0.05608 | 0.04081 | 4.53× |
| large_deflate.mf4 | 121.88 | 1.03163 | 8.33963 | 1.84666 | 1.79× |
| large_uncompressed.mf4 | 479.68 | 0.42228 | 1.40742 | 0.75322 | 1.78× |

## Peak process memory

MiB, measured independently with `/usr/bin/time -l`.

| Fixture | falcon | asammdf get() |
|---|---|---|
| large_deflate.mf4 | 1802.5 | 2365.0 |
| large_uncompressed.mf4 | 1672.4 | 2717.6 |

Whole-process RSS includes runtime costs. The Rust timing binary subsequently
performs an f64 read; the asammdf worker uses get(). These are process peaks,
not isolated decoder allocations or select() memory. The generated report
includes the bare asammdf import baseline; do not attribute that runtime cost
to decoding. Baseline/after Rust RSS values are also shown in the HTML review.

## README consistency and limits

README now links to this run and marks its earlier table as historical. The
old assertion that both decoders use the same inflate implementation no longer
applies: the baseline used miniz_oxide and this build uses zlib-rs. The new
compressed-file results improve on the old near-parity measurements; they do
not establish the same advantage on every vendor recording.

This was a focused reader review, not an exhaustive audit. Measurements cover
warm-cache Rust whole reads, not cold I/O, GUI frame times, Python bindings or
dataframe export. Large fixtures repeat a small set of logs and do not provide
large-file structural variety. The sample-count discrepancies the previous run
left open are resolved: four were the harness counting array elements, and the
three that remain are explained in the table above. The sample count is a coarse
work check; independent conformance and regression tests provide value checks.
