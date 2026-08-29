#!/usr/bin/env python3
"""Check that benchmarks/COMPARISON.md matches the raw benchmark results.

The curated summary is the only file in benchmarks/ that the harness does not
regenerate, so it is the only one that can go stale. This script compares its
header block — last-run date, falcon git hash, asammdf and CPython versions —
against the metadata recorded in latest_results.json / large_results.json and
exits non-zero on any mismatch. Run it after refreshing COMPARISON.md; the
benchmark run is not done until it passes.

Usage:
    .venv/bin/python .agents/skills/perf-benchmark/scripts/check_comparison.py

Exits 0 when in sync, 1 when stale or malformed.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

REQUIRED = [
    "COMPARISON.md",
    "latest_report.md",
    "latest_results.json",
]
OPTIONAL_PAIR = ("large_report.md", "large_results.json")


def find_root() -> pathlib.Path:
    for parent in pathlib.Path(__file__).resolve().parents:
        if (parent / ".git").exists() and (parent / "benchmarks").is_dir():
            return parent
    raise SystemExit("error: could not locate the repo root (no .git with a "
                     "benchmarks/ directory above this script)")


def load_machine(bench: pathlib.Path, name: str) -> dict | None:
    """Returns the machine block, {} if malformed, None if the file is absent."""
    path = bench / name
    if not path.exists():
        return None
    try:
        machine = json.loads(path.read_text())["machine"]
        if not isinstance(machine, dict):
            return {}
        return machine
    except (json.JSONDecodeError, KeyError):
        return {}


def parse_header(text: str) -> dict:
    """Extracts the metadata block the checklist keeps at the top."""
    header = {}

    m = re.search(r"\*\*Last run:\*\*\s*(\d{4}-\d{2}-\d{2})", text)
    header["last_run"] = m.group(1) if m else None

    m = re.search(r"\*\*falcon_mdf:\*\*\s*git\s*`([0-9a-f]{7,40})`", text)
    header["git_hash"] = m.group(1) if m else None

    m = re.search(r"\*\*asammdf:\*\*\s*(\S+)\s+on\s+CPython\s+(\S+)", text)
    header["asammdf_version"] = m.group(1) if m else None
    header["python_version"] = m.group(2) if m else None

    return header


def main() -> int:
    root = find_root()
    bench = root / "benchmarks"

    failures: list[str] = []
    notes: list[str] = []

    for name in REQUIRED:
        if not (bench / name).exists():
            failures.append(f"missing required file benchmarks/{name}")
    has_large_json = (bench / OPTIONAL_PAIR[1]).exists()
    has_large_report = (bench / OPTIONAL_PAIR[0]).exists()
    if has_large_json != has_large_report:
        failures.append(
            f"benchmarks/{OPTIONAL_PAIR[0]} and {OPTIONAL_PAIR[1]} must "
            f"appear together (the harness writes both)"
        )
    if failures:
        for f in failures:
            print(f"FAIL  {f}")
        return 1

    header = parse_header((bench / "COMPARISON.md").read_text())

    machines: dict[str, dict] = {}
    for name in ("latest_results.json", "large_results.json"):
        machine = load_machine(bench, name)
        if machine == {}:
            failures.append(f"benchmarks/{name} is malformed")
        elif machine is not None:
            machines[name] = machine
    if failures:
        for f in failures:
            print(f"FAIL  {f}")
        return 1

    for key, label in (("git_hash", "falcon git hash"),
                       ("asammdf_version", "asammdf version"),
                       ("python_version", "CPython version")):
        if header.get(key) is None:
            failures.append(f"COMPARISON.md header is missing the {label}")
            continue
        json_key = "falcon_git_hash" if key == "git_hash" else key
        for name, machine in machines.items():
            recorded = machine.get(json_key)
            if recorded in (None, "unknown"):
                notes.append(f"{name}: no recorded {label}, skipped")
            elif recorded != header[key]:
                failures.append(
                    f"{label}: COMPARISON.md says {header[key]}, "
                    f"{name} says {recorded}"
                )

    # Date check: only meaningful when the harness stamped generated_at.
    stamps = {name: machine["generated_at"][:10]
              for name, machine in machines.items()
              if machine.get("generated_at")}
    if header.get("last_run") is None:
        failures.append("COMPARISON.md header is missing the Last run date")
    elif stamps:
        newest = max(stamps.values())
        if header["last_run"] != newest:
            failures.append(
                f"Last run: COMPARISON.md says {header['last_run']}, "
                f"raw results were generated {newest} "
                f"({', '.join(sorted(stamps))})"
            )
    else:
        notes.append("raw results carry no generated_at (pre-timestamp run); "
                     "date check skipped — re-run the harness to enable it")

    hashes = {name: machine.get("falcon_git_hash")
              for name, machine in machines.items()}
    known = {n: h for n, h in hashes.items() if h and h != "unknown"}
    if len(set(known.values())) > 1:
        failures.append(
            "raw results disagree with each other: " +
            ", ".join(f"{n} at git {h}" for n, h in sorted(known.items())) +
            " — re-run the older artifact"
        )

    for note in notes:
        print(f"note  {note}")
    if failures:
        for f in failures:
            print(f"FAIL  {f}")
        print("\nCOMPARISON.md is stale. Work through the skill's "
              "'Updating the tracked comparison' checklist, then re-check.")
        return 1

    print("PASS  benchmarks/COMPARISON.md is in sync with the raw results "
          "(git hash, versions, last-run date)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
