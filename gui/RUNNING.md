# Running falcon

How to build the viewer, open a measurement in it, and tell a real failure
apart from a known limitation. Build and distribution notes — bundles, icons,
per-platform packaging — live in [PACKAGING.md](PACKAGING.md).

## Build and run

The GUI is the `falcon_mdf_gui` package, a workspace member. Its binary is
called `falcon`. From the repository root:

```bash
# Release. Worth it: decimation and plotting are noticeably smoother, and a
# debug build stutters on the larger bus logs.
cargo run --release -p falcon_mdf_gui

# Debug. Compiles faster, sluggish on anything large.
cargo run -p falcon_mdf_gui
```

`cargo build --release -p falcon_mdf_gui` leaves a standalone binary at
`target/release/falcon` that runs without cargo.

## Opening a file

Four ways, all equivalent once the file is open:

1. **On the command line**, which opens it immediately at startup:

   ```bash
   cargo run --release -p falcon_mdf_gui -- test_data/reference/ASAP2_Demo_V171.mf4

   # or, against the standalone binary
   target/release/falcon test_data/reference/ASAP2_Demo_V171.mf4
   ```

2. **"Open File…"** in the top bar. The dialog filters on both `mf4` and `MF4`,
   so the upper-case vendor files are not hidden.
3. **Drag and drop** onto the window. No extension filter at all — it takes
   whatever path you drop.
4. **Recent Files** in the top bar, which persists across runs.

The window title becomes `falcon — <filename>`, so several open viewers stay
tellable apart.

## Getting measurement files

There are none in the repository and there never will be: they are other
vendors' files, this project does not redistribute them, and they are large.
The fetch script pulls them into `test_data/`, which is gitignored:

```bash
bash scripts/fetch_reference_files.sh   # 67 files, ~10 MB
```

Only the values those files decode to — the JSON under `tests/data/` — is
checked in. See `scripts/fetch_reference_files.sh` for what is fetched and from
where.

Good files to open first, once fetched:

| File | Why |
| --- | --- |
| `test_data/reference/ASAP2_Demo_V171.mf4` | 45 channels over 7 groups, 50,974 samples — a broad first look |
| `test_data/reference/Vector_MeasurementArrays.mf4` | array channels, several shapes |
| `test_data/reference/multiple.MF4` | an *unfinalized* 4.11 bus log, with both CAN and LIN groups |
| `test_data/mf4-sample-data-v2.1/J1939 (truck)/LOG/958D2219/00002501/00002081.MF4` | a real CAN log; the one to use for plotting and decimation |

The bus-logging corpus under `mf4-sample-data-v2.1/` has no public source and is
not fetched by the script.

## Known limitation: channels that plot as nothing

The viewer is a plotter. Every channel goes through `values_f64()`, so anything
without a numeric value — **strings, complex numbers, byte arrays, CANopen
dates and times** — decodes to all-`NaN` and draws an empty chart.

It does so **without any warning**. The `⚠` marker beside a channel means
*undecodable*, and these channels decode perfectly well; they simply have no
number to plot. So an empty chart currently looks the same as a broken one.

Across the 67-file reference set this affects **41 channels**.
`test_data/reference/all_datatypes_test.mf4` shows it in one file: `int8_data`
plots, while `string_data` and `complex64_data` look broken and are not. Use
the library's `Signal::values()` to see those samples in their own type.

## Checking the viewer against a whole corpus

To confirm the GUI's file-handling path opens everything — no crash, no hang,
and any failure showing its real error text:

```bash
cargo run -p falcon_mdf_gui --example verify_corpus -- test_data/reference
```

It prints a `PASS`/`FAIL` line per file with version, group and channel counts,
then a total. All 67 reference files pass.

**Run it from the repository root.** The default paths are relative to the
working directory, so from inside `gui/` it finds nothing and reports
`0 passed, 0 failed, 0 total` rather than saying the corpus is missing.
