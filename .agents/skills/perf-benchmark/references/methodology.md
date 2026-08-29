# Benchmark Methodology & Interpretation Guide

## Where Results Live

Every run of `bench_extended.py` overwrites two files in the repo's tracked
`benchmarks/` folder: `<tag>_report.md` and `<tag>_results.json` (`tag`
defaults to `latest`; the large fixtures use `--tag large`). The curated
`benchmarks/COMPARISON.md` is **not** regenerated — update it by hand after
each run, following the checklist in SKILL.md.

## What Is Being Measured

### falcon_mdf path (Rust, compiled)

The `examples/bench.rs` binary measures three phases:
1. **`open`** — `Mf4File::open(path)` — parses block structure, builds channel
   index, no samples decoded.
2. **`read_native`** — iterates all channels, calls `signal(ch).values()` — each
   channel decodes to its own type (u8, u32, f64, bytes…).
3. **`read_f64`** — iterates all channels, calls `signal(ch).values_f64()` — 
   coerces everything to f64, costs an extra pass.

The "total" used for comparison is `open + read_native`, which matches what
asammdf's per-channel `mdf.get()` does.

### asammdf path (Python, interpreted)

Two entry points are compared:

#### `mdf.get()` (per-channel)
```python
mdf = MDF(path)
for gi, grp in enumerate(mdf.groups):
    for ci, ch in enumerate(grp.channels):
        sig = mdf.get(ch.name, group=gi, index=ci, raw=False)
        _ = sig.samples
```
This is the simplest access pattern but decompresses data blocks once per
channel, which is expensive for compressed files.

#### `mdf.select()` (batch, optional)
```python
mdf = MDF(path)
selections = [(ch.name, gi, ci) for gi, grp in ...]
signals = mdf.select(selections)
```
This amortises decompression across channels and can be 20–40% faster than
`get()` on compressed files. **Always report both for fairness.**

## Measurement Protocol

1. **Warm cache**: Each file is read once before timing starts to populate
   the OS page cache.
2. **Multiple runs**: The default is 5 runs (minimum 3); the **median** is
   reported, not the mean, to reject outliers.
3. **Release build**: falcon is compiled with `opt-level=3, lto=true,
   codegen-units=1`.
4. **Wall clock**: `time.perf_counter()` (Python), `std::time::Instant` (Rust).

## Known Caveats

### 1. File size asymmetry
The test corpus is dominated by files ≤5 MB. Falcon's advantage narrows on
larger files (>100 MB) as I/O and decompression dominate CPU-bound decoding.
The AUDIT.md reports 0.81× (falcon slower) on a 126 MB compressed file.

### 2. Compression type
- **Deflate blocks written by asammdf**: falcon is 5–9× faster.
- **Deflate blocks written by vendor tools** (Vector, ETAS): falcon is at
  parity or slightly slower (0.85–1.01×).
- This may reflect differences in block layout rather than decompression speed.

### 3. Entry point matters
asammdf `select()` amortises setup and decompression. Quoting only `get()`
numbers flatters falcon; quoting only `select()` flatters asammdf.

### 4. Channel count skew
A file with many tiny channels (e.g., 200 channels of 100 samples each) may
show different ratios than a file with few channels of 100,000 samples each.

### 5. Memory measurement
Both sides are the **peak resident set size of the whole process**, taken from
`/usr/bin/time -l` (macOS) / `-v` (Linux):
- **falcon**: `target/release/examples/bench <file>`.
- **asammdf**: `bench_extended.py --rss-worker <file>`, a subprocess that opens
  the file and decodes every channel with `mdf.get()`, then exits — the same
  workload the timing loop measures.

Earlier revisions compared falcon's RSS against asammdf's `tracemalloc` peak.
That was wrong in asammdf's favour by a large margin: `tracemalloc` sees only
Python-level allocations, missing the numpy backing buffers and the whole
interpreter. Do not reintroduce it.

The report also prints the peak RSS of a bare `import asammdf`. That import
alone costs well over 100 MB, and it is a fixed runtime cost rather than a
decoding cost, so subtract it before drawing conclusions about decoder memory
efficiency.

### 6. Python startup overhead
asammdf timing includes Python's import and MDF class construction overhead.
For very small files (<10 KB), this fixed overhead dominates and produces
artificially large speedup ratios.

## How to Report Results Honestly

1. **Always state the asammdf entry point** (`get()` vs `select()`).
2. **Always state the file type** (uncompressed, deflate, vendor-compressed).
3. **Use geometric mean** for aggregate speedup across files, not arithmetic
   mean — ratios are multiplicative.
4. **Acknowledge the gap**: if you only have ≤5 MB files, say so.
5. **Include the worst case**: the file where falcon is slowest relative to
   asammdf is as important as the best case.
