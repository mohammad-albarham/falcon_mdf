# falcon_mdf vs asammdf — performance comparison

Curated summary — the file to read. The raw numbers behind it sit next to it
in this folder: `latest_report.md` / `latest_results.json` (main corpus) and
`large_report.md` / `large_results.json` (large fixtures).

Regenerate everything with the `perf-benchmark` skill. The harness overwrites
the four raw files on every run; **this file is updated by hand afterwards**,
following the checklist in the skill's `Updating the tracked comparison`
section. That section ends with a sync check
(`scripts/check_comparison.py` in the skill) that compares this header against
the metadata stamped into the results JSONs — if you are reading this and
doubt its freshness, that check is the arbiter, and failing it, trust the raw
reports over this summary.

- **Last run:** 2026-08-29
- **falcon_mdf:** git `ba1e278` (release: `lto=true`, `codegen-units=1`, `opt-level=3`)
- **asammdf:** 8.7.2 on CPython 3.14.7
- **Machine:** macOS 26.6.2, arm64, 64 GB RAM
- **Corpus:** 78 files in `test_data/` (663 MB total). The glob now picks up
  the two generated fixtures from `test_data/large/` (122 MB and 480 MB,
  204.9 M samples each); the 76 reference files underneath them are unchanged
  (32.2 MB, 10.1 M samples). Earlier runs quoted 76 files for exactly that
  reason — the fixtures used to be a separate pass only.
- **Protocol:** warm cache, 1 warm-up + median of N runs per file (5 for the
  corpus, 3 for the separate fixture pass)

## Headline

**falcon's advantage is a function of file size and compression, and it decays
to almost nothing on large compressed files.** A single number is not
defensible.

| Workload | vs `mdf.get()` | vs `mdf.select()` |
|---|---|---|
| Corpus files > 1 MB (8 reference + 2 fixtures) | 5.0× | **3.2×** |
| 480 MB uncompressed | 2.9× | **1.5×** |
| 122 MB transposed-deflate | 4.9× | **1.1×** |

Against `select()` — the fair entry point for reading a whole file — the
advantage runs 3.2× → 1.5× → 1.1× as files grow and compression is applied.
On the 122 MB deflate fixture the two libraries are **effectively tied**.

falcon is faster on 78/78 files measured (worst case 1.1×), but "faster"
spans 71.4× and 1.1×.

## Where the advantage goes

Decompression is the equalizer. Both libraries hand DZ blocks to the same
zlib inflate, and neither can beat the other at it. The same 204.9 M samples,
read two ways:

| Fixture | falcon | `select()` | falcon's margin |
|---|---|---|---|
| 480 MB uncompressed | 0.664 s | 0.970 s | 0.306 s |
| 122 MB deflate | 2.286 s | 2.519 s | 0.233 s |

Compression adds ~1.62 s to falcon and ~1.55 s to asammdf — a shared cost
neither avoids, which dilutes a margin that was only ~0.3 s to begin with.
Note the 480 MB file reads **3.4× faster** than the 122 MB one: past a
certain point inflate, not I/O or parsing, is the whole workload.

This is the mechanism behind the README's caveat that falcon runs 0.85–1.01×
on vendor DZ files. The earlier 7-run re-verification measured **1.03×** on
this fixture; this run measures 1.1× — same conclusion, the advantage is
noise-level on deflate volume.

## The small-file numbers, and why not to quote them

On the 78-file corpus the aggregate is 27.6× vs `get()`. That number is an
artifact and should never be published. Two corrections apply, both cutting
against falcon.

**1. The corpus is mostly tiny files.** 61 of 78 are under 100 KB, ~40 are
~1.6 KB. There falcon totals ~0.0001 s — barely above the cost of spawning the
benchmark binary — while asammdf totals ~0.0050 s, essentially all of it the
fixed cost of constructing `MDF()`. That ratio measures Python startup, not
decoding, and drags the corpus mean from ~5× to ~28×.

**2. Five files aren't comparing equal work.** falcon and asammdf decode
identical sample counts on 73 of 78. On the rest they don't, so the ratio isn't
a speedup:

| File | Size | falcon samples | asammdf samples |
|---|---|---|---|
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 49 | 2 |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 205 | 20 |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 1,169 | 78 |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 50,010 | 25,005 |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 50,010 | 25,005 |

The first three are array channels: falcon counts flattened elements, asammdf
counts records. The two `HILAPI` files sit in the `> 1 MB` bucket with high
per-file ratios (9.0×, 10.3× vs `get()`) that flatter falcon; they are
excluded from the bucket aggregates by the sample-count rule above.

All five are excluded from every aggregate here. Which library is *correct*
about the counts is a correctness question this benchmark does not answer, and
is worth investigating separately.

Corpus geometric means, equal-work files only:

| Size bucket | Files | vs `get()` | vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 58 | 41.5× | 41.0× | 6.0× |
| 100 KB – 1 MB | 5 | 8.4× | 7.2× | 4.4× |
| **> 1 MB** | **10** | **5.0×** | **3.2×** | **1.1×** |

The `> 1 MB` bucket now contains the two generated fixtures — previously it
held the 8 reference files only, which is why earlier revisions quoted a
higher bucket figure (4.6×/3.5×).

## Per-file, files over 1 MB

Seconds, median of 5, whole read (open + decode all channels).

| File | Size | falcon | `get()` | `select()` | vs get | vs select |
|---|---|---|---|---|---|---|
| 00000002.MF4 | 1.0 MB | 0.0028 | 0.0152 | 0.0116 | 5.4× | 4.1× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0041 | 0.0138 | 0.0111 | 3.4× | 2.7× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0079 | 0.0475 | 0.0386 | 6.0× | 4.9× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0102 | 0.0584 | 0.0442 | 5.7× | 4.4× |
| 00002081.MF4 | 5.0 MB | 0.0136 | 0.0769 | 0.0582 | 5.7× | 4.3× |
| 00002082.MF4 | 5.0 MB | 0.0139 | 0.0788 | 0.0578 | 5.7× | 4.1× |
| 00002083.MF4 | 5.0 MB | 0.0138 | 0.0779 | 0.0577 | 5.7× | 4.2× |
| 00002084.MF4 | 5.0 MB | 0.0135 | 0.0773 | 0.0571 | 5.7× | 4.2× |
| _dSPACE_HILAPITimeout.mf4_ | 1.0 MB | 0.0006 | 0.0052 | 0.0059 | _9.0×_ | _10.2×_ |
| _dSPACE_HILAPITrigger.mf4_ | 1.0 MB | 0.0006 | 0.0059 | 0.0059 | _10.3×_ | _10.3×_ |
| **large_uncompressed.mf4** | **479.7 MB** | **0.6638** | **1.9346** | **0.9696** | **2.9×** | **1.5×** |
| **large_deflate.mf4** | **121.9 MB** | **2.2862** | **11.2727** | **2.5193** | **4.9×** | **1.1×** |

Italic rows are the unequal-work files, excluded from aggregates. Bold rows are
the generated fixtures (median of 3, from `large_report.md`; the corpus run
measures them too, at 2.2830 s / 0.6526 s).

The four 5.0 MB J1939 logs remain the most representative *real* files:
1,600,885 samples each, decoded identically by both libraries.

## Entry point matters

asammdf's `select()` amortises decompression and setup across channels;
per-channel `get()` repeats it. On the 122 MB fixture the gap between the two
asammdf entry points is enormous — 11.27 s for `get()` vs 2.52 s for
`select()`, a 4.5× difference within asammdf itself. Quoting `get()` alone
would let falcon claim 4.9× on a file where it is effectively tied. Both are
always measured; `select()` is the honest column.

## Memory

Peak resident set size of the whole process, `/usr/bin/time -l`.

| Workload | falcon | asammdf | asammdf net of import |
|---|---|---|---|
| 5.0 MB J1939 log | 28.2 MB | 184.6 MB | ~57 MB |
| 122 MB deflate | 1371.5 MB | 2341.5 MB | ~2214 MB |
| 480 MB uncompressed | 1672.0 MB | 2693.1 MB | ~2565 MB |

A bare `import asammdf` already peaks at **127.8 MB** on this corpus run
(127.7 MB in the fixture pass; a bare interpreter is a tenth of that). On
small files that import *is* the entire difference — net of it, the two
libraries use comparable memory (~28 vs ~57 MB), so the raw ratio there is
Python runtime cost, not decoder efficiency.

At scale the gap is real but modest: falcon is ~1.6–1.7× leaner. Both fully
materialise — falcon needs 1.67 GB to read a 480 MB file (3.5× the file
size), so neither is a streaming reader.

Earlier revisions compared falcon's RSS against asammdf's `tracemalloc` peak,
which made falcon look *worse* on memory. `tracemalloc` sees only Python-level
allocations and misses the numpy backing buffers and the interpreter entirely.
That comparison was wrong and has been removed.

## Consistency with the README

`README.md` publishes 3.9× decode-only and 4.8× whole-read on the OBD2 log,
and flags two rows it could not verify: vendor DZ files at 0.85–1.01× and a
126 MB file at 0.81×.

- The reference-file figures here (4.1–4.3× select on the J1939 logs, 5.0×
  bucket get) sit around the published numbers. Not overstated.
- **The DZ caveat stands**: 1.1× on a 122 MB deflate fixture (1.03× in the
  earlier 7-run verification).
- The 126 MB row is *partly* corroborated. Direction and magnitude match — the
  advantage vanishes — but this fixture does not reproduce falcon being
  outright slower (0.81×). That may need a real vendor file, or may be specific
  to a structure this fixture does not have.

No README correction is warranted; if anything its caveats are better supported
than they were.

## About the large fixtures

Generated by the perf-benchmark skill's `make_large_fixture.py` (which lives
under the gitignored `.agents/skills/perf-benchmark/scripts/`), concatenating
the four J1939 truck logs 32× **using asammdf as the writer**. That choice is deliberate: a
fixture written by falcon's own `Mf4Writer` would carry the block layout
falcon's reader is tuned for, and any speedup measured on it would be
self-favouring.

**Caveat:** 32 repetitions of four files is far more self-similar than a real
480 MB log — 19 channels, one uniform structure. These exercise size and
decompression volume, not structural variety, and they are **not** a substitute
for real vendor-written DZ files.

## Known gaps

1. **No real vendor-written DZ files.** The synthetic deflate fixture points the
   same way as the README's caveat, but only real Vector/ETAS/dSPACE output can
   close this. Not synthesizable.
2. **Warm cache only.** The 480 MB fixture is large enough that cold-cache I/O
   would matter, and it is still never exercised.
3. **The Rust binary is measured, not the Python bindings.** For a Python user
   the real substitution is falcon's PyO3 bindings vs asammdf, and those pay
   PyO3 + Arrow IPC costs this benchmark never sees. `import falcon_mdf`
   currently fails in `.venv`.
4. **`to_dataframe()` is not measured**, though it is the common real-world call.
5. **No CI guard.** These numbers are now tracked in `benchmarks/`, so a
   regression is visible in a diff, but nothing runs the benchmark
   automatically or fails a build on a slowdown.
