#!/usr/bin/env python3
"""Extended benchmarks: falcon_mdf vs asammdf on MF4 files.

Goes beyond scripts/bench_vs_asammdf.py by adding:
  - asammdf `mdf.select()` comparison (amortises decompression)
  - Per-phase timing: open, decode-native, decode-f64
  - Memory measurement via resource.getrusage (peak RSS)
  - Machine-readable JSON output
  - Markdown report generation (written to benchmarks/ and stdout)
  - Geometric mean speedup across all files

Usage:
    .venv/bin/python .agents/skills/perf-benchmark/scripts/bench_extended.py [OPTIONS]

Options:
    --data-dir DIR      Directory containing .mf4 files (default: test_data)
    --limit N           Max files to benchmark (default: 0 = all)
    --runs N            Benchmark runs per file (default: 5)
    --out-dir DIR       Artifact directory (default: benchmarks)
    --tag NAME          Artifact prefix: <tag>_report.md, <tag>_results.json
                        (default: latest)
    --no-memory         Skip memory measurement (faster)
    --select            Include asammdf mdf.select() comparison
    --verbose           Print progress to stderr
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
from datetime import datetime
import math
import os
import os.path
import pathlib
import platform
import re
import resource
import statistics
import subprocess
import sys
import time

def _find_root() -> pathlib.Path:
    """Finds the project root via git, falling back to ancestor traversal."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True,
            cwd=pathlib.Path(__file__).resolve().parent,
        )
        if result.returncode == 0:
            return pathlib.Path(result.stdout.strip())
    except Exception:
        pass
    # Fallback: script is at .agents/skills/perf-benchmark/scripts/
    return pathlib.Path(__file__).resolve().parent.parent.parent.parent.parent

ROOT = _find_root()

# ── asammdf import ──────────────────────────────────────────────────────────
try:
    from asammdf import MDF
except (ImportError, ModuleNotFoundError) as exc:
    print(f"skipped: asammdf is not installed ({exc})")
    sys.exit(0)


# ── Helpers ─────────────────────────────────────────────────────────────────

def format_size(num_bytes: int) -> str:
    if num_bytes < 1024:
        return f"{num_bytes} B"
    elif num_bytes < 1024 * 1024:
        return f"{num_bytes / 1024:.1f} KB"
    elif num_bytes < 1024 * 1024 * 1024:
        return f"{num_bytes / (1024 * 1024):.1f} MB"
    else:
        return f"{num_bytes / (1024 * 1024 * 1024):.1f} GB"


def peak_rss_mb() -> float:
    """Returns current peak resident set size in MB."""
    # On macOS, ru_maxrss is in bytes; on Linux it's in KB
    raw = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if platform.system() == "Darwin":
        return raw / (1024 * 1024)
    return raw / 1024


def peak_rss_of_command(cmd: list[str]) -> float | None:
    """Peak RSS in MB of a whole subprocess, via /usr/bin/time.

    Used for both falcon and asammdf so the two numbers are comparable:
    each is the peak resident set of an entire process that opens one file
    and decodes every channel.
    """
    try:
        if platform.system() == "Darwin":
            res = subprocess.run(["/usr/bin/time", "-l", *cmd],
                                 capture_output=True, text=True)
            m = re.search(r"(\d+)\s+maximum resident set size", res.stderr)
            if m:
                return int(m.group(1)) / (1024 * 1024)
        else:
            res = subprocess.run(["/usr/bin/time", "-v", *cmd],
                                 capture_output=True, text=True)
            m = re.search(r"Maximum resident set size.*?:\s*(\d+)", res.stderr)
            if m:
                return int(m.group(1)) / 1024
    except Exception:
        pass
    return None


def python_baseline_rss_mb() -> float | None:
    """Peak RSS of a bare interpreter that only imports asammdf.

    The asammdf figures include this; falcon's do not. Reporting it lets a
    reader subtract the fixed interpreter cost instead of guessing at it.
    """
    return peak_rss_of_command([sys.executable, "-c", "import asammdf"])


def machine_info() -> dict:
    """Collects machine metadata for reproducibility."""
    uname = platform.uname()
    info = {
        "system": uname.system,
        "machine": uname.machine,
        "processor": platform.processor() or uname.machine,
        "python_version": platform.python_version(),
        "platform": platform.platform(),
    }
    try:
        from asammdf import __version__ as asammdf_version
        info["asammdf_version"] = asammdf_version
    except Exception:
        info["asammdf_version"] = "unknown"

    # Get falcon_mdf git hash
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, cwd=ROOT,
        )
        info["falcon_git_hash"] = result.stdout.strip()
    except Exception:
        info["falcon_git_hash"] = "unknown"

    return info


# ── Bench binary ────────────────────────────────────────────────────────────

def ensure_bench_binary() -> pathlib.Path:
    bench_bin = ROOT / "target" / "release" / "examples" / "bench"
    bench_src = ROOT / "examples" / "bench.rs"
    if not bench_bin.exists() or (
        bench_src.exists() and bench_src.stat().st_mtime > bench_bin.stat().st_mtime
    ):
        print("Building falcon bench binary...", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release", "--example", "bench", "--quiet"],
            cwd=ROOT, check=True,
        )
    return bench_bin


# ── Falcon benchmarks ──────────────────────────────────────────────────────

def parse_bench_output(output: str) -> dict:
    """Parses bench.rs output into a dict of millisecond timings."""
    result = {}
    for key in ("open", "read_native", "read_f64", "samples"):
        match = re.search(rf"{key}=([\d.]+)", output)
        if match:
            result[key] = float(match.group(1))
    return result


def bench_falcon(bench_bin: pathlib.Path, path: pathlib.Path, runs: int,
                 measure_memory: bool = False) -> dict:
    """Benchmark falcon_mdf, returning timing dict with median values."""
    # Warmup
    subprocess.run([str(bench_bin), str(path)], capture_output=True, check=True)

    all_runs = []
    for _ in range(runs):
        res = subprocess.run(
            [str(bench_bin), str(path)],
            capture_output=True, text=True, check=True,
        )
        parsed = parse_bench_output(res.stdout.strip())
        all_runs.append(parsed)

    result = {}
    for key in ("open", "read_native", "read_f64", "samples"):
        values = [r.get(key, 0) for r in all_runs]
        result[key] = statistics.median(values)

    # open and read_native are in ms; convert to seconds
    result["open_s"] = result["open"] / 1000.0
    result["read_native_s"] = result["read_native"] / 1000.0
    result["read_f64_s"] = result["read_f64"] / 1000.0
    result["total_s"] = result["open_s"] + result["read_native_s"]

    # Memory: run once and measure peak RSS of the whole subprocess
    if measure_memory:
        rss = peak_rss_of_command([str(bench_bin), str(path)])
        if rss is not None:
            result["peak_rss_mb"] = rss

    return result


# ── asammdf benchmarks ──────────────────────────────────────────────────────

def asammdf_read_all(path: pathlib.Path) -> tuple[float, float, int]:
    """Open a file and decode every channel with per-channel mdf.get().

    Shared by the timing loop and the --rss-worker subprocess so both measure
    the identical workload.
    """
    t0 = time.perf_counter()
    mdf = MDF(str(path))
    t_open = time.perf_counter() - t0

    t1 = time.perf_counter()
    total_samples = 0
    for gi, grp in enumerate(mdf.groups):
        for ci, ch in enumerate(grp.channels):
            try:
                sig = mdf.get(ch.name, group=gi, index=ci, raw=False)
                total_samples += len(sig.samples)
            except Exception:
                pass
    t_decode = time.perf_counter() - t1
    return t_open, t_decode, total_samples


def bench_asammdf_get(path: pathlib.Path, runs: int,
                      measure_memory: bool = False) -> dict:
    """Benchmark asammdf using per-channel mdf.get(), returning timing dict."""
    def _run_once() -> tuple[float, float, int]:
        return asammdf_read_all(path)

    # Warmup
    _run_once()

    opens, decodes, samples = [], [], 0
    for _ in range(runs):
        t_open, t_decode, n = _run_once()
        opens.append(t_open)
        decodes.append(t_decode)
        samples = max(samples, n)

    result = {
        "open_s": statistics.median(opens),
        "decode_s": statistics.median(decodes),
        "total_s": statistics.median([o + d for o, d in zip(opens, decodes)]),
        "samples": samples,
    }

    if measure_memory:
        rss = peak_rss_of_command(
            [sys.executable, str(pathlib.Path(__file__).resolve()),
             "--rss-worker", str(path)]
        )
        if rss is not None:
            result["peak_rss_mb"] = rss

    return result


def bench_asammdf_select(path: pathlib.Path, runs: int) -> dict | None:
    """Benchmark asammdf using mdf.select() which amortises decompression."""
    try:
        mdf = MDF(str(path))
        # Collect all (channel_name, group_index, channel_index) tuples
        selections = []
        for gi, grp in enumerate(mdf.groups):
            for ci, ch in enumerate(grp.channels):
                selections.append((ch.name, gi, ci))
        if not selections:
            return None
    except Exception:
        return None

    def _run_once() -> float:
        t0 = time.perf_counter()
        mdf_inner = MDF(str(path))
        try:
            signals = mdf_inner.select(selections)
            total = sum(len(s.samples) for s in signals)
        except Exception:
            # select() can fail on some files; fall back gracefully
            return float("inf")
        return time.perf_counter() - t0

    # Warmup
    t = _run_once()
    if t == float("inf"):
        return None

    times = [_run_once() for _ in range(runs)]
    times = [t for t in times if t != float("inf")]
    if not times:
        return None

    return {"total_s": statistics.median(times)}


# ── File discovery ──────────────────────────────────────────────────────────

def find_mf4_files(data_dir: pathlib.Path) -> list[pathlib.Path]:
    if not data_dir.is_dir():
        return []
    files = list(data_dir.rglob("*.mf4")) + list(data_dir.rglob("*.MF4"))
    # Sort by size (smallest first) for predictable ordering
    files = sorted(set(files), key=lambda p: p.stat().st_size)
    return files


# ── Reporting ───────────────────────────────────────────────────────────────

def geometric_mean(values: list[float]) -> float:
    """Geometric mean of positive values."""
    positive = [v for v in values if v > 0]
    if not positive:
        return 0.0
    return math.exp(sum(math.log(v) for v in positive) / len(positive))


# Buckets used for the size breakdown. The corpus is dominated by files of a
# few KB, where asammdf's fixed ~5 ms `MDF()` construction cost is the whole
# measurement and falcon's time is barely above the process-spawn floor. A
# single geometric mean over all files is therefore a statement about Python
# startup, not about decoding. Reporting per bucket keeps the two apart.
SIZE_BUCKETS = [
    (0, 100 * 1024, "< 100 KB"),
    (100 * 1024, 1024 * 1024, "100 KB – 1 MB"),
    (1024 * 1024, float("inf"), "> 1 MB"),
]


def samples_agree(r: dict) -> bool:
    """True when falcon and asammdf decoded the same number of samples.

    Where they disagree the two libraries are not doing the same work (array
    channels counted flattened vs per-record, channel groups one side skips),
    so the ratio for that file is not a speedup and must not be aggregated.
    """
    f = r.get("falcon", {}).get("samples")
    a = r.get("asammdf_get", {}).get("samples")
    if f is None or a is None:
        return False
    return int(f) == int(a)


def print_sample_agreement(results: list[dict]) -> None:
    """Lists files where the two libraries decoded different sample counts."""
    checked = [r for r in results
               if r.get("falcon") and r.get("asammdf_get")]
    bad = [r for r in checked if not samples_agree(r)]
    print("### Sample-Count Agreement")
    print()
    print(f"falcon and asammdf decoded identical sample counts on "
          f"**{len(checked) - len(bad)}/{len(checked)}** files.")
    if not bad:
        print()
        return
    print()
    print("These files are excluded from the equal-work aggregates above, "
          "because a ratio between different amounts of work is not a speedup:")
    print()
    print("| File | Size | falcon samples | asammdf samples |")
    print("|---|---|---|---|")
    for r in sorted(bad, key=lambda r: r["file_size"]):
        print(f"| {r['file_name']} | {r['file_size_human']} | "
              f"{int(r['falcon']['samples']):,} | "
              f"{int(r['asammdf_get']['samples']):,} |")
    print()


def print_size_breakdown(results: list[dict], include_select: bool) -> None:
    """Prints speedups grouped by file size, and the headline number.

    The `> 1 MB` row is the one to quote: it is the only bucket where the
    measurement is dominated by decoding rather than by fixed per-call overhead.
    """
    print("### Results by File Size")
    print()
    print("Fixed overhead (asammdf's `MDF()` construction, ~5 ms) dominates the")
    print("smallest files, so the aggregate over the whole corpus overstates the")
    print("decoding advantage. Quote the `> 1 MB` row.")
    print()
    print("`Files` counts only files where both libraries decoded the same")
    print("number of samples; see Sample-Count Agreement below for the rest.")
    print()
    if include_select:
        print("| Size bucket | Files | Geo. mean vs `get()` | Geo. mean vs `select()` | Worst vs `select()` |")
        print("|---|---|---|---|---|")
    else:
        print("| Size bucket | Files | Geo. mean vs `get()` | Worst vs `get()` |")
        print("|---|---|---|---|")

    for lo, hi, label in SIZE_BUCKETS:
        rows = [
            r for r in results
            if lo <= r["file_size"] < hi
            and r.get("falcon", {}).get("total_s", 0) > 0
            and r.get("asammdf_get")
            and samples_agree(r)
        ]
        if not rows:
            continue
        gets = [r["asammdf_get"]["total_s"] / r["falcon"]["total_s"] for r in rows]
        if include_select:
            sels = [
                r["asammdf_select"]["total_s"] / r["falcon"]["total_s"]
                for r in rows if r.get("asammdf_select")
            ]
            sel_geo = f"{geometric_mean(sels):.1f}×" if sels else "N/A"
            sel_min = f"{min(sels):.1f}×" if sels else "N/A"
            print(f"| {label} | {len(rows)} | {geometric_mean(gets):.1f}× | "
                  f"{sel_geo} | {sel_min} |")
        else:
            print(f"| {label} | {len(rows)} | {geometric_mean(gets):.1f}× | "
                  f"{min(gets):.1f}× |")
    print()


def format_speedup(falcon_s: float, asammdf_s: float) -> str:
    if falcon_s <= 0:
        return "N/A"
    ratio = asammdf_s / falcon_s
    return f"{ratio:.1f}×"


def print_markdown_report(results: list[dict], machine: dict,
                          include_select: bool = False) -> None:
    """Prints a formatted Markdown report to stdout."""
    print(f"## Performance: falcon_mdf vs asammdf")
    print()
    print(f"**Machine**: {machine['platform']}")
    print(f"**Processor**: {machine['processor']}")
    print(f"**Generated**: {machine.get('generated_at', 'unknown')}")
    print(f"**Python**: {machine['python_version']}")
    print(f"**asammdf**: {machine['asammdf_version']}")
    print(f"**falcon_mdf**: git {machine['falcon_git_hash']}")
    print(f"**Files tested**: {len(results)}")
    print()

    # Summary
    speedups_get = []
    speedups_select = []
    faster_count = 0

    for r in results:
        if r.get("falcon") and r.get("asammdf_get"):
            sp = r["asammdf_get"]["total_s"] / r["falcon"]["total_s"]
            speedups_get.append(sp)
            if sp > 1.0:
                faster_count += 1
        if r.get("asammdf_select") and r.get("falcon"):
            sp = r["asammdf_select"]["total_s"] / r["falcon"]["total_s"]
            speedups_select.append(sp)

    print("### Summary")
    print()
    if speedups_get:
        print(f"| Metric | Value |")
        print(f"|---|---|")
        print(f"| Geometric mean speedup (vs `get()`) | "
              f"{geometric_mean(speedups_get):.1f}× |")
        if speedups_select:
            print(f"| Geometric mean speedup (vs `select()`) | "
                  f"{geometric_mean(speedups_select):.1f}× |")
        print(f"| Median speedup (vs `get()`) | "
              f"{statistics.median(speedups_get):.1f}× |")
        print(f"| Min speedup | {min(speedups_get):.1f}× |")
        print(f"| Max speedup | {max(speedups_get):.1f}× |")
        print(f"| Files where falcon faster | "
              f"{faster_count}/{len(speedups_get)} |")
    print()

    print_size_breakdown(results, include_select)
    print_sample_agreement(results)

    # Per-file table
    print("### Per-File Results")
    print()
    if include_select:
        print("| File | Size | falcon (s) | asammdf get (s) | "
              "asammdf select (s) | Speedup (get) | Speedup (select) |")
        print("|---|---|---|---|---|---|---|")
    else:
        print("| File | Size | falcon (s) | asammdf (s) | Speedup |")
        print("|---|---|---|---|---|")

    for r in results:
        name = r["file_name"]
        size = r["file_size_human"]
        f_total = r.get("falcon", {}).get("total_s", 0)
        a_get = r.get("asammdf_get", {}).get("total_s", 0)

        if include_select:
            a_sel = r.get("asammdf_select", {}).get("total_s")
            sp_get = format_speedup(f_total, a_get)
            sp_sel = format_speedup(f_total, a_sel) if a_sel else "N/A"
            a_sel_str = f"{a_sel:.4f}" if a_sel else "N/A"
            print(f"| {name} | {size} | {f_total:.4f} | {a_get:.4f} | "
                  f"{a_sel_str} | {sp_get} | {sp_sel} |")
        else:
            sp = format_speedup(f_total, a_get)
            print(f"| {name} | {size} | {f_total:.4f} | {a_get:.4f} | {sp} |")

    # Memory table
    has_memory = any(
        r.get("falcon", {}).get("peak_rss_mb") or
        r.get("asammdf_get", {}).get("peak_rss_mb")
        for r in results
    )
    if has_memory:
        print()
        print("### Memory")
        print()
        print("| File | falcon RSS (MB) | asammdf RSS (MB) | Ratio |")
        print("|---|---|---|---|")
        for r in results:
            f_rss = r.get("falcon", {}).get("peak_rss_mb")
            a_mem = r.get("asammdf_get", {}).get("peak_rss_mb")
            name = r["file_name"]
            f_str = f"{f_rss:.1f}" if f_rss else "—"
            a_str = f"{a_mem:.1f}" if a_mem else "—"
            if f_rss and a_mem and f_rss > 0:
                ratio = f"{a_mem / f_rss:.1f}×"
            else:
                ratio = "—"
            print(f"| {name} | {f_str} | {a_str} | {ratio} |")

        baseline = python_baseline_rss_mb()
        print()
        print("Both columns are peak resident set size of the whole process, "
              "measured with `/usr/bin/time`.")
        if baseline is not None:
            print(f"A bare interpreter that only does `import asammdf` already "
                  f"peaks at **{baseline:.1f} MB**; subtract that to compare "
                  f"decoding cost rather than runtime cost.")

    # Detailed timing breakdown
    print()
    print("### Timing Breakdown")
    print()
    print("| File | falcon open (ms) | falcon decode (ms) | "
          "asammdf open (ms) | asammdf decode (ms) |")
    print("|---|---|---|---|---|")
    for r in results:
        name = r["file_name"]
        f = r.get("falcon", {})
        a = r.get("asammdf_get", {})
        f_open = f.get("open", 0)
        f_decode = f.get("read_native", 0)
        a_open = a.get("open_s", 0) * 1000
        a_decode = a.get("decode_s", 0) * 1000
        print(f"| {name} | {f_open:.2f} | {f_decode:.2f} | "
              f"{a_open:.2f} | {a_decode:.2f} |")


# ── Main ────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Extended benchmark: falcon_mdf vs asammdf."
    )
    parser.add_argument(
        "--data-dir", default="test_data",
        help="Directory containing .mf4 files (default: test_data)",
    )
    parser.add_argument(
        "--limit", "-l", type=int, default=0,
        help="Limit number of files (default: 0 = all)",
    )
    parser.add_argument(
        "--runs", "-n", type=int, default=5,
        help="Benchmark runs per file (default: 5, minimum: 3)",
    )
    parser.add_argument(
        "--out-dir", default="benchmarks",
        help="Directory for the generated artifacts (default: benchmarks, "
             "which is tracked in git)",
    )
    parser.add_argument(
        "--tag", default="latest",
        help="Artifact name prefix: writes <tag>_report.md and "
             "<tag>_results.json (default: latest; use 'large' for the "
             "test_data/large fixtures)",
    )
    parser.add_argument(
        "--no-memory", action="store_true",
        help="Skip memory measurement",
    )
    parser.add_argument(
        "--select", action="store_true",
        help="Include asammdf mdf.select() comparison",
    )
    parser.add_argument(
        "--verbose", "-v", action="store_true",
        help="Print progress to stderr",
    )
    parser.add_argument(
        "--rss-worker", type=str, default=None,
        help=argparse.SUPPRESS,  # internal: decode one file, then exit
    )
    args = parser.parse_args()

    if args.rss_worker:
        asammdf_read_all(pathlib.Path(args.rss_worker))
        return

    data_dir = pathlib.Path(args.data_dir)
    if not data_dir.is_absolute():
        data_dir = (ROOT / data_dir).resolve()

    if not data_dir.is_dir():
        print(f"skipped: data directory not found: {args.data_dir}")
        sys.exit(0)

    mf4_files = find_mf4_files(data_dir)
    if not mf4_files:
        print(f"skipped: no .mf4 files found in {args.data_dir}")
        sys.exit(0)

    if args.limit > 0:
        mf4_files = mf4_files[:args.limit]

    runs = max(3, args.runs)
    measure_memory = not args.no_memory
    bench_bin = ensure_bench_binary()
    machine = machine_info()

    if args.verbose:
        print(f"Benchmarking {len(mf4_files)} files, {runs} runs each",
              file=sys.stderr)

    results = []
    for i, path in enumerate(mf4_files):
        file_size = path.stat().st_size
        entry = {
            "file_name": path.name,
            "file_path": str(path.relative_to(ROOT)),
            "file_size": file_size,
            "file_size_human": format_size(file_size),
        }

        if args.verbose:
            print(f"[{i+1}/{len(mf4_files)}] {path.name} ({entry['file_size_human']})...",
                  file=sys.stderr, end=" ", flush=True)

        # Falcon
        try:
            entry["falcon"] = bench_falcon(
                bench_bin, path, runs, measure_memory=measure_memory
            )
            if args.verbose:
                print(f"falcon={entry['falcon']['total_s']:.4f}s",
                      file=sys.stderr, end=" ", flush=True)
        except Exception as exc:
            entry["falcon_error"] = str(exc)
            if args.verbose:
                print(f"falcon=ERROR", file=sys.stderr, end=" ", flush=True)

        # asammdf get()
        try:
            entry["asammdf_get"] = bench_asammdf_get(
                path, runs, measure_memory=measure_memory
            )
            if args.verbose:
                print(f"asammdf={entry['asammdf_get']['total_s']:.4f}s",
                      file=sys.stderr, end=" ", flush=True)
        except Exception as exc:
            entry["asammdf_get_error"] = str(exc)
            if args.verbose:
                print(f"asammdf=ERROR", file=sys.stderr, end=" ", flush=True)

        # asammdf select() (optional)
        if args.select:
            try:
                sel_result = bench_asammdf_select(path, runs)
                if sel_result:
                    entry["asammdf_select"] = sel_result
                    if args.verbose:
                        print(f"select={sel_result['total_s']:.4f}s",
                              file=sys.stderr, end=" ", flush=True)
            except Exception as exc:
                entry["asammdf_select_error"] = str(exc)

        # Compute speedup
        if entry.get("falcon") and entry.get("asammdf_get"):
            f_t = entry["falcon"]["total_s"]
            a_t = entry["asammdf_get"]["total_s"]
            if f_t > 0:
                entry["speedup_get"] = a_t / f_t

        if entry.get("falcon") and entry.get("asammdf_select"):
            f_t = entry["falcon"]["total_s"]
            a_t = entry["asammdf_select"]["total_s"]
            if f_t > 0:
                entry["speedup_select"] = a_t / f_t

        if args.verbose:
            sp = entry.get("speedup_get")
            print(f"→ {sp:.1f}×" if sp else "", file=sys.stderr, flush=True)

        results.append(entry)

    # Artifacts. Both files are written on every run, so the tracked comparison
    # folder can never drift from the last benchmark that was actually run.
    machine["generated_at"] = datetime.now().astimezone().isoformat(
        timespec="seconds"
    )
    out_dir = pathlib.Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = (ROOT / out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    json_path = out_dir / f"{args.tag}_results.json"
    with open(json_path, "w") as f:
        json.dump({
            "machine": machine,
            "config": {
                "runs": runs,
                "data_dir": str(data_dir),
                "measure_memory": measure_memory,
                "include_select": args.select,
            },
            "results": results,
        }, f, indent=2)

    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        print_markdown_report(results, machine, include_select=args.select)
    report = buf.getvalue()

    report_path = out_dir / f"{args.tag}_report.md"
    report_path.write_text(report)

    print(report)

    def rel(path: pathlib.Path) -> str:
        return os.path.relpath(path, ROOT)

    print(f"\nWrote {rel(report_path)} and {rel(json_path)}", file=sys.stderr)
    print(f"Not regenerated: {rel(out_dir / 'COMPARISON.md')} — refresh the "
          f"curated summary, then verify with "
          f"scripts/check_comparison.py (in this skill) before calling the "
          f"run done.",
          file=sys.stderr)


if __name__ == "__main__":
    main()
