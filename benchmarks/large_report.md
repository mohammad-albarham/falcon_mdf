## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Generated**: 2026-09-06T11:01:11+02:00
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git 6d5df00 + uncommitted reader optimizations
**Files tested**: 2

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 5.2× |
| Geometric mean speedup (vs `select()`) | 1.7× |
| Median speedup (vs `get()`) | 5.7× |
| Min speedup | 3.4× |
| Max speedup | 8.0× |
| Files where falcon faster | 2/2 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| > 1 MB | 2 | 5.2× | 1.7× | 1.7× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **2/2** files.

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| large_deflate.mf4 | 121.9 MB | 1.0672 | 8.5043 | 1.8830 | 8.0× | 1.8× |
| large_uncompressed.mf4 | 479.7 MB | 0.4298 | 1.4541 | 0.7374 | 3.4× | 1.7× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| large_deflate.mf4 | 1371.4 | 2340.3 | 1.7× |
| large_uncompressed.mf4 | 1672.0 | 2693.0 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **129.1 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| large_deflate.mf4 | 0.19 | 1066.99 | 852.30 | 7649.23 |
| large_uncompressed.mf4 | 0.22 | 429.62 | 290.72 | 1163.39 |
