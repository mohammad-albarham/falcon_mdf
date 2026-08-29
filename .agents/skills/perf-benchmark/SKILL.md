---
name: perf-benchmark
description: >-
  Run performance benchmarks comparing falcon_mdf against asammdf.
  Use when the user asks to benchmark, compare performance, run perf tests,
  measure speed, profile, or compare against asammdf.
  Covers: full-read timing, per-channel timing, memory usage, Python bindings,
  and Rust-native benchmarks.
---

# Performance Benchmark: falcon_mdf vs asammdf

Compare falcon_mdf (Rust MF4 reader) against asammdf (Python MDF reference
library) on the local test data corpus.

## Quick Start

Run the existing comparison script (the fastest path):

```bash
.venv/bin/python scripts/bench_vs_asammdf.py --limit 0
```

If you need a deeper, more comprehensive benchmark, use the extended script
from this skill:

```bash
.venv/bin/python .agents/skills/perf-benchmark/scripts/bench_extended.py \
    --limit 0 --runs 5 --select
```

This writes `benchmarks/latest_report.md` and `benchmarks/latest_results.json`
— the tracked results folder — on every run. Then refresh
`benchmarks/COMPARISON.md` by hand; see
[Updating the tracked comparison](#updating-the-tracked-comparison). Finish by
running the sync check — it must print `PASS` before the run counts as done:

```bash
.venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py
```

`--select` is not optional in practice: without it the report quotes only
asammdf's per-channel `mdf.get()`, which flatters falcon. See
[methodology.md](./references/methodology.md).

## Prerequisites

| Dependency | Check | Install |
|---|---|---|
| Python 3.x with venv | `.venv/bin/python --version` | Already present |
| asammdf | `.venv/bin/python -c "import asammdf"` | `.venv/bin/pip install asammdf` |
| falcon bench binary | `test -f target/release/examples/bench` | `cargo build --release --example bench` |
| Test data corpus | `ls test_data/reference/*.mf4` | `scripts/fetch_reference_files.sh` |

## What Gets Benchmarked

### 1. Full-Read Comparison (primary — uses existing script)

Opens each MF4 file and decodes **all channels to native types**, end to end.
Both libraries do the same work: parse structure → decompress → decode every
channel → materialise samples.

- **falcon_mdf**: via the `examples/bench` binary (Rust, release build).
  Output format: `{path}: open={ms}ms read_native={ms}ms read_f64={ms}ms samples={n}`
- **asammdf**: via `MDF(path)` then `mdf.get(ch.name, group=gi, index=ci)` for every channel.

### 2. Extended Benchmark (uses skill script)

The [extended script](./scripts/bench_extended.py) adds:

- **Per-channel `mdf.get()` vs `mdf.select()`** — asammdf's `select()` amortises
  setup across channels and can be significantly faster than per-channel `get()`.
  Both should be reported for fairness.
- **Memory measurement** — whole-process peak RSS via `/usr/bin/time` for both
  falcon and asammdf, plus the `import asammdf` baseline to separate runtime
  cost from decoding cost.
- **File-open only** — time to parse structure without reading samples.
- **Machine-readable JSON output** — for downstream processing.
- **Markdown report** — written to `benchmarks/` and echoed to stdout.

### 3. Rust-Native Benchmarks (criterion)

```bash
cargo bench
```

Runs criterion benchmarks from [benches/read.rs](../../benches/read.rs):
- `open` — file structure parsing
- `read_all_native` — open + decode all channels
- `decode_cached` — decode with file already open
- `multi_group_alternation` — GUI access pattern simulation

## Architecture Overview

```
                                ┌──────────────────┐
                                │  test_data/       │
                                │  ├── reference/   │  ← Vector, ETAS, ASAP2 reference files
                                │  ├── mf4-sample…  │  ← CANedge OBD2, J1939, GPS/IMU
                                │  └── generated/   │  ← synthetic test files
                                └────────┬─────────┘
                                         │
                    ┌────────────────────┼────────────────────┐
                    ▼                    ▼                    ▼
          ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
          │ bench_vs_asammdf│  │ bench_extended   │  │ cargo bench     │
          │ (basic timing)  │  │ (comprehensive)  │  │ (criterion)     │
          └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                   │                    │                    │
                   ▼                    ▼                    ▼
          Markdown table         JSON + Markdown       HTML report in
          to stdout              report files          target/criterion/
```

## Test Data Inventory

The corpus lives under `test_data/`. Key files for benchmarking:

| Path | Description | Samples | Size |
|---|---|---|---|
| `mf4-sample-data-v2.1/OBD2 (Audi A4)/…/00000002.MF4` | **Primary benchmark** — OBD2 CANedge log | 326,623 | ~5 MB |
| `mf4-sample-data-v2.1/J1939 (truck)/…/*.MF4` | J1939 heavy-duty truck logs | varies | ~2-5 MB |
| `reference/ETAS_IntegerTypes.mf4` | Multi-group integer types | 10,000/group | ~1 MB |
| `reference/Vector_DataList_Deflate.mf4` | DZ-compressed (deflate) | varies | ~200 KB |
| `reference/Vector_DataList_TransposeDeflate.mf4` | DZ transposed deflate | varies | ~200 KB |
| `reference/Vector_CANape.MF4` | CANape measurement | varies | ~1 MB |
| `reference/ASAP2_Demo_V171.mf4` | ASAP2 demo | varies | ~100 KB |

To find all `.mf4`/`.MF4` files:
```bash
find test_data -iname '*.mf4' -exec ls -lh {} \;
```

### Large fixtures (not in the corpus)

The corpus tops out at 5.0 MB, which leaves the large-file regime — the one the
README reports falcon *losing* in — untested. Generate fixtures for it with:

```bash
.venv/bin/python .agents/skills/perf-benchmark/scripts/make_large_fixture.py --repeats 32
```

This concatenates the four J1939 truck logs 32× **using asammdf as the writer**
and saves two variants into `test_data/large/` (gitignored):

| File | Size | Notes |
|---|---|---|
| `large_uncompressed.mf4` | ~480 MB | 204.9 M samples, no DZ |
| `large_deflate.mf4` | ~122 MB | transposed deflate, matches the README's 126 MB case |

asammdf must be the writer. A fixture written by falcon's own `Mf4Writer` would
carry the block layout falcon's reader is tuned for, and any speedup measured
on it would be self-favouring.

Benchmark them separately from the main corpus:

```bash
.venv/bin/python .agents/skills/perf-benchmark/scripts/bench_extended.py \
    --data-dir test_data/large --limit 0 --runs 3 --select --tag large
```

`--tag large` keeps these artifacts in `benchmarks/large_report.md` /
`benchmarks/large_results.json` instead of overwriting the main-corpus pair.

**Caveat:** these are 32 repetitions of the same four files, so the data is
far more self-similar than a real 480 MB log. They exercise size and
decompression volume, not structural variety, and they do **not** substitute
for real vendor-written DZ files.

## Interpreting Results

### Speedup Calculation

```
speedup = asammdf_time / falcon_time
```

- `> 1.0×` means falcon is faster
- `< 1.0×` means asammdf is faster
- Results vary by file characteristics:

| File Type | Expected Range | Notes |
|---|---|---|
| Uncompressed, small-medium | 3×–32× | falcon's sweet spot |
| DZ-compressed (asammdf-written) | 5×–9× vs `get()`, 3×–8× vs `select()` | depends on entry point |
| DZ-compressed (vendor-written) | 0.85×–1.01× | near parity |
| Very large files (>100 MB) | 0.8×–1.3× | may converge to parity |

### Which number to quote

**There is no single number.** falcon's advantage is a function of file size
and compression, and it decays to a tie on large compressed files:

| Workload | vs `select()` |
|---|---|
| 1–5 MB corpus files | 3.5× |
| 480 MB uncompressed | 1.5× |
| 122 MB transposed-deflate | **1.03×** |

Always quote against `select()`, never `get()` alone, and always state the file
size and whether the data is DZ-compressed. Quoting "3.5× faster" unqualified
is wrong for anything above ~5 MB.

Decompression is why: both libraries call the same zlib inflate, so on a
heavily compressed file that shared cost swamps falcon's margin. See
`benchmarks/COMPARISON.md` for the full breakdown.

60 of the corpus's 76 files are under 100 KB, and 40-odd are ~1.6 KB. On those,
falcon's total is ~0.0001 s — barely above the cost of spawning the bench
binary — while asammdf's is ~0.0050 s, essentially all of it the fixed cost of
constructing `MDF()`. The ratio there measures Python's fixed overhead, and it
drags the whole-corpus geometric mean from roughly 4× up to roughly 32×.

Measured on this corpus at git `b8db8fc` (76 files, 5 runs, warm cache),
counting only the 71 files where both libraries decode the same sample count:

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` |
|---|---|---|---|
| < 100 KB | 58 | 45.6× | 45.6× |
| 100 KB – 1 MB | 5 | 7.5× | 6.9× |
| **> 1 MB** | **8** | **4.6×** | **3.5×** |

The equal-work filter is not cosmetic: including the two `dSPACE_HILAPI*.mf4`
files, where falcon decodes 50,010 samples and asammdf 25,005, raises the
`> 1 MB` select figure from 3.5× to 4.4×.

The `> 1 MB` figure agrees with the README's published 3.9×–4.8× on the OBD2
log, which is the number to stand behind.

## Updating the tracked comparison

Results live in `benchmarks/`, which **is tracked in git** (unlike this skill).
The layout:

| File | Regenerated by | Contents |
|---|---|---|
| `benchmarks/COMPARISON.md` | **you, by hand** | curated summary — headline numbers, why the advantage decays, memory, README consistency, known gaps |
| `benchmarks/latest_report.md` | the harness | main corpus, full generated Markdown |
| `benchmarks/latest_results.json` | the harness | main corpus, machine-readable |
| `benchmarks/large_report.md` | the harness, `--tag large` | large fixtures |
| `benchmarks/large_results.json` | the harness, `--tag large` | large fixtures |

The harness overwrites its four files itself. `COMPARISON.md` is the only one
that is not automatic, so **a benchmark run is not finished until it is
updated**. After every run, work through this list:

1. **Header block** — `Last run` date, falcon git hash, asammdf + CPython
   versions, machine, corpus size. The hash and versions are at the top of the
   generated report.
2. **Headline table** — the per-workload `get()` / `select()` figures. Pull the
   corpus row from the `> 1 MB` bucket in `latest_report.md`; pull the large-file
   rows from `large_report.md`.
3. **Size-bucket table** — copy the equal-work buckets verbatim from
   `latest_report.md`'s *Results by File Size*.
4. **Sample-count agreement** — if the excluded-file list changed, update the
   table and the sentence about what dropping those files does to the aggregate.
5. **Per-file table (>1 MB)** and **Memory table** — refresh from the report's
   per-file and memory sections.
6. **Consistency with the README** — re-check the claims in
   [README.md](../../README.md#performance) against the new numbers, and say so
   either way. If they no longer agree, that is a finding, not a rounding error.
7. **Known gaps** — drop anything the run closed; add anything it opened.
8. Report the numbers that moved. If a headline figure shifted materially,
   say so explicitly rather than letting the diff carry it.
9. **Verify.** Run the sync check; the run is not done until it prints `PASS`:
   ```bash
   .venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py
   ```
   It compares this file's header (Last run date, git hash, asammdf/CPython
   versions) against the metadata stamped into the results JSONs, and exits
   non-zero on any mismatch — including when the main-corpus and large-fixture
   results were generated at different commits.

Do not quote a number in `COMPARISON.md` that is not in one of the four
generated files. If a figure comes from a one-off script (as the 7-run deflate
re-verification did), say so in the text.

### Known Caveats (from [AUDIT.md](../../AUDIT.md))

1. **Entry point matters**: asammdf's `mdf.select()` amortises decompression
   across channels and can be 20% faster than per-channel `mdf.get()`.
   Always report both.
2. **File size matters**: falcon's advantage narrows on files >100 MB as I/O
   dominates CPU.
3. **Compression type matters**: vendor DZ blocks may decompress differently
   from asammdf-written DZ blocks.
4. **Warm cache**: all measurements use warm-cache medians. First-run (cold
   cache) may differ significantly on large files.

## Step-by-Step: Running the Full Benchmark Suite

1. **Ensure test data is present**:
   ```bash
   ls test_data/reference/*.mf4 | head -5
   # If empty:
   scripts/fetch_reference_files.sh
   ```

2. **Verify asammdf is installed**:
   ```bash
   .venv/bin/python -c "from asammdf import MDF; print('asammdf OK')"
   ```

3. **Build falcon bench binary**:
   ```bash
   cargo build --release --example bench --quiet
   ```

4. **Run basic comparison** (quick, ~2 min):
   ```bash
   .venv/bin/python scripts/bench_vs_asammdf.py --limit 10
   ```

5. **Run extended comparison** (comprehensive, ~10 min):
   ```bash
   .venv/bin/python .agents/skills/perf-benchmark/scripts/bench_extended.py \
       --limit 0 --runs 5 --select
   ```
   Writes `benchmarks/latest_report.md` + `benchmarks/latest_results.json`.
   Run it with the sandbox disabled, or both memory columns come back empty.

6. **Run the large fixtures** (optional, ~5 min, needs `test_data/large/`):
   ```bash
   .venv/bin/python .agents/skills/perf-benchmark/scripts/bench_extended.py \
       --data-dir test_data/large --limit 0 --runs 3 --select --tag large
   ```

7. **Refresh `benchmarks/COMPARISON.md`** — required, see
   [Updating the tracked comparison](#updating-the-tracked-comparison). The run
   is not done until this is current **and**
   `scripts/check_comparison.py` (in this skill) prints `PASS`.

8. **Run Rust criterion benchmarks** (optional):
   ```bash
   cargo bench
   ```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `check_comparison.py` prints `FAIL` | `COMPARISON.md` header (date / git hash / versions) no longer matches the raw results JSONs — usually a run whose curated refresh was skipped | Work through [Updating the tracked comparison](#updating-the-tracked-comparison), then re-run the check |
| Both RSS columns show `—` | The Bash sandbox blocks `sysctl kern.clockrate`, so macOS `/usr/bin/time -l` exits before printing the rusage block | Run the benchmark with the sandbox disabled |
| `skipped: asammdf is not installed` | Wrong interpreter | Use `.venv/bin/python`, not `python3` |
| Every speedup is 50×–70× | The corpus is sorted smallest-first and the small files are ~1.6 KB, where asammdf's fixed `MDF()` construction cost dominates | Ignore files under ~100 KB when quoting a headline number |

## Reporting Template

When presenting results, use this structure:

```markdown
## Performance: falcon_mdf vs asammdf

**Machine**: {uname}, {cpu}, {ram}
**falcon_mdf**: git {commit_hash}
**asammdf**: {version}
**Test data**: {n_files} MF4 files from test_data/

### Summary

| Metric | falcon_mdf | asammdf | Speedup |
|---|---|---|---|
| Median full-read time | {x}s | {y}s | {z}× |
| Geometric mean speedup | — | — | {g}× |
| Files where falcon faster | — | — | {n}/{total} |

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| ... | ... | ... | ... | ... | ... | ... |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| ... | ... | ... | ... |
```

## References

- `benchmarks/COMPARISON.md` — the curated, tracked summary
- [Existing benchmark script](../../scripts/bench_vs_asammdf.py) — the simple timing comparison
- [Rust bench example](../../examples/bench.rs) — the falcon timing binary
- [Criterion benchmarks](../../benches/read.rs) — Rust-native micro-benchmarks
- [AUDIT.md](../../AUDIT.md) — independent quality audit with performance findings
- [README Performance section](../../README.md#performance) — published numbers
