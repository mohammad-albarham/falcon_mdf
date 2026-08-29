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
- **falcon_mdf:** git `b8db8fc` (release: `lto=true`, `codegen-units=1`, `opt-level=3`)
- **asammdf:** 8.7.2 on CPython 3.14.7
- **Machine:** macOS 26.6.2, arm64, 64 GB RAM
- **Corpus:** 76 files in `test_data/` (32.2 MB, 10.1 M samples), plus two
  generated fixtures of 122 MB and 480 MB
- **Protocol:** warm cache, 1 warm-up + median of N runs per file

## Headline

**falcon's advantage is a function of file size and compression, and it decays
to nothing on large compressed files.** A single number is not defensible.

| Workload | vs `mdf.get()` | vs `mdf.select()` |
|---|---|---|
| Files 1–5 MB (76-file corpus) | 4.6× | **3.5×** |
| 480 MB uncompressed | 3.0× | **1.5×** |
| 122 MB transposed-deflate | 4.6× | **1.03×** |

Against `select()` — the fair entry point for reading a whole file — the
advantage runs 3.5× → 1.5× → 1.03× as files grow and compression is applied.
On the 122 MB deflate fixture the two libraries are **tied**.

falcon is faster on 78/78 files measured, but "faster" spans 85× and 1.03×.

## Where the advantage goes

Decompression is the equalizer. Both libraries hand DZ blocks to the same
zlib inflate, and neither can beat the other at it. The same 204.9 M samples,
read two ways:

| Fixture | falcon | `select()` | falcon's margin |
|---|---|---|---|
| 480 MB uncompressed | 0.493 s | 0.726 s | 0.233 s |
| 122 MB deflate | 1.819 s | 1.856 s | 0.037 s |

Compression adds ~1.33 s to falcon and ~1.13 s to asammdf — a shared cost
neither avoids, which dilutes a margin that was only ~0.23 s to begin with.
Note the 480 MB file reads **3.7× faster** than the 122 MB one: past a certain
point inflate, not I/O or parsing, is the whole workload.

This is the mechanism behind the README's caveat that falcon runs 0.85–1.01×
on vendor DZ files. That row is now independently reproduced: **1.03×**
(7 runs, falcon 1.814–1.835 s vs `select()` 1.853–1.904 s).

## The small-file numbers, and why not to quote them

On the 76-file corpus the aggregate is 30.8× vs `get()`. That number is an
artifact and should never be published. Two corrections apply, both cutting
against falcon.

**1. The corpus is mostly tiny files.** 61 of 76 are under 100 KB, ~40 are
~1.6 KB. There falcon totals ~0.0001 s — barely above the cost of spawning the
benchmark binary — while asammdf totals ~0.0050 s, essentially all of it the
fixed cost of constructing `MDF()`. That ratio measures Python startup, not
decoding, and drags the corpus mean from ~4× to 31×.

**2. Five files aren't comparing equal work.** falcon and asammdf decode
identical sample counts on 71 of 76. On the rest they don't, so the ratio isn't
a speedup:

| File | Size | falcon samples | asammdf samples |
|---|---|---|---|
| Vector_ArrayWithFixedAxes.MF4 | 2.2 KB | 49 | 2 |
| dSPACE_MeasurementArrays.mf4 | 6.3 KB | 205 | 20 |
| Vector_MeasurementArrays.mf4 | 12.2 KB | 1,169 | 78 |
| dSPACE_HILAPITimeout.mf4 | 1.0 MB | 50,010 | 25,005 |
| dSPACE_HILAPITrigger.mf4 | 1.0 MB | 50,010 | 25,005 |

The first three are array channels: falcon counts flattened elements, asammdf
counts records. The two `HILAPI` files matter more — they sit in the `> 1 MB`
bucket with the highest ratios in it (9.4×, 10.7×), and dropping them lowers
that bucket's `select()` figure from 4.4× to **3.5×**.

All five are excluded from every aggregate here. Which library is *correct*
about the counts is a correctness question this benchmark does not answer, and
is worth investigating separately.

Corpus geometric means, equal-work files only:

| Size bucket | Files | vs `get()` | vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| < 100 KB | 58 | 45.6× | 45.6× | 5.6× |
| 100 KB – 1 MB | 5 | 7.5× | 6.9× | 4.4× |
| **> 1 MB** | **8** | **4.6×** | **3.5×** | **2.6×** |

## Per-file, files over 1 MB

Seconds, median of 5, whole read (open + decode all channels).

| File | Size | falcon | `get()` | `select()` | vs get | vs select |
|---|---|---|---|---|---|---|
| 00000002.MF4 | 1.0 MB | 0.0029 | 0.0138 | 0.0108 | 4.8× | 3.8× |
| ASAP2_Demo_V171.mf4 | 1.2 MB | 0.0040 | 0.0132 | 0.0101 | 3.4× | 2.6× |
| 00000013-64BB9AA0.MF4 | 1.7 MB | 0.0116 | 0.0439 | 0.0377 | 3.8× | 3.3× |
| 00000014-64BBA8AF.MF4 | 2.1 MB | 0.0152 | 0.0535 | 0.0432 | 3.5× | 2.8× |
| 00002081.MF4 | 5.0 MB | 0.0138 | 0.0741 | 0.0547 | 5.4× | 4.0× |
| 00002082.MF4 | 5.0 MB | 0.0137 | 0.0766 | 0.0545 | 5.6× | 4.0× |
| 00002083.MF4 | 5.0 MB | 0.0139 | 0.0745 | 0.0536 | 5.4× | 3.9× |
| 00002084.MF4 | 5.0 MB | 0.0138 | 0.0733 | 0.0546 | 5.3× | 4.0× |
| _dSPACE_HILAPITimeout.mf4_ | 1.0 MB | 0.0005 | 0.0050 | 0.0064 | _9.4×_ | _12.1×_ |
| _dSPACE_HILAPITrigger.mf4_ | 1.0 MB | 0.0005 | 0.0057 | 0.0050 | _10.7×_ | _9.4×_ |
| **large_uncompressed.mf4** | **479.7 MB** | **0.4930** | **1.4573** | **0.7257** | **3.0×** | **1.5×** |
| **large_deflate.mf4** | **121.9 MB** | **1.8191** | **8.3580** | **1.8560** | **4.6×** | **1.03×** |

Italic rows are the unequal-work files, excluded from aggregates. Bold rows are
the generated fixtures (median of 3).

The four 5.0 MB J1939 logs remain the most representative *real* files:
1,600,885 samples each, decoded identically by both libraries.

## Entry point matters

asammdf's `select()` amortises decompression and setup across channels;
per-channel `get()` repeats it. On the 122 MB fixture the gap between the two
asammdf entry points is enormous — 8.36 s for `get()` vs 1.86 s for `select()`,
a 4.5× difference within asammdf itself. Quoting `get()` alone would let falcon
claim 4.6× on a file where it is actually tied. Both are always measured;
`select()` is the honest column.

## Memory

Peak resident set size of the whole process, `/usr/bin/time -l`.

| Workload | falcon | asammdf | asammdf net of import |
|---|---|---|---|
| 5.0 MB J1939 log | 36.2 MB | 170.3 MB | ~41 MB |
| 122 MB deflate | 1371 MB | 2340 MB | ~2211 MB |
| 480 MB uncompressed | 1676 MB | 2693 MB | ~2564 MB |

A bare `import asammdf` already peaks at **129.1 MB** (a bare interpreter is
15.4 MB). On small files that import *is* the entire difference — net of it,
the two libraries use comparable memory (~36 vs ~41 MB), so the raw 4.7× ratio
there is Python runtime cost, not decoder efficiency.

At scale the gap is real but modest: falcon is ~1.6× leaner. Both fully
materialise — falcon needs 1.68 GB to read a 480 MB file (3.5× the file size),
so neither is a streaming reader.

Earlier revisions compared falcon's RSS against asammdf's `tracemalloc` peak,
which made falcon look *worse* on memory. `tracemalloc` sees only Python-level
allocations and misses the numpy backing buffers and the interpreter entirely.
That comparison was wrong and has been removed.

## Consistency with the README

`README.md` publishes 3.9× decode-only and 4.8× whole-read on the OBD2 log,
and flags two rows it could not verify: vendor DZ files at 0.85–1.01× and a
126 MB file at 0.81×.

- The 1–5 MB figures here (3.5× select, 4.6× get) sit around the published
  numbers. Not overstated.
- **The DZ caveat is now confirmed**: 1.03× on a 122 MB deflate fixture.
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
