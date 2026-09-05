## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-08-29T20:35:46+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git ba1e278
**Files tested**: 2

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 3.8× |
| Geometric mean speedup (vs `select()`) | 1.3× |
| Median speedup (vs `get()`) | 3.9× |
| Min speedup | 2.9× |
| Max speedup | 4.9× |
| Files where falcon faster | 2/2 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| > 1 MB | 2 | 3.8× | 1.3× | 1.1× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **2/2** files.

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| large_deflate.mf4 | 121.9 MB | 2.2862 | 11.2727 | 2.5193 | 4.9× | 1.1× |
| large_uncompressed.mf4 | 479.7 MB | 0.6638 | 1.9346 | 0.9696 | 2.9× | 1.5× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| large_deflate.mf4 | 1371.5 | 2341.5 | 1.7× |
| large_uncompressed.mf4 | 1672.0 | 2693.1 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **127.7 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| large_deflate.mf4 | 0.24 | 2285.93 | 1152.57 | 10144.55 |
| large_uncompressed.mf4 | 0.27 | 663.57 | 378.10 | 1556.54 |
