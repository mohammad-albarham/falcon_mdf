#!/usr/bin/env python3
"""Build a wide MF4 fixture (50k+ channels, 100 MB+, DZ-compressed) with asammdf.

The large fixtures from make_large_fixture.py are tall and narrow: 19 channels,
205 M samples. They measure raw decode throughput. A calibration-tool log
(INCA, CANape, XCP) is the opposite shape: tens of thousands of channels in a
handful of raster groups, each channel with its own name, unit, comment and
conversion. That shape stresses metadata parsing and per-channel overhead,
which the tall fixtures never reach.

Layout mirrors an XCP measurement: fast rasters carry few channels, slow
rasters carry many. Channel data is a mix of the kinds a real ECU log holds,
so the DZ blocks compress like real data rather than like zeros or noise:

  - float32 physical values (smooth signal plus sensor noise)
  - int16 raw values with a linear conversion (quantised sensor readings)
  - uint8 status/enum values that change rarely

asammdf is the writer for the same reason as in make_large_fixture.py: a file
written by falcon's own Mf4Writer would favour falcon's reader.

Usage:
    make_wide_fixture.py --out-dir test_data/wide
"""

import argparse
import pathlib
import sys
import time

import numpy as np
from asammdf import MDF, Signal

# (raster period in s, channel count). 50,000 channels in total.
RASTERS = [
    (0.010, 2_000),
    (0.010, 2_000),
    (0.020, 3_000),
    (0.020, 3_000),
    (0.100, 5_000),
    (0.100, 5_000),
    (0.100, 5_000),
    (0.100, 5_000),
    (1.000, 10_000),
    (1.000, 10_000),
]

SUBSYSTEMS = ["Inj", "Ign", "Air", "Fuel", "Exh", "Trq", "Lam", "Knk", "Thr", "Cool"]
UNITS = ["rpm", "Nm", "degC", "kPa", "mg/stk", "%", "V", "ms", "deg", "-"]


def channel_block(rng, n_samples, n_channels, t):
    """Yield (samples, unit, conversion) for each channel of one raster."""
    for ci in range(n_channels):
        kind = ci % 4
        unit = UNITS[ci % len(UNITS)]
        if kind in (0, 1):
            # float32 physical value: slow sine at a random frequency plus noise.
            f = rng.uniform(0.01, 0.5)
            amp = rng.uniform(1, 1000)
            x = amp * np.sin(2 * np.pi * f * t + rng.uniform(0, 6.28))
            x += rng.normal(0, amp * 0.01, n_samples)
            yield x.astype(np.float32), unit, None
        elif kind == 2:
            # int16 raw with a linear conversion, like a 12-bit ADC reading.
            f = rng.uniform(0.01, 0.5)
            raw = 2048 + 1500 * np.sin(2 * np.pi * f * t)
            raw += rng.normal(0, 4, n_samples)
            yield raw.astype(np.int16), unit, {"a": 0.01, "b": -40.0}
        else:
            # uint8 status: a few steps over the whole recording.
            steps = rng.integers(0, 8, size=max(1, n_samples // 500) + 1)
            yield np.repeat(steps, 500)[:n_samples].astype(np.uint8), "-", None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--duration", type=float, default=120.0,
                    help="Recording length in seconds (default 120)")
    ap.add_argument("--out-dir", default="test_data/wide")
    ap.add_argument("--name", default="wide_deflate.mf4")
    ap.add_argument("--compression", type=int, default=2,
                    help="asammdf compression: 0 none, 1 deflate, 2 transposed deflate")
    args = ap.parse_args()

    root = pathlib.Path(__file__).resolve().parents[4]
    out_dir = root / args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    rng = np.random.default_rng(0)
    mdf = MDF(version="4.10")
    t0 = time.perf_counter()
    n_total = 0
    for gi, (period, n_channels) in enumerate(RASTERS):
        n_samples = int(args.duration / period)
        t = np.arange(n_samples, dtype=np.float64) * period
        sigs = []
        for ci, (samples, unit, conv) in enumerate(
                channel_block(rng, n_samples, n_channels, t)):
            sub = SUBSYSTEMS[ci % len(SUBSYSTEMS)]
            name = f"ECU{gi}.{sub}.{sub}Ctl_{ci:05d}"
            sigs.append(Signal(samples, t, name=name, unit=unit, conversion=conv,
                               comment=f"{sub} controller signal {ci} in raster {period * 1000:g} ms"))
        mdf.append(sigs, comment=f"raster {period * 1000:g} ms")
        n_total += n_channels
        print(f"group {gi}: {n_channels} channels x {n_samples} samples "
              f"({time.perf_counter() - t0:.1f}s)", file=sys.stderr)

    out = out_dir / args.name
    t1 = time.perf_counter()
    mdf.save(out, compression=args.compression, overwrite=True)
    size_mb = out.stat().st_size / 1024 / 1024
    print(f"{args.name}: {n_total} channels, {size_mb:.1f} MB "
          f"(compression={args.compression}) in {time.perf_counter() - t1:.1f}s",
          file=sys.stderr)


if __name__ == "__main__":
    main()
