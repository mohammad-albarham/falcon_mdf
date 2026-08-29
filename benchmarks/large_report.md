## Performance: falcon_mdf vs asammdf

**Machine**: macOS-26.6.2-arm64-arm-64bit-Mach-O
**Processor**: arm
**Python**: 3.14.7
**asammdf**: 8.7.2
**falcon_mdf**: git b8db8fc
**Files tested**: 2

### Summary

| Metric | Value |
|---|---|
| Geometric mean speedup (vs `get()`) | 3.7× |
| Geometric mean speedup (vs `select()`) | 1.2× |
| Median speedup (vs `get()`) | 3.8× |
| Min speedup | 3.0× |
| Max speedup | 4.6× |
| Files where falcon faster | 2/2 |

### Results by File Size

Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the
smallest files, so the aggregate over the whole corpus overstates the
decoding advantage. Quote the `> 1 MB` row.

`Files` counts only files where both libraries decoded the same
number of samples; see Sample-Count Agreement below for the rest.

| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |
|---|---|---|---|---|
| > 1 MB | 2 | 3.7× | 1.2× | 1.0× |

### Sample-Count Agreement

falcon and asammdf decoded identical sample counts on **2/2** files.

### Per-File Results

| File | Size | falcon (s) | asammdf get (s) | asammdf select (s) | Speedup (get) | Speedup (select) |
|---|---|---|---|---|---|---|
| large_deflate.mf4 | 121.9 MB | 1.8191 | 8.3580 | 1.8560 | 4.6× | 1.0× |
| large_uncompressed.mf4 | 479.7 MB | 0.4930 | 1.4573 | 0.7257 | 3.0× | 1.5× |

### Memory

| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |
|---|---|---|---|
| large_deflate.mf4 | 1371.4 | 2340.2 | 1.7× |
| large_uncompressed.mf4 | 1676.0 | 2693.0 | 1.6× |

Both columns are peak resident set size of the whole process, measured with `/usr/bin/time`.
A bare interpreter that only does `import asammdf` already peaks at **129.2 MB**; subtract that to compare decoding cost rather than runtime cost.

### Timing Breakdown

| File | falcon open (ms) | falcon decode (ms) | asammdf open (ms) | asammdf decode (ms) |
|---|---|---|---|---|
| large_deflate.mf4 | 0.19 | 1818.88 | 841.09 | 7516.90 |
| large_uncompressed.mf4 | 0.24 | 492.72 | 289.83 | 1164.75 |
