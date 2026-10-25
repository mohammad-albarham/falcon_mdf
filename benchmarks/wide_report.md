## Wide-file performance: falcon_mdf vs asammdf

**Machine**: macOS-27.0.1-arm64-arm-64bit-Mach-O  
**Generated**: 2026-10-10T15:12:33+02:00  
**asammdf**: 8.7.2  
**falcon_mdf**: git e204d02  
**Runs**: median of 5 after one warm-up

### File

| File | Size | Channel groups | Channels | DZ blocks | Samples |
|---|---|---|---|---|---|
| wide_deflate.mf4 | 211.3 MB | 10 | 50,010 | 74 | 110,441,040 |

Written by asammdf (transposed deflate), not by falcon's own writer, so the block layout does not favour falcon's reader.

### Open + read every channel

| Reader | Open | Read | Total | vs falcon batched | Peak RSS |
|---|---|---|---|---|---|
| falcon `signals()` batched | 36.2 ms | 1.24 s | **1.28 s** | 1.00× | 622 MB |
| falcon `signal()` per channel | 35.6 ms | 1.21 s | 1.24 s | 0.97× | 500 MB |
| asammdf `select()` | 220.5 ms | 1.08 s | **1.30 s** | 1.02× | 1972 MB |
| asammdf `get()` per channel (estimated) | — | — | ~34 min | ~1580× | — |

Sample counts agree: falcon 110,441,040, asammdf 110,441,040 (match).

`vs falcon batched` is that reader's time divided by falcon's: above 1.00× means falcon is faster.
Peak RSS is the whole process. A bare interpreter that imports asammdf already peaks at **154 MB**.

The `get()` row is extrapolated from 20 channels per group: each `get()` decompresses its whole group again, so the cost grows with channels × group size.

| Group | Channels | ms per `get()` | Est. group total |
|---|---|---|---|
| 0 | 2,001 | 138.8 | 278 s |
| 1 | 2,001 | 140.1 | 280 s |
| 2 | 3,001 | 104.6 | 314 s |
| 3 | 3,001 | 100.3 | 301 s |
| 4 | 5,001 | 34.6 | 173 s |
| 5 | 5,001 | 34.3 | 171 s |
| 6 | 5,001 | 35.0 | 175 s |
| 7 | 5,001 | 34.8 | 174 s |
| 8 | 10,001 | 7.9 | 79 s |
| 9 | 10,001 | 8.0 | 80 s |
