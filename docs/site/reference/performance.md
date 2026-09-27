# Performance

The project measures whole-file reads against asammdf's batched `select()` API. On the [recorded September 2026 comparison](https://github.com/mohammad-albarham/falcon_mdf/blob/main/benchmarks/COMPARISON.md), the geometric mean for the 12 files over 1 MB was 4.5×, with a slowest included file at 1.8×. Those figures come from a release build, one warm-up, and the median of three measured runs on an Apple-silicon Mac; only files with equal decoded sample counts are included.

The two largest fixtures were asammdf-written repetitions of J1939 logs, so they measure size and decompression volume more than vendor structural variety. Vendor-written compressed files were last measured near parity in an earlier audit and have not been remeasured with the current inflater. See the [full comparison and raw reports](https://github.com/mohammad-albarham/falcon_mdf/tree/main/benchmarks) for file sizes, compression, memory, and sample-count exceptions.

## Memory and backends

| Backend | Use when | Constraint |
| --- | --- | --- |
| Memory mapped (default) | A completed local file stays unchanged during reading | Truncation by another process can raise `SIGBUS` |
| Buffered | A file may change, be replaced, or live on a network share | Copies bytes as it reads; memory use varies by file |

The ordinary read path assembles a data group's records before decoding them. Peak memory therefore scales with the largest data group, not simply the requested channel. `signals_chunks` reads in bounded windows for large recordings, including unsorted groups.

Do not assume that buffered I/O always uses less memory. The project's measured files show different outcomes at different sizes; the backend choice should primarily follow whether the underlying file can change.

## Reproduce the comparison

The repository holds the benchmark harness and generated summaries:

```sh
scripts/fetch_reference_files.sh
.venv/bin/python scripts/bench_vs_asammdf.py --limit 10
```

The recorded comparison used `asammdf` 8.7.2. The second command requires a Python environment with `asammdf`; a run with no corpus or no `asammdf` reports a skip, not a benchmark result.
