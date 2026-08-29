#!/usr/bin/env python3
"""Build large MF4 fixtures for benchmarking, using asammdf as the writer.

The local corpus tops out at 5 MB, which leaves the regime the README reports
falcon losing in (a 126 MB compressed file at 0.81x) completely untested.

asammdf writes these deliberately: a fixture written by falcon's own Mf4Writer
would have the block layout falcon's reader is tuned for, which would make any
resulting speedup self-favouring and worthless.

Usage:
    make_large_fixture.py --repeats 32 --out-dir test_data/large
"""

import argparse
import pathlib
import sys
import time

from asammdf import MDF

# The four J1939 truck logs: same logger, same channel layout, ~5 MB each.
SOURCE_GLOB = "mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/*.MF4"

# asammdf compression levels: 0 = none, 1 = deflate, 2 = transposed deflate.
VARIANTS = [
    ("large_uncompressed.mf4", 0),
    ("large_deflate.mf4", 2),
]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repeats", type=int, default=10,
                    help="How many times to repeat the source set (default 10)")
    ap.add_argument("--out-dir", default="test_data/large")
    ap.add_argument("--data-dir", default="test_data")
    args = ap.parse_args()

    root = pathlib.Path(__file__).resolve().parents[4]
    data_dir = root / args.data_dir
    sources = sorted(data_dir.glob(SOURCE_GLOB))
    if not sources:
        print(f"no source files matched {SOURCE_GLOB} under {data_dir}")
        sys.exit(1)

    out_dir = root / args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    inputs = [str(p) for p in sources] * args.repeats
    src_mb = sum(p.stat().st_size for p in sources) * args.repeats / 1024 / 1024
    print(f"{len(sources)} source files x {args.repeats} = {len(inputs)} inputs "
          f"({src_mb:.0f} MB of source)", file=sys.stderr)

    t0 = time.perf_counter()
    merged = MDF.concatenate(inputs, version="4.10")
    print(f"concatenated in {time.perf_counter() - t0:.1f}s", file=sys.stderr)

    for name, compression in VARIANTS:
        out = out_dir / name
        t1 = time.perf_counter()
        merged.save(out, compression=compression, overwrite=True)
        size_mb = out.stat().st_size / 1024 / 1024
        print(f"{name}: {size_mb:.1f} MB (compression={compression}) "
              f"in {time.perf_counter() - t1:.1f}s", file=sys.stderr)


if __name__ == "__main__":
    main()
