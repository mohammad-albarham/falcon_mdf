#!/usr/bin/env python3
"""Benchmarks falcon_mdf against asammdf decoding channels from MF4 files.

Compares full read performance (opening the file and decoding all channels into
native types) between falcon_mdf and the Python reference library asammdf.

Usage:
    /path/to/.venv/bin/python scripts/bench_vs_asammdf.py [--data-dir DIR] [--limit N] [--runs N]

Defaults to scanning `test_data/` recursively for .mf4 files with 3 runs per file (taking the
median after a warm-up run) and a default limit of 10 files. If asammdf is not installed or the
data directory contains no .mf4 files, it prints "skipped: <reason>" and exits 0 so that it never
breaks build or CI environments without dependencies.
"""

import argparse
import os
import pathlib
import re
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Check for asammdf before doing any work.
try:
    from asammdf import MDF
except (ImportError, ModuleNotFoundError) as exc:
    print(f"skipped: asammdf is not installed ({exc})")
    sys.exit(0)


def format_size(num_bytes: int) -> str:
    """Formats file size into human-readable units (matching README conventions)."""
    if num_bytes < 1024:
        return f"{num_bytes} B"
    elif num_bytes < 1024 * 1024:
        return f"{num_bytes / 1024:.1f} KB"
    elif num_bytes < 1024 * 1024 * 1024:
        return f"{num_bytes / (1024 * 1024):.1f} MB"
    else:
        return f"{num_bytes / (1024 * 1024 * 1024):.1f} GB"


def ensure_bench_binary() -> pathlib.Path:
    """Ensures the release build of the bench example binary is compiled."""
    bench_bin = ROOT / "target" / "release" / "examples" / "bench"
    bench_src = ROOT / "examples" / "bench.rs"
    if not bench_bin.exists() or (
        bench_src.exists() and bench_src.stat().st_mtime > bench_bin.stat().st_mtime
    ):
        subprocess.run(
            ["cargo", "build", "--release", "--example", "bench", "--quiet"],
            cwd=ROOT,
            check=True,
        )
    return bench_bin


def bench_falcon(bench_bin: pathlib.Path, path: pathlib.Path, runs: int) -> float:
    """Times falcon_mdf opening and decoding all channels, returning median seconds."""
    # Warmup
    subprocess.run(
        [str(bench_bin), str(path)],
        capture_output=True,
        text=True,
        check=True,
    )

    times = []
    for _ in range(runs):
        res = subprocess.run(
            [str(bench_bin), str(path)],
            capture_output=True,
            text=True,
            check=True,
        )
        out = res.stdout.strip()
        # Output format: "{path}: open={open_ms}ms read_native={native_ms}ms read_f64={f64_ms}ms samples={samples}"
        open_match = re.search(r"open=([\d.]+)ms", out)
        native_match = re.search(r"read_native=([\d.]+)ms", out)
        if not open_match or not native_match:
            raise RuntimeError(f"Unexpected output from bench binary: {out}")
        open_sec = float(open_match.group(1)) / 1000.0
        native_sec = float(native_match.group(1)) / 1000.0
        times.append(open_sec + native_sec)

    return statistics.median(times)


def bench_asammdf(path: pathlib.Path, runs: int) -> float:
    """Times asammdf opening and decoding all channels, returning median seconds."""
    def _run_once() -> float:
        t0 = time.perf_counter()
        mdf = MDF(str(path))
        for gi, grp in enumerate(mdf.groups):
            for ci, ch in enumerate(grp.channels):
                try:
                    sig = mdf.get(ch.name, group=gi, index=ci, raw=False)
                    _ = sig.samples
                except Exception:
                    pass
        return time.perf_counter() - t0

    # Warmup
    _run_once()

    times = [_run_once() for _ in range(runs)]
    return statistics.median(times)


def find_mf4_files(data_dir: pathlib.Path) -> list[pathlib.Path]:
    """Recursively finds all .mf4 and .MF4 files in the target directory."""
    if not data_dir.is_dir():
        return []
    files = list(data_dir.rglob("*.mf4")) + list(data_dir.rglob("*.MF4"))
    # Deduplicate and sort by path
    return sorted(set(files))


def main() -> None:
    parser = argparse.ArgumentParser(description="Benchmark falcon_mdf vs asammdf.")
    parser.add_argument(
        "positional_data_dir",
        nargs="?",
        default=None,
        help="Directory containing .mf4 files (default: test_data)",
    )
    parser.add_argument(
        "--data-dir",
        default=None,
        help="Directory containing .mf4 files (default: test_data)",
    )
    parser.add_argument(
        "--limit",
        "-l",
        type=int,
        default=10,
        help="Limit number of files to benchmark (default: 10, use 0 for all)",
    )
    parser.add_argument(
        "--runs",
        "-n",
        type=int,
        default=3,
        help="Number of benchmark runs per file (default: 3, minimum: 3)",
    )
    args = parser.parse_args()

    raw_dir = args.data_dir or args.positional_data_dir or "test_data"
    data_dir = pathlib.Path(raw_dir)
    if not data_dir.is_absolute():
        data_dir = (ROOT / data_dir).resolve()

    if not data_dir.is_dir():
        print(f"skipped: data directory not found: {raw_dir}")
        sys.exit(0)

    mf4_files = find_mf4_files(data_dir)
    if not mf4_files:
        print(f"skipped: no .mf4 files found in {raw_dir}")
        sys.exit(0)

    if args.limit and args.limit > 0:
        mf4_files = mf4_files[:args.limit]

    runs = max(3, args.runs)
    bench_bin = ensure_bench_binary()

    print("| File | Size | falcon (s) | asammdf (s) | Speedup |")
    print("|---|---|---|---|---|")

    for path in mf4_files:
        try:
            sz = format_size(path.stat().st_size)
            tf = bench_falcon(bench_bin, path, runs=runs)
            ta = bench_asammdf(path, runs=runs)
            speedup = ta / tf if tf > 0 else 0.0
            sp_str = f"{speedup:.1f}×" if tf > 0 else "N/A"
            print(f"| {path.name} | {sz} | {tf:.4f} | {ta:.4f} | {sp_str} |")
        except Exception as exc:
            print(f"| {path.name} | ERROR | {exc} | | |", file=sys.stderr)


if __name__ == "__main__":
    main()
