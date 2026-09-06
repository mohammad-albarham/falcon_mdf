# falcon_mdf performance comparison

- **Last run:** 2026-09-06
- **falcon_mdf:** git `6d5df00` plus the uncommitted reader optimizations described below
- **asammdf:** 8.7.2 on CPython 3.14.7
- **Machine:** macOS-26.6.2-arm64-arm-64bit-Mach-O
- **Corpus:** 81 files; warm-up plus median of three measured runs per file
- **Build:** release, LTO, one codegen unit, optimization level 3

[HTML review](performance-review.html) includes changes, verification results,
all paired measurements and limitations. `latest_report.md` / `latest_results.json`
are the generated full-corpus report. `large_report.md` / `large_results.json`
contain the large-fixture subset of **the same measurements**, avoiding a
redundant benchmark pass. Both fixtures are asammdf-written repetitions of
J1939 logs: 121.9 MiB uses transposed DZ deflate; 479.7 MiB is uncompressed.

## Improvement over this session's baseline

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
| < 100 KB | 59 | 57.4× | 56.2× | 9.8× |
| 100 KB – 1 MB | 7 | 14.2× | 12.6× | 5.8× |
| > 1 MB | 10 | 5.5× | 3.6× | 1.7× |


## Sample-count agreement versus asammdf

falcon and asammdf decoded identical sample counts on **76/81** files.

These files are excluded from the equal-work aggregates above, because a ratio between different amounts of work is not a speedup:

| File | Size | falcon samples | asammdf samples |
|---|---|---|---|
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 49 | 2 |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 205 | 20 |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 1,169 | 78 |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 50,010 | 25,005 |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 50,010 | 25,005 |


## Whole-read results above 1 MiB

Seconds, three measured runs. Rows with unequal counts are excluded from
size-bucket aggregates. Compare with `select()`, which batches channel reads.

| File | MiB | falcon | get() | select() | vs select() |
|---|---|---|---|---|---|
| dSPACE_HILAPITimeout.mf4 (unequal counts) | 1.01 | 0.00040 | 0.00537 | 0.00509 | 12.73× |
| dSPACE_HILAPITrigger.mf4 (unequal counts) | 1.01 | 0.00040 | 0.00495 | 0.00494 | 12.35× |
| 00000002.MF4 | 1.03 | 0.00205 | 0.01101 | 0.00846 | 4.13× |
| ASAP2_Demo_V171.mf4 | 1.15 | 0.00286 | 0.01252 | 0.01024 | 3.58× |
| 00000013-64BB9AA0.MF4 | 1.66 | 0.00612 | 0.03499 | 0.02791 | 4.56× |
| 00000014-64BBA8AF.MF4 | 2.15 | 0.00760 | 0.04140 | 0.03256 | 4.28× |
| 00002081.MF4 | 5.00 | 0.00941 | 0.05765 | 0.04203 | 4.47× |
| 00002082.MF4 | 5.00 | 0.00929 | 0.05889 | 0.04219 | 4.54× |
| 00002084.MF4 | 5.00 | 0.00942 | 0.05718 | 0.04330 | 4.60× |
| 00002083.MF4 | 5.00 | 0.00958 | 0.05705 | 0.04266 | 4.45× |
| large_deflate.mf4 | 121.88 | 1.06718 | 8.50426 | 1.88296 | 1.76× |
| large_uncompressed.mf4 | 479.68 | 0.42984 | 1.45411 | 0.73743 | 1.72× |

## Peak process memory

MiB, measured independently with `/usr/bin/time -l`.

| Fixture | falcon | asammdf get() |
|---|---|---|
| large_deflate.mf4 | 1371.4 | 2340.3 |
| large_uncompressed.mf4 | 1672.0 | 2693.0 |

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
large-file structural variety. The five asammdf sample-count discrepancies
remain unresolved here. The benchmark's successful sample count is a coarse
work check; independent conformance and regression tests provide value checks.
