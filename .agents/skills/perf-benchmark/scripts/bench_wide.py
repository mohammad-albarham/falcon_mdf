#!/usr/bin/env python3
"""Benchmark falcon_mdf vs asammdf on wide files (tens of thousands of channels).

bench_extended.py times asammdf's per-channel `get()` on every file, which on
a 50k-channel DZ file takes about half an hour per pass: each call decompresses
its whole channel group again. Nobody reads a wide file that way, so this
script compares against `select()` and only estimates the `get()` cost from a
sample of channels per group.

falcon is timed two ways: per-channel `signal()` (examples/bench.rs) and one
batched `signals()` call (examples/bench_batch.rs).

Usage:
    .venv/bin/python .agents/skills/perf-benchmark/scripts/bench_wide.py \
        test_data/wide/wide_deflate.mf4 [--runs 5] [--tag wide]
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import statistics
import subprocess
import sys
import time
from datetime import datetime

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bench_extended import (  # noqa: E402
    MDF,
    ROOT,
    format_size,
    machine_info,
    peak_rss_of_command,
    python_baseline_rss_mb,
)


def build_examples() -> dict[str, pathlib.Path]:
    subprocess.run(
        ["cargo", "build", "--release", "--example", "bench",
         "--example", "bench_batch", "--quiet"],
        cwd=ROOT, check=True,
    )
    ex = ROOT / "target" / "release" / "examples"
    return {"bench": ex / "bench", "bench_batch": ex / "bench_batch"}


def run_falcon(binary: pathlib.Path, path: pathlib.Path, runs: int,
               keys: tuple[str, ...]) -> dict:
    subprocess.run([str(binary), str(path)], capture_output=True, check=True)
    rows = []
    for _ in range(runs):
        out = subprocess.run([str(binary), str(path)], capture_output=True,
                             text=True, check=True).stdout
        rows.append({k: float(re.search(rf"{k}=([\d.]+)", out).group(1))
                     for k in keys})
    res = {k: statistics.median(r[k] for r in rows) for k in keys}
    res["runs_total_ms"] = [r["open"] + r[keys[1]] for r in rows]
    res["total_s"] = statistics.median(res["runs_total_ms"]) / 1000.0
    res["peak_rss_mb"] = peak_rss_of_command([str(binary), str(path)])
    return res


def asammdf_select_once(path: pathlib.Path) -> tuple[float, float, int]:
    t0 = time.perf_counter()
    mdf = MDF(str(path))
    t_open = time.perf_counter() - t0
    sel = [(None, gi, ci) for gi, g in enumerate(mdf.groups)
           for ci in range(len(g.channels))]
    t1 = time.perf_counter()
    sigs = mdf.select(sel)
    n = sum(len(s.samples) for s in sigs)
    t_sel = time.perf_counter() - t1
    mdf.close()
    return t_open, t_sel, n


def run_asammdf_select(path: pathlib.Path, runs: int) -> dict:
    asammdf_select_once(path)
    rows = [asammdf_select_once(path) for _ in range(runs)]
    return {
        "open_s": statistics.median(r[0] for r in rows),
        "select_s": statistics.median(r[1] for r in rows),
        "runs_total_s": [r[0] + r[1] for r in rows],
        "total_s": statistics.median(r[0] + r[1] for r in rows),
        "samples": rows[0][2],
        "peak_rss_mb": peak_rss_of_command(
            [sys.executable, str(pathlib.Path(__file__).resolve()),
             "--rss-worker", str(path)]),
    }


def estimate_asammdf_get(path: pathlib.Path, per_group: int) -> dict:
    """Times get() on a sample of channels per group and scales to the group."""
    mdf = MDF(str(path))
    total_s, groups = 0.0, []
    for gi, g in enumerate(mdf.groups):
        idx = list(range(len(g.channels)))[1:per_group + 1]
        t = time.perf_counter()
        for ci in idx:
            mdf.get(group=gi, index=ci)
        per_ch = (time.perf_counter() - t) / max(1, len(idx))
        est = per_ch * len(g.channels)
        total_s += est
        groups.append({"group": gi, "channels": len(g.channels),
                       "ms_per_channel": per_ch * 1000, "est_s": est})
    mdf.close()
    return {"sampled_per_group": per_group, "est_total_s": total_s,
            "groups": groups}


def file_shape(path: pathlib.Path) -> dict:
    mdf = MDF(str(path))
    shape = {
        "groups": len(mdf.groups),
        "channels": sum(len(g.channels) for g in mdf.groups),
        "records_per_group": [g.channel_group.cycles_nr for g in mdf.groups],
    }
    mdf.close()
    data = path.read_bytes()
    shape["dz_blocks"] = data.count(b"##DZ")
    return shape


def mb(v: float | None) -> str:
    return f"{v:.0f} MB" if v is not None else "n/a"


def write_report(path: pathlib.Path, r: dict, machine: dict, baseline: float | None) -> str:
    s, f, fb, a = r["shape"], r["falcon"], r["falcon_batch"], r["asammdf_select"]
    g = r.get("asammdf_get_estimate")
    lines = [
        "## Wide-file performance: falcon_mdf vs asammdf",
        "",
        f"**Machine**: {machine['platform']}  ",
        f"**Generated**: {machine['generated_at']}  ",
        f"**asammdf**: {machine['asammdf_version']}  ",
        f"**falcon_mdf**: git {machine['falcon_git_hash']}  ",
        f"**Runs**: median of {r['runs']} after one warm-up",
        "",
        "### File",
        "",
        "| File | Size | Channel groups | Channels | DZ blocks | Samples |",
        "|---|---|---|---|---|---|",
        f"| {path.name} | {format_size(path.stat().st_size)} | {s['groups']} | "
        f"{s['channels']:,} | {s['dz_blocks']} | {int(fb['samples']):,} |",
        "",
        "Written by asammdf (transposed deflate), not by falcon's own writer, "
        "so the block layout does not favour falcon's reader.",
        "",
        "### Open + read every channel",
        "",
        "| Reader | Open | Read | Total | vs falcon batched | Peak RSS |",
        "|---|---|---|---|---|---|",
        f"| falcon `signals()` batched | {fb['open']:.1f} ms | "
        f"{fb['read_batch'] / 1000:.2f} s | **{fb['total_s']:.2f} s** | 1.00× | "
        f"{mb(fb['peak_rss_mb'])} |",
        f"| falcon `signal()` per channel | {f['open']:.1f} ms | "
        f"{f['read_native'] / 1000:.2f} s | {f['total_s']:.2f} s | "
        f"{f['total_s'] / fb['total_s']:.2f}× | {mb(f['peak_rss_mb'])} |",
        f"| asammdf `select()` | {a['open_s'] * 1000:.1f} ms | "
        f"{a['select_s']:.2f} s | **{a['total_s']:.2f} s** | "
        f"{a['total_s'] / fb['total_s']:.2f}× | {mb(a['peak_rss_mb'])} |",
    ]
    if g:
        lines.append(
            f"| asammdf `get()` per channel (estimated) | — | — | "
            f"~{g['est_total_s'] / 60:.0f} min | ~{g['est_total_s'] / fb['total_s']:.0f}× | — |")
    lines += [
        "",
        f"Sample counts agree: falcon {int(fb['samples']):,}, asammdf "
        f"{a['samples']:,} ({'match' if int(fb['samples']) == a['samples'] else 'MISMATCH'}).",
        "",
        "`vs falcon batched` is that reader's time divided by falcon's: above "
        "1.00× means falcon is faster.",
    ]
    if baseline:
        lines.append(f"Peak RSS is the whole process. A bare interpreter that "
                     f"imports asammdf already peaks at **{baseline:.0f} MB**.")
    if g:
        lines += [
            "",
            f"The `get()` row is extrapolated from {g['sampled_per_group']} "
            "channels per group: each `get()` decompresses its whole group "
            "again, so the cost grows with channels × group size.",
            "",
            "| Group | Channels | ms per `get()` | Est. group total |",
            "|---|---|---|---|",
        ]
        for row in g["groups"]:
            lines.append(f"| {row['group']} | {row['channels']:,} | "
                         f"{row['ms_per_channel']:.1f} | {row['est_s']:.0f} s |")
    return "\n".join(lines) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("file", nargs="?")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--get-sample", type=int, default=20,
                    help="Channels per group to time with get() (0 skips)")
    ap.add_argument("--out-dir", default="benchmarks")
    ap.add_argument("--tag", default="wide")
    ap.add_argument("--rss-worker", default=None, help=argparse.SUPPRESS)
    args = ap.parse_args()

    if args.rss_worker:
        asammdf_select_once(pathlib.Path(args.rss_worker))
        return

    path = pathlib.Path(args.file).resolve()
    bins = build_examples()
    machine = machine_info()
    machine["generated_at"] = datetime.now().astimezone().isoformat(timespec="seconds")

    print("shape...", file=sys.stderr)
    result = {"file": str(path.relative_to(ROOT)), "size": path.stat().st_size,
              "runs": args.runs, "shape": file_shape(path)}
    print("falcon per-channel...", file=sys.stderr)
    result["falcon"] = run_falcon(bins["bench"], path, args.runs,
                                  ("open", "read_native", "samples"))
    print("falcon batched...", file=sys.stderr)
    result["falcon_batch"] = run_falcon(bins["bench_batch"], path, args.runs,
                                        ("open", "read_batch", "samples"))
    print("asammdf select...", file=sys.stderr)
    result["asammdf_select"] = run_asammdf_select(path, args.runs)
    if args.get_sample > 0:
        print("asammdf get() sample...", file=sys.stderr)
        result["asammdf_get_estimate"] = estimate_asammdf_get(path, args.get_sample)

    baseline = python_baseline_rss_mb()
    out_dir = ROOT / args.out_dir
    (out_dir / f"{args.tag}_results.json").write_text(json.dumps(
        {"machine": machine, "python_baseline_rss_mb": baseline, "result": result},
        indent=2))
    report = write_report(path, result, machine, baseline)
    (out_dir / f"{args.tag}_report.md").write_text(report)
    print(report)


if __name__ == "__main__":
    main()
