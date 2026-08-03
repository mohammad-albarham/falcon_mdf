"""Records what an independent reader decodes from the reference files.

Writes `tests/data/reference_golden.json`, the ground truth `tests/reference.rs`
checks against. Run it once, when the reference set changes — never as part of
the test loop, which is why the values are committed and this script is not
wired into CI. Keeping Python out of the test loop is a standing goal of this
project.

    .venv/bin/python scripts/generate_reference_golden.py

The oracle is asammdf. That is worth stating plainly, because it is evidence
rather than an authority: it decodes the same bytes with a separately written
implementation, so agreement means two independent readings coincide. Where it
is known to be wrong, the entry is recorded with a `divergence` note explaining
which reader to believe and why, and the Rust test asserts falcon's value —
not asammdf's. Every one of those was established by decoding the file's raw
bytes by hand against the standard, not by preferring one reader over the other.
"""

import json
import pathlib
import sys

import numpy as np
from asammdf import MDF

TAKE = 20
ROOT = pathlib.Path(__file__).resolve().parent.parent
FILES = ROOT / "test_data" / "reference"
OUT = ROOT / "tests" / "data" / "reference_golden.json"

# Channels where asammdf's answer is not the one to assert, with the reason and
# the value falcon should produce. Each was settled against the raw bytes.
DIVERGENCES = {
    ("Vector_FixedLengthStringUTF16_LE.mf4", "Data channel"): (
        "asammdf drops the last UTF-16 code unit; the UTF-16BE sibling holds "
        "the same text and both readers agree on it",
        None,
    ),
    ("Vector_PartialConversionLinearIdentityAlgebraic.mf4", "Data channel"): (
        "outside every declared range the standard calls for the default "
        "conversion, which falcon applies; asammdf extends the neighbouring "
        "range instead. The two agree on every sample inside a declared range",
        None,
    ),
    ("Vector_StatusStringTableConversionAlgebraic.mf4", "Data channel"): (
        "a table mixing labels with nested conversions: falcon keeps the "
        "labels and renders the computed side, asammdf returns numbers and "
        "drops the labels",
        None,
    ),
    ("Vector_CANOpenDate.mf4", "Data channel"): (
        "asammdf returns nine bytes for a seven-byte CANopen field, "
        "misaligned. Verified instead by decoding the raw records against "
        "CiA 301 — see canopen_records_from_a_vendor_file_decode_to_the_"
        "instants_they_encode",
        None,
    ),
    ("Vector_CANOpenTime.mf4", "Data channel"): (
        "as above, for the six-byte time record",
        None,
    ),
}

STRING_ENCODINGS = {6: "latin-1", 7: "utf-8", 8: "utf-16-le", 9: "utf-16-be"}


def decode_text(samples, data_type):
    """Decodes text samples with the encoding the channel declares."""
    enc = STRING_ENCODINGS.get(data_type, "latin-1")
    out = []
    for x in samples[:TAKE]:
        if isinstance(x, bytes):
            if "16" in enc and len(x) % 2:
                x = x[:-1]
            t = x.decode(enc, "replace").rstrip("\x00")
            if enc == "latin-1":
                # Conversion-produced text carries no data type of its own, so
                # latin-1 is a guess; undo it when the bytes are really UTF-8.
                try:
                    t = t.encode("latin-1").decode("utf-8")
                except (UnicodeEncodeError, UnicodeDecodeError):
                    pass
            out.append(t)
        else:
            out.append(str(x).rstrip("\x00"))
    return out


def number(x):
    """JSON cannot hold inf or NaN, and they are different answers."""
    v = float(x)
    if v != v:
        return "nan"
    if v == float("inf"):
        return "inf"
    if v == float("-inf"):
        return "-inf"
    return v


def channel_entry(mdf, gi, ci, channel):
    try:
        sig = mdf.get(channel.name, group=gi, index=ci, raw=False)
        samples = sig.samples
    except Exception as exc:  # noqa: BLE001 — a reference failure is data too
        return {"kind": "error", "detail": str(exc)[:200]}

    if channel.data_type in (13, 14):
        return {"kind": "canopen", "n": len(samples)}
    if samples.dtype.kind in ("U", "S"):
        return {
            "kind": "str",
            "n": len(samples),
            "first": decode_text(samples, channel.data_type),
        }
    if samples.dtype.kind == "V" and samples.dtype.names:
        # An array channel. asammdf bundles the elements with the axes in one
        # record; the field named after the channel holds the elements, which is
        # what falcon returns as `SignalValues::Array` and what is worth
        # comparing. Flattened row-major, the order both readers use.
        field = (
            channel.name
            if channel.name in samples.dtype.names
            else samples.dtype.names[0]
        )
        try:
            flat = np.asarray(samples[field], dtype=float).ravel()
        except (TypeError, ValueError):
            return {"kind": "other", "n": len(samples)}
        return {
            "kind": "num",
            "n": len(samples),
            "elements_per_sample": int(flat.size // max(len(samples), 1)),
            "first": [number(x) for x in flat[:TAKE]],
        }
    if samples.dtype.kind == "V" or (samples.dtype.kind == "u" and samples.ndim > 1):
        return {
            "kind": "bytes",
            "n": len(samples),
            "first": ["".join(f"{b:02x}" for b in bytes(x)) for x in samples[:TAKE]],
        }
    try:
        flat = np.asarray(samples, dtype=float).ravel()
    except (TypeError, ValueError):
        return {"kind": "other", "n": len(samples)}
    return {
        "kind": "num",
        "n": len(samples),
        "first": [number(x) for x in flat[:TAKE]],
    }


def main():
    paths = sorted(p for p in FILES.iterdir() if p.suffix.lower() == ".mf4")
    if not paths:
        sys.exit(f"no reference files in {FILES}; run scripts/fetch_reference_files.sh")

    out = {}
    for path in paths:
        try:
            mdf = MDF(path)
        except Exception as exc:  # noqa: BLE001
            print(f"  skipping {path.name}: {exc}")
            continue

        channels = {}
        for gi, group in enumerate(mdf.groups):
            for ci, channel in enumerate(group.channels):
                key = f"{gi}:{channel.name}"
                if key in channels:
                    continue
                entry = channel_entry(mdf, gi, ci, channel)
                note = DIVERGENCES.get((path.name, channel.name))
                if note:
                    entry = {"kind": "divergence", "n": entry.get("n", 0),
                             "reason": note[0]}
                channels[key] = entry

        out[path.name] = {"version": mdf.version, "channels": channels}
        print(f"  {path.name}: {len(channels)} channels")

    OUT.write_text(json.dumps(out, indent=1, sort_keys=True) + "\n")
    print(f"wrote {OUT} ({OUT.stat().st_size // 1024} KiB, {len(out)} files)")


if __name__ == "__main__":
    main()
